//! 3.2 function-call 工具调用分发器(纯函数,零 tauri 依赖,单测友好)。
//!
//! 职责(见 docs/Phase3-设计.md §2):
//!   1. `ALL_TOOLS` 工具清单 + `tools_json()` —— 序列化为 OpenAI tools 数组格式,发给模型;
//!   2. `parse_tool_calls()` —— 解析模型回复中的 tool_calls(OpenAI 格式),产出结构化调用;
//!   3. `route()` —— 纯路由:工具名 + 已解析参数 → 执行意图(结构/必填校验,不碰系统调用);
//!   4. `rule_match()` —— 规则兜底:模型不支持 tools 时,靠关键词保住打开/查找两类基础指令。
//!
//! 工具执行在命令层(3.1 的 commands/ai.rs)对 `ToolAction` match 真实命令,本文件不执行任何系统调用。
//! 工具集即白名单:固定 6 个,AI 只能在这 6 个里选,新增工具 = 改本文件清单 + 路由,天然可审计。

use serde_json::{json, Value};

/// 工具规格:清单 + OpenAI tools 数组 JSON(给模型看)。
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

/// 固定 6 工具白名单(名称/描述/JSON Schema 严格照 §2.2 契约表)。
///
/// 注:设计骨架为 `const ALL_TOOLS: &[ToolSpec]`,但 `json!` 宏内部含运行时分配
/// (Vec/String),const 上下文不合法;用 `std::sync::LazyLock`(Rust 1.80+ 标准库,
/// 无新依赖)惰性构造,字段类型与消费方式不变(`ALL_TOOLS.iter()` 照常使用)。
pub static ALL_TOOLS: std::sync::LazyLock<Vec<ToolSpec>> = std::sync::LazyLock::new(|| {
    vec![
        ToolSpec {
            name: "open_item",
            description: "用系统默认方式打开应用/文件/文件夹(路径须为绝对路径)",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "绝对路径" }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "search_apps",
            description: "按名称搜索已安装应用(开始菜单索引,返回 top20 名称+路径)",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "应用名关键字" }
                },
                "required": ["query"]
            }),
        },
        ToolSpec {
            name: "search_files",
            description: "在用户配置目录集合(默认仅桌面)内按文件名搜索",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "文件名关键字,可含扩展名如 pdf" },
                    "dirs": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可选,限定搜索的子目录;越权目录被忽略"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolSpec {
            name: "set_wallpaper",
            description: "把指定图片设为桌面壁纸(绝对路径,jpg/jpeg/png/bmp)",
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "图片绝对路径" }
                },
                "required": ["file_path"]
            }),
        },
        ToolSpec {
            name: "get_system_status",
            description: "查询当前 CPU 使用率/内存/网络速率",
            parameters: json!({ "type": "object", "properties": {} }),
        },
        ToolSpec {
            name: "get_clipboard_history",
            description: "查看剪贴板历史(最近文本,供\"我复制了什么\"类提问)",
            parameters: json!({ "type": "object", "properties": {} }),
        },
        // ---- Phase5 5.3 动态壁纸(设计文档 §3)----
        ToolSpec {
            name: "set_dynamic_wallpaper",
            description: "把图片/视频/网页文件设为桌面壁纸(图片走系统壁纸;视频/html 走动态壁纸层,须先启用动态壁纸;file_path 为绝对路径)",
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "素材绝对路径" },
                    "url": { "type": "string", "description": "可选,http(s) 网页素材(当前需先下载到本地,直接给 url 会被拒)" }
                }
            }),
        },
        ToolSpec {
            name: "stop_dynamic_wallpaper",
            description: "恢复系统壁纸(撤下动态壁纸层)",
            parameters: json!({ "type": "object", "properties": {} }),
        },
    ]
});

