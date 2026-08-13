pub mod app_index;

use app_index::AppIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

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

/// 目录 mtime 与上次构建一致时复用缓存条目(轻量增量,不做磁盘缓存)
type DirMtimes = HashMap<PathBuf, SystemTime>;

/// 搜索前增量重扫的时间阈值(并发修复 2026-08-13):
/// 索引原只在启动时构建一次,新安装应用不重启永远搜不到。search_apps 每次按键都会
/// 调用 [refresh_if_stale],超过本阈值才做一次 mtime 增量检查(目录未变零成本复用),
/// 避免每次搜索都全量重扫开始菜单。
pub const RESCAN_INTERVAL: Duration = Duration::from_secs(60);

/// 进程内索引缓存:mtime 表 + 条目快照 + 最近一次检查时间(built_at 语义 =
/// "最近一次 mtime 检查",不是"最近一次重建":目录没变时检查本身同样是新鲜的)
struct CacheState {
    mt: DirMtimes,
    entries: Vec<app_index::AppEntry>,
    built_at: Instant,
}

impl Default for CacheState {
    fn default() -> Self {
        Self {
            mt: HashMap::new(),
            entries: Vec::new(),
            built_at: Instant::now(),
        }
    }
}

static INDEX_CACHE: OnceLock<Mutex<CacheState>> = OnceLock::new();

/// 构建开始菜单索引(默认两目录,mtime 增量)
pub fn build_index() -> AppIndex {
    let dirs = default_start_menu_dirs();
    build_index_from(&dirs)
}

/// 可注入目录的主逻辑(便于单测)。目录 mtime 与上次一致时复用缓存结果。
pub fn build_index_from(dirs: &[PathBuf]) -> AppIndex {
    let cache = INDEX_CACHE.get_or_init(|| Mutex::new(CacheState::default()));
    let mut guard = cache.lock().unwrap();
    let changed = dirs.iter().any(|d| match dir_mtime(d) {
        Some(t) => guard.mt.get(d).copied() != Some(t),
        None => true,
    });
    if changed {
        let mut entries = Vec::new();
        for d in dirs {
            if let Some(t) = dir_mtime(d) {
                guard.mt.insert(d.clone(), t);
            }
            let sub = AppIndex::from_dir(d);
            entries.extend(sub.entries);
        }
        let mut idx = AppIndex::empty();
        idx.entries = entries;
        idx.sort();
        guard.entries = idx.entries;
    }
    // 无论是否重扫,本次检查都是新鲜的:重置计时,阈值内不再重复 stat 目录
    guard.built_at = Instant::now();
    let mut idx = AppIndex::empty();
    idx.entries = guard.entries.clone();
    idx
}

/// search_apps 调用的增量重扫钩子:距上次构建检查超过 [RESCAN_INTERVAL] 时
/// 做一次 mtime 增量检查(目录未变化零成本复用缓存),返回最新条目供调用方
/// 合入 managed state;未到阈值返回 None(调用方继续用旧快照)。
/// 新装应用无需重启,最多 60s 内即可被搜到。
pub fn refresh_if_stale() -> Option<Vec<app_index::AppEntry>> {
    let stale = {
        let cache = INDEX_CACHE.get_or_init(|| Mutex::new(CacheState::default()));
        cache.lock().unwrap().built_at.elapsed() >= RESCAN_INTERVAL
    };
    if stale {
        Some(build_index_from(&default_start_menu_dirs()).entries)
    } else {
        None
    }
}

fn dir_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全局索引缓存(INDEX_CACHE)是本模块所有用例共享的进程级静态,测试默认并行
    /// 跑会互相干扰(条目/时间戳互相覆盖,refresh_if_stale 的断言会 flaky),
    /// 用一把测试锁把本模块用例串行化。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn incremental_rescan_picks_new_lnk() {
        let _g = TEST_LOCK.lock().unwrap();
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

    // ---- 搜索前增量重扫钩子(并发修复 2026-08-13,refresh_if_stale)----

    #[test]
    fn refresh_if_stale_returns_none_when_fresh() {
        let _g = TEST_LOCK.lock().unwrap();
        // 刚构建完(built_at 刚重置)→ 阈值内不重复检查,返回 None
        build_index_from(&default_start_menu_dirs());
        assert!(refresh_if_stale().is_none());
    }

    #[test]
    fn refresh_if_stale_rescans_after_interval_and_resets_timer() {
        let _g = TEST_LOCK.lock().unwrap();
        // 改写缓存时间戳模拟"距上次检查已超过阈值"(测试直接访问私有字段,
        // 不必真等 60s;build_index_from 保证缓存已初始化)
        build_index_from(&default_start_menu_dirs());
        {
            let cache = INDEX_CACHE.get().unwrap();
            cache.lock().unwrap().built_at = Instant::now() - RESCAN_INTERVAL - Duration::from_secs(1);
        }
        // 超过阈值 → 触发一次增量检查,返回最新条目
        assert!(refresh_if_stale().is_some());
        // 检查后计时已重置 → 紧接着再调返回 None,证明"60s 内不重复重扫"
        assert!(refresh_if_stale().is_none());
    }
}
