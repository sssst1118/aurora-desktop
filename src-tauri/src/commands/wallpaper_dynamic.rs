//! 4.1 动态壁纸命令层(四命令,全部 #[tauri::command];注册待集成 agent 接线)
//!
//! - wallpaper_dynamic_list:扫素材目录(回退链见实现层),白名单过滤排序截 100;
//! - wallpaper_dynamic_set:校验 → 按类型分派:图片复用 2.4 wallpaper_set_static_cmd(走系统壁纸,
//!   不占 WorkerW);mp4/html 记录素材路径 + 注入 WorkerW;同路径重复 set 幂等无副作用;
//!   返回 WallpaperSetInfo{path, url}(契约修正版,见设计 §1.2 备注);
//! - wallpaper_dynamic_clear:撤下 WorkerW + 清内存记录(不动系统壁纸);
//! - wallpaper_dynamic_get_state:状态徽标数据。
//!
//! 命令名与前端 invoke 名一致(当前无 stubs.rs 同名冲突);电池检测线程启动函数
//! `wallpaper_dynamic::spawn_battery_watcher` 由集成 agent 在 lib.rs setup 接线,
//! set 命令内部也做了惰性自启兜底(幂等)。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::wallpaper::WallpaperEntry;
use crate::wallpaper_dynamic::{self, WallpaperState};

/// set 命令返回契约(集成 agent 接线用):
/// - path:素材绝对路径;
/// - url:素材在默认图片目录(Pictures)内 → None(前端自行 convertFileSrc 走 asset 协议);
///   目录外 → Some(data URL)(后端读文件转 base64,≤50MB,超出报错提示放 Pictures 下)。
///   两段式方案见设计文档 §1.5/AD-2。
#[derive(Clone, Debug, Serialize)]
pub struct WallpaperSetInfo {
    pub path: String,
    pub url: Option<String>,
}

/// 动态壁纸状态(get_state 返回;设置区状态徽标与壁纸窗口渲染用)
/// 注:url 字段为两段式 URL 契约的增量扩展(设计 §1.5):壁纸窗口渲染目录外素材必须
/// 拿到 set 时算好的 data URL;素材在默认 Pictures 内时 url=None(前端 convertFileSrc)
#[derive(Clone, Debug, Serialize)]
pub struct DynamicWallpaperState {
    pub enabled: bool,          // enable_dynamic_wallpaper 配置值
    pub kind: String,           // "none" | "image" | "video" | "html"
    pub path: Option<String>,
    pub url: Option<String>,    // video/html 素材:目录外 = data URL;Pictures 内 = None(前端 convertFileSrc)
    pub on_battery: bool,       // 最近一次电池检测结果(无检测记录 = false)
    pub downshift_active: bool, // enable_dynamic_wallpaper && wallpaper_battery_downshift
}

/// 读配置(不走 config_load 的密钥脱敏,这里不涉及密钥)
fn load_cfg(app: &AppHandle) -> crate::commands::config::AppConfig {
    crate::commands::config::load_from(&crate::commands::config::config_path(app))
}

/// 列出动态壁纸素材(只扫配置目录;目录不存在返回空列表,不 panic)
#[tauri::command(rename = "wallpaper_dynamic_list")]
pub fn wallpaper_dynamic_list(app: AppHandle) -> Vec<WallpaperEntry> {
    let cfg = load_cfg(&app);
    let dir = wallpaper_dynamic::pick_dynamic_dir(cfg.wallpaper_dynamic_dir, cfg.wallpaper_dir);
    wallpaper_dynamic::scan_dynamic_dir(&dir)
}

/// 应用动态壁纸素材:
/// - 图片(jpg/jpeg/png/bmp)→ 复用 2.4 系统壁纸(webp/gif 由 2.4 设置白名单拒绝并提示);
/// - mp4/html → 记录素材到内存 + 注入 WorkerW 壁纸层;
/// - 同路径重复 set 幂等(不重复注入,直接返回当前记录)。
#[tauri::command(rename = "wallpaper_dynamic_set")]
pub fn wallpaper_dynamic_set(app: AppHandle, path: String) -> Result<WallpaperSetInfo, String> {
    wallpaper_dynamic::validate_set_args(&path)?;
    let kind = wallpaper_dynamic::material_kind(&path);
    match kind {
        "image" => {
            // 若当前已注入视频/html,先撤下(否则视频层会盖住新壁纸)
            detach_if_attached(&app)?;
            // 复用 2.4:SPI_SETDESKWALLPAPER 设系统壁纸(不占 WorkerW)
            crate::commands::wallpaper::wallpaper_set_static_cmd(path.clone())?;
            wallpaper_dynamic::set_state(Some(WallpaperState {
                path: path.clone(),
                kind: "image".to_string(),
                url: None,
            }));
            Ok(WallpaperSetInfo { path, url: None })
        }
        "video" | "html" => {
            let cfg = load_cfg(&app);
            if !cfg.enable_dynamic_wallpaper {
                return Err("动态壁纸未启用,请在设置中开启后使用".to_string());
            }
            // 电池检测线程惰性自启兜底(集成 agent 在 setup 接线后此处为幂等 no-op)
            wallpaper_dynamic::spawn_battery_watcher(app.clone(), &cfg);
            // 幂等:同路径重复 set 无副作用(不重复注入)
            if let Some(cur) = wallpaper_dynamic::current_state() {
                if (cur.kind == "video" || cur.kind == "html")
                    && cur.path.eq_ignore_ascii_case(&path)
                {
                    return Ok(WallpaperSetInfo { path, url: cur.url });
                }
            }
            // 切换素材:先撤下旧的注入再挂新的
            detach_if_attached(&app)?;
            // 两段式 URL:Pictures 内 → None(前端 convertFileSrc);目录外 → data URL(≤50MB)
            let url = wallpaper_dynamic::resolve_material_url(
                &path,
                &wallpaper_dynamic::default_pictures_dir(),
            )?;
            let state = WallpaperState {
                path: path.clone(),
                kind: kind.to_string(),
                url: url.clone(),
            };
            attach(&app, &state)?;
            wallpaper_dynamic::set_state(Some(state));
            Ok(WallpaperSetInfo { path, url })
        }
        _ => Err(format!("不支持的素材格式: {path}")), // validate_set_args 已拦截,防御性兜底
    }
}

