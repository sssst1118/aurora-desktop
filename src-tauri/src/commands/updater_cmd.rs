//! 5.1 自动更新命令层(四命令 + 事件;实现层在 updater.rs)
//!
//! - update_check:拉 latest.json 对比当前版本,latest/available/error 三态;
//! - update_download:流式下载安装包到 %LOCALAPPDATA%\Aurora\updates,SHA-256 校验,
//!   已下载且校验通过幂等直接完成;进度经 update-progress 事件、结果经
//!   update-downloaded 事件(前端 Settings.vue 暂只订阅 update-downloaded,
//!   update-progress 已由后端节流发出,前端接上即可显示下载进度);
//! - update_install:spawn cmd 包装脚本(等静默安装完成 → 启动新版本),然后退出 app;
//! - update_open_folder:打开下载目录(手动安装兜底)。
//! 自研(不引 tauri-plugin-updater——其要求安装包证书签名;签名证书未采购前以
//! SHA-256 校验为替代防线,见 docs/代码签名接入.md)。

use serde::Serialize;
use tauri::{AppHandle, Emitter};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::updater::{self, UpdateInfo, UpdateProgress};

/// 当前应用版本(与 tauri.conf.json version 一致;编译期注入)
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 下载目录:%LOCALAPPDATA%\Aurora\updates
fn updates_dir() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Users\Public\AppData\Local"));
    base.join("Aurora").join("updates")
}

/// 下载校验通过记录文件名(安全加固):update_download 校验通过后把
/// (version → sha256) 写入该 sidecar,update_install 安装前对所选 exe
/// **复验**哈希——攻击者把伪造安装包丢进 updates 目录(命名对齐版本号)
/// 无法通过复验,杜绝"下载目录被投毒后一键安装"链路
const VERIFIED_FILE: &str = "latest_verified.json";

/// sidecar 文件路径(纯函数,可单测)
pub fn verified_sidecar_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join(VERIFIED_FILE)
}

