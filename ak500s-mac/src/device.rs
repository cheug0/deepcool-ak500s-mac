//! 设备发现与打开（hidapi，macOS 走 IOKit HID 后端，免 root）。

use crate::protocol::{PID, VID};
use hidapi::{HidApi, HidDevice};

pub struct Display {
    pub device: HidDevice,
    /// SE 变体（产品名不以 "AK" 开头；报文不带 Report ID）
    pub se: bool,
    pub product: String,
}

impl Display {
    pub fn write(&self, data: &[u8; 64]) -> Result<(), hidapi::HidError> {
        self.device.write(data)?;
        Ok(())
    }
}

/// 在已枚举的设备表里找 AK500S（VID/PID 匹配）。
/// 找不到返回 None；调用方应 HidApi::refresh_devices() 后重试（热插拔）。
pub fn find(api: &HidApi) -> Option<Display> {
    for info in api.device_list() {
        if info.vendor_id() == VID && info.product_id() == PID {
            let product = info
                .product_string()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let se = !product.starts_with("AK");
            let device = api.open(VID, PID).ok()?;
            return Some(Display { device, se, product });
        }
    }
    None
}

/// 列出所有匹配设备的信息（doctor 用），返回 (product, serial, usage_page, usage)。
pub fn list_all(api: &HidApi) -> Vec<(String, String, u16, u16)> {
    api.device_list()
        .filter(|i| i.vendor_id() == VID && i.product_id() == PID)
        .map(|i| {
            (
                i.product_string().unwrap_or("").to_string(),
                i.serial_number().unwrap_or("").to_string(),
                i.usage_page(),
                i.usage(),
            )
        })
        .collect()
}
