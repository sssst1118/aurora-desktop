//! 3.1 AI 后端代理(唯一网络出口)。
//!
//! 双模式(设计文档 §1.2):统一 OpenAI 兼容 Chat Completions 格式,仅 base_url / 认证头 /
//! model / 连接超时差异:
//!   - deepseek:`https://api.deepseek.com/v1` + `Authorization: Bearer {key}`(无 key 由命令层
//!     发请求前快速报"未配置 API Key");connect_timeout 15s;
//!   - ollama:`http://127.0.0.1:11434/v1`,无认证;connect_timeout 3s(本机拒绝连接 = 未启动,
//!     秒级提示)。
//! 总请求超时 120s;temperature 0.7;max_tokens 不设(模型默认)。
//!
//! SSE 流式(§1.2 方案 1~7):`bytes_stream` 累积按 `\n` 切行,剥 `data: ` 前缀,
//! `data: [DONE]` 结束;坏 JSON 行跳过不中断;idle 超时 30s(两个 chunk 之间无数据视为断线);
//! 非 SSE 响应(Content-Type 不含 text/event-stream)降级为一次性返回完整文本(方案 7)。
//!
//! 纯函数(单测友好,§1.7):
//!   - `build_request` 请求体/头构造(双模式差异);
//!   - `parse_sse_line` + `is_done_line` SSE 行解析;
//!   - `classify_error` / `map_send_error` / `extract_error_message` 错误分类映射(§1.2 错误表);
//!   - `extract_completion_text` 非流式降级提取。
//!
//! 铁律(§0.4/§1.3):日志与错误信息不含密钥(仅 base_url+状态码);密钥只在 `build_request`
//! 的 header 注入这一处;命令层入参不含 key,密钥只从配置读。
//!
//! 内存铁律:reqwest Client 全局 OnceLock 惰性创建,不聊天零网络资源。
//! 双模式连接超时不同(connect_timeout 是 Client 构建参数),各持一个 OnceLock 连接池。

use reqwest::Client;
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

/// 总请求超时(DeepSeek 长思考 / 本地推理慢都覆盖,§1.2 双模式差异表)
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// SSE idle 超时:两个 chunk 之间 30s 无数据视为断线(§1.2 方案 4)
pub const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// DeepSeek 连接超时(断网/不可达快速失败,不长时间挂起)
pub const DEEPSEEK_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Ollama 连接超时(本机拒绝连接 = 未启动,秒级提示)
pub const OLLAMA_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// SSE 累积字节上限(安全加固 2026-08-14):ai_base_url 可配置任意地址,恶意端点可在
/// 120s 总超时内持续灌数据,行缓冲 buf 与累积文本(回复内容 + 工具参数分片)无上限
/// 会内存膨胀;超过即 StreamInterrupted 中止(与 MAX_TOOL_CALLS 同类加固风格)。
/// 正常对话回复远低于此量级(4MB 文本 ≈ 百万汉字)。
const MAX_SSE_BUF_BYTES: usize = 1024 * 1024;      // 跨 chunk 半行累积缓冲上限 1MB
const MAX_SSE_FULL_BYTES: usize = 4 * 1024 * 1024; // 累积回复文本/工具参数上限 4MB

/// 流式 tool_calls 分片条目数上限(安全加固 2026-08-18,中 1):tool_chunks 条目数
/// 无上限时,恶意端点可在 120s 总超时内每行(≤1MB)塞大量分片,条目自身开销持续
/// 累积;正常工具调用分片远低于此(单次工具循环至多几十片),128 绰绰有余。
const MAX_TOOL_CHUNKS: usize = 128;

/// reqwest Client 惰性创建(首次请求才建连接池,不聊天零网络线程)。
/// 双模式连接超时不同,无法共用单个 Client,各持一个 OnceLock。
static CLIENT_DS: OnceLock<Client> = OnceLock::new();
static CLIENT_OLLAMA: OnceLock<Client> = OnceLock::new();

/// local_only=true 时禁用全部代理(reqwest 0.12 默认启用系统代理,Windows 上会读
/// 注册表代理设置;实测本机系统代理 127.0.0.1:7897 对 127.0.0.1 目标返回 502,
/// Ollama 请求被转发给代理后必挂 → 本机地址显式 no_proxy();
/// DeepSeek(公网)保留系统代理行为,走代理或直连由系统设置决定)
fn build_client(connect_timeout: Duration, local_only: bool) -> Client {
    let mut builder = Client::builder().connect_timeout(connect_timeout);
    if local_only {
        builder = builder.no_proxy();
    }
    // 加固(2026-08-13):build 失败(极罕见,通常为系统 TLS 后端缺失/环境异常)不再
    // expect panic 崩进程,降级为默认 Client——请求仍可用(总超时在请求级 timeout,
    // 不受影响),仅失去连接超时差异化(deepseek 15s / ollama 3s 快速失败)
    builder.build().unwrap_or_else(|e| {
        eprintln!("[aurora] reqwest Client 构建失败,降级为默认 Client(连接超时不再按模式差异化): {e}");
        Client::new()
    })
}

