use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, Manager};

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
    // ---- 搜索框外观与几何记忆(2026-08-12)----
    pub search_style: String,              // "glass" 毛玻璃(默认) | "solid" 不透明
    pub search_x: Option<i32>,             // 记住的窗口位置(逻辑像素);None=启动居中
    pub search_y: Option<i32>,
    pub search_width: Option<f64>,         // 记住的窗口大小;None=配置默认 620x420
    pub search_height: Option<f64>,
    // ---- 稳定性包(2026-08-13)----
    pub launch_at_startup: bool,           // 开机自启;注册表 Run 键为真值(commands/launch.rs 读写)
    pub first_run_done: bool,              // 首次启动引导是否已完成(见 first_run.rs;默认 false)
    // ---- Phase6 UI 重构(2026-08-14,设计文档 §7)----
    pub skin: String,                 // 皮肤包 "deep"|"midnight"|"dawn"|"verdant",默认 "deep"
    pub island_x: Option<i32>,        // 灵动岛位置(逻辑像素);None=顶部居中
    pub island_y: Option<i32>,
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
            search_style: "solid".to_string(), // Phase6 真机反馈(2026-08-14):默认不透明;玻璃为可选
            search_x: None,
            search_y: None,
            search_width: None,
            search_height: None,
            launch_at_startup: false,
            first_run_done: false,
            skin: "deep".to_string(),
            island_x: None,
            island_y: None,
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

/// 配置读-改-写全局互斥锁(并发修复 2026-08-13)。
///
/// config_save / search_save_geometry 与 dock::save_items 都是"读全量→改字段→写全量"
/// 模式,无互斥时并发执行会互相覆盖(后写丢掉先写的字段,设置/窗口位置随机丢失)。
/// 所有配置读-改-写路径必须先持有本锁,把"读→改→写"串行为一个原子整体。
/// Dock 条目存在 config.json 的 dock_items 字段、与配置同文件,故共用本锁。
///
/// 纯读取(load_from)不取锁:[save_to] 的 tmp+rename 原子写保证读者要么见完整旧文件、
/// 要么见完整新文件,绝不会读到半写内容。
static CONFIG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn config_lock() -> &'static Mutex<()> {
    CONFIG_LOCK.get_or_init(|| Mutex::new(()))
}

#[tauri::command]
pub fn config_load(app: tauri::AppHandle) -> AppConfig {
    let mut cfg = load_from(&config_path(&app));
    // 密钥脱敏契约(设计文档 §1.3):前端永远不见明文密钥
    cfg.ai_api_key = mask_key(&cfg.ai_api_key);
    cfg
}

// ---- 设置导入导出(可维护性收尾 2026-08-13:主力工具换机迁移)----

/// 导出配置全文:原样返回 config.json 文本(**含明文密钥**——换机迁移必须连 API Key
/// 一起带走,否则导入后 DeepSeek 失效;这是用户显式主动的导出动作,文件由用户自行
/// 保管,与 config_load 的"前端只见掩码"展示契约不冲突)。
/// 文件不存在(从未保存过)时导出默认配置序列化,保证导出永远可用。
#[tauri::command]
pub fn config_export(app: tauri::AppHandle) -> Result<String, String> {
    let path = config_path(&app);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(_) => serde_json::to_string_pretty(&AppConfig::default())
            .map_err(|e| format!("导出配置失败: {e}")),
    }
}

/// 导入文件的标识字段(AppConfig 无 version 字段,用最早最核心的字段集合做归属判断):
/// 任一存在即初步判定为 Aurora 配置文件;全部缺失直接拒绝,防止把任意 JSON 导入后
/// 经 #[serde(default)] 静默变成"全默认配置"清空现有设置。
const IMPORT_IDENTIFIER_FIELDS: [&str; 5] = [
    "hotkey_search",
    "enable_island",
    "theme_mode",
    "search_style",
    "update_enabled",
];

