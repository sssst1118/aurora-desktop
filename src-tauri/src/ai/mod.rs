//! Phase3 AI 集成模块(3.1/3.2)。
//!
//! - `client`:双模式(DeepSeek/Ollama)HTTP 代理 + SSE 流式解析(3.1 独占);
//! - `tools`:function-call 工具清单/解析/路由/规则兜底,纯函数零 tauri 依赖(3.2 独占)。
//!
//! 集成收尾(2026-08-11):本文件为模块声明,接线时由集成 agent 创建。

pub mod client;
pub mod tools;