/// 按服务商取全局惰性 Client(ollama=3s 快速失败+禁代理,其余=15s 保留系统代理)
fn client_for(provider: &str) -> &'static Client {
    if provider == "ollama" {
        CLIENT_OLLAMA.get_or_init(|| build_client(OLLAMA_CONNECT_TIMEOUT, true))
    } else {
        CLIENT_DS.get_or_init(|| build_client(DEEPSEEK_CONNECT_TIMEOUT, false))
    }
}

// ==================== 错误分类(设计文档 §1.2 错误表) ====================

/// AI 请求错误分类(纯函数,可单测)。
#[derive(Debug, Clone, PartialEq)]
pub enum AiErrorKind {
    /// 密钥缺失(deepseek 模式且 key 为空;命令层请求前校验,不发请求)
    MissingKey,
    /// 连接被拒绝(本机 Ollama 未启动的典型表现)
    ConnectRefused,
    /// 连接超时 / DNS 失败 / 网络不可达
    ConnectTimeout,
    /// 总请求超时(120s)
    RequestTimeout,
    /// HTTP 4xx/5xx
    Http { code: u16, message: String },
    /// 模型不存在(HTTP 404,如 Ollama 未装对应模型)
    ModelNotFound,
    /// 流中断(idle 超时 / EOF 异常)
    StreamInterrupted,
    /// 其他(参数配置类错误,消息已可直接展示)
    Other(String),
}

/// 错误 → 中文文案(§1.2 错误表)。
/// ConnectRefused 按服务商区分:ollama=未运行提示;云端(deepseek)=网络失败提示。
pub fn classify_error(kind: &AiErrorKind, provider: &str) -> String {
    match kind {
        AiErrorKind::MissingKey => "未配置 API Key,请在设置中填写".to_string(),
        AiErrorKind::ConnectRefused => {
            if provider == "ollama" {
                "Ollama 未运行,请先启动 Ollama 再试".to_string()
            } else {
                "网络连接失败,请检查网络后重试".to_string()
            }
        }
        AiErrorKind::ConnectTimeout => "网络连接失败,请检查网络后重试".to_string(),
        AiErrorKind::RequestTimeout => "请求超时,请重试或换本地模型".to_string(),
        AiErrorKind::Http { code, message } => {
            let detail = if message.is_empty() {
                "无详细信息".to_string()
            } else {
                message.clone()
            };
            format!("模型服务错误(HTTP {code}):{detail}")
        }
        AiErrorKind::ModelNotFound => "模型不存在或未安装,请在设置中修改模型名".to_string(),
        AiErrorKind::StreamInterrupted => "对话流中断,请重试".to_string(),
        AiErrorKind::Other(m) => m.clone(),
    }
}

/// reqwest 发送错误 → 分类(纯函数)。
/// connect+timeout = 连接超时(断网);仅 timeout = 总请求超时;
/// 其他 connect 失败(含连接拒绝,Windows 常见于本地 Ollama 未启动)= ConnectRefused。
pub fn map_send_error(e: &reqwest::Error) -> AiErrorKind {
    if e.is_connect() && e.is_timeout() {
        AiErrorKind::ConnectTimeout
    } else if e.is_timeout() {
        AiErrorKind::RequestTimeout
    } else if e.is_connect() {
        AiErrorKind::ConnectRefused
    } else if e.is_builder() {
        AiErrorKind::Other(format!("请求构造失败:{e}"))
    } else {
        AiErrorKind::Other(format!("请求失败:{e}"))
    }
}

/// 从错误响应体提取人话 message(OpenAI 兼容 `{"error":{"message":...}}` / `{"message":...}`);
/// 提取失败返回空串(HTTP 错误文案里给"无详细信息")。截断 120 字符防刷屏。
pub fn extract_error_message(body: &str) -> String {
    let root: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    let msg = root
        .get("error")
        .and_then(|e| e.get("message"))
        .or_else(|| root.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    msg.chars().take(120).collect()
}

// ==================== 请求构造(纯函数) ====================

/// 构造好的 HTTP 请求(纯函数产物,可单测)。
pub struct ChatRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    pub total_timeout: Duration,
}

