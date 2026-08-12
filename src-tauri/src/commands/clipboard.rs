//! 2.3 剪贴板历史:命令实现 + 事件驱动监听 + 本地持久化。
//!
//! 隐私边界:内容只存本地(%APPDATA%\com.aurora.desktop\clipboard.json,与 config.json
//! 同级),绝不外发;图片场景只记文件路径,不存图片内容(位图快照属 Phase4)。
//!
//! 监听:tauri-plugin-clipboard v2 的 `start_monitor`(Windows 端基于
//! AddClipboardFormatListener / WM_CLIPBOARDUPDATE,**事件驱动,非轮询**)。
//! 监听生命周期与窗口解耦:程序启动(setup)即开始监听,不依赖剪贴板窗口打开。
//!
//! 接线说明(集成 agent 负责 lib.rs/tray.rs):
//! - lib.rs setup 中调用 `commands::clipboard::setup(&app.handle())?` → 启动即监听;
//! - 托盘退出流程调用 `commands::clipboard::teardown(&handle)` → 停止监听线程;
//! - 即使未接线,命令首次被调用时也会惰性初始化(ensure_ready),功能不依赖接线。
//!
//! 命令签名:JS 侧契约(index 参数与返回类型)与 stubs.rs 占位完全一致;
//! 实现中追加的 `app: AppHandle` 由 Tauri 自动注入,不参与 JS 契约(与 config.rs 同款)。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Listener, Manager};

use super::config::{config_path, load_from};

/// 前端订阅的广播事件:新条目入库后发出(payload = 最新一条 ClipboardItem)
pub const EVENT_CLIPBOARD_UPDATED: &str = "clipboard-updated";
/// 插件 monitor 的内部更新事件名(与 tauri-plugin-clipboard desktop.rs 契约一致)
const EVENT_PLUGIN_UPDATE: &str = "plugin:clipboard://clipboard-monitor/update";

/// 文本上限:>64KB 拒绝(开发文档"不保存超大二进制"落地)
const MAX_TEXT_BYTES: usize = 64 * 1024;

/// 剪贴板历史条目(与 stubs.rs 临时类型、开发文档 §5 契约一致)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub tp: String, // "text" / "image"
    pub payload: String,
    pub ts: u64,
}

/// 内存历史态(全局单例;单实例应用,无需 Tauri managed state)
#[derive(Default)]
struct History {
    /// 新条目在头部(索引 0 最新)
    items: Vec<ClipboardItem>,
    /// 最近一条内容的哈希,用于去重(与最近一条完全相同不记)
    last_hash: u64,
    /// 上限(clipboard_max_items,默认 200)
    max_items: u32,
}

impl History {
    /// 校验+清洗:纯空白拒绝,>64KB 拒绝;返回裁剪首尾空白后的文本
    fn should_record(payload: &str) -> Option<String> {
        let t = payload.trim();
        if t.is_empty() || t.len() > MAX_TEXT_BYTES {
            return None;
        }
        Some(t.to_string())
    }

    /// 入队头部(新条目顶掉最旧);与最近一条内容哈希相同 → 不记,返回 false
    fn push(&mut self, item: ClipboardItem) -> bool {
        if self.max_items == 0 {
            return false;
        }
        let h = content_hash(&item.payload);
        if h == self.last_hash {
            return false;
        }
        self.items.insert(0, item);
        self.last_hash = h;
        if self.items.len() as u32 > self.max_items {
            self.items.truncate(self.max_items as usize);
        }
        true
    }
}

/// 内容哈希(去重用;进程内与最近一条比较即可,无需密码学哈希)
fn content_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn history() -> &'static Mutex<History> {
    static HISTORY: OnceLock<Mutex<History>> = OnceLock::new();
    HISTORY.get_or_init(|| Mutex::new(History::default()))
}

fn monitor_started() -> &'static AtomicBool {
    static FLAG: AtomicBool = AtomicBool::new(false);
    &FLAG
}