/// 把 ALL_TOOLS 序列化为请求体 tools 数组(OpenAI 格式:`{"type":"function","function":{...}}`)。
pub fn tools_json() -> Value {
    let arr: Vec<Value> = ALL_TOOLS
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect();
    Value::Array(arr)
}

/// 模型回复中解析出的单个工具调用(OpenAI 格式 tool_calls 条目)。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    /// arguments 是 JSON 字符串,已解析为结构化 Value(由 route 再做必填/类型校验)
    pub arguments: Value,
}

/// 解析模型回复中的 tool_calls 数组:
/// `choices[0].message.tool_calls[]` → (id, name, arguments 字符串解析为 Value)。
/// 回复不是合法 JSON / 没有 tool_calls / 任一 arguments 解析失败 → None。
pub fn parse_tool_calls(reply: &str) -> Option<Vec<ParsedToolCall>> {
    let root: Value = serde_json::from_str(reply).ok()?;
    let calls = root
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("tool_calls")?
        .as_array()?;
    if calls.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(calls.len());
    for call in calls {
        let obj = call.as_object()?;
        let id = obj.get("id")?.as_str()?.to_string();
        let func = obj.get("function")?;
        let name = func.get("name")?.as_str()?.to_string();
        let args_str = func.get("arguments")?.as_str()?;
        let arguments: Value = serde_json::from_str(args_str).ok()?;
        out.push(ParsedToolCall { id, name, arguments });
    }
    Some(out)
}

/// 路由结果:工具执行意图(纯函数,不碰系统调用;执行在命令层对 ToolAction match)。
#[derive(Debug, Clone, PartialEq)]
pub enum ToolAction {
    Open { path: String },
    SearchApps { query: String },
    SearchFiles { query: String, dirs: Vec<String> },
    SetWallpaper { file_path: String },
    GetSystemStatus,
    GetClipboardHistory,
    // ---- Phase5 5.3 动态壁纸(设计文档 §3):url 保留供未来远端素材扩展,当前执行要求本地 path ----
    SetDynamicWallpaper { path: String, url: Option<String> },
    StopDynamicWallpaper,
}

/// 纯路由:工具名 + 已解析参数 → 执行意图。
/// 校验规则:path/dirs 必须绝对路径;缺必填 → Err;类型错误 → Err;未知工具名 → Err。
/// (dirs 的越权白名单校验在 file_search::ai_search_files 内完成,本函数只做结构与必填校验。)
pub fn route(name: &str, args: &Value) -> Result<ToolAction, String> {
    match name {
        "open_item" => {
            let path = required_str(args, "path")?;
            require_abs(&path, "path")?;
            Ok(ToolAction::Open { path })
        }
        "search_apps" => {
            let query = required_str(args, "query")?;
            Ok(ToolAction::SearchApps { query })
        }
        "search_files" => {
            let query = required_str(args, "query")?;
            let dirs = optional_str_array(args, "dirs")?;
            Ok(ToolAction::SearchFiles { query, dirs })
        }
        "set_wallpaper" => {
            let file_path = required_str(args, "file_path")?;
            require_abs(&file_path, "file_path")?;
            Ok(ToolAction::SetWallpaper { file_path })
        }
        "get_system_status" => {
            ensure_no_args(args)?;
            Ok(ToolAction::GetSystemStatus)
        }
        "get_clipboard_history" => {
            ensure_no_args(args)?;
            Ok(ToolAction::GetClipboardHistory)
        }
        "set_dynamic_wallpaper" => {
            let path = optional_str(args, "file_path")?.unwrap_or_default();
            let url = optional_str(args, "url")?;
            if path.is_empty() && url.is_none() {
                return Err("set_dynamic_wallpaper 需要 file_path 或 url 至少一个".to_string());
            }
            if !path.is_empty() {
                require_abs(&path, "file_path")?;
            }
            Ok(ToolAction::SetDynamicWallpaper { path, url })
        }
        "stop_dynamic_wallpaper" => {
            ensure_no_args(args)?;
            Ok(ToolAction::StopDynamicWallpaper)
        }
        _ => Err(format!("未知工具名: {name}")),
    }
}

