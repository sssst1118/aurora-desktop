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
