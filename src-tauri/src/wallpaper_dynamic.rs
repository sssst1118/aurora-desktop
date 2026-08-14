//! 4.1 动态壁纸(WorkerW 注入 + 电池降载)实现层(Windows 专用)
//!
//! - WorkerW 注入:FindWindowW("Progman") → SendMessageTimeoutW(0x052C) →
//!   FindWindowExW 找 Progman 的子 WorkerW(壁纸层;本机实测 DefView 是 Progman 的子窗口,
//!   与设计文档 §1.3"EnumWindows 找顶层 DefView"不同,详见 find_workerw 注释)→
//!   SetParent(webview_hwnd, workerw) + SetWindowPos 铺满主屏;
//! - 电池降载:GetSystemPowerStatus 每 wallpaper_battery_check_sec(默认 30)s 检测一次,
//!   仅状态翻转时 emit `wallpaper-power{on_battery}`(事件契约见设计文档 §0.3);
//! - 素材扫描:只扫配置目录(wallpaper_dynamic_dir → 2.4 wallpaper_dir →
//!   %USERPROFILE%\Pictures 回退链),白名单 mp4/webm/avi/mov/jpg/png/webp/bmp/gif,
//!   按名称排序截 100,目录不存在返回空列表不 panic;
//!   ⚠️ 安全加固:html 已移出白名单——Pictures 内 html 经 asset 协议加载会与前端同源,
//!   可调用全部后端命令(读剪贴板/写配置/执行程序),属高危注入面,一律拒绝;
//! - URL 两段式(设计 §1.5):默认 Pictures 内素材走 asset 协议(前端 convertFileSrc,
//!   url=None);目录外素材由 set 命令后端读文件转 base64 data URL(≤20MB,超出报错提示
//!   放 Pictures 下;上限取舍:20MB 已覆盖常见单视频壁纸,data URL 经 IPC 传前端且
//!   base64 膨胀 4/3,原 50MB 会产生约 67MB 的 IPC 载荷压内存拖慢链路,超限一律
//!   建议挪进 Pictures 走 asset 协议)。base64 无依赖手写(Cargo.toml 属集成 agent
//!   不可加依赖,与 2.5 手写等价 FFI 同风格)。
//!
//! windows-sys 0.59 feature 门控(集成 agent 已在 Cargo.toml 启用):
//! - Win32_System_Power:GetSystemPowerStatus / SYSTEM_POWER_STATUS;
//! - Win32_UI_WindowsAndMessaging:窗口注入全套 API。
//! 全部 unsafe 逐调用包裹 + 注释(风格参考 hotkey.rs / dock_icon.rs);
//! 注入动作本身无法单测(手动验收覆盖),纯函数(is_battery_mode / 素材过滤 /
//! 回退链 / find_workerw 不 panic / base64 / URL 两段式)已带单测。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager}; // Manager:get_webview_window(apply_monitor/multi_apply 用)
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, SendMessageTimeoutW, SetParent,
    SetWindowPos, ShowWindow, HWND_BOTTOM, SMTO_NORMAL, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE,
};

use crate::commands::wallpaper::WallpaperEntry;

/// 触发系统创建 WorkerW 的消息(WM_SPAWN_WORKERW):向 Progman 发送后,
/// 桌面图标层(SHELLDLL_DefView)会挂到新分裂出的 WorkerW 子窗口下
const WM_SPAWN_WORKERW: u32 = 0x052C;
/// 注入消息超时(ms)
const SPAWN_TIMEOUT_MS: u32 = 1000;
/// 素材列表白名单(视频/图片,大小写不敏感;与设计 §1.1 一致)。
/// 安全加固:html 已移除——asset 协议加载 Pictures 内 html 与前端同源,可注入调用
/// 全部后端命令(剪贴板/配置/进程执行),不再允许作为壁纸素材
pub const DYNAMIC_EXT_WHITELIST: [&str; 10] = [
    "mp4", "webm", "avi", "mov", "jpg", "jpeg", "png", "webp", "bmp", "gif",
];
/// 列表展示上限(有节制,不扫全盘)
const MAX_DYNAMIC_LIST: usize = 100;
/// 目录外素材走 data URL 的体积上限(20MB;2026-08-13 审计收紧:原 50MB 经 base64
/// 膨胀后约 67MB 走 IPC 到前端,载荷过大;单视频壁纸 20MB 以内已覆盖常见素材,
/// 超过提示用户挪进 Pictures 走 asset 协议,无 IPC 载荷)
const MAX_DATA_URL_BYTES: u64 = 20 * 1024 * 1024;

/// 注入互斥锁(并发修复 2026-08-13,M10)。
///
/// 竞态场景:`apply_monitor` / `multi_apply` / set 命令的注入路径与 lib.rs 的
/// 多屏热插拔探针线程(每 2s 检查布局签名,变化即调 `multi_apply`)三者并发执行。
/// 每条注入路径都是"先查窗口是否存在、不存在则创建",检查与创建之间是竞态窗口:
/// 两个线程可能同时创建同一 label 的窗口(tauri 窗口创建非原子,后建者报错或
/// 出现双窗口),或对同一 hwnd 并发 SetParent / SetWindowPos(父子关系与几何错乱)。
/// 本锁把"查窗口 → 创建/复用 → 注入"串行为一个原子整体。
///
/// 锁粒度约定:只锁注入/重建段,不锁素材读取(scan / resolve_material_url 等 IO
/// 一律在锁外);`attach_to_workerw[_at]` / `detach_from_workerw` 是原始 Win32
/// 封装,自身不取锁,调用方必须已持有本锁。
static INJECT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 获取进程内全局注入锁(探针线程与命令线程共用,天然跨线程互斥)
pub fn inject_lock() -> &'static Mutex<()> {
    INJECT_LOCK.get_or_init(|| Mutex::new(()))
}

