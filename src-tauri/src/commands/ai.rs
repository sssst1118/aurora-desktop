//! 3.1 AI 对话命令层(无状态后端,设计文档 §1.2/§2.3)。
//!
//! 命令:
//!   - `ai_chat_completion`:非流式单次对话(含工具循环);
//!   - `ai_chat_stream`:SSE 流式对话,spawn 异步任务逐行 emit `ai-event`
//!     (chunk/tool/done/error 四类,事件契约 §0.3),任务结束即回收;
//!   - `ai_execute_tool`:单轮工具执行(3.2 契约入口):带 tools 对话 → 有 tool_call
//!     执行第一个 → `{ok, action, msg}`;无 tool_call → 规则兜底(rule_match);
//!     兜底失败 → `{ok:false, action:"none", msg: 模型原文}`。
//!
//! 工具循环 `run_tool_loop`(§2.3):≤ ai_max_tool_rounds(默认 3)轮带 tools 请求,
//! parse_tool_calls → route 纯函数路由 → 命令层 match 执行真实命令 → 结果 JSON
//! 回填 role:"tool"(OpenAI 回填格式含 tool_call_id)→ 再请求;超限终止并追加提示。
//! 循环与工具执行均经 trait/闭包注入,假 client 可单测(§2.6 循环上限)。
//!
//! H3 安全加固(2026-08-13):
//!   - 危险工具(open_item,见 tools::DANGEROUS_TOOLS)执行前经 confirm 闭包询问用户:
//!     emit ai-tool-confirm → await ai_confirm_tool 回传(≤60s 超时视为拒绝);
//!     拒绝/超时 → 跳过该工具,回填"用户已拒绝执行 xxx"继续下一轮;
//!   - 剪贴板历史工具仅本地 Ollama 模式可用;云端(DeepSeek)模式直接回隐私提示,
//!     剪贴板内容绝不进入发往云端的下一轮 prompt。
//!
//! 配置:每次请求从磁盘读(不缓存,运行中改配置下次请求生效);密钥只从配置读,
//! 命令入参不含 key(铁律 §0.4);deepseek 模式 key 缺失在发请求前快速报错。
//! 日志/错误信息不含密钥(仅 base_url + 状态码,见 ai/client.rs)。

use crate::ai::client::{classify_error, AiErrorKind};
use crate::ai::tools::{
    danger_summary, is_dangerous_tool, parse_tool_calls, route, rule_match, tools_json,
    DangerRequest, ToolAction,
};
use crate::commands::config::AppConfig;
use crate::indexer::app_index::AppEntry;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use tauri::AppHandle;

/// 后端与前端 TS 一一对应的对话消息(role: "user"/"assistant"/"system";工具回填在内部转 Value)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// ai_execute_tool 返回契约(开发文档 §5):{ok, action, msg}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiToolResult {
    pub ok: bool,
    /// "open_item"/"set_wallpaper"/"search_files"/"search_apps"/"get_system_status"/"get_clipboard_history"/"none"
    pub action: String,
    /// 执行结果摘要(成功:如"已打开 记事本";失败/未匹配:模型原文或错误)
    pub msg: String,
}

/// 发往后端消息数组截断上限(§1.4 会话管理:前端主截断 + 后端兜底)
const MAX_MESSAGES: usize = 40;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ==================== 事件与配置 ====================

fn emit_ai(app: &AppHandle, payload: Value) {
    use tauri::Emitter;
    let _ = app.emit("ai-event", payload);
}

/// 每次请求从磁盘读配置(不缓存,运行中改配置下次请求生效)
fn load_cfg(app: &AppHandle) -> AppConfig {
    let path = crate::commands::config::config_path(app);
    crate::commands::config::load_from(&path)
}

/// 消息截断:保留最近 40 条(纯函数,可单测;前端主截断,后端兜底防超长请求体)
fn truncate_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let n = messages.len();
    if n <= MAX_MESSAGES {
        messages
    } else {
        messages[n - MAX_MESSAGES..].to_vec()
    }
}

/// 解析后的连接目标(双模式差异已收敛;密钥在此解析,之后只注入 client)
#[derive(Debug, Clone, PartialEq)]
struct Endpoint {
    provider: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

/// 双模式端点解析(纯函数):deepseek 带密钥(缺失 → MissingKey);ollama 无认证。
/// 安全加固:model/base_url 只从配置读取,不再接受命令入参覆盖——
/// 此前 base_url 可由前端传入而密钥仍从配置注入,恶意前端可把 key 引到攻击者服务器
fn resolve_endpoint(cfg: &AppConfig) -> Result<Endpoint, AiErrorKind> {
    let provider = cfg.ai_provider.trim().to_string();
    if provider.is_empty() {
        return Err(AiErrorKind::Other("AI 服务商未配置,请在设置中选择".to_string()));
    }
    let is_ollama = provider == "ollama";
    let model = if is_ollama { &cfg.ai_ollama_model } else { &cfg.ai_model };
    let base_url = if is_ollama { &cfg.ai_ollama_url } else { &cfg.ai_base_url };
    let api_key = if is_ollama {
        None
    } else {
        let k = cfg
            .ai_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or(AiErrorKind::MissingKey)?;
        Some(k.to_string())
    };
    Ok(Endpoint { provider, base_url: base_url.trim().to_string(), model: model.trim().to_string(), api_key })
}

// ==================== 工具循环(§2.3) ====================

/// 单轮对话请求抽象(工具循环依赖的唯一接口;假 client 注入可单测 §2.6)。
pub trait ChatClient: Send + Sync {
    /// 一次对话请求;stream=true 时增量经 on_delta 回调,返回完整回复文本。
    /// 'a 同时约束 &self 与 on_delta:返回 future 借两者,调用方保证两者活到 future 完成。
    fn chat<'a>(
        &'a self,
        messages: Vec<Value>,
        tools: Option<Value>,
        stream: bool,
        on_delta: &'a mut (dyn FnMut(String) + Send),
    ) -> BoxFuture<'a, Result<String, AiErrorKind>>;
}

