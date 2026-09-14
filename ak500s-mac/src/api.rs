//! 本机通用 API：其他软件借此驱动屏幕（时间 / LLM token / 进度 / 任意数值）。
//!
//! 架构原则（docs/09）：HID 设备单写者 —— 只有 `run` 守护进程写设备；
//! 外部程序通过本机套接字发布显示帧（NDJSON，一行一请求），
//! 守护进程以"TTL 抢占 + 到期回落"策略合成最终帧。
//! 传输：macOS 用 Unix domain socket `/tmp/ak500s.sock`；
//! Windows 仅作开发用，等价走 TCP 127.0.0.1:5577，协议完全一致。

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Mode;
use crate::protocol;

pub const PROTOCOL_V: u32 = 1;
pub const DEFAULT_TTL_S: u64 = 10;
pub const MAX_TTL_S: u64 = 3600;

#[cfg(unix)]
pub const SOCKET_PATH: &str = "/tmp/ak500s.sock";
#[cfg(windows)]
pub const TCP_ADDR: &str = "127.0.0.1:5577";

/// 外部显示帧（API 写入，主循环消费）
#[derive(Debug, Clone)]
pub struct ExternalFrame {
    /// D1 模式字节（决定屏幕上的单位图标：℃/℉/%）
    pub mode_byte: u8,
    /// 状态条：None = "usage"（由主循环按当前 CPU 占用率计算）
    pub bar: Option<u8>,
    pub value: u16,
    pub alarm: bool,
    pub expires: Instant,
}

/// 守护进程共享状态（API 线程写，主循环读）
#[derive(Debug, Default)]
pub struct SharedState {
    pub external: Option<ExternalFrame>,
    /// API 的 mode 命令覆盖内置显示模式；None = 用配置文件/CLI 的模式
    pub mode_override: Option<Mode>,
    /// API 的 config 命令覆盖内置指标显示开关；None = 用配置文件/CLI
    pub show_temp_override: Option<bool>,
    pub show_usage_override: Option<bool>,
}
pub type SharedHandle = Arc<Mutex<SharedState>>;

/// 主循环每帧调用：取走仍然有效的外部帧；过期帧顺带清除。
pub fn take_live_external(shared: &SharedHandle) -> Option<ExternalFrame> {
    let mut g = shared.lock().unwrap();
    let live = match &g.external {
        Some(e) if e.expires > Instant::now() => Some(e.clone()),
        _ => None,
    };
    if live.is_none() {
        g.external = None;
    }
    live
}

// ---------- 请求模型 ----------

#[derive(Deserialize)]
struct Req {
    cmd: String,
    #[serde(default)]
    value: Option<u16>,
    /// 整数 0–10（0=熄灭）或字符串 "usage"（跟随 CPU 占用率）
    #[serde(default)]
    bar: Option<Bar>,
    /// 数字旁的单位图标：pct(%) | c(℃) | f(℉)，默认 pct
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    alarm: Option<bool>,
    /// 前台占用时长（秒），到期自动回落默认显示；持续发布则每次刷新
    #[serde(default)]
    ttl_s: Option<u64>,
    /// mode 命令用：temp | usage | auto
    #[serde(default)]
    mode: Option<String>,
    /// config 命令用：内置指标显示开关（true/false）
    #[serde(default)]
    show_temp: Option<bool>,
    #[serde(default)]
    show_usage: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Bar {
    Level(u8),
    Usage(String),
}

/// 处理一行请求（服务端与测试共用）。返回一行响应 JSON。
pub fn dispatch(shared: &SharedHandle, line: &str) -> Value {
    let req: Req = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return err(&format!("请求解析失败: {e}")),
    };
    match req.cmd.as_str() {
        "ping" => json!({"ok": true, "v": PROTOCOL_V}),
        "show" => cmd_show(shared, &req),
        "mode" => cmd_mode(shared, &req),
        "config" => cmd_config(shared, &req),
        "release" => {
            let mut g = shared.lock().unwrap();
            g.external = None;
            g.mode_override = None;
            g.show_temp_override = None;
            g.show_usage_override = None;
            ok()
        }
        other => err(&format!("未知命令 {other:?}（可用: show/mode/config/release/ping）")),
    }
}

