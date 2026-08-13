//! 稳定性包(2026-08-13):首次启动引导。
//!
//! 首次启动(配置 `first_run_done=false`)时自动呼出一次搜索框窗口——前端空态
//! 自带"输入关键词搜索应用、文件"引导文案——并在托盘 tooltip 提示
//! "Ctrl+Shift+Space 呼出搜索";用户关闭搜索框后落盘 `first_run_done=true`,
//! 下次启动不再引导。
//!
//! "关闭"的判定(事件驱动,不轮询):搜索框是 hide 语义(不是销毁),Tauri 没有
//! "Hidden"窗口事件,故监听 `WindowEvent::Focused(false)`——用户按 Esc / 打开条目
//! 关闭窗口时窗口必先失焦——再延迟 500ms 复核 `is_visible`(消除"失焦事件先于
//! hide 生效"的执行时序竞态):不可见 = 用户已关闭;仍可见 = 只是点去了别的窗口,
//! 不算关闭。
//!
//! 兜底:任意一次 config_save 也会置位 first_run_done(用户已找到设置页,见
//! commands/config.rs),防窗口关闭事件丢失导致每次启动重复引导。
//!
//! 判定/迁移为纯函数([`is_first_run`] / [`complete_first_run`])带单测;
//! 落盘经 [`mark_done_at`] 可单测(临时路径),命令侧 [`mark_done`] 持配置锁调用。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, WindowEvent};

use crate::commands::config::{self, AppConfig};

/// 引导进行中标志:窗口焦点事件只在本标志为真时参与"关闭即完成"判定,
/// 引导完成后每次失焦不再起延迟复核线程
static GUIDE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 失焦后复核窗口可见性的延迟(等 hide 生效;毫秒)
const HIDE_CHECK_DELAY_MS: u64 = 500;

/// 是否应展示首次引导(纯函数,可单测):first_run_done 为 false 即未完成
pub fn is_first_run(cfg: &AppConfig) -> bool {
    !cfg.first_run_done
}

/// 把配置置为"引导已完成"(纯函数,可单测):返回是否发生了状态迁移(幂等)
pub fn complete_first_run(cfg: &mut AppConfig) -> bool {
    if cfg.first_run_done {
        false
    } else {
        cfg.first_run_done = true;
        true
    }
}

/// 落盘 first_run_done=true(不取配置锁——由调用方按需持锁;单测直接传临时路径)。
/// 已完成的配置不重复写盘
fn mark_done_at(path: &Path) -> bool {
    let mut cfg = config::load_from(path);
    if complete_first_run(&mut cfg) {
        config::save_to(path, &cfg)
    } else {
        true
    }
}

/// 引导完成收尾:落盘标记 + 撤下托盘提示 + 复位进行中标志(幂等,可重复调用)。
/// 落盘失败不视为错误(下次启动重新引导即可),但提示照撤——用户已关窗,
/// 提示没有继续展示的意义
pub fn mark_done(app: &AppHandle) -> bool {
    GUIDE_ACTIVE.store(false, Ordering::SeqCst);
    crate::tray::set_first_run_hint(false);
    let path = config::config_path(app);
    // 与 config_save / search_save_geometry / dock 条目共用配置锁,读-改-写原子
    let _guard = config::config_lock().lock().unwrap_or_else(|p| p.into_inner());
    let ok = mark_done_at(&path);
    if ok {
        crate::logger::log_event("INFO", "首次启动引导完成");
    }
    ok
}

/// 呼出引导:搜索框 + 托盘提示 + "关闭即完成"监听。
/// 只做引导展示,任何单项失败都不影响应用其余启动流程
pub fn start(app: &AppHandle) {
    crate::logger::log_event("INFO", "首次启动引导:呼出搜索框");
    GUIDE_ACTIVE.store(true, Ordering::SeqCst);
    // 1) 呼出搜索框(与热键呼出同款置顶/前台手法,空态自带引导文案)
    crate::win_utils::show_search_window(app);
    // 2) 托盘 tooltip 提示快捷键(优先于 2s 一次的系统状态 tooltip,见 tray.rs)
    crate::tray::set_first_run_hint(true);
    crate::tray::set_first_run_tooltip(app);
    // 3) 关闭即完成:失焦 → 延迟复核可见性(见模块注释的判定说明)
    let Some(win) = app.get_webview_window("search") else {
        return;
    };
    let app2 = app.clone();
    let _ = win.on_window_event(move |event| {
        if !matches!(event, WindowEvent::Focused(false)) || !GUIDE_ACTIVE.load(Ordering::SeqCst) {
            return;
        }
        let app3 = app2.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(HIDE_CHECK_DELAY_MS));
            let still_visible = app3
                .get_webview_window("search")
                .map(|w| w.is_visible().unwrap_or(true))
                .unwrap_or(true);
            if !still_visible {
                mark_done(&app3);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_first_run_false_when_not_done() {
        let cfg = AppConfig::default(); // first_run_done 默认 false
        assert!(is_first_run(&cfg));
        let done = AppConfig {
            first_run_done: true,
            ..AppConfig::default()
        };
        assert!(!is_first_run(&done));
    }

    #[test]
    fn complete_first_run_transitions_once() {
        let mut cfg = AppConfig::default();
        assert!(complete_first_run(&mut cfg), "首次迁移应发生");
        assert!(cfg.first_run_done);
        assert!(!complete_first_run(&mut cfg), "再次迁移应为 no-op(幂等)");
    }

    #[test]
    fn mark_done_at_persists_flag() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("aurora_first_run_{nanos}.json"));
        let _ = std::fs::remove_file(&p);
        // 初始配置未完成 → 落盘后完成
        assert!(config::save_to(&p, &AppConfig::default()));
        assert!(mark_done_at(&p));
        assert!(config::load_from(&p).first_run_done);
        // 已完成 → 再调用仍返回 true(不写盘也不报错)
        assert!(mark_done_at(&p));
        assert!(config::load_from(&p).first_run_done);
        let _ = std::fs::remove_file(&p);
    }
}
