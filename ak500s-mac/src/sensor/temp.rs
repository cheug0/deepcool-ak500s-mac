//! CPU 温度：macOS 读 SMC（IOKit "AppleSMC" user client，参照 osx-cpu-temp 的读取方法）。
//!
//! 黑苹果注意：温度键由 VirtualSMC + SMCProcessor（Intel）/ SMCAMDProcessor（AMD）提供，
//! 缺 kext 时 read 返回 Err → runner 自动降级为"仅占用率"模式。

/// SMC 键探测链：按序尝试，取第一个返回有效值（0–120℃）的键。
pub const KEY_CHAIN: &[&str] = &[
    "TC0P", // CPU 包温（最常见）
    "TC0C", "TC1C", "TC2C", "TC3C", "TC4C", "TC5C", "TC6C", "TC7C", // 核心温（取先命中者）
    "TCAD", // 部分平台
    "Tp01", // PECI（旧 Intel）
    "TC0H", // 供热管
];

/// 读一次 CPU 温度（℃）。成功返回 >0.0–120.0。
/// 0.0 一律视为坏读数（部分黑苹果 kext 对不存在的传感器常返回 0），
/// 让探测链继续尝试下一键，避免屏幕常显 0℃。
pub fn read_temp_c(key: &str) -> Result<f32, String> {
    smc::read_key(key)
        .and_then(|(dtype, bytes)| decode(dtype, &bytes))
        .and_then(|v| {
            if v > 0.0 && v <= 120.0 {
                Ok(v)
            } else {
                Err(format!("数值越界: {v}℃"))
            }
        })
}

/// 按探测链找可用键（doctor 与启动自检用）。
pub fn probe_key() -> Option<&'static str> {
    KEY_CHAIN.iter().copied().find(|k| read_temp_c(k).is_ok())
}

fn decode(dtype: u32, bytes: &[u8]) -> Result<f32, String> {
    match &dtype.to_be_bytes() {
        // "sp78"：有符号整数 + 1/256 小数（SMC 温度最常见类型）
        b"sp78" if bytes.len() >= 2 => {
            let v = bytes[0] as i8 as f32 + bytes[1] as f32 / 256.0;
            Ok(v)
        }
        // "flt "：IEEE754 单精度小端
        b"flt " if bytes.len() >= 4 => Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        // "fpe2"：定点 ×100 小端
        b"fpe2" if bytes.len() >= 2 => {
            let raw = u16::from_le_bytes([bytes[0], bytes[1]]);
            Ok(raw as f32 / 100.0)
        }
        _ => Err(format!("未知数据类型 0x{dtype:08x}")),
    }
}

#[cfg(target_os = "macos")]
pub mod smc {
    //! IOKit SMC user client 的最小 FFI（协议照抄 osx-cpu-temp/smc.c，devnull 原始实现）。
    //!
    //! 关键事实（本次审查核对）：
    //! * 输入/输出是**同一个** 80 字节结构 SMCKeyData_t；
    //! * IOConnectCallStructMethod 的选择器固定为 KERNEL_INDEX_SMC(2)；
    //! * 命令码放在结构体的 data8 字段：9=读键信息，5=读键值。

    use std::ffi::CString;

