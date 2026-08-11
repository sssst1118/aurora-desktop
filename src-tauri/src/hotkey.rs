use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 呼出/隐藏搜索框的全局热键
pub const SEARCH_HOTKEY: &str = "ctrl+shift+space";

/// 【模块实现后替换】2.2 抽屉窗口呼出/隐藏。
/// 骨架阶段仅注册热键入口,动作占位;模块完成后由集成 agent 接入显隐实现。
fn toggle_drawer_window_stub(_app: &AppHandle) {
    // 占位:2.2 模块完成后替换为 drawer 窗口 show/hide 切换
}

/// 【模块实现后替换】2.3 剪贴板历史窗口呼出/隐藏。
/// 骨架阶段仅注册热键入口,动作占位;模块完成后由集成 agent 接入显隐实现。
fn toggle_clipboard_window_stub(_app: &AppHandle) {
    // 占位:2.3 模块完成后替换为 clipboard 窗口 show/hide 切换
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
        if let Err(e) = register_hotkey(app, &hk, toggle_drawer_window_stub) {
            eprintln!("[aurora] 抽屉热键 {hk} 注册失败(可能被占用): {e}");
        }
    }
    if cfg.enable_clipboard_history && !cfg.hotkey_clipboard.trim().is_empty() {
        let hk = cfg.hotkey_clipboard.clone();
        if let Err(e) = register_hotkey(app, &hk, toggle_clipboard_window_stub) {
            eprintln!("[aurora] 剪贴板热键 {hk} 注册失败(可能被占用): {e}");
        }
    }
    Ok(())
}
