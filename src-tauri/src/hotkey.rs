use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 呼出/隐藏搜索框的全局热键
pub const SEARCH_HOTKEY: &str = "ctrl+shift+space";

/// 2.2 抽屉窗口呼出/隐藏(热键与托盘共用入口)
pub fn toggle_drawer_window(app: &AppHandle) {
    toggle_window(app, "drawer");
}

/// 2.3 剪贴板历史窗口呼出/隐藏(热键与托盘共用入口)
pub fn toggle_clipboard_window(app: &AppHandle) {
    toggle_window(app, "clipboard");
}

/// 通用窗口显隐切换(Phase1 toggle_search_window 同款手法):
/// 显示时置顶强制 Z 序提升 + Alt 伪按键绕过 Windows 前台锁,保证 WebView 输入可聚焦
fn toggle_window(app: &AppHandle, label: &str) {
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return;
    }
    let _ = win.show();
    let _ = win.set_always_on_top(true);
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

/// 注册全局热键。注册失败(如被系统或其他程序占用)不阻止应用启动,
/// 由调用方决定降级处理,不应导致应用崩溃。
pub fn setup_hotkey(app: &AppHandle) -> Result<(), tauri_plugin_global_shortcut::Error> {
    // 1) Phase1 搜索热键(固定默认值,失败向上传播由 setup 告警,保持 Phase1 行为)
    register_hotkey(app, SEARCH_HOTKEY, |app| crate::win_utils::toggle_search_window(app))?;

    // 2) Phase2 抽屉/剪贴板热键:从配置读取,受模块开关控制,注册冲突仅告警、互不阻塞
    let cfg = crate::commands::config::load_from(&crate::commands::config::config_path(app));
    if cfg.enable_file_drawer && !cfg.drawer_hotkey.trim().is_empty() {
        let hk = cfg.drawer_hotkey.clone();
        if let Err(e) = register_hotkey(app, &hk, toggle_drawer_window) {
            eprintln!("[aurora] 抽屉热键 {hk} 注册失败(可能被占用): {e}");
        }
    }
    if cfg.enable_clipboard_history && !cfg.hotkey_clipboard.trim().is_empty() {
        let hk = cfg.hotkey_clipboard.clone();
        if let Err(e) = register_hotkey(app, &hk, toggle_clipboard_window) {
            eprintln!("[aurora] 剪贴板热键 {hk} 注册失败(可能被占用): {e}");
        }
    }
    Ok(())
}
