# 03 · macOS 端系统设计

## 1. 技术选型

| 层 | 选型 | 理由 |
|---|---|---|
| 语言 | **Rust（2021 edition）** | 与参考实现同语言，`ak_series.rs` 报文逻辑可近乎原样复用；交叉编译产出无依赖单二进制 |
| HID 访问 | **`hidapi` crate 2.x**（macOS 走 IOKit HID 后端） | 参考项目同款；免 root 免驱动；Windows/macOS/Linux 三平台同 API，便于在 Windows 上先做协议冒烟 |
| CPU 占用率 | Mach `host_processor_info64`（经 `sysinfo` 或 `mach2` crate） | 原生 API，黑苹果与真机行为一致 |
| CPU 温度 | **IOKit SMC User Client**（读 SMC 键） | 黑苹果温度的唯一通用来源，见 §3 |
| 配置 | `~/.config/ak500s-mac/config.toml` + CLI 参数 | 与参考项目习惯一致 |
| 常驻 | **launchd LaunchAgent**（`~/Library/LaunchAgents/`） | macOS 标准自启动方式，无需 GUI 即可后台运行 |
| 日志 | `tracing` + 滚动文件 `~/Library/Logs/ak500s-mac/` | 排障友好 |

**为什么不做 Electron/Swift GUI 先行**：AK500S 屏幕功能面极窄（温度/占用/报警），
GUI 只用于切换模式与单位，CLI + 配置文件已覆盖 100% 硬件功能；
菜单栏 App 列为阶段 5 可选项（SwiftUI 薄壳 + XPC/Unix socket 调 Rust 守护进程）。

## 2. 模块设计

```
ak500s-mac（单二进制，守护进程模式 & 一次性命令模式）
├── device/            设备层
│   ├── probe.rs       枚举 0x3633:0x0004；读产品字符串判 SE；热插拔重连（IOHIDDeviceCallbacks）
│   └── protocol.rs    §02 协议常量、组包、SE 偏移、初始化序列   ← 自参考项目移植
├── sensor/            传感器层
│   ├── usage.rs       host_processor_info64 差分 → CPU%（1s 窗口）
│   ├── temp.rs        SMC 读取（键探测链，见 §3）
│   └── mod.rs         传感器健康自检：无温度源时降级（见 §3.4）
├── app/               应用层
│   ├── runner.rs      主循环（750ms tick）：采集 → 组包 → 写 → 异常恢复
│   ├── power.rs       睡眠/唤醒监听（IORegisterForSystemPower），唤醒后重初始化
│   └── config.rs      模式（temp/usage/auto）、单位、报警阈值、刷新周期
└── main.rs            clap CLI：`ak500s run` / `ak500s test --temp 47` / `ak500s doctor`
```

### 关键行为

- **auto 模式**：温度/占用每 10s 轮换（复刻官方 auto 行为）；
- **自检命令 `doctor`**：逐项输出 USB 枚举结果、产品字符串、SMC 可用键、
  采样到的温度/占用值 —— 黑苹果排障一键定位；
- **测试命令 `test`**：注入固定数值发送屏幕，用于脱离传感器单独验证链路。

## 3. 黑苹果 CPU 温度来源（本项目最大风险点，重点设计）

黑苹果没有真 SMC，温度由 **OpenCore + VirtualSMC 家族 kext 虚拟**。
用户态读取方式与真机相同：IOKit SMC User Client（`AppleSMC.kext` 服务，io_service `AppleSMC`），
发 `#KEY` 读取 4 字符键值。参考实现：`osx-cpu-temp`（读法成熟，几十行可移植为 Rust）。

### 3.1 键探测链（按序尝试，取第一个有效值）

| 优先级 | SMC 键 | 提供者 | 说明 |
|---|---|---|---|
| 1 | `TC0P` / `TC0C…TC7C`（取 max） | SMCProcessor.kext | CPU 包温/核心温，**黑苹果最常见可用** |
| 2 | `TCAD`/`TC1P` 等 OEM 变体 | SMCOemSensors.kext | 视主板而定 |
| 3 | `Tp01`（PECI） | 部分平台 | Intel 旧平台 |

> AMD 平台黑苹果：SMCProcessor 对 Ryzen 的支持依赖 `SMCAMDProcessor`（第三方 kext），
> 提供 `TC0P` 等。需在 README 中列出 kext 依赖清单。

### 3.2 SMC 读取算法（用户态，无需驱动）

```
IOServiceGetMatchingService("AppleSMC") → IOConnectCallMethod:
  1) SMC_READ_KEYINFO (kSMCGetKeyInfo) 取键长度/类型
  2) SMC_READ_KEY     (kSMCSReadKey)    取原始值
类型解码：`sp78`（温度常用，7位整数+8位小数）、`flt `、`ui16` 等
```

### 3.3 兜底策略

| 情形 | 行为 |
|---|---|
| SMC 无任何 CPU 温度键 | 自动切换 **usage-only 模式**并日志告警（屏幕显示占用率，功能不完全丢失） |
| 温度读数异常（<0 或 >120） | 丢弃该样本，连续 10 次异常后降级 |

### 3.4 验证方法

`ak500s doctor` 输出全部探测键的命中情况；同时提供与
Intel Power Gadget / Stats 菜单栏 App 读数的对照说明。

## 4. 打包与分发

- `cargo build --release --target universal2-apple-darwin`（或分别构建 x86_64/aarch64 后 lipo 合并）；
- ad-hoc 签名即可本机运行；发布到 GitHub Releases 提供裸二进制 + `install.sh`（写 LaunchAgent plist）；
- 可选：Homebrew tap。

## 5. 与官方 Windows 软件共存

双系统机器上互不干扰：Windows 侧官方软件、macOS 侧本项目各自独立访问 HID 设备；
无需修改散热器固件，无刷写风险（协议为单向显示，不含 OTA 命令）。
