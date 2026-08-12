//! Phase4 4.3 UI-Automation 控件操作:windows-sys `Uia*` 句柄式客户端 API 封装。
//!
//! 设计依据:docs/Phase4-设计.md §3(范围/签名/边界文案/决策)。
//!
//! ## 与 windows-sys 0.59 的核对结论(2026-08-12,本机源码 grep + Win11 实测)
//!
//! - Uia* 函数族(UiaNodeFromHandle/UiaGetPropertyValue/UiaSetFocus 等)在
//!   `Win32_UI_Accessibility` 模块,是 `windows_targets::link!` 裸声明,直接按函数名调用;
//! - 本模块不用 IUIAutomation COM 接口(规避手写大 vtable,见错误记录 COM 教训);
//! - `VARIANT`(Win32_System_Variant)按 C 布局手写读写(偏移/联合路径已实测核对);
//! - **控件树遍历基于 Win32 窗口树(方案 C),不用 UiaFind/UiaNavigate**:
//!   2026-08-12 Win11 实测,UiaFind/UiaNavigate 的 ppRequestedData 返回
//!   UIAutomationCore 内部格式({0x8000 内部引用 / 堆指针}周期记录),不能作为
//!   HUIANODE 传给 UiaGetPropertyValue(直传必崩 0xc0000005);而
//!   `窗口句柄 → UiaNodeFromHandle → UiaGetPropertyValue` 链路实测完全可用,
//!   故遍历 = EnumWindows/GetWindow 窗口树 + UiaNodeFromHandle 逐窗口取节点;
//! - 客户端 HUIANODE 生命周期由 UIAutomationCore 托管,本模块不做 UiaNodeRelease
//!   (风险点:以手动验收阶段实测为准,若需释放统一收口在本文件,勿散落)。
//!
//! 零常驻线程:全部函数瞬态调用,无监听/轮询。
//! 全部 unsafe 逐调用包 unsafe{} + 注释。

// 集成接线前(automation 为私有 mod 且 invoke_handler 未注册)整链未 reachable,
// dead_code 警告属预期;接线后(pub mod + 命令注册)可移除本属性。
#![allow(dead_code)]
// windows-sys 的 UIA_*ControlTypeId 常量按 Win32 惯例驼峰命名,与 rustc 命名 lint 冲突
#![allow(non_upper_case_globals)]

use windows_sys::Win32::Foundation::{E_ACCESSDENIED, E_FAIL, E_INVALIDARG, HWND, LPARAM, BOOL};
use windows_sys::Win32::System::Variant::{
    VariantClear, VARIANT, VT_ARRAY, VT_BSTR, VT_I4, VT_R8,
};
use windows_sys::Win32::UI::Accessibility::{
    UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId, UIA_CalendarControlTypeId,
    UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_ControlTypePropertyId,
    UIA_CustomControlTypeId, UIA_DataGridControlTypeId, UIA_DataItemControlTypeId,
    UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_GroupControlTypeId,
    UIA_HeaderControlTypeId, UIA_HeaderItemControlTypeId, UIA_HyperlinkControlTypeId,
    UIA_ImageControlTypeId, UIA_ListControlTypeId, UIA_ListItemControlTypeId,
    UIA_MenuBarControlTypeId, UIA_MenuControlTypeId, UIA_MenuItemControlTypeId,
    UIA_NamePropertyId, UIA_PaneControlTypeId, UIA_ProgressBarControlTypeId,
    UIA_RadioButtonControlTypeId, UIA_ScrollBarControlTypeId, UIA_SeparatorControlTypeId,
    UIA_SliderControlTypeId, UIA_SpinnerControlTypeId, UIA_SplitButtonControlTypeId,
    UIA_StatusBarControlTypeId, UIA_TabControlTypeId, UIA_TabItemControlTypeId,
    UIA_TableControlTypeId, UIA_TextControlTypeId, UIA_ThumbControlTypeId,
    UIA_TitleBarControlTypeId, UIA_ToolBarControlTypeId, UIA_ToolTipControlTypeId,
    UIA_TreeControlTypeId, UIA_TreeItemControlTypeId, UIA_ValueValuePropertyId,
    UIA_WindowControlTypeId, UiaGetPropertyValue, UiaNodeFromHandle, UiaSetFocus, HUIANODE,
    UIA_AppBarControlTypeId,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowTextW, GetWindowThreadProcessId, GW_CHILD,
    GW_HWNDNEXT, IsWindow, IsWindowVisible,
};
use windows_sys::core::BSTR;

use serde::Serialize;

// ---------------------------------------------------------------------------
// 边界文案(设计文档 §3.4 写死,与前端 UI 一致)
// ---------------------------------------------------------------------------

pub const MSG_UWP: &str = "UWP 应用不支持第三方控件操作(系统安全限制)";
pub const MSG_ADMIN: &str = "目标窗口为管理员权限,当前程序无权限访问";
pub const MSG_NO_PROVIDER: &str = "该控件不支持 UI 自动化,请换用键鼠模拟或系统原生操作";
pub const MSG_NOT_FOUND: &str = "未找到匹配的窗口/控件";
pub const MSG_DISABLED: &str = "自动化未启用,请在设置中开启";

