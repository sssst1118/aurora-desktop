use pinyin::ToPinyinMulti;
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppEntry {
    pub name: String,
    pub path: String,
    /// 预计算的拼音表示(搜索质量包 2026-08-13):索引构建时一次性算好,
    /// 查询时只做字符串前缀比较,零逐键转换成本;serde skip 保证前端
    /// 契约不变(线上仍只有 name/path 两个字段)
    #[serde(skip)]
    pub pinyin: PinyinRep,
}

/// 应用名的拼音表示(索引构建时预计算,查询时直接比对)
#[derive(Clone, Debug, PartialEq)]
pub struct PinyinRep {
    /// 全拼串:逐字取第一读音的无调拼音拼接;非汉字字符(拉丁/数字/符号)按小写原样保留。
    /// 例:微信 → "weixin"、360安全卫士 → "360anquanweishi"
    pub full: String,
    /// 首字母串:逐字取第一读音拼音的首字母;非汉字按小写原样保留。
    /// 例:微信 → "wx"、360安全卫士 → "360aqws"
    pub initials: String,
    /// 多音字备选读音的全拼组合(混合进制枚举,上限 [MAX_ALT]);无多音字时为空。
    /// 例:重庆 → ["chongqing"](主读音为 "zhongqing")、银行 → ["yinhang"]
    pub alt_full: Vec<String>,
    /// 多音字备选读音的首字母组合(同上)。
    /// 例:重庆 → ["cq"](主首字母为 "zq")
    pub alt_initials: Vec<String>,
}

impl PinyinRep {
    /// 多音字备选读音组合数上限。组合数 = 各多音字读音数的乘积,只有名字里
    /// 连续出现多个多音字的罕见场景才会触顶;8 种备选已覆盖常见应用名。
    pub const MAX_ALT: usize = 8;

    /// 从应用名构建拼音表示(索引构建时调用一次,查询不再转换)
    pub fn from_name(name: &str) -> PinyinRep {
        // 逐字读音表:无拼音数据的字符(拉丁/数字/符号)自身就是唯一"读音"
        let syls: Vec<Vec<String>> = name
            .chars()
            .map(|c| match c.to_pinyin_multi() {
                Some(m) => m.into_iter().map(|p| p.plain().to_string()).collect(),
                None => vec![c.to_lowercase().to_string()],
            })
            .collect();
        // 主读音:每字第一读音 → 全拼串 + 首字母串
        let mut full = String::new();
        let mut initials = String::new();
        for s in &syls {
            full.push_str(&s[0]);
            // 拼音音节的第一个字母即声母(非汉字的"读音"就是原字符,取它自身)
            initials.push(s[0].chars().next().unwrap_or(' '));
        }
        // 多音字备选:混合进制枚举全部读音组合(组合 0 = 全是第一读音,即主串,跳过);
        // total 每步 min 截断防溢出,也保证最多枚举 MAX_ALT 个备选
        let total: usize = syls
            .iter()
            .fold(1usize, |acc, v| acc.saturating_mul(v.len()).min(Self::MAX_ALT + 1));
        let mut alt_full = Vec::new();
        let mut alt_initials = Vec::new();
        for k in 1..total {
            let mut rem = k;
            let mut f = String::new();
            let mut i = String::new();
            for v in &syls {
                let idx = rem % v.len();
                rem /= v.len();
                f.push_str(&v[idx]);
                i.push(v[idx].chars().next().unwrap_or(' '));
            }
            alt_full.push(f);
            alt_initials.push(i);
        }
        PinyinRep {
            full,
            initials,
            alt_full,
            alt_initials,
        }
    }
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

    /// 拼音增强搜索(搜索质量包 2026-08-13):
    /// 三层匹配,数字越小优先级越高——名称直接子串命中 > 拼音全拼前缀命中
    /// > 拼音首字母命中(如 "jsq" = 计(j)算(s)器(q));层内"名称与查询完全相同"
    /// 优先(查询"计算器"时「计算器」排在「计算器Pro」前),其余保持构造时的
    /// 名称字典序;最多取前 20 条。拼音比对走预计算值,逐键零转换成本。
    pub fn search(&self, query: &str) -> Vec<AppEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(&AppEntry, u8, bool)> = Vec::new();
        for e in &self.entries {
            if let Some(tier) = match_tier(e, &q) {
                hits.push((e, tier, e.name.eq_ignore_ascii_case(&q)));
            }
        }
        // 稳定排序:entries 构造时已按名称字典序排好,这里只按(匹配层级升序,
        // 完全命中优先)重排,同层同命中组的原序即名称序,无需再比较名称
        hits.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
        hits.truncate(20);
        hits.into_iter().map(|(e, _, _)| e.clone()).collect()
    }
}

/// 匹配层级(数字越小越优先,见 [AppIndex::search] 的排序说明)
const TIER_SUBSTR: u8 = 0; // 名称直接子串命中
const TIER_FULL: u8 = 1; // 拼音全拼前缀命中
const TIER_INITIALS: u8 = 2; // 拼音首字母命中

