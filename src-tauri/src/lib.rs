mod commands;
mod hotkey;
mod indexer;
mod tray;
mod win_utils;

use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(Mutex::new(crate::indexer::build_index()))
        .setup(|app| {
            let handle = app.handle().clone();
            crate::tray::setup_tray(&handle)?;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
