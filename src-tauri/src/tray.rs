use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Listener,
};

/// 2.5 托盘 tooltip 文本(CPU/内存/网络,纯函数便于单测)
fn format_tooltip_text(s: &crate::commands::system::SysStatus) -> String {
    fn rate(bps: u64) -> String {
        if bps >= 1024 * 1024 {
            format!("{:.1}MB", bps as f64 / 1024.0 / 1024.0)
        } else if bps >= 1024 {
            format!("{:.0}KB", bps as f64 / 1024.0)
        } else {
            format!("{bps}B")
        }
    }
    format!(
        "CPU {:.0}% | 内存 {:.1}G / {:.1}G | 下载 {}/s 上传 {}/s",
        s.cpu,
        s.mem_used_mb as f64 / 1024.0,
        s.mem_total_mb as f64 / 1024.0,
        rate(s.net_rx_bps),
        rate(s.net_tx_bps),
    )
}

/// 2.5 托盘 tooltip 更新:订阅 `sys-status` 事件(2.5 采样线程每 2s 广播),
/// 每次收到快照即更新 tooltip 文本(CPU/内存/网络,后端直更,不经前端)
fn subscribe_sys_status_tooltip(app: &AppHandle) {
    let tray_app = app.clone();
    app.listen("sys-status", move |event| {
        let Ok(status) =
            serde_json::from_str::<crate::commands::system::SysStatus>(&event.payload())
        else {
            return;
        };
        if let Some(tray) = tray_app.tray_by_id("main") {
            let _ = tray.set_tooltip(Some(format_tooltip_text(&status)));
        }
    });
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show_all", "显示全部窗口", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "hide_all", "隐藏全部窗口", true, None::<&str>)?;
    let drawer_item = MenuItem::with_id(app, "toggle_drawer", "桌面抽屉", true, None::<&str>)?;
    let clipboard_item = MenuItem::with_id(app, "toggle_clipboard", "剪贴板历史", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    // Phase3 AI 对话入口:enable_ai 总开关关闭时不显示(设计文档 §1.5),按配置条件加入
    let mut menu_items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
        vec![&show_item, &hide_item, &drawer_item, &clipboard_item];
    let ai_item;
    if crate::commands::config::load_from(&crate::commands::config::config_path(app)).enable_ai {
        ai_item = MenuItem::with_id(app, "toggle_ai_panel", "AI 对话", true, None::<&str>)?;
        menu_items.push(&ai_item);
    }
    menu_items.push(&quit_item);
    let menu = Menu::with_items(app, &menu_items)?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().expect("no window icon").clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show_all" => crate::win_utils::show_all(app),
            "hide_all" => crate::win_utils::hide_all(app),
            "toggle_drawer" => crate::hotkey::toggle_drawer_window(app),
            "toggle_clipboard" => crate::hotkey::toggle_clipboard_window(app),
            "toggle_ai_panel" => crate::hotkey::toggle_ai_panel_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    // 2.5 托盘 tooltip:订阅 sys-status 事件实时更新(采样线程 2s 一广播)
    subscribe_sys_status_tooltip(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> crate::commands::system::SysStatus {
        crate::commands::system::SysStatus {
            cpu: 12.5,
            mem_used_mb: 6144,
            mem_total_mb: 16384,
            net_rx_bps: 3_500_000,
            net_tx_bps: 512,
            ..Default::default()
        }
    }

    #[test]
    fn tooltip_text_contains_all_metrics() {
        let text = format_tooltip_text(&status());
        // 12.5 按 Rust {:.0} 银行家舍入 → 12
        assert!(text.contains("CPU 12%"), "CPU 取整:{text}");
        assert!(text.contains("内存 6.0G / 16.0G"), "内存:{text}");
        assert!(text.contains("下载 3.3MB/s"), "下载速率:{text}");
        assert!(text.contains("上传 512B/s"), "上传速率:{text}");
    }

    #[test]
    fn tooltip_zero_status_does_not_panic() {
        let text = format_tooltip_text(&Default::default());
        assert!(text.contains("CPU 0%"));
        assert!(text.contains("下载 0B/s"));
    }
}