/// 真实客户端:包 ai/client.rs 的 chat(唯一网络出口,密钥已随 Endpoint 注入)
struct RealClient<'a> {
    provider: &'a str,
    base_url: &'a str,
    model: &'a str,
    api_key: Option<&'a str>,
}

impl ChatClient for RealClient<'_> {
    fn chat<'a>(
        &'a self,
        messages: Vec<Value>,
        tools: Option<Value>,
        stream: bool,
        on_delta: &'a mut (dyn FnMut(String) + Send),
    ) -> BoxFuture<'a, Result<String, AiErrorKind>> {
        Box::pin(crate::ai::client::chat(
            self.provider,
            self.base_url,
            self.model,
            self.api_key,
            messages,
            tools,
            stream,
            on_delta,
        ))
    }
}

/// 工具执行器:ToolAction → (工具名, 结果摘要);Err = 工具失败(失败隔离:回填 ok:false 给模型)。
/// 要求 Send + Sync(流任务在 spawn 线程跨 await 持有 &dyn Fn)。
type ToolExecutor<'a> = &'a (dyn Fn(ToolAction) -> BoxFuture<'static, Result<(String, String), String>> + Send + Sync);

/// 危险工具确认器:H3 安全加固,危险工具执行前询问用户(返回 true 才执行)。
/// 命令层实现 = emit ai-tool-confirm + await ai_confirm_tool(≤60s,超时/拒绝均返回 false)。
/// 要求 Send + Sync(同 ToolExecutor)。
type ToolConfirmer<'a> = &'a (dyn Fn(DangerRequest) -> BoxFuture<'static, bool> + Send + Sync);

/// 是否本地服务商(纯函数,H3 安全加固):剪贴板历史等隐私工具仅本地可用,云端一律禁用。
/// provider 来自 resolve_endpoint(已 trim),此处再 trim + 忽略大小写双重防御。
fn is_local_provider(provider: &str) -> bool {
    provider.trim().eq_ignore_ascii_case("ollama")
}

/// 云端模式剪贴板工具的统一拦截提示(隐私红线:剪贴板内容绝不进云端 prompt)
const CLOUD_CLIPBOARD_BLOCKED: &str = "云端模式为保护隐私不读取剪贴板历史";

/// 剪贴板历史工具在指定服务商下的拦截决策(纯函数,可单测两种模式):
/// None = 本地模式,允许读取;Some(hint) = 云端模式,回填隐私提示。
fn clipboard_block_hint(provider: &str) -> Option<&'static str> {
    if is_local_provider(provider) {
        None
    } else {
        Some(CLOUD_CLIPBOARD_BLOCKED)
    }
}

/// 工具循环(§2.3 骨架):
///   每轮带 tools 请求 → parse_tool_calls → 无工具直接返回文本;
///   有工具 → on_tool 通知(emit 动作条)→ route 路由 → [H3]危险工具经 confirm 确认
///   (拒绝/超时回填"用户已拒绝执行 xxx",跳过执行)→ exec_tool 执行 →
///   结果 JSON 回填 role:"tool"(含 tool_call_id)→ 再请求;
///   超限(max_rounds 轮)终止并追加"已停止"提示;
///   tools_enabled=false → 不带 tools 参数,纯单轮对话。
async fn run_tool_loop(
    client: &dyn ChatClient,
    tools_enabled: bool,
    max_rounds: u32,
    stream: bool,
    mut msgs: Vec<Value>,
    exec_tool: ToolExecutor<'_>,
    confirm: ToolConfirmer<'_>,
    on_tool: &mut (dyn FnMut(&str, &str) + Send),
    on_delta: &mut (dyn FnMut(String) + Send),
) -> Result<String, AiErrorKind> {
    if !tools_enabled || max_rounds == 0 {
        return client.chat(msgs, None, stream, on_delta).await;
    }
    let mut last_reply = String::new();
    for _round in 0..max_rounds {
        let reply = client
            .chat(msgs.clone(), Some(tools_json()), stream, on_delta)
            .await?;
        last_reply = reply.clone();
        let Some(calls) = parse_tool_calls(&reply) else {
            return Ok(reply); // 无工具 → 直接返回文本
        };
        for call in calls {
            on_tool(&call.name, &call.arguments.to_string());
            let result = match route(&call.name, &call.arguments) {
                Ok(action) => {
                    // H3 安全加固:危险工具(open_item 等)执行前必须经用户确认;
                    // 拒绝/超时 → 回填"用户已拒绝执行 xxx"给模型(记入后续上下文),不执行
                    let approved = if is_dangerous_tool(&call.name) {
                        confirm(DangerRequest {
                            tool_call_id: call.id.clone(),
                            tool: call.name.clone(),
                            summary: danger_summary(&call.name, &action),
                        })
                        .await
                    } else {
                        true
                    };
                    if !approved {
                        json!({
                            "ok": false,
                            "action": call.name,
                            "msg": format!("用户已拒绝执行 {}", call.name),
                        })
                    } else {
                        match exec_tool(action).await {
                            Ok((name, msg)) => json!({"ok": true, "action": name, "msg": msg}),
                            Err(e) => json!({"ok": false, "action": "none", "msg": e}),
                        }
                    }
                }
                Err(e) => json!({"ok": false, "action": "none", "msg": e}),
            };
            msgs.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": result.to_string(),
            }));
        }
    }
    Ok(format!("{last_reply}\n\n(已停止:工具调用超过 {max_rounds} 轮上限)"))
}