/// 请求体/头构造(纯函数):统一 OpenAI 兼容 Chat Completions 格式,仅 endpoint/key/model 差异。
/// - api_key 为 Some 且非空 → 注入 `Authorization: Bearer {key}`(密钥注入唯一位置);
/// - base_url / model 空串 → Err(配置类错误);
/// - tools 为 Some → body 附 tools 数组;stream → body.stream=true;
/// - temperature 0.7 固定;max_tokens 不设(模型默认)。
/// - `_provider` 当前构造不依赖(连接超时差异已在 client_for 层),保留作未来按服务商默认参数的扩展位。
pub fn build_request(
    _provider: &str,
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
    messages: Vec<Value>,
    tools: Option<Value>,
    stream: bool,
) -> Result<ChatRequest, AiErrorKind> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(AiErrorKind::Other("模型接口地址未配置,请在设置中填写".to_string()));
    }
    let model = model.trim();
    if model.is_empty() {
        return Err(AiErrorKind::Other("模型名未配置,请在设置中填写".to_string()));
    }
    let url = format!("{base}/chat/completions");
    let mut headers = Vec::new();
    if let Some(k) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        headers.push(("Authorization".to_string(), format!("Bearer {k}")));
    }
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
        "temperature": 0.7,
    });
    if let Some(t) = tools {
        body["tools"] = t;
    }
    // 注:连接超时(ollama 3s / deepseek 15s)已在 build_client 层(Client 构建参数)生效,
    // ChatRequest 不携带(避免死代码)
    Ok(ChatRequest { url, headers, body, total_timeout: TOTAL_TIMEOUT })
}

// ==================== SSE 行解析(纯函数) ====================

/// 判断一行(原始 SSE 行,含 `data: ` 前缀)是否为结束信号 `data: [DONE]`。
pub fn is_done_line(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("data:")
        .map(str::trim)
        .is_some_and(|p| p == "[DONE]")
}

