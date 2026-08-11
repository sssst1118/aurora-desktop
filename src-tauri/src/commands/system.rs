use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};

pub mod system_sampler;

/// 系统状态快照(2.5 命令返回与 `sys-status` 事件共用契约)。
/// 字段只增不改:`#[serde(default)]` 保证旧数据/旧前端缺新字段时兜底,保持向后兼容。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SysStatus {
    /// CPU 使用率 0.0-100.0(GetSystemTimes 相邻两次采样差商)
    pub cpu: f32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
    /// 聚合接收速率 bytes/s(新增)
    pub net_rx_bps: u64,
    /// 聚合发送速率 bytes/s(新增)
    pub net_tx_bps: u64,
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

/// 网络接口净字节采样(差商原料,来自 GetIfTable2 聚合后的 InOctets/OutOctets)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetSample {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// 单接口 octets 快照(过滤/聚合纯函数入参,与 Windows 结构解耦,便于单测)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IfOctets {
    /// 接口类型(MIB_IF_ROW2.Type)
    pub if_type: u32,
    /// 运行状态(MIB_IF_ROW2.OperStatus,IF_OPER_STATUS)
    pub oper_status: i32,
    pub rx: u64,
    pub tx: u64,
}

/// IF_TYPE_SOFTWARE_LOOPBACK(与 windows-sys IpHelper 常量一致)
pub const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
/// IF_OPER_STATUS_UP(与 windows-sys Ndis 的 NET_IF_OPER_STATUS_UP 一致,值 1)
pub const IF_OPER_STATUS_UP: i32 = 1;

/// 接口是否计入网络速率聚合:排除 loopback 与非 Up 接口
pub fn is_active_interface(if_type: u32, oper_status: i32) -> bool {
    if_type != IF_TYPE_SOFTWARE_LOOPBACK && oper_status == IF_OPER_STATUS_UP
}

/// 聚合所有有效接口的收发累计字节(无效接口直接跳过)
pub fn sum_active_octets(rows: &[IfOctets]) -> NetSample {
    let mut acc = NetSample::default();
    for r in rows {
        if is_active_interface(r.if_type, r.oper_status) {
            acc.rx_bytes = acc.rx_bytes.saturating_add(r.rx);
            acc.tx_bytes = acc.tx_bytes.saturating_add(r.tx);
        }
    }
    acc
}

/// 两次网络采样差商 → bps(字节/秒)。
/// 计数器回绕时 saturating_sub 归零(该周期记为 0,不 panic);dt 非正返回 0。
pub fn net_rate(prev: &NetSample, cur: &NetSample, dt_secs: f64) -> (u64, u64) {
    if dt_secs <= 0.0 {
        return (0, 0);
    }
    let rx = (cur.rx_bytes.saturating_sub(prev.rx_bytes) as f64 / dt_secs) as u64;
    let tx = (cur.tx_bytes.saturating_sub(prev.tx_bytes) as f64 / dt_secs) as u64;
    (rx, tx)
}

// ---- 共享快照(采样线程写入,命令读取) ----

static SNAPSHOT: OnceLock<Arc<Mutex<Option<SysStatus>>>> = OnceLock::new();

pub(crate) fn set_snapshot(status: SysStatus) {
    let lock = SNAPSHOT.get_or_init(|| Arc::new(Mutex::new(None)));
    if let Ok(mut g) = lock.lock() {
        *g = Some(status);
    }
}

pub(crate) fn get_snapshot() -> SysStatus {
    let lock = SNAPSHOT.get_or_init(|| Arc::new(Mutex::new(None)));
    lock.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

/// 实时系统状态命令:保持向后兼容(返回全量字段,旧前端忽略新增字段)。
/// 读取常驻采样线程的最近快照(不再命令内 sleep 采样);首次调用幂等触发
/// 采样线程懒启动(集成 agent 亦可在 setup 调 ensure_started,Once 保证只启一次)。
#[tauri::command]
pub fn sys_get_status(app: tauri::AppHandle) -> SysStatus {
    system_sampler::ensure_started(&app);
    get_snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- CPU ----

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

    // ---- 网络 ----

    #[test]
    fn net_rate_basic() {
        let prev = NetSample { rx_bytes: 1_000_000, tx_bytes: 500_000 };
        let cur = NetSample { rx_bytes: 2_000_000, tx_bytes: 1_000_000 };
        let (rx, tx) = net_rate(&prev, &cur, 1.0);
        assert_eq!((rx, tx), (1_000_000, 500_000));
    }

    #[test]
    fn net_rate_fractional_interval() {
        let prev = NetSample::default();
        let cur = NetSample { rx_bytes: 1_500_000, tx_bytes: 0 };
        let (rx, tx) = net_rate(&prev, &cur, 2.0);
        assert_eq!((rx, tx), (750_000, 0));
    }

    #[test]
    fn net_rate_zero_dt() {
        let prev = NetSample { rx_bytes: 100, tx_bytes: 100 };
        let cur = NetSample { rx_bytes: 200, tx_bytes: 200 };
        assert_eq!(net_rate(&prev, &cur, 0.0), (0, 0));
    }

    #[test]
    fn net_rate_counter_wrap_is_zero() {
        // 计数器回绕(新值小于旧值):saturating_sub 归零,不 panic
        let prev = NetSample { rx_bytes: u64::MAX, tx_bytes: u64::MAX };
        let cur = NetSample::default();
        assert_eq!(net_rate(&prev, &cur, 1.0), (0, 0));
    }

    #[test]
    fn active_interface_filter() {
        // 非 loopback 且 Up → 计入
        assert!(is_active_interface(6, IF_OPER_STATUS_UP));
        // loopback 即使 Up 也不计
        assert!(!is_active_interface(IF_TYPE_SOFTWARE_LOOPBACK, IF_OPER_STATUS_UP));
        // 非 Up 不计
        assert!(!is_active_interface(6, 0));
        assert!(!is_active_interface(6, 2));
        assert!(!is_active_interface(IF_TYPE_SOFTWARE_LOOPBACK, 0));
    }

    #[test]
    fn sum_octets_mixed() {
        let rows = [
            IfOctets { if_type: 6, oper_status: IF_OPER_STATUS_UP, rx: 1000, tx: 2000 },
            IfOctets { if_type: IF_TYPE_SOFTWARE_LOOPBACK, oper_status: IF_OPER_STATUS_UP, rx: 999_999, tx: 999_999 },
            IfOctets { if_type: 71, oper_status: 2, rx: 888_888, tx: 888_888 }, // Down 不计
            IfOctets { if_type: 71, oper_status: IF_OPER_STATUS_UP, rx: 500, tx: 700 },
        ];
        let s = sum_active_octets(&rows);
        assert_eq!(s, NetSample { rx_bytes: 1500, tx_bytes: 2700 });
    }

    #[test]
    fn sum_octets_empty() {
        assert_eq!(sum_active_octets(&[]), NetSample::default());
    }
}
