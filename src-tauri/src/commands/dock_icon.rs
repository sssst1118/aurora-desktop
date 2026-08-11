//! Dock 图标提取与双缓存(Phase2 2.1 模块)。
//!
//! 管线:ExtractIconExW → GetIconInfo → GetDIBits → BGRA→RGBA → png 编码 → base64 data URL。
//! 缓存:内存 HashMap(路径→data URL)+ 磁盘 `%APPDATA%\com.aurora.desktop\icons\{fnv1a:016x}.png`。
//!
//! 说明:当前 Cargo.toml 未启用 windows-sys 的 `Win32_Graphics_Gdi` feature(GetDIBits /
//! GetIconInfo / ICONINFO 均被 cfg 隐藏),而 Cargo.toml 属集成 agent 所有,本模块
//! 以手写 FFI 声明补齐,不依赖 Cargo.toml 变更。GDI 调用极轻量(一次性提取),无句柄泄漏:
//! HICON 用 DestroyIcon 释放,HBITMAP 用 DeleteObject 释放,DC 用 DeleteDC 释放。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::UI::Shell::ExtractIconExW;
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

// ---- 手写 FFI:windows-sys 未启用 Win32_Graphics_Gdi / 相关 user32 符号 ----

/// GDI 句柄与函数(user32.dll 中 GetIconInfo 的 ICONINFO 引用了 GDI 类型,一并手写)
#[repr(C)]
struct IconInfo {
    f_icon: i32,      // BOOL
    x_hotspot: u32,
    y_hotspot: u32,
    hbm_mask: *mut core::ffi::c_void,
    hbm_color: *mut core::ffi::c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BitmapInfoHeader {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

/// BITMAPINFO:40 字节头 + 调色板(查询位深时驱动会写回颜色表,保留 256 项防越界)
#[repr(C)]
#[derive(Clone, Copy)]
struct BitmapInfo {
    bmi_header: BitmapInfoHeader,
    bmi_colors: [u32; 256],
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetIconInfo(hicon: HICON, piconinfo: *mut IconInfo) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(hdc: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn DeleteDC(hdc: *mut core::ffi::c_void) -> i32;
    fn DeleteObject(h: *mut core::ffi::c_void) -> i32;
    fn GetDIBits(
        hdc: *mut core::ffi::c_void,
        hbm: *mut core::ffi::c_void,
        start: u32,
        lines: u32,
        lpvbits: *mut core::ffi::c_void,
        lpbmi: *mut BitmapInfo,
        usage: u32,
    ) -> i32;
}

const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;

// ---- 提取管线 ----

/// 从文件(exe/lnk/ico 等)提取图标像素,返回 (宽, 高, RGBA)
pub fn extract_icon_pixels(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut hicon_large: HICON = std::ptr::null_mut();
        let mut hicon_small: HICON = std::ptr::null_mut();
        // 成功时返回提取到的图标数;优先大图标,释放未被使用的那个
        if ExtractIconExW(wide.as_ptr(), 0, &mut hicon_large, &mut hicon_small, 1) == 0 {
            return None;
        }
        let hicon = if !hicon_large.is_null() { hicon_large } else { hicon_small };
        let spare = if hicon == hicon_large { hicon_small } else { hicon_large };
        if !spare.is_null() {
            DestroyIcon(spare);
        }
        if hicon.is_null() {
            return None;
        }
        let result = pixels_from_icon(hicon);
        DestroyIcon(hicon);
        result
    }
}

fn pixels_from_icon(hicon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut info = IconInfo {
            f_icon: 0,
            x_hotspot: 0,
            y_hotspot: 0,
            hbm_mask: std::ptr::null_mut(),
            hbm_color: std::ptr::null_mut(),
        };
        if GetIconInfo(hicon, &mut info) == 0 {
            return None;
        }
        if info.hbm_color.is_null() {
            // 纯掩码图标(罕见):无颜色位图,回退占位
            if !info.hbm_mask.is_null() {
                DeleteObject(info.hbm_mask);
            }
            return None;
        }
        let result = dibs_pixels(info.hbm_color);
        DeleteObject(info.hbm_color);
        if !info.hbm_mask.is_null() {
            DeleteObject(info.hbm_mask);
        }
        result
    }
}

/// GetDIBits 两次调用:先查询尺寸/位深,再按 32bpp BGRA 取像素
fn dibs_pixels(hbm: *mut core::ffi::c_void) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let hdc = CreateCompatibleDC(std::ptr::null_mut());
        if hdc.is_null() {
            return None;
        }
        let mut bi = BitmapInfo {
            bmi_header: BitmapInfoHeader {
                bi_size: 40,
                bi_width: 0,
                bi_height: 0,
                bi_planes: 1,
                bi_bit_count: 0,
                bi_compression: 0,
                bi_size_image: 0,
                bi_x_pels_per_meter: 0,
                bi_y_pels_per_meter: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            },
            bmi_colors: [0; 256],
        };
        // 第一次:只查元数据(bits=NULL)
        if GetDIBits(hdc, hbm, 0, 0, std::ptr::null_mut(), &mut bi, DIB_RGB_COLORS) == 0 {
            DeleteDC(hdc);
            return None;
        }
        let w = bi.bmi_header.bi_width;
        let ah = bi.bmi_header.bi_height;
        if w <= 0 || ah == 0 {
            DeleteDC(hdc);
            return None;
        }
        let top_down = ah < 0; // 负高度 = 自上而下;图标位图通常是 top-down
        let w = w as u32;
        let h = ah.unsigned_abs();
        // 第二次:32bpp BGRA
        bi.bmi_header.bi_bit_count = 32;
        bi.bmi_header.bi_compression = BI_RGB;
        bi.bmi_header.bi_size_image = 0;
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        if GetDIBits(
            hdc,
            hbm,
            0,
            h,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            &mut bi,
            DIB_RGB_COLORS,
        ) == 0
        {
            DeleteDC(hdc);
            return None;
        }
        DeleteDC(hdc);
        // BGRA → RGBA;bottom-up(正高度)时逐行翻转
        let row = w as usize * 4;
        let mut rgba = Vec::with_capacity(buf.len());
        for y in 0..h as usize {
            let src_y = if top_down { y } else { h as usize - 1 - y };
            for px in buf[src_y * row..(src_y + 1) * row].chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        Some((w, h, rgba))
    }
}