/// 规则兜底(Ollama 模型不支持 tools 时保证基础指令可用)。
/// 关键词表(§2.3 明确仅两条,不扩大,避免与模型能力打架):
///   含"打开/启动/运行" → SearchApps(剩余词);
///   含"找/查/搜"       → SearchFiles(剩余词,dirs 空 = 走配置默认目录);
///   其余 → None。
/// 剩余词为空(如指令就是"打开"两个字)视为无有效内容 → None。
pub fn rule_match(instruction: &str) -> Option<ToolAction> {
    const OPEN_KW: [&str; 3] = ["打开", "启动", "运行"];
    // 注意按长度降序排列:"搜索"须先于"搜"删除,否则"搜索"会残留"索"字
    const FIND_KW: [&str; 4] = ["搜索", "找", "查", "搜"];

    // open 类优先(如"打开文件查找器" → SearchApps 而非 SearchFiles);
    // 命中一类后删除该类关键词表中所有出现的词,剩余为查询词
    // (如"查找桌面pdf发票"同时含"找""查",须都删,剩余"桌面pdf发票")。
    for kw in OPEN_KW {
        if instruction.contains(kw) {
            let mut rest = instruction.to_string();
            for k in OPEN_KW {
                rest = rest.replace(k, "");
            }
            let rest = rest.trim().to_string();
            return if rest.is_empty() { None } else { Some(ToolAction::SearchApps { query: rest }) };
        }
    }
    for kw in FIND_KW {
        if instruction.contains(kw) {
            let mut rest = instruction.to_string();
            for k in FIND_KW {
                rest = rest.replace(k, "");
            }
            let rest = rest.trim().to_string();
            return if rest.is_empty() { None } else { Some(ToolAction::SearchFiles { query: rest, dirs: Vec::new() }) };
        }
    }
    // ---- Phase5 5.3:壁纸关键词(长度降序;保守词表宁缺勿误匹配,设计文档 §3.2)----
    // stop 先判(含"停/关"等独立语义),set 后判;与 open/find 无词面冲突
    const STOP_WALLPAPER_KW: [&str; 4] = ["停止壁纸", "关掉壁纸", "关闭壁纸", "停壁纸"];
    const SET_WALLPAPER_KW: [&str; 6] =
        ["设置壁纸", "换成壁纸", "设为壁纸", "改成壁纸", "换壁纸", "设壁纸"];

    for kw in STOP_WALLPAPER_KW {
        if instruction.contains(kw) {
            return Some(ToolAction::StopDynamicWallpaper);
        }
    }
    for kw in SET_WALLPAPER_KW {
        if instruction.contains(kw) {
            return Some(ToolAction::SetDynamicWallpaper { path: String::new(), url: None });
        }
    }
    None
}

// ---- 路由校验辅助(私有) ----

/// 取必填字符串参数:对象里缺失 / 非字符串 / 空字符串 → Err。
fn required_str(args: &Value, key: &str) -> Result<String, String> {
    let obj = args.as_object().ok_or_else(|| "参数必须是 JSON 对象".to_string())?;
    match obj.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
        Some(Value::String(_)) => Err(format!("参数 {key} 为空字符串")),
        Some(_) => Err(format!("参数 {key} 类型错误,应为字符串")),
        None => Err(format!("缺少必填参数 {key}")),
    }
}

/// 取可选字符串参数(如 set_dynamic_wallpaper 的 url):缺失 → Ok(None);
/// 空字符串按缺失处理(宽松);非字符串 → Err。
fn optional_str(args: &Value, key: &str) -> Result<Option<String>, String> {
    let obj = args.as_object().ok_or_else(|| "参数必须是 JSON 对象".to_string())?;
    match obj.get(key) {
        None => Ok(None),
        Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
        Some(Value::String(_)) => Ok(None), // 空串按缺失处理
        Some(_) => Err(format!("参数 {key} 类型错误,应为字符串")),
    }
}