// ---------------------------------------------------------------------------
// 素材目录与列表
// ---------------------------------------------------------------------------

/// 默认图片目录:%USERPROFILE%\Pictures(缺失时回退公共 Pictures,与 2.4 同款)
pub fn default_pictures_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|u| Path::new(&u).join("Pictures"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public\Pictures"))
}

/// 动态壁纸素材目录回退链(纯函数,可单测):
/// 配置 wallpaper_dynamic_dir → 2.4 wallpaper_dir → %USERPROFILE%\Pictures
pub fn pick_dynamic_dir(configured: Option<String>, fallback_2_4: Option<String>) -> PathBuf {
    match non_empty(configured) {
        Some(dir) => PathBuf::from(dir),
        None => match non_empty(fallback_2_4) {
            Some(dir) => PathBuf::from(dir),
            None => default_pictures_dir(),
        },
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// 扩展名是否命中动态壁纸白名单(大小写不敏感)
fn ext_whitelisted(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| DYNAMIC_EXT_WHITELIST.iter().any(|w| e.eq_ignore_ascii_case(w)))
}

/// 素材类型(纯函数):"image" | "video" | "other"
/// 安全加固:html 归为 "other"(已移出白名单,任何路径都不可能产生 "html" 状态)
pub fn material_kind(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "mp4" | "webm" | "avi" | "mov" => "video",
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif" => "image",
        _ => "other", // html/htm 与未知扩展名均不可用作壁纸素材
    }
}

