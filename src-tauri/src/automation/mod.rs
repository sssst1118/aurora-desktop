// Phase4 4.2/4.3 自动化模块容器(共享文件,由集成 agent 维护)。
// 子模块声明随模块合入逐个添加:
//   4.2 合入时 -> pub mod input_sim;
//   4.3 合入时 -> pub mod ui_automation_wrap; pub mod uia_cmd;
pub mod input_sim; // 4.2 键鼠模拟(SendInput)
pub mod ui_automation_wrap; // 4.3 Uia* 封装(命令层在 commands/uia_cmd.rs)
