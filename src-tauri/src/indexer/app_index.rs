use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppEntry {
    pub name: String,
    pub path: String,
}

pub struct AppIndex {
    pub entries: Vec<AppEntry>,
}

impl AppIndex {
    pub fn empty() -> AppIndex {
        AppIndex {
            entries: Vec::new(),
        }
    }

    pub fn from_dir(dir: &Path) -> AppIndex {
        let mut entries = Vec::new();
        collect_lnks(dir, &mut entries);
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        AppIndex { entries }
    }

    pub fn sort(&mut self) {
        self.entries
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }

    /// 大小写不敏感子串匹配,按名称排序(构造时已排序),取前 20 条
    pub fn search(&self, query: &str) -> Vec<AppEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&q))
            .take(20)
            .cloned()
            .collect()
    }
}

/// 递归收集深度上限(安全加固):开始菜单目录内出现符号链接环
/// (如链接指回父目录)时,无上限的递归会栈溢出崩溃;8 层对正常开始菜单布局绰绰有余
const MAX_COLLECT_DEPTH: usize = 8;

/// 递归收集目录下的 .lnk 文件;目录不可读时静默跳过(不 panic)。
/// 安全加固:深度 ≤ [MAX_COLLECT_DEPTH],符号链接环在深度上限处被截断,防栈溢出
fn collect_lnks(dir: &Path, out: &mut Vec<AppEntry>) {
    collect_lnks_inner(dir, 0, out);
}

/// 递归收集实现(depth 从 0 起,根目录为第 0 层)
fn collect_lnks_inner(dir: &Path, depth: usize, out: &mut Vec<AppEntry>) {
    if depth >= MAX_COLLECT_DEPTH {
        return; // 深度上限:防符号链接环导致的无限递归/栈溢出
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lnks_inner(&path, depth + 1, out);
        } else if path.extension().map(|e| e == "lnk").unwrap_or(false) {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(AppEntry {
                name,
                path: path.to_string_lossy().to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_lnk(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(format!("{name}.lnk"));
        fs::write(&p, b"fake lnk content").unwrap();
        p
    }

    #[test]
    fn scans_nested_lnk_files() {
        let base = std::env::temp_dir().join("aurora_idx_test_scan");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("sub")).unwrap();
        make_lnk(&base, "记事本");
        make_lnk(&base.join("sub"), "计算器");
        let idx = AppIndex::from_dir(&base);
        assert_eq!(idx.entries.len(), 2);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn ignores_non_lnk_files() {
        let base = std::env::temp_dir().join("aurora_idx_test_ignore");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("readme.txt"), "x").unwrap();
        let idx = AppIndex::from_dir(&base);
        assert_eq!(idx.entries.len(), 0);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn search_is_case_insensitive_and_sorted() {
        let base = std::env::temp_dir().join("aurora_idx_test_search");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        make_lnk(&base, "Zoom");
        make_lnk(&base, "记事本");
        make_lnk(&base, "zoom-camera");
        let idx = AppIndex::from_dir(&base);
        let r = idx.search("zoom");
        assert_eq!(r.len(), 2);
        // 大小写不敏感排序:"zoom"(Zoom) < "zoom-camera"(字典序前缀在前)
        assert_eq!(r[0].name, "Zoom");
        assert_eq!(r[1].name, "zoom-camera");
        assert!(idx.search("不存在的应用").is_empty());
        assert!(idx.search("").is_empty());
        assert!(idx.search("  ").is_empty());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn search_limits_to_top_20() {
        let base = std::env::temp_dir().join("aurora_idx_test_top20");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        for i in 0..25 {
            make_lnk(&base, &format!("app-{i:02}"));
        }
        let idx = AppIndex::from_dir(&base);
        let r = idx.search("app");
        assert_eq!(r.len(), 20);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn collect_lnks_stops_at_depth_limit() {
        // 安全加固:根为深度 0,最多收集到深度 7;深度 ≥ MAX_COLLECT_DEPTH 的层级
        // 不再进入(防符号链接环无限递归/栈溢出)
        let base = std::env::temp_dir().join("aurora_idx_test_depth");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // 第 0~7 层各放一个 lnk(可收集)
        let mut deep = base.clone();
        for i in 0..MAX_COLLECT_DEPTH {
            make_lnk(&deep, &format!("layer-{i}"));
            deep = deep.join("d");
            fs::create_dir_all(&deep).unwrap();
        }
        // 第 8 层(深度上限外)不放收集断言目标
        make_lnk(&deep, "too-deep");
        let idx = AppIndex::from_dir(&base);
        let names: Vec<&str> = idx.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), MAX_COLLECT_DEPTH, "第 0~7 层共 8 个,第 8 层不再收集");
        for i in 0..MAX_COLLECT_DEPTH {
            let layer = format!("layer-{i}");
            assert!(names.contains(&layer.as_str()), "缺少 {layer}");
        }
        assert!(!names.contains(&"too-deep"), "深度上限外不得收集");
        let _ = fs::remove_dir_all(&base);
    }
}
