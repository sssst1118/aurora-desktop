use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Manager;

/// Dock 快捷方式条目(AppConfig.dock_items 元素;2.1 模块命令直接复用本类型)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockItem {
    pub name: String,
    pub path: String,
}

/// 基础设置(与前端 AppConfig 字段一一对应)
/// #[serde(default)]:未来新增字段时,老配置文件缺失该字段仍可反序列化,不会整体失败回退丢失配置
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    // ---- Phase1 ----
    pub hotkey_search: String,
    pub enable_island: bool,
    // ---- Phase2 开关(Phase1 已含,默认 false,独立验收)----
    pub enable_dock: bool,
    pub enable_file_drawer: bool,
    pub enable_clipboard_history: bool,
    // ---- Phase2 2.1 Dock(并入搜索窗口后仅剩条目;position/auto_hide 已废弃 2026-08-12)----
    pub dock_items: Vec<DockItem>,
    // ---- Phase2 2.2 FileDrawer ----
    pub drawer_hotkey: String,
    pub drawer_open_on_launch: bool,
    // ---- Phase2 2.3 剪贴板历史 ----
    pub clipboard_max_items: u32,
    pub hotkey_clipboard: String,
    // ---- Phase2 2.4 壁纸 ----
    pub wallpaper_dir: Option<String>,
    // ---- Phase3 AI 集成(设计文档 §0.3.4;全部 ai 前缀)----
    pub enable_ai: bool,              // 总开关,默认 false;关闭时不注册热键、不显示面板入口
    pub ai_provider: String,          // "deepseek" | "ollama",默认 "deepseek"
    pub ai_api_key: Option<String>,   // 仅 DeepSeek 用;存文件明文,前端永远只见掩码(§1.3)
    pub ai_model: String,             // 默认 "deepseek-chat"
    pub ai_base_url: String,          // 默认 "https://api.deepseek.com/v1"
    pub ai_ollama_url: String,        // 默认 "http://127.0.0.1:11434/v1"
    pub ai_ollama_model: String,      // 默认 "qwen2.5:7b"(中文场景可用;用户按已装模型改)
    pub ai_tools_enabled: bool,       // 工具调用总开关,默认 true
    pub ai_search_roots: Vec<String>, // 3.3 搜索目录集合,默认空 = 仅桌面(禁止全盘)
    pub ai_max_tool_rounds: u32,      // 工具循环上限,默认 3(防死循环)
    pub ai_hotkey: String,            // 默认 "ctrl+alt+a"
    // ---- Phase4 4.1 动态壁纸(设计文档 §1)----
    pub enable_dynamic_wallpaper: bool,    // 总开关,默认 false;关闭时不创建壁纸窗口、不启动电池检测
    pub wallpaper_dynamic_dir: Option<String>, // 动态壁纸素材目录,默认 None = 与 2.4 wallpaper_dir 相同(仍为空则 %USERPROFILE%\Pictures)
    pub wallpaper_scale_mode: String,      // "cover" | "contain" | "stretch",默认 "cover"(视频/图片填充方式)
    pub wallpaper_battery_downshift: bool, // 电池降载开关,默认 true(§8 风险铁律,默认开)
    pub wallpaper_battery_threshold_pct: u8, // 降载阈值:电量低于该百分比即暂停;默认 0 = 只要在用电池就暂停
    pub wallpaper_battery_check_sec: u32,  // 电池检测周期,默认 30(有节制轮询)
    // ---- Phase4 4.2/4.3 自动化(风险最高模块,总开关默认关)----
    pub enable_automation: bool,           // 自动化总开关,默认 false;关闭时所有 automation_*/uia_* 命令直接返回错误
    pub automation_uia_enable: bool,       // 4.3 UIA 控件操作独立开关,默认 false(比键鼠模拟风险更高,独立可关)
    pub automation_click_delay_ms: u32,    // 连续点击最小间隔,默认 80(防连点风暴/误操作)
    // ---- Phase4 4.4 主题(设计文档 §4)----
    pub theme_mode: String,                // "system" | "dark" | "light",默认 "system"(跟随系统)
    pub theme_accent: String,              // 强调色 token 名,默认 "blue"(前端 CSS 变量令牌,§4.2)
    // ---- Phase5 5.1 自动更新(设计文档 §1)----
    pub update_enabled: bool,              // 更新总开关,默认 true;false 不检查不提示
    pub update_feed_url: String,           // 更新源 latest.json 地址,默认 GitHub 仓库内维护文件
    // ---- Phase5 5.2 多屏壁纸(设计文档 §2)----
    pub wallpaper_multi_monitor: bool,     // 多屏开关,默认 false(= 现状只铺主屏)
    pub wallpaper_span_mode: bool,         // true=拼接(一张素材铺满虚拟桌面);false=每屏独立素材
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_search: "Ctrl+Shift+Space".to_string(),
            enable_island: true,
            enable_dock: false,
            enable_file_drawer: false,
            enable_clipboard_history: false,
            dock_items: Vec::new(),
            drawer_hotkey: "ctrl+alt+d".to_string(),
            drawer_open_on_launch: false,
            clipboard_max_items: 200,
            hotkey_clipboard: "ctrl+alt+v".to_string(),
            wallpaper_dir: None,
            enable_ai: false,
            ai_provider: "deepseek".to_string(),
            ai_api_key: None,
            ai_model: "deepseek-chat".to_string(),
            ai_base_url: "https://api.deepseek.com/v1".to_string(),
            ai_ollama_url: "http://127.0.0.1:11434/v1".to_string(),
            ai_ollama_model: "qwen2.5:7b".to_string(),
            ai_tools_enabled: true,
            ai_search_roots: Vec::new(),
            ai_max_tool_rounds: 3,
            ai_hotkey: "ctrl+alt+a".to_string(),
            enable_dynamic_wallpaper: false,
            wallpaper_dynamic_dir: None,
            wallpaper_scale_mode: "cover".to_string(),
            wallpaper_battery_downshift: true,
            wallpaper_battery_threshold_pct: 0,
            wallpaper_battery_check_sec: 30,
            enable_automation: false,
            automation_uia_enable: false,
            automation_click_delay_ms: 80,
            theme_mode: "system".to_string(),
            theme_accent: "blue".to_string(),
            update_enabled: true,
            update_feed_url: "https://raw.githubusercontent.com/sssst1118/aurora-desktop/main/latest.json"
                .to_string(),
            wallpaper_multi_monitor: false,
            wallpaper_span_mode: true,
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
    let mut cfg = load_from(&config_path(&app));
    // 密钥脱敏契约(设计文档 §1.3):前端永远不见明文密钥
    cfg.ai_api_key = mask_key(&cfg.ai_api_key);
    cfg
}