/// 事件监听是否已注册(独立于 monitor 启停:热生效时 stop/start 反复切换,
/// 监听器只注册一次,避免重复注册导致一次剪贴板更新触发多次入库)
fn listener_registered() -> &'static AtomicBool {
    static FLAG: AtomicBool = AtomicBool::new(false);
    &FLAG
}

/// 历史文件路径:%APPDATA%\com.aurora.desktop\clipboard.json(与 config.json 同级)
pub fn history_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .map(|p| p.join("clipboard.json"))
        .unwrap_or_else(|_| std::env::temp_dir().join("aurora_clipboard.json"))
}

/// 从磁盘加载历史;文件缺失或 JSON 损坏返回空列表
fn load_from_file(path: &Path) -> Vec<ClipboardItem> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
            eprintln!("[aurora] clipboard.json 损坏,历史置空: {path:?}");
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

/// 落盘(自动创建父目录);失败仅告警,不阻断复制流程
fn save_to_file(path: &Path, items: &[ClipboardItem]) -> bool {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[aurora] 创建剪贴板历史目录失败: {e}");
            return false;
        }
    }
    match serde_json::to_string_pretty(items) {
        Ok(text) => match std::fs::write(path, text) {
            Ok(_) => true,
            Err(e) => {
                eprintln!("[aurora] 写剪贴板历史失败: {e}");
                false
            }
        },
        Err(e) => {
            eprintln!("[aurora] 序列化剪贴板历史失败: {e}");
            false
        }
    }
}

/// 程序启动即监听(与窗口解耦):加载磁盘历史、读配置上限、开关开启时启动事件驱动监听。
/// 幂等,可重复调用。由集成 agent 在 lib.rs setup 接线;命令首次调用也会惰性执行。
pub fn setup(app: &AppHandle) -> Result<(), String> {
    ensure_ready(app);
    Ok(())
}

/// 停止监听(托盘退出流程调用;不留后台线程)。幂等。
pub fn teardown(app: &AppHandle) {
    stop_monitor_if_needed(app);
}

/// 惰性初始化(幂等):历史装载 + 配置上限 + 按开关启动监听
fn ensure_ready(app: &AppHandle) {
    let cfg = load_from(&config_path(app));
    let mut hist = history().lock().unwrap();
    if hist.items.is_empty() {
        hist.items = load_from_file(&history_path(app));
        hist.last_hash = hist.items.first().map(|i| content_hash(&i.payload)).unwrap_or(0);
    }
    hist.max_items = cfg.clipboard_max_items;
    drop(hist);

    if cfg.enable_clipboard_history {
        start_monitor_if_needed(app);
    }
}

/// 注册插件事件监听(仅一次;后续 start/stop 只开关 monitor)
fn ensure_listener(app: &AppHandle) {
    if !listener_registered().swap(true, Ordering::SeqCst) {
        let listener_app = app.clone();
        app.listen(EVENT_PLUGIN_UPDATE, move |_event| {
            handle_clipboard_update(&listener_app);
        });
    }
}

/// 启动插件 monitor(幂等;插件内部同样防重复启动)
fn start_monitor_if_needed(app: &AppHandle) {
    ensure_listener(app);
    if !monitor_started().swap(true, Ordering::SeqCst) {
        if let Err(e) = app
            .state::<tauri_plugin_clipboard::Clipboard>()
            .start_monitor(app.clone())
        {
            eprintln!("[aurora] 剪贴板监听启动失败: {e}");
        }
    }
}

/// 停止插件 monitor(幂等;监听器保留,再次启动不需重注册)
fn stop_monitor_if_needed(app: &AppHandle) {
    if monitor_started().swap(false, Ordering::SeqCst) {
        let _ = app
            .state::<tauri_plugin_clipboard::Clipboard>()
            .stop_monitor(app.clone());
    }
}

