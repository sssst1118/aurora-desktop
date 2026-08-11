mod commands;
mod hotkey;
mod indexer;
mod tray;
mod win_utils;

use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 2.3 剪贴板历史:事件驱动监听/读写插件(模块完成后由 commands/clipboard.rs 使用)
        .plugin(tauri_plugin_clipboard::init())
        .manage(Mutex::new(crate::indexer::build_index()))
        .setup(|app| {
            let handle = app.handle().clone();
            crate::tray::setup_tray(&handle)?;
            // 设置开关生效:enable_island 关闭时隐藏灵动岛窗口(重启生效)
            let cfg = crate::commands::config::load_from(&crate::commands::config::config_path(&handle));
            if !cfg.enable_island {
                if let Some(win) = app.get_webview_window("island") {
                    let _ = win.hide();
                }
            }
            // 2.1 Dock:enable_dock 开启时显示,初始位置屏幕底部中央(默认 dock_position=bottom)
            if cfg.enable_dock {
                if let Some(win) = app.get_webview_window("dock") {
                    if let Ok(Some(mon)) = app.primary_monitor() {
                        let size = mon.size();
                        let _ = win.set_position(tauri::PhysicalPosition::new(
                            (size.width as i32 - 800) / 2,
                            size.height as i32 - 64,
                        ));
                    }
                    let _ = win.show();
                }
            }
            // 2.2 FileDrawer:drawer_open_on_launch 时启动即显示抽屉窗口
            if cfg.enable_file_drawer && cfg.drawer_open_on_launch {
                if let Some(win) = app.get_webview_window("drawer") {
                    let _ = win.show();
                }
            }
            // 热键注册失败(如被系统或其他程序占用)不阻止应用启动,只记录告警
            if let Err(e) = crate::hotkey::setup_hotkey(&handle) {
                eprintln!("[aurora] 全局热键注册失败(可能被占用): {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search::search_apps,
            commands::search::open_item,
            commands::search::open_search,
            commands::config::config_load,
            commands::config::config_save,
            commands::system::sys_get_status,
            // ---- Phase2 占位注册:模块实现后由集成 agent 逐个切换为 commands::<mod>::xxx ----
            commands::stubs::dock_get_items,
            commands::stubs::dock_set_items,
            commands::stubs::dock_launch,
            commands::stubs::dock_get_running,
            commands::stubs::dock_get_icon,
            commands::stubs::drawer_list_files,
            commands::stubs::drawer_open,
            commands::stubs::drawer_refresh,
            commands::stubs::clipboard_get_history,
            commands::stubs::clipboard_clear_history,
            commands::stubs::clipboard_copy_back,
            commands::stubs::wallpaper_set_static,
            commands::stubs::wallpaper_list_local,
            commands::stubs::wallpaper_get_current,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
