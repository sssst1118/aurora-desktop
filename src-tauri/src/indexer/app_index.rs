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

/// 递归收集目录下的 .lnk 文件;目录不可读时静默跳过(不 panic)
fn collect_lnks(dir: &Path, out: &mut Vec<AppEntry>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lnks(&path, out);
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
}
