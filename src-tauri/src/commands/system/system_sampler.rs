//! 2.5 系统状态常驻采样线程:每 2s 采样 CPU/内存/网络一次,
//! 更新共享快照(commands/system.rs)并广播 `sys-status` 事件。
//!
//! 说明:windows-sys 0.59 中 GetIfTable2/MIB_IF_TABLE2/MIB_IF_ROW2 被
//! `Win32_NetworkManagement_Ndis` feature 门控,而 Cargo.toml(集成 agent 所有)
//! 仅含 IpHelper feature,故此处手写等价 FFI 声明,布局与 windows-sys 0.59 源码
//! 逐字段一致(勿自行调整)。
//! 【需实机验证】GetIfTable2 系统调用本身(接口枚举、InOctets/OutOctets 计数、
//! 虚拟网卡聚合口径)尚未在实机上跑通验证。

#![allow(non_snake_case)]

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use super::{
    cpu_percent, net_rate, set_snapshot, sum_active_octets, CpuSample, IfOctets, NetSample,
    SysStatus,
};

/// 采样周期:固定 2s 一次(与设计一致,轮询节制,不做配置化)
const SAMPLING_INTERVAL: Duration = Duration::from_secs(2);

// ---- 网络接口表 FFI(iphlpapi.dll,布局照抄 windows-sys 0.59 IpHelper/Ndis) ----

#[link(name = "iphlpapi")]
extern "system" {
    /// GetIfTable2(table: *mut *mut MIB_IF_TABLE2) -> WIN32_ERROR(0 成功)
    fn GetIfTable2(table: *mut *mut MibIfTable2) -> u32;
    fn FreeMibTable(memory: *const c_void);
}

/// NET_LUID_LH(仅占位 8 字节,不读取)
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
union NetLuidLh {
    value: u64,
    info: u64,
}

/// GUID(占位 16 字节,不读取)
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// MIB_IF_ROW2(仅读取 if_type/oper_status/in_octets/out_octets,
/// 其余字段保持全量声明以维持 sizeof=1344,指针步进正确)
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MibIfRow2 {
    net_luid: NetLuidLh,
    interface_index: u32,
    interface_guid: Guid,
    alias: [u16; 257],
    description: [u16; 257],
    physical_address_length: u32,
    physical_address: [u8; 32],
    permanent_physical_address: [u8; 32],
    mtu: u32,
    if_type: u32,
    tunnel_type: i32,
    media_type: i32,
    physical_medium_type: i32,
    access_type: i32,
    direction_type: i32,
    interface_and_oper_status_flags: u8,
    oper_status: i32,
    admin_status: i32,
    media_connect_state: i32,
    network_guid: Guid,
    connection_type: i32,
    transmit_link_speed: u64,
    receive_link_speed: u64,
    in_octets: u64,
    in_ucast_pkts: u64,
    in_nucast_pkts: u64,
    in_discards: u64,
    in_errors: u64,
    in_unknown_protos: u64,
    in_ucast_octets: u64,
    in_multicast_octets: u64,
    in_broadcast_octets: u64,
    out_octets: u64,
    out_ucast_pkts: u64,
    out_nucast_pkts: u64,
    out_discards: u64,
    out_errors: u64,
    out_ucast_octets: u64,
    out_multicast_octets: u64,
    out_broadcast_octets: u64,
    out_qlen: u64,
}

/// MIB_IF_TABLE2(Table 为 [MIB_IF_ROW2; 1] 柔性数组模拟)
#[repr(C)]
#[derive(Clone, Copy)]
struct MibIfTable2 {
    num_entries: u32,
    table: [MibIfRow2; 1],
}

/// 遍历全部接口,聚合有效接口(非 loopback、OperStatus=Up)的累计收发字节。
/// 【需实机验证】GetIfTable2 系统调用本身;失败或失败时返回空采样。
fn sample_network() -> NetSample {
    let mut table: *mut MibIfTable2 = std::ptr::null_mut();
    let rc = unsafe { GetIfTable2(&mut table) };
    if rc != 0 || table.is_null() {
        return NetSample::default();
    }
    let result = unsafe {
        let rows =
            std::slice::from_raw_parts((*table).table.as_ptr(), (*table).num_entries as usize);
        let octets: Vec<IfOctets> = rows
            .iter()
            .map(|r| IfOctets {
                if_type: r.if_type,
                oper_status: r.oper_status,
                rx: r.in_octets,
                tx: r.out_octets,
            })
            .collect();
        sum_active_octets(&octets)
    };
    unsafe { FreeMibTable(table as *const c_void) };
    result
}

/// 读取 GetSystemTimes 原始计数(两次采样差商原料)
fn read_cpu_times() -> CpuSample {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    unsafe {
        let mut idle: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        GetSystemTimes(&mut idle, &mut kernel, &mut user);
        fn ft(t: &FILETIME) -> u64 {
            ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
        }
        CpuSample {
            idle: ft(&idle),
            kernel: ft(&kernel),
            user: ft(&user),
        }
    }
}

/// 读取物理内存使用(已用 MB, 总量 MB)
fn read_memory_mb() -> (u64, u64) {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    unsafe {
        let mut ms: MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        GlobalMemoryStatusEx(&mut ms);
        let used = ms.ullTotalPhys.saturating_sub(ms.ullAvailPhys);
        (used / (1024 * 1024), ms.ullTotalPhys / (1024 * 1024))
    }
}

/// 采样线程跨轮状态(上一轮 CPU 计数 / 接口字节 / 采样时刻)
struct SamplerState {
    cpu: CpuSample,
    net: NetSample,
    last: Instant,
}

