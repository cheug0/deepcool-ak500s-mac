//! CPU 占用率（sysinfo，macOS 内部走 host_processor_info，差分计算）。

use sysinfo::System;

pub struct CpuUsage {
    sys: System,
}

impl CpuUsage {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu();
        Self { sys }
    }

    /// 取一次占用率（0–100）。两次采样需间隔 ≥200ms 才有有效差分，
    /// 首次调用返回 0。runner 的主循环周期（≥750ms）天然满足。
    pub fn sample(&mut self) -> f32 {
        self.sys.refresh_cpu();
        self.sys.global_cpu_info().cpu_usage()
    }
}
