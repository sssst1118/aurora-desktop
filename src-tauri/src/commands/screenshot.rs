//! 截图功能(2026-08-18,设计 docs/截图功能-设计.md;零新依赖)
//!
//! - `screenshot_begin`:热键触发,为每个显示器创建/复用全屏透明遮罩窗口(label =
//!   `capture-{i}`),前端 CaptureView.vue 渲染变暗遮罩 + 十字光标 + 拖选矩形;
//!   松开后前端先隐藏遮罩窗(防遮罩入图),再 invoke 截图命令。
//! - `screenshot_capture`:整虚拟桌面一次 BitBlt(GetDC(NULL) + CreateDIBSection
//!   32bpp top-down + BitBlt SRCCOPY)→ 按选区(虚拟桌面物理坐标)裁剪 → BGRA→RGBA
//!   → png 保存到 `图片\Aurora 截图\` → CF_DIB 写剪贴板 → 返回 {path, w, h, copy_ok}。
//!
//! 坐标约定(设计 §②):前端把「窗口内逻辑坐标 × scaleFactor + 窗口物理位置」换算成
//! 虚拟桌面物理坐标传给后端,与 BitBlt 位图坐标系一致,零 monitor 匹配问题;
//! 多屏下每屏一个遮罩窗,选区落在哪个屏窗口就上报哪个屏的坐标(跨屏拖选 MVP 限制)。

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
// windows-sys 0.59 符号位置(实测确认,勿凭旧版记忆):GlobalFree/SYSTEMTIME 在
// Foundation,GlobalAlloc/GlobalLock/GlobalUnlock/GMEM_MOVEABLE 在 System::Memory,
// CF_DIB 在 System::Ole(CLIPBOARD_FORMAT=u16,SetClipboardData 参数是 u32 需转)
use windows_sys::Win32::{
    Foundation::{GlobalFree, SYSTEMTIME},
    Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        SRCCOPY,
    },
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        Ole::CF_DIB,
        SystemInformation::GetLocalTime,
    },
    UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    },
};

/// 截图结果(前端 emit `screenshot-done` 给岛窗口显示 hint)
#[derive(Debug, Serialize)]
pub struct CaptureResult {
    pub path: String,
    pub w: u32,
    pub h: u32,
    /// 复制剪贴板失败但保存成功(失败分级,设计 §③)
    pub copy_ok: bool,
}

/// 热键触发截图:为每台显示器创建/复用全屏遮罩窗口并置顶抢焦点。
/// ARMED/SELECTING 状态去重:任一 capture 窗口可见 → 忽略(热键连按/长按)。
/// ⚠️ 必须是 async:同步命令在主线程执行,WebviewWindowBuilder::build() 需要主线程
/// 完成窗口创建,同步调用自死锁(实测 2026-08-18:sync 版挂死全部 IPC)。
#[tauri::command(rename = "screenshot_begin")]
pub async fn screenshot_begin(app: AppHandle) -> Result<u32, String> {
    let visible = (0..16).any(|i| {
        app.get_webview_window(&format!("capture-{i}"))
            .is_some_and(|w| w.is_visible().unwrap_or(false))
    });
    if visible {
        return Ok(0); // 已在选区流程中
    }
    let mons = crate::wallpaper_dynamic::enum_monitors(&app);
    for (i, m) in mons.iter().enumerate() {
        let label = format!("capture-{i}");
        // 复用已创建窗口(截完只隐藏不销毁,与壁纸窗口常驻模式一致;应用退出时随窗口管理清理)
        let win = if let Some(w) = app.get_webview_window(&label) {
            w
        } else {
            WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
                .decorations(false)
                .transparent(true) // 变暗效果由前端半透明黑渲染,窗口本身透明
                .resizable(false)
                .skip_taskbar(true)
                .visible(false)
                .focused(false)
                .build()
                .map_err(|e| format!("创建截图遮罩窗口 {label} 失败: {e}"))?
        };
        let _ = win.set_size(PhysicalSize::new(m.width as u32, m.height as u32));
        let _ = win.set_position(PhysicalPosition::new(m.x, m.y));
        let _ = win.set_always_on_top(true); // 截图遮罩必须盖住一切(含置顶窗口)
        let _ = win.show();
        let _ = win.set_focus(); // 抢焦点收 Esc/拖选(临时模态,区别于岛/壁纸常驻不抢)
    }
    Ok(mons.len() as u32)
}

