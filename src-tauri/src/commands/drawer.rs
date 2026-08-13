//! 2.2 FileDrawer 桌面文件抽屉。
//!
//! - `drawer_list_files` / `drawer_refresh`:扫"资源管理器桌面"= 用户桌面(FOLDERID_Desktop)
//!   + 公共桌面(FOLDERID_PublicDesktop)合并,按扩展名分组(见 classify.rs),
//!   **逻辑收纳**——文件原位不动,只读展示;两个桌面同名文件以用户桌面为准;
//! - `drawer_open`:仅允许桌面目录内路径,转发 Phase1 `open_item`(文件/文件夹通吃);
//! - `init_watcher`:notify(ReadDirectoryChangesW 事件驱动,非轮询)监听桌面目录,
//!   变化 200ms 防抖后重扫 → 更新内存缓存 → emit "drawer-updated"(payload 空,信号用途),
//!   由集成 agent 在 lib.rs setup 中 enable_file_drawer=true 时调用;
//! - 上限保护:条目超过 [MAX_FILES] 时只展示排序后前 1500。
//!
//! 命令签名与 docs/Phase2-设计.md §2.2 及 commands/stubs.rs 占位完全一致。

use notify::Watcher;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use super::classify::{self, CATEGORY_ORDER};

/// FILE_ATTRIBUTE_HIDDEN(0x2;文件管理器"隐藏"属性)
#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

/// 单次展示的文件/文件夹条目上限(防 WebView 渲染卡顿)
pub const MAX_FILES: usize = 1500;

/// 桌面文件条目(逻辑收纳:仅展示,不移动/删除)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrawerFile {
    pub name: String,
    pub path: String,
    pub ext: String,
    pub is_dir: bool,
}

/// 分类分组(分类名见 classify::CATEGORY_ORDER,固定顺序,空组也返回)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrawerGroup {
    pub category: String,
    pub files: Vec<DrawerFile>,
}

/// 扫描结果:分组(命令只返回 groups;截断见 [MAX_FILES])
pub struct ScanResult {
    pub groups: Vec<DrawerGroup>,
}

/// 内存缓存:watcher 事件驱动刷新后写入;drawer_list_files 优先读缓存(免重复磁盘扫描)
static CACHE: Mutex<Option<Vec<DrawerGroup>>> = Mutex::new(None);

/// 用户桌面目录(FOLDERID_Desktop;桌面可被库重定向,不硬编码 C:\Users\...\Desktop)
pub fn desktop_dir() -> Option<PathBuf> {
    known_folders::get_known_folder_path(known_folders::KnownFolder::Desktop)
}

/// 公共桌面目录(FOLDERID_PublicDesktop,通常为 C:\Users\Public\Desktop)。
/// 软件安装时常把快捷方式放这里,资源管理器将公共桌面与用户桌面合并显示,
/// 抽屉必须一并扫描,否则用户会看到桌面图标缺失(如"联想浏览器")。
pub fn public_desktop_dir() -> Option<PathBuf> {
    known_folders::get_known_folder_path(known_folders::KnownFolder::PublicDesktop)
}

/// 把 secondary 的条目并入 primary 分组(重名跳过,primary 优先);
/// primary 需已按 CATEGORY_ORDER 建好全部分组(scan_dir 保证)。
fn merge_groups(primary: &mut Vec<DrawerGroup>, secondary: Vec<DrawerGroup>) {
    for g in secondary {
        for f in g.files {
            // 两个桌面同名文件:保留 primary 那份(与资源管理器"用户桌面优先"一致)
            let dup = primary
                .iter()
                .any(|g2| g2.files.iter().any(|f2| f2.name == f.name));
            if dup {
                continue;
            }
            if let Some(slot) = primary.iter_mut().find(|g2| g2.category == g.category) {
                slot.files.push(f);
            }
        }
    }
}

