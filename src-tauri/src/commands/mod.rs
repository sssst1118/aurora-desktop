pub mod ai; // 含 H3 新命令 ai_confirm_tool(危险工具确认回传;注册名 ai_confirm_tool,待 lib.rs invoke_handler 接线)
pub mod automation;
pub mod classify;
pub mod clipboard;
pub mod config;
pub mod dock;
pub mod drawer;
pub mod file_search;
pub mod launch;
pub mod search;
pub mod screenshot; // 2026-08-18 截图功能(热键触发遮罩 + BitBlt 截屏 + CF_DIB 剪贴板 + png 保存)
pub mod system;
pub mod uia_cmd;
pub mod updater_cmd;
pub mod wallpaper;
pub mod wallpaper_dynamic;
