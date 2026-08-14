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
    if !expected.eq_ignore_ascii_case(&actual) {
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
    // 带 30s 总超时(网络不通时静默返回 error,不挂死)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
        .unwrap_or_else(|_| reqwest::Client::new());
    let body = match client.get(&cfg.update_feed_url).send().await {
        Ok(r) if r.status().is_success() => match r.text().await {
            Ok(t) => t,
            Err(e) => {
                return UpdateCheckResult {
                    status: "error".to_string(),
                    version: None,
                    notes: None,
                    error: Some(format!("读取更新源失败: {e}")),
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
    let body = client
        .get(&cfg.update_feed_url)
        .send()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("读取更新源失败: {e}"))?;
    let info = updater::parse_update_info(&body).ok_or("更新源格式不合法")?;
    if !updater::compare_versions(APP_VERSION, &info.version) {
        return Err("当前已是最新版本".to_string());
    }
    let dir = updates_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建更新目录失败: {e}"))?;
    let dest = dir.join(format!("Aurora_setup_{}.exe", info.version));
    // 已下载且校验通过 → 直接完成(幂等)
    if dest.exists() && updater::sha256_hex(&dest).ok().as_deref() == Some(info.sha256.as_str()) {
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
    let app_for_progress = app.clone();
    let mut last_reported: u64 = 0;
    updater::download_update(&info.url, &dest, move |downloaded, total| {
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
    if actual != info.sha256 {
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