fn cmd_show(shared: &SharedHandle, req: &Req) -> Value {
    let Some(value) = req.value else {
        return err("show 需要 value 字段（0–999）");
    };
    let value = value.min(999);
    let mode_byte = match req.unit.as_deref().unwrap_or("pct") {
        "pct" => protocol::MODE_USAGE,
        "c" => protocol::MODE_TEMP_C,
        "f" => protocol::MODE_TEMP_F,
        other => return err(&format!("未知 unit {other:?}（可用: pct/c/f）")),
    };
    let bar = match &req.bar {
        None => Some(0), // 缺省熄灭
        Some(Bar::Usage(s)) if s == "usage" => None,
        Some(Bar::Usage(s)) => return err(&format!("bar 仅支持 0–10 或 \"usage\"，得到 {s:?}")),
        Some(Bar::Level(n)) if *n <= 10 => Some(*n),
        Some(Bar::Level(n)) => return err(&format!("bar 档位越界: {n}（0–10）")),
    };
    let ttl = req.ttl_s.unwrap_or(DEFAULT_TTL_S).clamp(1, MAX_TTL_S);
    let frame = ExternalFrame {
        mode_byte,
        bar,
        value,
        alarm: req.alarm.unwrap_or(false),
        expires: Instant::now() + Duration::from_secs(ttl),
    };
    shared.lock().unwrap().external = Some(frame);
    ok()
}

fn cmd_mode(shared: &SharedHandle, req: &Req) -> Value {
    let Some(m) = &req.mode else {
        return err("mode 需要 mode 字段（temp/usage/auto）");
    };
    let parsed = match m.as_str() {
        "temp" | "temperature" => Mode::Temp,
        "usage" => Mode::Usage,
        "auto" => Mode::Auto,
        other => return err(&format!("未知模式 {other:?}（可用: temp/usage/auto）")),
    };
    shared.lock().unwrap().mode_override = Some(parsed);
    ok()
}

/// config：远程开关内置采集指标（CPU 温度 / CPU 占用率）。
/// `{"cmd":"config","show_temp":false}` —— 关闭温度；不传的字段不动。
/// 全部关闭后进入"纯 API 屏幕模式"：仅外部 show 帧可见（看门狗心跳照常发送）。
fn cmd_config(shared: &SharedHandle, req: &Req) -> Value {
    if req.show_temp.is_none() && req.show_usage.is_none() {
        return err("config 需要 show_temp 和/或 show_usage 字段（true/false）");
    }
    let mut g = shared.lock().unwrap();
    if let Some(t) = req.show_temp {
        g.show_temp_override = Some(t);
    }
    if let Some(u) = req.show_usage {
        g.show_usage_override = Some(u);
    }
    ok()
}

fn ok() -> Value {
    json!({"ok": true})
}
fn err(msg: &str) -> Value {
    json!({"ok": false, "err": msg})
}

// ---------- 服务端 ----------

/// 在后台线程监听 API 套接字；错误返回给调用方（run 启动时打印）。
pub fn spawn(shared: SharedHandle) -> std::io::Result<()> {
    #[cfg(unix)]
    let listener = {
        let path = std::path::Path::new(SOCKET_PATH);
        // 双实例防护：先探测既有 socket 是否可连接（活着 = 已有守护进程），
        // 避免"删掉别人的 socket 再 bind"把老实例变成无人能连的孤儿。
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("已有守护进程在运行（{SOCKET_PATH} 可连接），请勿重复启动"),
            ));
        }
        let _ = std::fs::remove_file(path); // 上次异常退出的残留文件
        std::os::unix::net::UnixListener::bind(path)?
    };
    #[cfg(windows)]
    let listener = std::net::TcpListener::bind(TCP_ADDR).map_err(|e| {
        // 5577 被占用大概率就是另一个实例
        std::io::Error::new(e.kind(), format!("{e}（可能已有守护进程在运行）"))
    })?;

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let shared = shared.clone();
                    std::thread::spawn(move || handle_conn(s, shared));
                }
                Err(_) => continue,
            }
        }
    });
    Ok(())
}

#[cfg(unix)]
fn handle_conn(stream: std::os::unix::net::UnixStream, shared: SharedHandle) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    if let Ok(mut w) = stream.try_clone() {
        serve_loop(&mut BufReader::new(stream), &mut w, shared);
    }
}
#[cfg(windows)]
fn handle_conn(stream: std::net::TcpStream, shared: SharedHandle) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    if let Ok(mut w) = stream.try_clone() {
        serve_loop(&mut BufReader::new(stream), &mut w, shared);
    }
}

fn serve_loop(reader: &mut impl BufRead, writer: &mut impl std::io::Write, shared: SharedHandle) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return, // 客户端关闭
            Ok(_) => {}
        }
        let resp = dispatch(&shared, line.trim());
        let _ = writeln!(writer, "{resp}");
        let _ = writer.flush();
    }
}

// ---------- 客户端（CLI 子命令用） ----------