/// 取可选字符串数组参数(如 search_files 的 dirs):缺失 → 空 vec;每个元素须为非空字符串且绝对路径。
fn optional_str_array(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let obj = args.as_object().ok_or_else(|| "参数必须是 JSON 对象".to_string())?;
    match obj.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) if !s.is_empty() => {
                        require_abs(s, key)?;
                        out.push(s.clone());
                    }
                    Value::String(_) => return Err(format!("参数 {key} 包含空字符串")),
                    _ => return Err(format!("参数 {key} 类型错误,应为字符串数组")),
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!("参数 {key} 类型错误,应为字符串数组")),
    }
}

/// 绝对路径校验(Windows:盘符绝对路径 / UNC 路径均通过;空串与相对路径拒绝)。
fn require_abs(p: &str, key: &str) -> Result<(), String> {
    if std::path::Path::new(p).is_absolute() {
        Ok(())
    } else {
        Err(format!("参数 {key} 必须是绝对路径: {p}"))
    }
}

/// 无参工具:只接受 JSON 对象或 null(忽略多余字段,宽松兼容模型端差异)。
fn ensure_no_args(args: &Value) -> Result<(), String> {
    if args.is_object() || args.is_null() {
        Ok(())
    } else {
        Err("该工具不接受参数".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---------- tools_json:6 工具字段完整、required 正确 ----------

    #[test]
    fn tools_json_has_exactly_eight_tools() {
        let tools = tools_json();
        let arr = tools.as_array().expect("tools_json 应为数组");
        assert_eq!(arr.len(), 8, "工具数必须为 8(白名单固定;Phase5 新增壁纸两工具)");
        for t in arr {
            assert_eq!(t["type"], "function", "每个工具条目 type 应为 function");
            let f = &t["function"];
            assert!(f["name"].is_string(), "function.name 缺失");
            assert!(f["description"].is_string(), "function.description 缺失");
            assert!(f["parameters"].is_object(), "function.parameters 缺失");
        }
    }

    #[test]
    fn tools_json_names_match_contract() {
        let tools = tools_json();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "open_item",
                "search_apps",
                "search_files",
                "set_wallpaper",
                "get_system_status",
                "get_clipboard_history",
                "set_dynamic_wallpaper",
                "stop_dynamic_wallpaper",
            ]
        );
    }

    #[test]
    fn tools_json_required_fields_correct() {
        let tools = tools_json();
        let arr = tools.as_array().unwrap();
        let params_of = |name: &str| {
            arr.iter()
                .find(|t| t["function"]["name"] == name)
                .expect("工具存在")["function"]["parameters"]
                .clone()
        };
        // 三个带必填参数的工具
        assert_eq!(params_of("open_item")["required"], json!(["path"]));
        assert_eq!(params_of("search_apps")["required"], json!(["query"]));
        assert_eq!(params_of("search_files")["required"], json!(["query"]));
        assert_eq!(params_of("set_wallpaper")["required"], json!(["file_path"]));
        // 两个无参工具不声明 required
        assert!(params_of("get_system_status").get("required").is_none());
        assert!(params_of("get_clipboard_history").get("required").is_none());
        // 参数类型骨架
        assert_eq!(params_of("search_files")["properties"]["dirs"]["type"], "array");
        assert_eq!(params_of("search_files")["properties"]["dirs"]["items"]["type"], "string");
    }

    // ---------- parse_tool_calls:标准回复 / 纯工具回复 / 空 / 坏 JSON / 转义引号 ----------

    /// 标准 tool_calls 回复(带 content) → 3 个调用解析正确。
    #[test]
    fn parse_standard_tool_calls_reply() {
        let reply = r#"{
            "choices": [{
                "message": {
                    "content": "我来帮你处理",
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "open_item", "arguments": "{\"path\":\"C:\\\\a b\\\\n.txt\"}"}},
                        {"id": "call_2", "type": "function", "function": {"name": "search_apps", "arguments": "{\"query\":\"记事本\"}"}},
                        {"id": "call_3", "type": "function", "function": {"name": "get_system_status", "arguments": "{}"}}
                    ]
                }
            }]
        }"#;
        let calls = parse_tool_calls(reply).expect("标准回复应解析出调用");
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "open_item");
        assert_eq!(calls[0].arguments, json!({"path": "C:\\a b\\n.txt"}));
        assert_eq!(calls[1].name, "search_apps");
        assert_eq!(calls[1].arguments, json!({"query": "记事本"}));
        assert_eq!(calls[2].name, "get_system_status");
        assert_eq!(calls[2].arguments, json!({}));
    }

    /// content 为 null 的纯工具回复(模型只回 tool_calls)同样能解析。
    #[test]
    fn parse_pure_tool_reply_content_null() {
        let reply = r#"{
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [
                        {"id": "c1", "type": "function", "function": {"name": "set_wallpaper", "arguments": "{\"file_path\":\"D:\\\\pic.jpg\"}"}}
                    ]
                }
            }]
        }"#;
        let calls = parse_tool_calls(reply).expect("content null 纯工具回复应解析出调用");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "set_wallpaper");
        assert_eq!(calls[0].arguments, json!({"file_path": "D:\\pic.jpg"}));
    }

    /// 空串 → None。
    #[test]
    fn parse_empty_reply_is_none() {
        assert!(parse_tool_calls("").is_none());
        assert!(parse_tool_calls("  ").is_none());
    }

    /// 坏 JSON / 非对象 / 结构缺失 → None(不 panic)。
    #[test]
    fn parse_malformed_reply_is_none() {
        assert!(parse_tool_calls("not json").is_none());
        assert!(parse_tool_calls("{}").is_none(), "无 choices 应为 None");
        assert!(parse_tool_calls(r#"{"choices":[]}"#).is_none());
        assert!(parse_tool_calls(r#"{"choices":[{"message":{}}]}"#).is_none());
        assert!(
            parse_tool_calls(r#"{"choices":[{"message":{"tool_calls":[]}}]}"#).is_none(),
            "tool_calls 空数组应为 None"
        );
        assert!(
            parse_tool_calls(r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"x","arguments":"{bad"}}]}}]}"#).is_none(),
            "arguments 坏 JSON 应为 None"
        );
    }

    /// arguments 内含转义引号/反斜杠/中文 → 解析为正确的 Value。
    #[test]
    fn parse_arguments_with_escaped_quotes() {
        let reply = r#"{
            "choices": [{
                "message": {
                    "tool_calls": [
                        {"id": "c1", "type": "function", "function": {"name": "open_item", "arguments": "{\"path\":\"C:\\\\Program Files\\\\记事本.exe\",\"extra\":\"含\\\"引号\\\"\"}"}}
                    ]
                }
            }]
        }"#;
        let calls = parse_tool_calls(reply).expect("转义引号 arguments 应解析成功");
        assert_eq!(calls[0].arguments["path"], "C:\\Program Files\\记事本.exe");
        assert_eq!(calls[0].arguments["extra"], "含\"引号\"");
    }

    // ---------- route:合法 / 缺必填 / 类型错 / 未知工具名 ----------

    #[test]
    fn route_valid_arguments() {
        assert_eq!(
            route("open_item", &json!({"path": "C:\\a\\b.txt"})),
            Ok(ToolAction::Open { path: "C:\\a\\b.txt".into() })
        );
        assert_eq!(
            route("search_apps", &json!({"query": "记事本"})),
            Ok(ToolAction::SearchApps { query: "记事本".into() })
        );
        assert_eq!(
            route("search_files", &json!({"query": "发票", "dirs": ["D:\\work", "\\\\nas\\share"]})),
            Ok(ToolAction::SearchFiles { query: "发票".into(), dirs: vec!["D:\\work".into(), "\\\\nas\\share".into()] })
        );
        // dirs 可选:缺失 → 空 vec
        assert_eq!(
            route("search_files", &json!({"query": "发票"})),
            Ok(ToolAction::SearchFiles { query: "发票".into(), dirs: vec![] })
        );
        assert_eq!(
            route("set_wallpaper", &json!({"file_path": "D:\\pic.png"})),
            Ok(ToolAction::SetWallpaper { file_path: "D:\\pic.png".into() })
        );
        // 无参工具:空对象与 null 均合法
        assert_eq!(route("get_system_status", &json!({})), Ok(ToolAction::GetSystemStatus));
        assert_eq!(route("get_clipboard_history", &json!(null)), Ok(ToolAction::GetClipboardHistory));
    }

    #[test]
    fn route_missing_required_fields_is_err() {
        assert!(route("open_item", &json!({})).is_err(), "open_item 缺 path");
        assert!(route("search_apps", &json!({"query": ""})).is_err(), "query 空串");
        assert!(route("search_files", &json!({})).is_err(), "search_files 缺 query");
        assert!(route("set_wallpaper", &json!({"other": "x"})).is_err(), "set_wallpaper 缺 file_path");
    }

    #[test]
    fn route_wrong_types_is_err() {
        assert!(route("open_item", &json!({"path": 123})).is_err(), "path 数字");
        assert!(route("search_apps", &json!({"query": ["记事本"]})).is_err(), "query 数组");
        assert!(route("search_files", &json!({"query": "a", "dirs": "D:\\work"})).is_err(), "dirs 非数组");
        assert!(route("search_files", &json!({"query": "a", "dirs": [123]})).is_err(), "dirs 元素非字符串");
        assert!(route("get_system_status", &json!("hello")).is_err(), "无参工具传字符串");
    }

    #[test]
    fn route_relative_paths_is_err() {
        assert!(route("open_item", &json!({"path": "a\\b.txt"})).is_err(), "相对路径应拒绝");
        assert!(route("open_item", &json!({"path": "C:relative.txt"})).is_err(), "盘符无根(非绝对)应拒绝");
        assert!(route("set_wallpaper", &json!({"file_path": "pic.png"})).is_err());
        assert!(route("search_files", &json!({"query": "a", "dirs": ["relative"]})).is_err());
    }

    #[test]
    fn route_unknown_tool_name_is_err() {
        let err = route("exec_shell", &json!({})).unwrap_err();
        assert!(err.contains("未知工具名"), "错误信息应包含未知工具名,实际: {err}");
    }

    // ---------- rule_match:打开 / 查找 / 非指令文本 ----------

    #[test]
    fn rule_match_open_keywords() {
        assert_eq!(
            rule_match("打开记事本"),
            Some(ToolAction::SearchApps { query: "记事本".into() })
        );
        assert_eq!(
            rule_match("启动计算器"),
            Some(ToolAction::SearchApps { query: "计算器".into() })
        );
        assert_eq!(
            rule_match("帮我运行一下画图"),
            Some(ToolAction::SearchApps { query: "帮我一下画图".into() })
        );
    }

    #[test]
    fn rule_match_find_keywords() {
        assert_eq!(
            rule_match("查找桌面pdf发票"),
            Some(ToolAction::SearchFiles { query: "桌面pdf发票".into(), dirs: vec![] })
        );
        assert_eq!(
            rule_match("搜索壁纸图片"),
            Some(ToolAction::SearchFiles { query: "壁纸图片".into(), dirs: vec![] })
        );
        assert_eq!(
            rule_match("查一下昨天的报表"),
            Some(ToolAction::SearchFiles { query: "一下昨天的报表".into(), dirs: vec![] })
        );
    }

    #[test]
    fn rule_match_non_instruction_is_none() {
        assert_eq!(rule_match("今天天气怎么样"), None);
        assert_eq!(rule_match("你好"), None);
        assert_eq!(rule_match(""), None);
        assert_eq!(rule_match("打开"), None, "剩余词为空 → None");
        assert_eq!(rule_match("查找"), None, "剩余词为空 → None");
    }

    // ---------- Phase5 5.3:动态壁纸工具(设计文档 §3)----------

    #[test]
    fn route_set_dynamic_wallpaper_requires_path_or_url() {
        // 缺全部必填 → Err
        assert!(route("set_dynamic_wallpaper", &json!({})).is_err());
        // 相对路径 → Err
        assert!(route("set_dynamic_wallpaper", &json!({"file_path": "videos\\a.mp4"})).is_err());
        // 合法:绝对路径
        let ok = route("set_dynamic_wallpaper", &json!({"file_path": r"C:\mat\a.mp4"}));
        assert_eq!(
            ok,
            Ok(ToolAction::SetDynamicWallpaper { path: r"C:\mat\a.mp4".to_string(), url: None })
        );
        // 合法:url 网页素材
        let ok2 = route("set_dynamic_wallpaper", &json!({"url": "https://example.com/bg.html"}));
        assert_eq!(
            ok2,
            Ok(ToolAction::SetDynamicWallpaper {
                path: String::new(),
                url: Some("https://example.com/bg.html".to_string())
            })
        );
        // file_path 与 url 同时给 → 接受(file_path 优先消费)
        let ok3 =
            route("set_dynamic_wallpaper", &json!({"file_path": r"C:\mat\a.mp4", "url": "https://x.com/y.html"}));
        assert!(matches!(ok3, Ok(ToolAction::SetDynamicWallpaper { path, url }) if path == r"C:\mat\a.mp4" && url.as_deref() == Some("https://x.com/y.html")));
    }

    #[test]
    fn route_stop_dynamic_wallpaper_no_args() {
        // 空参数 OK;多余字段宽松通过(与 get_system_status 的 ensure_no_args 语义一致)
        assert!(route("stop_dynamic_wallpaper", &json!({})).is_ok());
        assert!(route("stop_dynamic_wallpaper", &json!({"extra": 1})).is_ok());
        assert_eq!(route("stop_dynamic_wallpaper", &json!({})), Ok(ToolAction::StopDynamicWallpaper));
    }

    #[test]
    fn rule_match_set_wallpaper_keywords() {
        assert!(matches!(rule_match("把这个视频设为壁纸"), Some(ToolAction::SetDynamicWallpaper { .. })));
        assert!(matches!(rule_match("换成壁纸"), Some(ToolAction::SetDynamicWallpaper { .. })));
        assert!(matches!(rule_match("设置壁纸"), Some(ToolAction::SetDynamicWallpaper { .. })));
        assert!(matches!(rule_match("改成壁纸"), Some(ToolAction::SetDynamicWallpaper { .. })));
    }

    #[test]
    fn rule_match_stop_wallpaper_keywords() {
        assert!(matches!(rule_match("停止壁纸"), Some(ToolAction::StopDynamicWallpaper)));
        assert!(matches!(rule_match("关闭壁纸"), Some(ToolAction::StopDynamicWallpaper)));
        // 组合词子串回归(3.2 老坑):"设置壁纸"不能误匹配 stop
        assert!(matches!(rule_match("设置壁纸"), Some(ToolAction::SetDynamicWallpaper { .. })));
    }

    #[test]
    fn tools_json_contains_dynamic_wallpaper_tools() {
        let tools = tools_json();
        let arr = tools.as_array().unwrap();
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"set_dynamic_wallpaper"));
        assert!(names.contains(&"stop_dynamic_wallpaper"));
    }
}
