//! 主循环：采集 → 组包 → 写屏幕 → 异常恢复。
//!
//! 关键行为（均来自实机验证结论，见 docs/02 §5）：
//! * 屏幕有看门狗：必须周期刷新（interval 300–3000ms，默认 1s）
//! * 写失败/设备拔出 → 外层重连循环自动恢复（也覆盖睡眠唤醒场景）
//! * 无 SMC 温度源 → 自动降级为"仅占用率"模式
//! * API 优先（docs/09）：外部帧有效期内抢占前台，到期回落内置模式

use crate::api::{self, SharedHandle, SharedState};
use crate::config::{Config, Mode};
use crate::device::{self, Display};
use crate::protocol;
use crate::sensor::temp;
use crate::sensor::usage::CpuUsage;
use hidapi::HidApi;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn run(cfg: Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut api_h = HidApi::new()?;

    // API 共享状态 + 本机监听（外部软件入口，docs/09）
    let shared: SharedHandle = Arc::new(Mutex::new(SharedState::default()));
    api::spawn(shared.clone()).map_err(|e| format!("API 监听启动失败: {e}"))?;
    #[cfg(unix)]
    eprintln!("[信息] API 已监听: {}（NDJSON，见 docs/09）", api::SOCKET_PATH);
    #[cfg(windows)]
    eprintln!("[信息] API 已监听: tcp/{}（开发用，见 docs/09）", api::TCP_ADDR);

    // 温度键探测（失败则降级为 usage 模式）
    let temp_key = temp::probe_key();
    let mut cfg = cfg;
    if temp_key.is_none() {
        eprintln!("[警告] 读不到 CPU 温度（SMC 键探测全部失败）。");
        eprintln!("       黑苹果需安装 VirtualSMC + SMCProcessor（AMD 平台用 SMCAMDProcessor）。");
        match cfg.mode {
            Mode::Temp => {
                eprintln!("       已自动降级为占用率模式。");
                cfg.mode = Mode::Usage;
            }
            Mode::Auto => {
                eprintln!("       auto 模式下温度半场不可用，实际将只显示占用率。");
            }
            Mode::Usage => {}
        }
    } else {
        eprintln!("[信息] 温度键: {}（℃）", temp_key.unwrap());
    }
    eprintln!(
        "[信息] 内置指标显示: 温度={} 占用率={}（API config 命令可运行时切换）",
        if cfg.show_temp { "开" } else { "关" },
        if cfg.show_usage { "开" } else { "关" }
    );
    if !cfg.show_temp && !cfg.show_usage {
        eprintln!("[信息] 内置指标已全关：进入纯 API 屏幕模式（仅外部 show 帧可见）。");
    }

    // CPU 占用率先做一次预采样
    let mut cpu = CpuUsage::new();
    let _ = cpu.sample();
    sleep(Duration::from_millis(300));

    // 外层：设备发现/重连循环（热插拔、睡眠唤醒恢复）
    loop {
        // 刷新设备表失败不退出守护进程（唤醒瞬间偶发）：记日志稍后重试
        if let Err(e) = api_h.refresh_devices() {
            eprintln!("[警告] 设备表刷新失败: {e}，3 秒后重试…");
        }
        match device::find(&api_h) {
            Some(disp) => {
                eprintln!(
                    "[信息] 已连接 {}{}，模式 {:?}，周期 {}ms",
                    disp.product,
                    if disp.se { "（SE 变体）" } else { "" },
                    cfg.mode,
                    cfg.interval_ms
                );
                let _ = disp.write(&protocol::init_packet(disp.se)); // 加载动画
                if let Err(e) = display_loop(&disp, &cfg, temp_key, &mut cpu, &shared) {
                    eprintln!("[信息] 设备写入中断: {e}，3 秒后重连…");
                }
            }
            None => {
                eprintln!("[信息] 未发现 AK500S（{:04x}:{:04x}），3 秒后重试…", protocol::VID, protocol::PID);
            }
        }
        sleep(Duration::from_secs(3));
    }
}

/// 内层：连上设备后的刷新循环，直到写失败（设备拔出/休眠）返回 Err。
fn display_loop(
    disp: &Display,
    cfg: &Config,
    temp_key: Option<&'static str>,
    cpu: &mut CpuUsage,
    shared: &SharedHandle,
) -> Result<(), hidapi::HidError> {
    let interval = Duration::from_millis(cfg.interval_ms);
    let auto_switch = Duration::from_secs(cfg.auto_switch_s.max(1));
    // auto 模式当前子模式（温度 ↔ 占用率轮换）
    let mut auto_show_temp = true;
    let mut auto_last_switch = Instant::now();

    loop {
        let usage = cpu.sample();
        let temp_c = temp_key.and_then(|k| temp::read_temp_c(k).ok());

        // 每帧先读一次共享覆盖（API 的 config/mode 命令；均为微秒级临界区）
        let (mode_override, show_temp_ov, show_usage_ov) = {
            let g = shared.lock().unwrap();
            (g.mode_override, g.show_temp_override, g.show_usage_override)
        };
        let show_temp_on = show_temp_ov.unwrap_or(cfg.show_temp);
        let show_usage_on = show_usage_ov.unwrap_or(cfg.show_usage);

        // 1) 外部帧（API 发布）有效 → 抢占前台
        if let Some(ext) = api::take_live_external(shared) {
            let bar = ext.bar.unwrap_or_else(|| protocol::bar_from_usage(usage));
            let pkt = protocol::packet(ext.mode_byte, bar, ext.value, ext.alarm, disp.se);
            disp.write(&pkt)?;
            sleep(interval);
            continue;
        }

        // 2) 内置采集指标渲染（开关可来自配置文件/CLI 或 API config 命令）
        let eff_mode = mode_override.unwrap_or(cfg.mode);
        // 本帧模式属意的指标：temp/usage/auto 半场
        let prefer_temp = match eff_mode {
            Mode::Temp => true,
            Mode::Usage => false,
            Mode::Auto => auto_show_temp,
        };
        // 温度侧可行 = 开关开 且 本帧真的采到了温度
        let temp_ok = show_temp_on && temp_c.is_some();
        // 决策：属意指标可用则用它；被关/不可用时回落另一侧；两侧皆不可 → 心跳空帧
        let render_temp = if prefer_temp { temp_ok } else { !show_usage_on && temp_ok };
        let render_usage = !render_temp && show_usage_on;

        let (mode_byte, value, alarm) = if render_temp {
            let t = temp_c.unwrap();
            let (v, m) = if cfg.fahrenheit {
                (t * 9.0 / 5.0 + 32.0, protocol::MODE_TEMP_F)
            } else {
                (t, protocol::MODE_TEMP_C)
            };
            (m, v.round().clamp(0.0, 999.0) as u16, t >= cfg.alarm_c)
        } else if render_usage {
            (protocol::MODE_USAGE, usage.round().clamp(0.0, 999.0) as u16, false)
        } else {
            // 3) 纯 API 模式（内置指标全关或不可用）：本帧不发送任何报文，
            //    屏幕由固件看门狗自动回默认状态；外部 show 帧仍会正常渲染（见上）。
            //    注：不发送意味着发现不了设备拔出——等外部帧到来时写失败再重连。
            sleep(interval);
            continue;
        };

        let pkt = protocol::packet(mode_byte, protocol::bar_from_usage(usage), value, alarm, disp.se);
        disp.write(&pkt)?;

        if eff_mode == Mode::Auto && auto_last_switch.elapsed() >= auto_switch {
            auto_show_temp = !auto_show_temp;
            auto_last_switch = Instant::now();
        }
        sleep(interval);
    }
}