/// 解析一行 SSE 数据(纯函数,输入为原始行,含 `data: ` 前缀):
/// - 非 `data: ` 前缀(空行/注释行)→ None(跳过不中断);
/// - `data: [DONE]` → None(结束信号由调用方先判 `is_done_line`);
/// - `data: {...}` → 取 `choices[0].delta.content` 非空字符串 → Some(增量文本);
///   坏 JSON / content 缺失或非字符串(如首块只带 role、finish 块 delta 为空对象)→ None。
/// 调用方负责把行按 `\n` 切好并 trim `\r`(CRLF 容忍)。
pub fn parse_sse_line(line: &str) -> Option<String> {
    let line = line.trim_end_matches('\r');
    let payload = line.trim_start().strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let root: Value = serde_json::from_str(payload).ok()?;
    let delta = root
        .get("choices")?
        .as_array()?
        .first()?
        .get("delta")?;
    match delta.get("content") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// 解析一行 SSE 中的流式 tool_calls 分片(OpenAI 流式工具调用格式):
/// `delta.tool_calls[]` → 每片 (index, id 或空, name 或空, arguments 增量或空)。
/// id/name 只在首片出现,arguments 跨片分段追加;无 tool_calls 行 → None(纯函数,容错同 parse_sse_line)。
pub fn parse_sse_tool_chunk(line: &str) -> Option<Vec<(usize, String, String, String)>> {
    let line = line.trim_end_matches('\r');
    let payload = line.trim_start().strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let root: Value = serde_json::from_str(payload).ok()?;
    let chunks = root
        .get("choices")?
        .as_array()?
        .first()?
        .get("delta")?
        .get("tool_calls")?
        .as_array()?;
    if chunks.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(chunks.len());
    for c in chunks {
        let idx = c.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let id = c.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let name = c
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = c
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        out.push((idx, id, name, args));
    }
    Some(out)
}

/// 非流式响应提取 `choices[0].message.content`(纯函数,降级路径用);失败 → None(调用方退回原文)。
/// 契约:content 为 null **或空字符串**(OpenAI 纯工具回复)均视为无文本 → None,
/// 调用方 `unwrap_or(body)` 回退完整 body JSON,使 parse_tool_calls 能解析 tool_calls(§2.3)。
pub fn extract_completion_text(body: &str) -> Option<String> {
    let root: Value = serde_json::from_str(body).ok()?;
    let content = root
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?;
    match content {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

// ==================== 响应体带上限读取(2026-08-18 中 2 加固) ====================

/// 响应体 Content-Length 声明是否超上限(纯函数,可单测):>4MB 直接拒绝,
/// 不必等流式累积(防恶意端点声明超大 body 后持续推送);无声明 → 放行,
/// 由 read_body_limited 的流式累积兜底。等于上限放行(与 sse_size_exceeded
/// "严格超限才断"同语义)。
fn content_length_too_large(len: Option<u64>) -> bool {
    len.is_some_and(|l| l > MAX_SSE_FULL_BYTES as u64)
}

/// 带上限的响应体读取(async;非 SSE 降级与错误响应体共用):
/// ① Content-Length 声明超 4MB → 直接拒绝;
/// ② bytes_stream 流式累积,超过 4MB → 拒绝(与 SSE 路径同口径上限)。
/// 修复背景(2026-08-18 中 2):原 resp.text() 无字节上限,恶意端点返回非
/// event-stream 巨型 body 时全量载入内存。
async fn read_body_limited(resp: reqwest::Response) -> Result<String, AiErrorKind> {
    if content_length_too_large(resp.content_length()) {
        return Err(AiErrorKind::StreamInterrupted);
    }
    use futures_util::StreamExt;
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|_| AiErrorKind::StreamInterrupted)?;
        buf.extend_from_slice(&bytes);
        if buf.len() > MAX_SSE_FULL_BYTES {
            return Err(AiErrorKind::StreamInterrupted);
        }
    }
    // 非 SSE 响应体按 UTF-8 文本处理(lossy 容错:恶意二进制 body 不引入新失败路径)
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 一行流式 tool_calls 分片的字符串字段总字节(纯函数,可单测):id + name +
/// arguments 三字段(元组第 2/3/4 元素),与 4MB 上限同口径累计——此前只累计
/// arguments 字节,恶意端点可用超长 id/name 绕过上限持续膨胀内存(2026-08-18 中 1)。
fn tool_chunks_bytes(chunks: &[(usize, String, String, String)]) -> usize {
    chunks.iter().map(|c| c.1.len() + c.2.len() + c.3.len()).sum()
}

// ==================== 对话请求(唯一网络出口) ====================

/// 校验 SSE 流累积量是否超限(纯函数,可单测):行缓冲 > 1MB 或累积文本(回复内容 +
/// 工具参数分片)> 4MB → Err(StreamInterrupted,中止);等于上限放行(严格超限才断)。
fn sse_size_exceeded(buf_len: usize, accumulated: usize) -> Result<(), AiErrorKind> {
    if buf_len > MAX_SSE_BUF_BYTES || accumulated > MAX_SSE_FULL_BYTES {
        return Err(AiErrorKind::StreamInterrupted);
    }
    Ok(())
}

/// 一次对话请求(单轮;工具循环在命令层 run_tool_loop)。
/// 返回契约(§2.3):回复含 tool_calls → 返回完整 message JSON 字符串(parse_tool_calls 可解析);
/// 纯文本回复 → 返回 content 文本。stream=false 一次性返回;stream=true SSE 逐行解析,
/// 增量经 on_delta 回调(工具分片不进 on_delta,工具轮次不发 chunk,§2.3 动作条只走 tool 事件)。
/// on_delta 要求 Send(流任务在 spawn 线程跨 await 持有);错误路径(§1.2 方案 6):
/// 任何阶段出错返回 Err(AiErrorKind),不 panic 不崩溃。
pub async fn chat<F>(
    provider: &str,
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
    messages: Vec<Value>,
    tools: Option<Value>,
    stream: bool,
    on_delta: &mut F,
) -> Result<String, AiErrorKind>
where
    F: FnMut(String) + Send + ?Sized, // 允许直接传 &mut dyn FnMut(RealClient 转发场景)
{
    let req = build_request(provider, base_url, model, api_key, messages, tools, stream)?;
    let client = client_for(provider);

    // 发送(连接超时按模式差异化;总超时 120s)
    let mut rb = client.post(&req.url).timeout(req.total_timeout);
    for (k, v) in &req.headers {
        rb = rb.header(k, v);
    }
    let resp = rb
        .json(&req.body)
        .send()
        .await
        .map_err(|e| map_send_error(&e))?;

    // 状态码分类(404 = 模型未装;其余 4xx/5xx 带响应体 message)
    let status = resp.status();
    if status.as_u16() == 404 {
        return Err(AiErrorKind::ModelNotFound);
    }
    if !status.is_success() {
        // 中 2 加固:错误响应体同样走带上限读取(恶意端点可用巨型错误 body 撑内存)
        let body = read_body_limited(resp).await.unwrap_or_default();
        let message = extract_error_message(&body);
        return Err(AiErrorKind::Http { code: status.as_u16(), message });
    }

    // 非 SSE 降级(方案 7):一次性读全文,能提取 content 就用,否则退回原文
    let is_sse = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));
    if !is_sse {
        // 中 2 加固:非 SSE 降级路径带 4MB 上限读取(此前 resp.text() 无上限,
        // 恶意端点返回非 event-stream 巨型 body 时全量载入内存)
        let body = read_body_limited(resp).await?;
        return Ok(extract_completion_text(&body).unwrap_or(body));
    }

    // SSE 循环(方案 1~5):bytes_stream 累积按 \n 切行;idle 超时 30s
    // 安全加固(2026-08-14):行缓冲 buf / 累积文本 full+工具参数无上限时,恶意端点
    // 可在 120s 总超时内持续灌数据撑爆内存;每收到一个 chunk、每累积一段文本都
    // 校验上限,超限即 StreamInterrupted 中止(见 sse_size_exceeded)
    use futures_util::StreamExt;
    let mut byte_stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut full = String::new();
    let mut tool_chunks: Vec<(usize, String, String, String)> = Vec::new();
    // 流式工具调用分片 id/name/arguments 三字段累计字节(与 full 同享 4MB 上限;
    // 2026-08-18 中 1:由仅算 arguments 扩为四字段同口径,防超长 id/name 绕过)
    let mut tool_bytes: usize = 0;
    loop {
        let next = tokio::time::timeout(SSE_IDLE_TIMEOUT, byte_stream.next())
            .await
            .map_err(|_| AiErrorKind::StreamInterrupted)?;
        match next {
            Some(Err(_)) => return Err(AiErrorKind::StreamInterrupted),
            Some(Ok(bytes)) => {
                buf.extend_from_slice(&bytes);
                sse_size_exceeded(buf.len(), full.len() + tool_bytes)?;
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: String = String::from_utf8_lossy(&buf[..pos]).into_owned();
                    buf.drain(..=pos);
                    if is_done_line(&line) {
                        return Ok(finish_stream(full, &tool_chunks));
                    }
                    if let Some(delta) = parse_sse_line(&line) {
                        full.push_str(&delta);
                        sse_size_exceeded(buf.len(), full.len() + tool_bytes)?;
                        on_delta(delta);
                    } else if let Some(chunks) = parse_sse_tool_chunk(&line) {
                        // 中 1 加固(2026-08-18):此前只累计 arguments 字节,
                        // 恶意端点可用超长 id/name 绕过 4MB 上限持续膨胀内存;
                        // 现累计 id+name+arguments 三字段总字节,并对分片条目数
                        // 一并设上限(任一超限即中止,与 4MB 同口径)
                        if tool_chunks.len() + chunks.len() > MAX_TOOL_CHUNKS {
                            return Err(AiErrorKind::StreamInterrupted);
                        }
                        tool_bytes += tool_chunks_bytes(&chunks);
                        tool_chunks.extend(chunks); // 流式工具调用分片(OpenAI 格式)
                        sse_size_exceeded(buf.len(), full.len() + tool_bytes)?;
                    }
                }
            }
            None => break, // 流正常结束(部分服务端不发送 [DONE] 也支持)
        }
    }
    Ok(finish_stream(full, &tool_chunks))
}

