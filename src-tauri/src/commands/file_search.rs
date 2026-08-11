//! 3.3 自然语言文件搜索。
//!
//! - `ai_search_files`:供前端与 AI 工具(3.2 的 search_files)两用;
//! - 目录白名单:dirs ⊆ ai_search_roots ∪ {桌面},越权/相对路径目录忽略(不报错,防 AI 越界);
//!   未指定 dirs → ai_search_roots,仍空 → 仅桌面(**禁止全盘扫描**铁律);
//! - 扩展名自然语言提示(`ext_hint_from_query`):query 含 pdf/word(文档)/excel(表格)/
//!   图片(照片)/视频 词时叠加扩展名过滤,大小写不敏感;
//! - 边界保护:递归深度 ≤[MAX_DEPTH]、每层条目排序后截断 [MAX_PER_LEVEL]、全局结果
//!   去重排序后截断 [MAX_RESULTS];目录读取错误跳过不中断;
//! - 命令 async + spawn_blocking:目录扫描耗时不可控,防阻塞主线程。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 单次搜索结果条目(对齐开发文档 §5 的 `{name, full_path, is_dir}[]` 契约)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileHit {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
}

/// 递归深度上限:根本身为 0,最多扫到根下第 3 层子目录(第 4 层目录内容不扫)
const MAX_DEPTH: usize = 3;
/// 每层条目上限(巨目录保护:按名称排序后先行截断,再匹配与入下一层)
const MAX_PER_LEVEL: usize = 500;
/// 全局结果上限
const MAX_RESULTS: usize = 20;

/// 图片扩展名提示(与 classify.rs 图片类保持一致,全项目统一)
const IMAGE_EXTS: [&str; 15] = [
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "ico", "tif", "tiff", "heic", "heif",
    "avif", "raw", "psd",
];

/// 视频扩展名提示(与 classify.rs 视频类保持一致;不含 ts——TS 源码更常见,与分类规则同理)
const VIDEO_EXTS: [&str; 15] = [
    "mp4", "avi", "mkv", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "rmvb", "rm", "3gp",
    "vob", "m2ts",
];

/// 从查询词中提取扩展名过滤提示(纯函数,大小写不敏感):
/// pdf / word(文档)/ excel(表格)/ 图片(照片)/ 视频 → 对应扩展名列表;无提示词 → None(不过滤)。
/// 多条提示词同时出现时按固定顺序取首个命中(pdf > word > excel > 图片 > 视频)。
pub fn ext_hint_from_query(query: &str) -> Option<Vec<&'static str>> {
    let q = query.to_ascii_lowercase();
    if q.contains("pdf") {
        Some(vec![".pdf"])
    } else if q.contains("word") || q.contains("文档") {
        Some(vec![".doc", ".docx"])
    } else if q.contains("excel") || q.contains("表格") {
        Some(vec![".xls", ".xlsx"])
    } else if q.contains("图片") || q.contains("照片") {
        Some(IMAGE_EXTS.to_vec())
    } else if q.contains("视频") {
        Some(VIDEO_EXTS.to_vec())
    } else {
        None
    }
}

/// 候选路径是否位于某个根目录内(组件级前缀比较,拒绝前缀仿冒;复用 drawer 的实现)
fn path_is_within(root: &Path, candidate: &Path) -> bool {
    super::drawer::path_is_within(root, candidate)
}

/// 用户桌面目录(FOLDERID_Desktop,复用 drawer.rs 只读函数)
fn desktop_dir() -> Option<PathBuf> {
    super::drawer::desktop_dir()
}

/// 目录是否在允许集合内(纯函数,可单测):
/// 必须为绝对路径,且位于某个 roots 目录之内(含自身与子目录)或桌面目录内;
/// roots 为空时仅桌面允许;桌面始终在允许集合内(白名单 = ai_search_roots ∪ {桌面})。
pub fn is_allowed_dir(dir: &str, roots: &[String]) -> bool {
    let p = Path::new(dir);
    if !p.is_absolute() {
        return false;
    }
    if roots.iter().any(|r| path_is_within(Path::new(r), p)) {
        return true;
    }
    desktop_dir()
        .as_deref()
        .is_some_and(|d| path_is_within(d, p))
}

/// 从 `%APPDATA%\com.aurora.desktop\config.json` 读 ai_search_roots(与 wallpaper 同模式)
fn configured_roots() -> Vec<String> {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return Vec::new();
    };
    let cfg_path = Path::new(&appdata).join("com.aurora.desktop").join("config.json");
    super::config::load_from(&cfg_path).ai_search_roots
}