/// 取某条目的最高匹配层级;三层都不中返回 None。
/// q 已由调用方 trim + 小写化;预计算的拼音串本身全小写,两侧可直接前缀比较。
fn match_tier(e: &AppEntry, q: &str) -> Option<u8> {
    if e.name.to_lowercase().contains(q) {
        return Some(TIER_SUBSTR);
    }
    if e.pinyin.full.starts_with(q) || e.pinyin.alt_full.iter().any(|s| s.starts_with(q)) {
        return Some(TIER_FULL);
    }
    if e.pinyin.initials.starts_with(q) || e.pinyin.alt_initials.iter().any(|s| s.starts_with(q)) {
        return Some(TIER_INITIALS);
    }
    None
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
                pinyin: PinyinRep::from_name(&name),
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

    // ---- 拼音匹配(搜索质量包 2026-08-13)----

    /// 手工构造条目(拼音表示与真实收集路径一致,均走 PinyinRep::from_name)
    fn ent(name: &str) -> AppEntry {
        AppEntry {
            name: name.to_string(),
            path: format!("C:\\fake\\{name}.lnk"),
            pinyin: PinyinRep::from_name(name),
        }
    }

    fn idx_of(names: &[&str]) -> AppIndex {
        let mut idx = AppIndex::empty();
        idx.entries = names.iter().map(|n| ent(n)).collect();
        idx
    }

    fn hit_names(idx: &AppIndex, q: &str) -> Vec<String> {
        idx.search(q).iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn pinyin_full_prefix_matches() {
        // 全拼前缀:"weixin" 命中「微信」;"weix" 前缀也命中;"weixinn" 超长不命中
        let idx = idx_of(&["微信", "记事本"]);
        assert_eq!(hit_names(&idx, "weixin"), vec!["微信"]);
        assert_eq!(hit_names(&idx, "weix"), vec!["微信"]);
        assert!(idx.search("weixinn").is_empty());
    }

    #[test]
    fn pinyin_initials_match() {
        // 首字母:"jsq" = 计(j)算(s)器(q),命中「计算器」
        let idx = idx_of(&["计算器", "微信"]);
        assert_eq!(hit_names(&idx, "jsq"), vec!["计算器"]);
        // "wx" = 微信 首字母
        assert_eq!(hit_names(&idx, "wx"), vec!["微信"]);
        assert!(idx.search("jsqx").is_empty());
    }

    #[test]
    fn substring_ranks_before_pinyin() {
        // 查询 "ji":「Jira」名称子串命中(层 0),「记事本」拼音前缀 jishiben 命中(层 1),
        // 子串必须排前
        let idx = idx_of(&["记事本", "Jira"]);
        assert_eq!(hit_names(&idx, "ji"), vec!["Jira", "记事本"]);
    }

    #[test]
    fn full_pinyin_ranks_before_initials() {
        // 查询 "we":「微信」全拼 weixin 前缀(层 1),「五二」首字母 w-e(层 2,
        // 全拼 wuer 不以 "we" 开头),全拼必须排前
        let idx = idx_of(&["五二", "微信"]);
        assert_eq!(hit_names(&idx, "we"), vec!["微信", "五二"]);
    }

    #[test]
    fn exact_name_hit_ranks_first() {
        // 同层(都是子串命中)时完全命中优先:「计算器」排在「计算器Pro」前
        let idx = idx_of(&["计算器Pro", "计算器"]);
        assert_eq!(hit_names(&idx, "计算器"), vec!["计算器", "计算器Pro"]);
    }

    #[test]
    fn polyphonic_alt_pinyin_matches() {
        // 多音字备选读音:「重庆」主读音 zhongqing,备选 chongqing,两种查询都命中
        let idx = idx_of(&["重庆"]);
        assert_eq!(hit_names(&idx, "zhongqing"), vec!["重庆"]);
        assert_eq!(hit_names(&idx, "chongqing"), vec!["重庆"]);
        assert_eq!(hit_names(&idx, "cq"), vec!["重庆"]);
    }

    #[test]
    fn ascii_names_unchanged() {
        // 纯英文名不退化:拼音表示 = 名字小写本身,行为与原子串匹配一致
        let idx = idx_of(&["Zoom", "记事本", "zoom-camera"]);
        assert_eq!(hit_names(&idx, "zoom"), vec!["Zoom", "zoom-camera"]);
    }

    #[test]
    fn mixed_name_pinyin_and_initials() {
        // 中英混合名:非汉字字符按小写原样进全拼串/首字母串
        let idx = idx_of(&["360安全卫士"]);
        assert_eq!(hit_names(&idx, "360anquan"), vec!["360安全卫士"]);
        assert_eq!(hit_names(&idx, "360aq"), vec!["360安全卫士"]);
    }
}
