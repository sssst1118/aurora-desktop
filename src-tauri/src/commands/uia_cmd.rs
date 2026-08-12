//! Phase4 4.3 UI-Automation 命令层(六命令,全部 #[tauri::command])。
//!
//! 入口统一校验双开关:`enable_automation && automation_uia_enable`(§3.2)。
//! 点击/输入复用 4.2 input_sim 公共契约(click_at / type_text,§2.2)。
//! 全部为瞬态调用,零常驻线程。

use tauri::AppHandle;

use crate::automation::input_sim;
use crate::automation::ui_automation_wrap::{
    self, bounds_center, UiaControl, UiaWindow, MSG_DISABLED, MSG_NOT_FOUND,
};
use crate::commands::config::{config_path, load_from, AppConfig};

/// 双开关校验(纯函数,§3.4 文案写死):`enable_automation && automation_uia_enable`
pub fn can_use_uia(cfg: &AppConfig) -> Result<(), String> {
    if cfg.enable_automation && cfg.automation_uia_enable {
        Ok(())
    } else {
        Err(MSG_DISABLED.to_string())
    }
}

fn load_cfg(app: &AppHandle) -> AppConfig {
    load_from(&config_path(app))
}

/// 按标题子串枚举可见顶层窗口
#[tauri::command]
pub fn uia_find_window(app: AppHandle, title: String) -> Result<Vec<UiaWindow>, String> {
    can_use_uia(&load_cfg(&app))?;
    ui_automation_wrap::find_top_windows(&title)
}

/// 按 hwnd 单查窗口详情
#[tauri::command]
pub fn uia_get_window_info(app: AppHandle, hwnd: i64) -> Result<UiaWindow, String> {
    can_use_uia(&load_cfg(&app))?;
    ui_automation_wrap::get_window_info(hwnd)
}

/// 从指定窗口根遍历 UIA 控件树(限深 3、上限 200),可按控件类型/名称过滤
#[tauri::command]
pub fn uia_find_controls(
    app: AppHandle,
    hwnd: i64,
    control_type: Option<String>,
    name: Option<String>,
) -> Result<Vec<UiaControl>, String> {
    can_use_uia(&load_cfg(&app))?;
    ui_automation_wrap::find_controls(
        hwnd,
        control_type.as_deref().unwrap_or(""),
        name.as_deref().unwrap_or(""),
    )
}

/// 读控件文本(Name 优先,回退 Value)
#[tauri::command]
pub fn uia_get_control_text(app: AppHandle, hwnd: i64, control_id: String) -> Result<String, String> {
    can_use_uia(&load_cfg(&app))?;
    ui_automation_wrap::get_control_text(hwnd, &control_id)
}

/// 点击控件:取 BoundingRectangle 中心 → 4.2 input_sim::click_at(坐标与 input_sim 同系,直接可用)
#[tauri::command]
pub fn uia_click_control(app: AppHandle, hwnd: i64, control_id: String) -> Result<(), String> {
    can_use_uia(&load_cfg(&app))?;
    let bounds = ui_automation_wrap::get_control_bounds(hwnd, &control_id)?;
    let (cx, cy) = bounds_center(bounds).ok_or_else(|| MSG_NOT_FOUND.to_string())?;
    input_sim::click_at(cx, cy, "left")
}

/// 向控件输入文本:UiaSetFocus 尝试聚焦;未获焦(控件不支持/被 UIPI 拦截)→ 点击中心兜底 → 4.2 input_sim::type_text
#[tauri::command]
pub fn uia_type_into(
    app: AppHandle,
    hwnd: i64,
    control_id: String,
    text: String,
) -> Result<(), String> {
    can_use_uia(&load_cfg(&app))?;
    let focused = ui_automation_wrap::set_focus(hwnd, &control_id)?;
    if !focused {
        // Select 兜底:点击控件中心让它拿到焦点(失败不阻断,文本注入结果由 UIPI 决定)
        if let Ok(bounds) = ui_automation_wrap::get_control_bounds(hwnd, &control_id) {
            if let Some((cx, cy)) = bounds_center(bounds) {
                let _ = input_sim::click_at(cx, cy, "left");
            }
        }
    }
    input_sim::type_text(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_use_uia_requires_both_switches() {
        let mut cfg = AppConfig::default();
        cfg.enable_automation = false;
        cfg.automation_uia_enable = false;
        assert!(can_use_uia(&cfg).is_err());
        assert_eq!(can_use_uia(&cfg).unwrap_err(), MSG_DISABLED);

        cfg.enable_automation = true;
        cfg.automation_uia_enable = false;
        assert!(can_use_uia(&cfg).is_err()); // 总开关开、UIA 子开关关 → 拒绝

        cfg.enable_automation = false;
        cfg.automation_uia_enable = true;
        assert!(can_use_uia(&cfg).is_err()); // 总开关关 → 拒绝

        cfg.enable_automation = true;
        cfg.automation_uia_enable = true;
        assert!(can_use_uia(&cfg).is_ok());
    }
}
