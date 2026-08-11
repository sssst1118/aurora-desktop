//! 2.4 静态壁纸切换(Windows 专用)
//!
//! - 设置:`SystemParametersInfoW(SPI_SETDESKWALLPAPER)`,零依赖、普通权限可用;
//! - 列表:只扫配置目录(默认 `%USERPROFILE%\Pictures`),禁止全盘扫描;
//! - 当前壁纸:设计文档 §4.2 要求读 `HKCU\Control Panel\Desktop\WallPaper`,实现走
//!   `SPI_GETDESKWALLPAPER`(内部读取的就是该注册表值)——原因是读注册表需要
//!   windows-sys 的 `Win32_System_Registry` feature,该 feature 当前未在 Cargo.toml
//!   启用(属集成 agent 共享文件),而 `Win32_UI_WindowsAndMessaging` 已启用;
//!   `SHRegGetValueW` 虽在 Shell 模块,但同样被 `Win32_System_Registry` 门控。
//! - 关键坑:SPI_SETDESKWALLPAPER 的 pvParam 必须是 **UTF-16 + 末尾 NUL 的绝对路径**
//!   指针,传 Rust `&str` 胖指针会得到黑色壁纸;相对路径静默失败。
//! - 函数名带 `_cmd` 后缀 + `#[tauri::command(rename = "xxx")]`:stubs.rs 中同名
//!   `#[tauri::command]` pub 函数的宏会被 `#[macro_export]` 导出到 crate root,
//!   两处同名直接 E0428 冲突;外部命令名(前端 invoke 名)与签名均与 stubs 占位一致。
//!   集成 agent 切换 invoke_handler 时用 `commands::wallpaper::wallpaper_set_static_cmd`;
//!   删除 stubs.rs 对应项后,可将函数名改回原名并去掉 rename(可选)。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE,
    SPIF_UPDATEINIFILE,
};

/// 壁纸条目(前端列表/预览用;size 为模块任务要求的附加字段,设计文档只列 name/path 契约)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WallpaperEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
}

/// 设置壁纸允许的扩展名(SPI_SETDESKWALLPAPER 官方支持位图类格式;webp 不可靠,不放入设置白名单)
const SET_EXT_WHITELIST: [&str; 4] = ["jpg", "jpeg", "png", "bmp"];

/// 列表展示允许的扩展名(webp 可预览;若用户点击设置 webp,后端按设置白名单拒绝并提示)
const LIST_EXT_WHITELIST: [&str; 5] = ["jpg", "jpeg", "png", "bmp", "webp"];

/// 列表展示上限
const MAX_LIST: usize = 200;

/// 扩展名是否命中白名单(大小写不敏感)
fn ext_whitelisted(name: &str, whitelist: &[&str]) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| whitelist.iter().any(|w| e.eq_ignore_ascii_case(w)))
}

/// 路径转 UTF-16 且末尾补 NUL(SystemParametersInfoW 的 pvParam 要求)
fn utf16_nul(path: &str) -> Vec<u16> {
    path.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 校验待设置壁纸:绝对路径 + 扩展名白名单 + 文件存在(纯函数,可单测)
fn validate_set_args(file_path: &str) -> Result<(), String> {
    if file_path.trim().is_empty() {
        return Err("壁纸路径为空".to_string());
    }
    let p = Path::new(file_path);
    if !p.is_absolute() {
        return Err(format!("壁纸路径必须是绝对路径: {file_path}"));
    }
    if !ext_whitelisted(file_path, &SET_EXT_WHITELIST) {
        return Err(format!("不支持的图片格式,仅支持 jpg/jpeg/png/bmp: {file_path}"));
    }
    if !p.is_file() {
        return Err(format!("壁纸文件不存在: {file_path}"));
    }
    Ok(())
}

/// 壁纸目录选择:配置 wallpaper_dir 非空用配置值,否则默认 `%USERPROFILE%\Pictures`(纯函数,可单测)
fn pick_dir(configured: Option<String>) -> PathBuf {
    match configured.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => default_pictures_dir(),
    }
}

fn default_pictures_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|u| Path::new(&u).join("Pictures"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public\Pictures"))
}

/// 从 `%APPDATA%\com.aurora.desktop\config.json` 读 wallpaper_dir(与 config.rs 同一落盘路径)
fn configured_wallpaper_dir() -> Option<String> {
    let appdata = std::env::var_os("APPDATA")?;
    let cfg_path = Path::new(&appdata).join("com.aurora.desktop").join("config.json");
    super::config::load_from(&cfg_path).wallpaper_dir
}