/// 执行截图:整虚拟桌面 BitBlt → 按虚拟桌面物理坐标选区裁剪 → 复制 + 保存。
#[tauri::command(rename = "screenshot_capture")]
pub fn screenshot_capture(x: i32, y: i32, w: u32, h: u32) -> Result<CaptureResult, String> {
    if w == 0 || h == 0 || w > 32767 || h > 32767 {
        return Err("选区尺寸不合法".into());
    }
    let (bgra, vw, vh) = capture_virtual_desktop()?;

    // 钳制选区到虚拟桌面内(拔屏/跨屏等极端场景前端坐标可能越界)
    let x0 = x.clamp(0, vw);
    let y0 = y.clamp(0, vh);
    let x1 = x.saturating_add(w as i32).clamp(0, vw);
    let y1 = y.saturating_add(h as i32).clamp(0, vh);
    if x1 <= x0 || y1 <= y0 {
        return Err("选区在虚拟桌面之外".into());
    }
    let cw = (x1 - x0) as usize;
    let ch = (y1 - y0) as usize;

    // 裁剪 + 双份像素:CF_DIB 要 BGRA **bottom-up 行序**,png 要 RGBA(一次循环完成)。
    // ⚠️ CF_DIB 负高度(top-down)实测(2026-08-18,GetImage 对照实验)系统解码失败
    // 返回 null;必须正 biHeight + 行翻转,微信/画图/PPT 才能粘贴。
    let mut dib = vec![0u8; cw * ch * 4];
    let mut rgba = Vec::with_capacity(cw * ch * 4);
    let row = vw as usize * 4;
    let line_w = cw * 4;
    for i in 0..ch {
        let src = (y0 as usize + i) * row + x0 as usize * 4;
        let line = &bgra[src..src + line_w];
        dib[(ch - 1 - i) * line_w..(ch - i) * line_w].copy_from_slice(line); // 行翻转
        let mut j = 0;
        while j < line.len() {
            rgba.push(line[j + 2]); // B
            rgba.push(line[j + 1]); // G
            rgba.push(line[j]);     // R
            rgba.push(line[j + 3]); // A
            j += 4;
        }
    }

    // 失败分级(设计 §③):剪贴板失败不阻断保存,结果里标记 copy_ok=false
    let copy_ok = set_clipboard_dib(&dib, cw as u32, ch as u32).is_ok();
    let path = save_png(&rgba, cw as u32, ch as u32)?;

    Ok(CaptureResult {
        path: path.to_string_lossy().into_owned(),
        w: cw as u32,
        h: ch as u32,
        copy_ok,
    })
}

/// 整虚拟桌面 BitBlt 截屏 → BGRA top-down 像素(行宽 = vw*4)+ 虚拟桌面尺寸。
/// 用 GetDC(NULL) 屏幕 DC + CreateDIBSection,一次拷贝整块虚拟桌面,选区裁剪在上层做。
fn capture_virtual_desktop() -> Result<(Vec<u8>, i32, i32), String> {
    let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let vw = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let vh = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if vw <= 0 || vh <= 0 {
        return Err(format!("虚拟桌面尺寸异常({vw}x{vh})"));
    }

    let hdc = unsafe { GetDC(std::ptr::null_mut()) };
    if hdc.is_null() {
        return Err("获取屏幕 DC 失败".into());
    }
    let mem = unsafe { CreateCompatibleDC(hdc) };
    if mem.is_null() {
        unsafe { ReleaseDC(std::ptr::null_mut(), hdc) };
        return Err("创建内存 DC 失败".into());
    }

    // 32bpp top-down DIB(负 biHeight;仅内存位图用——CF_DIB 剪贴板要 bottom-up,见裁剪处)
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = vw;
    bmi.bmiHeader.biHeight = -vh;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let hbmp = unsafe { CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0) };
    if hbmp.is_null() || bits.is_null() {
        unsafe {
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), hdc);
        }
        return Err("创建 DIB 失败".into());
    }

    let old = unsafe { SelectObject(mem, hbmp) };
    // 源坐标 = 虚拟桌面原点(GetDC(NULL) 的 DC 原点在虚拟桌面左上,双屏负坐标场景正确)
    let ok = unsafe { BitBlt(mem, 0, 0, vw, vh, hdc, vx, vy, SRCCOPY) };

    let mut pixels = vec![0u8; (vw as usize) * (vh as usize) * 4];
    if ok != 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(bits as *const u8, pixels.as_mut_ptr(), pixels.len());
        }
    }
    // 清理顺序:换回旧位图后才可删 DIB(先 SelectObject 换回,再 DeleteObject/DeleteDC/ReleaseDC)
    unsafe {
        SelectObject(mem, old);
        DeleteObject(hbmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), hdc);
    }
    if ok == 0 {
        return Err("BitBlt 截屏失败".into());
    }
    Ok((pixels, vw, vh))
}