// ---------------------------------------------------------------------------
// 公共类型(命令层/前端 serialize)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct UiaWindow {
    pub hwnd: i64,
    pub title: String,
    pub class: String,
    pub pid: u32,
    pub visible: bool,
}

/// 控件:id = 从窗口根到该控件的遍历路径(如 "0.2.1"),命令层用它定位
#[derive(Clone, Debug, Serialize)]
pub struct UiaControl {
    pub id: String,
    pub name: String,
    pub control_type: String,
    pub bounds: (i32, i32, i32, i32), // (left, top, right, bottom),屏幕绝对坐标
}

/// 遍历上限:深度 ≤ 3(路径段数),总数 ≤ 200(防卡死,设计文档 §3.1)
pub const MAX_TREE_DEPTH: u32 = 3;
pub const MAX_CONTROLS: usize = 200;

// ---------------------------------------------------------------------------
// 手写 FFI:windows-sys 未提供的 oleaut32 / kernel32 / advapi32 符号
// ---------------------------------------------------------------------------

#[link(name = "oleaut32")]
unsafe extern "system" {
    /// BSTR 的 UTF-16 字符数(不计数终止符)
    fn SysStringLen(bstr: BSTR) -> u32;
    /// 分配 BSTR(测试构造用)
    fn SysAllocStringLen(sz: *const u16, len: u32) -> BSTR;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> *mut core::ffi::c_void;
    fn CloseHandle(h: *mut core::ffi::c_void) -> BOOL;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetTokenInformation(
        token: *mut core::ffi::c_void,
        class: u32,
        info: *mut core::ffi::c_void,
        len: u32,
        ret: *mut u32,
    ) -> BOOL;
}

/// TokenAppContainerSid(判定 UWP 应用容器)
const TOKEN_APPCONTAINER_SID: u32 = 29;
/// PROCESS_QUERY_LIMITED_INFORMATION(受限查询权限即可读 token,无需管理员)
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

// ---------------------------------------------------------------------------
// 纯函数:窗口过滤 / 路径编解码 / 中心点 / 边界文案 / 控件类型映射
// ---------------------------------------------------------------------------

/// 标题大小写不敏感子串匹配;空过滤词 = 全匹配
pub fn window_title_matches(title: &str, filter: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&f.to_lowercase())
}

/// find_top_windows 的单条候选过滤(可见性 + 标题),纯函数可测
pub fn collect_window_candidate(
    hwnd: i64,
    title: &str,
    class: &str,
    pid: u32,
    visible: bool,
    filter: &str,
) -> Option<UiaWindow> {
    if !visible || !window_title_matches(title, filter) {
        return None;
    }
    Some(UiaWindow {
        hwnd,
        title: title.to_string(),
        class: class.to_string(),
        pid,
        visible,
    })
}

/// 遍历路径 → 控件 id("0.2.1")
pub fn encode_path(segs: &[u32]) -> String {
    segs.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(".")
}

/// 控件 id → 遍历路径数组;非法格式返回 None(往返与 encode_path 一致)
pub fn decode_path(id: &str) -> Option<Vec<u32>> {
    if id.is_empty() {
        return None;
    }
    id.split('.').map(|s| s.parse::<u32>().ok()).collect()
}

/// 矩形 (left, top, right, bottom) 的中心点;空/反向矩形返回 None
pub fn bounds_center(b: (i32, i32, i32, i32)) -> Option<(i32, i32)> {
    let (x0, y0, x1, y1) = b;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0 + (x1 - x0) / 2, y0 + (y1 - y0) / 2))
}

/// HRESULT → 边界文案(纯函数,§3.4):
/// - E_ACCESSDENIED:UIPI 隔离;再按是否 UWP 应用容器分流文案
/// - 其余:通用错误码文案
pub fn map_access_error(hr: i32, is_uwp: bool) -> String {
    if hr == E_ACCESSDENIED as i32 {
        if is_uwp {
            MSG_UWP.to_string()
        } else {
            MSG_ADMIN.to_string()
        }
    } else {
        format!("UI 自动化操作失败(错误码 0x{:08X})", hr as u32)
    }
}

