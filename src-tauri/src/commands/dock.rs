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
//!   - dock_get_icon    转发 dock_icon(ExtractIconExW→GetDIBits→png→base64 双缓存)
//!
//! 说明:Dock 已并入搜索窗口(2026-08-12 用户定调),本模块只提供数据命令
//! (条目/运行态/图标),无窗口/定位/自动隐藏逻辑;旧 dock 窗口与其交互已删除。
//!
//! 文件结构:dock_icon.rs 为 dock.rs 的私有子模块(设计原计划放 src 根,但 lib.rs
//! 属集成 agent 所有无法加 mod 声明,放 commands/ 下由 dock.rs 声明,功能等价)。

#[path = "dock_icon.rs"]
mod dock_icon;

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

use super::config::{config_path, load_from, save_to, DockItem};

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

/// Dock 项目的匹配目标路径:.lnk 解析为指向的 exe(解析失败回退原路径);其余原样
pub fn item_target_path(item_path: &str) -> String {
    if item_path.to_lowercase().ends_with(".lnk") {
        if let Some(t) = resolve_lnk_target(Path::new(item_path)) {
            return t;
        }
    }
    item_path.to_string()
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

/// 写 Dock 条目:更新内存缓存并写回 config(保留其余字段);成功返回 true
pub fn save_items(cfg_path: &Path, items: &[DockItem]) -> bool {
    let mut cfg = load_from(cfg_path);
    cfg.dock_items = items.to_vec();
    if !save_to(cfg_path, &cfg) {
        return false;
    }
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

/// 写回 Dock 条目(内存缓存 + config 落盘)
#[tauri::command]
pub fn dock_set_items(app: tauri::AppHandle, items: Vec<DockItem>) -> bool {
    save_items(&config_path(&app), &items)
}

/// 启动或聚焦 Dock 项:运行中 → 恢复最小化并置前台;未运行 → opener 启动
#[tauri::command]
pub fn dock_launch(item: DockItem) -> bool {
    let target = item_target_path(&item.path);
    if let Some(w) = running_windows()
        .into_iter()
        .find(|w| path_matches(&w.exe, &target))
    {
        unsafe {
            ShowWindow(w.hwnd as HWND, SW_RESTORE);
            SetForegroundWindow(w.hwnd as HWND);
        }
        return true;
    }
    opener::open(&item.path).is_ok()
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
            windows.iter().any(|w| path_matches(&w.exe, &target))
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

    // ---- 系统枚举冒烟 ----

    #[test]
    fn running_windows_smoke() {
        let windows = running_windows();
        let mut set = std::collections::HashSet::new();
        for w in &windows {
            let lower = w.exe.to_lowercase();
            assert!(lower.ends_with(".exe"), "非 exe 进程路径: {}", w.exe);
            assert!(set.insert(lower), "exe 路径不应重复: {}", w.exe);
        }
    }
}
