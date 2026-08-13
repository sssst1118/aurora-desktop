//! 稳定性包(2026-08-13):崩溃日志 + 关键事件日志。
//!
//! - 崩溃日志:[`init`] 注册 `std::panic::set_hook`,panic 时写
//!   `%APPDATA%\com.aurora.desktop\logs\panic-<时间戳>.log`(panic 信息 + 调用栈,
//!   `Backtrace::force_capture`),同目录保留最近 [`PANIC_LOG_KEEP`] 个 panic 文件,
//!   更旧的自动清理;
//! - 事件日志:[`log_event`] 追加写同目录 `aurora.log`(启动/退出/更新安装等
//!   关键事件),单文件上限 [`EVENT_LOG_MAX_BYTES`](2MB),超限改名 `aurora.log.old`
//!   覆盖旧备份后重新开始。
//!
//! 日志目录与 config.json 同级(`%APPDATA%\com.aurora.desktop`,与
//! commands/config.rs 的 `app_config_dir` 一致);panic hook 拿不到 AppHandle,
//! 这里直接按环境变量拼路径(APPDATA 缺失时回退临时目录,与 config_path 兜底同风格)。
//!
//! 铁律:日志系统自身绝不能崩应用——所有文件 IO 失败一律静默忽略(hook 内 panic
//! 会直接 abort 进程);全部写路径经全局互斥锁串行,防多线程交错写坏日志文件。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 事件日志单文件上限:2MB(超限滚动:改名 .old 覆盖旧备份)
pub const EVENT_LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// panic 日志保留数量(超出的旧文件按时间戳清理)
pub const PANIC_LOG_KEEP: usize = 5;

/// 事件日志文件名
const EVENT_LOG_NAME: &str = "aurora.log";
/// 滚动后的旧事件日志文件名
const EVENT_LOG_OLD_NAME: &str = "aurora.log.old";

/// 日志目录:%APPDATA%\com.aurora.desktop\logs(与 config.json 同级的 logs 子目录)
pub fn logs_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("com.aurora.desktop")
        .join("logs")
}

/// 日志写路径全局互斥(panic hook 与 log_event 共用,防交错写)
fn log_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

// ---------------------------------------------------------------------------
// 时间戳(纯函数,便于单测)
// ---------------------------------------------------------------------------

/// 事件日志时间戳文本(纯函数):`2026-08-13 14:22:33`(月/日/时/分/秒两位补零)
pub fn format_timestamp(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> String {
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// panic 文件名时间戳(纯函数):`20260813-142233-045`(年-月-日-时-分-秒-毫秒)。
/// 固定宽度保证按文件名字典序排序即时间序(清理旧文件依赖此性质)
pub fn format_file_timestamp(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ms: u32) -> String {
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}-{ms:03}")
}

/// 读取本地时间(Windows:GetLocalTime,经已启用的 Win32_System_SystemInformation
/// feature,零新依赖;非 Windows:退化按 UTC 计算——本项目仅 Windows 出包,
/// 该分支只为编译可达性,不追求时区正确)。
/// 返回 (年, 月, 日, 时, 分, 秒, 毫秒);读取失败 None
#[cfg(windows)]
fn now_local() -> Option<(i32, u32, u32, u32, u32, u32, u32)> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    Some((
        st.wYear as i32,
        st.wMonth as u32,
        st.wDay as u32,
        st.wHour as u32,
        st.wMinute as u32,
        st.wSecond as u32,
        st.wMilliseconds as u32,
    ))
}

#[cfg(not(windows))]
fn now_local() -> Option<(i32, u32, u32, u32, u32, u32, u32)> {
    // UTC 秒数 → 公历日期(Howard Hinnant 的 days_from_civil 逆算法;纯数学无依赖)
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    if mo <= 2 {
        y += 1;
    }
    Some((y as i32, mo as u32, d as u32, h, mi, s, 0))
}

/// 事件日志行(纯函数):`[2026-08-13 14:22:33] [INFO] 消息\n`
pub fn format_log_line(ts: &str, level: &str, msg: &str) -> String {
    format!("[{ts}] [{level}] {msg}\n")
}

