//! 配置热生效调度(2026-08-12 用户要求:所有功能热启动,不重启生效)。
//!
//! config_save 落盘成功后调用 [`apply`],按新配置对运行中功能做增删:
//! - 热键:抽屉/剪贴板/AI 按开关与键位 diff 式注册/注销(见 hotkey::apply_hotkeys);
//! - 剪贴板监听:开关开→start_monitor,关→stop_monitor(见 clipboard::apply_config);
//! - 抽屉 watcher:开关开→init_watcher(幂等),关→停止监听与防抖线程;
//! - 系统采样线程:enable_island 开→ensure_started,关→停止(见 system_sampler);
//! - 灵动岛窗口:enable_island 关→立即隐藏,开→显示;
//! - 动态壁纸:总开关关→撤下已注入的 WorkerW 壁纸(见 wallpaper_dynamic::apply_config);
//! - 电池降载/多屏热插拔/更新检查:常驻线程内部每轮重读配置,无需 apply(见 lib.rs setup)。
//!
//! 原则:apply 失败只告警,绝不阻断 config_save 落盘;各步骤幂等,可重复调用。

use tauri::{AppHandle, Manager};

use crate::commands::config::AppConfig;

/// 按新配置把运行中各功能调到一致状态(幂等;任何单项失败只告警)
pub fn apply(app: &AppHandle, cfg: &AppConfig) {
    // 1. 热键(抽屉/剪贴板/AI)
    crate::hotkey::apply_hotkeys(app, cfg);

    // 2. 剪贴板监听
    crate::commands::clipboard::apply_config(app, cfg);

    // 3. 抽屉 watcher(内部自读开关,幂等)
    let _ = crate::commands::drawer::apply_config(app.clone());

    // 4. 系统采样线程 + 灵动岛窗口(enable_island 门控)
    if cfg.enable_island {
        crate::commands::system::system_sampler::ensure_started(app);
    } else {
        crate::commands::system::system_sampler::stop();
        if let Some(win) = app.get_webview_window("island") {
            let _ = win.hide();
        }
    }
    if cfg.enable_island {
        if let Some(win) = app.get_webview_window("island") {
            let _ = win.show();
        }
    }

    // 5. 动态壁纸:关闭总开关 → 撤下已注入的 WorkerW 壁纸(开启由 set 命令显式设置)
    crate::commands::wallpaper_dynamic::apply_config(app, cfg);
}
