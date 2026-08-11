use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
pub struct SysStatus {
    pub cpu: f32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CpuSample {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

/// 两次采样差商得到 CPU 使用率(0.0-100.0)
/// Windows 的 GetSystemTimes:kernel 时间**包含** idle 时间,
/// 所以 total = user + kernel,busy = total - idle。
pub fn cpu_percent(prev: &CpuSample, cur: &CpuSample) -> f32 {
    let dt_total = (cur.kernel + cur.user).saturating_sub(prev.kernel + prev.user);
    let dt_idle = cur.idle.saturating_sub(prev.idle);
    if dt_total == 0 {
        return 0.0;
    }
    let busy = dt_total.saturating_sub(dt_idle) as f32 / dt_total as f32;
    (busy * 100.0).clamp(0.0, 100.0)
}

/// 实时系统状态:GetSystemTimes 两次采样差商算 CPU,GlobalMemoryStatusEx 取内存
#[tauri::command]
pub fn sys_get_status() -> SysStatus {
    sample_sys_status()
}

fn sample_sys_status() -> SysStatus {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    unsafe {
        let mut idle1: FILETIME = std::mem::zeroed();
        let mut kernel1: FILETIME = std::mem::zeroed();
        let mut user1: FILETIME = std::mem::zeroed();
        GetSystemTimes(&mut idle1, &mut kernel1, &mut user1);
        std::thread::sleep(Duration::from_millis(120));
        let mut idle2: FILETIME = std::mem::zeroed();
        let mut kernel2: FILETIME = std::mem::zeroed();
        let mut user2: FILETIME = std::mem::zeroed();
        GetSystemTimes(&mut idle2, &mut kernel2, &mut user2);

        fn ft(t: &FILETIME) -> u64 {
            ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
        }
        let prev = CpuSample {
            idle: ft(&idle1),
            kernel: ft(&kernel1),
            user: ft(&user1),
        };
        let cur = CpuSample {
            idle: ft(&idle2),
            kernel: ft(&kernel2),
            user: ft(&user2),
        };
        let cpu = cpu_percent(&prev, &cur);

        let mut ms: MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        GlobalMemoryStatusEx(&mut ms);
        SysStatus {
            cpu,
            mem_used_mb: (ms.ullTotalPhys - ms.ullAvailPhys) / (1024 * 1024),
            mem_total_mb: ms.ullTotalPhys / (1024 * 1024),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_basic() {
        let prev = CpuSample { idle: 100, kernel: 200, user: 100 };
        let cur = CpuSample { idle: 150, kernel: 250, user: 150 };
        // 增:total=100, idle=50, busy=50 → 50%
        assert!((cpu_percent(&prev, &cur) - 50.0).abs() < 0.001);
    }

    #[test]
    fn cpu_percent_zero_delta() {
        let s = CpuSample::default();
        assert_eq!(cpu_percent(&s, &s), 0.0);
    }

    #[test]
    fn cpu_percent_clamped() {
        let prev = CpuSample { idle: 0, kernel: 0, user: 0 };
        let cur = CpuSample { idle: 0, kernel: 10, user: 0 };
        let v = cpu_percent(&prev, &cur);
        assert!(v >= 0.0 && v <= 100.0);
    }

    #[test]
    fn cpu_percent_idle_only_is_zero() {
        // 只有 idle 增加 → 0%
        let prev = CpuSample { idle: 100, kernel: 100, user: 100 };
        let cur = CpuSample { idle: 200, kernel: 200, user: 100 };
        assert_eq!(cpu_percent(&prev, &cur), 0.0);
    }
}
