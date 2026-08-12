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
    pub monitors: Vec<PerMonitorState>, // Phase5 多屏逐屏状态;多屏关 = 空数组
}

/// 多屏逐屏状态(拼接模式:素材相同时每屏各一条;独立模式:各屏各自的素材)
#[derive(Clone, Debug, Serialize)]
pub struct PerMonitorState {
    pub index: u32,
    pub kind: String, // "none" | "video" | "html"
    pub path: Option<String>,
    pub url: Option<String>,
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
            // ---- Phase5 多屏分发(设计文档 §2.3):拼接 → 全屏同一素材;独立 → 只设主屏 ----
            if cfg.wallpaper_multi_monitor {
                let url = wallpaper_dynamic::resolve_material_url(
                    &path,
                    &wallpaper_dynamic::default_pictures_dir(),
                )?;
                let state = WallpaperState {
                    path: path.clone(),
                    kind: kind.to_string(),
                    url: url.clone(),
                };
                let mons = crate::wallpaper_dynamic::enum_monitors(&app);
                for m in &mons {
                    if cfg.wallpaper_span_mode || m.index == 0 {
                        wallpaper_dynamic::set_monitor_state(m.index, state.clone())?;
                    }
                }
                crate::wallpaper_dynamic::multi_apply(&app)?;
                // 同步单值状态(get_state 主屏聚合保持一致)
                wallpaper_dynamic::set_state(Some(state));
                return Ok(WallpaperSetInfo { path, url });
            }
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
    let cfg = load_cfg(&app);
    // Phase5 多屏:清全部屏素材 + 重建(无素材屏自动撤下注入)
    if cfg.wallpaper_multi_monitor {
        wallpaper_dynamic::clear_monitor_states();
        wallpaper_dynamic::set_state(None);
        return crate::wallpaper_dynamic::multi_apply(&app);
    }
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
    // Phase5 多屏:逐屏状态(实际屏数截断,防热插拔缩屏后残留脏数据;多屏关 = 空数组)
    let monitors = if cfg.wallpaper_multi_monitor {
        let states = wallpaper_dynamic::monitor_states();
        crate::wallpaper_dynamic::enum_monitors(&app)
            .into_iter()
            .map(|m| {
                match states.get(m.index as usize) {
                    Some(Some(st)) => PerMonitorState {
                        index: m.index,
                        kind: st.kind.clone(),
                        path: Some(st.path.clone()),
                        url: st.url.clone(),
                    },
                    _ => PerMonitorState {
                        index: m.index,
                        kind: "none".to_string(),
                        path: None,
                        url: None,
                    },
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    DynamicWallpaperState {
        enabled: cfg.enable_dynamic_wallpaper,
        kind,
        path,
        url,
        on_battery: wallpaper_dynamic::battery_latest(),
        downshift_active: cfg.enable_dynamic_wallpaper && cfg.wallpaper_battery_downshift,
        monitors,
    }
}

/// 热生效入口(config_save 后调用,runtime::apply 接线):
/// enable_dynamic_wallpaper 关闭 → 撤下已注入的动态壁纸,无需重启:
/// - 多屏模式:清全部屏状态 + 重建(无素材屏自动撤下注入,与 clear 命令同款);
/// - 单屏:video/html 撤 WorkerW 注入 + 清内存记录(图片走系统壁纸 API,无需撤)。
/// 开启 → 无操作(素材由 set 命令显式设置,开关只做清理门控)。
/// 幂等;任何单项失败只告警,不阻断保存。
pub fn apply_config(app: &AppHandle, cfg: &crate::commands::config::AppConfig) {
    if cfg.enable_dynamic_wallpaper {
        return;
    }
    if cfg.wallpaper_multi_monitor {
        wallpaper_dynamic::clear_monitor_states();
        wallpaper_dynamic::set_state(None);
        let _ = crate::wallpaper_dynamic::multi_apply(app);
        return;
    }
    if let Some(st) = wallpaper_dynamic::current_state() {
        if st.kind == "video" || st.kind == "html" {
            let _ = detach(app);
        }
        wallpaper_dynamic::set_state(None);
    }
}

// ---------------------------------------------------------------------------
// Phase5 5.2 多屏(设计文档 §2.3):三命令 + set/clear 按模式分发
// ---------------------------------------------------------------------------

/// 每屏窗口 label 约定:主屏沿用 "wallpaper",副屏 "wallpaper_<index>"(前端按 label 分流)
pub fn monitor_window_label(index: u32) -> String {
    if index == 0 {
        "wallpaper".to_string()
    } else {
        format!("wallpaper_{index}")
    }
}

/// 枚举显示器(设置区展示 + 前端 span 切片计算)
#[tauri::command(rename = "wallpaper_multi_monitors")]
pub fn wallpaper_multi_monitors(app: AppHandle) -> Vec<crate::wallpaper_dynamic::MonitorInfo> {
    crate::wallpaper_dynamic::enum_monitors(&app)
}

/// 按当前配置重建多屏 attach(开关/模式/素材变更后调用;热插拔检测线程也走这里)
#[tauri::command(rename = "wallpaper_multi_apply")]
pub fn wallpaper_multi_apply(app: AppHandle) -> Result<(), String> {
    crate::wallpaper_dynamic::multi_apply(&app)
}

/// 独立模式:只给指定屏设置素材(越界/拼接模式/多屏未启用报错)
#[tauri::command(rename = "wallpaper_dynamic_set_monitor")]
pub fn wallpaper_dynamic_set_monitor(
    app: AppHandle,
    path: String,
    index: u32,
) -> Result<WallpaperSetInfo, String> {
    let cfg = load_cfg(&app);
    if !cfg.wallpaper_multi_monitor {
        return Err("多屏壁纸未启用,请在设置中开启".to_string());
    }
    if cfg.wallpaper_span_mode {
        return Err("拼接模式下不支持单独设置某屏,请切换为独立模式".to_string());
    }
    let mons = crate::wallpaper_dynamic::enum_monitors(&app);
    if !mons.iter().any(|m| m.index == index) {
        return Err(format!("显示器 {index} 不存在(当前共 {} 台)", mons.len()));
    }
    crate::wallpaper_dynamic::validate_set_args(&path)?;
    let kind = crate::wallpaper_dynamic::material_kind(&path);
    if kind == "image" {
        return Err("图片素材走系统壁纸,不支持按屏设置,请使用视频/html".to_string());
    }
    let url = crate::wallpaper_dynamic::resolve_material_url(
        &path,
        &crate::wallpaper_dynamic::default_pictures_dir(),
    )?;
    let state = WallpaperState {
        path: path.clone(),
        kind: kind.to_string(),
        url: url.clone(),
    };
    crate::wallpaper_dynamic::set_monitor_state(index, state)?;
    crate::wallpaper_dynamic::apply_monitor(&app, index)?;
    Ok(WallpaperSetInfo { path, url })
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
