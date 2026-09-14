# 09 · 通用 API 接口设计（任意软件驱动屏幕）

> 实现：`ak500s-mac` v0.2.0 起，`src/api.rs` + `runner.rs` 共享状态。
> 已完成 12 项单测 + 本机 TCP 端到端实测（2026-09-09）。
> 用途：时间 / LLM token 吞吐 / 下载进度 / 倒计时 / 任意 0–999 数值。

## 1. 核心设计：单写者 + TTL 抢占

屏幕是 HID 设备，**多进程同时写会互相覆盖帧**。因此：

```
┌──────────┐  JSON 行   ┌────────────────────────────┐  HID 64B   ┌─────┐
│ 时钟脚本  │──────────→ │                            │──────────→ │     │
│ LLM 客户端│──────────→ │ ak500s run（唯一设备写者）    │            │ 屏幕 │
│ 任意程序  │──────────→ │  外部帧有效 → 抢占前台        │            │     │
└──────────┘            │  TTL 到期  → 回落内置模式      │            └─────┘
                        └────────────────────────────┘
```

- **发布帧带 TTL**（默认 10s，可 1–3600s）：持续发布（每秒一次）则一直保持前台；
  停止发布后 TTL 到期，自动回落到配置的内置模式（温度/占用率/auto）——
  客户端崩溃也不会把屏幕"劫持"在脏数据上；
- **最后写者胜**：多个发布者并存时不排队，后发布的帧覆盖前者（简单可预期）；
- **看门狗安全**：守护进程仍以固定周期重发当前帧（内置或外部），外部程序只需在
  内容变化时发布，无需关心刷新节奏。

## 2. 传输与寻址

| 平台 | 端点 | 说明 |
|---|---|---|
| **macOS** | Unix domain socket `/tmp/ak500s.sock` | 正式路径；本机用户级，connect 需 socket 文件写权限（默认 umask 下其他用户不可写） |
| Windows（开发） | TCP `127.0.0.1:5577` | 仅监听回环；协议完全一致，用于无 Mac 时联调 |

帧格式：**NDJSON**——一行一个 JSON 请求，一行一个 JSON 响应，UTF-8，连接可复用（发多条）。

## 3. 请求（4 条命令）

### 3.1 `show` —— 发布一帧显示

```json
{"cmd":"show","value":123,"bar":5,"unit":"pct","alarm":false,"ttl_s":10}
```

| 字段 | 类型 | 必填 | 约束/缺省 |
|---|---|---|---|
| `value` | int | ✅ | 0–999，超界钳位到 999（屏幕只有三位数字） |
| `bar` | int 或 `"usage"` | ❌ | 0–10 档位（**0=熄灭**）；`"usage"`=跟随 CPU 占用率（守护进程现算）；缺省=0 |
| `unit` | string | ❌ | 数字旁单位图标：`pct`(%，缺省) / `c`(℃) / `f`(℉)。固件只提供这三种图标，无"无图标"选项 |
| `alarm` | bool | ❌ | true 时屏幕显示报警标记（缺省 false） |
| `ttl_s` | int | ❌ | 前台占用秒数 1–3600（缺省 10） |

响应：`{"ok":true}`

### 3.2 `mode` —— 远程切换内置模式（TTL 无关，持久到再改或 release）

```json
{"cmd":"mode","mode":"auto"}    // temp | usage | auto
```

### 3.2b `config` —— 运行时开关内置采集指标显示