/// 采样线程运行门控(热生效:enable_island 关闭时 [stop] 置 false,线程自然退出)
static RUNNING: AtomicBool = AtomicBool::new(false);
/// 线程代次(2026-08-14 审计 F2-4):stop() 递增代次,在途旧线程(尚在 sleep 中,
/// 最长多活一个采样周期 2s)醒来发现代次不匹配立即退出——否则 stop→ensure_started
/// 快速切换期间,旧线程会因 RUNNING 已被 swap(true) 而"复活",与新线程并存
/// 双线程重复 emit sys-status。
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// 幂等启动常驻采样线程(懒启动):
/// 1. 首轮同步采样一次(CPU 用 120ms 双采样差商,网络速率为 0),立即写快照并 emit,
///    保证首个快照非空且 CPU/内存立即可见;
/// 2. 之后每 2s 采样 → 更新共享快照 → emit `sys-status`。
/// 首次 sys_get_status invoke 时懒启动;集成 agent 亦可在 setup 中调用。
/// 热生效:enable_island 关闭 → [stop] 停止线程;再次开启 → ensure_started 重启(幂等)。
pub fn ensure_started(app: &AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    // 领取新代次:旧线程(若尚在 sleep 未退出)醒来后因代次不匹配直接退出,
    // 不会因 RUNNING=true 而复活——双线程竞态消除(2026-08-14 审计 F2-4)
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    // 首轮同步采样
    let cpu_a = read_cpu_times();
    std::thread::sleep(Duration::from_millis(120));
    let cpu_b = read_cpu_times();
    let (mem_used_mb, mem_total_mb) = read_memory_mb();
    let net = sample_network();
    let first = SysStatus {
        cpu: cpu_percent(&cpu_a, &cpu_b),
        mem_used_mb,
        mem_total_mb,
        net_rx_bps: 0,
        net_tx_bps: 0,
    };
    set_snapshot(first.clone());
    let _ = app.emit("sys-status", &first);

    let handle = app.clone();
    let state = SamplerState { cpu: cpu_b, net, last: Instant::now() };
    if let Err(e) = std::thread::Builder::new()
        .name("sys-sampler".to_string())
        .spawn(move || sampler_loop(handle, state, gen))
    {
        eprintln!("[aurora] 启动 sys-sampler 采样线程失败: {e}");
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// 停止采样线程(热生效;幂等)。线程在下一轮采样前退出;最后快照保留(命令仍可读)。
pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
    // 递增代次:使在途旧线程(尚在 sleep 的循环体)醒来立即退出,
    // 不等下一轮 while 判断——配合 ensure_started 的代次领取防双线程并存
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// 循环继续条件(纯判断,便于单测):RUNNING 为 true 且代次仍是本线程的一代
fn sampler_should_run(gen: u64) -> bool {
    RUNNING.load(Ordering::SeqCst) && gen == GENERATION.load(Ordering::SeqCst)
}

/// 常驻循环:每 2s 采样一次,差商计算 CPU/网络速率后写快照并广播事件;
/// RUNNING 置 false 或代次被顶掉(新线程已启动)后线程在下一次醒来退出
fn sampler_loop(app: AppHandle, mut state: SamplerState, gen: u64) {
    while sampler_should_run(gen) {
        std::thread::sleep(SAMPLING_INTERVAL);
        let now = Instant::now();
        let cpu_cur = read_cpu_times();
        let net_cur = sample_network();
        let (mem_used_mb, mem_total_mb) = read_memory_mb();

        let cpu = cpu_percent(&state.cpu, &cpu_cur);
        let dt = (now - state.last).as_secs_f64();
        let (net_rx_bps, net_tx_bps) = net_rate(&state.net, &net_cur, dt);
        state = SamplerState { cpu: cpu_cur, net: net_cur, last: now };

        let status = SysStatus { cpu, mem_used_mb, mem_total_mb, net_rx_bps, net_tx_bps };
        set_snapshot(status.clone());
        // 广播给灵动岛/Dock/后续模块;托盘 tooltip 更新由集成 agent 在 tray.rs 收尾接线
        let _ = app.emit("sys-status", &status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 双线程竞态回归(2026-08-14 审计 F2-4):stop 后旧代次线程即使遇到 RUNNING
    /// 被新 ensure_started swap(true) 也不得复活;只有持最新代次的线程可运行。
    /// 注意:本用例是唯一读写 RUNNING/GENERATION 的测试,避免并行互踩;
    /// 结尾复位 RUNNING,保证不影响后续用例。
    #[test]
    fn stale_generation_thread_cannot_resume() {
        // 线程 A(代次 1)运行中
        RUNNING.store(true, Ordering::SeqCst);
        GENERATION.store(1, Ordering::SeqCst);
        assert!(sampler_should_run(1), "运行中且代次匹配 → 继续");

        // stop():RUNNING=false 且代次递增 → 旧代次线程立即判定退出
        stop();
        assert!(!sampler_should_run(1), "stop 后旧代次线程必须退出");

        // ensure_started 路径:swap(true) 拿到旧值 false → 领取新代次 → 新线程
        let swapped = RUNNING.swap(true, Ordering::SeqCst);
        assert!(!swapped, "stop 后 swap 应拿到 false(旧线程已不在运行)");
        let new_gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(new_gen > 1, "新线程必须持有更新的代次");
        assert!(sampler_should_run(new_gen), "新代次线程可运行");
        // 关键断言:即使 RUNNING 为 true,旧代次(仍可能在途 sleep)不得恢复循环
        assert!(
            !sampler_should_run(1),
            "旧代次线程不得因 RUNNING=true 复活——否则与新线程并存双 emit"
        );

        // 复位:避免影响其他用例(本测试进程中无其他用例读,复位是防御性)
        RUNNING.store(false, Ordering::SeqCst);
    }
}