// ==================== 工具执行分支(薄:只 match + 调真实命令) ====================

/// 执行分支:对 ToolAction match 真实命令,结果拼回填摘要。
/// 返回 (工具名, 结果摘要);Err = 工具执行失败(失败隔离:回填 ok:false 给模型,对话继续)。
/// app/provider 按 owned 传入:执行器闭包需要返回 'static future(owned 才能跨 spawn 线程持有)。
async fn exec_tool_action(
    app: AppHandle,
    provider: String,
    action: ToolAction,
) -> Result<(String, String), String> {
    use tauri::Manager;
    match action {
        ToolAction::Open { path } => {
            let ok = crate::commands::search::open_item(path.clone());
            let msg = if ok {
                format!("已打开 {}", display_path(&path))
            } else {
                format!("打开失败: {path}")
            };
            Ok(("open_item".to_string(), msg))
        }
        ToolAction::SearchApps { query } => {
            let idx = app.state::<Mutex<crate::indexer::app_index::AppIndex>>();
            let entries = idx.lock().map(|g| g.search(&query)).unwrap_or_default();
            Ok(("search_apps".to_string(), summarize_apps(&entries)))
        }
        ToolAction::SearchFiles { query, dirs } => {
            let dirs_arg = if dirs.is_empty() { None } else { Some(dirs) };
            let hits = crate::commands::file_search::ai_search_files(query, dirs_arg).await;
            Ok(("search_files".to_string(), summarize_hits(&hits)))
        }
        ToolAction::SetWallpaper { file_path } => {
            match crate::commands::wallpaper::wallpaper_set_static_cmd(file_path) {
                Ok(()) => Ok(("set_wallpaper".to_string(), "壁纸已更换".to_string())),
                Err(e) => Err(e),
            }
        }
        ToolAction::GetSystemStatus => {
            let s = crate::commands::system::sys_get_status(app.clone());
            Ok(("get_system_status".to_string(), fmt_sys_status(&s)))
        }
        ToolAction::GetClipboardHistory => {
            // H3 安全加固:云端模式(DeepSeek 等)为保护隐私不读剪贴板历史——
            // 历史(最多 200 条)会随工具结果拼入下一轮 prompt 外发云端,敏感文本外泄;
            // 云端下直接回隐私提示(不读剪贴板),仅本地 Ollama 模式可用
            if let Some(hint) = clipboard_block_hint(&provider) {
                return Ok(("get_clipboard_history".to_string(), hint.to_string()));
            }
            let items = crate::commands::clipboard::clipboard_get_history(app.clone());
            Ok(("get_clipboard_history".to_string(), fmt_clipboard_summary(&items)))
        }
        ToolAction::SetDynamicWallpaper { path, url } => {
            // 4.1 set 契约仅收本地素材路径;远端网页素材暂不支持(需先下载到本地)
            if path.is_empty() {
                let hint = url.as_deref().unwrap_or("");
                return Err(format!(
                    "动态壁纸需要本地素材路径,网页素材请先下载到本地: {hint}"
                ));
            }
            let info = crate::commands::wallpaper_dynamic::wallpaper_dynamic_set(app, path)?;
            Ok(("set_dynamic_wallpaper".to_string(), format!("动态壁纸已设置: {}", display_path(&info.path))))
        }
        ToolAction::StopDynamicWallpaper => {
            crate::commands::wallpaper_dynamic::wallpaper_dynamic_clear(app)?;
            Ok(("stop_dynamic_wallpaper".to_string(), "已恢复系统壁纸".to_string()))
        }
    }
}

/// 展示用路径:优先文件名(如"已打开 记事本");无文件名退回全路径
fn display_path(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| p.to_string())
}

