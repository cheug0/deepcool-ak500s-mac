//! 配置：CLI 参数 > 配置文件 > 默认值。
//! 配置文件为极简 key=value 文本，路径 ~/.config/ak500s-mac/config
//! （刻意不引入 toml/serde，减小构建面）。

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Temp,
    Usage,
    Auto,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    /// 温度单位：true=℉
    pub fahrenheit: bool,
    /// 刷新周期毫秒（必须 < 屏幕看门狗超时，实测数秒）
    pub interval_ms: u64,
    /// 报警阈值（℃）
    pub alarm_c: f32,
    /// auto 模式下单个子模式停留秒数
    pub auto_switch_s: u64,
    /// 内置采集指标显示开关（false=该指标不渲染；全关=纯 API 屏幕模式）
    pub show_temp: bool,
    pub show_usage: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Auto,
            fahrenheit: false,
            interval_ms: 1000,
            alarm_c: 90.0,
            auto_switch_s: 10,
            show_temp: true,
            show_usage: true,
        }
    }
}

impl Config {
    /// 读取配置文件（存在才读，不存在用默认值），再套用 CLI 覆盖项。
    pub fn load(cli_overrides: CliOverrides) -> Self {
        let text = read_config_file();
        Self::load_from(text.as_deref(), cli_overrides)
    }

    /// 指定配置文本加载（测试用，避免环境变量在并行测试中互相干扰）。
    fn load_from(file_text: Option<&str>, cli_overrides: CliOverrides) -> Self {
        let mut cfg = Config::default();
        if let Some(text) = file_text {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    apply_kv(&mut cfg, k.trim(), v.trim());
                }
            }
        }
        if let Some(m) = cli_overrides.mode {
            cfg.mode = m;
        }
        if let Some(f) = cli_overrides.fahrenheit {
            cfg.fahrenheit = f;
        }
        if let Some(i) = cli_overrides.interval_ms {
            cfg.interval_ms = i;
        }
        if let Some(a) = cli_overrides.alarm_c {
            cfg.alarm_c = a;
        }
        if let Some(s) = cli_overrides.auto_switch_s {
            cfg.auto_switch_s = s;
        }
        if let Some(t) = cli_overrides.show_temp {
            cfg.show_temp = t;
        }
        if let Some(u) = cli_overrides.show_usage {
            cfg.show_usage = u;
        }
        // 看门狗保护：周期过长屏幕会掉回默认状态
        if cfg.interval_ms > 3000 {
            cfg.interval_ms = 3000;
        }
        if cfg.interval_ms < 300 {
            cfg.interval_ms = 300;
        }
        cfg
    }
}

fn apply_kv(cfg: &mut Config, k: &str, v: &str) {
    match k {
        "mode" => {
            cfg.mode = match v {
                "temp" | "temperature" => Mode::Temp,
                "usage" => Mode::Usage,
                "auto" => Mode::Auto,
                _ => cfg.mode,
            }
        }
        "unit" => cfg.fahrenheit = matches!(v, "f" | "F" | "fahrenheit"),
        "fahrenheit" => cfg.fahrenheit = v == "true",
        "interval_ms" => {
            if let Ok(n) = v.parse() {
                cfg.interval_ms = n;
            }
        }
        "alarm_c" => {
            if let Ok(n) = v.parse() {
                cfg.alarm_c = n;
            }
        }
        "auto_switch_s" => {
            if let Ok(n) = v.parse() {
                cfg.auto_switch_s = n;
            }
        }
        "show_temp" => cfg.show_temp = v == "true" || v == "1" || v == "on",
        "show_usage" => cfg.show_usage = v == "true" || v == "1" || v == "on",
        _ => {}
    }
}

fn read_config_file() -> Option<String> {
    let path = config_path()?;
    fs::read_to_string(path).ok()
}

pub fn config_path() -> Option<PathBuf> {
    dirs_config().map(|d| d.join("config"))
}

fn dirs_config() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Some(PathBuf::from(x).join("ak500s-mac"));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("ak500s-mac"))
}

/// CLI 传入的覆盖项（None = 不覆盖）
#[derive(Debug, Default, Clone, Copy)]
pub struct CliOverrides {
    pub mode: Option<Mode>,
    pub fahrenheit: Option<bool>,
    pub interval_ms: Option<u64>,
    pub alarm_c: Option<f32>,
    pub auto_switch_s: Option<u64>,
    pub show_temp: Option<bool>,
    pub show_usage: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_with(text: &str) -> Config {
        Config::load_from(Some(text), CliOverrides::default())
    }

    #[test]
    fn defaults_and_unknown_keys() {
        let c = load_with("# 注释\nunknown=1\n");
        assert!(c.show_temp && c.show_usage);
        assert_eq!(c.mode, Mode::Auto); // 默认 auto（温度/占用率轮换）
    }

    #[test]
    fn show_switches() {
        let c = load_with("show_temp=false\nshow_usage=off\nmode=usage\n");
        assert!(!c.show_temp);
        assert!(!c.show_usage);
        assert_eq!(c.mode, Mode::Usage);
    }

    #[test]
    fn show_switches_on_forms() {
        let c = load_with("show_temp=1\nshow_usage=on\n");
        assert!(c.show_temp && c.show_usage);
    }

    #[test]
    fn cli_overrides_win() {
        let cfg = Config::load(CliOverrides {
            show_temp: Some(false),
            interval_ms: Some(750),
            ..Default::default()
        });
        assert!(!cfg.show_temp);
        assert_eq!(cfg.interval_ms, 750);
    }
}
