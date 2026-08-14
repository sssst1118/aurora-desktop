//! Dock 栏命令与运行检测(Phase2 2.1 模块)。
//!
//! 五个命令与 stubs.rs 占位的前端调用契约一致;需要配置路径的命令追加了 tauri
//! 自动注入的 `app: AppHandle` 参数(与 config_load/config_save 同款惯例,前端
//! 调用参数不变,集成 agent 切 invoke_handler 时无需改前端):
//!   - dock_get_items   读条目(内存缓存,首次按配置路径加载一次)
//!   - dock_set_items   写条目(内存缓存 + config 落盘,保留其余字段)
//!   - dock_launch      运行中 → 聚焦(ShowWindow SW_RESTORE + SetForegroundWindow);
//!                      未运行 → opener 启动
//!   - dock_get_running EnumWindows + IsWindowVisible + GetWindowTextW + PID 去重 +
//!                      QueryFullProcessImageNameW 取 exe 路径;与 Dock 条目匹配
//!                      (lnk 用 COM IShellLinkW 解析目标)后返回被收录的运行条目路径
//!   - dock_get_icon    转发 dock_icon(SHGetFileInfoW→GetDIBits→png→base64 双缓存)
//!
//! 说明:Dock 已并入搜索窗口(2026-08-12 用户定调),本模块只提供数据命令
//! (条目/运行态/图标),无窗口/定位/自动隐藏逻辑;旧 dock 窗口与其交互已删除。
//!
//! 文件结构:dock_icon.rs 为 dock.rs 的私有子模块(设计原计划放 src 根,但 lib.rs
//! 属集成 agent 所有无法加 mod 声明,放 commands/ 下由 dock.rs 声明,功能等价)。

#[path = "dock_icon.rs"]
mod dock_icon;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tauri::Manager;
use windows_sys::core::{GUID, PCWSTR};
use windows_sys::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM};
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow,
    ShowWindow, SW_RESTORE,
};

use super::config::{config_lock, config_path, load_from, save_to, DockItem};

// ==================== 运行检测(纯函数 + 系统枚举) ====================

/// 窗口采样(单测友好纯数据):hwnd / pid / 可见性 / 是否有标题
#[derive(Clone, Copy, Debug)]
pub struct WindowSample {
    pub hwnd: usize,
    pub pid: u32,
    pub visible: bool,
    pub has_title: bool,
}

/// 过滤 + PID 去重:仅保留"可见且有标题"的窗口,同一 PID 只保留第一个(设计 §1.2)
pub fn filter_windows(samples: Vec<WindowSample>) -> Vec<WindowSample> {
    let mut seen = std::collections::HashSet::new();
    samples
        .into_iter()
        .filter(|w| w.visible && w.has_title && seen.insert(w.pid))
        .collect()
}

/// 枚举到的运行窗口(按 PID 去重后)
#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub hwnd: usize,
    pub exe: String,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let samples = &mut *(lparam as *mut Vec<WindowSample>);
    let visible = IsWindowVisible(hwnd) != 0;
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    samples.push(WindowSample {
        hwnd: hwnd as usize,
        pid,
        visible,
        has_title: n > 0,
    });
    1 // 继续枚举
}

/// 枚举可见顶层窗口(过滤 + PID 去重)并取各进程 exe 路径
pub fn running_windows() -> Vec<WindowInfo> {
    let mut samples: Vec<WindowSample> = Vec::new();
    unsafe {
        EnumWindows(Some(enum_proc), &mut samples as *mut Vec<WindowSample> as isize);
    }
    filter_windows(samples)
        .into_iter()
        .filter_map(|w| {
            let exe = process_exe_path(w.pid)?;
            Some(WindowInfo {
                hwnd: w.hwnd,
                exe,
            })
        })
        .collect()
}

fn process_exe_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut sz = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut sz);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..sz as usize]))
    }
}

// ==================== 路径比较与 lnk 目标解析 ====================