```json
{"cmd":"config","show_temp":false}                   // 只关温度（不传的字段不动）
{"cmd":"config","show_temp":true,"show_usage":false} // 开温度、关占用率
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `show_temp` | bool | CPU 温度显示开关（缺省=不变） |
| `show_usage` | bool | CPU 占用率显示开关（缺省=不变） |

- 至少传一个字段，否则报错；
- **立即生效、不落盘**（重启守护进程后回到配置文件/CLI 的设定）；
- 当前显示的指标被关掉时自动回落另一指标；**两者全关 = 纯 API 屏幕模式**：
  守护进程停止发送内置帧（屏幕由看门狗回默认状态），仅外部 `show` 帧可见；
- `release` 会连同 config 覆盖一起复位。

### 3.3 `release` —— 立即放弃前台

清掉当前外部帧、mode 覆盖与 config 覆盖，屏幕立刻回到配置模式：`{"cmd":"release"}`

### 3.4 `ping` —— 健康检查

```json
{"cmd":"ping"}   →   {"ok":true,"v":1}
```

### 错误响应

字段校验失败（不崩服务）：`{"ok":false,"err":"bar 档位越界: 11（0–10）"}`

## 4. CLI 等价命令（脚本/Shortcuts/Raycast 直接调）

```bash
ak500s-mac show --value 123 --bar 5 --unit pct --ttl 10
ak500s-mac show --value 42 --bar usage --unit c     # 状态条跟随 CPU 占用率
ak500s-mac show --value 95 --alarm                   # 报警演示
ak500s-mac rmode --mode auto                         # 远程切内置模式
ak500s-mac config --show-temp off                    # 运行时关温度显示
ak500s-mac config --show-temp on --show-usage off    # 运行时开温度关占用
ak500s-mac ping
```

> CLI 内部走同一 API；`--bar` 接受 `0`–`10` 或 `usage`。

## 5. Cookbook

### 5.1 时钟（小时数 + 分钟进度条）

屏幕只有三位数字，放不下 HH:MM——用"数字=小时、状态条=分钟/6"的映射：
（14:37 → 数字显示 14，状态条 10 档中亮 6 档）

```bash
#!/bin/bash
# clock.sh —— 需在 macOS 上运行（连接 UDS）
while true; do
  h=$(date +%H); m=$(date +%M)
  bar=$(( 10#$m / 6 + 1 ))          # 0–59 → 1–10 档
  /usr/local/bin/ak500s-mac show --value $((10#$h)) --bar $bar --unit pct --ttl 3
  sleep 2
done
```

### 5.2 LLM token 吞吐（每秒 tokens/s）

Python 例：流式回调里统计 chunk 数，1 秒窗口发布一次：

```python
import json, socket, time

SOCK = "/tmp/ak500s.sock"

def publish(value: int, ttl_s: int = 5):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(SOCK)
        s.sendall(json.dumps({"cmd": "show", "value": min(value, 999),
                              "bar": "usage", "unit": "pct",
                              "ttl_s": ttl_s}).encode() + b"\n")
        print(s.recv(256).decode().strip())

# 你的推理循环里：
# 每秒调用 publish(tokens_in_last_second)
```

> 数值范围 0–999：token 吞吐超 999 时可显示 `t/10`（十位精度），脚本内自行换算。

### 5.3 任意语言/脚本的最小客户端

有 `ak500s-mac` 二进制就够（shell 一行）：

```bash
ak500s-mac show --value 87 --bar 9 --ttl 5
```

无依赖轮询式发布（如下载进度）：

```bash
while read -r pct; do ak500s-mac show --value "$pct" --bar $((pct/10+1)) --ttl 5; done
```

## 6. 行为细则

| 场景 | 行为 |
|---|---|
| 守护进程未运行 | CLI 报"无法连接 API"；第三方客户端 connect 失败（程序应提示用户先 `ak500s run`） |
| 设备拔出期间发布 | 请求照常受理（写入共享状态）；设备重连后按剩余 TTL 渲染 |
| TTL 内新的 show | 覆盖内容并重置 TTL 起点（新帧按自己的 ttl_s 计时） |
| `release` 后 | 立即回落内置模式（无需等 TTL） |
| mode 覆盖与外部帧并存 | 外部帧优先；帧过期后执行被覆盖到的内置模式 |
| 屏幕睡眠/系统睡眠 | 与内置模式同一恢复路径（重连+重初始化） |
| 连接空闲超 30 秒 | 服务端断开该连接（客户端按需重连即可；每请求新建连接的用法无感知） |
| 多用户隔离 | macOS socket 位于 /tmp，跨用户 connect 受文件权限限制；单用户黑苹果无此问题 |

## 7. 版本与演进

- 响应带 `v:1`（仅 ping 携带）；未来不兼容变更将升 `v` 并保留旧命令语义一版；
- 预留方向（按需实现）：订阅式回调（屏幕按键/温度阈值事件推送）、
  多帧队列（带优先级的轮播）、HTTP 回环镜像（若第三方生态需要 curl 直连）。
