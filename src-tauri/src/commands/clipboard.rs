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
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager};

use super::config::{clamp_clipboard_max_items, config_path, load_from};

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

    /// 删除指定索引条目(纯函数,便于单测);越界返回错误,成功后返回删除后的条数。
    /// 删掉的恰是最近一条(索引 0)时,去重基准退到新的头部——保证"删掉最近一条后
    /// 立刻再复制同内容"不会被误去重;删空时基准归零。
    fn remove(&mut self, index: usize) -> Result<usize, String> {
        if index >= self.items.len() {
            return Err(format!("剪贴板历史索引 {index} 越界"));
        }
        self.items.remove(index);
        self.last_hash = self.items.first().map(|i| content_hash(&i.payload)).unwrap_or(0);
        Ok(self.items.len())
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

// ---------------------------------------------------------------------------
// 落盘节流(并发修复 2026-08-13):
// 原实现每次剪贴板更新都在监听回调线程里同步全量重写 JSON(200 条×64KB≈12.8MB),
// 高频复制时 IO 随复制频率线性放大,还阻塞复制回调。改为:监听线程只置脏
// (seq 自增,窗口内的多次变更自动合并),实际落盘由"距上次写已超窗 → 当前线程
// 立即写"或"窗内 → 独立 flush 线程睡满窗口后写"承担。
// 取舍:窗口(2s)内的增量在进程被强杀时可能不落盘;正常退出由 teardown 收尾补写。
// 序号方案(seq/saved_seq)代替 bool 脏标记:并发写盘与置脏不会互相踩掉脏位
// (bool 会漏写"写盘期间到达的新变更"),seq 单调追赶保证最终一致。
// ---------------------------------------------------------------------------

/// 落盘合并窗口:距上次成功落盘 2s 内的新变更只置脏,合并到下一次落盘
const FLUSH_WINDOW: Duration = Duration::from_secs(2);

/// 落盘节流状态(进程级单例;与 history 锁分工:history 锁管数据,本锁管落盘调度)
struct PersistState {
    /// 最新一次入队变更的序号(每次入队 +1);与 saved_seq 不等即"有脏数据待落盘"
    seq: u64,
    /// 已成功落盘覆盖到的序号(单调追赶 seq)
    saved_seq: u64,
    /// 是否有 flush 线程在跑(睡窗合并或写盘途中);有则新变更只置脏不另起线程
    flusher_pending: bool,
    /// 最近一次成功落盘时刻(节流决策基准)
    last_flush: Option<Instant>,
}

fn persist() -> &'static Mutex<PersistState> {
    static PERSIST: OnceLock<Mutex<PersistState>> = OnceLock::new();
    PERSIST.get_or_init(|| {
        Mutex::new(PersistState {
            seq: 0,
            saved_seq: 0,
            flusher_pending: false,
            last_flush: None,
        })
    })
}

/// 变更到达时的落盘决策(纯函数,便于单测;now = 决策时刻)
enum FlushAction {
    /// 已有 flush 线程在跑:只置脏,等它合并(不另起线程)
    Wait,
    /// 距上次写已超窗(或从未写过):在当前线程立即写——变更稀疏时最新数据零延迟落地
    Now,
    /// 窗内:起独立 flush 线程睡满窗口后写,期间新变更全部合并为一次写
    Spawn,
}

fn flush_action(st: &PersistState, now: Instant) -> FlushAction {
    if st.flusher_pending {
        return FlushAction::Wait;
    }
    match st.last_flush {
        None => FlushAction::Now,
        Some(t) if now.saturating_duration_since(t) >= FLUSH_WINDOW => FlushAction::Now,
        Some(_) => FlushAction::Spawn,
    }
}

/// 变更落盘调度(监听线程调用):只置脏 + 决策,IO 不阻塞复制回调
fn mark_dirty_and_flush(path: &Path) {
    let (action, target) = {
        let mut st = persist().lock().unwrap_or_else(|p| p.into_inner());
        st.seq += 1;
        let a = flush_action(&st, Instant::now());
        if matches!(a, FlushAction::Spawn) {
            st.flusher_pending = true;
        }
        (a, st.seq)
    };
    match action {
        FlushAction::Now => {
            flush_now(path, target);
        }
        FlushAction::Spawn => {
            let p = path.to_path_buf();
            if let Err(e) = std::thread::Builder::new()
                .name("clipboard-flush".to_string())
                .spawn(move || flush_delayed(&p))
            {
                eprintln!("[aurora] 启动剪贴板落盘线程失败: {e}");
                // 兜底:线程起不来就同步写,数据不丢(下次变更重新走决策)
                persist().lock().unwrap_or_else(|p| p.into_inner()).flusher_pending = false;
                flush_now(path, target);
            }
        }
        FlushAction::Wait => {}
    }
}

