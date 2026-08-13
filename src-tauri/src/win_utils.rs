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
            show_search_window(app);
        }
    }
}

/// 呼出 search 窗口(只显示不隐藏;热键 toggle 与首次启动引导共用)。
///
/// 呼出时采用参考项目 ZeroLaunch 的"置顶强制提升"手法:
/// show 之后再次置顶(set_always_on_top(true)),利用 SetWindowPos(HWND_TOPMOST)
/// 强制把窗口提到前台,规避 Windows 前台锁下 set_focus 失效的问题;
/// search 窗口本身配置了 alwaysOnTop,重复置顶不改变属性,只强制 Z 序提升。
pub fn show_search_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("search") {
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

/// 记忆窗口位置恢复时的越界保护(纯函数,便于单测):
/// 窗口矩形与任一显示器工作区相交 → 保留原位置;完全在屏幕外
/// (显示器拔掉/分辨率变化)或没有显示器信息 → None,调用方应回退居中。
/// 入参 monitors 为 (x, y, width, height) 逻辑像素四元组。
pub fn clamp_to_visible(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    monitors: &[(i32, i32, i32, i32)],
) -> Option<(i32, i32)> {
    let (x2, y2) = (x.saturating_add(w), y.saturating_add(h));
    let visible = monitors.iter().any(|&(mx, my, mw, mh)| {
        let (mx2, my2) = (mx.saturating_add(mw), my.saturating_add(mh));
        // 交集非空 = 窗口至少有一角还在屏内
        x.max(mx) < x2.min(mx2) && y.max(my) < y2.min(my2)
    });
    visible.then_some((x, y))
}

#[cfg(test)]
mod tests {
    use super::clamp_to_visible;

    /// 单显示器工作区 1920x1080
    fn mono() -> Vec<(i32, i32, i32, i32)> {
        vec![(0, 0, 1920, 1080)]
    }

    #[test]
    fn position_inside_screen_kept() {
        assert_eq!(clamp_to_visible(100, 50, 620, 420, &mono()), Some((100, 50)));
    }

    #[test]
    fn partially_visible_kept() {
        // 窗口一半在屏外,仍保留(用户拖到边缘很正常)
        assert_eq!(clamp_to_visible(1800, 800, 620, 420, &mono()), Some((1800, 800)));
    }

    #[test]
    fn fully_off_screen_returns_none() {
        // 整体在屏幕外(如显示器已拔掉):回退居中
        assert_eq!(clamp_to_visible(3000, 3000, 620, 420, &mono()), None);
        assert_eq!(clamp_to_visible(-800, 0, 620, 420, &mono()), None);
    }

    #[test]
    fn no_monitors_returns_none() {
        assert_eq!(clamp_to_visible(10, 10, 620, 420, &[]), None);
    }

    #[test]
    fn placeholder_compiles() {
        assert!(true);
    }
}
