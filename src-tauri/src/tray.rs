use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Listener,
};

/// 首次启动引导 tooltip 文案(稳定性包 2026-08-13;快捷键与 hotkey.rs SEARCH_HOTKEY 对应)
pub const FIRST_RUN_TOOLTIP: &str = "Ctrl+Shift+Space 呼出搜索";

/// 首次引导提示标志:置真时 tooltip 固定显示快捷键提示,优先于 2s 一次的系统
/// 状态 tooltip;引导完成(config 保存/搜索框关闭,见 first_run.rs)后复位,
/// 下一次 sys-status 广播(≤2s)自动恢复系统状态 tooltip
static FIRST_RUN_HINT: AtomicBool = AtomicBool::new(false);

/// 设置首次引导提示标志(引导开始置真,完成复位)
pub fn set_first_run_hint(on: bool) {
    FIRST_RUN_HINT.store(on, Ordering::SeqCst);
}

/// 立即把 tooltip 设为首次引导提示(不等下一次 sys-status 广播)
pub fn set_first_run_tooltip(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(FIRST_RUN_TOOLTIP.to_string()));
    }
}

/// tooltip 文本裁决(纯函数,可单测):首次引导中 → 快捷键提示;否则系统状态
pub fn current_tooltip(status: &crate::commands::system::SysStatus, first_run_hint: bool) -> String {
    if first_run_hint {
        FIRST_RUN_TOOLTIP.to_string()
    } else {
        format_tooltip_text(status)
    }
}

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
            // 首次引导中:固定显示快捷键提示(见 current_tooltip 裁决)
            let _ = tray.set_tooltip(Some(current_tooltip(
                &status,
                FIRST_RUN_HINT.load(Ordering::SeqCst),
            )));
        }
    });
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show_all", "显示全部窗口", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "hide_all", "隐藏全部窗口", true, None::<&str>)?;
    let drawer_item = MenuItem::with_id(app, "toggle_drawer", "桌面抽屉", true, None::<&str>)?;
    let clipboard_item = MenuItem::with_id(app, "toggle_clipboard", "剪贴板历史", true, None::<&str>)?;
    // 5.1 自动更新:入口按 update_enabled 开关条件加入(关闭时不显示)
    let update_item = MenuItem::with_id(app, "check_update", "检查更新", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    // Phase3 AI 对话入口:enable_ai 总开关关闭时不显示(设计文档 §1.5),按配置条件加入
    let mut menu_items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
        vec![&show_item, &hide_item, &drawer_item, &clipboard_item];
    let ai_item;
    if crate::commands::config::load_from(&crate::commands::config::config_path(app)).enable_ai {
        ai_item = MenuItem::with_id(app, "toggle_ai_panel", "AI 对话", true, None::<&str>)?;
        menu_items.push(&ai_item);
    }
    if crate::commands::config::load_from(&crate::commands::config::config_path(app)).update_enabled {
        menu_items.push(&update_item);
    }
    menu_items.push(&quit_item);
    let menu = Menu::with_items(app, &menu_items)?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show_all" => crate::win_utils::show_all(app),
            "hide_all" => crate::win_utils::hide_all(app),
            // Phase6:托盘入口 = 呼出主面板对应视图(旧独立窗口已删除)
            "toggle_drawer" => crate::win_utils::show_panel_with_view(app, "drawer"),
            "toggle_clipboard" => crate::win_utils::show_panel_with_view(app, "clipboard"),
            "toggle_ai_panel" => crate::win_utils::show_panel_with_view(app, "ai"),
            // 5.1 检查更新:结果广播 update-check-result(前端/托盘弹窗均可消费)
            "check_update" => {
                let tray_app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let r = crate::commands::updater_cmd::update_check(tray_app.clone()).await;
                    let _ = tray_app.emit(
                        "update-check-result",
                        &serde_json::json!({ "status": r.status, "version": r.version, "notes": r.notes, "error": r.error }),
                    );
                });
            }
            "quit" => app.exit(0),
            _ => {}
        });
    // 图标兜底(加固 2026-08-13):打包期 tauri.conf.json 已保证默认图标存在(缺失打包即失败),
    // 运行时取不到(极端环境)仅跳过设置图标——托盘菜单仍可用,不再 expect panic 崩进程
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
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

    // ---- 首次引导 tooltip 裁决(稳定性包 2026-08-13,纯函数) ----

    #[test]
    fn first_run_hint_overrides_status_tooltip() {
        // 引导中:固定快捷键提示(不显示系统状态)
        assert_eq!(current_tooltip(&status(), true), FIRST_RUN_TOOLTIP);
        assert_eq!(current_tooltip(&Default::default(), true), FIRST_RUN_TOOLTIP);
        // 引导结束:恢复系统状态 tooltip
        let text = current_tooltip(&status(), false);
        assert!(text.contains("CPU"), "引导结束后应显示系统状态: {text}");
    }
}