/// 校验导入 JSON(纯函数,便于单测):必须解析为合法 JSON 对象 + 含标识字段 +
/// 各字段类型与 AppConfig 一致(serde 对类型不符报具体错误);通过后返回完整配置,
/// 缺失字段按 #[serde(default)] 逐字段补默认(与老配置升级同语义)。
pub fn validate_import_json(json: &str) -> Result<AppConfig, String> {
    let root: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("导入文件不是合法 JSON: {e}"))?;
    let Some(obj) = root.as_object() else {
        return Err("导入文件不是 Aurora 配置文件(顶层不是 JSON 对象)".to_string());
    };
    if !IMPORT_IDENTIFIER_FIELDS.iter().any(|f| obj.contains_key(*f)) {
        return Err("导入文件不是 Aurora 配置文件(缺少标识字段,请使用\"导出设置\"生成的 .json)".to_string());
    }
    serde_json::from_value::<AppConfig>(root).map_err(|e| format!("配置字段校验失败: {e}"))
}

/// 导入前备份现有 config.json → `config.json.pre-import`(防误导入丢配置,可人工恢复)。
/// 已有旧备份先删除再复制(只保留最近一次导入前现场,与 backup_corrupt_file 同策略);
/// 原文件不存在(首次导入)或复制失败 → 静默跳过,不阻断导入流程。
pub fn backup_before_import(path: &Path) {
    if !path.exists() {
        return; // 首次导入,无旧配置可备份
    }
    let mut backup_os = path.as_os_str().to_os_string();
    backup_os.push(".pre-import");
    let backup = PathBuf::from(backup_os);
    let _ = std::fs::remove_file(&backup);
    if let Err(e) = std::fs::copy(path, &backup) {
        eprintln!("[aurora] 导入前备份配置失败({}): {e}", backup.display());
    }
}

/// 导入配置(换机迁移):先校验 → 备份现有配置为 .pre-import → 落盘 → 热生效。
/// 与 config_save 复用同一套安全链路(密钥掩码裁决 + 更新源白名单校验 +
/// first_run_done 兜底 + 配置锁串行化 + save_to 原子写 + 锁外 runtime::apply)。
#[tauri::command]
pub fn config_import(app: tauri::AppHandle, json: String) -> Result<bool, String> {
    let mut cfg = validate_import_json(&json)?;
    let path = config_path(&app);
    // 并发修复:与 config_save / search_save_geometry 共用配置锁,读-改-写整体串行
    let guard = config_lock().lock().unwrap_or_else(|p| p.into_inner());
    let prev = load_from(&path);
    // 密钥裁决:导入文件里是掩码 "******"(如手工编辑过)→ 保留本机磁盘旧值,防清空密钥
    cfg.ai_api_key = resolve_key_save(&prev.ai_api_key, &cfg.ai_api_key);
    // H1 供应链加固:导入同样是把 update_feed_url 写入配置的路径,白名单边界保持一致
    // (与磁盘旧值相同 → 放行,防历史 URL 误拒;有改动 → 严格校验)
    validate_feed_url_change(&prev.update_feed_url, &cfg.update_feed_url)?;
    if !cfg.first_run_done {
        cfg.first_run_done = true;
        crate::tray::set_first_run_hint(false);
    }
    // 校验全部通过后才备份:非法导入不产生备份、不动现有配置
    backup_before_import(&path);
    let ok = save_to(&path, &cfg);
    // 落盘完成即放锁:热生效只读配置,不参与读-改-写,留在锁外缩小临界区(同 config_save)
    drop(guard);
    if ok {
        crate::runtime::apply(&app, &cfg);
        // Phase6 皮肤跨窗口热生效:全局广播(区别于前端 window 级 aurora:config-saved),
        // 岛窗口监听到后重新 config_load + apply_theme,无需重启
        let _ = app.emit("config-saved", ());
    }
    Ok(ok)
}

// ---- H1 供应链加固:更新源域名白名单(2026-08-13)----

