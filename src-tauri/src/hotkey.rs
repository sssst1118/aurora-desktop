use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::commands::config::AppConfig;

/// 呼出/隐藏搜索框的全局热键
pub const SEARCH_HOTKEY: &str = "ctrl+shift+space";

/// 动态热键当前注册表:用途 → 已注册的键(热生效 diff 依据)。
/// 搜索热键固定注册、不参与 diff;本表只管抽屉/剪贴板/AI 三个可配置热键。
static REGISTERED: Mutex<Option<HashMap<&'static str, String>>> = Mutex::new(None);

fn registered() -> &'static Mutex<Option<HashMap<&'static str, String>>> {
    &REGISTERED
}

/// 2.2 抽屉窗口呼出/隐藏(热键与托盘共用入口)
pub fn toggle_drawer_window(app: &AppHandle) {
    toggle_window(app, "drawer", true);
}

/// 2.3 剪贴板历史窗口呼出/隐藏(热键与托盘共用入口)
pub fn toggle_clipboard_window(app: &AppHandle) {
    toggle_window(app, "clipboard", true);
}

/// Phase3 3.1 AI 对话面板呼出/隐藏(热键与托盘共用入口);
/// ai_panel 设计为不置顶(设计文档 §0.3.5),故 set_top=false
pub fn toggle_ai_panel_window(app: &AppHandle) {
    toggle_window(app, "ai_panel", false);
}

/// 通用窗口显隐切换(Phase1 toggle_search_window 同款手法):
/// 显示时按 set_top 决定是否置顶强制 Z 序提升(drawer/clipboard 配置为置顶,ai_panel 不置顶)
/// + Alt 伪按键绕过 Windows 前台锁,保证 WebView 输入可聚焦
fn toggle_window(app: &AppHandle, label: &str, set_top: bool) {
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return;
    }
    let _ = win.show();
    if set_top {
        let _ = win.set_always_on_top(true);
    }
    unsafe {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            keybd_event, KEYEVENTF_KEYUP, VK_MENU,
        };
        keybd_event(VK_MENU as u8, 0, 0, 0);
        keybd_event(VK_MENU as u8, 0, KEYEVENTF_KEYUP, 0);
    }
    if let Ok(hwnd) = win.hwnd() {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
            let _ = SetForegroundWindow(hwnd.0 as *mut core::ffi::c_void);
        }
    }
    let _ = win.set_focus();
}

/// 注册单个热键,按下时触发 on_press
fn register_hotkey<F>(
    app: &AppHandle,
    shortcut: &str,
    on_press: F,
) -> Result<(), tauri_plugin_global_shortcut::Error>
where
    F: Fn(&AppHandle) + Send + Sync + 'static,
{
    app.global_shortcut().on_shortcut(shortcut, move |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            on_press(app);
        }
    })?;
    Ok(())
}

/// 按配置把可配置热键(抽屉/剪贴板/AI)调到一致状态(热生效 diff,幂等):
/// - 开关关闭或键位为空 → 注销已注册的旧键;
/// - 键位变化 → 注销旧键、注册新键;
/// - 开关开启且键位不变 → 不动(避免重复注册)。
/// 注册失败(如被系统或其他程序占用)仅告警,不影响其他热键与应用运行。
pub fn apply_hotkeys(app: &AppHandle, cfg: &AppConfig) {
    // 目标表:用途 → 开关开且键位非空时的键(键位 trim 后为空 = 未配置)
    let targets: [(&'static str, Option<&str>); 3] = [
        ("drawer", cfg.enable_file_drawer.then_some(cfg.drawer_hotkey.as_str())),
        ("clipboard", cfg.enable_clipboard_history.then_some(cfg.hotkey_clipboard.as_str())),
        ("ai", cfg.enable_ai.then_some(cfg.ai_hotkey.as_str())),
    ];
    let mut binding = registered().lock().unwrap_or_else(|p| p.into_inner());
    let mut g = binding.get_or_insert_with(HashMap::new);

    for (kind, target) in targets {
        let target = target.map(str::trim).filter(|s| !s.is_empty());
        let current = g.get(kind).cloned();
        match (current, target) {
            (Some(old), Some(new)) if old == new => {} // 不变,保持
            (Some(old), Some(new)) => {
                // 键位变了:注销旧的、注册新的
                unregister_shortcut(app, &old);
                if let Err(e) = register_hotkey(app, new, handler_for(kind)) {
                    eprintln!("[aurora] {kind} 热键 {new} 注册失败(可能被占用): {e}");
                } else {
                    g.insert(kind, new.to_string());
                }
            }
            (Some(old), None) => {
                // 开关关闭/键位清空:注销
                unregister_shortcut(app, &old);
                g.remove(kind);
            }
            (None, Some(new)) => {
                if let Err(e) = register_hotkey(app, new, handler_for(kind)) {
                    eprintln!("[aurora] {kind} 热键 {new} 注册失败(可能被占用): {e}");
                } else {
                    g.insert(kind, new.to_string());
                }
            }
            (None, None) => {} // 本来就没有,不动
        }
    }
}

/// 注销已注册热键(字符串键位解析失败则跳过,不阻断)
fn unregister_shortcut(app: &AppHandle, key: &str) {
    if let Ok(sc) = Shortcut::from_str(key) {
        let _ = app.global_shortcut().unregister(sc);
    }
}

/// 用途 → 按键处理函数(抽屉/剪贴板/AI 三窗口)
fn handler_for(kind: &str) -> fn(&AppHandle) {
    match kind {
        "drawer" => toggle_drawer_window,
        "clipboard" => toggle_clipboard_window,
        _ => toggle_ai_panel_window,
    }
}

/// 注册全局热键。注册失败(如被系统或其他程序占用)不阻止应用启动,
/// 由调用方决定降级处理,不应导致应用崩溃。
pub fn setup_hotkey(app: &AppHandle) -> Result<(), tauri_plugin_global_shortcut::Error> {
    // 1) Phase1 搜索热键(固定默认值,失败向上传播由 setup 告警,保持 Phase1 行为)
    register_hotkey(app, SEARCH_HOTKEY, |app| crate::win_utils::toggle_search_window(app))?;

    // 2) 可配置热键(抽屉/剪贴板/AI):与 config_save 后的热生效走同一 diff 逻辑
    let cfg = crate::commands::config::load_from(&crate::commands::config::config_path(app));
    apply_hotkeys(app, &cfg);
    Ok(())
}
