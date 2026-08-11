pub mod app_index;

use app_index::AppIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// 用户开始菜单 + 公共开始菜单(禁止全盘扫描,仅此两目录)
pub fn default_start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(p) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(p).join("Microsoft/Windows/Start Menu/Programs"));
    }
    let program_data = PathBuf::from("C:/ProgramData");
    dirs.push(program_data.join("Microsoft/Windows/Start Menu/Programs"));
    dirs
}

/// 进程内索引缓存:目录 mtime 未变化时不重扫(轻量增量,不做磁盘缓存)
type DirMtimes = HashMap<PathBuf, SystemTime>;
static INDEX_CACHE: OnceLock<Mutex<(DirMtimes, Vec<app_index::AppEntry>)>> = OnceLock::new();

/// 构建开始菜单索引(默认两目录,mtime 增量)
pub fn build_index() -> AppIndex {
    let dirs = default_start_menu_dirs();
    build_index_from(&dirs)
}

/// 可注入目录的主逻辑(便于单测)。目录 mtime 与上次一致时复用缓存结果。
pub fn build_index_from(dirs: &[PathBuf]) -> AppIndex {
    let cache = INDEX_CACHE.get_or_init(|| Mutex::new((HashMap::new(), Vec::new())));
    let mut guard = cache.lock().unwrap();
    let changed = dirs.iter().any(|d| match dir_mtime(d) {
        Some(t) => guard.0.get(d).copied() != Some(t),
        None => true,
    });
    if changed {
        let mut entries = Vec::new();
        for d in dirs {
            if let Some(t) = dir_mtime(d) {
                guard.0.insert(d.clone(), t);
            }
            let sub = AppIndex::from_dir(d);
            entries.extend(sub.entries);
        }
        let mut idx = AppIndex::empty();
        idx.entries = entries;
        idx.sort();
        guard.1 = idx.entries;
    }
    let mut idx = AppIndex::empty();
    idx.entries = guard.1.clone();
    idx
}

fn dir_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_rescan_picks_new_lnk() {
        let base = std::env::temp_dir().join("aurora_idx_test_incr");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // 首次扫描:1 个 lnk
        std::fs::write(base.join("a.lnk"), b"x").unwrap();
        let idx1 = build_index_from(&[base.clone()]);
        assert_eq!(idx1.entries.len(), 1);
        // 目录 mtime 变化后重扫:新增 lnk 被索引(Windows 目录 mtime 更新有微小延迟,留缓冲)
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(base.join("b.lnk"), b"x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let idx2 = build_index_from(&[base.clone()]);
        assert_eq!(idx2.entries.len(), 2);
        // mtime 未变化:结果保持一致
        let idx3 = build_index_from(&[base.clone()]);
        assert_eq!(idx3.entries.len(), 2);
        let _ = std::fs::remove_dir_all(&base);
    }
}