/// 流式工具调用分片 index 上限(安全加固):index 来自远端 JSON,无上限时
/// 恶意 index(如 40 亿)会让 `merged.resize(idx+1)` 直接 OOM abort 进程。
/// 正常工具循环一次最多几个工具,32 绰绰有余;超限分片直接丢弃。
const MAX_TOOL_CALLS: usize = 32;

/// 流结束收尾(纯函数):收集到 tool_calls 分片 → 返回构造的完整 message JSON
/// (与 chat 返回契约一致,供 parse_tool_calls 解析,§2.3);
/// 无分片 → 返回 content 全文。
fn finish_stream(full: String, tool_chunks: &[(usize, String, String, String)]) -> String {
    if tool_chunks.is_empty() {
        return full;
    }
    // 按 index 归并:id/name 取非空(仅首片携带),arguments 跨片拼接;
    // index 超上限的分片直接丢弃(防远端恶意大 index 触发巨量 resize)
    let mut merged: Vec<(String, String, String)> = Vec::new();
    for (idx, id, name, args) in tool_chunks {
        if *idx >= MAX_TOOL_CALLS {
            continue;
        }
        if merged.len() <= *idx {
            merged.resize(*idx + 1, (String::new(), String::new(), String::new()));
        }
        let e = &mut merged[*idx];
        if !id.is_empty() {
            e.0.clone_from(id);
        }
        if !name.is_empty() {
            e.1.clone_from(name);
        }
        e.2.push_str(args);
    }
    // 空槽过滤:恶意 index 跳号产生的中间空槽(id/name/arguments 全空)不产出 tool_call
    let calls: Vec<Value> = merged
        .into_iter()
        .filter(|(id, name, args)| !(id.is_empty() && name.is_empty() && args.is_empty()))
        .map(|(id, name, args)| {
            json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": args }
            })
        })
        .collect();
    json!({
        "choices": [{
            "message": { "role": "assistant", "content": full, "tool_calls": calls }
        }]
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- build_request:双模式差异 / 尾斜杠 / 参数校验 ----------

    #[test]
    fn build_request_deepseek_adds_bearer_and_temperature() {
        let req = build_request(
            "deepseek",
            "https://api.deepseek.com/v1",
            "deepseek-chat",
            Some("sk-test-123"),
            vec![json!({"role": "user", "content": "你好"})],
            None,
            false,
        )
        .expect("合法参数应构造成功");
        assert_eq!(req.url, "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(req.headers, vec![("Authorization".to_string(), "Bearer sk-test-123".to_string())]);
        assert_eq!(req.body["model"], "deepseek-chat");
        assert_eq!(req.body["stream"], false);
        assert_eq!(req.body["temperature"], 0.7);
        assert_eq!(req.body["messages"][0]["content"], "你好");
        assert!(req.body.get("tools").is_none(), "未传 tools 时 body 不应出现");
        assert_eq!(req.total_timeout, TOTAL_TIMEOUT);
    }

    #[test]
    fn build_request_ollama_no_auth_and_short_connect_timeout() {
        let req = build_request(
            "ollama",
            "http://127.0.0.1:11434/v1/",
            "qwen2.5:7b",
            None,
            vec![],
            None,
            true,
        )
        .expect("合法参数应构造成功");
        assert_eq!(req.url, "http://127.0.0.1:11434/v1/chat/completions");
        assert!(req.headers.is_empty(), "ollama 无认证头");
        assert_eq!(req.body["stream"], true);
        // 尾斜杠多串也容忍
        let req2 = build_request("ollama", "http://127.0.0.1:11434/v1///", "m", None, vec![], None, false).unwrap();
        assert_eq!(req2.url, "http://127.0.0.1:11434/v1/chat/completions");
    }

    #[test]
    fn build_request_attaches_tools_when_some() {
        let req = build_request("deepseek", "https://x/v1", "m", Some("k"), vec![], Some(json!([{"type": "function"}])), false).unwrap();
        assert_eq!(req.body["tools"][0]["type"], "function");
    }

    #[test]
    fn build_request_empty_key_skips_header() {
        let req = build_request("deepseek", "https://x/v1", "m", Some("   "), vec![], None, false).unwrap();
        assert!(req.headers.is_empty(), "空白 key 不注入认证头");
    }

    #[test]
    fn build_request_empty_base_or_model_is_err() {
        assert!(build_request("deepseek", "", "m", None, vec![], None, false).is_err());
        assert!(build_request("deepseek", "   ", "m", None, vec![], None, false).is_err());
        assert!(build_request("deepseek", "https://x/v1", "", None, vec![], None, false).is_err());
        assert!(build_request("deepseek", "https://x/v1", "  ", None, vec![], None, false).is_err());
    }

    // ---------- SSE 行解析 ----------

    #[test]
    fn parse_sse_line_extracts_delta() {
        let line = r#"data: {"choices":[{"index":0,"delta":{"content":"很"},"finish_reason":null}]}"#;
        assert_eq!(parse_sse_line(line), Some("很".to_string()));
        let line = r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"抱歉"},"finish_reason":null}]}"#;
        assert_eq!(parse_sse_line(line), Some("抱歉".to_string()));
    }

    #[test]
    fn parse_sse_line_skips_noise_without_panic() {
        // 空行 / 注释行 / 非 data 前缀 → None
        assert_eq!(parse_sse_line(""), None);
        assert_eq!(parse_sse_line("\n"), None);
        assert_eq!(parse_sse_line(": keep-alive comment"), None);
        assert_eq!(parse_sse_line("event: done"), None);
        // 坏 JSON → None(跳过不中断)
        assert_eq!(parse_sse_line("data: {bad json"), None);
        assert_eq!(parse_sse_line("data: "), None);
        assert_eq!(parse_sse_line("data"), None);
        // 结构缺失 / content 非字符串 → None
        assert_eq!(parse_sse_line(r#"data: {}"#), None);
        assert_eq!(parse_sse_line(r#"data: {"choices":[]}"#), None);
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#), None);
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{}}]}"#), None);
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{"content":null}}]}"#), None);
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{"content":""}}]}"#), None);
    }

    #[test]
    fn parse_sse_line_tolerates_crlf() {
        let line = r#"data: {"choices":[{"delta":{"content":"好"}}]}"#.to_string() + "\r";
        assert_eq!(parse_sse_line(&line), Some("好".to_string()));
    }

    #[test]
    fn is_done_line_variants() {
        assert!(is_done_line("data: [DONE]"));
        assert!(is_done_line("data:[DONE]"));
        assert!(is_done_line("  data: [DONE]  "));
        assert!(!is_done_line("data: [DONE]x"));
        assert!(!is_done_line("[DONE]"));
        assert!(!is_done_line("data: {}"));
        assert!(!is_done_line(""));
    }

    // ---------- 错误分类(§1.2 错误表) ----------

    #[test]
    fn classify_error_table_ollama_mode() {
        assert_eq!(classify_error(&AiErrorKind::MissingKey, "deepseek"), "未配置 API Key,请在设置中填写");
        assert_eq!(classify_error(&AiErrorKind::ConnectRefused, "ollama"), "Ollama 未运行,请先启动 Ollama 再试");
        assert_eq!(classify_error(&AiErrorKind::ConnectTimeout, "ollama"), "网络连接失败,请检查网络后重试");
        assert_eq!(classify_error(&AiErrorKind::RequestTimeout, "ollama"), "请求超时,请重试或换本地模型");
        assert_eq!(
            classify_error(&AiErrorKind::Http { code: 429, message: "rate limit".into() }, "ollama"),
            "模型服务错误(HTTP 429):rate limit"
        );
        assert_eq!(classify_error(&AiErrorKind::Http { code: 500, message: "".into() }, "ollama"), "模型服务错误(HTTP 500):无详细信息");
        assert_eq!(classify_error(&AiErrorKind::ModelNotFound, "ollama"), "模型不存在或未安装,请在设置中修改模型名");
        assert_eq!(classify_error(&AiErrorKind::StreamInterrupted, "ollama"), "对话流中断,请重试");
        assert_eq!(classify_error(&AiErrorKind::Other("自定义".into()), "ollama"), "自定义");
    }

    #[test]
    fn classify_error_refused_differs_by_provider() {
        assert_eq!(classify_error(&AiErrorKind::ConnectRefused, "deepseek"), "网络连接失败,请检查网络后重试");
        assert_eq!(classify_error(&AiErrorKind::ConnectRefused, "ollama"), "Ollama 未运行,请先启动 Ollama 再试");
    }

    // ---------- 响应体提取 ----------

    #[test]
    fn extract_error_message_variants() {
        let s = r#"{"error":{"message":"model 'x' not found, try pulling it first"}}"#;
        assert_eq!(extract_error_message(s), "model 'x' not found, try pulling it first");
        assert_eq!(extract_error_message(r#"{"message":"直接结构"}"#), "直接结构");
        assert_eq!(extract_error_message("not json"), "");
        assert_eq!(extract_error_message(r#"{"error":{}}"#), "");
        assert_eq!(extract_error_message(r#"{"error":{"message":123}}"#), "");
    }

    #[test]
    fn extract_completion_text_variants() {
        let s = r#"{"choices":[{"message":{"role":"assistant","content":"你好世界"}}]}"#;
        assert_eq!(extract_completion_text(s), Some("你好世界".to_string()));
        // content 为 null(纯工具回复)→ None(调用方退回 body,parse_tool_calls 可解析)
        assert_eq!(extract_completion_text(r#"{"choices":[{"message":{"content":null}}]}"#), None);
        // content 为空字符串(实测 Ollama 工具回复)→ None,同 null 契约
        assert_eq!(extract_completion_text(r#"{"choices":[{"message":{"content":""}}]}"#), None);
        assert_eq!(extract_completion_text("not json"), None);
        assert_eq!(extract_completion_text(r#"{}"#), None);
    }

    // ---------- 流式工具调用分片(§2.3 工具循环的流式路径) ----------

    #[test]
    fn parse_sse_tool_chunk_first_piece_has_id_name() {
        let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_system_status","arguments":""}}]}}]}"#;
        assert_eq!(
            parse_sse_tool_chunk(line),
            Some(vec![(0, "call_abc".to_string(), "get_system_status".to_string(), String::new())])
        );
    }

    #[test]
    fn parse_sse_tool_chunk_args_delta_piece() {
        let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"C:\\x\"}"}}]}}]}"#;
        assert_eq!(
            parse_sse_tool_chunk(line),
            Some(vec![(0, String::new(), String::new(), "{\"path\":\"C:\\x\"}".to_string())])
        );
    }

    #[test]
    fn parse_sse_tool_chunk_skips_noise() {
        assert_eq!(parse_sse_tool_chunk("data: [DONE]"), None);
        assert_eq!(parse_sse_tool_chunk("data: {}"), None);
        assert_eq!(parse_sse_tool_chunk("not sse"), None);
        assert_eq!(parse_sse_tool_chunk(r#"{"choices":[{"delta":{}}]}"#), None);
        assert_eq!(parse_sse_tool_chunk(r#"data: {"choices":[{"delta":{"content":"纯文本"}}]}"#), None, "纯 content 行归 parse_sse_line,这里必须 None");
    }

    #[test]
    fn finish_stream_merges_chunks_by_index() {
        let chunks = vec![
            (0, "call_1".to_string(), "open_item".to_string(), String::new()),
            (0, String::new(), String::new(), r#"{"path":"C:\x"}"#.to_string()),
        ];
        let out = finish_stream(String::new(), &chunks);
        let v: Value = serde_json::from_str(&out).unwrap();
        let call = &v["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "call_1");
        assert_eq!(call["function"]["name"], "open_item");
        assert_eq!(call["function"]["arguments"], r#"{"path":"C:\x"}"#);
        assert_eq!(v["choices"][0]["message"]["content"], "");
    }

    #[test]
    fn finish_stream_returns_plain_text_without_chunks() {
        assert_eq!(finish_stream("纯文本".to_string(), &[]), "纯文本");
    }

    #[test]
    fn finish_stream_rejects_index_beyond_limit() {
        // 安全加固:远端恶意超大 index(如 40 亿)不能触发巨量 resize OOM,
        // 超上限分片直接丢弃,合并结果不含它们
        let chunks = vec![
            (0, "call_ok".to_string(), "open_item".to_string(), String::new()),
            (MAX_TOOL_CALLS, "call_bad".to_string(), "evil".to_string(), "{}".to_string()),
            (usize::MAX, "call_worse".to_string(), "evil2".to_string(), "{}".to_string()),
        ];
        let out = finish_stream(String::new(), &chunks);
        let v: Value = serde_json::from_str(&out).unwrap();
        let calls = v["choices"][0]["message"]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1, "仅保留合法 index 的分片,实际: {calls:?}");
        assert_eq!(calls[0]["function"]["name"], "open_item");
    }

    #[test]
    fn finish_stream_index_at_boundary_accepted() {
        // 边界:index == 上限-1 合法;index == 上限 被拒
        let chunks = vec![
            (MAX_TOOL_CALLS - 1, "call_last".to_string(), "get_system_status".to_string(), "{}".to_string()),
            (MAX_TOOL_CALLS, String::new(), "bad".to_string(), String::new()),
        ];
        let out = finish_stream(String::new(), &chunks);
        let v: Value = serde_json::from_str(&out).unwrap();
        let calls = v["choices"][0]["message"]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_last");
    }

    #[test]
    fn finish_stream_filters_empty_gap_slots() {
        // index 跳号产生的中间空槽(id/name/arguments 全空)不产出空 tool_call
        let chunks = vec![
            (0, "call_0".to_string(), "open_item".to_string(), "{}".to_string()),
            (5, "call_5".to_string(), "search_files".to_string(), "{}".to_string()),
        ];
        let out = finish_stream(String::new(), &chunks);
        let v: Value = serde_json::from_str(&out).unwrap();
        let calls = v["choices"][0]["message"]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2, "空槽不应产出 tool_call");
        assert_eq!(calls[0]["id"], "call_0");
        assert_eq!(calls[1]["id"], "call_5");
    }

    // ---------- SSE 累积上限(2026-08-14 加固:防恶意端点灌数据内存膨胀) ----------

    #[test]
    fn sse_size_limit_bounds() {
        // 等于上限放行、严格超限才中止(与 MAX_TOOL_CALLS 同类边界测试)
        assert!(sse_size_exceeded(MAX_SSE_BUF_BYTES, 0).is_ok(), "行缓冲等于上限放行");
        assert!(sse_size_exceeded(0, MAX_SSE_FULL_BYTES).is_ok(), "累积文本等于上限放行");
        assert_eq!(
            sse_size_exceeded(MAX_SSE_BUF_BYTES + 1, 0),
            Err(AiErrorKind::StreamInterrupted),
            "行缓冲超限中止"
        );
        assert_eq!(
            sse_size_exceeded(0, MAX_SSE_FULL_BYTES + 1),
            Err(AiErrorKind::StreamInterrupted),
            "累积文本超限中止"
        );
        // 行缓冲与累积文本是"或"关系:任一超限即中止
        assert_eq!(
            sse_size_exceeded(MAX_SSE_BUF_BYTES + 1, MAX_SSE_FULL_BYTES + 1),
            Err(AiErrorKind::StreamInterrupted)
        );
    }

    // ---------- 分片四字段字节统计与响应体上限(2026-08-18 中 1/中 2 加固) ----------

    #[test]
    fn tool_chunks_bytes_counts_id_name_args() {
        // id/name/arguments 三字段都计入(元组第 2/3/4 元素),index 不计
        let chunks = vec![
            (0, "call_abc".to_string(), "open_item".to_string(), r#"{"path":"C:\x"}"#.to_string()),
            (1, String::new(), String::new(), "{}".to_string()),
        ];
        assert_eq!(
            tool_chunks_bytes(&chunks),
            "call_abc".len() + "open_item".len() + r#"{"path":"C:\x"}"#.len() + "{}".len()
        );
        assert_eq!(tool_chunks_bytes(&[]), 0);
    }

    #[test]
    fn oversize_id_name_trips_full_limit() {
        // 中 1 复现场景:arguments 很小但 id 超长——三字段累计后必须触发
        // 与 full 同享的 4MB 上限(此前只累计 args 会放行;严格超限才断,
        // 故 +1 构造超限值)
        let big_id = "i".repeat(MAX_SSE_FULL_BYTES + 1);
        let chunks = vec![(0, big_id, String::new(), String::new())];
        let bytes = tool_chunks_bytes(&chunks);
        assert!(bytes > MAX_SSE_FULL_BYTES, "超长 id 累计字节应超 4MB");
        assert_eq!(sse_size_exceeded(0, bytes), Err(AiErrorKind::StreamInterrupted));
        // 条目数上限:128 条是硬顶(与 MAX_TOOL_CHUNKS 定义一致,代码走查循环内校验)
        assert_eq!(MAX_TOOL_CHUNKS, 128);
    }

    #[test]
    fn content_length_too_large_bounds() {
        // 无 Content-Length(分块/流式)→ 放行,靠流式累积兜底
        assert!(!content_length_too_large(None));
        assert!(!content_length_too_large(Some(0)));
        // 恰等于上限放行(与 sse_size_exceeded "严格超限才断"同语义)
        assert!(!content_length_too_large(Some(MAX_SSE_FULL_BYTES as u64)));
        assert!(content_length_too_large(Some(MAX_SSE_FULL_BYTES as u64 + 1)));
        assert!(content_length_too_large(Some(u64::MAX)));
    }
}
