//! Phase4 4.2 键鼠模拟自动化:SendInput 封装(Windows 专用)。
//!
//! - 公共契约(4.3 复用):`move_to` / `click_at` / `scroll` / `press_key` / `type_text`;
//! - 单一入口 `SendInput`(INPUT 数组一次提交,比 keybd_event/mouse_event 更稳,支持
//!   UNICODE 文本与绝对坐标);`Win32_UI_Input_KeyboardAndMouse` feature 已启用,零新依赖;
//! - 坐标:屏幕绝对坐标(与 GetCursorPos 同一坐标系),虚拟桌面关(Phase4 仅主屏);
//! - 权限边界铁律:注入被系统拒绝(常见 = UIPI 目标为 UAC 提权窗口)返回明确中文错误,
//!   不重试、不提权;
//! - 零常驻线程:全部为瞬态调用,无任何后台资源;
//! - 文本输入:`KEYEVENTF_UNICODE` 逐 UTF-16 码元注入(非 ASCII 安全,不碰剪贴板);
//! - 单测覆盖纯函数(INPUT 构造/键盘映射/modifiers 规范化/坐标归一化),注入本身
//!   不单测(手动验收覆盖,设计文档 §2.5)。
//!
//! unsafe 风格:全部 windows 调用逐调用 `unsafe {}` + 注释(参考 hotkey.rs / wallpaper.rs)。

use std::mem;
use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    MOUSEINPUT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_LEFT, VK_LWIN,
    VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// MOUSEEVENTF_ABSOLUTE 坐标系的总长(0..=65535 归一化坐标)
const NORMALIZED_AXIS: i32 = 65536;
/// 滚轮单步距离(Windows 标准 WHEEL_DELTA;delta 每单位 120)
const WHEEL_STEP: i32 = 120;

/// 注入被系统全部拒绝时的错误文案(铁律边界,设计文档 §2.2)
const UIPI_REJECTED_MSG: &str =
    "输入被系统拒绝(目标窗口可能为管理员权限,UIPI 限制),错误码 ";

// ---------------------------------------------------------------------------
// 纯函数:INPUT 构造(可单测;注入本身不单测)
// ---------------------------------------------------------------------------

/// 屏幕绝对坐标 → MOUSEEVENTF_ABSOLUTE 归一化坐标(0..=65535;纯函数,可单测)
///
/// 公式取 MSDN 经典写法 `x * 65536 / 屏幕宽`;末列像素会算出 65536,越界钳到 65535
/// (归一化坐标上限),避免事件落到虚拟屏外。
fn normalize_coord(v: i32, screen: i32) -> i32 {
    if screen <= 0 {
        return 0;
    }
    (v.saturating_mul(NORMALIZED_AXIS) / screen).clamp(0, NORMALIZED_AXIS - 1)
}

/// 屏幕绝对坐标 → 归一化坐标对(调用 GetSystemMetrics 取主屏尺寸;
/// VIRTUALDESK 但 Phase4 仅主屏,主屏原点即虚拟屏原点)
fn normalized_position(x: i32, y: i32) -> (i32, i32) {
    // unsafe: GetSystemMetrics 为 windows FFI 调用,参数为常量、无 UB
    unsafe {
        let cx = GetSystemMetrics(SM_CXSCREEN);
        let cy = GetSystemMetrics(SM_CYSCREEN);
        (normalize_coord(x, cx), normalize_coord(y, cy))
    }
}

/// 构造鼠标移动 INPUT(单条,纯函数)
fn build_move_input(x: i32, y: i32) -> INPUT {
    let (dx, dy) = normalized_position(x, y);
    let mut input = INPUT {
        r#type: INPUT_MOUSE,
        ..unsafe { mem::zeroed() }
    };
    input.Anonymous.mi = MOUSEINPUT {
        dx,
        dy,
        mouseData: 0,
        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        time: 0,
        dwExtraInfo: 0,
    };
    input
}

/// 解析鼠标按键名(纯函数,可单测):"left"|"right"|"middle",非法 → Err
fn parse_button(button: &str) -> Result<(MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS), String> {
    match button {
        "left" => Ok((MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)),
        "right" => Ok((MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)),
        "middle" => Ok((MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP)),
        _ => Err(format!("不支持的鼠标按键: {button}(仅支持 left/right/middle)")),
    }
}