/// 滚动判定(纯函数):体积达到 2MB 即滚动(现文件改名 .old,重新开始写新文件)
pub fn should_rollover(size: u64) -> bool {
    size >= EVENT_LOG_MAX_BYTES
}

/// 从 panic 文件名集合中选出应清理的旧文件(纯函数,可单测):
/// 文件名按字典序(时间戳固定宽度 ⇒ 字典序 = 时间序)升序,保留最新 keep 个,
/// 其余返回待删列表。调用方传入的须是已按 `panic-*.log` 前缀过滤的名字。
pub fn panic_files_to_prune(files: &[String], keep: usize) -> Vec<String> {
    let mut sorted = files.to_vec();
    sorted.sort();
    if sorted.len() <= keep {
        Vec::new()
    } else {
        sorted.truncate(sorted.len() - keep);
        sorted
    }
}

// ---------------------------------------------------------------------------
// 写入(全部失败静默,绝不 panic)
// ---------------------------------------------------------------------------

/// 追加一行事件到日志目录(滚动已处理;纯 IO,可单测直接传临时目录)。
/// 任何失败静默返回 false——日志系统绝不能影响应用运行。
fn append_event(dir: &Path, line: &str) -> bool {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(EVENT_LOG_NAME);
    // 滚动:超限 → 删旧 .old → 现文件改名 .old → 追加到全新文件。
    // 改名失败则继续追加(不丢本次事件,文件只多写超限的部分)
    if path.exists() {
        let oversized = std::fs::metadata(&path)
            .map(|m| should_rollover(m.len()))
            .unwrap_or(false);
        if oversized {
            let old = dir.join(EVENT_LOG_OLD_NAME);
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&path, &old);
        }
    }
    use std::io::Write;
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => f.write_all(line.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// 关键事件日志(启动/退出/更新安装等):追加一行到 aurora.log。
/// 失败静默——日志系统绝不能崩应用。
pub fn log_event(level: &str, msg: &str) {
    let Some((y, mo, d, h, mi, s, _)) = now_local() else {
        return;
    };
    let line = format_log_line(&format_timestamp(y, mo, d, h, mi, s), level, msg);
    let dir = logs_dir();
    let _guard = log_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _ = append_event(&dir, &line);
}

/// 安装 panic hook(必须在 `run()` 最前面调用)。
/// 保留默认行为(打印到 stderr),额外把 panic 信息 + 调用栈写 `panic-<时间戳>.log`,
/// 同目录保留最近 [`PANIC_LOG_KEEP`] 个 panic 文件。
/// hook 内所有 IO 失败一律忽略:hook 自身 panic 会直接 abort 进程,这是铁律。
pub fn init() {
    std::panic::set_hook(Box::new(|info| {
        // 控制台可见性保留(与原默认 hook 行为一致)
        eprintln!("[aurora] 程序发生严重错误(panic): {info}");
        write_panic_log(info);
    }));
}

/// 组装 panic 日志文本(纯函数,可单测):时间/线程/位置/信息/调用栈
fn build_panic_text(
    ts: &str,
    thread_name: &str,
    location: &str,
    payload: &str,
    backtrace: &str,
) -> String {
    format!(
        "时间: {ts}\n线程: {thread_name}\n位置: {location}\n信息: {payload}\n\n调用栈:\n{backtrace}\n"
    )
}

/// 写单个 panic 日志文件(不 panic;失败静默)
fn write_panic_log(info: &std::panic::PanicHookInfo<'_>) {
    let Some((y, mo, d, h, mi, s, ms)) = now_local() else {
        return;
    };
    let name = format!("panic-{}.log", format_file_timestamp(y, mo, d, h, mi, s, ms));
    let dir = logs_dir();
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let location = info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()))
        .unwrap_or_else(|| "<unknown>".to_string());
    let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<非字符串 payload>".to_string()
    };
    let text = build_panic_text(
        &format_timestamp(y, mo, d, h, mi, s),
        thread_name,
        &location,
        &payload,
        &std::backtrace::Backtrace::force_capture().to_string(),
    );
    // 用 try_lock 而非 lock:hook 运行在 panic 展开路径上,阻塞等待锁会把
    // 崩溃线程挂死(进程退出卡住);拿不到锁也照写,崩溃场景下日志可读性
    // 优先于互斥。锁内无 panic 代码,正常路径不可能走到这里
    let _guard = log_lock().try_lock();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(&name), text);
    let _ = prune_panic_logs(&dir);
}

