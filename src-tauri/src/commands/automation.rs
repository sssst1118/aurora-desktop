//! Phase4 4.2 键鼠模拟自动化命令层。
//!
//! - 六个命令:`automation_sim_click/move/scroll/key/type/input`(input 为开发文档 §5
//!   契约名,与 type 同一实现);
//! - 统一入口校验 `can_use_automation`(enable_automation 总开关,默认关),关闭即 Err;
//! - 点击间隔守卫 `check_click_interval`(automation_click_delay_ms 默认 80,防连点风暴),
//!   上次点击时间存 `static Mutex<Option<Instant>>`,零常驻线程;
//! - 命令层只做校验与状态维护,实际注入全部走 `crate::automation::input_sim`(公共契约,
//!   4.3 复用 click_at/type_text);
//! - 每次请求从磁盘读配置(不缓存,运行中改配置下次请求生效,同 ai.rs 模式)。

use crate::commands::config::AppConfig;
use std::sync::Mutex;
use std::time::Instant;
use tauri::AppHandle;

/// 自动化总开关关闭时的错误文案(设计文档 §2.1/§3.4,前端红字展示)
const AUTOMATION_DISABLED_MSG: &str = "自动化未启用,请在设置中开启";

/// 上次点击时间(命令层维护;仅 click 命令受间隔守卫约束)
static LAST_CLICK: Mutex<Option<Instant>> = Mutex::new(None);

/// 每次请求从磁盘读配置(不缓存;同 ai.rs 模式)
fn load_cfg(app: &AppHandle) -> AppConfig {
    let path = crate::commands::config::config_path(app);
    crate::commands::config::load_from(&path)
}

/// 入口校验纯函数:自动化总开关关闭 → Err(设计文档 §2.1;可单测)
fn can_use_automation(cfg: &AppConfig) -> Result<(), String> {
    if cfg.enable_automation {
        Ok(())
    } else {
        Err(AUTOMATION_DISABLED_MSG.to_string())
    }
}

/// 点击间隔守卫纯函数:距上次点击 < automation_click_delay_ms → 拒绝(防连点风暴;可单测)。
/// `automation_click_delay_ms == 0` 视为守卫关闭(0 恒放行)。
fn check_click_interval(last: Option<Instant>, cfg: &AppConfig) -> Result<(), String> {
    let delay = cfg.automation_click_delay_ms;
    let Some(last) = last else {
        return Ok(()); // 首次点击无历史
    };
    if delay > 0 && last.elapsed() < std::time::Duration::from_millis(delay as u64) {
        return Err(format!("点击过于频繁,请间隔 {delay}ms 以上"));
    }
    Ok(())
}

/// 点击命令:总开关 → 间隔守卫 → 注入;成功后才更新上次点击时间(失败可立即重试)
#[tauri::command]
pub fn automation_sim_click(
    app: tauri::AppHandle,
    x: i32,
    y: i32,
    button: Option<String>,
) -> Result<(), String> {
    let cfg = load_cfg(&app);
    can_use_automation(&cfg)?;
    let btn = button.as_deref().unwrap_or("left");
    let mut guard = LAST_CLICK.lock().map_err(|_| "内部状态错误,请重启应用".to_string())?;
    check_click_interval(*guard, &cfg)?;
    crate::automation::input_sim::click_at(x, y, btn)?;
    *guard = Some(Instant::now());
    Ok(())
}

/// 移动鼠标到绝对屏幕坐标
#[tauri::command]
pub fn automation_sim_move(app: tauri::AppHandle, x: i32, y: i32) -> Result<(), String> {
    let cfg = load_cfg(&app);
    can_use_automation(&cfg)?;
    crate::automation::input_sim::move_to(x, y)
}

/// 滚轮:delta>0 向下滚动(Windows 语义),可选先移动鼠标到 (x,y)
#[tauri::command]
pub fn automation_sim_scroll(
    app: tauri::AppHandle,
    delta: i32,
    x: Option<i32>,
    y: Option<i32>,
) -> Result<(), String> {
    let cfg = load_cfg(&app);
    can_use_automation(&cfg)?;
    crate::automation::input_sim::scroll(delta, x, y)
}

/// 按键/组合键:key 见键盘表,modifiers = ["ctrl","shift","alt","win"] 子集
#[tauri::command]
pub fn automation_sim_key(
    app: tauri::AppHandle,
    key: String,
    modifiers: Option<Vec<String>>,
) -> Result<(), String> {
    let cfg = load_cfg(&app);
    can_use_automation(&cfg)?;
    let mods: Vec<&str> = modifiers.as_deref().unwrap_or(&[]).iter().map(String::as_str).collect();
    crate::automation::input_sim::press_key(&key, &mods)
}

/// Unicode 文本输入(非 ASCII 安全;不碰剪贴板)
#[tauri::command]
pub fn automation_sim_type(app: tauri::AppHandle, text: String) -> Result<(), String> {
    type_cmd_impl(&app, &text)
}

/// 开发文档 §5 兼容契约名:与 automation_sim_type 同一实现
#[tauri::command]
pub fn automation_sim_input(app: tauri::AppHandle, text: String) -> Result<(), String> {
    type_cmd_impl(&app, &text)
}

/// type/input 两命令共用实现
fn type_cmd_impl(app: &AppHandle, text: &str) -> Result<(), String> {
    let cfg = load_cfg(app);
    can_use_automation(&cfg)?;
    crate::automation::input_sim::type_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn can_use_automation_on_ok() {
        let cfg = AppConfig { enable_automation: true, ..AppConfig::default() };
        assert!(can_use_automation(&cfg).is_ok());
    }

    #[test]
    fn can_use_automation_off_err() {
        let cfg = AppConfig { enable_automation: false, ..AppConfig::default() };
        let err = can_use_automation(&cfg).unwrap_err();
        assert_eq!(err, "自动化未启用,请在设置中开启");
    }

    #[test]
    fn click_interval_first_click_allowed() {
        let cfg = AppConfig::default(); // 默认 80ms
        assert!(check_click_interval(None, &cfg).is_ok());
    }

    #[test]
    fn click_interval_too_fast_rejected() {
        let cfg = AppConfig { automation_click_delay_ms: 80, ..AppConfig::default() };
        // 上一次点击就在刚刚(elapsed ~0 < 80)→ 拒绝
        let err = check_click_interval(Some(Instant::now()), &cfg).unwrap_err();
        assert!(err.contains("点击过于频繁"), "错误文案应提示间隔: {err}");
        assert!(err.contains("80"));
    }

    #[test]
    fn click_interval_slow_enough_allowed() {
        let cfg = AppConfig { automation_click_delay_ms: 80, ..AppConfig::default() };
        // 100ms 前点过,超过 80ms 阈值 → 放行
        let past = Some(Instant::now() - Duration::from_millis(100));
        assert!(check_click_interval(past, &cfg).is_ok());
    }

    #[test]
    fn click_interval_delay_zero_guard_off() {
        // delay=0 = 守卫关闭,任意间隔放行(含刚点过)
        let cfg = AppConfig { automation_click_delay_ms: 0, ..AppConfig::default() };
        assert!(check_click_interval(Some(Instant::now()), &cfg).is_ok());
    }

    #[test]
    fn click_interval_threshold_boundary() {
        // 恰好达到阈值(>= delay)也放行
        let cfg = AppConfig { automation_click_delay_ms: 5, ..AppConfig::default() };
        let past = Some(Instant::now() - Duration::from_millis(5));
        assert!(check_click_interval(past, &cfg).is_ok());
    }
}