/// 应用搜索结果摘要:前 5 条 `名称 | 路径`;空 → "未找到匹配应用"(纯函数)
fn summarize_apps(entries: &[AppEntry]) -> String {
    if entries.is_empty() {
        return "未找到匹配应用".to_string();
    }
    entries
        .iter()
        .take(5)
        .map(|e| format!("{} | {}", e.name, e.path))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 文件搜索结果摘要:前 5 条 `名称 | 路径`;空 → "未找到文件"(纯函数)
fn summarize_hits(hits: &[crate::commands::file_search::FileHit]) -> String {
    if hits.is_empty() {
        return "未找到文件".to_string();
    }
    hits.iter()
        .take(5)
        .map(|h| format!("{} | {}", h.name, h.full_path))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 系统状态格式化(§2.2 契约):CPU 12% / 内存 5.8G/16G / 上行 1.2M/s 下行 3.4M/s(纯函数)
fn fmt_sys_status(s: &crate::commands::system::SysStatus) -> String {
    format!(
        "CPU {:.0}% / 内存 {}/{} / 上行 {} 下行 {}",
        s.cpu,
        fmt_memory(s.mem_used_mb),
        fmt_memory(s.mem_total_mb),
        fmt_speed(s.net_tx_bps),
        fmt_speed(s.net_rx_bps),
    )
}

/// 内存显示:≥1024MB → "5.8G";否则 "512M"(纯函数)
fn fmt_memory(mb: u64) -> String {
    if mb >= 1024 {
        format!("{:.1}G", mb as f64 / 1024.0)
    } else {
        format!("{mb}M")
    }
}

/// 速率显示:≥1MB/s → "1.2M/s";≥1KB/s → "3.4K/s";否则 "0B/s"(纯函数)
fn fmt_speed(bps: u64) -> String {
    if bps >= 1_048_576 {
        format!("{:.1}M/s", bps as f64 / 1_048_576.0)
    } else if bps >= 1024 {
        format!("{:.0}K/s", bps as f64 / 1024.0)
    } else {
        format!("{bps}B/s")
    }
}

/// 剪贴板历史摘要(§2.2 契约):前 20 条,每条约 100 字符截断、换行折叠;图片条目标注"图片"(纯函数)
fn fmt_clipboard_summary(items: &[crate::commands::clipboard::ClipboardItem]) -> String {
    if items.is_empty() {
        return "剪贴板历史为空".to_string();
    }
    items
        .iter()
        .take(20)
        .map(|it| {
            if it.tp == "image" {
                "图片".to_string()
            } else {
                let flat: String = it.payload.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
                flat.chars().take(100).collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ==================== 命令 ====================

/// 流式对话(SSE):spawn 异步任务,逐行 emit `ai-event`(chunk/tool/done/error)。
/// 返回 Err 仅代表"任务未能启动"(消息为空/参数配置类错误,已 emit error)。
/// 安全加固:model/base_url 不再作为入参(只信任配置),防 base_url 可覆盖而
/// 密钥仍从配置注入 → key 被引到攻击者服务器的组合漏洞
#[tauri::command]
pub fn ai_chat_stream(app: AppHandle, messages: Vec<ChatMessage>) -> Result<(), String> {
    if messages.is_empty() {
        return Err("消息不能为空".to_string());
    }
    // 任务启动前同步校验配置类错误(无 key / 服务商未配置等),快速失败
    let cfg = load_cfg(&app);
    let ep = match resolve_endpoint(&cfg) {
        Ok(ep) => ep,
        Err(kind) => return Err(classify_error(&kind, &cfg.ai_provider)),
    };
    let tools_enabled = cfg.ai_tools_enabled;
    let max_rounds = cfg.ai_max_tool_rounds.max(1);

    tauri::async_runtime::spawn(async move {
        let msgs: Vec<Value> = truncate_messages(messages)
            .into_iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();
        let client = RealClient {
            provider: &ep.provider,
            base_url: &ep.base_url,
            model: &ep.model,
            api_key: ep.api_key.as_deref(),
        };
        let app_ref = app.clone();
        let provider = ep.provider.clone();
        let exec = move |action: ToolAction| {
            Box::pin(exec_tool_action(app_ref.clone(), provider.clone(), action))
                as BoxFuture<'_, Result<(String, String), String>>
        };
        // H3 危险工具确认:emit ai-tool-confirm + 等待 ai_confirm_tool(≤60s 超时拒绝)
        let confirm_app = app.clone();
        let confirm = move |req: DangerRequest| {
            Box::pin(crate::ai::confirm::request_confirm(confirm_app.clone(), req))
                as BoxFuture<'_, bool>
        };
        let mut on_tool = |name: &str, args: &str| {
            let args_short: String = args.chars().take(80).collect();
            emit_ai(&app, json!({"kind": "tool", "tool": name, "args": args_short}));
        };
        let mut on_delta = |delta: String| {
            emit_ai(&app, json!({"kind": "chunk", "delta": delta}));
        };
        match run_tool_loop(&client, tools_enabled, max_rounds, true, msgs, &exec, &confirm, &mut on_tool, &mut on_delta).await {
            Ok(full) => emit_ai(&app, json!({"kind": "done", "full": full})),
            Err(kind) => {
                let msg = classify_error(&kind, &ep.provider);
                emit_ai(&app, json!({"kind": "error", "message": msg}));
            }
        }
    });
    Ok(())
}

/// 非流式对话(含工具循环):直接返回完整回复文本。
/// 安全加固:model/base_url 不再作为入参(只信任配置,同 ai_chat_stream)
#[tauri::command]
pub async fn ai_chat_completion(
    app: AppHandle,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    let cfg = load_cfg(&app);
    let ep = match resolve_endpoint(&cfg) {
        Ok(ep) => ep,
        Err(kind) => return Err(classify_error(&kind, &cfg.ai_provider)),
    };
    let msgs: Vec<Value> = truncate_messages(messages)
        .into_iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let client = RealClient {
        provider: &ep.provider,
        base_url: &ep.base_url,
        model: &ep.model,
        api_key: ep.api_key.as_deref(),
    };
    let app_ref = app.clone();
    let provider = ep.provider.clone();
    let exec = move |action: ToolAction| {
        Box::pin(exec_tool_action(app_ref.clone(), provider.clone(), action))
            as BoxFuture<'_, Result<(String, String), String>>
    };
    // H3 危险工具确认(同 ai_chat_stream;非流式命令同样必须过确认)
    let confirm_app = app.clone();
    let confirm = move |req: DangerRequest| {
        Box::pin(crate::ai::confirm::request_confirm(confirm_app.clone(), req))
            as BoxFuture<'_, bool>
    };
    let mut noop_tool = |_: &str, _: &str| {};
    let mut noop_delta = |_: String| {};
    match run_tool_loop(
        &client,
        cfg.ai_tools_enabled,
        cfg.ai_max_tool_rounds.max(1),
        false,
        msgs,
        &exec,
        &confirm,
        &mut noop_tool,
        &mut noop_delta,
    )
    .await
    {
        Ok(full) => Ok(full),
        Err(kind) => Err(classify_error(&kind, &ep.provider)),
    }
}

/// 单轮工具执行(3.2 契约入口,开发文档 §5):
/// 带 tools 对话 → 有 tool_call 执行**第一个**工具 → {ok, action, msg};
/// 无 tool_call → rule_match 规则兜底;兜底失败 → {ok:false, action:"none", msg: 模型原文}。
#[tauri::command]
pub async fn ai_execute_tool(app: AppHandle, instruction: String) -> AiToolResult {
    let cfg = load_cfg(&app);
    let ep = match resolve_endpoint(&cfg) {
        Ok(ep) => ep,
        Err(kind) => {
            return AiToolResult {
                ok: false,
                action: "none".to_string(),
                msg: classify_error(&kind, &cfg.ai_provider),
            }
        }
    };
    let instruction = instruction.trim().to_string();
    if instruction.is_empty() {
        return AiToolResult { ok: false, action: "none".to_string(), msg: "指令为空".to_string() };
    }
    let msgs = vec![json!({"role": "user", "content": instruction.clone()})];
    let tools = if cfg.ai_tools_enabled { Some(tools_json()) } else { None };
    let client = RealClient {
        provider: &ep.provider,
        base_url: &ep.base_url,
        model: &ep.model,
        api_key: ep.api_key.as_deref(),
    };
    let mut noop_delta = |_: String| {};
    match client.chat(msgs, tools, false, &mut noop_delta).await {
        Ok(reply) => {
            if let Some(calls) = parse_tool_calls(&reply) {
                // 执行第一个工具调用
                let call = &calls[0];
                match route(&call.name, &call.arguments) {
                    Ok(action) => {
                        // H3 安全加固:危险工具同样须经用户确认(与工具循环同一机制)
                        if is_dangerous_tool(&call.name)
                            && !crate::ai::confirm::request_confirm(
                                app.clone(),
                                DangerRequest {
                                    tool_call_id: call.id.clone(),
                                    tool: call.name.clone(),
                                    summary: danger_summary(&call.name, &action),
                                },
                            )
                            .await
                        {
                            return AiToolResult {
                                ok: false,
                                action: call.name.clone(),
                                msg: format!("用户已拒绝执行 {}", call.name),
                            };
                        }
                        match exec_tool_action(app.clone(), ep.provider.clone(), action).await {
                            Ok((action_name, msg)) => AiToolResult { ok: true, action: action_name, msg },
                            Err(e) => AiToolResult { ok: false, action: "none".to_string(), msg: e },
                        }
                    }
                    Err(e) => AiToolResult { ok: false, action: "none".to_string(), msg: e },
                }
            } else if let Some(action) = rule_match(&instruction) {
                // 规则兜底(模型不支持 tools 时保住打开/查找两类指令;
                // 兜底动作不含危险工具,无需确认)
                match exec_tool_action(app.clone(), ep.provider.clone(), action).await {
                    Ok((action_name, msg)) => AiToolResult { ok: true, action: action_name, msg },
                    Err(e) => AiToolResult { ok: false, action: "none".to_string(), msg: e },
                }
            } else {
                // 兜底失败:返回模型原文
                AiToolResult { ok: false, action: "none".to_string(), msg: reply }
            }
        }
        Err(kind) => AiToolResult {
            ok: false,
            action: "none".to_string(),
            msg: classify_error(&kind, &ep.provider),
        },
    }
}

/// 前端回传危险工具确认决策(H3 安全加固;注册名 ai_confirm_tool,待 lib.rs 接线)。
/// 返回语义:approve=true 且发信成功 → "已确认";approve=false 且发信成功 → "已拒绝";
/// 条目不存在(60s 超时已清理/从未注册)→ "不存在或已超时"。
#[tauri::command]
pub fn ai_confirm_tool(app: AppHandle, confirm_id: String, approve: bool) -> String {
    let state = crate::ai::confirm::confirm_state(&app);
    if state.resolve(&confirm_id, approve) {
        if approve {
            "已确认".to_string()
        } else {
            "已拒绝".to_string()
        }
    } else {
        "不存在或已超时".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::tools_json;

    // ---------- 消息截断(§1.7) ----------

    #[test]
    fn truncate_keeps_at_most_40() {
        let mk = |i: usize| ChatMessage { role: "user".into(), content: format!("m{i}") };
        let msgs: Vec<ChatMessage> = (0..40).map(mk).collect();
        assert_eq!(truncate_messages(msgs.clone()).len(), 40, "40 条不截");
        let msgs: Vec<ChatMessage> = (0..50).map(mk).collect();
        let kept = truncate_messages(msgs);
        assert_eq!(kept.len(), 40);
        assert_eq!(kept.first().unwrap().content, "m10", "保留最近 40 条(m10..m49)");
        assert_eq!(kept.last().unwrap().content, "m49");
    }

    // ---------- 双模式端点解析 ----------

    #[test]
    fn resolve_endpoint_ollama_no_key_needed() {
        let cfg = AppConfig { ai_provider: "ollama".into(), ai_ollama_model: "qwen2.5:7b".into(), ai_ollama_url: "http://127.0.0.1:11434/v1".into(), ..AppConfig::default() };
        let ep = resolve_endpoint(&cfg).unwrap();
        assert_eq!(ep.provider, "ollama");
        assert_eq!(ep.model, "qwen2.5:7b");
        assert_eq!(ep.base_url, "http://127.0.0.1:11434/v1");
        assert!(ep.api_key.is_none());
    }

    #[test]
    fn resolve_endpoint_deepseek_missing_key_is_err() {
        let cfg = AppConfig { ai_provider: "deepseek".into(), ai_api_key: None, ..AppConfig::default() };
        assert_eq!(resolve_endpoint(&cfg), Err(AiErrorKind::MissingKey));
        let cfg2 = AppConfig { ai_provider: "deepseek".into(), ai_api_key: Some("   ".into()), ..AppConfig::default() };
        assert_eq!(resolve_endpoint(&cfg2), Err(AiErrorKind::MissingKey), "空白 key 视为缺失");
    }

    #[test]
    fn resolve_endpoint_uses_config_only() {
        // 安全加固:model/base_url 一律取配置值,命令入参不再可覆盖(入参已移除)
        let cfg = AppConfig {
            ai_provider: "deepseek".into(),
            ai_api_key: Some("sk-real".into()),
            ai_model: "deepseek-chat".into(),
            ai_base_url: "https://api.deepseek.com/v1".into(),
            ..AppConfig::default()
        };
        let ep = resolve_endpoint(&cfg).unwrap();
        assert_eq!(ep.model, "deepseek-chat");
        assert_eq!(ep.base_url, "https://api.deepseek.com/v1");
        assert_eq!(ep.api_key.as_deref(), Some("sk-real"));
        // 空服务商 → Err
        let bad = AppConfig { ai_provider: "  ".into(), ..AppConfig::default() };
        assert!(resolve_endpoint(&bad).is_err());
    }

    // ---------- 结果格式化(纯函数) ----------

    #[test]
    fn fmt_memory_and_speed() {
        assert_eq!(fmt_memory(512), "512M");
        assert_eq!(fmt_memory(16384), "16.0G");
        assert_eq!(fmt_memory(5945), "5.8G");
        assert_eq!(fmt_speed(0), "0B/s");
        assert_eq!(fmt_speed(2048), "2K/s");
        assert_eq!(fmt_speed(1_258_291), "1.2M/s");
    }

    #[test]
    fn fmt_sys_status_contract_format() {
        let s = crate::commands::system::SysStatus {
            cpu: 12.4,
            mem_used_mb: 5945,
            mem_total_mb: 16384,
            net_tx_bps: 1_258_291,
            net_rx_bps: 3_565_158,
        };
        let text = fmt_sys_status(&s);
        assert!(text.starts_with("CPU 12% / 内存 5.8G/16.0G"), "实际: {text}");
        assert!(text.contains("上行 1.2M/s"), "实际: {text}");
        assert!(text.contains("下行 3.4M/s"), "实际: {text}");
    }

    #[test]
    fn fmt_clipboard_summary_truncates_and_marks_images() {
        let long: String = "字".repeat(200);
        let items = vec![
            crate::commands::clipboard::ClipboardItem { tp: "text".into(), payload: long, ts: 1 },
            crate::commands::clipboard::ClipboardItem { tp: "image".into(), payload: "C:\\pic.png".into(), ts: 2 },
            crate::commands::clipboard::ClipboardItem { tp: "text".into(), payload: "a\nb".into(), ts: 3 },
        ];
        let text = fmt_clipboard_summary(&items);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].chars().count(), 100, "长文本截断 100 字符");
        assert_eq!(lines[1], "图片");
        assert_eq!(lines[2], "a b", "换行折叠为空格");
        // 空历史
        assert_eq!(fmt_clipboard_summary(&[]), "剪贴板历史为空");
        // 20 条上限
        let many: Vec<_> = (0..30).map(|i| crate::commands::clipboard::ClipboardItem { tp: "text".into(), payload: format!("t{i}"), ts: i }).collect();
        assert_eq!(fmt_clipboard_summary(&many).lines().count(), 20);
    }

    #[test]
    fn summarize_apps_and_hits_caps_at_5() {
        let entries: Vec<AppEntry> = (0..8).map(|i| AppEntry { name: format!("app{i}"), path: format!("C:\\a\\app{i}.exe") }).collect();
        let text = summarize_apps(&entries);
        assert_eq!(text.lines().count(), 5);
        assert!(text.starts_with("app0 | C:\\a\\app0.exe"));
        assert_eq!(summarize_apps(&[]), "未找到匹配应用");
        assert_eq!(summarize_hits(&[]), "未找到文件");
    }

    #[test]
    fn display_path_prefers_file_name() {
        assert_eq!(display_path(r"C:\Windows\System32\notepad.exe"), "notepad.exe");
        assert_eq!(display_path(r"C:\"), r"C:\");
    }

    // ---------- 工具循环(§2.6:假 client 注入) ----------

    /// 假 client:按 replies 轮转回复,记录请求数、最近一次 tools 参数与最近一次消息数组
    struct FakeClient {
        replies: Vec<String>,
        requests: Mutex<usize>,
        last_tools: Mutex<Option<Value>>,
        last_messages: Mutex<Option<Vec<Value>>>,
    }

    impl ChatClient for FakeClient {
        fn chat<'a>(
            &'a self,
            messages: Vec<Value>,
            tools: Option<Value>,
            _stream: bool,
            _on_delta: &'a mut (dyn FnMut(String) + Send),
        ) -> BoxFuture<'a, Result<String, AiErrorKind>> {
            let mut n = self.requests.lock().unwrap();
            let i = *n;
            *n += 1;
            *self.last_tools.lock().unwrap() = tools;
            *self.last_messages.lock().unwrap() = Some(messages);
            let reply = self.replies[i.min(self.replies.len() - 1)].clone();
            Box::pin(async move { Ok(reply) })
        }
    }

    /// 始终带 tool_calls 的工具回复(OpenAI 格式);json! 构造保证 arguments 嵌套引号正确转义
    fn tool_reply(name: &str, args: &str) -> String {
        json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "c1",
                        "type": "function",
                        "function": { "name": name, "arguments": args }
                    }]
                }
            }]
        })
        .to_string()
    }

    /// 假 client 构造器:回复轮转,其余观测字段清零
    fn fake_client(replies: Vec<String>) -> FakeClient {
        FakeClient {
            replies,
            requests: Mutex::new(0),
            last_tools: Mutex::new(None),
            last_messages: Mutex::new(None),
        }
    }

    /// 假工具执行器:恒成功(绑定为具名变量避免临时值借用;返回类型显式标注供 coerce)
    fn fake_exec() -> impl Fn(ToolAction) -> BoxFuture<'static, Result<(String, String), String>> {
        |_action: ToolAction| -> BoxFuture<'static, Result<(String, String), String>> {
            Box::pin(async { Ok::<(String, String), String>(("fake".to_string(), "已执行".to_string())) })
        }
    }

    /// 假确认器:恒批准(H3 确认流测试外的既有用例默认放行)
    fn auto_approve() -> impl Fn(DangerRequest) -> BoxFuture<'static, bool> {
        |_req: DangerRequest| -> BoxFuture<'static, bool> { Box::pin(async { true }) }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn tool_loop_returns_text_without_tool_calls() {
        let client = fake_client(vec!["你好,我是助手".to_string()]);
        let msgs = vec![json!({"role": "user", "content": "hi"})];
        let mut tools_log: Vec<(String, String)> = Vec::new();
        let mut delta_log: Vec<String> = Vec::new();
        let exec = fake_exec();
        let confirm = auto_approve();
        let r = block_on(run_tool_loop(
            &client, true, 3, false, msgs, &exec, &confirm,
            &mut |n, a| tools_log.push((n.to_string(), a.to_string())),
            &mut |d| delta_log.push(d),
        ))
        .unwrap();
        assert_eq!(r, "你好,我是助手");
        assert_eq!(*client.requests.lock().unwrap(), 1, "无工具回复只请求 1 次");
        assert!(tools_log.is_empty(), "无工具不触发 on_tool");
        assert!(*client.last_tools.lock().unwrap() == Some(tools_json()), "带 tools 请求(默认开关)");
    }

    #[test]
    fn tool_loop_stops_after_max_rounds_with_hint() {
        // 模型每轮都回工具调用 → 3 轮上限后终止并带"已停止"提示(§2.6)
        let client = fake_client(vec![tool_reply("get_system_status", "{}")]);
        let msgs = vec![json!({"role": "user", "content": "查状态"})];
        let mut tools_log: Vec<(String, String)> = Vec::new();
        let exec = fake_exec();
        let confirm = auto_approve();
        let r = block_on(run_tool_loop(
            &client, true, 3, false, msgs, &exec, &confirm,
            &mut |n, a| tools_log.push((n.to_string(), a.to_string())),
            &mut |_d| {},
        ))
        .unwrap();
        assert!(r.contains("已停止:工具调用超过 3 轮上限"), "超限终止并提示,实际: {r}");
        assert_eq!(*client.requests.lock().unwrap(), 3, "3 轮上限 = 恰好 3 次请求,不死循环");
        assert_eq!(tools_log.len(), 3, "每轮 1 个工具调用 → 3 次 on_tool");
    }

    #[test]
    fn tool_loop_disabled_tools_skips_loop() {
        let client = fake_client(vec!["纯对话回复".to_string()]);
        let msgs = vec![json!({"role": "user", "content": "hi"})];
        let exec = fake_exec();
        let confirm = auto_approve();
        let r = block_on(run_tool_loop(
            &client, false, 3, false, msgs, &exec, &confirm,
            &mut |_, _| panic!("tools 关闭不应触发工具"),
            &mut |_| {},
        ))
        .unwrap();
        assert_eq!(r, "纯对话回复");
        assert_eq!(*client.requests.lock().unwrap(), 1);
        assert!(*client.last_tools.lock().unwrap() == None, "tools_enabled=false 不带 tools 参数");
    }

    #[test]
    fn tool_loop_exec_failure_isolation_feeds_back_false() {
        // 工具执行失败 → 失败隔离:回填 ok:false 继续下一轮,不中断对话
        let client = fake_client(vec![
            tool_reply("open_item", r#"{"path":"C:\\x"}"#),
            "工具执行失败但对话继续".to_string(),
        ]);
        let exec = |_action: ToolAction| -> BoxFuture<'static, Result<(String, String), String>> {
            Box::pin(async { Err::<(String, String), String>("打开失败".to_string()) })
        };
        let confirm = auto_approve();
        let msgs = vec![json!({"role": "user", "content": "打开 x"})];
        let r = block_on(run_tool_loop(
            &client, true, 3, false, msgs, &exec, &confirm,
            &mut |_, _| {},
            &mut |_| {},
        ))
        .unwrap();
        assert_eq!(r, "工具执行失败但对话继续", "失败隔离:模型可解释/换招");
        assert_eq!(*client.requests.lock().unwrap(), 2, "失败后仍进入下一轮");
    }

    // ---------- AiToolResult 序列化契约(§2.6) ----------

    #[test]
    fn ai_tool_result_serializes_contract_fields() {
        let r = AiToolResult { ok: true, action: "open_item".into(), msg: "已打开 记事本".into() };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"ok":true,"action":"open_item","msg":"已打开 记事本"}"#
        );
    }

    // ---------- H3 安全加固:危险工具确认流 ----------

    /// 记录调用次数的执行器(H3 确认流断言"拒绝则绝不执行")
    fn counting_exec(
        calls: &std::sync::Mutex<Vec<ToolAction>>,
    ) -> impl Fn(ToolAction) -> BoxFuture<'static, Result<(String, String), String>> + '_ {
        |action: ToolAction| -> BoxFuture<'static, Result<(String, String), String>> {
            calls.lock().unwrap().push(action);
            Box::pin(async { Ok::<(String, String), String>(("fake".to_string(), "已执行".to_string())) })
        }
    }

    /// 恒拒绝的确认器(等价前端取消 / 60s 超时)
    fn deny_all() -> impl Fn(DangerRequest) -> BoxFuture<'static, bool> {
        |_req: DangerRequest| -> BoxFuture<'static, bool> { Box::pin(async { false }) }
    }

    #[test]
    fn tool_loop_dangerous_tool_rejected_skips_execution() {
        // open_item 属危险工具:确认返回 false → 不执行,回填"用户已拒绝执行 open_item"
        // 记入下一轮上下文(模型可解释/换招),对话继续
        let client = fake_client(vec![
            tool_reply("open_item", r#"{"path":"C:\\Windows\\System32\\cmd.exe"}"#),
            "已拒绝,对话继续".to_string(),
        ]);
        let calls = std::sync::Mutex::new(Vec::new());
        let exec = counting_exec(&calls);
        let confirm = deny_all();
        let msgs = vec![json!({"role": "user", "content": "打开 cmd"})];
        let r = block_on(run_tool_loop(
            &client, true, 3, false, msgs, &exec, &confirm,
            &mut |_, _| {},
            &mut |_| {},
        ))
        .unwrap();
        assert_eq!(r, "已拒绝,对话继续", "拒绝后仍进入下一轮对话");
        assert!(calls.lock().unwrap().is_empty(), "被拒的危险工具绝不能执行");
        // 拒绝提示已作为 tool 结果记入下一轮请求上下文
        let last = client.last_messages.lock().unwrap().clone().unwrap();
        let tool_msg = last.iter().find(|m| m["role"] == "tool").expect("应有 tool 回填");
        let content = tool_msg["content"].as_str().unwrap();
        assert!(content.contains("用户已拒绝执行 open_item"), "实际: {content}");
        assert!(content.contains("ok"), "回填结果应带 ok 字段");
    }

    #[test]
    fn tool_loop_dangerous_tool_approved_executes() {
        // 确认返回 true → 正常执行;确认器收到的请求应带正确工具名与摘要
        let client = fake_client(vec![
            tool_reply("open_item", r#"{"path":"C:\\Windows\\System32\\notepad.exe"}"#),
            "已完成".to_string(),
        ]);
        let calls = std::sync::Mutex::new(Vec::new());
        let exec = counting_exec(&calls);
        let seen = std::sync::Mutex::new(Vec::new());
        // 注意不加 move:按引用捕获 seen(闭包仍为 Fn),循环结束后还能断言
        let confirm = |req: DangerRequest| -> BoxFuture<'static, bool> {
            seen.lock().unwrap().push((req.tool.clone(), req.summary.clone()));
            Box::pin(async { true })
        };
        let msgs = vec![json!({"role": "user", "content": "打开记事本"})];
        let r = block_on(run_tool_loop(
            &client, true, 3, false, msgs, &exec, &confirm,
            &mut |_, _| {},
            &mut |_| {},
        ))
        .unwrap();
        assert_eq!(r, "已完成");
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "批准后执行一次"
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "危险工具确认器只被询问一次");
        assert_eq!(seen[0].0, "open_item");
        assert_eq!(seen[0].1, r"C:\Windows\System32\notepad.exe", "摘要应为目标路径");
    }

    #[test]
    fn tool_loop_non_dangerous_tool_skips_confirm() {
        // 非危险工具(get_system_status)不应触发确认器(触发即 panic)
        let client = fake_client(vec![tool_reply("get_system_status", "{}"), "状态已返回".to_string()]);
        let exec = fake_exec();
        let confirm = |_req: DangerRequest| -> BoxFuture<'static, bool> {
            Box::pin(async { panic!("非危险工具不应触发确认") })
        };
        let msgs = vec![json!({"role": "user", "content": "查状态"})];
        let r = block_on(run_tool_loop(
            &client, true, 3, false, msgs, &exec, &confirm,
            &mut |_, _| {},
            &mut |_| {},
        ))
        .unwrap();
        assert_eq!(r, "状态已返回");
    }

    #[test]
    fn tool_loop_dangerous_tool_rejected_within_multiple_calls() {
        // 一轮内多个工具调用:危险工具被拒只跳过自身,其余照常执行
        let client = fake_client(vec![
            // open_item(拒) + get_system_status(放行)
            json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [
                            {"id": "c1", "type": "function", "function": {"name": "open_item", "arguments": "{\"path\":\"C:\\\\a.exe\"}"}},
                            {"id": "c2", "type": "function", "function": {"name": "get_system_status", "arguments": "{}"}}
                        ]
                    }
                }]
            })
            .to_string(),
            "第二轮".to_string(),
        ]);
        let calls = std::sync::Mutex::new(Vec::new());
        let exec = counting_exec(&calls);
        // 只拒 open_item
        let confirm = |req: DangerRequest| -> BoxFuture<'static, bool> {
            Box::pin(async move { req.tool != "open_item" })
        };
        let msgs = vec![json!({"role": "user", "content": "hi"})];
        let r = block_on(run_tool_loop(
            &client, true, 3, false, msgs, &exec, &confirm,
            &mut |_, _| {},
            &mut |_| {},
        ))
        .unwrap();
        assert_eq!(r, "第二轮");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "只执行了未被拒的工具");
        assert!(matches!(&calls[0], ToolAction::GetSystemStatus));
    }

    // ---------- H3 安全加固:剪贴板云端禁用(两种模式) ----------

    #[test]
    fn is_local_provider_recognizes_ollama_only() {
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("  ollama  "), "容忍首尾空白");
        assert!(is_local_provider("OLLAMA"), "忽略大小写");
        assert!(!is_local_provider("deepseek"));
        assert!(!is_local_provider("openai"));
        assert!(!is_local_provider(""));
    }

    #[test]
    fn clipboard_blocked_in_cloud_allowed_local() {
        // 云端模式(DeepSeek 等)→ 拦截并回隐私提示,剪贴板内容不进下一轮 prompt
        assert_eq!(clipboard_block_hint("deepseek"), Some("云端模式为保护隐私不读取剪贴板历史"));
        assert_eq!(clipboard_block_hint("openai"), Some("云端模式为保护隐私不读取剪贴板历史"));
        assert_eq!(clipboard_block_hint(""), Some("云端模式为保护隐私不读取剪贴板历史"), "未知/空服务商按云端处理(默认拒绝)");
        // 本地模式(Ollama)→ 放行
        assert_eq!(clipboard_block_hint("ollama"), None);
        assert_eq!(clipboard_block_hint("  Ollama "), None);
    }
}
