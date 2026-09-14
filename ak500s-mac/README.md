# ak500s-mac

DeepCool **AK500S DIGITAL**（九州风神 AK500S 数显版）CPU 散热器屏幕的 macOS（黑苹果）驱动。
纯用户态实现（hidapi / IOKit HID），**无需内核扩展、无需 root**。

协议已实机验证（2026-09-08，见 `../docs/02-通信协议规格.md`）：
VID `0x3633` / PID `0x0004`，64 字节 HID Output Report，Report ID 16。
屏幕带固件看门狗——停止刷新数秒后回退默认状态，因此本程序以 1s 周期常驻刷新。

## 功能

- 默认 **auto 模式**：CPU 温度（℃/℉）与占用率每 10s 轮换显示
- 状态条跟随占用率；温度超阈值（默认 90℃）触发屏幕报警
- **通用 API**：任意本机软件可发布显示内容（时间/LLM token/进度…），
  TTL 到期自动回落默认显示（协议见 `../docs/09-通用API接口设计.md`）
- 热插拔/睡眠唤醒自动重连重试
- 无温度源（缺 SMC kext）时自动降级为占用率模式
- `doctor` 一键自检：设备枚举 / 变体判定 / SMC 键探测 / 采样

## 构建（在 Mac 上）

```bash
# 依赖：Xcode CLT（hidapi 需要编译一小段 C）与 Rust
xcode-select --install        # 若已装可跳过
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

cargo build --release
./target/release/ak500s-mac doctor     # 先自检
./target/release/ak500s-mac test --mode temp --value 47   # 链路测试（屏幕显示 47，数秒后消失属正常）
./target/release/ak500s-mac run        # 常驻运行（默认 auto：温度/占用率轮换）
```

通用二进制（Intel + Apple Silicon 黑苹果）：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
lipo -create \
  target/x86_64-apple-darwin/release/ak500s-mac \
  target/aarch64-apple-darwin/release/ak500s-mac \
  -out ak500s
```

## 开机自启动

```bash
./scripts/install.sh            # 安装并启动 LaunchAgent（日志在 ~/Library/Logs/ak500s-mac/）
./scripts/uninstall.sh          # 停止并卸载
```

## 配置

CLI 参数优先于配置文件 `~/.config/ak500s-mac/config`：

```
mode=auto            # auto（默认）| temp | usage
unit=c               # c | f
interval_ms=1000     # 刷新周期 300–3000（看门狗保护）
alarm_c=90           # 报警阈值（℃）
auto_switch_s=10     # auto 模式子模式停留秒数
show_temp=true       # CPU 温度采集显示开关
show_usage=true      # CPU 占用率采集显示开关（全关=纯 API 屏幕模式）
```

示例：`ak500s run --mode auto --fahrenheit --alarm_c 85`
（`--no-temp` / `--no-usage` 可在启动时关闭对应采集指标显示）

## 通用 API（给其它软件用）

`run` 运行时，本机套接字（macOS `/tmp/ak500s.sock`，开发用 Windows `127.0.0.1:5577`）
接受 NDJSON 请求，CLI 也提供等价命令：

```bash
ak500s-mac show --value 123 --bar 5 --ttl 10     # 显示 123，状态条 5 档，10 秒后回落
ak500s-mac show --value 42 --bar usage --unit c  # 状态条跟随 CPU 占用率
ak500s-mac show --value 88 --bar 9 --alarm       # 带报警位
ak500s-mac rmode --mode auto                     # 远程切换内置模式
ak500s-mac ping
```

完整协议与 cookbook（时钟 / LLM tokens/s / 下载进度脚本）：
`../docs/09-通用API接口设计.md`

## 黑苹果温度读不到？

温度键由 SMC kext 提供，请确认 OpenCore 装有：

- **Intel 平台**：VirtualSMC.kext + SMCProcessor.kext（+ SMCSuperIO.kext）
- **AMD 平台**：VirtualSMC.kext + [SMCAMDProcessor.kext](https://github.com/trulysinclair/SMCAMDProcessor)

装好后 `ak500s doctor` 应看到 `TC0P = xx.x℃ ← 将使用此键`。

## 已知行为

- 屏幕内容"几秒后消失"是固件看门狗设计（官方软件同样需要常驻刷新）；
- `test` 命令发完即停，屏幕随后回默认属正常现象；
- SE 变体（产品名不以 "AK" 开头）自动识别并切换无 Report ID 报文（本机实测为标准版）。

## 许可

MIT。本项目与 DeepCool 官方无关；协议信息来自社区逆向项目与实机验证（详见 `../docs/`）。