    const KIO_RETURN_SUCCESS: i32 = 0;
    const APPLE_SMC: &str = "AppleSMC";
    const KERNEL_INDEX_SMC: u32 = 2;
    const SMC_CMD_READ_KEYINFO: u8 = 9;
    const SMC_CMD_READ_BYTES: u8 = 5;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct PLimit {
        version: u16,
        length: u16,
        cpu: u32,
        gpu: u32,
        mem: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct KeyInfo {
        data_size: u32,
        data_type: u32,
        data_attributes: u8,
    }

    /// 与 C 端 SMCKeyData_t 逐字段对应（80 字节）。
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SmcKeyData {
        key: u32,          // @0
        vers: [u8; 6],     // @4  char[4] + UInt16 release
        p_limit: PLimit,   // @12 u16,u16,u32,u32,u32 = 16 字节
        key_info: KeyInfo, // @28 u32,u32,char = 12 字节
        result: u8,        // @40
        status: u8,        // @41
        data8: u8,         // @42 ← 命令码在这里
        _pad: u8,          // @43
        data32: u32,       // @44
        bytes: [u8; 32],   // @48
    }

    impl Default for SmcKeyData {
        fn default() -> Self {
            // 全零安全：无 padding 之外的非平凡初始化
            unsafe { std::mem::zeroed() }
        }
    }

    // 布局守护：与 C 结构失配直接编译失败（防止未来重构再引入静默错位）
    const _: () = assert!(std::mem::size_of::<SmcKeyData>() == 80);

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const std::os::raw::c_char) -> *mut std::ffi::c_void;
        fn IOServiceGetMatchingService(
            master_port: u32,
            matching: *mut std::ffi::c_void,
        ) -> u32;
        fn IOServiceOpen(
            service: u32,
            owner_task: u32,
            conn_type: u32,
            connection: *mut u32,
        ) -> i32;
        fn IOServiceClose(connection: u32) -> i32;
        fn IOObjectRelease(obj: u32) -> i32;
        fn IOConnectCallStructMethod(
            connection: u32,
            selector: u32,
            input: *const std::ffi::c_void,
            input_cnt: usize,
            output: *mut std::ffi::c_void,
            output_cnt: *mut usize,
        ) -> i32;
    }

    extern "C" {
        fn mach_task_self() -> u32;
    }

    struct SmcConn(u32);

    impl SmcConn {
        fn open() -> Result<Self, String> {
            unsafe {
                let name = CString::new(APPLE_SMC).map_err(|e| e.to_string())?;
                let dict = IOServiceMatching(name.as_ptr());
                if dict.is_null() {
                    return Err("IOServiceMatching 失败".into());
                }
                let service = IOServiceGetMatchingService(0 /* kIOMasterPortDefault */, dict);
                if service == 0 {
                    return Err("找不到 AppleSMC 服务（黑苹果需 VirtualSMC.kext）".into());
                }
                let mut conn: u32 = 0;
                let kr = IOServiceOpen(service, mach_task_self(), 0, &mut conn);
                IOObjectRelease(service);
                if kr != KIO_RETURN_SUCCESS {
                    return Err(format!("IOServiceOpen 失败 kr={kr}"));
                }
                Ok(Self(conn))
            }
        }

        fn call(&self, input: &SmcKeyData) -> Result<SmcKeyData, String> {
            let mut output = SmcKeyData::default();
            let mut out_len = std::mem::size_of::<SmcKeyData>();
            let kr = unsafe {
                IOConnectCallStructMethod(
                    self.0,
                    KERNEL_INDEX_SMC,
                    input as *const SmcKeyData as *const std::ffi::c_void,
                    std::mem::size_of::<SmcKeyData>(),
                    &mut output as *mut SmcKeyData as *mut std::ffi::c_void,
                    &mut out_len,
                )
            };
            if kr != KIO_RETURN_SUCCESS {
                return Err(format!("SMC 调用失败 kr=0x{kr:x}"));
            }
            Ok(output)
        }

        fn read_key(&self, key: &str) -> Result<(u32, [u8; 32]), String> {
            let mut input = SmcKeyData::default();
            input.key = pack_key(key)?;

            // 第一步：读键信息（长度/类型），命令码 9
            input.data8 = SMC_CMD_READ_KEYINFO;
            let out1 = self.call(&input)?;
            let data_type = out1.key_info.data_type; // 类型取自本次输出（照抄参考实现）

            // 第二步：读键值，命令码 5；dataSize 回填进输入结构
            input.data8 = SMC_CMD_READ_BYTES;
            input.key_info.data_size = out1.key_info.data_size;
            let out2 = self.call(&input)?;

            Ok((data_type, out2.bytes))
        }
    }

    impl Drop for SmcConn {
        fn drop(&mut self) {
            // 必须用 IOServiceClose 关闭 user client 连接；
            // IOObjectRelease 只减引用计数，1s 一读会泄漏内核句柄。
            unsafe { IOServiceClose(self.0) };
        }
    }

    fn pack_key(key: &str) -> Result<u32, String> {
        let b = key.as_bytes();
        if b.len() != 4 {
            return Err(format!("SMC 键必须 4 字符: {key}"));
        }
        Ok((b[0] as u32) << 24 | (b[1] as u32) << 16 | (b[2] as u32) << 8 | b[3] as u32)
    }

    /// 读取一个 SMC 键，返回 (dataType 四字符码, 32 字节原始值)。
    /// 连接按进程缓存复用（OnceLock），避免每秒开关一次 user client。
    pub fn read_key(key: &str) -> Result<(u32, [u8; 32]), String> {
        static CONN: std::sync::OnceLock<Result<SmcConn, String>> = std::sync::OnceLock::new();
        let conn = match CONN.get_or_init(|| SmcConn::open()) {
            Ok(c) => c,
            Err(e) => return Err(e.clone()),
        };
        conn.read_key(key)
    }
}

#[cfg(not(target_os = "macos"))]
pub mod smc {
    //! 非 macOS 平台的占位实现（本工程在 Windows 上也能编译运行除温度外的功能）。
    pub fn read_key(_key: &str) -> Result<(u32, [u8; 32]), String> {
        Err("SMC 温度仅在 macOS 上可用".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_sp78() {
        let dtype = u32::from_be_bytes(*b"sp78");
        assert_eq!(decode(dtype, &[47, 0, 0, 0]).unwrap(), 47.0);
        assert_eq!(decode(dtype, &[47, 128, 0, 0]).unwrap(), 47.5);
        assert_eq!(decode(dtype, &[0x80, 0, 0, 0]).unwrap(), -128.0); // 极端负值可判越界
        assert_eq!(decode(dtype, &[0xFF, 0, 0, 0]).unwrap(), -1.0);
        assert_eq!(decode(dtype, &[0xFF, 0x80, 0, 0]).unwrap(), -0.5);
    }

    #[test]
    fn decode_flt() {
        let dtype = u32::from_be_bytes(*b"flt ");
        let b = 47.5f32.to_le_bytes();
        assert_eq!(decode(dtype, &[b[0], b[1], b[2], b[3]]).unwrap(), 47.5);
    }
}