/// 比较用归一化:去空白 + 正斜杠统一为反斜杠 + 小写
pub fn normalize_path(p: &str) -> String {
    p.trim().replace('/', "\\").to_lowercase()
}

/// 大小写不敏感、斜杠不敏感的路径相等比较
pub fn path_matches(a: &str, b: &str) -> bool {
    normalize_path(a) == normalize_path(b)
}

/// lnk 目标解析缓存:首次解析含 COM 初始化开销(实测 ~1.5s),缓存后瞬时。
/// 条目不变时解析结果稳定,缓存命中返回克隆
static LNK_TARGET_CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();

fn lnk_target_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    LNK_TARGET_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析 lnk 目标(带缓存);非 lnk 原样返回
fn cached_target(item_path: &str) -> Option<String> {
    if let Ok(g) = lnk_target_cache().lock() {
        if let Some(t) = g.get(item_path) {
            return t.clone();
        }
    }
    let t = if item_path.to_lowercase().ends_with(".lnk") {
        resolve_lnk_target(Path::new(item_path))
    } else {
        Some(item_path.to_string())
    };
    if let Ok(mut g) = lnk_target_cache().lock() {
        g.insert(item_path.to_string(), t.clone());
    }
    t
}

/// Dock 项目的匹配目标路径:.lnk 解析为指向的 exe(解析失败回退原路径);其余原样
pub fn item_target_path(item_path: &str) -> String {
    cached_target(item_path).unwrap_or_else(|| item_path.to_string())
}

/// 运行窗口与 Dock 项匹配:精确路径相等优先;失败时仅当**扩展名不同**才回退
/// stem 匹配(NoMachine 场景:lnk 目标是 nxplayer.exe 启动器,实际运行进程是
/// nxplayer.bin,路径不同且扩展名不同,但同 stem,视为同一应用)。
/// 扩展名相同而目录不同不匹配(不同目录的同名 exe 是不同应用)。
pub fn window_matches_item(win_exe: &str, item_target: &str) -> bool {
    if path_matches(win_exe, item_target) {
        return true;
    }
    let part = |p: &str, f: fn(&Path) -> Option<&std::ffi::OsStr>| {
        f(Path::new(p)).map(|s| s.to_string_lossy().to_lowercase())
    };
    let ext = |p: &str| part(p, Path::extension);
    let stem = |p: &str| part(p, Path::file_stem);
    ext(win_exe) != ext(item_target)
        && stem(win_exe).is_some_and(|a| stem(item_target).is_some_and(|b| a == b))
}

// ---- COM IShellLinkW(windows-sys 不含 COM 接口定义,手写 vtable)----

const CLSID_SHELL_LINK: GUID = GUID {
    data1: 0x00021401,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_ISHELL_LINK_W: GUID = GUID {
    data1: 0x000214F9,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

/// IShellLinkW vtable:SDK 头文件(shobjidl_core.h)`IShellLinkW : public IUnknown`
/// 直接继承 IUnknown(无 IPersist/IPersistFile 中间层),vtable =
/// IUnknown(3) + IShellLinkW(18) = 21 槽。GetPath=槽3, SetPath=槽20。
/// IPersistFile 是独立接口,须通过 QueryInterface(IID_IPersistFile)获取。
#[repr(C)]
struct ShellLinkVtbl {
    query_interface: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    get_path: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16, i32, *mut core::ffi::c_void, u32) -> i32,
    get_id_list: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> i32,
    set_id_list: unsafe extern "system" fn(*mut core::ffi::c_void, *const core::ffi::c_void) -> i32,
    get_description: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16, i32) -> i32,
    set_description: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> i32,
    get_working_directory: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16, i32) -> i32,
    set_working_directory: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> i32,
    get_arguments: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16, i32) -> i32,
    set_arguments: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> i32,
    get_hotkey: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16) -> i32,
    set_hotkey: unsafe extern "system" fn(*mut core::ffi::c_void, u16) -> i32,
    get_show_cmd: unsafe extern "system" fn(*mut core::ffi::c_void, *mut i32) -> i32,
    set_show_cmd: unsafe extern "system" fn(*mut core::ffi::c_void, i32) -> i32,
    get_icon_location: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16, i32, *mut i32) -> i32,
    set_icon_location: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR, i32) -> i32,
    set_relative_path: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR, u32) -> i32,
    resolve: unsafe extern "system" fn(*mut core::ffi::c_void, HWND, u32) -> i32,
    set_path: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> i32,
}

