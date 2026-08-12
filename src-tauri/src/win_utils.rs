use std::sync::Mutex;
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
            // 置顶强制 Z 序提升(参考 ZeroLaunch 手法;窗口本就配置置顶,不改变属性)
            let _ = win.set_always_on_top(true);
            // Windows 前台锁:热键呼出时本进程未必持有前台权限,SetForegroundWindow 会被拒,
            // 表现:窗口已显示但不激活,键盘输入进不了 WebView。
            // 经典绕过:注入一次 Alt 键(伪输入)后,系统将"最后输入"归属本进程,随后激活被允许。
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
    }
}

/// 托盘"隐藏全部"时记录可见窗口快照,"显示全部"按快照恢复(恢复后清空)
static HIDDEN_SNAPSHOT: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// 隐藏全部交互窗口(wallpaper 壁纸渲染窗口不受托盘管理,不参与);
/// 隐藏前记录当前可见窗口,供 show_all 恢复
pub fn hide_all(app: &AppHandle) {
    const LABELS: [&str; 5] = ["island", "search", "drawer", "clipboard", "ai_panel"];
    let visible: Vec<String> = LABELS
        .iter()
        .filter(|l| {
            app.get_webview_window(l)
                .map(|w| w.is_visible().unwrap_or(false))
                .unwrap_or(false)
        })
        .map(|s| s.to_string())
        .collect();
    if let Ok(mut g) = HIDDEN_SNAPSHOT.lock() {
        *g = Some(visible);
    }
    for label in LABELS {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.hide();
        }
    }
}

/// 恢复"隐藏全部"那一刻的可见窗口布局(快照),恢复后清空;
/// 未经过隐藏全部(无快照)时不做任何事;期间手动呼出的窗口不受影响
pub fn show_all(app: &AppHandle) {
    let snapshot = HIDDEN_SNAPSHOT.lock().ok().and_then(|mut g| g.take());
    let Some(snapshot) = snapshot else {
        return;
    };
    for label in snapshot {
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.show();
        }
    }
}

/// 全局快捷键"全部显示/隐藏"切换:当前无快照(处于正常状态)→ 隐藏全部并记快照;
/// 已有快照(处于全隐藏状态)→ 按快照恢复。与托盘 hide_all/show_all 共用同一快照,
/// 两种入口互相同步:托盘隐藏后按键恢复、按键隐藏后托盘可恢复。
pub fn toggle_all_windows(app: &AppHandle) {
    let has_snapshot = HIDDEN_SNAPSHOT.lock().map(|g| g.is_some()).unwrap_or(false);
    if has_snapshot {
        show_all(app);
    } else {
        hide_all(app);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_compiles() {
        assert!(true);
    }
}