/// 构造一次点击的按下+抬起 INPUT 数组(单次 SendInput 一次提交,更稳;纯函数)
fn build_click_inputs(x: i32, y: i32, button: &str) -> Result<Vec<INPUT>, String> {
    let (down_flag, up_flag) = parse_button(button)?;
    let pos = normalized_position(x, y);
    let mk = |flag| {
        let mut input = INPUT {
            r#type: INPUT_MOUSE,
            ..unsafe { mem::zeroed() }
        };
        input.Anonymous.mi = MOUSEINPUT {
            dx: pos.0,
            dy: pos.1,
            mouseData: 0,
            // 按下/抬起都带 ABSOLUTE|VIRTUALDESK:保证按下前光标先落到位
            dwFlags: flag | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            time: 0,
            dwExtraInfo: 0,
        };
        input
    };
    Ok(vec![mk(down_flag), mk(up_flag)])
}

/// 构造滚轮 INPUT:可选先移动到 (x,y),再发 WHEEL(delta>0 向下;纯函数)
fn build_scroll_inputs(delta: i32, x: Option<i32>, y: Option<i32>) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(2);
    if let (Some(x), Some(y)) = (x, y) {
        inputs.push(build_move_input(x, y));
    }
    // delta 转 wheel 步进:每单位 120;负数经 u32 回绕即为向上滚动(WHEEL 以 u32 存储)
    let mouse_data = (delta.saturating_mul(WHEEL_STEP)) as u32;
    let mut input = INPUT {
        r#type: INPUT_MOUSE,
        ..unsafe { mem::zeroed() }
    };
    input.Anonymous.mi = MOUSEINPUT {
        dx: 0,
        dy: 0,
        mouseData: mouse_data,
        dwFlags: MOUSEEVENTF_WHEEL,
        time: 0,
        dwExtraInfo: 0,
    };
    inputs.push(input);
    inputs
}

/// 键盘映射表:特殊键名 → VK(键名大小写不敏感;纯函数,可单测)。
/// 字母/数字在 `key_to_vk` 中直传(字母统一大写 VK,数字直传),不在本表。
fn special_key_vk(name: &str) -> Option<VIRTUAL_KEY> {
    match name.to_ascii_lowercase().as_str() {
        "enter" => Some(VK_RETURN),
        "tab" => Some(VK_TAB),
        "space" => Some(VK_SPACE),
        "backspace" => Some(VK_BACK),
        "delete" => Some(VK_DELETE),
        "esc" => Some(VK_ESCAPE),
        "left" => Some(VK_LEFT),
        "right" => Some(VK_RIGHT),
        "up" => Some(VK_UP),
        "down" => Some(VK_DOWN),
        "home" => Some(VK_HOME),
        "end" => Some(VK_END),
        "pageup" => Some(VK_PRIOR),
        "pagedown" => Some(VK_NEXT),
        "f1" => Some(VK_F1),
        "f2" => Some(VK_F1 + 1),
        "f3" => Some(VK_F1 + 2),
        "f4" => Some(VK_F1 + 3),
        "f5" => Some(VK_F1 + 4),
        "f6" => Some(VK_F1 + 5),
        "f7" => Some(VK_F1 + 6),
        "f8" => Some(VK_F1 + 7),
        "f9" => Some(VK_F1 + 8),
        "f10" => Some(VK_F1 + 9),
        "f11" => Some(VK_F1 + 10),
        "f12" => Some(VK_F1 + 11),
        _ => None,
    }
}

/// 按键名 → VK(纯函数,可单测):
/// 单个字母 → 其大写 VK(按下即输出小写,组合 shift 输出大写);
/// 单个数字 → 直传;特殊键 → 映射表;其余 → None(由调用方报"不支持的按键")。
fn key_to_vk(key: &str) -> Option<VIRTUAL_KEY> {
    if let Some(vk) = special_key_vk(key) {
        return Some(vk);
    }
    let bytes = key.as_bytes();
    if bytes.len() != 1 {
        return None;
    }
    let c = bytes[0];
    match c {
        b'a'..=b'z' => Some((c as u16) - b'a' as u16 + b'A' as u16),
        b'A'..=b'Z' | b'0'..=b'9' => Some(c as u16),
        _ => None,
    }
}

