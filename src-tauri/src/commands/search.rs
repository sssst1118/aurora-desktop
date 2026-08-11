use crate::indexer::app_index::{AppEntry, AppIndex};
use std::sync::Mutex;
use tauri::State;

/// 在内存索引中做大小写不敏感子串匹配,返回按名称排序的 top 20
#[tauri::command]
pub fn search_apps(query: String, index: State<'_, Mutex<AppIndex>>) -> Vec<AppEntry> {
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