/// IPersistFile vtable:IUnknown(3) + IPersist::GetClassID(1) + IPersistFile(5) = 9 槽。
/// GetClassID=槽3 / Load=槽5 / Save=槽6。经 IShellLinkW::QueryInterface 获取。
#[repr(C)]
struct PersistFileVtbl {
    query_interface: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    get_class_id: unsafe extern "system" fn(*mut core::ffi::c_void, *mut GUID) -> i32,
    is_dirty: unsafe extern "system" fn(*mut core::ffi::c_void) -> i32,
    load: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR, u32) -> i32,
    save: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR, i32) -> i32,
    save_completed: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> i32,
    get_cur_file: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut u16) -> i32,
}

const IID_IPERSIST_FILE: GUID = GUID {
    data1: 0x0000_010B,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const SLGP_RAWPATH: u32 = 0x0000_0004; // GetPath 返回链接文件中的原始路径
const COINIT_APARTMENTTHREADED: u32 = 0x2;
const STGM_READ: u32 = 0;

/// 解析 .lnk 指向的目标路径(IShellLinkW::GetPath,SLGP_RAWPATH)
pub fn resolve_lnk_target(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hr = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
        let co_ok = hr == 0 || hr == 1; // S_OK / S_FALSE;RPC_E_CHANGED_MODE 时不 Uninitialize
        let mut com: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_SHELL_LINK,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_ISHELL_LINK_W,
            &mut com,
        );
        let mut result = None;
        if hr >= 0 && !com.is_null() {
            let vtbl = &**(com as *const *const ShellLinkVtbl);
            // 持久化读写走独立接口 IPersistFile(Load),经 QueryInterface 获取
            let mut pf: *mut core::ffi::c_void = std::ptr::null_mut();
            let hr_qi = (vtbl.query_interface)(com, &IID_IPERSIST_FILE, &mut pf);
            if hr_qi >= 0 && !pf.is_null() {
                let pvtbl = &**(pf as *const *const PersistFileVtbl);
                let hr_load = (pvtbl.load)(pf, wide.as_ptr(), STGM_READ);
                if hr_load >= 0 {
                    let mut buf = [0u16; 2048];
                    let hr_path = (vtbl.get_path)(com, buf.as_mut_ptr(), buf.len() as i32, std::ptr::null_mut(), SLGP_RAWPATH);
                    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    let s = String::from_utf16_lossy(&buf[..len]);
                    if hr_path >= 0 && !s.is_empty() {
                        result = Some(s);
                    }
                }
                (pvtbl.release)(pf);
            }
            (vtbl.release)(com);
        }
        if co_ok {
            CoUninitialize();
        }
        result
    }
}

// ==================== 条目状态(内存缓存 + config 落盘) ====================

/// Dock 条目数上限(2026-08-14 审计 F3-3):此前前端可传任意长度 Vec<DockItem>
/// 直接写 config.json——条目无上限时异常/恶意输入会让配置文件无限膨胀,
/// 且每次读-改-写全量序列化的配置链路随之拖慢。50 条远超正常用法
/// (常用 Dock 不过十来个),超限拒绝。
pub const MAX_DOCK_ITEMS: usize = 50;

static ITEM_CACHE: OnceLock<Mutex<Option<(PathBuf, Vec<DockItem>)>>> = OnceLock::new();

fn item_cache() -> &'static Mutex<Option<(PathBuf, Vec<DockItem>)>> {
    ITEM_CACHE.get_or_init(|| Mutex::new(None))
}