/// UIA 控件类型 ID → 可读名(未知类型归 "Custom")
pub fn control_type_name(id: i32) -> &'static str {
    match id {
        UIA_ButtonControlTypeId => "Button",
        UIA_CalendarControlTypeId => "Calendar",
        UIA_CheckBoxControlTypeId => "CheckBox",
        UIA_ComboBoxControlTypeId => "ComboBox",
        UIA_EditControlTypeId => "Edit",
        UIA_HyperlinkControlTypeId => "Hyperlink",
        UIA_ImageControlTypeId => "Image",
        UIA_ListItemControlTypeId => "ListItem",
        UIA_ListControlTypeId => "List",
        UIA_MenuControlTypeId => "Menu",
        UIA_MenuBarControlTypeId => "MenuBar",
        UIA_MenuItemControlTypeId => "MenuItem",
        UIA_ProgressBarControlTypeId => "ProgressBar",
        UIA_RadioButtonControlTypeId => "RadioButton",
        UIA_ScrollBarControlTypeId => "ScrollBar",
        UIA_SliderControlTypeId => "Slider",
        UIA_SpinnerControlTypeId => "Spinner",
        UIA_StatusBarControlTypeId => "StatusBar",
        UIA_TabControlTypeId => "Tab",
        UIA_TabItemControlTypeId => "TabItem",
        UIA_TextControlTypeId => "Text",
        UIA_ToolBarControlTypeId => "ToolBar",
        UIA_ToolTipControlTypeId => "ToolTip",
        UIA_TreeControlTypeId => "Tree",
        UIA_TreeItemControlTypeId => "TreeItem",
        UIA_CustomControlTypeId => "Custom",
        UIA_GroupControlTypeId => "Group",
        UIA_ThumbControlTypeId => "Thumb",
        UIA_DataGridControlTypeId => "DataGrid",
        UIA_DataItemControlTypeId => "DataItem",
        UIA_DocumentControlTypeId => "Document",
        UIA_SplitButtonControlTypeId => "SplitButton",
        UIA_WindowControlTypeId => "Window",
        UIA_PaneControlTypeId => "Pane",
        UIA_HeaderControlTypeId => "Header",
        UIA_HeaderItemControlTypeId => "HeaderItem",
        UIA_TableControlTypeId => "Table",
        UIA_TitleBarControlTypeId => "TitleBar",
        UIA_SeparatorControlTypeId => "Separator",
        UIA_AppBarControlTypeId => "AppBar",
        _ => "Custom",
    }
}

