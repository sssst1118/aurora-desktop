use crate::indexer::app_index::{AppEntry, AppIndex};
use std::sync::Mutex;
use tauri::{Manager, State};

/// 在内存索引中做大小写不敏感子串匹配,返回按名称排序的 top 20
#[tauri::command]
pub fn search_apps(query: String, index: State<'_, Mutex<AppIndex>>) -> Vec<AppEntry> {
    // 并发修复(2026-08-13):索引原只在启动时构建一次,新安装应用不重启永远搜不到。
    // 每次搜索前走 refresh_if_stale:距上次检查超过阈值(60s)才做一次 mtime 增量
    // 重扫(目录未变化零成本复用缓存),把新条目合入 managed state,避免每次按键全量重建。
    if let Some(entries) = crate::indexer::refresh_if_stale() {
        if let Ok(mut g) = index.lock() {
            g.entries = entries;
        }
    }
    index
        .lock()
        .map(|g| g.search(&query))
        .unwrap_or_default()
}

/// 用系统默认方式打开(lnk/exe/目录通吃,ShellExecute 语义)
#[tauri::command]
pub fn open_item(path: String) -> bool {
    opener::open(&path).is_ok()
}

/// 显示 search 窗口并聚焦(island 点击唤起搜索;纯打开语义,AIPanel 设置入口在用)
#[tauri::command]
pub fn open_search(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("search") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 显隐切换 search 窗口(island 点击/双击呼出;与热键同款 toggle 逻辑)
#[tauri::command]
pub fn toggle_search(app: tauri::AppHandle) {
    crate::win_utils::toggle_search_window(&app);
}