/// 发送一行请求并返回一行响应（CLI show/ping/mode 用）。
pub fn send_request(line: &str) -> Result<String, String> {
    #[cfg(unix)]
    let mut s = std::os::unix::net::UnixStream::connect(SOCKET_PATH)
        .map_err(|_| format!("无法连接 API（守护进程未运行？先执行 ak500s run）：{SOCKET_PATH}"))?;
    #[cfg(windows)]
    let mut s = std::net::TcpStream::connect(TCP_ADDR)
        .map_err(|_| format!("无法连接 API（守护进程未运行？先执行 ak500s run）：{TCP_ADDR}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    writeln!(s, "{line}").map_err(|e| e.to_string())?;
    let mut resp = String::new();
    BufReader::new(&mut s).read_line(&mut resp).map_err(|e| e.to_string())?;
    Ok(resp.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_shared() -> SharedHandle {
        Arc::new(Mutex::new(SharedState::default()))
    }

    #[test]
    fn ping_ok() {
        let r = dispatch(&new_shared(), r#"{"v":1,"cmd":"ping"}"#);
        assert_eq!(r["ok"], true);
    }

    #[test]
    fn show_basic_and_defaults() {
        let sh = new_shared();
        let r = dispatch(&sh, r#"{"cmd":"show","value":123}"#);
        assert_eq!(r["ok"], true);
        let g = sh.lock().unwrap();
        let f = g.external.as_ref().unwrap();
        assert_eq!(f.value, 123);
        assert_eq!(f.bar, Some(0)); // 缺省熄灭
        assert_eq!(f.mode_byte, protocol::MODE_USAGE); // 默认 pct
    }

    #[test]
    fn show_clamps_and_bar_usage() {
        let sh = new_shared();
        let r = dispatch(&sh, r#"{"cmd":"show","value":4567,"bar":"usage","unit":"c","alarm":true}"#);
        assert_eq!(r["ok"], true);
        let g = sh.lock().unwrap();
        let f = g.external.as_ref().unwrap();
        assert_eq!(f.value, 999);
        assert_eq!(f.bar, None);
        assert_eq!(f.mode_byte, protocol::MODE_TEMP_C);
        assert!(f.alarm);
    }

    #[test]
    fn show_validation_errors() {
        let sh = new_shared();
        assert_eq!(dispatch(&sh, r#"{"cmd":"show"}"#)["ok"], false); // 缺 value
        assert_eq!(dispatch(&sh, r#"{"cmd":"show","value":1,"bar":11}"#)["ok"], false);
        assert_eq!(dispatch(&sh, r#"{"cmd":"show","value":1,"unit":"k"}"#)["ok"], false);
        assert_eq!(dispatch(&sh, r#"{"cmd":"show","value":1,"bar":"fast"}"#)["ok"], false);
        assert_eq!(dispatch(&sh, r#"{"cmd":"bogus"}"#)["ok"], false);
        assert_eq!(dispatch(&sh, "not json")["ok"], false);
    }

    #[test]
    fn expiry_and_release() {
        let sh = new_shared();
        dispatch(&sh, r#"{"cmd":"show","value":42,"ttl_s":3600}"#);
        assert!(take_live_external(&sh).is_some());

        // 手动构造过期帧 → 应取不到且被清除
        sh.lock().unwrap().external = Some(ExternalFrame {
            mode_byte: 19, bar: Some(1), value: 1, alarm: false,
            expires: Instant::now() - Duration::from_secs(1),
        });
        assert!(take_live_external(&sh).is_none());
        assert!(sh.lock().unwrap().external.is_none());

        dispatch(&sh, r#"{"cmd":"show","value":42}"#);
        dispatch(&sh, r#"{"cmd":"release"}"#);
        assert!(sh.lock().unwrap().external.is_none());
        assert!(sh.lock().unwrap().mode_override.is_none());
    }

    #[test]
    fn mode_override() {
        let sh = new_shared();
        let r = dispatch(&sh, r#"{"cmd":"mode","mode":"auto"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(sh.lock().unwrap().mode_override, Some(Mode::Auto));
        assert_eq!(dispatch(&sh, r#"{"cmd":"mode","mode":"zzz"}"#)["ok"], false);
    }

    #[test]
    fn config_switches() {
        let sh = new_shared();
        // 部分设置：只关温度，占用率不动
        assert_eq!(dispatch(&sh, r#"{"cmd":"config","show_temp":false}"#)["ok"], true);
        {
            let g = sh.lock().unwrap();
            assert_eq!(g.show_temp_override, Some(false));
            assert_eq!(g.show_usage_override, None);
        }
        // 再开回来
        assert_eq!(dispatch(&sh, r#"{"cmd":"config","show_temp":true}"#)["ok"], true);
        assert_eq!(sh.lock().unwrap().show_temp_override, Some(true));
        // 空请求报错
        assert_eq!(dispatch(&sh, r#"{"cmd":"config"}"#)["ok"], false);
        // release 连同开关覆盖一起清理
        dispatch(&sh, r#"{"cmd":"config","show_usage":false}"#);
        dispatch(&sh, r#"{"cmd":"release"}"#);
        let g = sh.lock().unwrap();
        assert!(g.external.is_none());
        assert!(g.mode_override.is_none());
        assert!(g.show_temp_override.is_none());
        assert!(g.show_usage_override.is_none());
    }
}
