//! 5.1 自动更新实现层(纯逻辑 + 下载;零 tauri-plugin 依赖,自研)
//!
//! - 更新源:远端 latest.json(GitHub Releases 或自建静态服务,地址可配置);
//! - 版本比较:自实现语义化(x.y.z 逐段,不可解析段按 0,忽略预发布后缀);
//! - 下载:分块流式落盘 + SHA-256 校验(sha2 crate,防中间人;签名证书未采购前的替代防线);
//! - 安装:退出 app 前 spawn cmd 包装脚本(等安装器静默完成 → 启动新版本)。

use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// version 字段允许字符白名单(安全加固 2026-08-14):version 会拼进
/// `Aurora_setup_{version}.exe` 文件名与复验 sidecar 键,含 `\` `/` 等分隔符的版本
/// 存在路径逃逸可能(需预建目录才可写出 updates 目录外,利用条件苛刻,但属未校验
/// 输入);白名单外直接拒绝。允许:ASCII 字母数字 + `.` `_` `-`(覆盖常见语义化版本写法)。
fn valid_version(s: &str) -> bool {
    !s.is_empty()
        // 低 2 加固(2026-08-18):纯点号串(如 ".." / "....")此前能通过白名单,
        // 会拼出 "Aurora_setup_....exe" 之类的文件名;要求至少含一个非点字符
        && !s.chars().all(|c| c == '.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// 解析 latest.json;缺 version/url/sha256 任一、version 含白名单外字符或 JSON 非法 → None。
/// version 首尾空白先 trim(白名单校验的是 trim 后的值,落库为 trim 后的版本,
/// 避免空白字符混进文件名)。
pub fn parse_update_info(json: &str) -> Option<UpdateInfo> {
    let mut info: UpdateInfo = serde_json::from_str(json).ok()?;
    if info.url.trim().is_empty() || info.sha256.trim().is_empty() {
        return None;
    }
    let version = info.version.trim();
    if !valid_version(version) {
        return None;
    }
    info.version = version.to_string();
    Some(info)
}

/// 语义化版本三段解析:x.y.z 逐段数值;段缺失按 0;后续段不可解析按 0;预发布后缀(-/+)忽略。
/// 首段不可解析 → None(整个版本串不可解析,调用方按 Equal 处理防误判更新)
fn version_parts(v: &str) -> Option<[u32; 3]> {
    let mut out = [0u32; 3];
    let mut it = v.split(['-', '+']).next().unwrap_or("").split('.');
    out[0] = it.next().unwrap_or("").parse::<u32>().ok()?;
    for slot in out.iter_mut().skip(1) {
        *slot = it.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    }
    Some(out)
}

/// 语义化版本 Ordering(供 max_by 选最新版与比较);任一版本串不可解析 → Equal(不更新/不覆盖)
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (version_parts(a), version_parts(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => std::cmp::Ordering::Equal,
    }
}

/// 语义化版本比较:latest > current → true(有更新);相等/更旧 → false
pub fn compare_versions(current: &str, latest: &str) -> bool {
    version_cmp(current, latest) == std::cmp::Ordering::Less
}

/// 文件 SHA-256 hex(小写)
pub fn sha256_hex(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    // 中 4 加固(2026-08-18):std::fs::read 全量载入内存再哈希,1GB 上限的安装包
    // 校验时内存峰值 ≈1GB,抵消下载阶段的流式优化;改 BufReader 分块流式哈希
    // (sha2 流式 API:Digest::update 逐块喂入),内存占用 O(64KB) 恒定
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("读取文件失败: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// SHA-256 hex 相等比较(大小写不敏感,纯函数,可单测):下载校验与安装前复验统一走本函数。
/// 修复背景(2026-08-14 G1):update_download 的哈希比较大小写敏感(本地计算的 hex
/// 恒为小写),而 recheck_installer 用 eq_ignore_ascii_case——feed 提供大写 sha256 时
/// 下载永远"校验失败"误报;统一 to_lowercase 后两处语义一致。
pub fn sha256_eq(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// 下载进度回调契约(update-progress 事件 payload;由 download_update 逐块回调):
/// downloaded_bytes = 已落盘字节数;total_bytes = 响应 Content-Length(缺失为 None);
/// percent = 已下载占比 0.0-1.0(total 未知为 None)
#[derive(Clone, Debug, Serialize)]
pub struct UpdateProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percent: Option<f64>,
}

/// 安装包下载大小上限(安全加固 2026-08-14):feed 的 url 字段不受 host 白名单约束
/// (白名单只校验 update_feed_url 本身),任意地址的无限/巨型响应可在 300s 超时内
/// 填满磁盘;超过即中止并清理半成品文件。正常安装包几十 MB,1GB 上限绰绰有余。
pub const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;

/// 分块流式下载到 dest(5 分钟总超时;请求失败/写失败/超限 → Err;调用方负责 sha256 校验与清理)。
///
/// 2026-08-13 审计整改:原实现 resp.bytes() 把安装包全量载入内存再落盘(几十 MB 常驻),
/// 改为 chunk 流式逐块写文件,内存占用 O(单块)。每写完一块同步回调一次
/// on_progress(downloaded, total)(内部 block_on,回调在调用线程执行),total 取
/// Content-Length(缺失传 0,调用方按未知处理)。
///
/// 2026-08-14 加固:新增 max_bytes 下载上限——响应体累计超过即中止并清理半成品
/// 文件(先 drop 文件句柄再删,Windows 下占用中的文件删除会失败);其余失败路径
/// 仍由调用方清理(行为不变)。
pub fn download_update<F: FnMut(u64, u64)>(
    url: &str,
    dest: &std::path::Path,
    max_bytes: u64,
    mut on_progress: F,
) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("创建运行时失败: {e}"))?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| format!("创建请求客户端失败: {e}"))?;
        let mut resp = client.get(url).send().await.map_err(|e| format!("下载失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("下载失败: HTTP {}", resp.status()));
        }
        let total = resp.content_length().unwrap_or(0);
        let mut file = std::fs::File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
        let mut downloaded: u64 = 0;
        loop {
            // resp.chunk():reqwest 逐块读取响应体(每块几 KB~64KB),不整体缓冲
            let Some(chunk) =
                resp.chunk().await.map_err(|e| format!("读取响应失败: {e}"))?
            else {
                break; // 响应体结束
            };
            downloaded += chunk.len() as u64;
            if downloaded > max_bytes {
                // 超限:本块不落盘,清理半成品后中止(先 drop 句柄再删,
                // 防 Windows 对占用中文件删除失败)
                drop(file);
                let _ = std::fs::remove_file(dest);
                return Err(format!(
                    "下载中止:响应体超过 {max_bytes} 字节上限(疑似异常/恶意响应源),已清理半成品文件"
                ));
            }
            file.write_all(&chunk).map_err(|e| format!("写入文件失败: {e}"))?;
            on_progress(downloaded, total);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_update_info_valid() {
        let j = r#"{"version":"0.2.0","url":"https://github.com/sssst1118/aurora-desktop/releases/download/v0.2.0/Aurora_0.2.0_x64-setup.exe","sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","notes":"修复若干问题"}"#;
        let info = parse_update_info(j).unwrap();
        assert_eq!(info.version, "0.2.0");
        assert!(info.url.ends_with("x64-setup.exe"));
        assert_eq!(info.notes.as_deref(), Some("修复若干问题"));
    }

    #[test]
    fn parse_update_info_missing_fields_rejected() {
        assert!(parse_update_info(r#"{"version":"0.2.0"}"#).is_none());
        assert!(parse_update_info("not json").is_none());
        assert!(parse_update_info(r#"{"version":"0.2.0","url":"x","sha256":""}"#).is_none());
    }

    #[test]
    fn compare_versions_semver() {
        assert!(compare_versions("0.1.0", "0.2.0"));
        assert!(compare_versions("0.1.9", "0.1.10"));
        assert!(compare_versions("0.9.0", "1.0.0"));
        assert!(!compare_versions("0.2.0", "0.2.0")); // 相等无更新
        assert!(!compare_versions("0.2.0", "0.1.0")); // 更旧无更新
        assert!(!compare_versions("bad", "0.2.0")); // 当前不可解析 → 视为无更新(0.0.0)
        assert!(!compare_versions("0.1.0", "bad")); // 最新不可解析 → 无更新
    }

    #[test]
    fn version_cmp_ordering_for_max() {
        use std::cmp::Ordering;
        assert_eq!(version_cmp("0.1.0", "0.2.0"), Ordering::Less);
        assert_eq!(version_cmp("0.2.0", "0.2.0"), Ordering::Equal);
        assert_eq!(version_cmp("1.0.0", "0.9.9"), Ordering::Greater);
        assert_eq!(version_cmp("0.1.0-beta", "0.1.0"), Ordering::Equal); // 预发布后缀忽略
    }

    #[test]
    fn sha256_hex_known_vector() {
        let p = std::env::temp_dir().join("aurora_sha_test.txt");
        std::fs::write(&p, b"abc").unwrap();
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(sha256_hex(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn sha256_hex_missing_file_err() {
        let p = std::env::temp_dir().join("aurora_sha_none.bin");
        assert!(sha256_hex(&p).is_err());
    }

    #[test]
    fn sha256_hex_streams_large_file_correctly() {
        // 中 4 加固回归:流式分块哈希的结果必须与全量一次哈希一致
        // (大文件路径;10MB 确定性内容,无需随机源)
        use sha2::{Digest, Sha256};
        let p = std::env::temp_dir().join("aurora_sha_large.bin");
        let content: Vec<u8> = (0..10_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &content).unwrap();
        let expected: String = Sha256::digest(&content)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(sha256_hex(&p).unwrap(), expected, "流式哈希必须与全量哈希一致");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn download_update_streams_chunks_with_progress() {
        // 本地极简 HTTP 服务器(线程):返回固定字节流 + Content-Length,验证流式
        // 落盘与进度回调契约(downloaded 单调递增、末次 == total;127.0.0.1 回环,
        // reqwest 未开 proxy feature 不走代理,测试无外网依赖)
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let body_for_server = body.clone();
        let server = std::thread::spawn(move || {
            use std::io::Read;
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf); // 读请求头即可,不校验内容
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_for_server.len()
            );
            sock.write_all(head.as_bytes()).unwrap();
            sock.write_all(&body_for_server).unwrap();
        });
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dest = std::env::temp_dir().join(format!("aurora_upd_dl_{nanos}.bin"));
        let mut events: Vec<(u64, u64)> = Vec::new();
        download_update(
            &format!("http://{addr}/Aurora_setup.exe"),
            &dest,
            MAX_DOWNLOAD_BYTES,
            |d, t| {
                events.push((d, t));
            },
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body, "落盘内容必须与响应体一致");
        assert!(!events.is_empty(), "分块回调至少一次");
        let last = events.last().unwrap();
        assert_eq!(last.0, body.len() as u64, "末次 downloaded == 总字节数");
        assert_eq!(last.1, body.len() as u64, "total 取自 Content-Length");
        // 进度单调递增(流式无回退)
        for w in events.windows(2) {
            assert!(w[1].0 >= w[0].0, "downloaded 必须单调递增");
        }
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn download_update_aborts_over_size_limit_and_cleans_up() {
        // 上限参数化使边界可测:服务器发 100KB,上限 4KB → 必须中止 +
        // 半成品文件被清理;服务器端写入容忍 EPIPE(客户端超限中止后连接断开)
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let body_for_server = body.clone();
        let server = std::thread::spawn(move || {
            use std::io::Read;
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_for_server.len()
            );
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(&body_for_server); // 客户端中止后 EPIPE,忽略
        });
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dest = std::env::temp_dir().join(format!("aurora_upd_dl_limit_{nanos}.bin"));
        let err = download_update(&format!("http://{addr}/Aurora_setup.exe"), &dest, 4096, |_, _| {})
            .expect_err("响应体超过上限必须中止");
        server.join().unwrap();
        assert!(err.contains("上限"), "错误信息应说明超限原因: {err}");
        assert!(!dest.exists(), "超限后半成品文件应被清理: {}", dest.display());
    }

    // ---- version 白名单(2026-08-14 G2 加固:防文件名路径逃逸) ----

    #[test]
    fn valid_version_whitelist() {
        for good in ["0.2.0", "v1.2.3", "0.2.0-beta.1", "1.0_build5", "0.2", "abc"] {
            assert!(valid_version(good), "白名单内应放行: {good}");
        }
        for bad in [
            "",          // 空串
            ".",         // 纯点号(低 2 加固:拼出 "Aurora_setup_..exe" 类文件名)
            "..",        // 纯点号
            "...",       // 纯点号
            "....",      // 纯点号
            "0.2.0/1",   // 正斜杠路径分隔符
            "..\\..",    // 反斜杠 + 目录穿越
            "1 0",       // 空白
            "1.0\n",     // 控制字符
            "版本号",     // 非 ASCII
            "1.0!",      // 符号
            "a=b",       // 符号
        ] {
            assert!(!valid_version(bad), "白名单外应拒绝: {bad:?}");
        }
    }

    #[test]
    fn parse_update_info_trims_and_rejects_version() {
        // 首尾空白 trim 后放行,落库为 trim 后的版本(防空白混进文件名)
        let j = r#"{"version":" 0.2.0 ","url":"https://x/a.exe","sha256":"abcd"}"#;
        assert_eq!(parse_update_info(j).unwrap().version, "0.2.0");
        // 白名单外字符(JSON 合法但版本非法)→ 整体拒绝
        let bad = r#"{"version":"0.2.0 中文","url":"https://x/a.exe","sha256":"abcd"}"#;
        assert!(parse_update_info(bad).is_none());
    }

    // ---- SHA-256 比较(2026-08-14 G1 加固:大小写统一) ----

    #[test]
    fn sha256_eq_case_insensitive() {
        assert!(sha256_eq("ABC", "abc"));
        assert!(sha256_eq("abc", "ABC"));
        assert!(sha256_eq("", ""));
        assert!(!sha256_eq("abc", "abd"));
        // 典型场景:feed 提供大写 vs 本地计算小写(sha256_hex 恒小写)
        let upper = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        let lower = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(sha256_eq(upper, lower));
        // 内容不同(非仅大小写差异)→ 不相等
        assert!(!sha256_eq(upper, &format!("{lower}0")));
    }
}