/// 扩展名是否命中提示列表(大小写不敏感;无提示 → 恒通过)
fn ext_matches(path: &Path, ext_hint: Option<&[&str]>) -> bool {
    let Some(list) = ext_hint else { return true };
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false; // 有提示但文件无扩展名 → 不匹配
    };
    // hint 可能带点(".pdf")或裸扩展名("jpg"),统一剥点后比较
    list.iter().any(|hint| ext.eq_ignore_ascii_case(hint.trim_start_matches('.')))
}

/// 深度优先扫描:深度 ≤[MAX_DEPTH];每层条目按名称排序后截断 [MAX_PER_LEVEL];
/// 匹配 = 文件名小写 contains query_lower + 扩展名提示叠加;目录读取错误跳过,不中断整体。
fn dfs(
    dir: &Path,
    depth: usize,
    query_lower: &str,
    ext_hint: Option<&[&str]>,
    out: &mut Vec<FileHit>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return; // 权限不足/目录不存在:跳过该目录
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    entries.truncate(MAX_PER_LEVEL); // 巨目录保护:同层超限先行截断
    for p in entries {
        let Ok(meta) = p.metadata() else { continue };
        let is_dir = meta.is_dir();
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.to_lowercase().contains(query_lower) && ext_matches(&p, ext_hint) {
            out.push(FileHit { name, full_path: p.to_string_lossy().into_owned(), is_dir });
        }
        if is_dir {
            dfs(&p, depth + 1, query_lower, ext_hint, out);
        }
    }
}

/// 去重键:优先规范化路径(同一目录不同写法收敛),失败退回原串小写。
fn dedup_key(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_ascii_lowercase())
}

/// 核心搜索(同步,由命令层 spawn_blocking 包裹):
/// 目录集合 = dirs(白名单过滤后)或 roots 或仅桌面;
/// 结果全局去重 + 名称排序 + 截断 [MAX_RESULTS]。
fn search_in_roots(query: &str, dirs: &[String], roots: &[String]) -> Vec<FileHit> {
    let query_lower = query.to_lowercase();
    let ext_hint = ext_hint_from_query(query);

    // 确定扫描目录集合(白名单约束)
    let mut scan_dirs: Vec<PathBuf> = Vec::new();
    if !dirs.is_empty() {
        for d in dirs {
            if is_allowed_dir(d, roots) {
                scan_dirs.push(PathBuf::from(d));
            }
        }
    } else if !roots.is_empty() {
        scan_dirs.extend(roots.iter().map(PathBuf::from));
    } else if let Some(d) = desktop_dir() {
        scan_dirs.push(d); // dirs 与 roots 均为空 → 仅桌面
    }

    // 逐个根目录 DFS,全局去重
    let mut hits: Vec<FileHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for dir in &scan_dirs {
        let mut bucket: Vec<FileHit> = Vec::new();
        dfs(dir, 0, &query_lower, ext_hint.as_deref(), &mut bucket);
        for h in bucket {
            if seen.insert(dedup_key(&h.full_path)) {
                hits.push(h);
            }
        }
    }

    // 名称排序(大小写不敏感字典序)+ 全局截断
    hits.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    hits.truncate(MAX_RESULTS);
    hits
}

