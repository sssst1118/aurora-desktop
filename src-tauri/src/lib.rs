mod ai;
mod automation;
mod commands;
mod first_run;
mod hotkey;
mod indexer;
mod logger;
mod runtime;
mod tray;
mod updater;
mod wallpaper_dynamic;
mod win_utils;

use std::sync::Mutex;
use tauri::{Emitter, Manager, RunEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 稳定性包:崩溃日志 hook 必须最先注册(此后任何 panic 都会留下日志文件);
    // 事件日志记录启动事件
    crate::logger::init();
    crate::logger::log_event("INFO", &format!("应用启动 v{}", env!("CARGO_PKG_VERSION")));
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 2.3 剪贴板历史:事件驱动监听/读写插件(模块完成后由 commands/clipboard.rs 使用)
        .plugin(tauri_plugin_clipboard::init())
        .manage(Mutex::new(crate::indexer::build_index()))
        .manage(crate::ai::confirm::ToolConfirmState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            crate::tray::setup_tray(&handle)?;
            // 设置开关生效:enable_island 关闭时隐藏灵动岛窗口(运行时改动走 runtime::apply 热生效)
            let cfg = crate::commands::config::load_from(&crate::commands::config::config_path(&handle));
            if !cfg.enable_island {
                if let Some(win) = app.get_webview_window("island") {
                    let _ = win.hide();
                }
            }
            // Phase6:drawer_open_on_launch 字段保留兼容但不再消费(旧独立 drawer 窗口已删除,
            // 小桌面是主面板默认视图,呼出面板即见)
            // 热键注册失败(如被系统或其他程序占用)不阻止应用启动,只记录告警
            if let Err(e) = crate::hotkey::setup_hotkey(&handle) {
                eprintln!("[aurora] 全局热键注册失败(可能被占用): {e}");
            }
            // 2.3 剪贴板历史:启动即监听(事件驱动,非轮询;内部按 enable_clipboard_history 开关自判)
            crate::commands::clipboard::setup(&handle)?;
            // 2.2 FileDrawer:启动桌面目录 watcher(事件驱动;内部按 enable_file_drawer 开关自判)
            crate::commands::drawer::init_watcher(handle.clone())?;
            // 主面板几何记忆:只恢复记住的尺寸(Phase6 起位置不再恢复——呼出时由
            // show_search_window 绑定岛下方;无记忆尺寸时跟随 tauri.conf.json 680x520)
            if let Some(win) = app.get_webview_window("search") {
                let w = cfg.search_width.unwrap_or(680.0);
                let h = cfg.search_height.unwrap_or(520.0);
                let _ = win.set_size(tauri::LogicalSize::new(w, h));
            }
            // Phase6:灵动岛位置恢复(拖动记忆;越界回退顶部居中)
            if let Some(win) = app.get_webview_window("island") {
                if let (Some(x), Some(y)) = (cfg.island_x, cfg.island_y) {
                    let monitors = crate::win_utils::logical_monitors(&handle);
                    if let Some((cx, cy)) =
                        crate::win_utils::clamp_to_visible(x, y, 378, 46, &monitors)
                    {
                        let _ = win.set_position(tauri::LogicalPosition::new(cx, cy));
                    }
                }
            }
            // Phase4 4.1 电池降载:常驻检测线程,内部每轮重读配置
            // (热生效:总开关/降载开关/阈值/周期运行时改配置即时生效,不重启)
            crate::wallpaper_dynamic::spawn_battery_watcher(handle.clone(), &cfg);
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
                            // 热生效:每轮重读 update_enabled,运行时关开关即停止检查(开恢复)
                            let cfg = crate::commands::config::load_from(
                                &crate::commands::config::config_path(&upd_app),
                            );
                            if cfg.update_enabled {
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
                            }
                            std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
                        }
                    })
                    .ok();
            }
            // Phase5 多屏热插拔:2s 轮询显示器布局签名(数量/坐标/尺寸/主屏),变化即重建多屏 attach;
            // 常驻线程内部自行判开关(热生效:运行中开/关动态壁纸与多屏都响应),
            // 总开关或多屏关时不做事只重置基线
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
            // 2.5 采样线程无需接线:首个 sys_get_status invoke(灵动岛挂载)时幂等懒启动;
            // 托盘 tooltip 的更新订阅在 tray::setup_tray 内完成
            // 稳定性包:首次启动引导(未完成过 → 自动呼出一次搜索框 + 托盘提示快捷键;
            // 用户关闭搜索框或保存任意配置后落盘 first_run_done,下次不再引导)
            if crate::first_run::is_first_run(&cfg) {
                crate::first_run::start(&handle);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // ---- Phase1 ----
            commands::search::search_apps,
            commands::search::open_item,
            commands::search::open_search,
            commands::search::toggle_search,
            commands::config::config_load,
            commands::config::config_save,
            commands::config::config_export,
            commands::config::config_import,
            commands::config::search_save_geometry,
            commands::config::island_save_geometry,
            commands::system::sys_get_status,
            // ---- 稳定性包:开机自启动(注册表 Run 键为真值) ----
            commands::launch::launch_set_startup,
            commands::launch::launch_get_startup,
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
            commands::clipboard::clipboard_delete_item,
            // ---- Phase2 2.4 壁纸(函数名带 _cmd 后缀,tauri::command(rename) 保持外部命令名) ----
            commands::wallpaper::wallpaper_set_static_cmd,
            commands::wallpaper::wallpaper_list_local_cmd,
            commands::wallpaper::wallpaper_thumbnail_cmd,
            commands::wallpaper::wallpaper_get_current_cmd,
            // ---- Phase3 3.1 AI 对话(流式/非流式/单轮工具执行) ----
            commands::ai::ai_chat_stream,
            commands::ai::ai_chat_completion,
            commands::ai::ai_execute_tool,
            commands::ai::ai_confirm_tool,
            // ---- Phase3 3.3 自然语言文件搜索(前端/AI 工具双入口) ----
            commands::file_search::ai_search_files,
            // ---- 自主迭代:H3 危险工具确认 + 搜索框直接搜文件 ----
            commands::file_search::search_files,
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
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // 退出时停止剪贴板监听,不留后台线程(与 setup 的启动对称);
            // 事件日志记录退出事件(稳定性包)
            if let RunEvent::ExitRequested { .. } = event {
                crate::logger::log_event("INFO", "应用退出");
                crate::commands::clipboard::teardown(app);
            }
        });
}