/// 扫描动态壁纸素材目录:白名单扩展名 + 非隐藏 + 常规文件,按名称排序截 100(纯函数,可单测);
/// 目录不存在/无权限 → 空列表(前端显示错误提示),不 panic
pub fn scan_dynamic_dir(dir: &Path) -> Vec<WallpaperEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for item in rd.flatten() {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = item.file_name().to_str().map(String::from) else {
            continue; // 非 UTF-8 文件名跳过
        };
        if name.starts_with('.') {
            continue; // 隐藏文件
        }
        if !ext_whitelisted(&name) {
            continue;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        out.push(WallpaperEntry { name, path: path.to_string_lossy().into_owned(), size });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.truncate(MAX_DYNAMIC_LIST);
    out
}

/// 校验动态壁纸素材:绝对路径 + 白名单扩展名 + 文件存在(纯函数,可单测)
pub fn validate_set_args(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("壁纸素材路径为空".to_string());
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("壁纸素材路径必须是绝对路径: {path}"));
    }
    if !ext_whitelisted(path) {
        return Err(format!(
            "不支持的素材格式,仅支持 mp4/webm/avi/mov/jpg/png/webp/bmp/gif: {path}"
        ));
    }
    if !p.is_file() {
        return Err(format!("壁纸素材文件不存在: {path}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// WorkerW 注入(设计 §1.3 经典三连,全部 Win32_UI_WindowsAndMessaging,零新 feature)
// ---------------------------------------------------------------------------

/// 触发系统创建 WorkerW:向 Progman 发送 WM_SPAWN_WORKERW(0x052C)。
/// 发送失败(空句柄/超时)不视为致命——桌面可能已处于目标结构,继续枚举即可
fn spawn_workerw(progman: HWND) -> bool {
    if progman.is_null() {
        return false;
    }
    let mut result: usize = 0;
    // SendMessageTimeoutW:同步等待 Progman 处理完(超时 1s),防止注入时桌面结构未就绪
    let rc = unsafe {
        SendMessageTimeoutW(
            progman,
            WM_SPAWN_WORKERW,
            0,
            0,
            SMTO_NORMAL,
            SPAWN_TIMEOUT_MS,
            &mut result,
        )
    };
    rc != 0
}

/// EnumWindows 回调(分裂结构兜底):找类名 "WorkerW" 且含子窗口 "SHELLDLL_DefView" 的窗口
/// (即桌面图标层的宿主),结果写入 LPARAM 携带的 Option<HWND>
unsafe extern "system" fn enum_workerw_with_defview(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam as *mut Option<HWND>) };
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if len == 0 {
        return 1; // 无类名,继续枚举
    }
    let class = String::from_utf16_lossy(&buf[..len as usize]);
    if class != "WorkerW" {
        return 1;
    }
    // 该 WorkerW 的子窗口中是否有桌面图标层(SHELLDLL_DefView)
    let defview = unsafe {
        FindWindowExW(
            hwnd,
            std::ptr::null_mut(),
            windows_sys::core::w!("SHELLDLL_DefView"),
            std::ptr::null(),
        )
    };
    if !defview.is_null() {
        *out = Some(hwnd);
        return 0; // 找到即停止枚举
    }
    1
}

/// 分裂结构兜底:枚举顶层窗口,找含 SHELLDLL_DefView 子窗口的 WorkerW
/// (设计文档 §1.3 描述的"DefView 父窗口即 WorkerW"场景;Win10/11 实测
/// DefView 直接挂在 Progman 下,通常走不到这里)
fn find_workerw_via_defview() -> Option<HWND> {
    let mut result: Option<HWND> = None;
    unsafe {
        EnumWindows(Some(enum_workerw_with_defview), &mut result as *mut Option<HWND> as LPARAM);
    }
    result
}

/// 查找 WorkerW 壁纸层窗口(纯函数,可单测不 panic)。
///
/// ⚠️ 本机实测(Win11)与设计文档 §1.3 描述的"EnumWindows 找顶层 SHELLDLL_DefView"
/// 不符:实际桌面结构中 **SHELLDLL_DefView 是 Progman 的子窗口**(非顶层,EnumWindows
/// 永远枚举不到它);0x052C 后 Progman 新增一个**子 WorkerW**——它才是壁纸层
/// (在 DefView 之下渲染,壁纸自然显示在图标后面)。查找顺序:
/// ① Progman 的子 WorkerW(Windows 10/11 主路径,本项目最低支持 Win10 21H2);
/// ② 分裂结构兜底:含 SHELLDLL_DefView 子窗口的顶层 WorkerW(Win7/8 风格)。
/// 两轮未果返回 None(由调用方报错提示)
pub fn find_workerw(progman: HWND) -> Option<HWND> {
    if progman.is_null() {
        return None;
    }
    for _ in 0..2 {
        spawn_workerw(progman);
        // ① Win10/11 主路径:Progman 的子 WorkerW 即壁纸层
        let child = unsafe {
            FindWindowExW(
                progman,
                std::ptr::null_mut(),
                windows_sys::core::w!("WorkerW"),
                std::ptr::null(),
            )
        };
        if !child.is_null() {
            return Some(child);
        }
        // ② 分裂结构兜底
        if let Some(workerw) = find_workerw_via_defview() {
            return Some(workerw);
        }
    }
    None
}

/// 把 webview 窗口注入 WorkerW 壁纸层并铺满主屏(仅主屏,Phase4 不做多屏)。
/// 注入失败返回错误,窗口保持原样(不残留半注入状态)。
/// ⚠️ 调用方必须已持有 [inject_lock](M10 并发修复):SetParent 是全局桌面状态,
/// 与探针线程/多屏重建并发时父子关系会错乱
pub fn attach_to_workerw(hwnd: HWND, width: i32, height: i32) -> Result<(), String> {
    // 1) 拿桌面窗口 Progman(Windows 桌面宿主,自 Vista 起稳定存在)
    let progman = unsafe { FindWindowW(windows_sys::core::w!("Progman"), std::ptr::null()) };
    if progman.is_null() {
        return Err("未找到桌面窗口(Progman),请确认资源管理器运行中".to_string());
    }
    // 2) 触发并查找 WorkerW
    let workerw = find_workerw(progman)
        .ok_or_else(|| "WorkerW 壁纸层查找失败(桌面结构异常,建议重启资源管理器)".to_string())?;
    // 3) 注入:SetParent 到 WorkerW + 铺满主屏(不激活、立即显示、置兄弟最底)
    unsafe {
        // SetParent:把壁纸窗口挂到 WorkerW 下,成为桌面图标层的兄弟
        SetParent(hwnd, workerw);
        // SetWindowPos:铺满主屏;HWND_BOTTOM 压到兄弟窗口最底(兜底路径下保证图标在上);
        // SWP_NOACTIVATE 不抢焦点,SWP_SHOWWINDOW 立即显示
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
    Ok(())
}

/// 从 WorkerW 撤下:SetParent 回 null(恢复为独立顶层窗口)+ 隐藏。
/// 不碰系统壁纸——WorkerW 只是"盖在图标后面的一层",撤掉即恢复原壁纸显示(设计 §1.3)
/// ⚠️ 调用方必须已持有 [inject_lock](M10 并发修复):与注入同锁串行,防并发父子切换
pub fn detach_from_workerw(hwnd: HWND) {
    unsafe {
        SetParent(hwnd, std::ptr::null_mut());
        ShowWindow(hwnd, SW_HIDE);
    }
}

// ---------------------------------------------------------------------------
// 多屏(设计 §2):显示器枚举 + 坐标化注入。
// WorkerW 覆盖整个虚拟桌面:多屏无需每屏找独立 WorkerW,每屏窗口 SetParent
// 到同一 Progman 子 WorkerW 后,SetWindowPos 定位到该屏虚拟桌面坐标即可。
// ---------------------------------------------------------------------------

/// 显示器信息(虚拟桌面物理坐标;index = enum_monitors 排序后的序号,主屏恒为 0)
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MonitorInfo {
    pub index: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

/// 枚举全部显示器:tauri available_monitors(物理像素,免手写 FFI)。
/// 排序 = 主屏优先 + (x, y) 升序稳定排,index 即排序序号。
/// 枚举失败(无桌面/异常)→ 返回仅主屏的兜底 1920x1080(与 attach 现状一致,不 panic)
pub fn enum_monitors(app: &tauri::AppHandle) -> Vec<MonitorInfo> {
    let Ok(mons) = app.available_monitors() else {
        return vec![MonitorInfo { index: 0, x: 0, y: 0, width: 1920, height: 1080, primary: true }];
    };
    // 主屏判定:tauri Monitor 无 is_primary,用 primary_monitor() 的 position 比对
    // (grep 本机 tauri-2.11.5 src/window/mod.rs 确认 API,勿凭记忆)
    let primary_pos = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| (m.position().x, m.position().y));
    let mut out: Vec<MonitorInfo> = mons
        .iter()
        .map(|m| {
            MonitorInfo {
                index: 0,
                x: m.position().x, // 虚拟桌面坐标(引用字段,自动解引用)
                y: m.position().y,
                width: m.size().width as i32, // 物理像素
                height: m.size().height as i32,
                primary: Some((m.position().x, m.position().y)) == primary_pos,
            }
        })
        .collect();
    out.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.x.cmp(&b.x)).then(a.y.cmp(&b.y)));
    for (i, m) in out.iter_mut().enumerate() {
        m.index = i as u32;
    }
    out
}

/// 布局签名(纯函数,热插拔检测用):屏数 + 每屏 (x,y,w,h,primary) 排序后拼接。
/// 排序保证枚举顺序变化不误触发;任一屏移动/改分辨率/增减屏都会改变签名
pub fn layout_signature(monitors: &[MonitorInfo]) -> String {
    let mut sigs: Vec<String> = monitors
        .iter()
        .map(|m| format!("{},{},{},{},{}", m.x, m.y, m.width, m.height, m.primary))
        .collect();
    sigs.sort();
    format!("{}|{}", monitors.len(), sigs.join(";"))
}