/// `update_feed_url` 允许的 host 白名单(全部为 GitHub 官方域名及其文件/API/CDN 域)。
///
/// 加固背景:config_save 对 update_feed_url 全盘接受,配合可写的 config.json 构成
/// 供应链攻击面——自研 updater 下载产物后 SHA-256 校验通过即静默升级安装,
/// 恶意内容可把更新源指到任意服务器。故保存时强制 host 精确命中本表。
/// ⚠️ 用户自有静态服务器分发场景需扩展此表(设计权衡:白名单优先,不开放任意域)。
const UPDATE_FEED_HOST_WHITELIST: [&str; 6] = [
    "github.com",
    "raw.githubusercontent.com",
    "api.github.com",
    "objects.githubusercontent.com",
    "github-releases.githubusercontent.com",
    "releases.github.com",
];

/// 校验更新源 URL(纯函数,便于单测)。
/// 规则:必须 https 开头 + host 精确命中白名单(大小写不敏感);解析失败一律拒绝
/// (白名单是安全边界,收紧方向,失败即拒不宽容)。
/// 注:只在 config_save 拦截,load_from 不校验——启动时不因老配置里的历史 URL 误拒。
pub fn validate_update_feed_url(url: &str) -> Result<(), String> {
    let s = url.trim();
    let (scheme, rest) = s
        .split_once("://")
        .ok_or_else(|| "更新源不是合法 URL".to_string())?;
    if !scheme.eq_ignore_ascii_case("https") {
        return Err("更新源必须是 https 地址".to_string());
    }
    // authority = 首个 / ? # 之前的部分
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // 用户信息段攻击面(https://github.com@evil.com/):host 取最后一个 @ 之后
    let host_port = authority.rsplit('@').next().unwrap_or("");
    // 端口剥离(https://host:443/ 与无端口等价);IPv6 字面量等奇形输入因
    // 无法精确命中白名单自然被拒
    let host = host_port.split(':').next().unwrap_or("");
    if UPDATE_FEED_HOST_WHITELIST.iter().any(|w| w.eq_ignore_ascii_case(host)) {
        Ok(())
    } else {
        Err(format!(
            "更新源域名 {host} 不在白名单内(仅允许 GitHub 官方域名;自有服务器分发场景请联系维护者扩展白名单)"
        ))
    }
}

/// 保存时更新源 URL 校验(纯函数,便于单测):
/// 与磁盘旧值相同(忽略大小写)→ 放行——老用户配置里可能已有白名单外 URL
/// (旧版本全盘接受写入),未改动时不得因保存无关设置而误拒;
/// 有改动 → 严格校验(必须 https + host 在白名单)。
fn validate_feed_url_change(prev: &str, incoming: &str) -> Result<(), String> {
    if prev.eq_ignore_ascii_case(incoming) {
        return Ok(());
    }
    validate_update_feed_url(incoming)
}

#[tauri::command]
pub fn config_save(app: tauri::AppHandle, mut cfg: AppConfig) -> Result<bool, String> {
    let path = config_path(&app);
    // 并发修复:与 search_save_geometry / dock::save_items 共用配置锁,
    // "读全量(prev)→改字段→写全量"整体串行化,防并发保存互相覆盖。
    let guard = config_lock().lock().unwrap_or_else(|p| p.into_inner());
    let prev = load_from(&path);
    // 密钥脱敏契约:前端传回掩码 "******" = 未修改,保留磁盘旧值;新值或 None/空串直接生效
    cfg.ai_api_key = resolve_key_save(&prev.ai_api_key, &cfg.ai_api_key);
    // H1 供应链加固:更新源 URL 白名单校验(只拦 save 不拦 load_from,避免启动误拒)。
    // 校验失败返回 Err 拒绝保存(前端 saveSafe 已有捕获链路:提示 + 回滚本地值)
    validate_feed_url_change(&prev.update_feed_url, &cfg.update_feed_url)?;
    // 稳定性包:首次启动引导兜底——保存任意配置即视为引导完成(用户已找到设置页)。
    // 主路径是引导窗口关闭即置位(见 first_run.rs),此处兜底防关闭事件丢失导致
    // 每次启动重复引导;同步撤下托盘快捷键提示
    if !cfg.first_run_done {
        cfg.first_run_done = true;
        crate::tray::set_first_run_hint(false);
    }
    let ok = save_to(&path, &cfg);
    // 落盘完成即可放锁:热生效只读配置(见 runtime::apply),不参与读-改-写,留在锁外缩小临界区
    drop(guard);
    // 热生效(2026-08-12 用户要求:所有功能不重启即生效):落盘成功后按新配置
    // 同步运行中状态(热键/监听/watcher/采样线程/窗口显隐)。失败不阻断保存。
    if ok {
        crate::runtime::apply(&app, &cfg);
        // Phase6 皮肤跨窗口热生效:全局广播(区别于前端 window 级 aurora:config-saved),
        // 岛窗口监听到后重新 config_load + apply_theme,无需重启
        let _ = app.emit("config-saved", ());
    }
    Ok(ok)
}