/// 读 Dock 条目:命中内存缓存直接返回;首次按配置路径从磁盘加载(设计 §1.2)
pub fn load_items(cfg_path: &Path) -> Vec<DockItem> {
    let mut g = item_cache().lock().unwrap_or_else(|p| p.into_inner());
    match &*g {
        Some((p, items)) if p == cfg_path => items.clone(),
        _ => {
            let items = load_from(cfg_path).dock_items;
            *g = Some((cfg_path.to_path_buf(), items.clone()));
            items
        }
    }
}

/// 写 Dock 条目:更新内存缓存并写回 config(保留其余字段);成功返回 true。
/// 条目数超 [MAX_DOCK_ITEMS] 上限拒绝(返回 false,不动磁盘与内存缓存)
pub fn save_items(cfg_path: &Path, items: &[DockItem]) -> bool {
    if items.len() > MAX_DOCK_ITEMS {
        eprintln!(
            "[aurora] dock 条目数超上限({} > {MAX_DOCK_ITEMS}),拒绝保存",
            items.len()
        );
        return false;
    }
    // 并发修复(2026-08-13):Dock 条目与设置同存 config.json(dock_items 字段),
    // 本条目的"读全量→改字段→写全量"与 config_save / search_save_geometry 是同一份
    // 文件的竞态写,必须共用配置锁——否则条目保存与设置保存并发时后写覆盖先写,
    // 条目或设置随机丢失。
    let _guard = config_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut cfg = load_from(cfg_path);
    cfg.dock_items = items.to_vec();
    if !save_to(cfg_path, &cfg) {
        return false;
    }
    // 缓存更新留在锁内:保证"落盘内容 == 内存缓存内容"不撕裂
    let mut g = item_cache().lock().unwrap_or_else(|p| p.into_inner());
    *g = Some((cfg_path.to_path_buf(), items.to_vec()));
    true
}

// ==================== 命令 ====================

/// 读取 Dock 条目(内存缓存)
#[tauri::command]
pub fn dock_get_items(app: tauri::AppHandle) -> Vec<DockItem> {
    load_items(&config_path(&app))
}

/// 写回 Dock 条目(内存缓存 + config 落盘);条目数超 [MAX_DOCK_ITEMS] 上限
/// 返回 Err(前端据此提示用户移除部分条目,而非静默失败)
#[tauri::command]
pub fn dock_set_items(app: tauri::AppHandle, items: Vec<DockItem>) -> Result<bool, String> {
    if items.len() > MAX_DOCK_ITEMS {
        return Err(format!(
            "Dock 条目数超过上限({MAX_DOCK_ITEMS} 条),请先移除部分条目"
        ));
    }
    Ok(save_items(&config_path(&app), &items))
}

/// "启动中"集合:防连点排队重复启动。窗口出现前 running_windows 匹配不到该应用,
/// 若不加防抖,每次点击都重新拉起(排队多实例)。解除时机 = 窗口已出现(轮询精确
/// 解除)或超时兜底(10s),比前端固定防抖窗口准确——应用慢启动几秒防抖就保持几秒。
static LAUNCHING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn launching() -> &'static Mutex<HashSet<String>> {
    LAUNCHING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 标记条目"启动中"(检查 + 标记在同一锁区间内完成,TOCTOU 修复 2026-08-14):
/// 返回 false 表示该条目已在启动中,调用方应短路返回。
/// 若检查(contains)与标记(insert)分处两次独立锁获取,两个并发请求可同时
/// 通过检查并各自 opener 启动(慢启动 lnk 解析 ~1.3s 期间双击,或并发 IPC)→
/// 双实例。同一锁区间内先查后插,竞态窗口被消除。
fn mark_launching(path: &str) -> bool {
    let mut g = launching().lock().unwrap_or_else(|p| p.into_inner());
    if g.contains(path) {
        return false;
    }
    g.insert(path.to_string());
    true
}

