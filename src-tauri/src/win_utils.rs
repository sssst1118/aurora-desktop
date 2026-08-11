use tauri::{AppHandle, Manager};

/// 切换 search 窗口显隐(热键触发)。
///
/// 呼出时采用参考项目 ZeroLaunch 的"置顶强制提升"手法:
/// show 之后再次置顶(set_always_on_top(true)),利用 SetWindowPos(HWND_TOPMOST)
/// 强制把窗口提到前台,规避 Windows 前台锁下 set_focus 失效的问题;
/// search 窗口本身配置了 alwaysOnTop,重复置顶不改变属性,只强制 Z 序提升。
pub fn toggle_search_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("search") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_always_on_top(true);
            let _ = win.set_focus();
        }
    }
}

pub fn show_all(app: &AppHandle) {
    for label in ["island", "search"] {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.show();
        }
    }
}

pub fn hide_all(app: &AppHandle) {
    for label in ["island", "search"] {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.hide();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_compiles() {
        assert!(true);
    }
}