/// 搜索框拖动/缩放后记忆几何(前端 onMoved/onResized 防抖后调用)。
/// 只写配置文件、不触发热生效:几何是纯展示状态,无运行时逻辑需要同步;
/// 下次启动由 setup 恢复。
#[tauri::command]
pub fn search_save_geometry(app: tauri::AppHandle, x: i32, y: i32, w: f64, h: f64) -> bool {
    let path = config_path(&app);
    // 并发修复:几何记忆的读-改-写同样取配置锁,与 config_save / dock_set_items 串行,
    // 否则窗口拖动与设置保存并发时互相覆盖(位置/大小随机丢失)
    let _guard = config_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut cfg = load_from(&path);
    cfg.search_x = Some(x);
    cfg.search_y = Some(y);
    cfg.search_width = Some(w);
    cfg.search_height = Some(h);
    save_to(&path, &cfg)
}

/// 灵动岛拖动后记忆位置(前端 onMoved 防抖后调用;None 语义=居中,由前端判断)。
/// 只写配置文件、不触发热生效:位置是纯展示状态,下次启动由 setup 恢复。
#[tauri::command]
pub fn island_save_geometry(app: tauri::AppHandle, x: i32, y: i32) -> bool {
    let path = config_path(&app);
    let _guard = config_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut cfg = load_from(&path);
    cfg.island_x = Some(x);
    cfg.island_y = Some(y);
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

/// 损坏的配置文件备份:改名为 `<文件名>.broken`(已存在先删除——只保留最近一次
/// 损坏现场)。改名失败静默,不影响回退流程。
/// 备份只发生一次:文件改名后,后续读取走"文件缺失 → 默认"分支。
/// 审计场景「配置 JSON 损坏」:不备份的话,损坏文件会在下一次 config_save 时被
/// 默认配置静默覆盖,原始内容(可能人工可修)永久丢失。
pub fn backup_corrupt_file(path: &Path) {
    let mut backup_os = path.as_os_str().to_os_string();
    backup_os.push(".broken");
    let backup = PathBuf::from(backup_os);
    let _ = std::fs::remove_file(&backup);
    let _ = std::fs::rename(path, &backup);
}

/// 读取配置;文件缺失时回退默认值;JSON 损坏时先把原文件备份为 `<文件名>.broken`
/// 再回退默认值(防止后续保存把无法解析的原文件静默覆盖)
pub fn load_from(path: &Path) -> AppConfig {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
            eprintln!("[aurora] config.json 损坏,回退默认配置(原文件已备份为 .broken): {path:?}");
            backup_corrupt_file(path);
            AppConfig::default()
        }),
        Err(_) => AppConfig::default(),
    }
}