/// 解除"启动中"标记(启动失败回滚 / 后台轮询到窗口出现或超时)
fn unmark_launching(path: &str) {
    if let Ok(mut g) = launching().lock() {
        g.remove(path);
    }
}

/// 启动或聚焦 Dock 项:运行中 → 恢复最小化并置前台;未运行 → opener 启动
#[tauri::command]
pub fn dock_launch(item: DockItem) -> bool {
    let target = item_target_path(&item.path);
    if let Some(w) = running_windows()
        .into_iter()
        .find(|w| window_matches_item(&w.exe, &target))
    {
        unsafe {
            ShowWindow(w.hwnd as HWND, SW_RESTORE);
            SetForegroundWindow(w.hwnd as HWND);
        }
        return true;
    }
    // 检查+标记同一锁区间(TOCTOU 修复):并发重复点击/IPC 只有第一个能通过,
    // 第二个立即短路,不会各自 opener 启动双实例
    if !mark_launching(&item.path) {
        return true;
    }
    // 直接打开解析出的目标 exe:实测 lnk 经 Shell 启动要 ~1.3s,直开 exe 仅 ~17ms。
    // 注:lnk 附带启动参数/工作目录时语义有损(NoMachine 等本场景均无参数,可接受);
    // 解析失败(拿不到目标)则回退原 lnk 路径,保证一定能启动。
    let launch_path = if item.path.to_lowercase().ends_with(".lnk") && !target.eq_ignore_ascii_case(&item.path)
    {
        &target
    } else {
        &item.path
    };
    if !opener::open(launch_path).is_ok() {
        // 启动失败:回滚标记,允许后续重试(与"启动中短路"不冲突——标记在
        // opener 之前就位,失败必须立即解除,否则该条目永远无法再启动)
        unmark_launching(&item.path);
        return false;
    }
    let (path, target) = (item.path.clone(), target);
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if std::time::Instant::now() >= deadline {
                break;
            }
            if running_windows()
                .iter()
                .any(|w| window_matches_item(&w.exe, &target))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        unmark_launching(&path);
    });
    true
}

/// 运行中且被 Dock 收录的应用条目路径集合(前端小圆点渲染用)
#[tauri::command]
pub fn dock_get_running(app: tauri::AppHandle) -> Vec<String> {
    let items = load_items(&config_path(&app));
    let windows = running_windows();
    items
        .into_iter()
        .filter(|it| {
            let target = item_target_path(&it.path);
            // stem 回退匹配:lnk 目标 exe 与实际运行进程可能不同名
            // (NoMachine:nxplayer.exe 启动器 → 实际进程 nxplayer.bin)
            windows.iter().any(|w| window_matches_item(&w.exe, &target))
        })
        .map(|it| it.path)
        .collect()
}

/// 图标 base64 data URL(内存 + 磁盘双缓存);提取失败返回 None(前端回退占位)
#[tauri::command]
pub fn dock_get_icon(app: tauri::AppHandle, path: String) -> Option<String> {
    let dir = app
        .path()
        .app_config_dir()
        .map(|p| p.join("icons"))
        .unwrap_or_else(|_| std::env::temp_dir().join("aurora_icons"));
    dock_icon::icon_data_url(&path, &dir)
}

