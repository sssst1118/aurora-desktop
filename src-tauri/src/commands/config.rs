use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Manager;

/// 基础设置(与前端 AppConfig 字段一一对应)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkey_search: String,
    pub enable_island: bool,
    pub enable_dock: bool,
    pub enable_file_drawer: bool,
    pub enable_clipboard_history: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_search: "Ctrl+Shift+Space".to_string(),
            enable_island: true,
            enable_dock: false,
            enable_file_drawer: false,
            enable_clipboard_history: false,
        }
    }
}

/// 配置文件路径:%APPDATA%\com.aurora.desktop\config.json
pub fn config_path(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .map(|p| p.join("config.json"))
        .unwrap_or_else(|_| std::env::temp_dir().join("aurora_config.json"))
}

#[tauri::command]
pub fn config_load(app: tauri::AppHandle) -> AppConfig {
    load_from(&config_path(&app))
}

#[tauri::command]
pub fn config_save(app: tauri::AppHandle, cfg: AppConfig) -> bool {
    save_to(&config_path(&app), &cfg)
}

/// 读取配置;文件缺失或 JSON 损坏时回退默认值
pub fn load_from(path: &Path) -> AppConfig {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
            eprintln!("[aurora] config.json 损坏,回退默认配置: {path:?}");
            AppConfig::default()
        }),
        Err(_) => AppConfig::default(),
    }
}

/// 保存配置(自动创建父目录);成功返回 true
pub fn save_to(path: &Path, cfg: &AppConfig) -> bool {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[aurora] 创建配置目录失败: {e}");
            return false;
        }
    }
    match serde_json::to_string_pretty(cfg) {
        Ok(text) => match std::fs::write(path, text) {
            Ok(_) => true,
            Err(e) => {
                eprintln!("[aurora] 写配置失败: {e}");
                false
            }
        },
        Err(e) => {
            eprintln!("[aurora] 序列化配置失败: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_cfg(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("aurora_cfg_test_{tag}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn save_load_roundtrip() {
        let p = tmp_cfg("roundtrip");
        let cfg = AppConfig {
            hotkey_search: "ctrl+alt+x".to_string(),
            enable_island: false,
            ..AppConfig::default()
        };
        assert!(save_to(&p, &cfg));
        let loaded = load_from(&p);
        assert_eq!(loaded.hotkey_search, "ctrl+alt+x");
        assert!(!loaded.enable_island);
        assert!(!loaded.enable_dock);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let p = tmp_cfg("missing");
        let cfg = load_from(&p);
        assert_eq!(cfg.hotkey_search, "Ctrl+Shift+Space");
        assert!(cfg.enable_island);
    }

    #[test]
    fn corrupted_json_falls_back_to_default() {
        let p = tmp_cfg("corrupt");
        std::fs::write(&p, "{ not valid json !!").unwrap();
        let cfg = load_from(&p);
        assert_eq!(cfg.hotkey_search, "Ctrl+Shift+Space");
        let _ = std::fs::remove_file(&p);
    }
}
