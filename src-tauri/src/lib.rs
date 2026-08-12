mod ai;
mod automation;
mod commands;
mod hotkey;
mod indexer;
mod tray;
mod updater;
mod wallpaper_dynamic;
mod win_utils;

use std::sync::Mutex;
use tauri::{Emitter, Manager};

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
            // 2.3 剪贴板历史:启动即监听(事件驱动,非轮询;内部按 enable_clipboard_history 开关自判)
            crate::commands::clipboard::setup(&handle)?;
            // 2.2 FileDrawer:启动桌面目录 watcher(事件驱动;内部按 enable_file_drawer 开关自判)
            crate::commands::drawer::init_watcher(handle.clone())?;
            // Phase4 4.1 电池降载:总开关+降载开关都开才启动 30s 检测线程(状态变化才 emit wallpaper-power)
            if cfg.enable_dynamic_wallpaper && cfg.wallpaper_battery_downshift {
                crate::wallpaper_dynamic::spawn_battery_watcher(handle.clone(), &cfg);
            }
            // 5.1 自动更新:启动 15s 后 + 每 6h 静默检查;发现新版 emit update-available
            if crate::commands::config::load_from(&crate::commands::config::config_path(&handle))
                .update_enabled
            {
                let upd_app = handle.clone();
                std::thread::Builder::new()
                    .name("updater-watch".to_string())
                    .spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(15));
                        loop {
                            // 注意:app2 由 async 块独占(move),emit 也在块内用 app2,
                            // 外层循环只持有 upd_app 用于 clone,避免 use-after-move
                            let app2 = upd_app.clone();
                            tauri::async_runtime::spawn(async move {
                                let r = crate::commands::updater_cmd::update_check(app2.clone())
                                    .await;
                                if r.status == "available" {
                                    let _ = app2.emit(
                                        "update-available",
                                        &serde_json::json!({ "version": r.version, "notes": r.notes }),
                                    );
                                }
                            });
                            std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
                        }
                    })
                    .ok();
            }
            // Phase5 多屏热插拔:2s 轮询显示器布局签名(数量/坐标/尺寸/主屏),变化即重建多屏 attach;
            // 线程内部自行判开关(运行中开/关多屏都响应),总开关关时不做事只重置基线
            if cfg.enable_dynamic_wallpaper {
                let probe = handle.clone();
                std::thread::spawn(move || {
                    let mut last = String::new();
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        let cur = crate::commands::config::load_from(
                            &crate::commands::config::config_path(&probe),
                        );
                        if !cur.enable_dynamic_wallpaper || !cur.wallpaper_multi_monitor {
                            last.clear();
                            continue;
                        }
                        let sig = crate::wallpaper_dynamic::layout_signature(
                            &crate::wallpaper_dynamic::enum_monitors(&probe),
                        );
                        if sig != last {
                            last = sig;
                            let _ = crate::wallpaper_dynamic::multi_apply(&probe);
                        }
                    }
                });
            }
            // 2.5 采样线程无需接线:首个 sys_get_status invoke(灵动岛挂载)时幂等懒启动;
            // 托盘 tooltip 的更新订阅在 tray::setup_tray 内完成
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // ---- Phase1 ----
            commands::search::search_apps,
            commands::search::open_item,
            commands::search::open_search,
            commands::config::config_load,
            commands::config::config_save,
            commands::system::sys_get_status,
            // ---- Phase2 2.1 Dock ----
            commands::dock::dock_get_items,
            commands::dock::dock_set_items,
            commands::dock::dock_launch,
            commands::dock::dock_get_running,
            commands::dock::dock_get_icon,
            // ---- Phase2 2.2 FileDrawer ----
            commands::drawer::drawer_list_files,
            commands::drawer::drawer_open,
            commands::drawer::drawer_refresh,
            // ---- Phase2 2.3 剪贴板历史 ----
            commands::clipboard::clipboard_get_history,
            commands::clipboard::clipboard_clear_history,
            commands::clipboard::clipboard_copy_back,
            // ---- Phase2 2.4 壁纸(函数名带 _cmd 后缀,tauri::command(rename) 保持外部命令名) ----
            commands::wallpaper::wallpaper_set_static_cmd,
            commands::wallpaper::wallpaper_list_local_cmd,
            commands::wallpaper::wallpaper_get_current_cmd,
            // ---- Phase3 3.1 AI 对话(流式/非流式/单轮工具执行) ----
            commands::ai::ai_chat_stream,
            commands::ai::ai_chat_completion,
            commands::ai::ai_execute_tool,
            // ---- Phase3 3.3 自然语言文件搜索(前端/AI 工具双入口) ----
            commands::file_search::ai_search_files,
            // ---- Phase4 4.2 键鼠模拟自动化(SendInput;入口校验在命令内)----
            commands::automation::automation_sim_click,
            commands::automation::automation_sim_move,
            commands::automation::automation_sim_scroll,
            commands::automation::automation_sim_key,
            commands::automation::automation_sim_type,
            commands::automation::automation_sim_input,
            // ---- Phase4 4.1 动态壁纸(WorkerW 注入 + 电池降载;static 命令在 wallpaper 模块)----
            commands::wallpaper_dynamic::wallpaper_dynamic_list,
            commands::wallpaper_dynamic::wallpaper_dynamic_set,
            commands::wallpaper_dynamic::wallpaper_dynamic_clear,
            commands::wallpaper_dynamic::wallpaper_dynamic_get_state,
            // ---- Phase5 5.2 多屏壁纸(逐屏注入 + 布局枚举;热插拔走 setup 线程)----
            commands::wallpaper_dynamic::wallpaper_multi_monitors,
            commands::wallpaper_dynamic::wallpaper_multi_apply,
            commands::wallpaper_dynamic::wallpaper_dynamic_set_monitor,
            // ---- Phase5 5.1 自动更新(自研 updater;定时检查在 setup 线程)----
            commands::updater_cmd::update_check,
            commands::updater_cmd::update_download,
            commands::updater_cmd::update_install,
            commands::updater_cmd::update_open_folder,
            // ---- Phase4 4.3 UIA 控件自动化(Uia* 句柄式 API;入口校验在命令内)----
            commands::uia_cmd::uia_find_window,
            commands::uia_cmd::uia_get_window_info,
            commands::uia_cmd::uia_find_controls,
            commands::uia_cmd::uia_get_control_text,
            commands::uia_cmd::uia_click_control,
            commands::uia_cmd::uia_type_into,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