#[tauri::command]
pub fn config_save(app: tauri::AppHandle, mut cfg: AppConfig) -> bool {
    let path = config_path(&app);
    let prev = load_from(&path);
    // 密钥脱敏契约:前端传回掩码 "******" = 未修改,保留磁盘旧值;新值或 None/空串直接生效
    cfg.ai_api_key = resolve_key_save(&prev.ai_api_key, &cfg.ai_api_key);
    save_to(&path, &cfg)
}

/// 密钥脱敏(设计文档 §1.3,纯函数):已配置 → 掩码占位 "******";未配置/空 → None
pub fn mask_key(key: &Option<String>) -> Option<String> {
    key.as_deref()
        .filter(|k| !k.is_empty())
        .map(|_| "******".to_string())
}

/// 密钥保存裁决(设计文档 §1.3,纯函数):incoming 为掩码占位 → 保留磁盘旧值(未修改);
/// 否则 incoming 直接生效(新值覆盖,或 None/空串清空)
pub fn resolve_key_save(prev: &Option<String>, incoming: &Option<String>) -> Option<String> {
    match incoming.as_deref() {
        Some("******") => prev.clone(),
        _ => incoming.clone(),
    }
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

    #[test]
    fn missing_new_fields_fall_back_per_field() {
        // 模拟老版本配置文件缺少未来新增字段:应逐字段回退默认,不整体失败
        let p = tmp_cfg("partial");
        std::fs::write(&p, r#"{"hotkey_search":"ctrl+alt+z"}"#).unwrap();
        let cfg = load_from(&p);
        assert_eq!(cfg.hotkey_search, "ctrl+alt+z");
        assert!(cfg.enable_island);
        assert!(!cfg.enable_dock);
        let _ = std::fs::remove_file(&p);
    }

    // ---- Phase4 字段(设计文档 §0.3.4):老配置缺 Phase4 字段逐字段回退默认 ----

    #[test]
    fn missing_phase4_fields_fall_back_per_field() {
        let p = tmp_cfg("p4partial");
        std::fs::write(&p, r#"{"hotkey_search":"ctrl+alt+z"}"#).unwrap();
        let cfg = load_from(&p);
        // 4.1 动态壁纸:全部默认关/空
        assert!(!cfg.enable_dynamic_wallpaper);
        assert_eq!(cfg.wallpaper_dynamic_dir, None);
        assert_eq!(cfg.wallpaper_scale_mode, "cover");
        assert!(cfg.wallpaper_battery_downshift);
        assert_eq!(cfg.wallpaper_battery_threshold_pct, 0);
        assert_eq!(cfg.wallpaper_battery_check_sec, 30);
        // 4.2/4.3 自动化:默认全关(风险最高模块)
        assert!(!cfg.enable_automation);
        assert!(!cfg.automation_uia_enable);
        assert_eq!(cfg.automation_click_delay_ms, 80);
        // 4.4 主题:system + blue(老用户升级后外观不变)
        assert_eq!(cfg.theme_mode, "system");
        assert_eq!(cfg.theme_accent, "blue");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn phase4_fields_roundtrip() {
        let p = tmp_cfg("p4roundtrip");
        let cfg = AppConfig {
            enable_dynamic_wallpaper: true,
            wallpaper_dynamic_dir: Some("D:\\wps".to_string()),
            wallpaper_scale_mode: "contain".to_string(),
            wallpaper_battery_downshift: false,
            wallpaper_battery_threshold_pct: 20,
            wallpaper_battery_check_sec: 60,
            enable_automation: true,
            automation_uia_enable: true,
            automation_click_delay_ms: 120,
            theme_mode: "light".to_string(),
            theme_accent: "green".to_string(),
            ..AppConfig::default()
        };
        assert!(save_to(&p, &cfg));
        let loaded = load_from(&p);
        assert!(loaded.enable_dynamic_wallpaper);
        assert_eq!(loaded.wallpaper_dynamic_dir.as_deref(), Some("D:\\wps"));
        assert_eq!(loaded.wallpaper_scale_mode, "contain");
        assert!(!loaded.wallpaper_battery_downshift);
        assert_eq!(loaded.wallpaper_battery_threshold_pct, 20);
        assert_eq!(loaded.wallpaper_battery_check_sec, 60);
        assert!(loaded.enable_automation);
        assert!(loaded.automation_uia_enable);
        assert_eq!(loaded.automation_click_delay_ms, 120);
        assert_eq!(loaded.theme_mode, "light");
        assert_eq!(loaded.theme_accent, "green");
        let _ = std::fs::remove_file(&p);
    }

    // ---- Phase3 密钥脱敏契约(设计文档 §1.3/§1.7)----

    #[test]
    fn mask_key_unset_returns_none() {
        assert_eq!(mask_key(&None), None);
        assert_eq!(mask_key(&Some(String::new())), None);
        assert_eq!(mask_key(&Some("".to_string())), None);
    }

    #[test]
    fn mask_key_set_returns_mask() {
        assert_eq!(
            mask_key(&Some("sk-abc123".to_string())),
            Some("******".to_string())
        );
    }

    #[test]
    fn resolve_key_save_mask_keeps_prev() {
        let prev = Some("sk-old-secret".to_string());
        let incoming = Some("******".to_string());
        assert_eq!(resolve_key_save(&prev, &incoming), prev);
    }

    #[test]
    fn resolve_key_save_new_value_overwrites() {
        let prev = Some("sk-old-secret".to_string());
        let incoming = Some("sk-new-secret".to_string());
        assert_eq!(
            resolve_key_save(&prev, &incoming),
            Some("sk-new-secret".to_string())
        );
    }

    #[test]
    fn resolve_key_save_none_or_empty_clears() {
        let prev = Some("sk-old-secret".to_string());
        assert_eq!(resolve_key_save(&prev, &None), None);
        assert_eq!(resolve_key_save(&prev, &Some(String::new())), Some(String::new()));
        // 未配置过 + 掩码 → 保持未配置
        assert_eq!(resolve_key_save(&None, &Some("******".to_string())), None);
    }

    // ---- Phase5 字段(设计文档 §5):老配置缺 Phase5 字段逐字段回退默认 ----

    #[test]
    fn missing_phase5_fields_fall_back_per_field() {
        let p = tmp_cfg("p5partial");
        std::fs::write(&p, r#"{"hotkey_search":"ctrl+alt+z"}"#).unwrap();
        let cfg = load_from(&p);
        // 5.1 更新:默认开 + 默认 GitHub 源
        assert!(cfg.update_enabled);
        assert_eq!(
            cfg.update_feed_url,
            "https://raw.githubusercontent.com/sssst1118/aurora-desktop/main/latest.json"
        );
        // 5.2 多屏:默认关(= 现状只铺主屏),拼接模式默认开
        assert!(!cfg.wallpaper_multi_monitor);
        assert!(cfg.wallpaper_span_mode);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn phase5_fields_roundtrip() {
        let p = tmp_cfg("p5roundtrip");
        let cfg = AppConfig {
            update_enabled: false,
            update_feed_url: "https://example.com/aurora/latest.json".to_string(),
            wallpaper_multi_monitor: true,
            wallpaper_span_mode: false,
            ..AppConfig::default()
        };
        assert!(save_to(&p, &cfg));
        let loaded = load_from(&p);
        assert!(!loaded.update_enabled);
        assert_eq!(loaded.update_feed_url, "https://example.com/aurora/latest.json");
        assert!(loaded.wallpaper_multi_monitor);
        assert!(!loaded.wallpaper_span_mode);
        let _ = std::fs::remove_file(&p);
    }
}