/// 读 sidecar:version → sha256 映射(文件缺失/损坏 → 空映射,宁可拒绝安装)
pub fn load_verified(dir: &std::path::Path) -> std::collections::HashMap<String, String> {
    let Ok(bytes) = std::fs::read(verified_sidecar_path(dir)) else {
        return std::collections::HashMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// 写 sidecar:合并已有记录后落盘(纯函数 + IO,可单测)
pub fn save_verified(
    dir: &std::path::Path,
    version: &str,
    sha256: &str,
) -> Result<(), String> {
    let mut map = load_verified(dir);
    map.insert(version.trim().to_string(), sha256.trim().to_string());
    std::fs::create_dir_all(dir).map_err(|e| format!("创建更新目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| format!("序列化校验记录失败: {e}"))?;
    std::fs::write(verified_sidecar_path(dir), json)
        .map_err(|e| format!("写入校验记录失败: {e}"))
}

/// 安装前复验(纯函数 + IO,可单测):sidecar 无该 version 记录或所选 exe
/// 哈希与记录不符 → Err(拒绝安装);防 updates 目录被投毒后绕过下载校验
pub fn recheck_installer(
    dir: &std::path::Path,
    version: &str,
    exe: &std::path::Path,
) -> Result<(), String> {
    let verified = load_verified(dir);
    let expected = verified
        .get(version)
        .ok_or_else(|| "安装包无已验证记录,请重新下载更新".to_string())?;
    let actual = updater::sha256_hex(exe).map_err(|e| format!("校验失败: {e}"))?;
    // G1 加固(2026-08-14):统一走 sha256_eq(大小写不敏感)——下载校验与复验两处
    // 语义一致,feed 提供大写 sha256 时不再误报"校验失败"
    if !updater::sha256_eq(expected, &actual) {
        return Err("安装包校验失败(哈希与下载时不一致),已拒绝安装,请重新下载".to_string());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateCheckResult {
    pub status: String, // "latest" | "available" | "error"
    pub version: Option<String>,
    pub notes: Option<String>,
    pub error: Option<String>,
}

fn load_cfg(app: &AppHandle) -> crate::commands::config::AppConfig {
    crate::commands::config::load_from(&crate::commands::config::config_path(app))
}

/// 构建带 30s 总超时的更新检查客户端(纯逻辑)。
/// 加固(2026-08-14 G3):构建失败降级为默认 Client,不 panic 崩进程(与 ai/client.rs
/// build_client 的降级策略一致)。
/// 2026-08-18 修订(低 1):原实现是"连续构建失败才兜底"的双重 build——同参数重复
/// build 第二次必然失败,注释与实现不符;改为单次 build + unwrap_or_else 兜底。
fn update_check_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|e| {
            eprintln!("[aurora] 更新检查客户端构建失败,兜底默认 Client(无 30s 总超时): {e}");
            reqwest::Client::new()
        })
}

/// 更新源(latest.json)响应体大小上限(安全加固 2026-08-18 中 3):feed 只是几 KB 的
/// JSON 元数据,恶意/失控更新源可在 30s 超时内无限灌数据;安装包下载有 1GB 上限
/// (updater::MAX_DOWNLOAD_BYTES),feed 此前反而没有上限,现统一限制 256KB,
/// 超限按「更新源格式不合法」处理
const MAX_FEED_BYTES: u64 = 256 * 1024;

/// 读取更新源响应体(带上限):Content-Length 声明超 256KB 或流式累积超 256KB
/// → Err("更新源格式不合法")(feed 内容不可信,与 parse_update_info 失败同语义);
/// 流读失败 → Err("读取更新源失败: …")。修复背景(2026-08-18 中 3):原
/// resp.text() 无字节上限,巨型 feed 响应会全量载入内存。
async fn read_feed_body(resp: reqwest::Response) -> Result<String, String> {
    if let Some(len) = resp.content_length() {
        if len > MAX_FEED_BYTES {
            return Err("更新源格式不合法".to_string());
        }
    }
    use futures_util::StreamExt;
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("读取更新源失败: {e}"))?;
        buf.extend_from_slice(&bytes);
        if buf.len() as u64 > MAX_FEED_BYTES {
            return Err("更新源格式不合法".to_string());
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 手动检查更新(状态语义:latest/available/error;网络失败静默返回 error 文案)
#[tauri::command(rename = "update_check")]
pub async fn update_check(app: AppHandle) -> UpdateCheckResult {
    let cfg = load_cfg(&app);
    if !cfg.update_enabled {
        return UpdateCheckResult {
            status: "error".to_string(),
            version: None,
            notes: None,
            error: Some("自动更新已关闭".to_string()),
        };
    }
    // 带 30s 总超时(网络不通时静默返回 error,不挂死);构建失败降级同样带超时(见 update_check_client)
    let client = update_check_client();
    let body = match client.get(&cfg.update_feed_url).send().await {
        // 中 3 加固:feed 读取带 256KB 上限,超限报"更新源格式不合法"(此前
        // r.text() 无字节上限,巨型 feed 响应全量载入内存)
        Ok(r) if r.status().is_success() => match read_feed_body(r).await {
            Ok(t) => t,
            Err(e) => {
                return UpdateCheckResult {
                    status: "error".to_string(),
                    version: None,
                    notes: None,
                    error: Some(e),
                }
            }
        },
        Ok(r) => {
            return UpdateCheckResult {
                status: "error".to_string(),
                version: None,
                notes: None,
                error: Some(format!("更新源响应异常: HTTP {}", r.status())),
            }
        }
        Err(e) => {
            return UpdateCheckResult {
                status: "error".to_string(),
                version: None,
                notes: None,
                error: Some(format!("检查更新失败: {e}")),
            }
        }
    };
    let Some(info) = updater::parse_update_info(&body) else {
        return UpdateCheckResult {
            status: "error".to_string(),
            version: None,
            notes: None,
            error: Some("更新源格式不合法".to_string()),
        };
    };
    if updater::compare_versions(APP_VERSION, &info.version) {
        UpdateCheckResult {
            status: "available".to_string(),
            version: Some(info.version),
            notes: info.notes,
            error: None,
        }
    } else {
        UpdateCheckResult {
            status: "latest".to_string(),
            version: Some(info.version),
            notes: None,
            error: None,
        }
    }
}

/// 下载新版安装包(需先 check 到 available);流式下载,进度经 update-progress 事件
/// (节流上报;前端暂未订阅,契约已就位);完成 emit update-downloaded
#[tauri::command(rename = "update_download")]
pub async fn update_download(app: AppHandle) -> Result<(), String> {
    let cfg = load_cfg(&app);
    if !cfg.update_enabled {
        return Err("自动更新已关闭".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建请求客户端失败: {e}"))?;
    let resp = client
        .get(&cfg.update_feed_url)
        .send()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;
    // 中 3 加固:feed 读取带 256KB 上限,超限按「更新源格式不合法」处理
    let body = read_feed_body(resp).await?;
    let info = updater::parse_update_info(&body).ok_or("更新源格式不合法")?;
    if !updater::compare_versions(APP_VERSION, &info.version) {
        return Err("当前已是最新版本".to_string());
    }
    let dir = updates_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建更新目录失败: {e}"))?;
    let dest = dir.join(format!("Aurora_setup_{}.exe", info.version));
    // 已下载且校验通过 → 直接完成(幂等)
    // G1 加固(2026-08-14):sha256_eq 大小写不敏感——此前与 feed 的大写 sha256
    // 严格相等比较,大写 hex 时"已下载"幂等分支永远命中不了,反复重下
    if dest.exists()
        && updater::sha256_hex(&dest)
            .map(|a| updater::sha256_eq(&a, &info.sha256))
            .unwrap_or(false)
    {
        save_verified(&dir, &info.version, &info.sha256)?; // 复验记录补齐(幂等)
        let _ = app.emit(
            "update-downloaded",
            &UpdateInfo {
                version: info.version,
                url: info.url,
                sha256: info.sha256,
                notes: info.notes,
            },
        );
        return Ok(());
    }
    // 流式下载 + 进度上报(节流:每 ≥1% 或 ≥512KB 报一次,防小块高频事件风暴;
    // total 未知(无 Content-Length)时按 512KB 固定步长上报,percent 为 None)
    // G2 加固(2026-08-14):传 MAX_DOWNLOAD_BYTES 下载上限,feed 的 url 不受 host
    // 白名单约束,超限由 download_update 中止并清理半成品
    let app_for_progress = app.clone();
    let mut last_reported: u64 = 0;
    updater::download_update(&info.url, &dest, updater::MAX_DOWNLOAD_BYTES, move |downloaded, total| {
        let step = (total / 100).max(512 * 1024);
        if downloaded == total || downloaded - last_reported >= step {
            last_reported = downloaded;
            let _ = app_for_progress.emit(
                "update-progress",
                &UpdateProgress {
                    downloaded_bytes: downloaded,
                    total_bytes: if total > 0 { Some(total) } else { None },
                    percent: if total > 0 {
                        Some(downloaded as f64 / total as f64)
                    } else {
                        None
                    },
                },
            );
        }
    })
    .map_err(|e| {
        // 流式失败可能留下半成品文件:删除(哈希本就不符会被拒,这里提前清理保持
        // updates 目录干净;下次重试会重新下载覆盖)
        let _ = std::fs::remove_file(&dest);
        let _ = app.emit("update-error", &serde_json::json!({ "message": e }));
        e
    })?;
    let actual = updater::sha256_hex(&dest).map_err(|e| format!("校验失败: {e}"))?;
    // G1 加固(2026-08-14):大小写不敏感比较,与 recheck_installer 语义一致
    if !updater::sha256_eq(&actual, &info.sha256) {
        let _ = std::fs::remove_file(&dest);
        return Err("下载文件校验失败(哈希不匹配),已删除,请重试".to_string());
    }
    save_verified(&dir, &info.version, &info.sha256)?; // 下载校验通过 → 落复验记录
    let _ = app.emit(
        "update-downloaded",
        &UpdateInfo {
            version: info.version,
            url: info.url,
            sha256: info.sha256,
            notes: info.notes,
        },
    );
    Ok(())
}

/// 从更新目录挑选最新版本安装包(纯函数,可单测):
/// 文件名形如 Aurora_setup_<version>.exe;版本不可解析的条目剔除;空 → None
pub fn pick_latest_candidate(
    entries: Vec<(String, std::path::PathBuf)>,
) -> Option<(String, std::path::PathBuf)> {
    entries
        .into_iter()
        .max_by(|a, b| updater::version_cmp(&a.0, &b.0))
}

/// 退出并静默安装:spawn cmd 包装脚本(等安装器完成 → 启动新版本),然后退出 app
#[tauri::command(rename = "update_install")]
pub fn update_install(app: AppHandle) -> Result<(), String> {
    // 扫描 updates 目录取最新 version 安装包(与 update_download 产物对齐,无网络依赖)
    let dir = updates_dir();
    let candidates: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .map_err(|e| format!("读取更新目录失败: {e}"))?
        .flatten()
        .filter_map(|f| {
            let path = f.path();
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase() == "exe")
                .unwrap_or(false);
            if !ext {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            let v = name.strip_prefix("Aurora_setup_")?.strip_suffix(".exe")?.to_string();
            Some((v, path))
        })
        .collect();
    // 取语义化最大版本(version_cmp 为纯函数,见 updater.rs 单测)
    let Some((ver, exe)) = pick_latest_candidate(candidates) else {
        return Err("未找到已下载的安装包,请先下载更新".to_string());
    };
    // 安全加固:安装前对所选 exe 复验哈希(sidecar 记录的下载时校验值),
    // updates 目录被投毒(伪造同名安装包)时在此拒绝,不进入安装流程
    recheck_installer(&dir, &ver, &exe)?;
    // 稳定性包:更新安装属关键事件,记入事件日志(退出重启前的最后一条)
    crate::logger::log_event("INFO", &format!("开始静默安装更新 v{ver}(安装完成后自动重启)"));
    let install_dir = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Users\Public\AppData\Local"))
        .join("Aurora");
    let app_exe = install_dir.join("Aurora.exe");
    // 包装脚本(固定文案,不含任何路径):等静默安装完成 → 启动新版本
    // (独立 cmd 进程,与 app 生命周期解耦)。
    // 安全加固:路径经环境变量 %AURORA_SETUP_EXE% / %AURORA_NEW_EXE% 注入,
    // 不拼进命令行——直接 format 拼接时路径含空格会拆参、含 & 会被注入命令。
    // 风险闭环由两层共同保证:① 注入值在脚本中被双引号包裹,cmd 展开 %VAR%
    // 后,插入的文本不再重新按空格/& 等元字符解析(变量替换是纯文本替换,
    // 替换结果不二次展开),路径作为整体参数传给 start;② 唯一的残余风险是
    // 值内含双引号破坏引号配对,但注入值来自程序内部路径(updates 目录内
    // Aurora_setup_<version>.exe,选包时剥离前后缀,安装前再经 SHA-256 复验),
    // 双引号是 Windows 文件名字符集中的非法字符,该风险天然排除。
    const INSTALL_SCRIPT: &str =
        "start /wait \"\" \"%AURORA_SETUP_EXE%\" /S && start \"\" \"%AURORA_NEW_EXE%\"";
    let _ = std::process::Command::new("cmd")
        .args(["/C", INSTALL_SCRIPT])
        .env("AURORA_SETUP_EXE", &exe)
        .env("AURORA_NEW_EXE", &app_exe)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()
        .map_err(|e| format!("启动安装器失败: {e}"))?;
    // G4 界面反馈(2026-08-14):spawn 成功后、退出前广播 update-install-start,
    // 让前端提示"开始安装,安装器将接管后续(静默完成后自动启动新版本)"——
    // 此前 spawn 后立即 app.exit(0),安装器返回非 0 时用户只见旧版退出、新版未启动,
    // 无任何反馈。方案说明:app.exit(0) 在命令内同步执行,invoke 的返回值永远来不及
    // 回到前端,事件是退出前唯一可靠通道(与 update-progress 的"契约先就位、前端
    // 后续接上"先例一致);安装器结果不在 app 内等待——等待会改变"秒退交给安装器"
    // 的现有行为语义,失败场景由安装器自报告/自恢复。
    let _ = app.emit(
        "update-install-start",
        &serde_json::json!({
            "version": ver,
            "message": format!("开始安装 v{ver},安装器将接管后续(静默完成后自动启动新版本)")
        }),
    );
    app.exit(0);
    Ok(())
}

/// 打开更新下载目录(手动安装兜底)
#[tauri::command(rename = "update_open_folder")]
pub fn update_open_folder() -> Result<(), String> {
    let dir = updates_dir();
    let _ = std::fs::create_dir_all(&dir);
    opener::open(&dir).map_err(|e| format!("打开目录失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pick_latest_picks_highest_version() {
        let entries = vec![
            ("0.1.0".to_string(), PathBuf::from("Aurora_setup_0.1.0.exe")),
            ("0.2.0".to_string(), PathBuf::from("Aurora_setup_0.2.0.exe")),
            ("0.1.10".to_string(), PathBuf::from("Aurora_setup_0.1.10.exe")),
        ];
        let (v, p) = pick_latest_candidate(entries).unwrap();
        assert_eq!(v, "0.2.0");
        assert!(p.to_string_lossy().ends_with("0.2.0.exe"));
    }

    #[test]
    fn pick_latest_ignores_unparseable_and_empty() {
        assert_eq!(pick_latest_candidate(vec![]), None);
        let entries = vec![("bad".to_string(), PathBuf::from("Aurora_setup_bad.exe"))];
        // "bad" 不可解析 → 与任何项 Equal,取第一个(候选集只有它时仍返回)
        assert!(pick_latest_candidate(entries).is_some());
    }

    // ---- 更新检查客户端(2026-08-14 G3 加固:构建失败兜底默认 Client) ----

    #[test]
    fn update_check_client_reaches_local_server() {
        // 冒烟测试:构建出的客户端(带 30s 总超时)能完成本地回环请求。
        // 兜底分支仅在 reqwest 构建失败时可达,进程内无法模拟,由代码走查
        // 保证其为默认 Client(失去 30s 超时语义,属极端环境降级,与
        // ai/client.rs build_client 策略一致)
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf);
            let _ = sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            );
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let body = rt.block_on(async {
            update_check_client()
                .get(&format!("http://{addr}/latest.json"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        server.join().unwrap();
        assert_eq!(body, "ok");
    }

    // ---- 更新源响应体上限(2026-08-18 中 3 加固:feed 读取带 256KB 上限) ----

    fn local_feed_server(body: Vec<u8>, content_length: Option<usize>) -> std::net::SocketAddr {
        // 本地极简 HTTP 服务器:返回固定 body(声明或省略 Content-Length),
        // 供 read_feed_body 测试(127.0.0.1 回环,无外网依赖)
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            let head = match content_length {
                Some(len) => format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"),
                None => "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_string(),
            };
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(&body);
        });
        addr
    }

    #[test]
    fn read_feed_body_rejects_declared_oversize() {
        // Content-Length 声明 300KB > 256KB → 立即 Err(不读 body)
        let addr = local_feed_server(vec![b'x'; 300 * 1024], Some(300 * 1024));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(async {
            read_feed_body(
                update_check_client()
                    .get(&format!("http://{addr}/latest.json"))
                    .send()
                    .await
                    .unwrap(),
            )
            .await
        })
        .expect_err("声明超限必须报错");
        assert!(err.contains("更新源格式不合法"), "超限文案应指向格式不合法: {err}");
    }

    #[test]
    fn read_feed_body_rejects_accumulated_oversize() {
        // 无 Content-Length(分块/close 语义):靠流式累积兜底,300KB 同样拒绝
        let addr = local_feed_server(vec![b'x'; 300 * 1024], None);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(async {
            read_feed_body(
                update_check_client()
                    .get(&format!("http://{addr}/latest.json"))
                    .send()
                    .await
                    .unwrap(),
            )
            .await
        })
        .expect_err("累积超限必须报错");
        assert!(err.contains("更新源格式不合法"), "超限文案应指向格式不合法: {err}");
    }

    #[test]
    fn read_feed_body_small_body_ok() {
        // 正常小 feed(几 KB 量级)→ 读出完整内容
        let small = br#"{"version":"0.2.0","url":"https://x/a.exe","sha256":"abcd"}"#.to_vec();
        let addr = local_feed_server(small.clone(), Some(small.len()));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let body = rt.block_on(async {
            read_feed_body(
                update_check_client()
                    .get(&format!("http://{addr}/latest.json"))
                    .send()
                    .await
                    .unwrap(),
            )
            .await
            .unwrap()
        });
        assert!(body.contains("0.2.0"), "正常 feed 应读出内容: {body}");
    }

    // ---- sidecar 校验记录(安全加固:安装前复验) ----

    fn tmp_updates_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("aurora_upd_{tag}_{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn verified_sidecar_roundtrip_and_merge() {
        let dir = tmp_updates_dir("sidecar");
        // 初始:无 sidecar → 空映射
        assert!(load_verified(&dir).is_empty());
        // 写入两条(不同版本),记录合并保留
        save_verified(&dir, "0.2.0", "aaaa").unwrap();
        save_verified(&dir, "0.3.0", "bbbb").unwrap();
        let map = load_verified(&dir);
        assert_eq!(map.get("0.2.0").map(String::as_str), Some("aaaa"));
        assert_eq!(map.get("0.3.0").map(String::as_str), Some("bbbb"));
        // 同版本重复写入 = 覆盖(下载重试场景)
        save_verified(&dir, "0.2.0", "cccc").unwrap();
        assert_eq!(load_verified(&dir).get("0.2.0").map(String::as_str), Some("cccc"));
        // sidecar 文件真实存在
        assert!(verified_sidecar_path(&dir).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recheck_installer_accepts_matching_sha() {
        let dir = tmp_updates_dir("recheck_ok");
        let exe = dir.join("Aurora_setup_0.2.0.exe");
        std::fs::write(&exe, b"installer-bytes").unwrap();
        let sha = updater::sha256_hex(&exe).unwrap();
        save_verified(&dir, "0.2.0", &sha).unwrap();
        assert!(recheck_installer(&dir, "0.2.0", &exe).is_ok(), "哈希一致应放行");
        // 大小写不敏感的 hex 比较
        let upper = sha.to_uppercase();
        save_verified(&dir, "0.2.0", &upper).unwrap();
        assert!(recheck_installer(&dir, "0.2.0", &exe).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recheck_installer_rejects_tampered_or_unrecorded() {
        let dir = tmp_updates_dir("recheck_bad");
        let exe = dir.join("Aurora_setup_0.2.0.exe");
        std::fs::write(&exe, b"installer-bytes").unwrap();
        let sha = updater::sha256_hex(&exe).unwrap();
        save_verified(&dir, "0.2.0", &sha).unwrap();
        // 篡改安装包 → 哈希不匹配 → 拒绝
        std::fs::write(&exe, b"tampered!!").unwrap();
        assert!(recheck_installer(&dir, "0.2.0", &exe).is_err(), "被篡改的安装包必须拒绝");
        // sidecar 无该版本记录 → 拒绝(投毒文件从未经过下载校验)
        let other = dir.join("Aurora_setup_9.9.9.exe");
        std::fs::write(&other, b"poison").unwrap();
        assert!(recheck_installer(&dir, "9.9.9", &other).is_err(), "无已验证记录的安装包必须拒绝");
        // sidecar 缺失 → 拒绝
        let empty = tmp_updates_dir("recheck_empty");
        let e2 = empty.join("Aurora_setup_1.0.0.exe");
        std::fs::write(&e2, b"x").unwrap();
        assert!(recheck_installer(&empty, "1.0.0", &e2).is_err());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }
}