/// 隐藏判定:文件名以 . 开头(Unix 约定,如 .gitignore)或带 Windows 隐藏属性。
/// 两种都滤,防"手动隐藏"与"点开头文件"漏网。
fn is_hidden(meta: &std::fs::Metadata, name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// 扫描指定目录**顶层**条目(非递归),按名称排序分组,超 [MAX_FILES] 截断;
/// 隐藏文件/文件夹(属性隐藏或 . 开头)不显示。
/// 目录不存在/不可读返回 Err;个别条目元数据读取失败时跳过。
pub fn scan_dir(dir: &Path) -> Result<ScanResult, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    // 按文件名大小写不敏感排序(截断先于分组,保证留下的是字典序最前的一批)
    entries.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    entries.truncate(MAX_FILES);

    // 固定顺序建组(空组也保留,前端左侧分类 tab 稳定)
    let mut groups: Vec<DrawerGroup> = CATEGORY_ORDER
        .iter()
        .map(|c| DrawerGroup {
            category: c.to_string(),
            files: Vec::new(),
        })
        .collect();

    for p in entries {
        let Ok(meta) = p.metadata() else { continue };
        let is_dir = meta.is_dir();
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // 隐藏文件/文件夹不显示:Windows 隐藏属性 或 文件名以 . 开头(如 .gitignore)
        if is_hidden(&meta, &name) {
            continue;
        }
        let category = classify::classify_file(&name, is_dir);
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file = DrawerFile {
            name,
            path: p.to_string_lossy().into_owned(),
            ext,
            is_dir,
        };
        if let Some(g) = groups.iter_mut().find(|g| g.category == category) {
            g.files.push(file);
        }
    }
    Ok(ScanResult { groups })
}

/// 扫描"资源管理器桌面" = 用户桌面 + 公共桌面,分组合并
/// (公共桌面缺失/不可读时退化为仅用户桌面,不影响主流程)
pub fn scan_desktop() -> Result<ScanResult, String> {
    let user = desktop_dir().ok_or_else(|| "无法获取用户桌面目录".to_string())?;
    let mut r = scan_dir(&user)?;
    if let Some(public) = public_desktop_dir() {
        if let Ok(pr) = scan_dir(&public) {
            merge_groups(&mut r.groups, pr.groups);
        }
    }
    Ok(r)
}

/// 候选路径是否位于根目录内:两端规范化(处理大小写/符号链接)后做组件级前缀比较。
/// 组件级比较天然拒绝前缀仿冒(如 Desktop2 不算 Desktop 内)。
pub fn path_is_within(root: &Path, candidate: &Path) -> bool {
    let Ok(root) = std::fs::canonicalize(root) else { return false };
    let Ok(candidate) = std::fs::canonicalize(candidate) else {
        return false;
    };
    candidate.starts_with(&root)
}

/// 仅允许打开桌面目录内的路径(用户桌面或公共桌面均可;文件/文件夹通吃,转发 Phase1 open_item)
pub fn open_within_desktop(path: &str) -> bool {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(u) = desktop_dir() {
        roots.push(u);
    }
    if let Some(p) = public_desktop_dir() {
        roots.push(p);
    }
    open_within_roots(&roots, path)
}

/// 候选路径位于任一根目录内时才放行(纯校验逻辑,单测直接测这里)
fn open_within_roots(roots: &[PathBuf], path: &str) -> bool {
    let candidate = Path::new(path);
    if !roots.iter().any(|r| path_is_within(r, candidate)) {
        eprintln!("[aurora] drawer_open 拒绝桌面目录外路径: {path}");
        return false;
    }
    super::search::open_item(path.to_string())
}

fn write_cache(groups: Vec<DrawerGroup>) {
    if let Ok(mut c) = CACHE.lock() {
        *c = Some(groups);
    }
}

fn read_cache() -> Option<Vec<DrawerGroup>> {
    CACHE.lock().map(|c| c.clone()).unwrap_or_default()
}

/// 重扫并更新缓存;失败时返回 None(调用方兜底为空列表)
fn refresh_scan() -> Option<Vec<DrawerGroup>> {
    match scan_desktop() {
        Ok(r) => {
            let groups = r.groups;
            write_cache(groups.clone());
            Some(groups)
        }
        Err(e) => {
            eprintln!("[aurora] 桌面扫描失败: {e}");
            None
        }
    }
}

/// 扫描桌面并按扩展名分组(优先读 watcher 维护的缓存)。
#[tauri::command]
pub fn drawer_list_files() -> Vec<DrawerGroup> {
    read_cache().or_else(refresh_scan).unwrap_or_default()
}

