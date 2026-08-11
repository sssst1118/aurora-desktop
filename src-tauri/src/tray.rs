use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle,
};

/// 【模块实现后替换】2.2 抽屉窗口呼出/隐藏(托盘入口动作占位,
/// 与 hotkey.rs 中同名 stub 语义一致;模块完成后由集成 agent 统一接入显隐实现)
fn toggle_drawer_window_stub(_app: &AppHandle) {}

/// 【模块实现后替换】2.3 剪贴板历史窗口呼出/隐藏(托盘入口动作占位)
fn toggle_clipboard_window_stub(_app: &AppHandle) {}

/// 【模块实现后替换】2.5 托盘 tooltip 更新(CPU/内存/网络,由 2.5 采样线程
/// 每 2s 调用;骨架阶段占位,模块完成后由集成 agent 接入)
#[allow(dead_code)]
fn update_tray_tooltip_stub(_app: &AppHandle) {}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show_all", "显示全部窗口", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "hide_all", "隐藏全部窗口", true, None::<&str>)?;
    let drawer_item = MenuItem::with_id(app, "toggle_drawer", "桌面抽屉", true, None::<&str>)?;
    let clipboard_item = MenuItem::with_id(app, "toggle_clipboard", "剪贴板历史", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &hide_item, &drawer_item, &clipboard_item, &quit_item])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().expect("no window icon").clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show_all" => crate::win_utils::show_all(app),
            "hide_all" => crate::win_utils::hide_all(app),
            "toggle_drawer" => toggle_drawer_window_stub(app),
            "toggle_clipboard" => toggle_clipboard_window_stub(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