/// 把 BGRA top-down 像素写剪贴板(CF_DIB = Windows 图片剪贴板标准格式,
/// 微信/画图/PowerPoint 均可直接粘贴)。失败返回错误,由调用方决定降级。
fn set_clipboard_dib(bgra: &[u8], w: u32, h: u32) -> Result<(), String> {
    // BITMAPINFOHEADER(40B)+ 像素行(32bpp 行宽天然 4 字节对齐)
    let total = 40usize + bgra.len();
    let hmem = unsafe { GlobalAlloc(GMEM_MOVEABLE, total) };
    if hmem.is_null() {
        return Err("剪贴板内存分配失败".into());
    }
    unsafe {
        let p = GlobalLock(hmem) as *mut u8;
        if p.is_null() {
            GlobalFree(hmem);
            return Err("剪贴板内存锁定失败".into());
        }
        // BITMAPINFOHEADER 逐字段手写小端字节(Windows 结构约定;u16/u32 混合,
        // 按字段逐段写入:biSize=40 | biWidth | biHeight(top-down 负值) | biPlanes |
        // biBitCount | biCompression | biSizeImage | 4 个保留零段)
        let mut off = 0usize;
        let mut put = |v: &[u8]| {
            std::ptr::copy_nonoverlapping(v.as_ptr(), p.add(off), v.len());
            off += v.len();
        };
        put(&40u32.to_le_bytes());
        put(&(w as i32).to_le_bytes());
        put(&(h as i32).to_le_bytes()); // 正 biHeight:像素已翻转成 bottom-up(负值实测解码失败)
        put(&1u16.to_le_bytes());
        put(&32u16.to_le_bytes());
        put(&BI_RGB.to_le_bytes());
        put(&(bgra.len() as u32).to_le_bytes());
        put(&0i32.to_le_bytes());
        put(&0i32.to_le_bytes());
        put(&0u32.to_le_bytes());
        put(&0u32.to_le_bytes());
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), p.add(40), bgra.len());
        GlobalUnlock(hmem);
    }

    if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
        unsafe { GlobalFree(hmem) };
        return Err("打开剪贴板失败(可能被其他程序占用)".into());
    }
    let r = unsafe {
        EmptyClipboard();
        SetClipboardData(CF_DIB as u32, hmem) // CF_DIB 是 u16 常量,API 收 u32;成功后系统接管内存,失败必须 GlobalFree
    };
    unsafe { CloseClipboard() };
    if r.is_null() {
        unsafe { GlobalFree(hmem) };
        return Err("写入剪贴板失败".into());
    }
    Ok(())
}

/// 保存 RGBA 像素到 `图片\Aurora 截图\截图_YYYYMMDD_HHMMSS.png`
/// (known-folders 定位图片目录 + GetLocalTime 命名,零新依赖)。
fn save_png(rgba: &[u8], w: u32, h: u32) -> Result<PathBuf, String> {
    let dir = known_folders::get_known_folder_path(known_folders::KnownFolder::Pictures)
        .ok_or_else(|| "找不到图片目录(库文件夹不可用)".to_string())?
        .join("Aurora 截图");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建截图目录失败: {e}"))?;

    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    let name = format!(
        "截图_{:04}{:02}{:02}_{:02}{:02}{:02}.png",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    );
    let path = dir.join(name);

    let file = std::fs::File::create(&path).map_err(|e| format!("创建截图文件失败: {e}"))?;
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc
        .write_header()
        .map_err(|e| format!("png 编码初始化失败: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("png 写入失败: {e}"))?;
    Ok(path)
}
