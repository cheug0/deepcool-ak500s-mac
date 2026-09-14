//! AK500S DIGITAL 屏幕报文协议（依据 docs/02-通信协议规格.md，已实机验证）
//!
//! 标准版 64 字节 Output Report：
//!   D0=16(Report ID) D1=模式 D2=状态条 D3..D5=百十个位 D6=报警 D7..=0
//! SE 变体不带 Report ID，全部字段左移一位（本机为标准版，保留以防批次差异）。

pub const VID: u16 = 0x3633;
pub const PID: u16 = 0x0004;
pub const REPORT_LEN: usize = 64;
pub const REPORT_ID: u8 = 16;

pub const MODE_LOADING: u8 = 170; // 初始化/加载动画
pub const MODE_TEMP_C: u8 = 19; // 温度 ℃
pub const MODE_TEMP_F: u8 = 35; // 温度 ℉
pub const MODE_USAGE: u8 = 76; // 占用率 %

/// 按协议组一帧显示报文。
///
/// * `mode` - 模式字节（MODE_TEMP_C / MODE_TEMP_F / MODE_USAGE / MODE_LOADING）
/// * `bar`  - 状态条档位 1–10（0 = 关闭）
/// * `value` - 0–999 的待显数值，按位拆分
/// * `alarm` - 超温报警位
/// * `se`   - SE 变体则不带 Report ID（字段整体左移一位）
pub fn packet(mode: u8, bar: u8, value: u16, alarm: bool, se: bool) -> [u8; REPORT_LEN] {
    let (h, t, u) = digits(value);
    let mut d = [0u8; REPORT_LEN];
    if se {
        d[0] = mode;
        d[1] = bar;
        d[2] = h;
        d[3] = t;
        d[4] = u;
        d[5] = alarm as u8;
    } else {
        d[0] = REPORT_ID;
        d[1] = mode;
        d[2] = bar;
        d[3] = h;
        d[4] = t;
        d[5] = u;
        d[6] = alarm as u8;
    }
    d
}

/// 初始化帧（加载动画），连接后首先发送。
pub fn init_packet(se: bool) -> [u8; REPORT_LEN] {
    packet(MODE_LOADING, 0, 0, false, se)
}

/// 占用率 → 状态条档位（参考实现：<15% 记 1 档，否则四舍五入，封顶 10）。
pub fn bar_from_usage(usage: f32) -> u8 {
    if usage < 15.0 {
        1
    } else {
        (usage / 10.0).round().clamp(1.0, 10.0) as u8
    }
}

fn digits(v: u16) -> (u8, u8, u8) {
    let v = v.min(999);
    (((v / 100) % 10) as u8, ((v / 10) % 10) as u8, (v % 10) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_packet_layout() {
        let p = packet(MODE_TEMP_C, 5, 47, false, false);
        assert_eq!(p[0], 16);
        assert_eq!(p[1], 19);
        assert_eq!(p[2], 5);
        assert_eq!(&p[3..6], &[0, 4, 7]);
        assert_eq!(p[6], 0);
        assert!(p[7..].iter().all(|&b| b == 0));
    }

    #[test]
    fn se_packet_shifted() {
        let p = packet(MODE_USAGE, 6, 62, true, true);
        assert_eq!(p[0], 76);
        assert_eq!(p[1], 6);
        assert_eq!(&p[2..5], &[0, 6, 2]);
        assert_eq!(p[5], 1);
    }

    #[test]
    fn three_digit_clamp() {
        // 超过 999 钳位到 999
        let p = packet(MODE_TEMP_C, 10, 1234, false, false);
        assert_eq!(&p[3..6], &[9, 9, 9]);
        // 三位数正常拆分
        let p = packet(MODE_TEMP_C, 10, 234, false, false);
        assert_eq!(&p[3..6], &[2, 3, 4]);
    }

    #[test]
    fn bar_mapping() {
        assert_eq!(bar_from_usage(0.0), 1);
        assert_eq!(bar_from_usage(14.9), 1);
        assert_eq!(bar_from_usage(55.0), 6);
        assert_eq!(bar_from_usage(100.0), 10);
        assert_eq!(bar_from_usage(250.0), 10);
    }
}
