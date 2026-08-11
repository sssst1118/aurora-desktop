use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 呼出/隐藏搜索框的全局热键
pub const SEARCH_HOTKEY: &str = "ctrl+shift+space";

/// 注册全局热键。注册失败(如被系统或其他程序占用)返回 Err,
/// 由调用方决定降级处理,不应导致应用崩溃。
pub fn setup_hotkey(app: &AppHandle) -> Result<(), tauri_plugin_global_shortcut::Error> {
    app.global_shortcut().on_shortcut(SEARCH_HOTKEY, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            crate::win_utils::toggle_search_window(app);
        }
    })?;
    Ok(())
}