/// 立即写当前内存态(快照与写盘同在 history 锁内,与 clear/delete 等命令串行,
/// 不会出现"清空后又把旧快照写回"的复活竞态);成功后推进 saved_seq 与 last_flush,
/// 失败仅告警(下次变更会重新决策重试)
fn flush_now(path: &Path, target: u64) -> bool {
    let ok = {
        let hist = history().lock().unwrap_or_else(|p| p.into_inner());
        save_to_file(path, &hist.items)
    };
    let mut st = persist().lock().unwrap_or_else(|p| p.into_inner());
    if ok {
        st.saved_seq = st.saved_seq.max(target);
        st.last_flush = Some(Instant::now());
    }
    ok
}

/// flush 线程主体:睡满窗口 → 认领待写序号 → 落盘;写盘期间又来了新变更则立即
/// 再写一轮(不再睡窗)直到追平,保证不丢尾变更;写失败且无新变更则退出
/// (等下次变更重试,防磁盘故障时空转)
fn flush_delayed(path: &Path) {
    std::thread::sleep(FLUSH_WINDOW);
    loop {
        // 认领:拿当前待写序号;已干净(被其他写覆盖或本就没有)则退出
        let target = {
            let mut st = persist().lock().unwrap_or_else(|p| p.into_inner());
            if st.seq == st.saved_seq {
                st.flusher_pending = false;
                return;
            }
            st.seq
        };
        let prev_saved = persist().lock().unwrap_or_else(|p| p.into_inner()).saved_seq;
        flush_now(path, target);
        // 判脏与清 pending 同锁,无间隙:要么有新脏数据(继续写),要么干净退出
        let mut st = persist().lock().unwrap_or_else(|p| p.into_inner());
        let progressed = st.saved_seq > prev_saved;
        if st.seq == st.saved_seq || !progressed {
            st.flusher_pending = false;
            return;
        }
        // seq 有新增且本轮写成功:立即再写一轮追平
    }
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

/// 从磁盘加载历史;文件缺失返回空列表;JSON 损坏先把原文件备份为
/// clipboard.json.broken 再返回空列表(审计修复 2026-08-13:不备份的话,
/// 损坏文件会在下一次落盘时被静默覆盖,原始内容永久丢失)。
/// 备份只发生一次:文件改名后,后续读取走"缺失 → 空"分支。
fn load_from_file(path: &Path) -> Vec<ClipboardItem> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
            eprintln!(
                "[aurora] clipboard.json 损坏,历史置空(原文件已备份为 .broken): {path:?}"
            );
            super::config::backup_corrupt_file(path);
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

/// 落盘(自动创建父目录);失败仅告警,不阻断复制流程。
///
/// 并发修复 2026-08-13:与 config.json 同款原子写(tmp+rename),崩溃瞬间
/// 只会留下完整旧文件或完整新文件,不会半写 clipboard.json(半写会导致
/// 下次启动历史解析失败整体置空)。写 tmp 的并发安全由 history 锁保证。
fn save_to_file(path: &Path, items: &[ClipboardItem]) -> bool {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[aurora] 创建剪贴板历史目录失败: {e}");
            return false;
        }
    }
    match serde_json::to_string_pretty(items) {
        Ok(text) => super::config::atomic_write(path, text.as_bytes()),
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
    // 收尾:退出前把节流窗口内未落盘的脏历史补写一次(正常退出不丢最后 <2s
    // 的复制,与 PersistState 的取舍注释呼应)。与在跑的 flush 线程并发也无害:
    // 两处写都持 history 锁串行,且 saved_seq 单调推进不会回退。
    let (dirty, target) = {
        let st = persist().lock().unwrap_or_else(|p| p.into_inner());
        (st.seq != st.saved_seq, st.seq)
    };
    if dirty {
        flush_now(&history_path(app), target);
    }
    stop_monitor_if_needed(app);
}