/// 恢复系统壁纸:撤下 WorkerW 注入 + 清内存记录。
/// 图片走系统壁纸本就无需"恢复";视频/html 撤下后系统壁纸自然可见,不碰 SystemParametersInfoW
#[tauri::command(rename = "wallpaper_dynamic_clear")]
pub fn wallpaper_dynamic_clear(app: AppHandle) -> Result<(), String> {
    let cur = wallpaper_dynamic::current_state();
    if let Some(st) = &cur {
        if st.kind == "video" || st.kind == "html" {
            detach(&app)?;
        }
    }
    wallpaper_dynamic::set_state(None);
    Ok(())
}

/// 查询动态壁纸状态(设置区状态徽标/壁纸窗口初始渲染用)
#[tauri::command(rename = "wallpaper_dynamic_get_state")]
pub fn wallpaper_dynamic_get_state(app: AppHandle) -> DynamicWallpaperState {
    let cfg = load_cfg(&app);
    let (kind, path, url) = match wallpaper_dynamic::current_state() {
        Some(st) => (st.kind, Some(st.path), st.url),
        None => ("none".to_string(), None, None),
    };
    DynamicWallpaperState {
        enabled: cfg.enable_dynamic_wallpaper,
        kind,
        path,
        url,
        on_battery: wallpaper_dynamic::battery_latest(),
        downshift_active: cfg.enable_dynamic_wallpaper && cfg.wallpaper_battery_downshift,
    }
}

// ---------------------------------------------------------------------------
// 内部:窗口注入/撤下(WorkerW 三连的具体 Win32 调用在实现层 wallpaper_dynamic)
// ---------------------------------------------------------------------------

/// 当前素材若占用了 WorkerW(视频/html),撤下
fn detach_if_attached(app: &AppHandle) -> Result<(), String> {
    if let Some(st) = wallpaper_dynamic::current_state() {
        if st.kind == "video" || st.kind == "html" {
            detach(app)?;
        }
    }
    Ok(())
}

/// 从 WorkerW 撤下并隐藏壁纸窗口
fn detach(app: &AppHandle) -> Result<(), String> {
    let Some(win) = app.get_webview_window("wallpaper") else {
        return Err("壁纸窗口不存在".to_string());
    };
    if let Ok(hwnd) = win.hwnd() {
        // tauri::HWND(hwnd.0) 即 windows-sys HWND(*mut c_void),直接转换(hotkey.rs 同款);
        // detach_from_workerw 为安全封装(内部逐 unsafe 调用)
        wallpaper_dynamic::detach_from_workerw(hwnd.0 as *mut core::ffi::c_void);
    }
    let _ = win.hide();
    Ok(())
}

/// 把壁纸窗口注入 WorkerW:先按主屏尺寸 set_size + show + 关置顶,再 SetParent(设计 §1.3)
fn attach(app: &AppHandle, _st: &WallpaperState) -> Result<(), String> {
    let Some(win) = app.get_webview_window("wallpaper") else {
        return Err("壁纸窗口不存在".to_string());
    };
    // 尺寸:主屏物理像素(100%/125% 缩放时仍铺满,每次 set 重算)
    let (w, h) = match app.primary_monitor() {
        Ok(Some(mon)) => {
            let size = mon.size();
            (size.width as i32, size.height as i32)
        }
        _ => (1920, 1080),
    };
    let _ = win.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
    // SetParent 前:先显示再注入;关闭置顶(防 WorkerW 层被顶穿)
    let _ = win.show();
    let _ = win.set_always_on_top(false);
    let hwnd = win.hwnd().map_err(|e| format!("获取壁纸窗口句柄失败: {e}"))?;
    // attach_to_workerw 为安全封装(内部逐 unsafe 调用)
    wallpaper_dynamic::attach_to_workerw(hwnd.0 as *mut core::ffi::c_void, w, h)
}