// ---- base64(RFC 4648,自实现避免动 Cargo.toml 加依赖)----

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_TABLE[(n >> 18) as usize & 63] as char);
        out.push(B64_TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64_TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64_TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// FNV-1a 64 位哈希(磁盘缓存文件名,跨版本稳定)
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---- png 编码与双缓存 ----

/// RGBA 像素编码为 PNG 字节
pub fn encode_png_rgba(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if rgba.len() != w as usize * h as usize * 4 {
        return None;
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

static MEM_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn mem_cache() -> &'static Mutex<HashMap<String, String>> {
    MEM_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn data_url_from_png(png_bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", b64_encode(png_bytes))
}

/// 取图标 data URL:内存缓存 → 磁盘缓存 → 重新提取并落盘。
/// 提取失败返回 None(前端回退占位图标+名称)。
pub fn icon_data_url(path: &str, icons_dir: &Path) -> Option<String> {
    // 1. 内存缓存
    if let Ok(g) = mem_cache().lock() {
        if let Some(url) = g.get(path) {
            return Some(url.clone());
        }
    }
    // 2. 磁盘缓存
    let key = format!("{:016x}", fnv1a(path));
    let file = icons_dir.join(format!("{key}.png"));
    let data_url = match std::fs::read(&file).ok() {
        Some(bytes) => data_url_from_png(&bytes),
        None => {
            let (w, h, rgba) = extract_icon_pixels(path)?;
            let png_bytes = encode_png_rgba(w, h, &rgba)?;
            let _ = std::fs::create_dir_all(icons_dir);
            let _ = std::fs::write(&file, &png_bytes);
            data_url_from_png(&png_bytes)
        }
    };
    // 3. 回填内存缓存
    if let Ok(mut g) = mem_cache().lock() {
        g.insert(path.to_string(), data_url.clone());
    }
    Some(data_url)
}

/// 清空内存缓存(测试用;磁盘缓存保留,下一次命中自动回填)
pub fn clear_memory_cache() {
    if let Ok(mut g) = mem_cache().lock() {
        g.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_standard_vectors() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(b64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn fnv1a_standard_vectors() {
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn png_roundtrip() {
        let (w, h) = (2u32, 1u32);
        let rgba = [255, 0, 0, 255, 0, 0, 255, 128];
        let png = encode_png_rgba(w, h, &rgba).expect("编码应成功");
        // 用 png 解码器验证结构与像素
        let mut dec = png::Decoder::new(&png[..]);
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().expect("解码信息应成功");
        let info = reader.info();
        assert_eq!(info.width, w);
        assert_eq!(info.height, h);
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buf).expect("解码帧应成功");
        assert_eq!(&buf[..frame.buffer_size()], &rgba);
    }

    #[test]
    fn extract_icon_from_real_exe() {
        // 环境无该文件(理论不会)时跳过,避免 CI 上假失败
        let np = "C:\\Windows\\System32\\notepad.exe";
        if !std::path::Path::new(np).exists() {
            return;
        }
        let (w, h, rgba) = extract_icon_pixels(np).expect("notepad 图标应可提取");
        assert!(w > 0 && h > 0);
        assert_eq!(rgba.len(), w as usize * h as usize * 4);
    }

    #[test]
    fn extract_icon_garbage_path_returns_none() {
        assert!(extract_icon_pixels(r"C:\definitely\not\exists_aurora.exe").is_none());
    }

    #[test]
    fn icon_cache_memory_then_disk() {
        let np = "C:\\Windows\\System32\\notepad.exe";
        if !std::path::Path::new(np).exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("aurora_dock_icon_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let url1 = icon_data_url(np, &dir).expect("图标 data URL 应生成");
        assert!(url1.starts_with("data:image/png;base64,"));
        // 磁盘缓存文件应已落盘
        let key = format!("{:016x}", fnv1a(np));
        assert!(dir.join(format!("{key}.png")).exists(), "磁盘缓存文件应存在");

        // 清空内存缓存 → 再次调用应命中磁盘缓存,且结果一致
        clear_memory_cache();
        let url2 = icon_data_url(np, &dir).expect("磁盘缓存应命中");
        assert_eq!(url1, url2);

        // 内存缓存命中
        let url3 = icon_data_url(np, &dir).expect("内存缓存应命中");
        assert_eq!(url1, url3);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