/// 热生效入口(config_save 后调用):开关开 → 启动监听;关 → 停止(监听器保留)。
/// 幂等,与启动路径共用。
pub fn apply_config(app: &AppHandle, cfg: &super::config::AppConfig) {
    ensure_ready(app);
    if cfg.enable_clipboard_history {
        start_monitor_if_needed(app);
    } else {
        stop_monitor_if_needed(app);
    }
}

/// 剪贴板更新处理(在插件 watcher 线程上执行):读取 → 校验 → 去重 → 入队 → 裁剪 → 落盘 → 广播
fn handle_clipboard_update(app: &AppHandle) {
    let clipboard = app.state::<tauri_plugin_clipboard::Clipboard>();
    let Some(item) = read_latest_item(&clipboard) else {
        return;
    };
    let inserted = {
        let mut hist = history().lock().unwrap();
        hist.push(item.clone())
    };
    if !inserted {
        return;
    }
    {
        let hist = history().lock().unwrap();
        save_to_file(&history_path(app), &hist.items);
    }
    let _ = app.emit(EVENT_CLIPBOARD_UPDATED, &item);
}

/// 读取剪贴板当前内容为条目:文本优先;无文本时取文件列表第一项路径(图片文件场景)
fn read_latest_item(clipboard: &tauri_plugin_clipboard::Clipboard) -> Option<ClipboardItem> {
    pick_item(
        clipboard.read_text().ok(),
        clipboard.read_files().unwrap_or_default(),
        now_secs(),
    )
}

/// 从原始读取结果挑选记录条目(纯函数,便于单测):文本优先,其次文件路径
fn pick_item(text: Option<String>, files: Vec<String>, ts: u64) -> Option<ClipboardItem> {
    if let Some(raw) = text {
        if let Some(clean) = History::should_record(&raw) {
            return Some(ClipboardItem {
                tp: "text".to_string(),
                payload: clean,
                ts,
            });
        }
    }
    for p in files {
        let path = p.trim();
        if !path.is_empty() {
            return Some(ClipboardItem {
                tp: "image".to_string(),
                payload: path.to_string(),
                ts,
            });
        }
    }
    None
}

// ==================== 命令(JS 契约与 stubs.rs 占位一致) ====================

/// 读取剪贴板历史(内存态;启动/首次调用时从 json 加载)
#[tauri::command]
pub fn clipboard_get_history(app: tauri::AppHandle) -> Vec<ClipboardItem> {
    ensure_ready(&app);
    history().lock().unwrap().items.clone()
}

/// 清空剪贴板历史(清内存 + 删除历史文件)
#[tauri::command]
pub fn clipboard_clear_history(app: tauri::AppHandle) {
    let mut hist = history().lock().unwrap();
    hist.items.clear();
    hist.last_hash = 0;
    drop(hist);
    let _ = std::fs::remove_file(history_path(&app));
}

