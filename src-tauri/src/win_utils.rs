use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

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
        // Phase6:主面板位置绑定岛——水平居中于岛,垂直在岛下方 12px;越界 clamp
        if let Some(island) = app.get_webview_window("island") {
            if let (Ok(ipos), Ok(isz)) = (island.outer_position(), island.outer_size()) {
                let scale = island.scale_factor().unwrap_or(1.0);
                let ix = ipos.x as f64 / scale;
                let iy = ipos.y as f64 / scale;
                let iw = isz.width as f64 / scale;
                let ih = isz.height as f64 / scale;
                if let Ok(ssize) = win.outer_size() {
                    let sscale = win.scale_factor().unwrap_or(1.0);
                    let w = ssize.width as f64 / sscale;
                    let h = ssize.height as f64 / sscale;
                    let monitors = logical_monitors(app);
                    let px = (ix + iw / 2.0 - w / 2.0).round() as i32;
                    let py = (iy + ih + 12.0).round() as i32;
                    if let Some((cx, cy)) = clamp_to_visible(px, py, w as i32, h as i32, &monitors) {
                        let _ = win.set_position(tauri::LogicalPosition::new(cx, cy));
                    } else {
                        // 完全越界(窗口与所有显示器无交集,如岛被拖出屏或显示器
                        // 热插拔):回退主显示器顶部居中,保证窗口呼出必可见。
                        // 此前 None 分支不做任何事 = "越界不动",窗口可能留在
                        // 屏外完全不可见(2026-08-14 审计 F2-3 修复)
                        if let Some((fx, fy)) = fallback_position(w as i32, h as i32, &monitors) {
                            let _ = win.set_position(tauri::LogicalPosition::new(fx, fy));
                        }
                    }
                }
            }
        }
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

/// 呼出主面板并定位到指定视图(抽屉/剪贴板/AI 热键与托盘入口共用)。
/// 事件在 show 后 emit,前端 MainPanel 监听 "panel-open-view" 切换视图。
pub fn show_panel_with_view(app: &AppHandle, view: &str) {
    show_search_window(app);
    let _ = app.emit("panel-open-view", serde_json::json!({ "view": view }));
}

/// 枚举显示器工作区(物理像素 → 逻辑像素四元组 (x, y, w, h));
/// 呼出定位与位置恢复共用的坐标换算(scale_factor 为 DPI 缩放)
pub(crate) fn logical_monitors(app: &AppHandle) -> Vec<(i32, i32, i32, i32)> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let s = m.scale_factor();
            let p = m.position();
            let sz = m.size();
            (
                (p.x as f64 / s).round() as i32,
                (p.y as f64 / s).round() as i32,
                (sz.width as f64 / s).round() as i32,
                (sz.height as f64 / s).round() as i32,
            )
        })
        .collect()
}

