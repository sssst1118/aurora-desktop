//! Phase2 命令占位(stub)。
//!
//! 骨架合并阶段:invoke_handler 需全量注册 Phase2 命令,但各模块命令文件
//! (commands/dock.rs、files.rs、clipboard.rs、wallpaper.rs)由并行模块 agent
//! 编写,尚未存在。此处提供同签名 stub 保证编译通过、调用不崩(返回空数据)。
//!
//! 【模块实现后替换】各模块完成后,由集成 agent 按模块逐个:
//! 1. invoke_handler 中 `commands::stubs::xxx` 切为 `commands::xxx::xxx`;
//! 2. 删除本文件对应 stub 函数与本文件底部临时类型定义。
//!
//! 命令签名以 docs/Phase2-设计.md §1.2/2.2/3.2/4.2 为准,模块 agent 照此实现,
//! 若实现时签名有调整,由集成 agent 在合并时对齐。

use serde::{Deserialize, Serialize};

use super::config::DockItem;

// ---- 2.2 FileDrawer 临时类型(模块实现后随 stubs 一并删除,改用 files.rs 定义)----
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrawerFile {
    pub name: String,
    pub path: String,
    pub ext: String,
    pub is_dir: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrawerGroup {
    pub category: String,
    pub files: Vec<DrawerFile>,
}

// ---- 2.3 剪贴板临时类型(模块实现后随 stubs 一并删除,改用 clipboard.rs 定义)----
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub tp: String, // "text" / "image"
    pub payload: String,
    pub ts: u64,
}

// ---- 2.4 壁纸临时类型(模块实现后随 stubs 一并删除,改用 wallpaper.rs 定义)----
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WallpaperEntry {
    pub name: String,
    pub path: String,
}

// ==================== 2.1 Dock ====================

/// 【模块实现后替换】读取 Dock 条目
#[tauri::command]
pub fn dock_get_items() -> Vec<DockItem> {
    Vec::new()
}

/// 【模块实现后替换】写回 Dock 条目
#[tauri::command]
pub fn dock_set_items(_items: Vec<DockItem>) -> bool {
    false
}

/// 【模块实现后替换】启动或聚焦 Dock 项
#[tauri::command]
pub fn dock_launch(_item: DockItem) -> bool {
    false
}

/// 【模块实现后替换】运行中且被 Dock 收录的应用路径集合
#[tauri::command]
pub fn dock_get_running() -> Vec<String> {
    Vec::new()
}

/// 【模块实现后替换】图标 base64 data URL
#[tauri::command]
pub fn dock_get_icon(_path: String) -> Option<String> {
    None
}

// ==================== 2.2 FileDrawer ====================

/// 【模块实现后替换】扫描桌面并按扩展名分组
#[tauri::command]
pub fn drawer_list_files() -> Vec<DrawerGroup> {
    Vec::new()
}

/// 【模块实现后替换】打开抽屉内文件/文件夹
#[tauri::command]
pub fn drawer_open(_path: String) -> bool {
    false
}

/// 【模块实现后替换】手动刷新
#[tauri::command]
pub fn drawer_refresh() -> Vec<DrawerGroup> {
    Vec::new()
}

// ==================== 2.3 剪贴板历史 ====================

/// 【模块实现后替换】读取剪贴板历史
#[tauri::command]
pub fn clipboard_get_history() -> Vec<ClipboardItem> {
    Vec::new()
}

/// 【模块实现后替换】清空剪贴板历史
#[tauri::command]
pub fn clipboard_clear_history() {}

/// 【模块实现后替换】回贴第 index 条
#[tauri::command]
pub fn clipboard_copy_back(_index: usize) -> Result<(), String> {
    Err("剪贴板模块尚未实现".to_string())
}

// ==================== 2.4 壁纸 ====================

/// 【模块实现后替换】设置静态壁纸
#[tauri::command]
pub fn wallpaper_set_static(_file_path: String) -> Result<(), String> {
    Err("壁纸模块尚未实现".to_string())
}

/// 【模块实现后替换】列出壁纸目录图片
#[tauri::command]
pub fn wallpaper_list_local() -> Vec<WallpaperEntry> {
    Vec::new()
}

/// 【模块实现后替换】读取当前壁纸路径
#[tauri::command]
pub fn wallpaper_get_current() -> Option<String> {
    None
}
