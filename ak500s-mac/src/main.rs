//! ak500s-mac —— DeepCool AK500S DIGITAL 数显散热器的 macOS（黑苹果）驱动
//!
//! 用法：
//!   ak500s run                       # 常驻：温度/占用率轮换（默认 auto）
//!   ak500s run --mode temp           # 仅 CPU 温度
//!   ak500s run --mode usage          # 仅 CPU 占用率
//!   ak500s test --mode temp --value 47 --bar 5   # 发送固定报文验证链路
//!   ak500s doctor                    # 环境自检（设备/温度键/采样）
//!
//! 配置文件 ~/.config/ak500s-mac/config（key=value，见 docs/03 §1）。

mod api;
mod config;
mod device;
mod protocol;
mod runner;
mod sensor;

use clap::{Parser, Subcommand};
use config::{CliOverrides, Mode};
use std::thread::sleep;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "ak500s",
    version,
    about = "DeepCool AK500S DIGITAL display driver for macOS"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 常驻运行，持续刷新屏幕（登录自启动请配 LaunchAgent，见 scripts/）
    Run {
        /// 显示模式: temp | usage | auto
        #[arg(long)]
        mode: Option<String>,
        /// 华氏度
        #[arg(long)]
        fahrenheit: bool,
        /// 刷新周期毫秒（300–3000）
        #[arg(long)]
        interval_ms: Option<u64>,
        /// 报警阈值（℃）
        #[arg(long)]
        alarm_c: Option<f32>,
        /// auto 模式子模式停留秒数
        #[arg(long)]
        auto_switch_s: Option<u64>,
        /// 关闭 CPU 温度采集显示（配置文件键 show_temp=false 等效）
        #[arg(long)]
        no_temp: bool,
        /// 关闭 CPU 占用率采集显示；两者都关 = 纯 API 屏幕模式
        #[arg(long)]
        no_usage: bool,
    },
    /// 发送固定报文验证链路（屏幕显示几秒后自动消失属正常，看门狗行为）
    Test {
        /// temp | usage
        #[arg(long, default_value = "temp")]
        mode: String,
        /// 显示数值（0–999）
        #[arg(long, default_value_t = 47)]
        value: u16,
        /// 状态条档位（1–10）
        #[arg(long, default_value_t = 5)]
        bar: u8,
        /// 触发报警位
        #[arg(long)]
        alarm: bool,
        /// 重复发送次数（每秒一次，可观察看门狗行为）
        #[arg(long, default_value_t = 5)]
        repeat: u32,
    },
    /// 环境自检：设备枚举、产品字符串、SMC 温度键探测、实时采样
    Doctor,
    /// 向守护进程发布一帧显示（通用 API 的 CLI 等价物，见 docs/09）
    /// 例：ak500s show --value 123 --bar 5 --unit pct --ttl 10
    Show {
        /// 显示数值（0–999）
        #[arg(long)]
        value: u16,
        /// 状态条：0–10（0 熄灭）或 "usage"（跟随 CPU 占用率），缺省熄灭
        #[arg(long)]
        bar: Option<String>,
        /// 数字旁单位图标：pct | c | f（缺省 pct）
        #[arg(long, default_value = "pct")]
        unit: String,
        /// 触发报警位
        #[arg(long)]
        alarm: bool,
        /// 前台占用秒数，到期自动回落默认显示
        #[arg(long, default_value_t = api::DEFAULT_TTL_S)]
        ttl: u64,
    },
    /// 探测守护进程 API 是否在线
    Ping,
    /// 运行时开关内置采集指标显示（不传的字段不动；远程、立即生效、不落盘）
    /// 例：ak500s-mac config --show-temp off
    Config {
        /// true/false（on/off/1/0 也可）
        #[arg(long)]
        show_temp: Option<String>,
        #[arg(long)]
        show_usage: Option<String>,
    },
    /// 远程切换内置显示模式（temp/usage/auto）
    Rmode {
        #[arg(long)]
        mode: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let rc = match cli.cmd {
        Cmd::Run { mode, fahrenheit, interval_ms, alarm_c, auto_switch_s, no_temp, no_usage } => {
            let overrides = match parse_mode(mode.as_deref()) {
                Ok(m) => CliOverrides {
                    mode: m,
                    fahrenheit: if fahrenheit { Some(true) } else { None },
                    interval_ms,
                    alarm_c,
                    auto_switch_s,
                    show_temp: if no_temp { Some(false) } else { None },
                    show_usage: if no_usage { Some(false) } else { None },
                },
                Err(e) => {
                    eprintln!("[错误] {e}");
                    std::process::exit(2);
                }
            };
            let cfg = config::Config::load(overrides);
            match runner::run(cfg) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("[错误] {e}");
                    1
                }
            }
        }
        Cmd::Test { mode, value, bar, alarm, repeat } => test_cmd(&mode, value, bar, alarm, repeat),
        Cmd::Doctor => doctor_cmd(),
        Cmd::Show { value, bar, unit, alarm, ttl } => {
            // CLI 的 bar 是字符串："usage" 原样传；数字转成 JSON 数值
            let bar_json = match bar.as_deref().map(str::trim) {
                None => serde_json::Value::Null,
                Some("usage") => serde_json::Value::String("usage".into()),
                Some(s) => match s.parse::<u8>() {
                    Ok(n) => serde_json::json!(n),
                    Err(_) => serde_json::Value::String(s.into()), // 让服务端给出明确报错
                },
            };
            api_client_cmd(
                &serde_json::json!({
                    "cmd": "show", "value": value, "unit": unit,
                    "bar": bar_json, "alarm": alarm, "ttl_s": ttl
                })
                .to_string(),
            )
        }
        Cmd::Ping => api_client_cmd(r#"{"cmd":"ping"}"#),
        Cmd::Rmode { mode } => api_client_cmd(
            &serde_json::json!({ "cmd": "mode", "mode": mode }).to_string(),
        ),
        Cmd::Config { show_temp, show_usage } => {
            let parse_flag = |s: &Option<String>, name: &str| -> Result<serde_json::Value, String> {
                match s.as_deref().map(str::trim) {
                    None => Ok(serde_json::Value::Null),
                    Some("true" | "on" | "1") => Ok(serde_json::json!(true)),
                    Some("false" | "off" | "0") => Ok(serde_json::json!(false)),
                    Some(other) => Err(format!("{name} 需要 true/false（得到 {other:?}）")),
                }
            };
            match (parse_flag(&show_temp, "--show-temp"), parse_flag(&show_usage, "--show-usage")) {
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("[错误] {e}");
                    1
                }
                (Ok(t), Ok(u)) if t.is_null() && u.is_null() => {
                    eprintln!("[错误] 至少指定 --show-temp 或 --show-usage 之一");
                    1
                }
                (Ok(t), Ok(u)) => api_client_cmd(
                    &serde_json::json!({ "cmd": "config", "show_temp": t, "show_usage": u })
                        .to_string(),
                ),
            }
        }
    };
    std::process::exit(rc);
}