/// 托盘"隐藏全部"时记录可见窗口快照,"显示全部"按快照恢复(恢复后清空)
static HIDDEN_SNAPSHOT: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// 隐藏全部交互窗口(wallpaper 壁纸渲染窗口不受托盘管理,不参与);
/// 隐藏前记录当前可见窗口,供 show_all 恢复
pub fn hide_all(app: &AppHandle) {
    // Phase6 一岛一窗:交互窗口收敛为 island + search(drawer/clipboard/ai_panel 已删除)
    const LABELS: [&str; 2] = ["island", "search"];
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
/// (显示器拔掉/分辨率变化)或没有显示器信息 → None,调用方应回退
/// [fallback_position] 兜底可见位置——注意:None 分支不做任何事等于
/// "越界不动"(窗口可能留在屏外完全不可见),语义与"越界 clamp"不符,
/// 调用方必须处理 None(2026-08-14 审计 F2-3)。
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

/// 完全越界时的兜底可见位置(纯函数,与 [clamp_to_visible] 配套):
/// 主显示器(虚拟桌面坐标中包含原点的那个;找不到则取第一个)顶部居中,
/// y 留 8px 视觉边距。没有显示器信息 → None(调用方放弃定位)。
pub fn fallback_position(
    w: i32,
    _h: i32,
    monitors: &[(i32, i32, i32, i32)],
) -> Option<(i32, i32)> {
    let primary = monitors
        .iter()
        .find(|&&(x, y, mw, mh)| x <= 0 && 0 < x + mw && y <= 0 && 0 < y + mh)
        .or_else(|| monitors.first())?;
    Some((primary.0 + (primary.2 - w) / 2, primary.1 + 8))
}

/// 灵动岛窗口固定逻辑尺寸(Phase6 定稿):lib.rs 位置恢复与 config.rs 落盘
/// 共用,免两处魔法数漂移
pub const ISLAND_W: i32 = 378;
pub const ISLAND_H: i32 = 46;

/// 可见位置裁决(纯函数,[clamp_to_visible] 与 [fallback_position] 的组合):
/// 与任一显示器相交 → 保留原位置;完全越界(显示器拔掉/分辨率变化)→
/// 主屏顶部居中兜底;没有显示器信息 → None(调用方放弃定位)。
/// 岛位置恢复与落盘共用,保证"落盘的位置下次启动必可见"。
pub fn clamp_or_fallback_position(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    monitors: &[(i32, i32, i32, i32)],
) -> Option<(i32, i32)> {
    clamp_to_visible(x, y, w, h, monitors).or_else(|| fallback_position(w, h, monitors))
}

/// 搜索框尺寸下限(逻辑像素):与前端缩放手柄下限一致(MainPanel.vue
/// applyResize:Math.max(360, w)/Math.max(260, h))——后端 clamp 不得放行
/// 前端手柄都达不到的尺寸
pub const MIN_SEARCH_W: f64 = 360.0;
pub const MIN_SEARCH_H: f64 = 260.0;

/// 搜索框尺寸落盘/恢复共用 clamp(纯函数,2026-08-14 审计 F2-2):
/// - 非有限值(NaN/Inf)返回 None,拒绝落盘——配置损坏(如 w=1e30)或手改
///   可致 4G 像素窗口;负值可致 0 尺寸窗口;
/// - 下限 360×260(前端缩放手柄下限),上限 = 最大显示器逻辑尺寸 × 2
///   (窗口允许跨屏放大,但两倍单屏足够宽松地拒绝荒谬值);
/// - 无显示器信息时上限兜底 4096×4096。
/// 入参 monitors 为 (x, y, width, height) 逻辑像素四元组(同 logical_monitors)。
pub fn clamp_search_size(
    w: f64,
    h: f64,
    monitors: &[(i32, i32, i32, i32)],
) -> Option<(f64, f64)> {
    if !w.is_finite() || !h.is_finite() {
        return None;
    }
    let (max_w, max_h) = monitors
        .iter()
        .fold((0i32, 0i32), |(mw, mh), &(_, _, mw2, mh2)| (mw.max(mw2), mh.max(mh2)));
    let (max_w, max_h) = if max_w > 0 && max_h > 0 {
        (max_w as f64 * 2.0, max_h as f64 * 2.0)
    } else {
        (4096.0, 4096.0)
    };
    Some((
        w.clamp(MIN_SEARCH_W, max_w.max(MIN_SEARCH_W)),
        h.clamp(MIN_SEARCH_H, max_h.max(MIN_SEARCH_H)),
    ))
}

#[cfg(test)]
mod tests {
    use super::{clamp_or_fallback_position, clamp_search_size, clamp_to_visible, fallback_position};

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

    // ---- 越界兜底位置(2026-08-14 审计 F2-3)----

    #[test]
    fn fallback_centers_top_on_primary_monitor() {
        let mons = mono();
        // 680x520 窗口在主屏(1920x1080)顶部居中:y=0+8
        assert_eq!(fallback_position(680, 520, &mons), Some((620, 8)));
    }

    #[test]
    fn fallback_picks_monitor_containing_origin() {
        // 双屏:主屏在右侧(坐标含原点),副屏在左侧(负坐标)
        let mons = vec![(-1920, 0, 1920, 1080), (0, 0, 1920, 1080)];
        assert_eq!(fallback_position(680, 520, &mons), Some((620, 8)));
        // 无显示器信息 → None(调用方放弃定位)
        assert_eq!(fallback_position(680, 520, &[]), None);
    }

    // ---- 可见位置裁决(2026-08-14 审计 F3-1:clamp 与 fallback 串联)----

    #[test]
    fn clamp_or_fallback_keeps_visible_and_falls_back_off_screen() {
        let mons = mono();
        // 屏内/部分可见 → 保留原值
        assert_eq!(clamp_or_fallback_position(100, 50, 378, 46, &mons), Some((100, 50)));
        assert_eq!(clamp_or_fallback_position(1900, 50, 378, 46, &mons), Some((1900, 50)));
        // 完全越界 → 主屏顶部居中兜底(378 宽:(1920-378)/2=771;y=8)
        assert_eq!(clamp_or_fallback_position(3000, 3000, 378, 46, &mons), Some((771, 8)));
        assert_eq!(clamp_or_fallback_position(-800, 0, 378, 46, &mons), Some((771, 8)));
        // 无显示器信息 → None(调用方放弃定位)
        assert_eq!(clamp_or_fallback_position(10, 10, 378, 46, &[]), None);
    }

    // ---- 搜索框尺寸 clamp(2026-08-14 审计 F2-2)----

    #[test]
    fn search_size_rejects_non_finite() {
        let mons = mono();
        assert_eq!(clamp_search_size(f64::NAN, 400.0, &mons), None);
        assert_eq!(clamp_search_size(400.0, f64::INFINITY, &mons), None);
        assert_eq!(clamp_search_size(f64::NEG_INFINITY, -3.0, &mons), None);
    }

    #[test]
    fn search_size_clamped_to_sane_range() {
        let mons = mono(); // 1920x1080 → 上限 3840x2160
        // 超大值(配置损坏:1e30)→ 上限
        assert_eq!(clamp_search_size(1e30, 1e30, &mons), Some((3840.0, 2160.0)));
        // 负值/过小 → 下限 360x260(前端缩放手柄下限)
        assert_eq!(clamp_search_size(-5.0, 10.0, &mons), Some((360.0, 260.0)));
        // 正常值原样返回
        assert_eq!(clamp_search_size(680.0, 520.0, &mons), Some((680.0, 520.0)));
        // 单维越界单维 clamp
        assert_eq!(clamp_search_size(680.0, 1e9, &mons), Some((680.0, 2160.0)));
    }

    #[test]
    fn search_size_no_monitors_uses_fallback_cap() {
        assert_eq!(clamp_search_size(1e30, 1e30, &[]), Some((4096.0, 4096.0)));
        assert_eq!(clamp_search_size(300.0, 200.0, &[]), Some((360.0, 260.0)));
    }
}