/// 可读名 → UIA 控件类型 ID(过滤用,大小写不敏感);未知返回 None
pub fn control_type_id(name: &str) -> Option<i32> {
    match name.trim().to_ascii_lowercase().as_str() {
        "button" => Some(UIA_ButtonControlTypeId),
        "calendar" => Some(UIA_CalendarControlTypeId),
        "checkbox" => Some(UIA_CheckBoxControlTypeId),
        "combobox" => Some(UIA_ComboBoxControlTypeId),
        "edit" | "textbox" => Some(UIA_EditControlTypeId),
        "hyperlink" | "link" => Some(UIA_HyperlinkControlTypeId),
        "image" | "picture" => Some(UIA_ImageControlTypeId),
        "listitem" => Some(UIA_ListItemControlTypeId),
        "list" => Some(UIA_ListControlTypeId),
        "menu" => Some(UIA_MenuControlTypeId),
        "menubar" => Some(UIA_MenuBarControlTypeId),
        "menuitem" => Some(UIA_MenuItemControlTypeId),
        "progressbar" => Some(UIA_ProgressBarControlTypeId),
        "radiobutton" | "radio" => Some(UIA_RadioButtonControlTypeId),
        "scrollbar" => Some(UIA_ScrollBarControlTypeId),
        "slider" => Some(UIA_SliderControlTypeId),
        "spinner" => Some(UIA_SpinnerControlTypeId),
        "statusbar" => Some(UIA_StatusBarControlTypeId),
        "tab" => Some(UIA_TabControlTypeId),
        "tabitem" => Some(UIA_TabItemControlTypeId),
        "text" => Some(UIA_TextControlTypeId),
        "toolbar" => Some(UIA_ToolBarControlTypeId),
        "tooltip" => Some(UIA_ToolTipControlTypeId),
        "tree" => Some(UIA_TreeControlTypeId),
        "treeitem" => Some(UIA_TreeItemControlTypeId),
        "custom" => Some(UIA_CustomControlTypeId),
        "group" => Some(UIA_GroupControlTypeId),
        "thumb" => Some(UIA_ThumbControlTypeId),
        "datagrid" => Some(UIA_DataGridControlTypeId),
        "dataitem" => Some(UIA_DataItemControlTypeId),
        "document" => Some(UIA_DocumentControlTypeId),
        "splitbutton" => Some(UIA_SplitButtonControlTypeId),
        "window" => Some(UIA_WindowControlTypeId),
        "pane" => Some(UIA_PaneControlTypeId),
        "header" => Some(UIA_HeaderControlTypeId),
        "headeritem" => Some(UIA_HeaderItemControlTypeId),
        "table" => Some(UIA_TableControlTypeId),
        "titlebar" => Some(UIA_TitleBarControlTypeId),
        "separator" => Some(UIA_SeparatorControlTypeId),
        "appbar" => Some(UIA_AppBarControlTypeId),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// VARIANT 读写辅助(纯函数,VT_BSTR/VT_I4/VT_R8/VT_ARRAY|VT_R8 分支)
// ---------------------------------------------------------------------------

/// VARIANT 的类型字段(VARENUM)
pub fn variant_vt(v: &VARIANT) -> u16 {
    unsafe { v.Anonymous.Anonymous.vt }
}

/// VT_I4 → i32
pub fn variant_i4(v: &VARIANT) -> Option<i32> {
    unsafe {
        if variant_vt(v) != VT_I4 {
            return None;
        }
        Some(v.Anonymous.Anonymous.Anonymous.lVal)
    }
}

/// VT_R8 → f64
pub fn variant_r8(v: &VARIANT) -> Option<f64> {
    unsafe {
        if variant_vt(v) != VT_R8 {
            return None;
        }
        Some(v.Anonymous.Anonymous.Anonymous.dblVal)
    }
}

/// VT_BSTR → BSTR(调用方负责在 VARIANT 上 VariantClear)
pub fn variant_bstr(v: &VARIANT) -> Option<BSTR> {
    unsafe {
        if variant_vt(v) != VT_BSTR {
            return None;
        }
        let b = v.Anonymous.Anonymous.Anonymous.bstrVal;
        if b.is_null() {
            None
        } else {
            Some(b)
        }
    }
}

/// VT_ARRAY|VT_R8 → [f64; 4](UIA_BoundingRectanglePropertyId 返回 4 个 double)
pub fn variant_r8_array4(v: &VARIANT) -> Option<[f64; 4]> {
    unsafe {
        if variant_vt(v) != VT_ARRAY | VT_R8 {
            return None;
        }
        let sa = v.Anonymous.Anonymous.Anonymous.parray;
        if sa.is_null() {
            return None;
        }
        let arr = &*sa;
        if arr.cDims < 1 || arr.rgsabound[0].cElements < 4 || arr.pvData.is_null() {
            return None;
        }
        let p = arr.pvData as *const f64;
        Some([*p, *p.add(1), *p.add(2), *p.add(3)])
    }
}

/// 构造 VT_I4 VARIANT(纯函数,条件构造/测试用)
pub fn variant_i4_from(v: i32) -> VARIANT {
    unsafe {
        let mut out: VARIANT = std::mem::zeroed();
        out.Anonymous.Anonymous.vt = VT_I4;
        out.Anonymous.Anonymous.Anonymous.lVal = v;
        out
    }
}

/// 构造 VT_R8 VARIANT
pub fn variant_r8_from(v: f64) -> VARIANT {
    unsafe {
        let mut out: VARIANT = std::mem::zeroed();
        out.Anonymous.Anonymous.vt = VT_R8;
        out.Anonymous.Anonymous.Anonymous.dblVal = v;
        out
    }
}

/// 构造 VT_BSTR VARIANT(持有一个已分配 BSTR;释放由 VARIANT 上的 VariantClear 负责)
pub fn variant_bstr_from(b: BSTR) -> VARIANT {
    unsafe {
        let mut out: VARIANT = std::mem::zeroed();
        out.Anonymous.Anonymous.vt = VT_BSTR;
        out.Anonymous.Anonymous.Anonymous.bstrVal = b;
        out
    }
}

/// BSTR → String(null 安全,无效 UTF-16 用 lossy)
pub fn bstr_to_string(b: BSTR) -> String {
    if b.is_null() {
        return String::new();
    }
    unsafe {
        let len = SysStringLen(b) as usize; // UTF-16 字符数
        let slice = std::slice::from_raw_parts(b, len);
        String::from_utf16_lossy(slice)
    }
}

// ---------------------------------------------------------------------------
// 纯函数:控件树遍历(walk_tree,与真实 UIA 遍历共用,可假树注入测试)
// ---------------------------------------------------------------------------

/// 按限深/限量收集控件树路径(纯遍历逻辑,与 UIA 句柄无关,可单测)。
///
/// - 语义:虚拟根的直接子为第 1 层(路径 1 段),路径段数 ≤ max_depth;
///   收集结果按 DFS 序,总数 ≤ max_count;
/// - `get_children(path)` 返回该路径节点的子段号列表(如 [0, 2] 表示第 0、2 个子);
///   根层用 `get_children(&[])` 返回首层子段号。
pub fn walk_tree<F>(get_children: F, max_depth: u32, max_count: usize) -> Vec<Vec<u32>>
where
    F: FnMut(&[u32]) -> Vec<u32>,
{
    let mut out: Vec<Vec<u32>> = Vec::new();
    // FnMut:允许遍历闭包带外部状态(如权限错误标记)
    let mut get_children = get_children;
    // 栈:(路径, 下一层要压入的子段号),从首层子开始
    let mut stack: Vec<Vec<u32>> = get_children(&[]).into_iter().map(|s| vec![s]).collect();
    while let Some(path) = stack.pop() {
        if out.len() >= max_count {
            break;
        }
        out.push(path.clone());
        // 深度封顶:路径段数已达 max_depth 不再展开
        if (path.len() as u32) >= max_depth {
            continue;
        }
        // 数量封顶:展开前检查,防一次性压入超量
        let remaining = max_count.saturating_sub(out.len());
        if remaining == 0 {
            break;
        }
        for seg in get_children(&path).into_iter().take(remaining) {
            let mut p = path.clone();
            p.push(seg);
            stack.push(p);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// UIA 内部 FFI 辅助
// ---------------------------------------------------------------------------

/// 窗口句柄 → UIA 节点(UIAutomationCore 托管,无需释放)
fn node_from_hwnd(hwnd: HWND) -> Result<HUIANODE, i32> {
    if hwnd.is_null() {
        return Err(E_INVALIDARG);
    }
    unsafe {
        let mut node: HUIANODE = std::ptr::null_mut();
        // 取窗口根节点:失败多为权限隔离(管理员/UWP)或无 UIA 提供者
        let hr = UiaNodeFromHandle(hwnd, &mut node);
        if hr != 0 {
            return Err(hr);
        }
        if node.is_null() {
            return Err(E_FAIL);
        }
        Ok(node)
    }
}

/// 直接子窗口列表(GetWindow GW_CHILD + GW_HWNDNEXT 链式遍历,Z 序)。
/// 窗口枚举不跨进程校验,不会因 UIPI 返回错误;权限判定在 UiaNodeFromHandle。
fn child_windows(hwnd: HWND) -> Vec<HWND> {
    unsafe {
        let mut out = Vec::new();
        let mut h = GetWindow(hwnd, GW_CHILD);
        while !h.is_null() {
            out.push(h);
            h = GetWindow(h, GW_HWNDNEXT);
        }
        out
    }
}

/// 按路径段从根窗口逐层定位子窗口;树变化(子列表截断/越界)→ None
fn locate_window_by_segments(root_hwnd: HWND, segs: &[u32]) -> Option<HWND> {
    let mut h = root_hwnd;
    for s in segs {
        let kids = child_windows(h);
        if (*s as usize) >= kids.len() {
            return None;
        }
        h = kids[*s as usize];
    }
    Some(h)
}

/// 根窗口 → 路径定位 → UIA 节点(权限校验 + 树变化容错)。
/// 读文本/读边界/聚焦的公共入口;路径为空 = 根窗口自身。
fn locate_control_node(hwnd: HWND, id: &str) -> Result<HUIANODE, String> {
    let uwp = is_app_container(hwnd as i64);
    let root = node_from_hwnd(hwnd).map_err(|hr| map_access_error(hr, uwp))?;
    let segs = decode_path(id).ok_or_else(|| format!("控件路径格式错误:{id}"))?;
    if segs.is_empty() {
        return Ok(root);
    }
    let ch = locate_window_by_segments(hwnd, &segs).ok_or_else(|| MSG_NOT_FOUND.to_string())?;
    node_from_hwnd(ch).map_err(|hr| map_access_error(hr, uwp))
}

/// 读单个属性到 VARIANT(调用方负责 VariantClear)
fn read_property(node: HUIANODE, prop: i32) -> Result<VARIANT, i32> {
    unsafe {
        let mut v: VARIANT = std::mem::zeroed();
        let hr = UiaGetPropertyValue(node, prop, &mut v);
        if hr != 0 {
            return Err(hr);
        }
        Ok(v)
    }
}

/// 读字符串属性(Name/Value):失败或类型不符 → 空串
fn read_property_str(node: HUIANODE, prop: i32) -> String {
    match read_property(node, prop) {
        Ok(mut v) => {
            let s = match variant_bstr(&v) {
                Some(b) => bstr_to_string(b),
                None => String::new(),
            };
            // VARIANT 持有 BSTR,统一清理
            unsafe { VariantClear(&mut v) };
            s
        }
        Err(_) => String::new(),
    }
}

/// 读 i32 属性(ControlType)
fn read_property_i4(node: HUIANODE, prop: i32) -> Option<i32> {
    match read_property(node, prop) {
        Ok(mut v) => {
            let out = variant_i4(&v);
            unsafe { VariantClear(&mut v) };
            out
        }
        Err(_) => None,
    }
}

/// 读 BoundingRectangle(屏幕绝对坐标)→ (left, top, right, bottom)
fn read_property_bounds(node: HUIANODE) -> Option<(i32, i32, i32, i32)> {
    match read_property(node, UIA_BoundingRectanglePropertyId) {
        Ok(mut v) => {
            let out = variant_r8_array4(&v);
            // VARIANT 持有 SAFEARRAY,统一清理
            unsafe { VariantClear(&mut v) };
            out.map(|r| (r[0] as i32, r[1] as i32, (r[0] + r[2]) as i32, (r[1] + r[3]) as i32))
        }
        Err(_) => None,
    }
}

/// 窗口是否 UWP 应用容器(GetTokenInformation TokenAppContainerSid;失败 → false)
pub fn is_app_container(hwnd: i64) -> bool {
    let h = hwnd as HWND;
    if h.is_null() {
        return false;
    }
    let mut pid: u32 = 0;
    unsafe {
        // 取窗口所属进程 PID(线程 ID 返回值此处不需要)
        GetWindowThreadProcessId(h, &mut pid);
    }
    if pid == 0 {
        return false;
    }
    pid_is_app_container(pid)
}

fn pid_is_app_container(pid: u32) -> bool {
    unsafe {
        // 受限查询权限即可读 token(同用户进程),管理员进程也无需提权
        let token = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if token.is_null() {
            return false;
        }
        let mut buf = [0u8; 256];
        let mut len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_APPCONTAINER_SID,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            buf.len() as u32,
            &mut len,
        );
        CloseHandle(token);
        // TokenAppContainerSid 可读出 = 该进程运行在应用容器(UWP)
        ok != 0
    }
}

// ---------------------------------------------------------------------------
// 公共 API(设计文档 §3.2 契约)
// ---------------------------------------------------------------------------

/// 按标题子串枚举可见顶层窗口(EnumWindows,普通 API 不依赖 UIA)
pub fn find_top_windows(title_like: &str) -> Result<Vec<UiaWindow>, String> {
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        // EnumWindows 回调收集全部顶层窗口句柄(常规几十个,无需预过滤)
        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let list = &mut *(lparam as *mut Vec<HWND>);
            list.push(hwnd);
            1 // TRUE 继续枚举
        }
        let ok = EnumWindows(Some(enum_proc), &mut hwnds as *mut Vec<HWND> as LPARAM);
        if ok == 0 {
            return Err("窗口枚举失败(EnumWindows)".to_string());
        }
    }
    let mut out = Vec::new();
    for h in hwnds {
        let visible = unsafe { IsWindowVisible(h) != 0 };
        let title = read_window_text(h);
        let class = read_window_class(h);
        let pid = read_window_pid(h);
        if let Some(w) = collect_window_candidate(h as i64, &title, &class, pid, visible, title_like) {
            out.push(w);
        }
    }
    Ok(out)
}

/// 按 hwnd 单查窗口详情
pub fn get_window_info(hwnd: i64) -> Result<UiaWindow, String> {
    let h = hwnd as HWND;
    if h.is_null() {
        return Err(MSG_NOT_FOUND.to_string());
    }
    unsafe {
        if IsWindow(h) == 0 {
            return Err(MSG_NOT_FOUND.to_string());
        }
    }
    let visible = unsafe { IsWindowVisible(h) != 0 };
    Ok(UiaWindow {
        hwnd,
        title: read_window_text(h),
        class: read_window_class(h),
        pid: read_window_pid(h),
        visible,
    })
}

/// 从指定窗口根遍历控件树(窗口树方案 C,限深 3、上限 200),可按类型/名称子串过滤
pub fn find_controls(hwnd: i64, control_type: &str, name_like: &str) -> Result<Vec<UiaControl>, String> {
    let h = hwnd as HWND;
    let uwp = is_app_container(hwnd);
    // 根窗口必须有 UIA 节点:权限门槛在此(管理员/UWP 窗口直接拒绝)
    node_from_hwnd(h).map_err(|hr| map_access_error(hr, uwp))?;

    let filter_type: Option<i32> = if control_type.trim().is_empty() {
        None
    } else {
        control_type_id(control_type)
    };

    // 遍历:每窗口枚举子窗口;子窗口取 UIA 节点失败(权限隔离)→ 记 denied,跳过该分支继续
    let mut denied = false;
    let paths = walk_tree(
        |path: &[u32]| -> Vec<u32> {
            match locate_window_by_segments(h, path) {
                Some(ch) => match node_from_hwnd(ch) {
                    Ok(_) => (0..child_windows(ch).len() as u32).collect(),
                    Err(hr) => {
                        if hr == E_ACCESSDENIED {
                            denied = true;
                        }
                        Vec::new() // 无 UIA 提供者/不可访问:不再展开其子窗口
                    }
                },
                None => Vec::new(), // 树变化(子列表截断/越界),跳过该分支
            }
        },
        MAX_TREE_DEPTH,
        MAX_CONTROLS,
    );

    let mut out = Vec::new();
    for path in &paths {
        let Some(ch) = locate_window_by_segments(h, path) else { continue };
        let Ok(node) = node_from_hwnd(ch) else { continue };
        // 读不到 ControlType 的节点视为无 UIA 提供者,跳过(不入结果)
        let Some(ctype_id) = read_property_i4(node, UIA_ControlTypePropertyId) else {
            continue;
        };
        if let Some(t) = filter_type {
            if ctype_id != t {
                continue;
            }
        }
        let name = read_property_str(node, UIA_NamePropertyId);
        if !name_like.trim().is_empty() && !window_title_matches(&name, name_like) {
            continue;
        }
        let bounds = read_property_bounds(node).unwrap_or((0, 0, 0, 0));
        out.push(UiaControl {
            id: encode_path(path),
            name,
            control_type: control_type_name(ctype_id).to_string(),
            bounds,
        });
        if out.len() >= MAX_CONTROLS {
            break;
        }
    }

    if out.is_empty() {
        if denied {
            return Err(map_access_error(E_ACCESSDENIED, uwp));
        }
        return Err(MSG_NOT_FOUND.to_string());
    }
    Ok(out)
}

/// 按路径读控件文本:Name 优先,空则回退 Value;两者皆空 → 无 UIA 提供者文案
pub fn get_control_text(hwnd: i64, id: &str) -> Result<String, String> {
    let node = locate_control_node(hwnd as HWND, id)?;
    let name = read_property_str(node, UIA_NamePropertyId);
    if !name.is_empty() {
        return Ok(name);
    }
    // ValuePattern.ValueProperty = 30045(windows-sys 命名为 UIA_ValueValuePropertyId)
    let value = read_property_str(node, UIA_ValueValuePropertyId);
    if !value.is_empty() {
        return Ok(value);
    }
    Err(MSG_NO_PROVIDER.to_string())
}

/// 按路径读控件边界(屏幕绝对坐标,与 input_sim 坐标系一致)
pub fn get_control_bounds(hwnd: i64, id: &str) -> Result<(i32, i32, i32, i32), String> {
    let node = locate_control_node(hwnd as HWND, id)?;
    read_property_bounds(node).ok_or_else(|| MSG_NO_PROVIDER.to_string())
}

/// UiaSetFocus 尝试聚焦控件(失败不代表错误:部分控件不支持,由命令层 click 兜底)
pub fn set_focus(hwnd: i64, id: &str) -> Result<bool, String> {
    let node = locate_control_node(hwnd as HWND, id)?;
    unsafe {
        // UiaSetFocus:焦点定位尝试;被 UIPI 拦截/不支持时返回非零,不视为致命
        let hr = UiaSetFocus(node);
        Ok(hr == 0)
    }
}

// ---------------------------------------------------------------------------
// 窗口信息读取辅助(GetWindowTextW/GetClassNameW/GetWindowThreadProcessId)
// ---------------------------------------------------------------------------

fn read_window_text(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 512];
        // 返回字符数(不含终止符);失败返回 0
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let n = n.max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

fn read_window_class(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let n = n.max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

fn read_window_pid(hwnd: HWND) -> u32 {
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid
    }
}

// ---------------------------------------------------------------------------
// 单测(纯函数部分;Uia* 真实调用需真实控件树,留给 §3.5 手动验收)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;


    // ---- 窗口过滤 ----

    #[test]
    fn title_match_case_insensitive_substring() {
        assert!(window_title_matches("无标题 - 记事本", "记事本"));
        assert!(window_title_matches("Notepad - doc.txt", "notepad"));
        assert!(window_title_matches("Notepad - doc.txt", "  NOTEPAD  ")); // 过滤词去首尾空白
        assert!(window_title_matches("anything", ""));
        assert!(!window_title_matches("abc", "xyz"));
        assert!(!window_title_matches("", "x"));
    }

    #[test]
    fn window_candidate_filters_visibility_and_title() {
        let w = collect_window_candidate(0x1234, "Test", "Notepad", 42, true, "test").unwrap();
        assert_eq!(w.hwnd, 0x1234);
        assert_eq!(w.pid, 42);
        assert_eq!(w.class, "Notepad");
        assert!(w.visible);
        // 不可见 → 过滤
        assert!(collect_window_candidate(1, "Test", "Notepad", 42, false, "test").is_none());
        // 标题不匹配 → 过滤
        assert!(collect_window_candidate(1, "Other", "Notepad", 42, true, "test").is_none());
        // 空过滤 → 可见窗口全收
        assert!(collect_window_candidate(1, "Other", "Notepad", 42, true, "").is_some());
    }

    // ---- 路径编解码 ----

    #[test]
    fn path_roundtrip() {
        assert_eq!(encode_path(&[0, 2, 1]), "0.2.1");
        assert_eq!(encode_path(&[]), "");
        assert_eq!(encode_path(&[7]), "7");
        assert_eq!(decode_path("0.2.1"), Some(vec![0, 2, 1]));
        assert_eq!(decode_path("7"), Some(vec![7]));
        // 非法输入 → None
        assert_eq!(decode_path(""), None);
        assert_eq!(decode_path("0.2.x"), None);
        assert_eq!(decode_path("0..1"), None);
        assert_eq!(decode_path(".1"), None);
        assert_eq!(decode_path("1."), None);
        // 往返一致
        for id in ["0", "0.2.1", "199.0.200"] {
            assert_eq!(encode_path(&decode_path(id).unwrap()), id);
        }
    }

    // ---- walk_tree:深度/数量截断(假树注入) ----

    /// 假树:根 2 子,每个节点 2 子,最深 3 层(总节点 2+4+8=14)
    fn fake_children(path: &[u32]) -> Vec<u32> {
        if path.is_empty() {
            return vec![0, 1];
        }
        if path.len() < 3 {
            vec![0, 1]
        } else {
            vec![]
        }
    }

    #[test]
    fn walk_tree_depth_limit() {
        let all = walk_tree(fake_children, 3, usize::MAX);
        assert_eq!(all.len(), 14);
        assert!(all.iter().all(|p| p.len() <= 3));
        // 深度 1 = 只取第一层子
        let l1 = walk_tree(fake_children, 1, usize::MAX);
        assert_eq!(l1.len(), 2);
        assert!(l1.iter().all(|p| p.len() == 1));
        // 深度 2 = 2 + 4
        assert_eq!(walk_tree(fake_children, 2, usize::MAX).len(), 6);
    }

    #[test]
    fn walk_tree_count_limit() {
        assert_eq!(walk_tree(fake_children, 3, 5).len(), 5);
        assert_eq!(walk_tree(fake_children, 3, 200).len(), 14); // 未触顶
        assert_eq!(walk_tree(fake_children, 3, 1).len(), 1);
        assert_eq!(walk_tree(fake_children, 3, 0).len(), 0);
    }

    #[test]
    fn walk_tree_empty_and_wide_root() {
        assert!(walk_tree(|_| vec![], 3, 200).is_empty());
        // 根层一次性压入超量:take(remaining) 截断
        let wide = walk_tree(|p| if p.is_empty() { (0..1000).collect() } else { vec![] }, 3, 10);
        assert_eq!(wide.len(), 10);
    }

    // ---- VARIANT 读写 ----

    #[test]
    fn variant_i4_roundtrip() {
        let mut v = variant_i4_from(-42);
        assert_eq!(variant_vt(&v), VT_I4);
        assert_eq!(variant_i4(&v), Some(-42));
        assert_eq!(variant_r8(&v), None); // 类型不符
        assert_eq!(variant_bstr(&v), None);
        unsafe { VariantClear(&mut v) };
    }

    #[test]
    fn variant_r8_roundtrip() {
        let mut v = variant_r8_from(3.5);
        assert_eq!(variant_vt(&v), VT_R8);
        assert_eq!(variant_r8(&v), Some(3.5));
        assert_eq!(variant_i4(&v), None);
        unsafe { VariantClear(&mut v) };
    }

    #[test]
    fn variant_bstr_read() {
        let wide: Vec<u16> = "你好ab".encode_utf16().collect();
        // 真实分配 BSTR(oleaut32),测 VT_BSTR 分支读取
        let b = unsafe { SysAllocStringLen(wide.as_ptr(), wide.len() as u32) };
        assert!(!b.is_null());
        let mut v = variant_bstr_from(b);
        assert_eq!(variant_vt(&v), VT_BSTR);
        assert_eq!(variant_bstr(&v), Some(b));
        assert_eq!(bstr_to_string(b), "你好ab");
        unsafe { VariantClear(&mut v) }; // 释放 BSTR
    }

    #[test]
    fn bstr_null_safe() {
        assert_eq!(bstr_to_string(std::ptr::null()), "");
    }

    #[test]
    fn variant_r8_array4_guard() {
        // 非 VT_ARRAY|VT_R8 类型 → None(真实 SAFEARRAY 布局留给手动验收实测)
        let mut v = variant_i4_from(1);
        assert!(variant_r8_array4(&v).is_none());
        unsafe { VariantClear(&mut v) };
    }

    // ---- 边界文案映射 ----

    #[test]
    fn access_error_mapping() {
        assert_eq!(map_access_error(E_ACCESSDENIED, true), MSG_UWP);
        assert_eq!(map_access_error(E_ACCESSDENIED, false), MSG_ADMIN);
        assert_eq!(map_access_error(E_ACCESSDENIED as i32, true), MSG_UWP);
        assert!(map_access_error(E_FAIL, false).contains("0x"));
    }

    #[test]
    fn boundary_messages_are_defined() {
        assert!(!MSG_UWP.is_empty());
        assert!(!MSG_ADMIN.is_empty());
        assert!(!MSG_NO_PROVIDER.is_empty());
        assert!(!MSG_NOT_FOUND.is_empty());
        assert!(!MSG_DISABLED.is_empty());
    }

    // ---- 中心点 ----

    #[test]
    fn center_computation() {
        assert_eq!(bounds_center((100, 200, 300, 400)), Some((200, 300)));
        assert_eq!(bounds_center((0, 0, 1, 1)), Some((0, 0)));
        assert_eq!(bounds_center((0, 0, 0, 0)), None); // 零矩形
        assert_eq!(bounds_center((10, 10, 10, 20)), None); // 宽为 0
        assert_eq!(bounds_center((20, 10, 10, 20)), None); // 反向
    }

    // ---- 控件类型映射 ----

    #[test]
    fn control_type_mapping() {
        assert_eq!(control_type_name(UIA_ButtonControlTypeId), "Button");
        assert_eq!(control_type_name(UIA_EditControlTypeId), "Edit");
        assert_eq!(control_type_name(UIA_WindowControlTypeId), "Window");
        assert_eq!(control_type_name(12345), "Custom"); // 未知类型兜底
        assert_eq!(control_type_id("button"), Some(UIA_ButtonControlTypeId));
        assert_eq!(control_type_id("EDIT"), Some(UIA_EditControlTypeId));
        assert_eq!(control_type_id("  ListItem  "), Some(UIA_ListItemControlTypeId));
        assert_eq!(control_type_id("nosuchtype"), None);
        // 常见类型名 → ID → 名往返
        for name in ["Button", "Edit", "CheckBox", "ComboBox", "MenuItem", "Tree", "Pane"] {
            assert_eq!(control_type_name(control_type_id(name).unwrap()), name);
        }
    }
}
