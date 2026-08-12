//! 5.1 自动更新命令层(四命令 + 事件;实现层在 updater.rs)
//!
//! - update_check:拉 latest.json 对比当前版本,latest/available/error 三态;
//! - update_download:下载安装包到 %LOCALAPPDATA%\Aurora\updates,SHA-256 校验,
//!   已下载且校验通过幂等直接完成;进度/结果经 update-progress / update-downloaded 事件;
//! - update_install:spawn cmd 包装脚本(等静默安装完成 → 启动新版本),然后退出 app;
//! - update_open_folder:打开下载目录(手动安装兜底)。
//! 自研(不引 tauri-plugin-updater——其要求安装包证书签名;签名证书未采购前以
//! SHA-256 校验为替代防线,见 docs/代码签名接入.md)。

use serde::Serialize;
use tauri::{AppHandle, Emitter};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::updater::{self, UpdateInfo};

/// 当前应用版本(与 tauri.conf.json version 一致;编译期注入)
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 下载目录:%LOCALAPPDATA%\Aurora\updates
fn updates_dir() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Users\Public\AppData\Local"));
    base.join("Aurora").join("updates")
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

/// 下载新版安装包(需先 check 到 available);进度经 update-progress 事件;完成 emit update-downloaded
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
    updater::download_update(&info.url, &dest).map_err(|e| {
        let _ = app.emit("update-error", &serde_json::json!({ "message": e }));
        e
    })?;
    let actual = updater::sha256_hex(&dest).map_err(|e| format!("校验失败: {e}"))?;
    if actual != info.sha256 {
        let _ = std::fs::remove_file(&dest);
        return Err("下载文件校验失败(哈希不匹配),已删除,请重试".to_string());
    }
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
    let Some((_ver, exe)) = pick_latest_candidate(candidates) else {
        return Err("未找到已下载的安装包,请先下载更新".to_string());
    };
    let install_dir = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Users\Public\AppData\Local"))
        .join("Aurora");
    let app_exe = install_dir.join("Aurora.exe");
    // 包装脚本:等静默安装完成 → 启动新版本(独立 cmd 进程,与 app 生命周期解耦)
    let script = format!(
        "start /wait \"\" \"{}\" /S && start \"\" \"{}\"",
        exe.display(),
        app_exe.display()
    );
    let _ = std::process::Command::new("cmd")
        .args(["/C", &script])
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
}