/// 打开抽屉内文件/文件夹(仅用户桌面/公共桌面内的路径)
#[tauri::command]
pub fn drawer_open(path: String) -> bool {
    open_within_desktop(&path)
}

/// 手动刷新(强制重扫并更新缓存)
#[tauri::command]
pub fn drawer_refresh() -> Vec<DrawerGroup> {
    refresh_scan().unwrap_or_default()
}

/// watcher 是否已启动(managed state 持有;热生效时用标志判断是否需要重建)
static WATCHER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 启动桌面目录 watcher(事件驱动,非轮询)。
/// 集成 agent 在 lib.rs setup 中调用一次即可;`enable_file_drawer` 关闭时自动跳过。
/// 目录变化(Create/Modify/Remove)→ 1s 防抖合并 → 重扫 → 更新缓存 →
/// emit "drawer-updated"(payload 空,信号用途)→ 前端收到后调 drawer_list_files 拉取。
/// 防抖 1s:桌面被大文件下载/日志持续写入时,合并高频事件为低频重扫,避免重扫风暴;
/// 同时后台预热一次缓存(打开抽屉即出数据,无需等首次同步扫描)。
/// watcher 放入 managed state 保活;热生效时 [stop_watcher] 可停止并重建。
/// 幂等:已启动时不重复创建。
///
/// 并发修复(2026-08-13):入口的"检查 WATCHER_ACTIVE → 置位"原为 load+store 两步,
/// 并发调用时两线程都能通过检查、都走到 app.manage —— tauri 对同类型重复 manage
/// 会 panic。现改为 compare_exchange 一步原子置位:抢到位的线程负责创建,抢不到的
/// 直接返回。抢到位后若失败(桌面目录缺失等)或开关关闭未启动,必须回滚标志,
/// 否则后续重试被短路、watcher 永远起不来。
pub fn init_watcher(app: tauri::AppHandle) -> notify::Result<()> {
    if WATCHER_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(()); // 已有线程在创建(或已创建),本调用直接返回
    }
    // 实际创建逻辑拆到内层,返回是否真正启动:false(开关关闭跳过)或 Err 都需回滚标志
    match init_watcher_inner(&app) {
        Ok(true) => Ok(()),
        Ok(false) => {
            WATCHER_ACTIVE.store(false, Ordering::SeqCst);
            Ok(())
        }
        Err(e) => {
            WATCHER_ACTIVE.store(false, Ordering::SeqCst);
            Err(e)
        }
    }
}

/// [init_watcher] 的内层实现(调用方已通过 CAS 拿到启动权)。
/// 返回 Ok(true)=watcher 已启动;Ok(false)=开关关闭,未启动。
fn init_watcher_inner(app: &tauri::AppHandle) -> notify::Result<bool> {
    // 设置开关关闭时不启动 watcher(不注册监听、不留句柄)
    let cfg_path = crate::commands::config::config_path(app);
    let cfg = crate::commands::config::load_from(&cfg_path);
    if !cfg.enable_file_drawer {
        return Ok(false);
    }
    let Some(desktop) = desktop_dir() else {
        return Err(notify::Error::generic("无法获取用户桌面目录,watcher 未启动"));
    };

    let (tx, rx) = std::sync::mpsc::channel::<notify::Event>();
    let mut watcher = notify::recommended_watcher(
        move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                use notify::EventKind::{Create, Modify, Remove};
                match ev.kind {
                    Create(_) | Modify(_) | Remove(_) => {
                        let _ = tx.send(ev);
                    }
                    _ => {} // 访问类等无关事件忽略
                }
            }
        },
    )?;
    watcher.watch(&desktop, notify::RecursiveMode::NonRecursive)?;
    // 公共桌面一并监听(软件安装可能往公共桌面放快捷方式;目录缺失则跳过)
    if let Some(public) = public_desktop_dir() {
        let _ = watcher.watch(&public, notify::RecursiveMode::NonRecursive);
    }
    // 保活 watcher(不 drop 即持续监听;热生效停止时替换回 None)。
    // 并发修复:热生效 stop→start 时 state 仍被 manage(stop_watcher 只 take 掉 watcher
    // 并不卸载 state),直接 manage 会因同类型重复注册 panic;先 try_state 探测,
    // 已存在则复用 state 放回新 watcher。
    if let Some(state) = app.try_state::<Mutex<Option<notify::RecommendedWatcher>>>() {
        if let Ok(mut g) = state.lock() {
            *g = Some(watcher);
        }
    } else {
        app.manage(Mutex::new(Some(watcher)));
    }

    // 后台预热缓存:首次打开抽屉即出数据,不用等同步扫描(失败无碍,首次打开兜底重扫)
    std::thread::spawn(|| {
        let _ = refresh_scan();
    });

    // 防抖线程:首个事件后 1s 窗口内的后续事件合并为一次重扫;
    // 停止后 rx 通道关闭(发送端随 watcher drop),recv 返回 Err 退出线程
    let app2 = app.clone();
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            while rx.recv_timeout(Duration::from_secs(1)).is_ok() {}
            refresh_scan();
            let _ = app2.emit("drawer-updated", ());
        }
    });
    Ok(true)
}