/// 回贴第 index 条到系统剪贴板(文本 → writeText;图片文件路径 → 还原"复制文件"场景);
/// 越界返回 Err
#[tauri::command]
pub fn clipboard_copy_back(app: tauri::AppHandle, index: usize) -> Result<(), String> {
    let item = history()
        .lock()
        .unwrap()
        .items
        .get(index)
        .cloned()
        .ok_or_else(|| format!("剪贴板历史索引 {index} 越界"))?;
    let clipboard = app.state::<tauri_plugin_clipboard::Clipboard>();
    if item.tp == "image" {
        clipboard.write_files_uris(vec![item.payload])?;
    } else {
        clipboard.write_text(item.payload)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_file(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("aurora_clipboard_test_{tag}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn text_item(s: &str, ts: u64) -> ClipboardItem {
        ClipboardItem {
            tp: "text".to_string(),
            payload: s.to_string(),
            ts,
        }
    }

    #[test]
    fn should_record_rejects_blank_and_oversize() {
        assert!(History::should_record("").is_none());
        assert!(History::should_record("   ").is_none());
        assert!(History::should_record("\n\t ").is_none());
        assert!(History::should_record(&"x".repeat(MAX_TEXT_BYTES + 1)).is_none());
        assert_eq!(
            History::should_record(&"x".repeat(MAX_TEXT_BYTES)).unwrap().len(),
            MAX_TEXT_BYTES
        );
        assert_eq!(History::should_record("  你好 ").unwrap(), "你好");
    }

    #[test]
    fn dedup_same_content_as_latest_only_once() {
        let mut h = History {
            max_items: 10,
            ..Default::default()
        };
        assert!(h.push(text_item("你好", 1)));
        assert!(!h.push(text_item("你好", 2))); // 与最近一条相同 → 不记
        assert!(h.push(text_item("世界", 3)));
        assert_eq!(h.items.len(), 2);
        // 中间插入不同内容后,相同文本可再记(语义=只与最近一条比较)
        assert!(h.push(text_item("你好", 4)));
        assert_eq!(h.items.len(), 3);
        // 最近一条变成"你好"后,再次相同 → 去重
        assert!(!h.push(text_item("你好", 5)));
        assert_eq!(h.items.len(), 3);
    }

    #[test]
    fn cap_truncates_oldest() {
        let mut h = History {
            max_items: 3,
            ..Default::default()
        };
        for i in 0..5u64 {
            assert!(h.push(text_item(&format!("item{i}"), i)));
        }
        assert_eq!(h.items.len(), 3);
        assert_eq!(h.items[0].payload, "item4"); // 最新在头部
        assert_eq!(h.items[2].payload, "item2"); // item0/item1 被淘汰
    }

    #[test]
    fn zero_max_records_nothing() {
        let mut h = History {
            max_items: 0,
            ..Default::default()
        };
        assert!(!h.push(text_item("x", 0)));
        assert!(h.items.is_empty());
    }

    #[test]
    fn save_load_roundtrip() {
        let p = tmp_file("roundtrip");
        let items = vec![
            ClipboardItem {
                tp: "text".to_string(),
                payload: "你好".to_string(),
                ts: 10,
            },
            ClipboardItem {
                tp: "image".to_string(),
                payload: "C:\\tmp\\a.png".to_string(),
                ts: 9,
            },
        ];
        assert!(save_to_file(&p, &items));
        let loaded = load_from_file(&p);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].payload, "你好");
        assert_eq!(loaded[1].tp, "image");
        assert_eq!(loaded[1].ts, 9);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_missing_or_corrupt_is_empty() {
        let p = tmp_file("missing");
        assert!(load_from_file(&p).is_empty());
        std::fs::write(&p, "{oops").unwrap();
        assert!(load_from_file(&p).is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn copy_back_out_of_bounds_is_error() {
        let mut h = History {
            max_items: 10,
            ..Default::default()
        };
        h.push(text_item("a", 1));
        assert!(h.items.get(0).is_some());
        assert!(h.items.get(1).is_none());
        assert!(h.items.get(usize::MAX).is_none());
    }

    #[test]
    fn pick_item_text_priority_and_file_path() {
        let ts = 42;
        // 文本存在 → 文本条目(且裁剪空白)
        let item = pick_item(Some("  hello  ".to_string()), vec!["C:\\a.txt".to_string()], ts);
        let item = item.unwrap();
        assert_eq!(item.payload, "hello");
        assert_eq!(item.tp, "text");
        // 文本空白 → 回落文件路径
        let item = pick_item(Some("   ".to_string()), vec!["C:\\a.png".to_string()], ts);
        let item = item.unwrap();
        assert_eq!(item.tp, "image");
        assert_eq!(item.payload, "C:\\a.png");
        // 文本超长 → 回落文件路径
        let big = "x".repeat(MAX_TEXT_BYTES + 1);
        let item = pick_item(Some(big), vec!["C:\\b.png".to_string()], ts);
        assert!(item.is_some());
        assert_eq!(item.unwrap().tp, "image");
        // 全空 → None
        assert!(pick_item(None, vec![], ts).is_none());
        assert!(pick_item(Some("   ".to_string()), vec![], ts).is_none());
        assert!(pick_item(None, vec!["   ".to_string()], ts).is_none());
    }
}
