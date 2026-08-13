//! 稳定性包(2026-08-13):开机自启动开关。
//!
//! 实现走系统 `reg.exe`(零新依赖,不碰 Cargo.toml):写
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 值名 "Aurora",
//! 数据 = 当前 exe 路径带引号(`std::env::current_exe` 取路径);
//! 删除用 `reg delete`,读取状态用 `reg query`(值不存在 = 未启用)。
//!
//! 状态以注册表实际值为准:`launch_set_startup` 返回 `reg query` 复核结果,
//! 前端再 invoke `launch_get_startup` 校准显示。
//!
//! 异常语义(审计场景):reg.exe 缺失 / 权限不足时,读取按"未启用"处理(不崩),
//! 设置返回 Err 文案(前端红字提示);关闭时值本就不存在(reg delete 报"找不到")
//! 同样视为成功——目标态(未启用)已达成。
//! 全部参数经 `Command::args([...])` 数组传参,路径不拼进 shell 命令,无注入面。

use std::path::Path;

/// 开机自启注册表 Run 键
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
/// 值名
const VALUE_NAME: &str = "Aurora";

/// 开机自启数据 = exe 路径带引号(纯函数,可单测)
pub fn startup_value_data(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// reg query 结果 → 是否已启用(纯函数,可单测):
/// 退出码 0 = 值存在 = 已启用;非 0(含 reg.exe 缺失导致的 spawn 失败)= 未启用。
fn query_result_enabled(code: Option<i32>) -> bool {
    code == Some(0)
}

/// 读取开机自启状态(注册表为准);reg.exe 缺失/查询失败一律按未启用(不报错)
pub fn get_startup() -> bool {
    query_result_enabled(
        std::process::Command::new("reg")
            .args(["query", RUN_KEY, "/v", VALUE_NAME])
            .output()
            .ok()
            .and_then(|o| o.status.code()),
    )
}

/// 设置开机自启;失败(reg.exe 缺失/权限不足等)返回 Err 文案(前端红字)。
/// 关闭时值本就不存在(reg delete 报"找不到")视为成功(目标态已达成)。
pub fn set_startup(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取程序路径: {e}"))?;
    if enabled {
        let data = startup_value_data(&exe);
        let out = std::process::Command::new("reg")
            .args(["add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &data, "/f"])
            .output()
            .map_err(|e| format!("无法启动 reg.exe(设置开机自启动失败): {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let detail = String::from_utf8_lossy(&out.stderr);
            Err(format!("写入注册表失败: {}", detail.trim()))
        }
    } else {
        let out = std::process::Command::new("reg")
            .args(["delete", RUN_KEY, "/v", VALUE_NAME, "/f"])
            .output();
        match out {
            Ok(o) if o.status.success() => Ok(()),
            _ => {
                // 删除失败(含值不存在/reg.exe 缺失):以注册表实际状态为准,
                // 已处于未启用即视为成功
                if get_startup() {
                    Err("删除注册表启动项失败,请检查权限".to_string())
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// 命令:设置开机自启(前端契约);返回注册表实际状态供前端校准
#[tauri::command]
pub fn launch_set_startup(enabled: bool) -> Result<bool, String> {
    set_startup(enabled)?;
    Ok(get_startup())
}

/// 命令:读取开机自启状态(注册表为准)
#[tauri::command]
pub fn launch_get_startup() -> bool {
    get_startup()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_data_quotes_exe_path() {
        assert_eq!(
            startup_value_data(Path::new(r"C:\Program Files\Aurora\Aurora.exe")),
            r#""C:\Program Files\Aurora\Aurora.exe""#
        );
        // 无空格路径同样带引号(注册表 Run 键统一带引号格式)
        assert_eq!(startup_value_data(Path::new(r"D:\apps\aurora.exe")), r#""D:\apps\aurora.exe""#);
    }

    #[test]
    fn query_exit_code_zero_means_enabled() {
        assert!(query_result_enabled(Some(0)));
        assert!(!query_result_enabled(Some(1)), "值不存在(reg query 退出码 1)");
        assert!(!query_result_enabled(Some(5)), "权限不足");
        assert!(!query_result_enabled(None), "reg.exe 缺失(spawn 失败)");
    }
}