/// 停止桌面目录 watcher(热生效:开关关闭时调用)。
/// 从 managed state 取出 watcher drop 掉(系统句柄随之释放),通道关闭后防抖线程退出。
/// 幂等:未启动时无操作。
pub fn stop_watcher(app: &tauri::AppHandle) {
    if !WATCHER_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Some(state) = app.try_state::<Mutex<Option<notify::RecommendedWatcher>>>() {
        if let Ok(mut g) = state.lock() {
            let _ = g.take();
        }
    }
}

/// 热生效入口(config_save 后调用):开关开 → 启动 watcher(幂等);关 → 停止。
/// 失败仅告警,不阻断保存。
pub fn apply_config(app: tauri::AppHandle) -> notify::Result<()> {
    let cfg = crate::commands::config::load_from(&crate::commands::config::config_path(&app));
    if cfg.enable_file_drawer {
        init_watcher(app)
    } else {
        stop_watcher(&app);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建独立临时目录(按 pid 区分,防并行测试串扰)
    fn tmp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "aurora_drawer_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(p: &Path, content: &str) {
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn scan_groups_and_classifies_correctly() {
        let dir = tmp_dir("scan_mixed");
        write(&dir.join("a报告.docx"), "x");
        write(&dir.join("b.png"), "x");
        write(&dir.join("c.mp4"), "x");
        write(&dir.join("d.mp3"), "x");
        write(&dir.join("e.zip"), "x");
        write(&dir.join("f.exe"), "x");
        write(&dir.join("g.rs"), "x");
        write(&dir.join("h.xyz"), "x");
        write(&dir.join("i.txt"), "x");
        std::fs::create_dir(dir.join("项目资料")).unwrap();

        let r = scan_dir(&dir).unwrap();
        let shown: usize = r.groups.iter().map(|g| g.files.len()).sum();
        assert_eq!(shown, 10);
        // 全部 9 组都在,且按固定顺序
        let cats: Vec<&str> = r.groups.iter().map(|g| g.category.as_str()).collect();
        assert_eq!(cats, CATEGORY_ORDER.to_vec());
        let find = |cat: &str| {
            r.groups
                .iter()
                .find(|g| g.category == cat)
                .unwrap_or_else(|| panic!("缺少分组 {cat}"))
        };
        // 文件夹组:目录 1 个
        let folder = find(classify::CATEGORY_FOLDER);
        assert_eq!(folder.files.len(), 1);
        assert!(folder.files[0].is_dir);
        assert_eq!(folder.files[0].name, "项目资料");
        // 文档:docx + txt 两个,组内按名称排序
        let doc = find(classify::CATEGORY_DOC);
        assert_eq!(doc.files.len(), 2);
        assert_eq!(doc.files[0].name, "a报告.docx");
        assert_eq!(doc.files[1].name, "i.txt");
        assert_eq!(doc.files[0].ext, "docx");
        // 其余各一组一个
        assert_eq!(find(classify::CATEGORY_IMAGE).files.len(), 1);
        assert_eq!(find(classify::CATEGORY_VIDEO).files.len(), 1);
        assert_eq!(find(classify::CATEGORY_AUDIO).files.len(), 1);
        assert_eq!(find(classify::CATEGORY_ARCHIVE).files.len(), 1);
        assert_eq!(find(classify::CATEGORY_PROGRAM).files.len(), 1);
        assert_eq!(find(classify::CATEGORY_CODE).files.len(), 1);
        assert_eq!(find(classify::CATEGORY_OTHER).files.len(), 1);
        // 路径为绝对路径且可解析
        assert!(Path::new(&doc.files[0].path).is_absolute());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_empty_dir_returns_all_empty_groups() {
        let dir = tmp_dir("scan_empty");
        let r = scan_dir(&dir).unwrap();
        assert_eq!(r.groups.len(), CATEGORY_ORDER.len());
        assert!(r.groups.iter().all(|g| g.files.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_truncates_at_max_files() {
        let dir = tmp_dir("scan_cap");
        // 造 1600 个文件:名称字典序排序后前 1500 个保留
        for i in 0..1600 {
            write(&dir.join(format!("file{i:04}.txt")), "x");
        }
        let r = scan_dir(&dir).unwrap();
        let shown: usize = r.groups.iter().map(|g| g.files.len()).sum();
        assert_eq!(shown, MAX_FILES, "展示数被截到上限");
        let names: Vec<&str> = r
            .groups
            .iter()
            .flat_map(|g| g.files.iter())
            .map(|f| f.name.as_str())
            .collect();
        // 字典序最前的保留了,最后的被截掉
        assert!(names.contains(&"file0000.txt"));
        assert!(names.contains(&"file1499.txt"));
        assert!(!names.contains(&"file1599.txt"));
        // 组内与全局保持名称升序
        assert!(names.windows(2).all(|w| w[0] <= w[1]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 手写 kernel32 声明(避免为单测加 windows-sys feature;与 system_sampler 手写 FFI 同款)
    #[cfg(windows)]
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileAttributesW(lp_file_name: *const u16, dw_file_attributes: u32) -> i32;
    }

    /// 把文件设为 Windows 隐藏属性(用户手动"隐藏"场景)
    #[cfg(windows)]
    fn set_hidden_attr(p: &Path) {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            SetFileAttributesW(wide.as_ptr(), 0x2); // FILE_ATTRIBUTE_HIDDEN
        }
    }

    /// 隐藏文件(. 开头或 Windows 隐藏属性)不显示;其余正常分类
    #[test]
    fn hidden_files_are_filtered_out() {
        let dir = tmp_dir("scan_hidden");
        write(&dir.join(".gitignore"), "node_modules");
        write(&dir.join("README"), "x");
        #[cfg(windows)]
        {
            let secret = dir.join("secret.txt");
            write(&secret, "x");
            set_hidden_attr(&secret);
        }
        let r = scan_dir(&dir).unwrap();
        let shown: usize = r.groups.iter().map(|g| g.files.len()).sum();
        #[cfg(windows)]
        assert_eq!(shown, 1, ".gitignore 与隐藏属性文件都应被过滤");
        #[cfg(not(windows))]
        assert_eq!(shown, 1, ".gitignore 应被过滤");
        // 只剩 README,落在 other(无扩展名)
        let other = r
            .groups
            .iter()
            .find(|g| g.category == classify::CATEGORY_OTHER)
            .unwrap();
        assert_eq!(other.files.len(), 1);
        assert_eq!(other.files[0].name, "README");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_missing_dir_returns_err() {
        let dir = std::env::temp_dir().join(format!(
            "aurora_drawer_nonexist_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(scan_dir(&dir).is_err());
    }

    #[test]
    fn path_is_within_accepts_children_rejects_others() {
        let root = tmp_dir("within");
        std::fs::create_dir(root.join("sub")).unwrap();
        write(&root.join("sub").join("inner.txt"), "x");
        write(&root.join("top.txt"), "x");
        // 桌面内:根自身 / 直接子文件 / 子目录内文件
        assert!(path_is_within(&root, &root));
        assert!(path_is_within(&root, &root.join("top.txt")));
        assert!(path_is_within(&root, &root.join("sub").join("inner.txt")));
        // 目录外:兄弟目录 / 父目录 / 前缀仿冒(Desktop2)
        let sibling = std::env::temp_dir().join("aurora_drawer_within_sibling");
        std::fs::create_dir_all(sibling.join("sub2")).unwrap();
        write(&sibling.join("sub2").join("inner.txt"), "x");
        assert!(!path_is_within(&root, &sibling));
        assert!(!path_is_within(&root, &sibling.join("sub2").join("inner.txt")));
        assert!(!path_is_within(&root, root.parent().unwrap()));
        let mimic = std::env::temp_dir().join("aurora_drawer_within_Desktop2");
        std::fs::create_dir(&mimic).unwrap();
        assert!(!path_is_within(&root, &mimic));
        // 不存在的路径一律拒绝
        assert!(!path_is_within(&root, &root.join("不存在.txt")));
        assert!(!path_is_within(&root, &std::env::temp_dir().join("aurora_drawer_nonexist_zzz")));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
        let _ = std::fs::remove_dir_all(&mimic);
    }

    #[test]
    fn open_rejects_path_outside_desktop() {
        let desktop = tmp_dir("open_desktop");
        let public = tmp_dir("open_public");
        let outside = tmp_dir("open_outside");
        write(&desktop.join("ok.txt"), "x");
        write(&public.join("pub.txt"), "x");
        write(&outside.join("evil.txt"), "x");
        let roots = [desktop.clone(), public.clone()];
        // 桌面外路径:拒绝,不触发打开
        let outside_path = outside.join("evil.txt").to_string_lossy().into_owned();
        assert!(!open_within_roots(&roots, &outside_path));
        // 用户桌面内与公共桌面内路径都放行(真实打开由 opener 承担,单测不触发)
        assert!(path_is_within(&desktop, &desktop.join("ok.txt")));
        assert!(path_is_within(&public, &public.join("pub.txt")));
        // 不存在路径:拒绝
        let missing = desktop.join("不存在.txt").to_string_lossy().into_owned();
        assert!(!open_within_roots(&roots, &missing));
        let _ = std::fs::remove_dir_all(&desktop);
        let _ = std::fs::remove_dir_all(&public);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 用户桌面 + 公共桌面合并:独有文件都并入;同名文件只留用户桌面那份
    #[test]
    fn merge_groups_user_desktop_wins_on_duplicate() {
        let primary = tmp_dir("merge_pri");
        let secondary = tmp_dir("merge_sec");
        write(&primary.join("同名.txt"), "user");
        write(&primary.join("user_only.txt"), "x");
        write(&secondary.join("同名.txt"), "public");
        write(&secondary.join("public_only.txt"), "x");
        let mut r = scan_dir(&primary).unwrap();
        let sec = scan_dir(&secondary).unwrap();
        merge_groups(&mut r.groups, sec.groups);
        let shown: usize = r.groups.iter().map(|g| g.files.len()).sum();
        assert_eq!(shown, 3, "同名只留一份,两个独有文件都并入");
        let names: Vec<&str> = r
            .groups
            .iter()
            .flat_map(|g| g.files.iter())
            .map(|f| f.name.as_str())
            .collect();
        assert!(names.contains(&"user_only.txt"));
        assert!(names.contains(&"public_only.txt"));
        assert_eq!(names.iter().filter(|n| **n == "同名.txt").count(), 1);
        // 留下的同名条目来自用户桌面
        let doc = r
            .groups
            .iter()
            .find(|g| g.category == classify::CATEGORY_DOC)
            .unwrap();
        let dup = doc.files.iter().find(|f| f.name == "同名.txt").unwrap();
        assert_eq!(dup.path, primary.join("同名.txt").to_string_lossy());
        let _ = std::fs::remove_dir_all(&primary);
        let _ = std::fs::remove_dir_all(&secondary);
    }

    #[test]
    fn desktop_dir_resolves_to_existing_dir() {
        // 本机 Windows:known_folders 必须能解析用户桌面目录
        let d = desktop_dir();
        assert!(d.is_some(), "无法解析 FOLDERID_Desktop");
        assert!(d.unwrap().is_dir());
    }
}