/// 清理超出保留数量的旧 panic 日志(按文件名时间戳排序,删最旧的;可单测传临时目录)
fn prune_panic_logs(dir: &Path) -> std::io::Result<()> {
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("panic-") && name.ends_with(".log") {
            names.push(name);
        }
    }
    for old in panic_files_to_prune(&names, PANIC_LOG_KEEP) {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独立临时目录(按 pid + 时间戳区分,防并行测试串扰)
    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("aurora_log_{tag}_{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    // ---- 日志行格式(纯函数) ----

    #[test]
    fn log_line_format() {
        assert_eq!(
            format_log_line("2026-08-13 14:22:33", "INFO", "应用启动"),
            "[2026-08-13 14:22:33] [INFO] 应用启动\n"
        );
        assert_eq!(
            format_log_line("2026-08-13 14:22:33", "ERROR", "写入失败"),
            "[2026-08-13 14:22:33] [ERROR] 写入失败\n"
        );
    }

    #[test]
    fn timestamp_zero_padded() {
        assert_eq!(format_timestamp(2026, 8, 13, 9, 5, 3), "2026-08-13 09:05:03");
        assert_eq!(format_timestamp(2026, 12, 31, 23, 59, 59), "2026-12-31 23:59:59");
    }

    #[test]
    fn file_timestamp_fixed_width_sortable() {
        // 固定宽度:字典序 = 时间序(清理旧文件依赖)
        assert_eq!(
            format_file_timestamp(2026, 8, 13, 14, 22, 33, 45),
            "20260813-142233-045"
        );
        assert_eq!(
            format_file_timestamp(2026, 1, 2, 3, 4, 5, 6),
            "20260102-030405-006"
        );
        let a = format_file_timestamp(2026, 8, 13, 14, 22, 33, 0);
        let b = format_file_timestamp(2026, 8, 13, 14, 22, 34, 0);
        assert!(a < b, "早的时刻字典序更小: {a} < {b}");
    }

    // ---- 滚动判定(纯函数) ----

    #[test]
    fn rollover_boundaries() {
        assert!(!should_rollover(0), "空文件不滚动");
        assert!(!should_rollover(EVENT_LOG_MAX_BYTES - 1), "2MB-1 不滚动");
        assert!(should_rollover(EVENT_LOG_MAX_BYTES), "恰好 2MB 滚动");
        assert!(should_rollover(EVENT_LOG_MAX_BYTES + 1), "超 2MB 滚动");
    }

    // ---- panic 文件清理(纯函数) ----

    #[test]
    fn prune_keeps_newest_five() {
        let files: Vec<String> = (1..=8)
            .map(|i| format!("panic-20260813-100000-00{i}.log"))
            .collect();
        let doomed = panic_files_to_prune(&files, PANIC_LOG_KEEP);
        // 保留下标 3..8(时间戳最大的 5 个),删 1..=3
        assert_eq!(
            doomed,
            vec![
                "panic-20260813-100000-001.log".to_string(),
                "panic-20260813-100000-002.log".to_string(),
                "panic-20260813-100000-003.log".to_string(),
            ]
        );
    }

    #[test]
    fn prune_within_limit_removes_nothing() {
        let files: Vec<String> = (1..=5)
            .map(|i| format!("panic-20260813-100000-00{i}.log"))
            .collect();
        assert!(panic_files_to_prune(&files, PANIC_LOG_KEEP).is_empty());
        assert!(panic_files_to_prune(&[], PANIC_LOG_KEEP).is_empty());
    }

    // ---- 目录级清理与滚动(临时目录 IO 测试) ----

    #[test]
    fn prune_panic_logs_dir_keeps_five_newest() {
        let dir = tmp_dir("prune_dir");
        for i in 1..=7 {
            std::fs::write(dir.join(format!("panic-20260813-100000-00{i}.log")), "x").unwrap();
        }
        // 无关文件不参与清理
        std::fs::write(dir.join("aurora.log"), "x").unwrap();
        prune_panic_logs(&dir).unwrap();
        let remaining: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("panic-"))
            .collect();
        assert_eq!(remaining.len(), PANIC_LOG_KEEP);
        assert!(remaining.contains(&"panic-20260813-100000-007.log".to_string()));
        assert!(!remaining.contains(&"panic-20260813-100000-001.log".to_string()));
        assert!(dir.join("aurora.log").exists(), "非 panic 文件不受影响");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_event_rolls_over_at_limit() {
        let dir = tmp_dir("rollover");
        let log = dir.join(EVENT_LOG_NAME);
        // 预先写一个超过 2MB 的旧日志
        let big = "x".repeat((EVENT_LOG_MAX_BYTES + 1024) as usize);
        std::fs::write(&log, &big).unwrap();
        assert!(should_rollover(big.len() as u64));
        // 追加一行 → 旧文件改名 .old,新文件只含新行
        assert!(append_event(&dir, "[2026-08-13 14:22:33] [INFO] 新事件\n"));
        let old = dir.join(EVENT_LOG_OLD_NAME);
        assert!(old.exists(), "旧日志应滚动为 .old");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), big);
        let fresh = std::fs::read_to_string(&log).unwrap();
        assert_eq!(fresh, "[2026-08-13 14:22:33] [INFO] 新事件\n");
        // 再次滚动:旧 .old 被覆盖,新 .old 内容为上一条新文件
        let big2 = "y".repeat((EVENT_LOG_MAX_BYTES + 1) as usize);
        std::fs::write(&log, &big2).unwrap();
        assert!(append_event(&dir, "[2026-08-13 14:22:34] [INFO] 再滚一次\n"));
        assert_eq!(
            std::fs::read_to_string(&old).unwrap(),
            big2,
            ".old 应为上一次的现文件(覆盖旧 .old)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_event_creates_dir_and_file() {
        let dir = tmp_dir("append_new");
        let sub = dir.join("logs");
        assert!(append_event(&sub, "[2026-08-13 14:22:33] [INFO] 第一条\n"));
        assert_eq!(
            std::fs::read_to_string(sub.join(EVENT_LOG_NAME)).unwrap(),
            "[2026-08-13 14:22:33] [INFO] 第一条\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- panic 日志文本 ----

    #[test]
    fn panic_text_contains_all_sections() {
        let text = build_panic_text(
            "2026-08-13 14:22:33",
            "main",
            "src/lib.rs:10",
            "boom",
            "frame1\nframe2",
        );
        assert!(text.contains("时间: 2026-08-13 14:22:33"));
        assert!(text.contains("线程: main"));
        assert!(text.contains("位置: src/lib.rs:10"));
        assert!(text.contains("信息: boom"));
        assert!(text.contains("调用栈:\nframe1\nframe2"));
    }

    #[test]
    fn logs_dir_points_at_appdata() {
        // 环境变量缺失时回退临时目录(不 panic);存在时指向 %APPDATA%\com.aurora.desktop\logs
        let dir = logs_dir();
        if let Some(appdata) = std::env::var_os("APPDATA") {
            assert_eq!(
                dir,
                PathBuf::from(&appdata).join("com.aurora.desktop").join("logs")
            );
        } else {
            let s = dir.to_string_lossy();
            assert!(
                s.ends_with("com.aurora.desktop/logs") || s.ends_with("com.aurora.desktop\\logs"),
                "回退路径应仍以 com.aurora.desktop/logs 结尾: {s}"
            );
        }
    }
}