/// 把 webview 窗口注入 WorkerW 并定位到虚拟桌面指定 rect(x/y 为虚拟桌面坐标)。
/// 与 attach_to_workerw 同三步,仅 SetWindowPos 坐标化(多屏场景每屏窗口各自定位)。
/// 注入失败返回错误,窗口保持原样
/// ⚠️ 调用方必须已持有 [inject_lock](M10 并发修复)
pub fn attach_to_workerw_at(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) -> Result<(), String> {
    let progman = unsafe { FindWindowW(windows_sys::core::w!("Progman"), std::ptr::null()) };
    if progman.is_null() {
        return Err("未找到桌面窗口(Progman),请确认资源管理器运行中".to_string());
    }
    let workerw = find_workerw(progman)
        .ok_or_else(|| "WorkerW 壁纸层查找失败(桌面结构异常,建议重启资源管理器)".to_string())?;
    unsafe {
        SetParent(hwnd, workerw);
        SetWindowPos(hwnd, HWND_BOTTOM, x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 多屏状态与重建(设计 §2.2/§2.3):每屏素材内存状态 + 窗口创建/注入/重建
// ---------------------------------------------------------------------------

/// 每屏素材内存状态(index → 素材;多屏模式使用;与单值状态并存,多屏关时单值照旧)
static MONITOR_STATES: OnceLock<Mutex<Vec<Option<WallpaperState>>>> = OnceLock::new();

/// 写指定屏素材状态(自动扩容)
pub fn set_monitor_state(index: u32, st: WallpaperState) -> Result<(), String> {
    let mut g = MONITOR_STATES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "壁纸状态锁失败".to_string())?;
    if g.len() <= index as usize {
        g.resize(index as usize + 1, None);
    }
    g[index as usize] = Some(st);
    Ok(())
}

/// 读全部屏素材状态(缺屏 = None;锁失败/未初始化 = 空)
pub fn monitor_states() -> Vec<Option<WallpaperState>> {
    MONITOR_STATES
        .get()
        .map(|m| m.lock().ok().map(|g| g.clone()).unwrap_or_default())
        .unwrap_or_default()
}

/// 清空全部屏素材状态(多屏 clear 用;锁失败静默)
pub fn clear_monitor_states() {
    if let Some(m) = MONITOR_STATES.get() {
        if let Ok(mut g) = m.lock() {
            g.clear();
        }
    }
}

/// 创建/复用指定屏的壁纸窗口并注入(公开入口,自取注入锁):
/// 窗口不存在 → WebviewWindowBuilder 创建(label = wallpaper_<index>,加载 index.html,
/// 前端按 window label 分流渲染,与主屏 wallpaper 窗口同配置:无边框/不透明/不置顶/
/// 不可缩放/skipTaskbar/不抢焦点);
/// 窗口存在 → 直接 set_size(该屏物理尺寸)+ set_position(虚拟坐标)+ show + attach
pub fn apply_monitor(app: &tauri::AppHandle, index: u32) -> Result<(), String> {
    // M10 并发修复:整段注入持锁,防与 multi_apply / 探针线程同 label 双创建、
    // 同 hwnd 并发 SetParent(竞态场景详见 inject_lock 注释)
    let _guard = inject_lock().lock().unwrap_or_else(|p| p.into_inner());
    apply_monitor_locked(app, index)
}

/// 同上,但要求调用方已持有注入锁(multi_apply 整体持锁时走这里,避免重入死锁)
fn apply_monitor_locked(app: &tauri::AppHandle, index: u32) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let label = crate::commands::wallpaper_dynamic::monitor_window_label(index);
    let mons = enum_monitors(app);
    let Some(mon) = mons.iter().find(|m| m.index == index) else {
        return Err(format!("显示器 {index} 不存在(当前共 {} 台)", mons.len()));
    };
    let win = if let Some(w) = app.get_webview_window(&label) {
        w
    } else {
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .decorations(false)
            .transparent(false)
            .resizable(false)
            .skip_taskbar(true)
            .visible(false)
            .focused(false) // 铁律:不抢焦点(grep tauri-2.11.5 源码确认存在)
            .build()
            .map_err(|e| format!("创建壁纸窗口 {label} 失败: {e}"))?
    };
    let hwnd = win.hwnd().map_err(|e| format!("获取壁纸窗口句柄失败: {e}"))?;
    let _ = win.set_size(tauri::PhysicalSize::new(mon.width as u32, mon.height as u32));
    let _ = win.set_position(tauri::PhysicalPosition::new(mon.x, mon.y));
    let _ = win.show();
    let _ = win.set_always_on_top(false);
    attach_to_workerw_at(hwnd.0 as *mut core::ffi::c_void, mon.x, mon.y, mon.width, mon.height)
}

/// 多屏整体重建(设置变更/热插拔后调用;错误聚合为一条,不中断其他屏):
/// 多屏关 → 撤下并销毁全部副屏窗口(主屏保持现状);
/// 多屏开 → 有素材的屏逐个注入;主屏无素材 → 撤下主屏注入(系统壁纸可见)
pub fn multi_apply(app: &tauri::AppHandle) -> Result<(), String> {
    // M10 并发修复:配置与显示器枚举是只读操作留在锁外;撤下/销毁/注入整段持锁。
    // 本函数是探针线程(2s 轮询)与 set/clear/热生效命令的共同入口,锁把
    // "查窗口→创建/复用→SetParent"串行为原子整体;内部改走 apply_monitor_locked
    // (重入会死锁,见 inject_lock 注释)。
    let cfg = crate::commands::config::load_from(&crate::commands::config::config_path(app));
    let mons = enum_monitors(app);
    let _guard = inject_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut errors: Vec<String> = Vec::new();
    if !cfg.wallpaper_multi_monitor {
        for m in mons.iter().filter(|m| m.index > 0) {
            let label = crate::commands::wallpaper_dynamic::monitor_window_label(m.index);
            if let Some(w) = app.get_webview_window(&label) {
                if let Ok(hwnd) = w.hwnd() {
                    detach_from_workerw(hwnd.0 as *mut core::ffi::c_void);
                }
                let _ = w.destroy();
            }
        }
        return Ok(()); // 主屏保持现状(由 4.1 set/attach 路径管理)
    }
    let states = monitor_states();
    for m in &mons {
        let st = states.get(m.index as usize).and_then(|s| s.clone());
        match st {
            // 历史遗留:kind == "html" 曾是分支之一(html 曾属壁纸白名单),2026-08-13
            // 安全加固已把 html 移出白名单(material_kind 只返回 image/video/other),
            // 状态机不可能写入 "html" —— 只保留 video 可注入,其余(图片/other)走系统壁纸
            Some(st) if st.kind == "video" => {
                if let Err(e) = apply_monitor_locked(app, m.index) {
                    errors.push(format!("屏 {} 注入失败: {e}", m.index));
                }
            }
            _ => {
                // 无素材(或图片素材走系统壁纸):撤下该屏注入,系统壁纸可见
                let label = crate::commands::wallpaper_dynamic::monitor_window_label(m.index);
                if let Some(w) = app.get_webview_window(&label) {
                    if let Ok(hwnd) = w.hwnd() {
                        detach_from_workerw(hwnd.0 as *mut core::ffi::c_void);
                    }
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

// ---------------------------------------------------------------------------
// 电池降载(设计 §1.4,Win32_System_Power)
// ---------------------------------------------------------------------------

/// 电池降载判定(设计 §1.4 原文,纯函数,可单测):
/// - ACLineStatus:1 = AC 供电,0 = 电池;
/// - BatteryFlag:8 = 充电中(拔电源但插着充电线也算充电 → 不降载),255 = 未知;
/// - BatteryLifePercent:0-100,255 = 未知。
/// 未知电量(255)在默认阈值 0 下按"用电池即降载"(255<=0 分支由 threshold==0 短路覆盖);
/// 显式阈值下未知电量视为高于阈值(不误降载,宁可多耗电不可打断壁纸)
pub fn is_battery_mode(st: &SYSTEM_POWER_STATUS, threshold_pct: u8) -> bool {
    if st.ACLineStatus == 1 {
        return false; // 接电源
    }
    if st.BatteryFlag == 8 {
        return false; // 充电中
    }
    if threshold_pct == 0 {
        return true; // 默认:用电池即降载(含电量未知 255)
    }
    st.BatteryLifePercent <= threshold_pct // 阈值模式:电量低于阈值才降载
}

/// `wallpaper-power` 事件 payload(公共契约,见设计文档 §0.3;Settings 区块也会消费)
#[derive(Clone, Debug, serde::Serialize)]
pub struct WallpaperPowerPayload {
    pub on_battery: bool,
}

/// 最新电池状态(None = 尚无检测结果,视为非降载;get_state 命令层读取)
pub fn battery_latest() -> bool {
    BATTERY_LATEST
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| *g)
        .unwrap_or(false)
}

/// 电池检测线程只启动一次(幂等)
static BATTERY_WATCHER_ONCE: Once = Once::new();
/// 最近一次检测到的电池状态(跨线程共享)
static BATTERY_LATEST: OnceLock<Mutex<Option<bool>>> = OnceLock::new();

/// 启动电池检测线程(常驻,幂等;集成 agent 在 lib.rs setup 中调用):
/// - 线程每轮重读配置(热生效:enable_dynamic_wallpaper / wallpaper_battery_downshift /
///   threshold / check_sec 运行时改配置即时生效,无需重启);
/// - enable_dynamic_wallpaper 或 wallpaper_battery_downshift 关闭 → 本轮跳过检测
///   (保留上次状态,不翻转不广播;开关再开自动恢复);
/// - 每 wallpaper_battery_check_sec(默认 30)s 调 GetSystemPowerStatus(轻量轮询,无窗口/无 COM);
/// - 仅状态翻转时 emit `wallpaper-power{on_battery}`(不变化不广播,防轮询风暴);
/// - 线程随进程结束,无句柄/线程泄漏(托盘退出由集成收尾验证)。
pub fn spawn_battery_watcher(app: AppHandle, cfg: &crate::commands::config::AppConfig) {
    BATTERY_WATCHER_ONCE.call_once(|| {
        // 首轮立即检测并 emit(不必等满一个周期,设置区徽标/壁纸窗口立即可见)
        let handle = app.clone();
        check_battery(&handle, cfg.wallpaper_battery_threshold_pct);
        if let Err(e) = std::thread::Builder::new()
            .name("battery-watcher".to_string())
            .spawn(move || loop {
                // 热生效:每轮重读配置(开关/阈值/周期改动即时生效,不重启)
                let cfg = crate::commands::config::load_from(
                    &crate::commands::config::config_path(&handle),
                );
                let interval =
                    Duration::from_secs(cfg.wallpaper_battery_check_sec.max(1) as u64);
                std::thread::sleep(interval);
                if !cfg.enable_dynamic_wallpaper || !cfg.wallpaper_battery_downshift {
                    continue; // 开关关:跳过检测(保留上次状态,不翻转不广播)
                }
                check_battery(&handle, cfg.wallpaper_battery_threshold_pct);
            })
        {
            eprintln!("[aurora] 启动 battery-watcher 线程失败: {e}");
        }
    });
}

/// 单轮检测:读电源状态 → 判定 → 仅状态翻转时 emit(读取失败保持上次状态)
fn check_battery(app: &AppHandle, threshold: u8) {
    let mut st: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetSystemPowerStatus(&mut st) };
    if ok == 0 {
        return; // 读取失败:保持上次状态,不翻转不广播
    }
    let on_battery = is_battery_mode(&st, threshold);
    let Some(mut m) = BATTERY_LATEST.get_or_init(|| Mutex::new(None)).lock().ok() else {
        return;
    };
    if *m == Some(on_battery) {
        return; // 状态未变化,不广播(防轮询风暴)
    }
    *m = Some(on_battery);
    drop(m);
    let _ = app.emit("wallpaper-power", &WallpaperPowerPayload { on_battery });
}

// ---------------------------------------------------------------------------
// 当前素材内存状态(静态 Mutex,无需 manage;set/clear/get_state 共享)
// ---------------------------------------------------------------------------

/// 当前动态壁纸素材记录(内存态;图片与视频/html 均记录,供 get_state 与幂等判断)
#[derive(Clone, Debug)]
pub struct WallpaperState {
    pub path: String,
    pub kind: String,        // "image" | "video" | "html"
    pub url: Option<String>, // video/html 目录外素材的 data URL;Pictures 内为 None(前端 convertFileSrc)
}

fn state_slot() -> &'static Mutex<Option<WallpaperState>> {
    static STATE: OnceLock<Mutex<Option<WallpaperState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// 读当前素材状态(无记录 → None)
pub fn current_state() -> Option<WallpaperState> {
    state_slot().lock().ok().and_then(|g| g.clone())
}

/// 写当前素材状态(None = 清除)
pub fn set_state(st: Option<WallpaperState>) {
    if let Ok(mut g) = state_slot().lock() {
        *g = st;
    }
}

// ---------------------------------------------------------------------------
// URL 两段式(设计 §1.5/AD-2):Pictures 内走 asset 协议;目录外走 data URL
// ---------------------------------------------------------------------------

/// 素材 MIME(仅视频需要 data URL;图片不走到这里,兜底 video/mp4)。
/// 安全加固:html/htm 不再映射(已移出白名单,resolve_material_url 先拒绝)
pub fn mime_for_path(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        _ => "video/mp4",
    }
}

/// path 是否位于 dir 内(规范化后前缀判断;两侧都解析失败视为不在内)
fn path_in_dir(path: &Path, dir: &Path) -> bool {
    let (Ok(p), Ok(d)) = (std::fs::canonicalize(path), std::fs::canonicalize(dir)) else {
        return false;
    };
    p.starts_with(&d)
}

/// 素材 URL 两段式(纯函数,可单测):
/// 素材在默认图片目录(Pictures)内 → Ok(None)(前端用 convertFileSrc 走 asset 协议);
/// 目录外 → 后端读文件转 base64 data URL(≤20MB,超出报错提示放 Pictures 下)。
/// 安全加固:html/htm 一律拒绝——asset 协议加载 html 与前端同源可注入 IPC,
/// 即使绕过 validate_set_args 也不能生成 data:text/html URL
pub fn resolve_material_url(path: &str, default_pictures: &Path) -> Result<Option<String>, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "html" || ext == "htm" {
        return Err("不支持 html 素材,请使用图片或视频".to_string());
    }
    if path_in_dir(Path::new(path), default_pictures) {
        return Ok(None);
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("读取素材失败: {e}"))?;
    if meta.len() > MAX_DATA_URL_BYTES {
        return Err("素材超过 20MB,请放到默认图片目录(Pictures)下使用".to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读取素材失败: {e}"))?;
    Ok(Some(format!(
        "data:{};base64,{}",
        mime_for_path(path),
        base64_encode(&bytes)
    )))
}

/// base64 编码(RFC 4648;无依赖手写——Cargo.toml 属集成 agent 不可加依赖,
/// 与 2.5 手写等价 FFI 同风格;纯函数,标准向量单测覆盖)
pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

// ---------------------------------------------------------------------------
// 单元测试(纯函数全覆盖;注入动作本身无法单测,由 §1.6 手动验收覆盖)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::core::w;

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("aurora_wp_dyn_{tag}_{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn touch(p: &Path) {
        std::fs::write(p, b"fake bytes").unwrap();
    }

    /// 构造 SYSTEM_POWER_STATUS(ACLineStatus / BatteryFlag / BatteryLifePercent)
    fn sp(ac: u8, flag: u8, pct: u8) -> SYSTEM_POWER_STATUS {
        SYSTEM_POWER_STATUS {
            ACLineStatus: ac,
            BatteryFlag: flag,
            BatteryLifePercent: pct,
            SystemStatusFlag: 0,
            BatteryLifeTime: 0,
            BatteryFullLifeTime: 0,
        }
    }

    // ---- is_battery_mode 全分支(设计 §1.4) ----

    #[test]
    fn battery_ac_power_never_downshift() {
        assert!(!is_battery_mode(&sp(1, 1, 100), 0)); // AC + 高电量
        assert!(!is_battery_mode(&sp(1, 255, 255), 0)); // AC + 全未知
        assert!(!is_battery_mode(&sp(1, 8, 50), 20)); // AC + 充电中
    }

    #[test]
    fn battery_charging_never_downshift() {
        assert!(!is_battery_mode(&sp(0, 8, 50), 0)); // 电池但充电中(插着充电线)
        assert!(!is_battery_mode(&sp(0, 8, 255), 20));
        assert!(!is_battery_mode(&sp(0, 8, 5), 10)); // 电量低于阈值但充电中 → 不降载
    }

    #[test]
    fn battery_threshold_zero_downshift_on_battery() {
        assert!(is_battery_mode(&sp(0, 1, 100), 0)); // 默认阈值 0:高电量也用电池即降载
        assert!(is_battery_mode(&sp(0, 255, 255), 0)); // 电量未知(255)按阈值 0 规则 → 降载
        assert!(is_battery_mode(&sp(0, 1, 0), 0));
    }

    #[test]
    fn battery_threshold_percent_boundary() {
        assert!(is_battery_mode(&sp(0, 1, 20), 20)); // == 阈值 → 降载(边界)
        assert!(is_battery_mode(&sp(0, 1, 19), 20)); // 低于阈值 → 降载
        assert!(!is_battery_mode(&sp(0, 1, 21), 20)); // 高于阈值 → 不降载
        assert!(!is_battery_mode(&sp(0, 1, 255), 20)); // 未知电量 + 显式阈值 → 不误降载
    }

    // ---- 素材列表过滤(设计 §1.6) ----

    #[test]
    fn scan_filters_dynamic_whitelist_sorted() {
        let dir = tmp_dir("scan");
        touch(&dir.join("b.mp4"));
        touch(&dir.join("A.WEBM")); // 大写扩展名
        touch(&dir.join("c.html")); // 安全加固:html 已移出白名单,必须被过滤
        touch(&dir.join("d.jpg"));
        touch(&dir.join("e.gif"));
        touch(&dir.join("f.avi"));
        touch(&dir.join("g.mov"));
        touch(&dir.join("note.txt")); // 非白名单
        touch(&dir.join("h.bin"));
        touch(&dir.join(".hidden.mp4")); // 隐藏文件
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        touch(&dir.join("subdir/inside.mp4")); // 不递归子目录
        let list = scan_dynamic_dir(&dir);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["A.WEBM", "b.mp4", "d.jpg", "e.gif", "f.avi", "g.mov"]);
        assert!(!names.contains(&"c.html"), "html 素材必须被过滤");
        assert!(list.iter().all(|e| e.size > 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_missing_dir_returns_empty() {
        let dir = tmp_dir("missing");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(scan_dynamic_dir(&dir).is_empty());
    }

    #[test]
    fn scan_truncates_to_100() {
        let dir = tmp_dir("cap");
        for i in 0..105 {
            touch(&dir.join(format!("mat_{i:03}.mp4")));
        }
        let list = scan_dynamic_dir(&dir);
        assert_eq!(list.len(), MAX_DYNAMIC_LIST);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 目录回退链(设计 §1.1) ----

    #[test]
    fn pick_dir_fallback_chain() {
        let pictures = default_pictures_dir();
        // 配置优先
        assert_eq!(
            pick_dynamic_dir(Some(r"D:\dynamic".to_string()), None),
            PathBuf::from(r"D:\dynamic")
        );
        // 配置为空 → 2.4 wallpaper_dir
        assert_eq!(
            pick_dynamic_dir(None, Some(r"D:\wp".to_string())),
            PathBuf::from(r"D:\wp")
        );
        // 配置为空串/纯空白 → 2.4 wallpaper_dir
        assert_eq!(
            pick_dynamic_dir(Some("   ".to_string()), Some(r"D:\wp".to_string())),
            PathBuf::from(r"D:\wp")
        );
        // 两者皆空 → %USERPROFILE%\Pictures
        assert_eq!(pick_dynamic_dir(None, None), pictures);
        assert_eq!(pick_dynamic_dir(Some(" ".to_string()), None), pictures);
    }

    // ---- set 校验(设计 §1.6) ----

    #[test]
    fn set_rejects_relative_path() {
        assert!(validate_set_args(r"Pictures\a.mp4").is_err());
    }

    #[test]
    fn set_rejects_missing_file() {
        let dir = tmp_dir("missing_set");
        let p = dir.join("none.mp4");
        assert!(validate_set_args(&p.to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_rejects_non_whitelist_ext() {
        let dir = tmp_dir("badext");
        touch(&dir.join("a.txt"));
        touch(&dir.join("b.exe"));
        assert!(validate_set_args(&dir.join("a.txt").to_string_lossy()).is_err());
        assert!(validate_set_args(&dir.join("b.exe").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_rejects_empty_path() {
        assert!(validate_set_args("").is_err());
        assert!(validate_set_args("   ").is_err());
    }

    #[test]
    fn set_accepts_absolute_whitelist() {
        let dir = tmp_dir("good");
        touch(&dir.join("a.mp4"));
        touch(&dir.join("b.HTML")); // 大小写不敏感,但 html 已移出白名单 → 拒绝
        touch(&dir.join("c.PNG"));
        assert!(validate_set_args(&dir.join("a.mp4").to_string_lossy()).is_ok());
        assert!(validate_set_args(&dir.join("b.HTML").to_string_lossy()).is_err());
        assert!(validate_set_args(&dir.join("c.PNG").to_string_lossy()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 素材类型 ----

    #[test]
    fn kind_classification() {
        assert_eq!(material_kind("a.MP4"), "video");
        assert_eq!(material_kind("a.webm"), "video");
        assert_eq!(material_kind("a.html"), "other", "安全加固:html 归 other,不再支持");
        assert_eq!(material_kind("a.JPG"), "image");
        assert_eq!(material_kind("a.jpeg"), "image");
        assert_eq!(material_kind("a.webp"), "image");
        assert_eq!(material_kind("a.txt"), "other");
    }

    // ---- base64(RFC 4648 标准向量) ----

    #[test]
    fn base64_standard_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(&[0x00, 0x01, 0x02]), "AAEC");
    }

    #[test]
    fn base64_random_binary_roundtrip() {
        // 随机字节编码后按 base64 字符集校验(防 0x00 截断类错误)
        let data: Vec<u8> = (0..256u16).map(|i| (i as u8).wrapping_mul(7)).collect();
        let enc = base64_encode(&data);
        assert!(enc.len() % 4 == 0);
        assert!(enc.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'='));
    }

    // ---- URL 两段式(设计 §1.5) ----

    #[test]
    fn mime_mapping() {
        assert_eq!(mime_for_path("a.mp4"), "video/mp4");
        assert_eq!(mime_for_path("a.WEBM"), "video/webm");
        assert_eq!(mime_for_path("a.avi"), "video/x-msvideo");
        assert_eq!(mime_for_path("a.mov"), "video/quicktime");
        assert_eq!(mime_for_path("a.html"), "video/mp4", "html 不再映射 text/html(已移出白名单)");
        assert_eq!(mime_for_path("a.htm"), "video/mp4");
        assert_eq!(mime_for_path("a.unknown"), "video/mp4");
    }

    #[test]
    fn resolve_url_inside_pictures_returns_none() {
        let pictures = tmp_dir("pics");
        touch(&pictures.join("a.mp4"));
        let r = resolve_material_url(&pictures.join("a.mp4").to_string_lossy(), &pictures);
        assert!(matches!(r, Ok(None)));
        let _ = std::fs::remove_dir_all(&pictures);
    }

    #[test]
    fn resolve_url_outside_pictures_returns_data_url() {
        let pictures = tmp_dir("pics2");
        let outside = tmp_dir("out2");
        std::fs::write(outside.join("v.mp4"), b"testvideo").unwrap();
        let r = resolve_material_url(&outside.join("v.mp4").to_string_lossy(), &pictures);
        // data URL 前缀 + base64("testvideo") 尾缀
        assert!(matches!(&r, Ok(Some(u)) if u.starts_with("data:video/mp4;base64,")));
        assert_eq!(r.unwrap().unwrap(), format!("data:video/mp4;base64,{}", base64_encode(b"testvideo")));
        let _ = std::fs::remove_dir_all(&pictures);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn resolve_url_html_rejected() {
        // 安全加固:html 素材一律拒绝(asset 协议同源注入 IPC 高危面),目录内外都一样
        let pictures = tmp_dir("pics3");
        let outside = tmp_dir("out3");
        std::fs::write(outside.join("page.html"), b"<html></html>").unwrap();
        let r = resolve_material_url(&outside.join("page.html").to_string_lossy(), &pictures);
        assert!(r.is_err(), "目录外 html 必须拒绝");
        std::fs::write(pictures.join("page.htm"), b"<html></html>").unwrap();
        let r = resolve_material_url(&pictures.join("page.htm").to_string_lossy(), &pictures);
        assert!(r.is_err(), "Pictures 内 htm 也必须拒绝");
        let _ = std::fs::remove_dir_all(&pictures);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn resolve_url_oversize_rejected() {
        let pictures = tmp_dir("pics4");
        let outside = tmp_dir("out4");
        let big = outside.join("big.mp4");
        std::fs::write(&big, vec![0u8; (MAX_DATA_URL_BYTES + 1) as usize]).unwrap();
        let r = resolve_material_url(&big.to_string_lossy(), &pictures);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("20MB"));
        let _ = std::fs::remove_dir_all(&pictures);
        let _ = std::fs::remove_dir_all(&outside);
    }

    // ---- find_workerw 不 panic(注入动作本身手动验收覆盖,设计 §1.6) ----

    #[test]
    fn find_workerw_null_progman_no_panic() {
        // 空窗口句柄场景:不应 panic(发送失败即返回,枚举照常进行)
        let _ = find_workerw(std::ptr::null_mut());
    }

    #[test]
    fn find_workerw_real_desktop() {
        // 真实桌面:Progman 存在时应能找到 WorkerW(本机即 Windows 桌面环境);
        // 无桌面环境(CI/远程会话异常)直接跳过
        let progman = unsafe { FindWindowW(w!("Progman"), std::ptr::null()) };
        if progman.is_null() {
            return;
        }
        let workerw = find_workerw(progman);
        assert!(workerw.is_some(), "本机桌面结构异常:WorkerW 应可找到");
        assert!(!workerw.unwrap().is_null());
    }

    // ---- Phase5 5.2 多屏(设计文档 §2):纯函数可单测,枚举/注入动作手动验收 ----

    #[test]
    fn layout_signature_changes_when_monitors_change() {
        let a = vec![
            MonitorInfo { index: 0, x: 0, y: 0, width: 1920, height: 1080, primary: true },
            MonitorInfo { index: 1, x: 1920, y: 0, width: 1280, height: 720, primary: false },
        ];
        let b = vec![MonitorInfo { index: 0, x: 0, y: 0, width: 1920, height: 1080, primary: true }];
        assert_ne!(layout_signature(&a), layout_signature(&b));
    }

    #[test]
    fn layout_signature_order_independent() {
        let a = vec![
            MonitorInfo { index: 0, x: 0, y: 0, width: 1920, height: 1080, primary: true },
            MonitorInfo { index: 1, x: 1920, y: 0, width: 1280, height: 720, primary: false },
        ];
        let mut b = a.clone();
        b.reverse(); // 枚举顺序翻转(热插拔偶发),签名必须不变
        assert_eq!(layout_signature(&a), layout_signature(&b));
    }

    #[test]
    fn layout_signature_changes_on_position_move() {
        let a = vec![
            MonitorInfo { index: 0, x: 0, y: 0, width: 1920, height: 1080, primary: true },
            MonitorInfo { index: 1, x: 1920, y: 0, width: 1280, height: 720, primary: false },
        ];
        let mut b = a.clone();
        b[1].x = 3840; // 副屏挪到更右边
        assert_ne!(layout_signature(&a), layout_signature(&b));
    }
}