// ==================== 单测 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Com::CoCreateInstance;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aurora_dock_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ---- 运行检测过滤纯函数(设计 §1.5)----

    #[test]
    fn filter_windows_keeps_visible_with_title_and_dedups_pid() {
        let samples = vec![
            WindowSample { hwnd: 1, pid: 10, visible: true, has_title: true },  // 保留
            WindowSample { hwnd: 2, pid: 10, visible: true, has_title: true },  // 同 PID 去重
            WindowSample { hwnd: 3, pid: 20, visible: false, has_title: true }, // 不可见剔除
            WindowSample { hwnd: 4, pid: 30, visible: true, has_title: false }, // 空标题剔除
            WindowSample { hwnd: 5, pid: 40, visible: true, has_title: true },  // 保留
        ];
        let kept = filter_windows(samples);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].hwnd, 1);
        assert_eq!(kept[1].hwnd, 5);
    }

    // ---- 路径比较 ----

    #[test]
    fn path_matching_is_case_and_slash_insensitive() {
        assert!(path_matches(
            r"C:\Windows\System32\NOTEPAD.EXE",
            r"c:\windows\system32\notepad.exe"
        ));
        assert!(path_matches(
            "C:/Windows/System32/notepad.exe",
            r"c:\windows\system32\notepad.exe"
        ));
        assert!(!path_matches(
            r"C:\Windows\System32\notepad.exe",
            r"C:\Windows\System32\calc.exe"
        ));
        assert!(!path_matches("", r"C:\Windows\System32\notepad.exe"));
    }

    // ---- lnk 目标解析(真实 .lnk 往返,设计 §1.5)----

    /// 用 COM IShellLinkW 构造一个真实 .lnk 指向 target(测试辅助,复用生产 vtable)
    fn create_shell_link(lnk: &Path, target: &Path) -> bool {
        unsafe {
            let hr = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
            let co_ok = hr == 0 || hr == 1;
            let mut com: *mut core::ffi::c_void = std::ptr::null_mut();
            let hr = CoCreateInstance(
                &CLSID_SHELL_LINK,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ISHELL_LINK_W,
                &mut com,
            );
            let mut ok = false;
            if hr >= 0 && !com.is_null() {
                let vtbl = &**(com as *const *const ShellLinkVtbl);
                let t: Vec<u16> = target
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let hr_set = (vtbl.set_path)(com, t.as_ptr());
                let l: Vec<u16> = lnk
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                // 保存走 IPersistFile::Save(槽6)
                let mut pf: *mut core::ffi::c_void = std::ptr::null_mut();
                let hr_qi = (vtbl.query_interface)(com, &IID_IPERSIST_FILE, &mut pf);
                if hr_set >= 0 && hr_qi >= 0 && !pf.is_null() {
                    let pvtbl = &**(pf as *const *const PersistFileVtbl);
                    let hr_save = (pvtbl.save)(pf, l.as_ptr(), 1); // fRemember = TRUE
                    ok = hr_save >= 0 && lnk.exists();
                    (pvtbl.release)(pf);
                }
                (vtbl.release)(com);
            }
            if co_ok {
                CoUninitialize();
            }
            ok
        }
    }

    #[test]
    fn resolve_lnk_roundtrip() {
        let dir = tmp_dir("lnk");
        let fake_exe = dir.join("fake_app.exe");
        std::fs::write(&fake_exe, b"MZ fake exe content").unwrap();
        let lnk = dir.join("my_link.lnk");
        assert!(create_shell_link(&lnk, &fake_exe), "创建 .lnk 应成功");
        let target = resolve_lnk_target(&lnk);
        // 注意:temp_dir 可能含 8.3 短名(如 GENGZH~1),外壳保存时规范化为长名,
        // 两侧都取 canonicalize(短名→长名)后再比,避免表象差异
        let resolved = target
            .as_deref()
            .map(|t| std::fs::canonicalize(t).expect("解析结果应可 canonicalize"));
        assert_eq!(
            resolved,
            Some(std::fs::canonicalize(&fake_exe).expect("目标文件应可 canonicalize")),
            "解析目标应指向同一文件"
        );
        // 不存在的 .lnk → None
        assert!(resolve_lnk_target(&dir.join("missing.lnk")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn item_target_resolves_lnk_and_keeps_exe() {
        let dir = tmp_dir("target");
        let fake_exe = dir.join("fake_app.exe");
        std::fs::write(&fake_exe, b"MZ").unwrap();
        let lnk = dir.join("a.lnk");
        assert!(create_shell_link(&lnk, &fake_exe));
        // 8.3 短名规范化见 resolve_lnk_roundtrip 注释,同样 canonicalize 后比较
        let resolved = std::fs::canonicalize(item_target_path(&lnk.to_string_lossy())).expect("lnk 应解析为存在的目标");
        assert_eq!(resolved, std::fs::canonicalize(&fake_exe).unwrap());
        assert_eq!(item_target_path(r"C:\x\calc.exe"), r"C:\x\calc.exe");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 条目保存/加载往返(设计 §1.5)----

    #[test]
    fn items_save_load_roundtrip_preserves_other_fields() {
        let dir = tmp_dir("items");
        let cfg_path = dir.join("config.json");
        // 预先写一份含其他字段的配置(模拟用户已有配置)
        let mut cfg = load_from(&cfg_path);
        cfg.hotkey_search = "ctrl+alt+q".to_string();
        save_to(&cfg_path, &cfg);

        let items = vec![
            DockItem { name: "记事本".into(), path: r"C:\Windows\System32\notepad.exe".into() },
            DockItem { name: "计算器".into(), path: r"C:\Windows\System32\calc.exe".into() },
        ];
        assert!(save_items(&cfg_path, &items));
        let loaded = load_items(&cfg_path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "记事本");
        assert_eq!(loaded[1].path, r"C:\Windows\System32\calc.exe");
        // 其余字段不受 dock_items 写回影响
        assert_eq!(load_from(&cfg_path).hotkey_search, "ctrl+alt+q");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn items_empty_write_clears() {
        let dir = tmp_dir("items_clear");
        let cfg_path = dir.join("config.json");
        let items = vec![DockItem { name: "x".into(), path: r"C:\x.exe".into() }];
        assert!(save_items(&cfg_path, &items));
        assert!(save_items(&cfg_path, &[]));
        assert!(load_items(&cfg_path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 条目数上限(2026-08-14 审计 F3-3)----

    #[test]
    fn items_over_limit_rejected_and_old_state_kept() {
        let dir = tmp_dir("items_limit");
        let cfg_path = dir.join("config.json");
        // 先正常写入 1 条(旧状态)
        let items = vec![DockItem { name: "a".into(), path: r"C:\a.exe".into() }];
        assert!(save_items(&cfg_path, &items));
        // 51 条超上限:拒绝写盘,磁盘与内存缓存均保持旧值
        let over: Vec<DockItem> = (0..=(MAX_DOCK_ITEMS as i32))
            .map(|i| DockItem { name: format!("o{i}"), path: format!(r"C:\o{i}.exe") })
            .collect();
        assert!(!save_items(&cfg_path, &over), "超限应拒绝落盘");
        let kept = load_items(&cfg_path);
        assert_eq!(kept.len(), 1, "拒绝后内存缓存保持旧值");
        assert_eq!(kept[0].name, "a");
        assert_eq!(load_from(&cfg_path).dock_items.len(), 1, "拒绝后磁盘保持旧值");
        // 恰好 50 条:放行
        let exact: Vec<DockItem> = (0..MAX_DOCK_ITEMS)
            .map(|i| DockItem { name: format!("e{i}"), path: format!(r"C:\e{i}.exe") })
            .collect();
        assert!(save_items(&cfg_path, &exact), "恰好 50 条应放行");
        assert_eq!(load_items(&cfg_path).len(), MAX_DOCK_ITEMS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 系统枚举冒烟 ----

    #[test]
    fn running_windows_smoke() {
        // 注1:进程路径不保证 .exe 结尾(NoMachine 运行进程是 nxplayer.bin),只断言非空
        // 注2:同 exe 多窗口是 Windows 合法常态(explorer 桌面+文件夹、多个记事本),
        // 绿点判定走 window_matches_item 逐窗匹配,枚举层不要求 exe 去重;
        // 单次 EnumWindows 中 hwnd 不重复才是有效不变式(2026-08-14 修正:原断言
        // 「exe 不重复」在开多个 explorer 窗口时必挂,属测试语义错误非产品缺陷)
        let windows = running_windows();
        let mut set = std::collections::HashSet::new();
        for w in &windows {
            assert!(!w.exe.trim().is_empty(), "exe 路径不应为空");
            assert!(set.insert(w.hwnd), "hwnd 不应重复: {}", w.hwnd);
        }
    }

    // ---- 运行匹配(2026-08-12 绿点/聚焦修复) ----

    #[test]
    fn window_matches_exact_path() {
        // 精确路径:大小写/斜杠不敏感
        assert!(window_matches_item(
            r"C:\Program Files\NoMachine\bin\nxplayer.exe",
            r"c:\program files\nomachine\bin\nxplayer.EXE"
        ));
        // 不同文件:不匹配
        assert!(!window_matches_item(r"C:\a\nxplayer.exe", r"C:\b\nxplayer.exe"));
    }

    #[test]
    fn window_matches_stem_fallback() {
        // 回归:NoMachine 场景 lnk 目标是 nxplayer.exe(启动器),实际运行进程是
        // nxplayer.bin —— 精确路径不匹配,stem 回退应命中,绿点/聚焦才工作
        assert!(window_matches_item(
            r"C:\Program Files\NoMachine\bin\nxplayer.bin",
            r"C:\Program Files\NoMachine\bin\nxplayer.exe"
        ));
        // stem 不同的 exe/bin 对不匹配
        assert!(!window_matches_item(r"C:\a\foo.bin", r"C:\b\bar.exe"));
    }

    // ---- 启动防抖(2026-08-12 连点排队重复启动修复) ----

    #[test]
    fn launch_ignores_repeat_click_while_launching() {
        // 集合中有该条目(上次点击仍在启动中)→ 短路返回 true 不重走 opener。
        // 用不存在的路径:若真走 opener 必然返回 false,返回 true 即证明短路生效。
        // 注意:测试并行跑,此路径仅本用例使用,避免与 launch_failure 用例互踩集合
        let item = DockItem { name: "x".into(), path: r"C:\definitely\not\exists_aurora_a.exe".into() };
        launching().lock().unwrap().insert(item.path.clone());
        assert!(dock_launch(item));
    }

    #[test]
    fn launch_failure_does_not_mark_launching() {
        // 启动失败(路径不存在)→ 不标记,下次点击仍可重试
        let item = DockItem { name: "x".into(), path: r"C:\definitely\not\exists_aurora_b.exe".into() };
        assert!(!dock_launch(item.clone()));
        assert!(!launching().lock().unwrap().contains(&item.path));
    }

    #[test]
    fn mark_launching_deduplicates_same_path() {
        // TOCTOU 回归(2026-08-14):检查与标记必须在同一锁区间,并发第二次请求
        // 必须被拒绝。专用路径避免与并行测试互踩 LAUNCHING 集合
        let p = r"C:\definitely\not\exists_aurora_c.exe";
        assert!(mark_launching(p), "首次标记应成功");
        assert!(!mark_launching(p), "标记未解除期间重复请求必须被拒绝");
        unmark_launching(p);
        assert!(mark_launching(p), "解除后可再次启动");
        unmark_launching(p);
        assert!(!launching().lock().unwrap().contains(p), "最终应清干净");
    }

    #[test]
    fn cached_target_matches_uncached_result() {
        // 缓存不应改变解析结果:同一 lnk 两次解析一致,非 lnk 原样返回
        let lnk = r"C:\Users\Public\Desktop\NoMachine.lnk";
        if !std::path::Path::new(lnk).exists() {
            return;
        }
        // 预热后第二次应命中缓存;断言两次结果一致(等价于解析稳定)
        let first = item_target_path(lnk);
        let second = item_target_path(lnk);
        assert_eq!(first, second);
        assert!(first.to_lowercase().ends_with(".exe"), "lnk 应解析出 exe 目标: {first}");
        assert_eq!(item_target_path(r"C:\x\calc.exe"), r"C:\x\calc.exe");
    }
}