/// 保存配置(自动创建父目录);成功返回 true。
///
/// 并发修复 2026-08-13:落盘走 [atomic_write](tmp+rename),不再直接 fs::write——
/// 崩溃瞬间只会留下完整的旧文件或完整的新文件,不会出现半写 config.json
/// (半写文件下次启动 JSON 解析失败,全部配置回退默认)。
pub fn save_to(path: &Path, cfg: &AppConfig) -> bool {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[aurora] 创建配置目录失败: {e}");
            return false;
        }
    }
    match serde_json::to_string_pretty(cfg) {
        Ok(text) => atomic_write(path, text.as_bytes()),
        Err(e) => {
            eprintln!("[aurora] 序列化配置失败: {e}");
            false
        }
    }
}

/// 原子写文件:先写同目录 `<文件名>.tmp` 临时文件,成功后 rename 覆盖目标
/// (同目录 rename 原子,Windows 上 std::fs::rename 等价 MoveFileExW(REPLACE_EXISTING))。
/// 任何失败路径都清理 tmp,不留垃圾。写 tmp 的并发安全由调用方保证
/// (config 写路径持配置锁,clipboard 写路径持 history 锁)。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> bool {
    let mut tmp_os = path.as_os_str().to_os_string();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    if let Err(e) = std::fs::write(&tmp, bytes) {
        eprintln!("[aurora] 写临时文件失败({}): {e}", tmp.display());
        return false;
    }
    match std::fs::rename(&tmp, path) {
        Ok(_) => true,
        Err(e) => {
            // 失败路径清理 tmp:半写临时文件对读取无影响(读只认正式文件名),但会残留垃圾
            let _ = std::fs::remove_file(&tmp);
            eprintln!("[aurora] 原子替换文件失败({}): {e}", path.display());
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
        // 同时清掉可能的 tmp / broken / pre-import 残留
        // (原子写/损坏备份/导入备份测试需从干净状态开始)
        let tmp = PathBuf::from(format!("{}.tmp", p.display()));
        let _ = std::fs::remove_file(&tmp);
        let broken = PathBuf::from(format!("{}.broken", p.display()));
        let _ = std::fs::remove_file(&broken);
        let pre = PathBuf::from(format!("{}.pre-import", p.display()));
        let _ = std::fs::remove_file(&pre);
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

    // ---- 原子写(并发修复 2026-08-13):tmp+rename,写入后文件存在且内容完整 ----

    #[test]
    fn atomic_write_leaves_complete_file_and_no_tmp() {
        let p = tmp_cfg("atomic");
        let cfg = AppConfig {
            theme_mode: "dark".to_string(),
            search_x: Some(120),
            ..AppConfig::default()
        };
        assert!(save_to(&p, &cfg), "原子写应成功");
        // 文件存在且内容完整可反序列化(半写会解析失败)
        let loaded = load_from(&p);
        assert_eq!(loaded.theme_mode, "dark");
        assert_eq!(loaded.search_x, Some(120));
        // 不残留 tmp 临时文件
        let tmp = PathBuf::from(format!("{}.tmp", p.display()));
        assert!(!tmp.exists(), "成功路径不应残留 tmp: {}", tmp.display());
        // 二次保存覆盖旧内容(rename 覆盖已有目标)
        let cfg2 = AppConfig {
            theme_mode: "light".to_string(),
            ..AppConfig::default()
        };
        assert!(save_to(&p, &cfg2));
        let loaded2 = load_from(&p);
        assert_eq!(loaded2.theme_mode, "light");
        assert_eq!(loaded2.search_x, None);
        assert!(!tmp.exists(), "覆盖后仍不应残留 tmp: {}", tmp.display());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn atomic_write_failure_cleans_tmp() {
        // 目标路径指向已存在的目录 → rename 失败 → 走失败路径,应清理 tmp
        let dir = std::env::temp_dir().join(format!("aurora_cfg_test_atomic_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 构造一个"目标是目录"的路径:atomic_write 写到 dir 本身不行,这里用一个
        // 子路径,让 tmp 可写但 rename 目标已被目录占位
        let target = dir.join("sub.json");
        std::fs::create_dir_all(&target).unwrap(); // target 是目录,rename 文件→目录必失败
        assert!(!atomic_write(&target, b"{}"));
        let tmp = PathBuf::from(format!("{}.tmp", target.display()));
        assert!(!tmp.exists(), "失败路径应清理 tmp: {}", tmp.display());
        let _ = std::fs::remove_dir_all(&dir);
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

    // ---- 损坏备份(审计修复 2026-08-13):损坏的 config.json 改名 .broken 后再回退,
    //      防止下一次保存把无法解析的原文件静默覆盖 ----

    #[test]
    fn corrupted_json_backed_up_before_fallback() {
        let p = tmp_cfg("corrupt_backup");
        let original = r#"{ "hotkey_search": "ctrl+alt+z", "enable_ai": tr"#;
        std::fs::write(&p, original).unwrap();
        // 读取:回退默认,原文件被备份为 .broken,原路径不再存在
        let cfg = load_from(&p);
        assert_eq!(cfg.hotkey_search, "Ctrl+Shift+Space");
        let broken = PathBuf::from(format!("{}.broken", p.display()));
        assert!(broken.exists(), "损坏原文件应备份为 .broken");
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), original, "备份内容与原件一致");
        assert!(!p.exists(), "原路径应已改名");
        // 再次读取:走"缺失 → 默认"分支,不重复备份、不 panic
        let cfg2 = load_from(&p);
        assert_eq!(cfg2.hotkey_search, "Ctrl+Shift+Space");
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), original);
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(&broken);
    }

    // ---- 稳定性包字段(2026-08-13):老配置缺字段逐字段回退默认 false ----

    #[test]
    fn missing_stability_fields_fall_back_false() {
        let p = tmp_cfg("stability_partial");
        std::fs::write(&p, r#"{"hotkey_search":"ctrl+alt+z"}"#).unwrap();
        let cfg = load_from(&p);
        assert!(!cfg.launch_at_startup);
        assert!(!cfg.first_run_done);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn stability_fields_roundtrip() {
        let p = tmp_cfg("stability_roundtrip");
        let cfg = AppConfig {
            launch_at_startup: true,
            first_run_done: true,
            ..AppConfig::default()
        };
        assert!(save_to(&p, &cfg));
        let loaded = load_from(&p);
        assert!(loaded.launch_at_startup);
        assert!(loaded.first_run_done);
        let _ = std::fs::remove_file(&p);
    }

    // ---- Phase6 字段(设计文档 §7):老配置缺 Phase6 字段逐字段回退默认 ----

    #[test]
    fn missing_phase6_fields_fall_back_per_field() {
        let p = tmp_cfg("p6partial");
        std::fs::write(&p, r#"{"hotkey_search":"ctrl+alt+z"}"#).unwrap();
        let cfg = load_from(&p);
        assert_eq!(cfg.skin, "deep");
        assert_eq!(cfg.island_x, None);
        assert_eq!(cfg.island_y, None);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn phase6_fields_roundtrip() {
        let p = tmp_cfg("p6roundtrip");
        let cfg = AppConfig {
            skin: "dawn".to_string(),
            island_x: Some(120),
            island_y: Some(30),
            ..AppConfig::default()
        };
        assert!(save_to(&p, &cfg));
        let loaded = load_from(&p);
        assert_eq!(loaded.skin, "dawn");
        assert_eq!(loaded.island_x, Some(120));
        assert_eq!(loaded.island_y, Some(30));
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

    // ---- H1 更新源白名单(2026-08-13):save 校验 / load 不校验 ----

    #[test]
    fn feed_url_whitelist_allows_github_domains() {
        for u in [
            "https://github.com/sssst1118/aurora-desktop/releases/latest.json",
            "https://raw.githubusercontent.com/sssst1118/aurora-desktop/main/latest.json",
            "https://api.github.com/repos/sssst1118/aurora-desktop/releases/latest",
            "https://objects.githubusercontent.com/abc/def.bin",
            "https://github-releases.githubusercontent.com/123/abc",
            "https://releases.github.com/download/abc",
            "https://github.com", // 无路径也放行(host 合法)
            "https://github.com:443/latest.json", // 默认端口等价
            "https://GITHUB.com/x",                // host 大小写不敏感
            "HTTPS://Raw.GitHubusercontent.com/x", // 协议大小写不敏感
        ] {
            assert!(validate_update_feed_url(u).is_ok(), "应放行: {u}");
        }
    }

    #[test]
    fn feed_url_whitelist_rejects_outside_hosts() {
        for u in [
            "https://example.com/aurora/latest.json",
            "https://evilgithub.com/x",      // 前缀撞名不算命中(精确 host 匹配)
            "https://github.com.evil.com/x", // 子域伪装
            "https://raw.githubusercontent.com.evil.com/x",
            "https://github.com@evil.com/x", // 用户信息段攻击
            "https://user:pass@evil.com/x",
            "https://github.com\\evil.com/x", // 反斜杠混淆
            "https://github.com.evil.com",    // 尾缀撞名
        ] {
            assert!(validate_update_feed_url(u).is_err(), "应拒绝: {u}");
        }
    }

    #[test]
    fn feed_url_rejects_http_and_non_urls() {
        // http 明文拒绝
        assert!(validate_update_feed_url("http://github.com/x").is_err());
        assert!(validate_update_feed_url("HTTP://github.com/x").is_err());
        // 其他协议拒绝
        assert!(validate_update_feed_url("ftp://github.com/x").is_err());
        assert!(validate_update_feed_url("file:///etc/passwd").is_err());
        // 非 URL 拒绝
        assert!(validate_update_feed_url("not a url").is_err());
        assert!(validate_update_feed_url("").is_err());
        assert!(validate_update_feed_url("   ").is_err());
        assert!(validate_update_feed_url("https://").is_err());
    }

    #[test]
    fn feed_url_change_unchanged_passthrough_changed_validated() {
        // 老配置历史值(白名单外)未改动 → 放行:保存无关设置不因历史 URL 误拒
        let legacy = "https://example.com/legacy.json";
        assert!(validate_feed_url_change(legacy, legacy).is_ok());
        assert!(validate_feed_url_change(legacy, &legacy.to_uppercase()).is_ok()); // 忽略大小写
        // 改为白名单内 → 放行
        assert!(validate_feed_url_change(legacy, "https://github.com/x/latest.json").is_ok());
        // 改为白名单外 → 拒绝
        assert!(validate_feed_url_change(legacy, "https://evil.com/x").is_err());
        assert!(validate_feed_url_change("https://github.com/x", "http://github.com/x").is_err());
    }

    // ---- 设置导入导出(可维护性收尾 2026-08-13:换机迁移)----

    #[test]
    fn import_valid_json_parses_with_default_fallback() {
        // 合法导出文件通过校验;缺失字段按 #[serde(default)] 补默认(老文件导入同升级语义)
        let json = serde_json::to_string_pretty(&AppConfig {
            hotkey_search: "ctrl+alt+x".to_string(),
            theme_mode: "dark".to_string(),
            ..AppConfig::default()
        })
        .unwrap();
        let cfg = validate_import_json(&json).expect("合法导出文件应通过校验");
        assert_eq!(cfg.hotkey_search, "ctrl+alt+x");
        assert_eq!(cfg.theme_mode, "dark");
        assert!(cfg.enable_island, "未写字段回落默认值");
    }

    #[test]
    fn import_partial_json_ok_when_identifier_present() {
        // 仅含一个标识字段的最小 JSON 也可导入(其余字段补默认)——与 load_from 的老配置语义一致
        let cfg = validate_import_json(r#"{"hotkey_search":"ctrl+alt+z"}"#).expect("含标识字段应通过");
        assert_eq!(cfg.hotkey_search, "ctrl+alt+z");
        assert!(cfg.enable_island);
    }

    #[test]
    fn import_invalid_json_rejected() {
        assert!(validate_import_json("{ not json").is_err(), "坏 JSON 拒绝");
        assert!(validate_import_json("").is_err(), "空串拒绝");
        assert!(validate_import_json("[1,2,3]").is_err(), "顶层数组不是对象,拒绝");
        assert!(validate_import_json("null").is_err(), "顶层 null 拒绝");
        assert!(validate_import_json("\"hotkey_search\"").is_err(), "顶层字符串拒绝");
    }

    #[test]
    fn import_missing_identifier_fields_rejected() {
        // 缺标识字段的任意 JSON 不允许导入(否则经 serde(default) 静默变成全默认配置,清空现有设置)
        assert!(validate_import_json(r#"{}"#).is_err());
        assert!(validate_import_json(r#"{"foo":1}"#).is_err());
        assert!(validate_import_json(r#"{"name":"aurora"}"#).is_err(), "撞名非标识字段也拒绝");
    }

    #[test]
    fn import_wrong_field_type_rejected() {
        // 标识字段在但类型不符 → serde 报具体类型错误,拒绝导入(不会部分生效)
        assert!(validate_import_json(r#"{"hotkey_search":123}"#).is_err());
        assert!(validate_import_json(r#"{"theme_mode":42}"#).is_err());
        assert!(validate_import_json(r#"{"enable_island":"yes"}"#).is_err());
        let err = validate_import_json(r#"{"hotkey_search":123}"#).unwrap_err();
        assert!(err.contains("配置字段校验失败"), "错误信息应定位到字段校验: {err}");
    }

    #[test]
    fn import_flow_backs_up_and_persists() {
        // 模拟 config_import 落盘路径(纯函数串联,免构造 AppHandle):
        // 旧配置先写盘 → 校验新 JSON → 备份 → 落盘;断言新值生效 + .pre-import 备份存在且内容为旧值
        let p = tmp_cfg("import");
        let old = AppConfig {
            hotkey_search: "ctrl+alt+o".to_string(),
            theme_mode: "light".to_string(),
            ..AppConfig::default()
        };
        assert!(save_to(&p, &old));
        let incoming = serde_json::to_string_pretty(&AppConfig {
            hotkey_search: "ctrl+alt+n".to_string(),
            theme_mode: "dark".to_string(),
            ..AppConfig::default()
        })
        .unwrap();
        let cfg = validate_import_json(&incoming).unwrap();
        backup_before_import(&p);
        assert!(save_to(&p, &cfg), "导入落盘应成功");
        let loaded = load_from(&p);
        assert_eq!(loaded.hotkey_search, "ctrl+alt+n");
        assert_eq!(loaded.theme_mode, "dark");
        // 备份存在且内容 = 导入前的旧配置(误导入可人工恢复)
        let backup = PathBuf::from(format!("{}.pre-import", p.display()));
        assert!(backup.exists(), "导入前备份应存在: {}", backup.display());
        let prev: AppConfig =
            serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
        assert_eq!(prev.hotkey_search, "ctrl+alt+o");
        assert_eq!(prev.theme_mode, "light");
        // 二次导入:旧备份被覆盖为最近一次导入前现场
        let incoming2 = serde_json::to_string_pretty(&AppConfig {
            hotkey_search: "ctrl+alt+m".to_string(),
            ..AppConfig::default()
        })
        .unwrap();
        let cfg2 = validate_import_json(&incoming2).unwrap();
        backup_before_import(&p);
        assert!(save_to(&p, &cfg2));
        let prev2: AppConfig =
            serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
        assert_eq!(prev2.hotkey_search, "ctrl+alt+n");
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(&backup);
    }

    #[test]
    fn import_backup_skipped_when_no_existing_config() {
        // 首次导入(磁盘无 config.json):备份静默跳过、不 panic、不产生空备份文件
        let p = tmp_cfg("import_first");
        backup_before_import(&p);
        let backup = PathBuf::from(format!("{}.pre-import", p.display()));
        assert!(!backup.exists(), "无旧配置时不应产生备份文件");
    }
}