/// 扫描目录:过滤图片扩展名 + 隐藏文件 + 常规文件,按名称排序截断上限(纯函数,可单测)
fn scan_dir(dir: &Path) -> Vec<WallpaperEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out; // 目录不存在/无权限 → 空列表(前端显示错误提示)
    };
    for item in rd.flatten() {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = item.file_name().to_str().map(String::from) else {
            continue; // 非 UTF-8 文件名跳过
        };
        if name.starts_with('.') {
            continue; // 隐藏文件
        }
        if !ext_whitelisted(&name, &LIST_EXT_WHITELIST) {
            continue;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        out.push(WallpaperEntry { name, path: path.to_string_lossy().into_owned(), size });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.truncate(MAX_LIST);
    out
}

/// 设置静态壁纸;立即生效并持久化(SPIF_UPDATEINIFILE|SPIF_SENDCHANGE,系统注册表持久化)
#[tauri::command(rename = "wallpaper_set_static")]
pub fn wallpaper_set_static_cmd(file_path: String) -> Result<(), String> {
    validate_set_args(&file_path)?;
    // UTF-16 + 末尾 NUL 的绝对路径;传 Rust &str(胖指针)会得到黑色壁纸
    let wide = utf16_nul(&file_path);
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            wide.as_ptr() as *const core::ffi::c_void as *mut core::ffi::c_void,
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        Err(format!("设置壁纸失败(系统错误码 {code})"))
    } else {
        Ok(())
    }
}

/// 列出壁纸目录图片(只扫配置目录,默认 `%USERPROFILE%\Pictures`,不递归不扫全盘)
#[tauri::command(rename = "wallpaper_list_local")]
pub fn wallpaper_list_local_cmd() -> Vec<WallpaperEntry> {
    let dir = pick_dir(configured_wallpaper_dir());
    scan_dir(&dir)
}

/// 读取当前壁纸路径(SPI_GETDESKWALLPAPER;底层即 HKCU\Control Panel\Desktop\WallPaper)
#[tauri::command(rename = "wallpaper_get_current")]
pub fn wallpaper_get_current_cmd() -> Option<String> {
    let mut buf = [0u16; 1024];
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return None; // 系统主题壁纸时注册表值可能为空
    }
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("aurora_wallpaper_{tag}_{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn touch(p: &Path) {
        std::fs::write(p, b"fake image bytes").unwrap();
    }

    // ---- 列表扫描 ----

    #[test]
    fn scan_filters_to_image_exts_sorted() {
        let dir = tmp_dir("scan");
        touch(&dir.join("b.jpg"));
        touch(&dir.join("A.PNG")); // 大写扩展名也要识别
        touch(&dir.join("c.bmp"));
        touch(&dir.join("d.webp")); // webp 允许预览
        touch(&dir.join("note.txt")); // 非图片
        touch(&dir.join("e.gif")); // 非白名单图片格式
        touch(&dir.join(".hidden.png")); // 隐藏文件
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        touch(&dir.join("subdir/inside.jpg")); // 不递归子目录

        let list = scan_dir(&dir);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["A.PNG", "b.jpg", "c.bmp", "d.webp"]); // 大小写不敏感字典序
        assert!(list.iter().all(|e| e.size > 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_missing_dir_returns_empty() {
        let dir = tmp_dir("missing");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(scan_dir(&dir).is_empty());
    }

    #[test]
    fn scan_truncates_to_200() {
        let dir = tmp_dir("cap");
        for i in 0..205 {
            touch(&dir.join(format!("img_{i:03}.png")));
        }
        let list = scan_dir(&dir);
        assert_eq!(list.len(), MAX_LIST);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 设置校验 ----

    #[test]
    fn set_rejects_relative_path() {
        assert!(validate_set_args(r"Pictures\a.jpg").is_err());
    }

    #[test]
    fn set_rejects_missing_file() {
        let dir = tmp_dir("missing_set");
        let p = dir.join("none.jpg");
        assert!(validate_set_args(&p.to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_rejects_unsupported_ext() {
        let dir = tmp_dir("badext");
        touch(&dir.join("a.webp")); // 可预览但不可设置
        touch(&dir.join("b.txt"));
        assert!(validate_set_args(&dir.join("a.webp").to_string_lossy()).is_err());
        assert!(validate_set_args(&dir.join("b.txt").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_accepts_absolute_image() {
        let dir = tmp_dir("good");
        touch(&dir.join("a.jpg"));
        touch(&dir.join("b.JPEG")); // 大小写不敏感
        assert!(validate_set_args(&dir.join("a.jpg").to_string_lossy()).is_ok());
        assert!(validate_set_args(&dir.join("b.JPEG").to_string_lossy()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- UTF-16 编码(黑屏坑的保护) ----

    #[test]
    fn utf16_nul_is_nul_terminated() {
        let wide = utf16_nul(r"C:\Users\me\Pictures\a.jpg");
        assert_eq!(*wide.last().unwrap(), 0);
        assert_eq!(wide.len(), r"C:\Users\me\Pictures\a.jpg".encode_utf16().count() + 1);
        let s = String::from_utf16_lossy(&wide[..wide.len() - 1]);
        assert_eq!(s, r"C:\Users\me\Pictures\a.jpg");
    }

    // ---- 目录选择 ----

    #[test]
    fn pick_dir_uses_configured_when_nonempty() {
        assert_eq!(pick_dir(Some(r"D:\wp".to_string())), PathBuf::from(r"D:\wp"));
        assert_eq!(pick_dir(Some("  ".to_string())).to_string_lossy(), default_pictures_dir().to_string_lossy());
        assert_eq!(pick_dir(None).to_string_lossy(), default_pictures_dir().to_string_lossy());
    }
}