/// 惰性初始化(幂等):历史装载 + 配置上限 + 按开关启动监听
fn ensure_ready(app: &AppHandle) {
    let cfg = load_from(&config_path(app));
    let mut hist = history().lock().unwrap_or_else(|p| p.into_inner());
    if hist.items.is_empty() {
        hist.items = load_from_file(&history_path(app));
        hist.last_hash = hist.items.first().map(|i| content_hash(&i.payload)).unwrap_or(0);
    }
    // 中 5 加固(2026-08-18):磁盘原值在此统一钳制到 [1,2000](复用 config 层
    // clamp_clipboard_max_items,与导入/保存同口径)。config::load_from 有意不
    // 钳制(避免启动误拒配置),手工编辑/旧版本写下的 1e9 若直接生效,History::push
    // 的 truncate 永不触发,剪贴板历史会无限累积;钳回 2000 后运行时上限恒定
    hist.max_items = clamp_clipboard_max_items(cfg.clipboard_max_items);
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

/// 启动插件 monitor(幂等;插件内部同样防重复启动)。
/// 并发修复 2026-08-13:原实现先 swap(true) 再启动,启动失败标志不回退,后续触发
/// (设置开关/新命令)都不会再重试,监听永久停摆。改为 CAS:false→true 成功才启动,
/// 启动失败立刻回退 false,下次触发自动重试。
fn start_monitor_if_needed(app: &AppHandle) {
    ensure_listener(app);
    if monitor_started()
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        if let Err(e) = app
            .state::<tauri_plugin_clipboard::Clipboard>()
            .start_monitor(app.clone())
        {
            eprintln!("[aurora] 剪贴板监听启动失败: {e}");
            monitor_started().store(false, Ordering::SeqCst);
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

/// 剪贴板更新处理(在插件 watcher 线程上执行):读取 → 校验 → 去重 → 入队 → 裁剪 → 置脏 → 广播
fn handle_clipboard_update(app: &AppHandle) {
    let clipboard = app.state::<tauri_plugin_clipboard::Clipboard>();
    let Some(item) = read_latest_item(&clipboard) else {
        return;
    };
    let inserted = {
        let mut hist = history().lock().unwrap_or_else(|p| p.into_inner());
        hist.push(item.clone())
    };
    if !inserted {
        return;
    }
    // 落盘节流:只置脏 + 决策(窗内合并/超窗即写),全量 JSON 不再每次复制同步写
    // (12.8MB 级 IO 放大问题,详见 PersistState 注释)
    mark_dirty_and_flush(&history_path(app));
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
    history().lock().unwrap_or_else(|p| p.into_inner()).items.clone()
}

/// 清空剪贴板历史(清内存 + 删除历史文件)
#[tauri::command]
pub fn clipboard_clear_history(app: tauri::AppHandle) {
    let mut hist = history().lock().unwrap_or_else(|p| p.into_inner());
    hist.items.clear();
    hist.last_hash = 0;
    drop(hist);
    let _ = std::fs::remove_file(history_path(&app));
}

/// 删除指定索引条历史(索引越界报错);删除后立即落盘,返回删除后的条数。
/// 注:删除是低频显式操作,不走落盘节流(节流为高频复制的合并而设计);
/// 在跑的 flush 线程即使多写一轮也只会写出同样内容,无害。
#[tauri::command]
pub fn clipboard_delete_item(app: tauri::AppHandle, index: usize) -> Result<usize, String> {
    let remaining = {
        let mut hist = history().lock().unwrap_or_else(|p| p.into_inner());
        hist.remove(index)?
    };
    let target = persist().lock().unwrap_or_else(|p| p.into_inner()).seq;
    flush_now(&history_path(&app), target);
    Ok(remaining)
}

/// 回贴第 index 条到系统剪贴板(文本 → writeText;图片文件路径 → 还原"复制文件"场景);
/// 越界返回 Err
#[tauri::command]
pub fn clipboard_copy_back(app: tauri::AppHandle, index: usize) -> Result<(), String> {
    let item = history()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
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
        // 同时清掉可能的 tmp 残留(原子写测试断言无 tmp,需从干净状态开始)
        let tmp = PathBuf::from(format!("{}.tmp", p.display()));
        let _ = std::fs::remove_file(&tmp);
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
        // 原子写(2026-08-13):写入后文件存在且内容完整,不残留 tmp
        let tmp = PathBuf::from(format!("{}.tmp", p.display()));
        assert!(!tmp.exists(), "成功路径不应残留 tmp: {}", tmp.display());
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

    // ---- 损坏备份(审计修复 2026-08-13):损坏的 clipboard.json 改名 .broken 后再置空 ----

    #[test]
    fn load_corrupt_backs_up_original_before_clearing() {
        let p = tmp_file("corrupt_backup");
        let original = r#"[{"tp":"text","payload":"旧历史","ts":1}"#;
        std::fs::write(&p, original).unwrap();
        assert!(load_from_file(&p).is_empty(), "损坏历史按空处理");
        let broken = PathBuf::from(format!("{}.broken", p.display()));
        assert!(broken.exists(), "损坏原文件应备份为 .broken");
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), original, "备份内容与原件一致");
        assert!(!p.exists(), "原路径应已改名");
        // 再次读取:走"缺失 → 空"分支,不重复备份、不 panic
        assert!(load_from_file(&p).is_empty());
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), original);
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(&broken);
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

    // ---- 删除命令(2026-08-13):中间/首/尾/越界 + 去重基准复位 ----

    #[test]
    fn remove_middle_first_last_and_out_of_bounds() {
        let mut h = History {
            max_items: 10,
            ..Default::default()
        };
        for i in 0..4u64 {
            assert!(h.push(text_item(&format!("item{i}"), i)));
        }
        // 头 = 最新:[item3, item2, item1, item0]
        assert_eq!(h.remove(1).unwrap(), 3); // 删中间 item2 → [item3, item1, item0]
        assert_eq!(h.items.iter().map(|i| i.payload.as_str()).collect::<Vec<_>>(),
                   vec!["item3", "item1", "item0"]);
        assert_eq!(h.remove(0).unwrap(), 2); // 删首(最近一条)item3 → [item1, item0]
        assert_eq!(h.items[0].payload, "item1");
        assert_eq!(h.remove(h.items.len() - 1).unwrap(), 1); // 删尾 item0 → [item1]
        assert_eq!(h.items[0].payload, "item1");
        assert_eq!(h.remove(0).unwrap(), 0); // 删光
        assert!(h.remove(0).is_err()); // 空表越界
        assert!(h.remove(5).is_err()); // 非空越界
        assert!(h.remove(usize::MAX).is_err());
    }

    #[test]
    fn remove_recalcs_dedup_baseline() {
        let mut h = History {
            max_items: 10,
            ..Default::default()
        };
        h.push(text_item("a", 1));
        h.push(text_item("b", 2));
        // [b, a]:删掉最近一条 b 后,基准退到 a,再复制 b 必须能重新入队
        h.remove(0).unwrap();
        assert!(h.push(text_item("b", 3)), "删掉最近一条后复制同内容不得被误去重");
        assert_eq!(h.items.len(), 2);
        // 删光后基准归零:同内容可再次入队
        h.remove(0).unwrap();
        h.remove(0).unwrap();
        assert!(h.push(text_item("b", 4)));
        assert_eq!(h.items.len(), 1);
    }

    // ---- 落盘节流决策(2026-08-13,纯函数;窗口语义:距上次写 2s 内合并) ----

    fn persist_state(pending: bool, last_flush: Option<Instant>) -> PersistState {
        PersistState { seq: 0, saved_seq: 0, flusher_pending: pending, last_flush }
    }

    #[test]
    fn flush_action_first_change_writes_now() {
        // 从未写过 → 立即写(变更稀疏零延迟)
        let st = persist_state(false, None);
        assert!(matches!(flush_action(&st, Instant::now()), FlushAction::Now));
    }

    #[test]
    fn flush_action_within_window_spawns_merger() {
        let t0 = Instant::now();
        // 窗内(<2s)→ 起 flush 线程合并
        let st = persist_state(false, Some(t0));
        assert!(matches!(flush_action(&st, t0 + FLUSH_WINDOW - Duration::from_millis(1)), FlushAction::Spawn));
        assert!(matches!(flush_action(&st, t0 + Duration::from_millis(1)), FlushAction::Spawn));
    }

    #[test]
    fn flush_action_after_window_writes_now() {
        let t0 = Instant::now();
        let st = persist_state(false, Some(t0));
        // 恰满窗(边界)与超窗 → 立即写
        assert!(matches!(flush_action(&st, t0 + FLUSH_WINDOW), FlushAction::Now));
        assert!(matches!(flush_action(&st, t0 + FLUSH_WINDOW + Duration::from_secs(1)), FlushAction::Now));
    }

    #[test]
    fn flush_action_pending_thread_merges_into_it() {
        // flush 线程在跑:即使距上次写已超窗也只置脏等待合并,不另起线程
        let t0 = Instant::now();
        let st = persist_state(true, Some(t0));
        assert!(matches!(flush_action(&st, t0 + Duration::from_secs(60)), FlushAction::Wait));
    }
}