fn api_client_cmd(request: &str) -> i32 {
    match api::send_request(request) {
        Ok(resp) => {
            println!("{resp}");
            // 服务端拒绝（ok:false）也返回非零，脚本可凭退出码判断成败
            if resp.contains("\"ok\":false") {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("[错误] {e}");
            1
        }
    }
}

fn parse_mode(s: Option<&str>) -> Result<Option<Mode>, String> {
    match s.map(|m| m.trim()).filter(|m| !m.is_empty()) {
        None => Ok(None),
        Some("temp") | Some("temperature") => Ok(Some(Mode::Temp)),
        Some("usage") => Ok(Some(Mode::Usage)),
        Some("auto") => Ok(Some(Mode::Auto)),
        Some(other) => Err(format!("未知模式 {other:?}（可用: temp/usage/auto）")),
    }
}

fn test_cmd(mode: &str, value: u16, bar: u8, alarm: bool, repeat: u32) -> i32 {
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[错误] hidapi 初始化失败: {e}");
            return 1;
        }
    };
    let disp = match device::find(&api) {
        Some(d) => d,
        None => {
            eprintln!("[错误] 未发现 AK500S（{:04x}:{:04x}）", protocol::VID, protocol::PID);
            return 1;
        }
    };
    eprintln!(
        "[信息] 已连接 {}{}，发送 {repeat} 帧（每秒 1 帧）…（若 `ak500s run` 正在运行，双方会抢写设备，屏幕可能闪烁；测试期间建议先停掉它）",
        disp.product,
        if disp.se { "（SE 变体）" } else { "" }
    );
    let mode_byte = match mode {
        "temp" | "temperature" => protocol::MODE_TEMP_C,
        "usage" => protocol::MODE_USAGE,
        other => {
            eprintln!("[错误] 未知 test 模式 {other:?}（可用: temp/usage）");
            return 1;
        }
    };
    for i in 1..=repeat {
        let pkt = protocol::packet(mode_byte, bar, value, alarm, disp.se);
        match disp.write(&pkt) {
            Ok(()) => eprintln!("  第 {i}/{repeat} 帧已发送"),
            Err(e) => {
                eprintln!("[错误] 写入失败: {e}");
                return 1;
            }
        }
        sleep(Duration::from_secs(1));
    }
    eprintln!("[完成] 停止发送后屏幕内容几秒内消失属正常（固件看门狗）。");
    0
}

fn doctor_cmd() -> i32 {
    let mut ok = true;
    println!("== ak500s doctor ==");

    // 1. HID 设备
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            println!("[FAIL] hidapi 初始化失败: {e}");
            return 1;
        }
    };
    let devices = device::list_all(&api);
    if devices.is_empty() {
        println!("[FAIL] 未发现 {:04x}:{:04x} 设备（检查 9-pin 接线 / USB 端口映射）", protocol::VID, protocol::PID);
        ok = false;
    } else {
        println!("[OK] 找到 {} 台 AK500S:", devices.len());
        for (p, s, up, u) in &devices {
            println!("     product={p:?} serial={s:?} usagePage=0x{up:x} usage=0x{u:x}");
            println!("     变体判定: {}", if p.starts_with("AK") { "标准版（报文带 Report ID 16）" } else { "SE 变体（报文不带 Report ID）" });
        }
    }

    // 2. SMC 温度键
    println!("— SMC CPU 温度键探测 —");
    let mut any_key = false;
    for k in sensor::temp::KEY_CHAIN {
        match sensor::temp::read_temp_c(k) {
            Ok(v) => {
                println!("[OK] {k} = {v:.1}℃ ← 将使用此键");
                any_key = true;
                break;
            }
            Err(e) => println!("[--] {k} 不可用: {e}"),
        }
    }
    if !any_key {
        println!("[FAIL] 无可用温度键。黑苹果请安装 VirtualSMC + SMCProcessor（AMD: SMCAMDProcessor）");
        ok = false;
    }

    // 3. CPU 占用率采样
    println!("— CPU 占用率采样（2 秒窗口）—");
    let mut cpu = sensor::usage::CpuUsage::new();
    let _ = cpu.sample();
    sleep(Duration::from_millis(2000));
    println!("[OK] 当前占用率 {:.1}%", cpu.sample());

    println!("{}", if ok { "== 自检通过，可 `ak500s run` ==" } else { "== 存在问题，请按上面提示处理 ==" });
    if ok { 0 } else { 1 }
}