/// modifiers 规范化(纯函数,可单测):"ctrl"|"shift"|"alt"|"win" 子集,
/// 按出现顺序去重,非法名 → Err。
fn normalize_modifiers(modifiers: &[&str]) -> Result<Vec<VIRTUAL_KEY>, String> {
    let mut out: Vec<VIRTUAL_KEY> = Vec::new();
    for m in modifiers {
        let vk = match m.to_ascii_lowercase().as_str() {
            "ctrl" => VK_CONTROL,
            "shift" => VK_SHIFT,
            "alt" => VK_MENU,
            "win" => VK_LWIN,
            _ => return Err(format!("不支持的修饰键: {m}(仅支持 ctrl/shift/alt/win)")),
        };
        if !out.contains(&vk) {
            out.push(vk); // 去重:重复修饰键只注入一次
        }
    }
    Ok(out)
}

/// 构造单个虚拟键按下或抬起的 INPUT(纯函数)
fn build_key_input(key: VIRTUAL_KEY, keyup: bool) -> INPUT {
    let mut input = INPUT {
        r#type: INPUT_KEYBOARD,
        ..unsafe { mem::zeroed() }
    };
    input.Anonymous.ki = KEYBDINPUT {
        wVk: key,
        wScan: 0,
        dwFlags: if keyup { KEYEVENTF_KEYUP } else { 0 },
        time: 0,
        dwExtraInfo: 0,
    };
    input
}

/// 构造按键(含组合键)的 INPUT 数组(纯函数):
/// 顺序 = modifiers 逐个按下 → 主键按下+抬起 → modifiers 逆序抬起(标准释放序,防粘滞键)。
fn build_key_inputs(key: &str, modifiers: &[&str]) -> Result<Vec<INPUT>, String> {
    let Some(vk) = key_to_vk(key) else {
        return Err(format!("不支持的按键: {key}"));
    };
    let mods = normalize_modifiers(modifiers)?;
    let mut inputs = Vec::with_capacity(mods.len() * 2 + 2);
    for m in &mods {
        inputs.push(build_key_input(*m, false)); // 修饰键逐个按下
    }
    inputs.push(build_key_input(vk, false)); // 主键按下
    inputs.push(build_key_input(vk, true)); // 主键抬起
    for m in mods.iter().rev() {
        inputs.push(build_key_input(*m, true)); // 修饰键逐个抬起
    }
    Ok(inputs)
}

/// 构造 UNICODE 文本输入的 INPUT 数组(纯函数):
/// 逐 UTF-16 码元,每码元 = 按下(KEYEVENTF_UNICODE)+ 抬起(KEYEVENTF_UNICODE|KEYEVENTF_KEYUP),
/// wVk=0、wScan=码元(非 ASCII 安全,不碰剪贴板)。空文本 → 空数组(无操作成功)。
fn build_unicode_inputs(text: &str) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        for keyup in [false, true] {
            let mut input = INPUT {
                r#type: INPUT_KEYBOARD,
                ..unsafe { mem::zeroed() }
            };
            input.Anonymous.ki = KEYBDINPUT {
                wVk: 0,
                wScan: unit,
                dwFlags: KEYEVENTF_UNICODE | if keyup { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            };
            inputs.push(input);
        }
    }
    inputs
}

// ---------------------------------------------------------------------------
// 注入(unsafe,手动验收覆盖)
// ---------------------------------------------------------------------------