/// 搜索命令(供前端与 AI 工具两用)。
/// 目录扫描是阻塞 IO,用 spawn_blocking 移出 async 主线程,防卡 UI。
#[tauri::command]
pub async fn ai_search_files(query: String, dirs: Option<Vec<String>>) -> Vec<FileHit> {
    tauri::async_runtime::spawn_blocking(move || {
        let roots = configured_roots();
        let dirs = dirs.unwrap_or_default();
        search_in_roots(&query, &dirs, &roots)
    })
    .await
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建独立临时目录(按时间戳区分,防并行测试串扰)
    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("aurora_fs_{tag}_{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, b"x").unwrap();
    }

    /// 根目录字符串(测试 roots 参数用)
    fn root_str(root: &Path) -> String {
        root.to_string_lossy().into_owned()
    }

    // ---- ext_hint_from_query ----

    #[test]
    fn ext_hint_maps_all_groups() {
        assert_eq!(ext_hint_from_query("pdf"), Some(vec![".pdf"]));
        assert_eq!(ext_hint_from_query("查找 pdf 发票"), Some(vec![".pdf"]));
        assert_eq!(ext_hint_from_query("word"), Some(vec![".doc", ".docx"]));
        assert_eq!(ext_hint_from_query("文档"), Some(vec![".doc", ".docx"]));
        assert_eq!(ext_hint_from_query("excel"), Some(vec![".xls", ".xlsx"]));
        assert_eq!(ext_hint_from_query("表格"), Some(vec![".xls", ".xlsx"]));
        assert_eq!(ext_hint_from_query("图片"), Some(IMAGE_EXTS.to_vec()));
        assert_eq!(ext_hint_from_query("照片"), Some(IMAGE_EXTS.to_vec()));
        assert_eq!(ext_hint_from_query("视频"), Some(VIDEO_EXTS.to_vec()));
        // 无提示词 → None(纯文件名子串匹配)
        assert_eq!(ext_hint_from_query("发票"), None);
        assert_eq!(ext_hint_from_query("报告"), None);
    }

    #[test]
    fn ext_hint_is_case_insensitive() {
        assert_eq!(ext_hint_from_query("PDF"), Some(vec![".pdf"]));
        assert_eq!(ext_hint_from_query("Word"), Some(vec![".doc", ".docx"]));
        assert_eq!(ext_hint_from_query("EXCEL"), Some(vec![".xls", ".xlsx"]));
        assert_eq!(ext_hint_from_query("WORD"), Some(vec![".doc", ".docx"]));
    }

    // ---- is_allowed_dir ----

    #[test]
    fn is_allowed_dir_accepts_roots_and_children() {
        let root = tmp_dir("allowed");
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let roots = vec![root_str(&root)];
        assert!(is_allowed_dir(&root_str(&root), &roots));
        assert!(is_allowed_dir(&root_str(&sub), &roots), "根内子目录应允许");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_allowed_dir_rejects_outside_relative_and_missing() {
        let root = tmp_dir("reject_root");
        let other = tmp_dir("reject_other");
        let roots = vec![root_str(&root)];
        assert!(!is_allowed_dir(&root_str(&other), &roots));
        // 前缀仿冒(root 同名加后缀)拒绝
        let mimic = tmp_dir("reject_root_x2");
        assert!(!is_allowed_dir(&root_str(&mimic), &roots));
        // 相对路径/空串拒绝
        assert!(!is_allowed_dir("relative/path", &roots));
        assert!(!is_allowed_dir("", &roots));
        assert!(!is_allowed_dir(r"..\..\x", &roots));
        // 不存在的路径拒绝(无法规范化)
        assert!(!is_allowed_dir(&root.join("不存在.txt").to_string_lossy(), &roots));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&other);
        let _ = std::fs::remove_dir_all(&mimic);
    }

    #[test]
    fn is_allowed_dir_empty_roots_allows_only_desktop() {
        let root = tmp_dir("empty_roots");
        let roots: Vec<String> = Vec::new();
        assert!(!is_allowed_dir(&root_str(&root), &roots), "空 roots 时临时目录应拒绝");
        if let Some(d) = desktop_dir() {
            assert!(is_allowed_dir(&d.to_string_lossy(), &roots), "空 roots 时桌面应允许");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_allowed_dir_desktop_always_allowed() {
        let root = tmp_dir("desk_always");
        let roots = vec![root_str(&root)];
        if let Some(d) = desktop_dir() {
            assert!(is_allowed_dir(&d.to_string_lossy(), &roots), "桌面始终在允许集合内");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- 临时目录树扫描 ----

    #[test]
    fn search_tree_hits_with_depth3_cutoff_and_case_insensitive() {
        let root = tmp_dir("tree");
        let roots = vec![root_str(&root)];
        // 第 1 层:多扩展名 + 大小写混合 + 名字带空格的扩展名叠加测试
        touch(&root.join("发票.PDF"));
        touch(&root.join("发票.docx"));
        touch(&root.join("invoice pdf.PDF"));
        touch(&root.join("invoice pdf.docx"));
        touch(&root.join("照片.JPG"));
        // 第 2 层
        touch(&root.join("d1").join("发票.pdf"));
        // 第 3 层(根下第 3 级,可达)
        touch(&root.join("d1").join("d2").join("发票.pdf"));
        // 第 4 层(根下第 4 级:深度边界,不可达)
        touch(&root.join("d1").join("d2").join("d3").join("发票.pdf"));
        // 第 5 层(更深处同样不可达)
        touch(&root.join("d1").join("d2").join("d3").join("d4").join("发票.pdf"));

        // 纯文件名子串:第 1~3 层 4 个 发票* 命中,第 4 层起不命中
        let hits = search_in_roots("发票", &[], &roots);
        assert_eq!(hits.len(), 4, "深度 3 内应命中 4 个(第 4 层截断)");
        assert!(hits.iter().all(|h| !h.full_path.contains("d3")), "第 4 层文件不应命中");
        let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
        for expect in ["发票.PDF", "发票.docx", "发票.pdf"] {
            assert!(names.contains(&expect), "缺少命中 {expect}: {names:?}");
        }

        // 大小写不敏感:大写文件名 + 大写查询
        let hits = search_in_roots("照片", &[], &roots);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "照片.JPG");

        // 扩展名叠加:query 含 pdf 词 → 同名但 docx 的被滤掉,只留 .pdf/.PDF
        let hits = search_in_roots("invoice pdf", &[], &roots);
        assert_eq!(hits.len(), 1, "扩展名叠加后应只剩 pdf 命中");
        assert_eq!(hits[0].name, "invoice pdf.PDF");
        // query 原样子串:不带 pdf 词时 docx 也能命中
        let hits = search_in_roots("invoice", &[], &roots);
        assert_eq!(hits.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- 越权 dirs 忽略 ----

    #[test]
    fn search_ignores_unauthorized_dirs_without_panic() {
        let root = tmp_dir("unauth_root");
        let outside = tmp_dir("unauth_outside");
        touch(&root.join("aurora目标.pdf"));
        touch(&outside.join("aurora目标.pdf"));
        let roots = vec![root_str(&root)];

        // 全部越权 → 空结果,不 panic
        let hits = search_in_roots("aurora目标", &[root_str(&outside)], &roots);
        assert!(hits.is_empty());

        // 混入合法 + 越权 → 只搜合法的
        let hits = search_in_roots(
            "aurora目标",
            &[root_str(&outside), root_str(&root)],
            &roots,
        );
        assert_eq!(hits.len(), 1);

        // 相对路径 dirs 被忽略
        let hits = search_in_roots("aurora目标", &[r"relative\dir".to_string()], &roots);
        assert!(hits.is_empty());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    // ---- 巨目录保护 ----

    #[test]
    fn search_truncates_huge_dir_at_per_level_cap() {
        let root = tmp_dir("huge");
        let roots = vec![root_str(&root)];
        // 600 个文件:排序后前 500 个(file0000..file0499)参与匹配,后 100 个被每层截断
        for i in 0..600 {
            touch(&root.join(format!("file{i:04}.txt")));
        }
        // 被每层 500 截断的 file05xx 搜不到
        let hits = search_in_roots("file05", &[], &roots);
        assert!(hits.is_empty(), "每层截断后的 file05xx 不应命中");
        // 全局结果上限 20
        let hits = search_in_roots("file0", &[], &roots);
        assert_eq!(hits.len(), MAX_RESULTS);
        assert_eq!(hits[0].name, "file0000.txt");
        assert!(hits.iter().all(|h| h.name.starts_with("file00")));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- 去重与错误跳过 ----

    #[test]
    fn search_dedups_across_duplicate_dirs() {
        let root = tmp_dir("dedup");
        touch(&root.join("aurora去重.pdf"));
        let roots = vec![root_str(&root)];
        // dirs 传同一个目录两次 → 同一文件被扫两遍 → 全局去重
        let dirs = vec![root_str(&root), root_str(&root)];
        let hits = search_in_roots("aurora去重", &dirs, &roots);
        assert_eq!(hits.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_skips_missing_roots_without_panic() {
        let root = tmp_dir("missing_root");
        let missing = tmp_dir("missing_dir");
        std::fs::remove_dir_all(&missing).unwrap();
        let roots = vec![root_str(&root), root_str(&missing)];
        touch(&root.join("aurora存在.pdf"));
        // 缺失/不可读目录被跳过,不中断整体
        let hits = search_in_roots("aurora存在", &[], &roots);
        assert_eq!(hits.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- 序列化契约 ----

    #[test]
    fn file_hit_serializes_with_contract_fields() {
        let h = FileHit {
            name: "a.pdf".to_string(),
            full_path: r"C:\x\a.pdf".to_string(),
            is_dir: false,
        };
        let s = serde_json::to_string(&h).unwrap();
        // JSON 中反斜杠须转义为双反斜杠
        assert_eq!(s, r#"{"name":"a.pdf","full_path":"C:\\x\\a.pdf","is_dir":false}"#);
    }
}
