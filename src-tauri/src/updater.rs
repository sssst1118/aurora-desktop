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

/// 解析 latest.json;缺 version/url/sha256 任一或 JSON 非法 → None
pub fn parse_update_info(json: &str) -> Option<UpdateInfo> {
    let info: UpdateInfo = serde_json::from_str(json).ok()?;
    if info.version.trim().is_empty() || info.url.trim().is_empty() || info.sha256.trim().is_empty() {
        return None;
    }
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
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
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

/// 分块流式下载到 dest(5 分钟总超时;请求失败/写失败 → Err;调用方负责 sha256 校验与清理)。
///
/// 2026-08-13 审计整改:原实现 resp.bytes() 把安装包全量载入内存再落盘(几十 MB 常驻),
/// 改为 chunk 流式逐块写文件,内存占用 O(单块)。每写完一块同步回调一次
/// on_progress(downloaded, total)(内部 block_on,回调在调用线程执行),total 取
/// Content-Length(缺失传 0,调用方按未知处理)。失败可能留下半成品文件,由调用方清理。
pub fn download_update<F: FnMut(u64, u64)>(
    url: &str,
    dest: &std::path::Path,
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
            file.write_all(&chunk).map_err(|e| format!("写入文件失败: {e}"))?;
            downloaded += chunk.len() as u64;
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
        download_update(&format!("http://{addr}/Aurora_setup.exe"), &dest, |d, t| {
            events.push((d, t));
        })
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
}