/// 注入一组 INPUT:返回 0 = 全部被系统拒绝(最常见 = UIPI 目标为管理员窗口)。
/// 部分注入(0 < n < len,系统输入缓冲满等)同样按失败处理。
fn inject(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(()); // 空动作(如空文本)视为成功
    }
    unsafe {
        // 调用前清错误,便于用 GetLastError 区分"被拒"与其他失败(设计文档 §8 风险 3)
        SetLastError(0);
        let sent = SendInput(inputs.len() as u32, inputs.as_ptr(), mem::size_of::<INPUT>() as i32);
        if sent == 0 {
            let err = GetLastError();
            return Err(format!("{UIPI_REJECTED_MSG}{err}"));
        }
        if (sent as usize) < inputs.len() {
            let err = GetLastError();
            return Err(format!("输入注入不完整({sent}/{},错误码 {err})", inputs.len()));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 公共契约(4.3 复用 click_at / type_text;签名与设计文档 §2.2 一字不差)
// ---------------------------------------------------------------------------

/// 移动鼠标到绝对屏幕坐标(单位:像素)
pub fn move_to(x: i32, y: i32) -> Result<(), String> {
    inject(std::slice::from_ref(&build_move_input(x, y)))
}

/// 在坐标处点击(默认左键;button: "left"|"right"|"middle")
pub fn click_at(x: i32, y: i32, button: &str) -> Result<(), String> {
    inject(&build_click_inputs(x, y, button)?)
}

/// 滚轮:delta>0 向下滚动(Windows 语义),可选先移动鼠标到 (x,y)
pub fn scroll(delta: i32, x: Option<i32>, y: Option<i32>) -> Result<(), String> {
    inject(&build_scroll_inputs(delta, x, y))
}

/// 按键:key 见键盘表,modifiers = ["ctrl","shift","alt","win"] 子集
/// (按下顺序注入,全部抬起后收尾)
pub fn press_key(key: &str, modifiers: &[&str]) -> Result<(), String> {
    inject(&build_key_inputs(key, modifiers)?)
}

/// Unicode 文本输入(逐字符 SendInput KEYEVENTF_UNICODE,非 ASCII 安全;不依赖剪贴板)
pub fn type_text(text: &str) -> Result<(), String> {
    inject(&build_unicode_inputs(text))
}

// ---------------------------------------------------------------------------
// 单测(纯函数部分;注入本身不测)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 便捷取 INPUT 的 mi/ki 字段(测试读 union Copy 字段是安全的)
    fn mi(input: &INPUT) -> MOUSEINPUT {
        unsafe { input.Anonymous.mi }
    }
    fn ki(input: &INPUT) -> KEYBDINPUT {
        unsafe { input.Anonymous.ki }
    }

    // ---- 坐标归一化 ----

    #[test]
    fn normalize_coord_basic() {
        assert_eq!(normalize_coord(0, 1920), 0);
        assert_eq!(normalize_coord(960, 1920), 32768); // 中点为半轴
        assert_eq!(normalize_coord(1920, 1920), 65535); // 末列钳到上限
        assert_eq!(normalize_coord(-5, 1920), 0); // 负值钳到 0
    }

    // ---- 键盘映射表 ----

    #[test]
    fn key_map_letters_and_digits() {
        assert_eq!(key_to_vk("a"), Some(0x41)); // 'A'
        assert_eq!(key_to_vk("z"), Some(0x5A)); // 'Z'
        assert_eq!(key_to_vk("A"), Some(0x41)); // 大写与小写同一 VK
        assert_eq!(key_to_vk("0"), Some(0x30)); // '0'
        assert_eq!(key_to_vk("9"), Some(0x39)); // '9'
    }

    #[test]
    fn key_map_special_keys() {
        assert_eq!(key_to_vk("enter"), Some(VK_RETURN));
        assert_eq!(key_to_vk("Enter"), Some(VK_RETURN)); // 大小写不敏感
        assert_eq!(key_to_vk("TAB"), Some(VK_TAB));
        assert_eq!(key_to_vk("space"), Some(VK_SPACE));
        assert_eq!(key_to_vk("backspace"), Some(VK_BACK));
        assert_eq!(key_to_vk("delete"), Some(VK_DELETE));
        assert_eq!(key_to_vk("esc"), Some(VK_ESCAPE));
        assert_eq!(key_to_vk("left"), Some(VK_LEFT));
        assert_eq!(key_to_vk("right"), Some(VK_RIGHT));
        assert_eq!(key_to_vk("up"), Some(VK_UP));
        assert_eq!(key_to_vk("down"), Some(VK_DOWN));
        assert_eq!(key_to_vk("home"), Some(VK_HOME));
        assert_eq!(key_to_vk("end"), Some(VK_END));
        assert_eq!(key_to_vk("pageup"), Some(VK_PRIOR));
        assert_eq!(key_to_vk("pagedown"), Some(VK_NEXT));
    }

    #[test]
    fn key_map_function_keys() {
        for n in 1..=12 {
            let name = format!("f{n}");
            assert_eq!(key_to_vk(&name), Some(VK_F1 + (n as u16) - 1));
        }
        assert_eq!(key_to_vk("F12"), Some(VK_F1 + 11)); // 大小写不敏感
        assert_eq!(key_to_vk("f13"), None); // 超出 F12 范围
    }

    #[test]
    fn key_map_unknown_rejected() {
        assert_eq!(key_to_vk("ctrl"), None); // ctrl 走 modifiers,不是 key
        assert_eq!(key_to_vk(";"), None); // 非字母数字单字符
        assert_eq!(key_to_vk("enter "), None); // 带空白
        assert_eq!(key_to_vk(""), None);
        assert_eq!(key_to_vk("ab"), None); // 多字符非特殊键
        assert_eq!(key_to_vk("你好"), None);
    }

    // ---- modifiers 规范化 ----

    #[test]
    fn modifiers_normalize_dedup_keep_order() {
        assert_eq!(normalize_modifiers(&[]), Ok(vec![]));
        assert_eq!(normalize_modifiers(&["ctrl"]), Ok(vec![VK_CONTROL]));
        assert_eq!(
            normalize_modifiers(&["ctrl", "shift", "alt", "win"]),
            Ok(vec![VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN])
        );
        // 去重:重复只留首个,顺序保持
        assert_eq!(
            normalize_modifiers(&["shift", "ctrl", "shift", "alt"]),
            Ok(vec![VK_SHIFT, VK_CONTROL, VK_MENU])
        );
        // 大小写不敏感
        assert_eq!(normalize_modifiers(&["Ctrl", "SHIFT"]), Ok(vec![VK_CONTROL, VK_SHIFT]));
    }

    #[test]
    fn modifiers_reject_illegal() {
        assert!(normalize_modifiers(&["capslock"]).is_err());
        assert!(normalize_modifiers(&["ctrl", "super"]).is_err());
        assert!(normalize_modifiers(&[""]).is_err());
    }

    // ---- INPUT 构造 ----

    #[test]
    fn build_click_input_flags() {
        let inputs = build_click_inputs(100, 200, "left").unwrap();
        assert_eq!(inputs.len(), 2); // 按下 + 抬起
        let base = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        assert_eq!(mi(&inputs[0]).dwFlags, MOUSEEVENTF_LEFTDOWN | base);
        assert_eq!(mi(&inputs[1]).dwFlags, MOUSEEVENTF_LEFTUP | base);
        assert_eq!(inputs[0].r#type, INPUT_MOUSE);
        // 位置归一化后两个事件同点
        assert_eq!(mi(&inputs[0]).dx, mi(&inputs[1]).dx);
        assert_eq!(mi(&inputs[0]).dy, mi(&inputs[1]).dy);
    }

    #[test]
    fn build_click_right_middle_flags() {
        let right = build_click_inputs(0, 0, "right").unwrap();
        let base = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        assert_eq!(mi(&right[0]).dwFlags, MOUSEEVENTF_RIGHTDOWN | base);
        assert_eq!(mi(&right[1]).dwFlags, MOUSEEVENTF_RIGHTUP | base);
        let middle = build_click_inputs(0, 0, "middle").unwrap();
        assert_eq!(mi(&middle[0]).dwFlags, MOUSEEVENTF_MIDDLEDOWN | base);
        assert_eq!(mi(&middle[1]).dwFlags, MOUSEEVENTF_MIDDLEUP | base);
        // 非法按键 → Err
        assert!(build_click_inputs(0, 0, "double").is_err());
    }

    #[test]
    fn build_move_input_flags() {
        let input = build_move_input(10, 20);
        assert_eq!(input.r#type, INPUT_MOUSE);
        assert_eq!(
            mi(&input).dwFlags,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK
        );
    }

    #[test]
    fn build_scroll_inputs_behavior() {
        // 无坐标:单条 WHEEL,delta 正 = 向下(120 每单位)
        let only = build_scroll_inputs(2, None, None);
        assert_eq!(only.len(), 1);
        assert_eq!(mi(&only[0]).dwFlags, MOUSEEVENTF_WHEEL);
        assert_eq!(mi(&only[0]).mouseData, 240); // 2 * 120
        // 负 delta 向上(u32 回绕 = 负数)
        let up = build_scroll_inputs(-1, None, None);
        assert_eq!(up.len(), 1);
        assert_eq!(mi(&up[0]).mouseData as i32, -120);
        // 带坐标:先移动再滚动
        let both = build_scroll_inputs(1, Some(5), Some(6));
        assert_eq!(both.len(), 2);
        assert_eq!(mi(&both[0]).dwFlags, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK);
        assert_eq!(mi(&both[1]).dwFlags, MOUSEEVENTF_WHEEL);
    }

    #[test]
    fn build_key_inputs_vk_and_order() {
        // ctrl+a:ctrl 按下 → a 按下 → a 抬起 → ctrl 抬起
        let inputs = build_key_inputs("a", &["ctrl"]).unwrap();
        assert_eq!(inputs.len(), 4);
        assert_eq!(inputs[0].r#type, INPUT_KEYBOARD);
        assert_eq!(ki(&inputs[0]).wVk, VK_CONTROL);
        assert_eq!(ki(&inputs[0]).dwFlags, 0); // 按下
        assert_eq!(ki(&inputs[1]).wVk, 0x41);
        assert_eq!(ki(&inputs[1]).dwFlags, 0);
        assert_eq!(ki(&inputs[2]).wVk, 0x41);
        assert_eq!(ki(&inputs[2]).dwFlags, KEYEVENTF_KEYUP);
        assert_eq!(ki(&inputs[3]).wVk, VK_CONTROL);
        assert_eq!(ki(&inputs[3]).dwFlags, KEYEVENTF_KEYUP);
        // 未指定主键时 wScan 应为 0(纯 VK 注入)
        assert_eq!(ki(&inputs[1]).wScan, 0);
    }

    #[test]
    fn build_key_inputs_multi_modifier_order() {
        // ctrl+shift+esc:ctrl↓ shift↓ esc↓ esc↑ shift↑ ctrl↑(修饰键逆序抬起)
        let inputs = build_key_inputs("esc", &["ctrl", "shift"]).unwrap();
        assert_eq!(inputs.len(), 6);
        assert_eq!(ki(&inputs[0]).wVk, VK_CONTROL);
        assert_eq!(ki(&inputs[1]).wVk, VK_SHIFT);
        assert_eq!(ki(&inputs[2]).wVk, VK_ESCAPE);
        assert_eq!(ki(&inputs[3]).wVk, VK_ESCAPE);
        assert_eq!(ki(&inputs[4]).wVk, VK_SHIFT); // 逆序抬起
        assert_eq!(ki(&inputs[5]).wVk, VK_CONTROL);
        assert!(ki(&inputs[3]).dwFlags & KEYEVENTF_KEYUP != 0);
        assert!(ki(&inputs[4]).dwFlags & KEYEVENTF_KEYUP != 0);
        assert!(ki(&inputs[5]).dwFlags & KEYEVENTF_KEYUP != 0);
    }

    #[test]
    fn build_key_inputs_unknown_key_and_modifier_err() {
        assert!(build_key_inputs("unknown", &[]).is_err());
        assert!(build_key_inputs("a", &["evil"]).is_err());
    }

    #[test]
    fn build_unicode_inputs_ascii_and_cjk() {
        // 每个字符 = 按下 + 抬起 两事件;ASCII 每码元 1 个 u16
        let ascii = build_unicode_inputs("ab");
        assert_eq!(ascii.len(), 4);
        assert_eq!(ki(&ascii[0]).wScan, b'a' as u16);
        assert_eq!(ki(&ascii[0]).dwFlags, KEYEVENTF_UNICODE);
        assert_eq!(ki(&ascii[1]).dwFlags, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
        assert_eq!(ki(&ascii[1]).wScan, b'a' as u16);
        // wVk 必须为 0(UNICODE 注入语义)
        assert_eq!(ki(&ascii[0]).wVk, 0);

        // 中文"你" = U+4F60,单码元
        let cjk = build_unicode_inputs("你");
        assert_eq!(cjk.len(), 2);
        assert_eq!(ki(&cjk[0]).wScan, 0x4F60);
        assert_eq!(ki(&cjk[0]).dwFlags, KEYEVENTF_UNICODE);
        assert_eq!(ki(&cjk[1]).dwFlags, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);

        // 空文本 → 空数组(注入端视为成功)
        assert!(build_unicode_inputs("").is_empty());
    }
}
