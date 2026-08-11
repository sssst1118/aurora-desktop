# Aurora Phase3 设计(AI 集成)

> 状态:已定稿(2026-08-11)
> 规格依据:[docs/开发文档.md](./开发文档.md) §2 技术栈、§5 命令接口、§6.7 AI 模块、§7 Phase3 里程碑、§8 风险约束、§9 测试要点
> 上级状态:[docs/开发进度.md](./开发进度.md)(Phase2 完成后启动 Phase3)
> 前置条件:Phase2 任务 2.0~2.5 全部 ✅(已满足)
> 代码事实核对基准:本文档所有"复用命令/结构体/配置字段"均已对照 src-tauri 实际代码(2026-08-11 工作树),不凭空写

---

## 0 总览与并行契约

### 0.1 目标

Phase3 在 Phase1(启动器)+ Phase2(效率五模块)基础上做 AI 集成,三个任务模块划分明确、可独立验收、并行可开发:

| # | 模块 | 核心能力 | 验收一句话 |
|---|---|---|---|
| 3.1 | AI 对话面板 + 后端代理 | Rust 后端 HTTP 代理 DeepSeek(云端)/ Ollama(本地)双模式,SSE 流式对话 | 在 AI 面板里用 DeepSeek 聊一句话,流式出字;切 Ollama 模式同样可聊 |
| 3.2 | function-call 工具调用 | AI 输出工具指令,后端分发执行真实 Tauri 命令并回填续答 | 对 AI 说"打开记事本",记事本真的被打开,对话继续 |
| 3.3 | 自然语言文件搜索 | 在用户配置目录集合内按文件名搜索,供 AI 工具与命令调用 | 对 AI 说"查找桌面 pdf 发票",返回文件名/路径列表 |

三条铁律贯穿本阶段(开发文档 §1.2/§8,落地见 0.4):
1. **API Key 只存本地配置文件,永不传前端、不上传网络**;
2. **禁止全盘扫描**:文件搜索只扫用户配置的目录集合;
3. **Ollama 离线模式断网不崩溃**(§9 测试要点)。

### 0.2 三任务一览

| # | 后端新文件(src-tauri/src/) | 前端新文件(src/) | 关键新依赖 | 模块间耦合 |
|---|---|---|---|---|
| 3.1 | commands/ai.rs(命令层+会话无关对话)、ai/client.rs(HTTP 代理:双模式+SSE) | components/core/AIPanel.vue、composables/useAiChat.ts | reqwest(json+stream)、futures-util(可选,SSE 逐行) | 复用 config(密钥/模型配置)、search/open_item;消费 3.2 的 tools.rs 纯函数 |
| 3.2 | ai/tools.rs(工具清单+JSON Schema+解析路由,纯函数) | AIPanel.vue 内工具动作条(改 3.1 的文件,见 0.3 所有权) | 无新依赖 | 工具执行映射到 Phase1/2 已有命令 + 3.3 的 ai_search_files(契约先行) |
| 3.3 | commands/file_search.rs(search_files 命令,目录集合扫描) | 无独立 UI(结果经聊天呈现) | 无新依赖(known-folders 已有) | 复用 drawer.rs 的 desktop_dir()、config(search_roots) |

**耦合结论:3.2/3.3 后端均为纯函数/独立命令文件,与 3.1 之间只有「类型契约 + 事件契约」,无共享可变状态;3.1 的 ai.rs 依赖 3.2 的 ai/tools.rs 的公共类型(签名在本文档 §2.3 定死)。全部可并行,契约以本文档为准,实现偏差由集成 agent 统一对齐。**

### 0.3 并行开发契约(Phase3 开工必读)

#### 共享文件所有权

| 文件 | 维护者 | 约定 |
|---|---|---|
| src-tauri/src/lib.rs(invoke_handler、setup) | **集成 agent 独占** | 模块 agent 不得修改;ai.rs / file_search.rs 的命令注册与 setup 由集成 agent 合入 |
| src-tauri/src/commands/mod.rs | 集成 agent | 同上(mod 声明,新增 ai/file_search) |
| src-tauri/src/commands/config.rs(AppConfig) | 集成 agent | 新增 AI 字段见 0.3.4;**密钥脱敏逻辑在 config_load/config_save 内实现**(见 §1.3),一次合入 |
| src-tauri/capabilities/default.json | 集成 agent | windows 数组追加 "ai_panel";严格 JSON,无注释(错误记录:解析器为严格 JSON) |
| src-tauri/tauri.conf.json | 集成 agent | 注册 ai_panel 窗口(见 0.3.5) |
| src-tauri/Cargo.toml、package.json | 集成 agent | 一次加齐 Phase3 依赖(reqwest/futures-util,见 §1.6) |
| src-tauri/src/hotkey.rs | 集成 agent | ai_hotkey(默认 ctrl+alt+a)呼出 AI 面板 |
| src-tauri/src/tray.rs | 集成 agent | 托盘菜单加"AI 对话"入口 |
| src/App.vue(label 分流) | 集成 agent | 加 ai_panel 渲染分支 |
| src/components/core/Settings.vue、src/stores/config.ts | 集成 agent | AI 设置区块 + AppConfig TS 接口同步(含密钥掩码交互) |
| src-tauri/src/commands/ai.rs | **3.1 模块 agent 独占** | 含工具执行分支(execute_tool_action);3.2 合入后按其契约扩展 |
| src-tauri/src/ai/client.rs | 3.1 模块 agent | 独占 |
| src-tauri/src/ai/tools.rs | **3.2 模块 agent 独占** | 纯函数,零 tauri 依赖,可最先合入不冲突 |
| src-tauri/src/commands/file_search.rs | **3.3 模块 agent 独占** | 独立命令,可最先合入不冲突 |
| src/components/core/AIPanel.vue、src/composables/useAiChat.ts | 3.1 模块 agent | 独占;3.2 的动作条展示在该文件内追加(3.1 预留事件消费分支,见 §2.4) |
| 其余全部新文件(见 0.2 表) | 各自模块 agent | 独占,零重叠 |

#### 协作流程

1. Phase2 全部 ✅ 后,集成 agent 先做**骨架合并**(一次 commit):ai_panel 窗口、capabilities、Cargo.toml 依赖、AppConfig AI 字段(带 `#[serde(default)]` + 密钥脱敏)、invoke_handler 占位、App.vue ai_panel 分支、hotkey/托盘入口,全部到位;
2. 随后 3 个模块 agent 并行开工:**3.2/3.3 先合**(纯文件零依赖),3.1 的 ai.rs 以其契约开发;
3. 各模块完成后 merge 到 main;集成 agent 负责最终全量编译 + 回归(全量 `cargo test` + `pnpm build`);
4. 冲突预案:任何 agent 发现共享文件被他人改动,先 pull 再基于最新版提交,禁止 force push;共享 checkout 上未完成文件勿提交半成品(错误记录:整树编译会被半成品拖垮,验证用 `git worktree add` 隔离)。

#### 事件契约(模块间唯一共享通道)

AI 流式对话的全部事件统一走**单事件 `ai-event`**,payload 带 kind 区分阶段(一个 listen 消费整条流,比 Phase2 多事件风格更适合时序流):

| 事件名 | kind | payload | 发布者 | 语义 |
|---|---|---|---|---|
| `ai-event` | `chunk` | `{kind:"chunk", delta:string}` | 3.1 SSE 循环(每增量) | 增量文本,前端追加到当前助手消息 |
| `ai-event` | `tool` | `{kind:"tool", tool:string, args:string}` | 3.2 工具执行前(3.1 ai.rs 发布) | 工具动作条展示(如"正在打开 记事本") |
| `ai-event` | `done` | `{kind:"done", full:string}` | 3.1 SSE 循环结束(含工具循环最终文本) | 流结束,完整回复落地 |
| `ai-event` | `error` | `{kind:"error", message:string}` | 3.1 异常路径 | 断网/超时/Ollama 未启动/密钥缺失/工具失败,前端错误条 |

payload 结构属公共契约,不允许单方面改字段;工具执行细节(工具名/参数摘要)由前端展示层自定义,不进契约。

#### AppConfig 扩展规则

Phase3 新增字段**全部带 ai 前缀**,由集成 agent 一次合入(config.rs,沿用 `#[serde(default)]` 铁律,Phase2 已整体声明,新增字段自动获得回退):

```rust
// ---- Phase3 AI 集成 ----
pub enable_ai: bool,                 // 总开关,默认 false;关闭时不注册热键、不显示面板入口
pub ai_provider: String,             // "deepseek" | "ollama",默认 "deepseek"
pub ai_api_key: Option<String>,      // 仅 DeepSeek 用;存文件明文,前端永远只见掩码(§1.3)
pub ai_model: String,                // 默认 "deepseek-chat"
pub ai_base_url: String,             // 默认 "https://api.deepseek.com/v1"
pub ai_ollama_url: String,           // 默认 "http://127.0.0.1:11434/v1"
pub ai_ollama_model: String,         // 默认 "qwen2.5:7b"(中文场景可用;用户按已装模型改)
pub ai_tools_enabled: bool,          // 工具调用总开关,默认 true
pub ai_search_roots: Vec<String>,    // 3.3 搜索目录集合,默认空 = 仅桌面(禁止全盘)
pub ai_max_tool_rounds: u32,         // 工具循环上限,默认 3(防死循环)
pub ai_hotkey: String,               // 默认 "ctrl+alt+a"
```

**铁律:AppConfig 结构体整体 `#[serde(default)]` 已存在,集成 agent 合入字段时不得移除;config_load 命令返回前对 `ai_api_key` 脱敏(§1.3),config_save 对掩码占位保留旧值。**

#### 窗口注册与权限

- ai_panel 窗口**静态注册在 tauri.conf.json**(当前文件里还没有,需新增;对齐开发文档 §4.1 规划,尺寸 540×680):

| label | 尺寸 | 特性 | 默认可见 |
|---|---|---|---|
| ai_panel | 540×680 | 无边框/居中/skipTaskbar;**不透明**(对话面板有内容背景,省渲染)/**不置顶**(聊天窗口无需盖一切) | 隐藏,热键/托盘呼出 |

```json
{ "label": "ai_panel", "title": "Aurora-AI", "width": 540, "height": 680,
  "center": true, "decorations": false, "transparent": false,
  "alwaysOnTop": false, "resizable": false, "visible": false, "skipTaskbar": true }
```

- capabilities/default.json 的 `windows` 数组追加 `"ai_panel"`(错误记录:capabilities 用严格 JSON 解析器,不写注释;不存在的权限先 grep 本机 crate permissions 目录);
- 前端事件订阅/窗口操作所需权限:`core:default`(含 core:event:default listen/unlisten)已在 Phase1 配好,Phase3 **不新增任何 capability 权限**——HTTP 请求全部走 Rust 后端,前端不直接联网;
- 多入口沿用单入口模式:ai_panel 的 url 指向 index.html,main.ts 默认落到 App.vue,App.vue 按 label 分流挂 AIPanel.vue(与 dock/drawer/clipboard 同款,集成 agent 在 App.vue 加一行分支)。

### 0.4 全局铁律在 Phase3 的落地

| 铁律 | Phase3 落地 |
|---|---|
| API Key 不进前端/不上网络 | 密钥只在 Rust 侧注入请求头;config_load 返回掩码(§1.3);日志/错误信息不得包含密钥(仅记 base_url+状态码);前端任何请求都不带密钥 |
| 禁止全盘扫描 | 3.3 只扫 `ai_search_roots` ∪ 桌面目录,递归深度 ≤3、命中上限 20、单次扫描有超时(§3.2);AI 工具传入的 dirs 参数同样受白名单约束 |
| 权限边界(普通用户) | 全部工具执行复用 Phase1/2 已验证的普通权限 API(open_item/wallpaper SystemParametersInfoW/sys 采样),无新增提权路径 |
| 内存 <120MB | reqwest::Client 惰性创建(OnceLock,不聊天零网络线程);SSE 任务结束即回收;对话消息上限 40 条截断;Phase3 收尾复测空闲内存基线 |
| 后台轮询有节制 | AI 无任何轮询:流式靠事件推送;工具循环在单次 invoke 内同步完成;搜索是瞬态任务,不建索引、无常驻线程 |
| 普通用户权限 | 同"权限边界":无管理员依赖 |

### 0.5 与 Phase1/2 的接口对接清单

- 复用命令(真实签名已核实):
  - `search::open_item(path: String) -> bool`——打开应用/文件/文件夹(Phase1);
  - `search::search_apps(query: String, index: State<Mutex<AppIndex>>) -> Vec<AppEntry>`——应用搜索(Phase1,需 State,故工具执行器必须在命令层持 State);
  - `commands::wallpaper::wallpaper_set_static_cmd(file_path: String) -> Result<(), String>`(invoke 名 `wallpaper_set_static`)——设置壁纸(Phase2 2.4);
  - `commands::system::sys_get_status() -> SysStatus`(cpu/mem/net)——系统状态(Phase2 2.5);
  - `commands::clipboard::clipboard_get_history() -> Vec<ClipboardItem>`——剪贴板历史(Phase2 2.3);
  - `commands::drawer::desktop_dir() -> Option<PathBuf>`——桌面目录(FOLDERID_Desktop,3.3 复用;只读复用,不改 drawer.rs);
  - `commands::config::{config_load, config_save, load_from, save_to}`——配置(Phase1,密钥/模型/搜索目录全走它);
- 新增命令(3.3 实现,契约见 §3.2):`ai_search_files`(搜索命令本体,供前端/工具两用);
- 新增命令(3.1 实现):`ai_chat_completion` / `ai_chat_stream` / `ai_execute_tool`(契约见 §1.2/§2.3);
- 事件:新增 `ai-event`(§0.3),与现有 `sys-status`/`clipboard-updated`/`drawer-updated` 并存;
- 托盘:加"AI 对话"菜单项(集成 agent 改 tray.rs);热键:hotkey.rs 追加 ai_hotkey(注册失败仅告警,同 drawer/clipboard 模式)。

---

## 1 AI 对话面板 + 后端代理(3.1)

### 1.1 范围

**做**

| 功能 | 说明 |
|---|---|
| AI 对话窗口 | label=ai_panel,540×680 不透明无边框;呼出 = `ai_hotkey`(默认 ctrl+alt+a)+ 托盘菜单;`enable_ai=false` 时不注册热键 |
| 双模式代理 | Rust 后端 HTTP 代理:`ai_provider=deepseek` → OpenAI 兼容云端 API;`ollama` → 本地 `http://127.0.0.1:11434/v1`;**前端不直接访问任何模型接口** |
| 流式输出 | SSE(server-sent events)逐行解析,增量 emit `ai-event{kind:chunk}`,前端边收边显示;SSE 不可用/解析失败自动降级为一次性返回完整文本 |
| 会话管理 | **无状态后端**:前端(useAiChat.ts)维护消息数组,invoke 全量传(截断最近 40 条);后端不存会话,模块解耦、可测 |
| 密钥/模型配置 | 复用 config.json(§1.3),Settings 页 AI 区块可改;运行中改配置下次请求生效(每次请求读配置,不缓存) |
| 清空对话 | 前端按钮,本地清空即可(无后端会话) |
| 断网降级 | DeepSeek 连不上/Ollama 未启动 → 结构化中文错误 emit `ai-event{kind:error}`,面板显示错误条,应用不崩溃(§9 测试要点) |

**不做**

- 会话历史落盘/多会话管理、Markdown 渲染、代码高亮(Phase4 主题与体验完善);
- 流式逐字打字机效果之外的速度优化、token 用量统计;
- 图片/多模态输入(DeepSeek-chat 无多模态,后续模型支持再议);
- 自定义 OpenAI 兼容第三方 provider(本期仅 deepseek/ollama 二选一,base_url 可改但 UI 不展开)。

### 1.2 后端:Rust 命令与 HTTP 客户端

文件:`src-tauri/src/commands/ai.rs`(命令层+工具执行分支)、`src-tauri/src/ai/client.rs`(HTTP 代理,3.1 独占)

| 命令 | 签名 | 实现要点 |
|---|---|---|
| ai_chat_completion | `(app: AppHandle, messages: Vec<ChatMessage>, model: Option<String>, base_url: Option<String>) -> Result<String, String>` | 非流式;`model`/`base_url` 为 None 或空串时用配置值(deepseek 模式还要带密钥);对齐开发文档 §5 契约 `{messages,model,base_url?}` |
| ai_chat_stream | `(app: AppHandle, messages: Vec<ChatMessage>, model: Option<String>, base_url: Option<String>) -> Result<(), String>` | 流式:内部 spawn 异步任务,SSE 逐行解析 → emit `ai-event`(chunk/tool/done/error);任务结束即回收;返回 Err 仅代表"任务未能启动"(参数非法等) |
| ai_execute_tool | `(app: AppHandle, instruction: String) -> AiToolResult` | 3.2 契约入口(§2.3):单轮带 tools 对话 → 有 tool_call 执行单一工具 → 返回 `{ok, action, msg}`;无 tool_call 走规则兜底 |

```rust
// ai.rs 公共类型(与前端 TS 一一对应)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage { pub role: String, pub content: String }   // "user"/"assistant"/"system"

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiToolResult {
    pub ok: bool,
    pub action: String,   // "open_item"/"set_wallpaper"/"search_files"/"search_apps"/"get_system_status"/"get_clipboard_history"/"none"
    pub msg: String,      // 执行结果摘要(成功:如"已打开 记事本";失败/未匹配:模型原文或错误)
}

// client.rs 双模式差异(唯一网络出口)
// 请求体统一走 OpenAI 兼容 Chat Completions 格式:
// POST {base_url}/chat/completions, Authorization: Bearer {key}(仅 deepseek)
// 双端差异仅:endpoint(base_url)、model、是否带 key、stream 兼容性(两者都支持 SSE)
```

**双模式差异表**

| 维度 | DeepSeek(云端) | Ollama(本地) |
|---|---|---|
| base_url(默认) | `https://api.deepseek.com/v1` | `http://127.0.0.1:11434/v1` |
| model(默认) | `deepseek-chat` | `qwen2.5:7b`(按本机已装模型修改) |
| 认证 | `Authorization: Bearer {ai_api_key}`(无 key 直接报"未配置密钥") | 无 |
| 连接超时 | 15s(网络不可达/断网快速失败,不长时间挂起) | 3s(本机拒绝连接 = Ollama 未启动,秒级提示) |
| 总请求超时 | 120s(DeepSeek 长思考) | 120s(本地推理慢) |
| 流式 | SSE,`data: {...}` 行 + 末尾 `data: [DONE]` | 同左(OpenAI 兼容端点支持 stream) |
| 工具支持 | 完整(deepseek-chat 支持 function calling) | 模型相关(如 qwen2.5/llama3.1+ 支持 tools);不支持时走规则兜底(§2.3) |
| 参数 | temperature 0.7、max_tokens 不设(模型默认) | 同左;options 不传(用 Ollama 默认) |

**SSE 流式方案(已定实现方式)**

1. `reqwest::Client`(全局 `OnceLock`,带 connect_timeout;`features=["json","stream"]`)POST 请求,`stream=true` 放 body;
2. `response.bytes_stream()` 逐块累积到缓冲,按 `\n` 切行;每行 `data: ` 前缀去掉,`[DONE]` 结束;
3. 每行 JSON 解析失败**跳过不中断**(容错,错误记录:解析鲁棒性优先);解析出 `choices[0].delta.content` 非空 → emit `ai-event{kind:chunk, delta}`;
4. 读流期间 idle 超时 30s(两个 chunk 之间无数据视为断线)→ emit error;
5. 结束 emit `ai-event{kind:done, full}`(full = 全部 delta 拼接);
6. 任何阶段出错:emit `ai-event{kind:error, message: 中文错误}`,**不 panic、不崩溃**(§9 铁律)。
7. 降级:若响应头非 SSE(Content-Type 不含 text/event-stream)或 2 秒内收到完整 JSON 正文,按非流式一次性文本 emit done。

**错误分类(统一 map 为中文,前端错误条展示)**

| 错误 | 判定 | 提示文案 |
|---|---|---|
| 密钥缺失 | deepseek 模式且 key 为空 | "未配置 API Key,请在设置中填写" |
| Ollama 未启动 | 连接拒绝(refused) | "Ollama 未运行,请先启动 Ollama 再试" |
| 断网/不可达 | 连接超时/DNS 失败 | "网络连接失败,请检查网络后重试" |
| 服务端错误 | HTTP 4xx/5xx | "模型服务错误(HTTP {code}):{message}" |
| 流中断 | idle 超时/EOF 异常 | "对话流中断,请重试" |
| 请求超时 | 总时长超限 | "请求超时,请重试或换本地模型" |

### 1.3 密钥存储方案(复用 config 机制 + 脱敏契约)

- 落盘:沿用现有 `config.rs`——`%APPDATA%\com.aurora.desktop\config.json` 的 `ai_api_key` 字段(明文存文件,文件权限受用户目录 ACL 保护;不引凭据管理器,保持与 Phase1/2 同路线);
- **前端永不接触明文密钥**,在 config_load/config_save 命令内脱敏(集成 agent 在 config.rs 实现,纯函数可单测):

```rust
/// config_load 返回前端前:已配置 → 返回掩码占位 "******";未配置 → None
fn mask_key(key: &Option<String>) -> Option<String> {
    key.as_deref().filter(|k| !k.is_empty()).map(|_| "******".to_string())
}
/// config_save 写入前:若前端传回的 ai_api_key 是掩码占位,用磁盘旧值覆盖(只改其他字段)
fn resolve_key_save(prev: &Option<String>, incoming: &Option<String>) -> Option<String> {
    match incoming.as_deref() {
        Some("******") => prev.clone(),   // 掩码 = 未修改,保留旧密钥
        _ => incoming.clone(),            // 新值或 None(清空)直接生效
    }
}
```

- 前端交互:Settings AI 区块密钥输入框按 password 型;已配置时显示掩码 "******"(从 config_load 拿到的就是掩码),用户不改则不覆盖;点"清除"传空串删除;
- 铁律延伸:请求日志/错误信息不得打印密钥;reqwest header 注入仅在 client.rs 一处;`ai_chat_completion`/`ai_chat_stream` 入参不含 key(密钥只从配置读,调用方传不进来)。

### 1.4 前端:AIPanel.vue + useAiChat.ts + 窗口入口

- `src/composables/useAiChat.ts`(开发文档 §3 已规划):

```ts
export interface ChatMessage { role: "user" | "assistant" | "system"; content: string }
export interface AiEvent {
  kind: "chunk" | "tool" | "done" | "error";
  delta?: string; tool?: string; args?: string; full?: string; message?: string;
}
export function useAiChat() {
  // messages: Ref<ChatMessage[]>(最近 40 条)
  // streaming: Ref<boolean>
  // send(text: string):  push user → invoke("ai_chat_stream",{messages: 截断后数组})
  //                      → listen("ai-event") 消费 chunk 追加/tool 记录动作/done 落地/error 置错误条
  // stop(): unlisten + streaming=false(显示"已停止")
  // clear(): messages=[]
}
```

- `src/components/core/AIPanel.vue`:顶部栏(标题"AI 助手"、设置按钮→打开 Settings、清空对话)+ 消息列表(用户右/助手左,助手消息纯文本 + `<pre>` 代码块,不引 markdown 库;流式期间光标 `▋`)+ 工具动作条(收到 `tool` 事件显示 chip,如"🔧 正在打开 记事本",见 §2.4)+ 输入框(Enter 发送/Shift+Enter 换行,streaming 时回车改为停止)+ 错误条(error 事件红色提示,3s 后可点击关闭);
- 样式:深色卡片风(不透明窗口,`bg-gray-950/95` + 边框),与 Phase2 毛玻璃面板视觉一致但不依赖透明;
- 入口:App.vue 加 `label === "ai_panel"` 分支(集成 agent);呼出逻辑走 hotkey.rs + tray.rs(集成 agent)。

### 1.5 降级行为汇总

| 场景 | 行为 |
|---|---|
| 断网(deepseek) | 15s 超时 → error "网络连接失败" → 面板错误条,应用正常;可切 ollama 模式继续用 |
| Ollama 未启动 | 3s 连接拒绝 → error "Ollama 未运行" → 提示用户启动;不崩溃、不重试风暴(单次请求一次尝试) |
| 密钥缺失 | 请求前校验 → error "未配置 API Key" → 面板显示"去设置"按钮(Settings 页) |
| enable_ai=false | 热键不注册、托盘入口隐藏、窗口不显示;已打开窗口保持(下次启动不再显示) |
| 流中断 | error + 已收文本保留(前端不丢已显示的增量) |
| 模型不支持工具 | 走规则兜底(§2.3),只保证 open_item/search_files 关键词指令可用 |

### 1.6 依赖与数据流

```
AIPanel.vue ──invoke ai_chat_stream──> commands/ai.rs ──> ai/client.rs(reqwest Client)
    │              └── spawn 任务:组装 body(tools 见 §2.3)→ POST → SSE 逐行
    │              └── tool_calls → tools.rs::route → 命令层执行 → 回填 role:tool → 再请求(≤ ai_max_tool_rounds)
    └──listen("ai-event") <── emit(chunk/tool/done/error)
Settings.vue ──config_save/load──> config.rs(掩码契约 §1.3)
```

新增依赖(集成 agent 一次合入 Cargo.toml):

```toml
# AI 代理:OpenAI 兼容 HTTP 客户端;Windows 上默认 native-tls = schannel(系统证书库),
# 无需 OpenSSL 环境;json=请求/响应序列化,stream=SSE bytes_stream
reqwest = { version = "0.12", features = ["json", "stream"] }
# SSE 逐行解析用 StreamExt(实现时可选手写缓冲读取,二选一,不引也可)
futures-util = "0.3"
```

- 前端无新 npm 依赖(`@tauri-apps/api` 的 invoke/listen 已具备);
- reqwest 的 async 在 Tauri 2 自带 tokio runtime 上运行,无需显式加 tokio;超时用 reqwest 的 `timeout()`(整体) + `connect_timeout()`,idle 超时在读取循环内用 `tokio::time::timeout`(tauri 依赖树已含 tokio,直接 `use tokio::time` 需要显式声明——**若编译报未声明依赖,由集成 agent 在 Cargo.toml 补 `tokio = { version="1", features=["time"] }`**,3.1 agent 在模块注释里标注即可)。

### 1.7 测试要点(3.1)

- Rust 单测(client.rs 纯函数):
  - 请求体构造:双模式(messages/stream/tools/认证头差异)序列化正确,deepseek 模式带 Bearer、ollama 不带;
  - SSE 行解析:`data: {...}` 提取、`[DONE]` 终止、空行/注释行/坏 JSON 跳过不 panic、多行 JSON 拼接;
  - 错误分类纯函数:超时/拒绝/HTTP 码 → 中文文案(§1.2 表);
  - 掩码契约(mask_key / resolve_key_save):未配置→None;已配置→"******";保存掩码→保留旧值;传新值/空→覆盖/清空(此两函数在 config.rs,集成 agent 合入时带单测);
  - 消息截断 40 条逻辑;
- 手动验收:deepseek 聊一句流式出字;中途断网 → 错误条不崩溃;切 ollama(未启动)→ "Ollama 未运行";启动 ollama 后正常;`ai_chat_completion` 非流式路径同样可用;热键 ctrl+alt+a 呼出/隐藏;重启应用配置保留;空闲内存复测不劣化。

### 1.8 技术决策(3.1)

| 决策点 | 选择 | 理由 |
|---|---|---|
| 前端直连 vs 后端代理 | **后端代理(铁律)** | 开发文档 §1.2/§6.7 定死;密钥只留在 Rust 侧,前端任何请求不带密钥;能力上还规避 WebView CORS |
| 会话状态位置 | 前端维护消息数组,后端无状态 | 无状态后端可测、无并发同步问题;40 条截断控制请求体;DeepSeek 上下文窗口充足 |
| 流式方案 | SSE 逐行 + emit 单事件 `ai-event` | 与 OpenAI 兼容 API 原生契合;事件驱动零轮询(铁律);降级路径保底 |
| 双模式差异最小化 | 统一 OpenAI 兼容格式,仅 endpoint/key/model 不同 | Ollama 原生 API 与 OpenAI 兼容端点能力对齐,一份解析代码走两端 |
| 密钥存储 | config.json 明文 + 命令层脱敏掩码 | 复用 Phase1 配置机制零新依赖;掩码契约保证前端无明文;不引 Windows Credential Manager(普通用户场景收益低) |
| Client 生命周期 | 全局 OnceLock 惰性创建 | 不聊天零常驻网络资源(内存铁律);首次请求才建连接池 |

---

## 2 function-call 工具调用(3.2)

### 2.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 工具清单 | 6 个工具,名称/描述/参数 JSON Schema 见 2.2,全部映射到**真实存在的命令**(签名已核实) |
| 工具循环 | AI 回复含 tool_calls → 后端分发执行 → 结果以 role:tool 回填 → 再次请求 → 直至自然语言回复;循环上限 `ai_max_tool_rounds`(默认 3)防死循环 |
| 动作可见性 | 工具执行时 emit `ai-event{kind:tool}`,前端动作条展示 |
| ai_execute_tool 命令 | 开发文档 §5 契约 `{instruction} → {ok, action, msg}`:单轮工具执行 + 无工具能力时的规则兜底 |
| 总开关 | `ai_tools_enabled=false` 时不带 tools 参数(纯对话,工具字段不出现) |
| 失败隔离 | 单个工具执行失败不影响对话(错误回填给模型,模型可解释/换招) |

**不做**

- 任意 shell 命令执行(工具集固定白名单,不开 `exec`/`cmd` 类工具——权限边界铁律);
- 前端驱动的半自动工具确认弹窗(轻量优先,工具集已最小化且无破坏性操作);
- 工具结果渲染富交互(壁纸预览/搜索结果卡片等,Phase4 主题系统时做)。

### 2.2 工具契约(名称/描述/JSON Schema → 真实命令映射表)

| 工具名 | 描述(给模型看) | 参数 JSON Schema | 后端执行(真实命令,签名已核实) | 返回给模型 |
|---|---|---|---|---|
| `open_item` | 用系统默认方式打开应用/文件/文件夹(路径须为绝对路径) | `{"type":"object","properties":{"path":{"type":"string","description":"绝对路径"}},"required":["path"]}` | `search::open_item(path) -> bool` | `{"ok":bool,"msg":"已打开 记事本"/"打开失败"}` |
| `search_apps` | 按名称搜索已安装应用(开始菜单索引,返回 top20 名称+路径) | `{"type":"object","properties":{"query":{"type":"string","description":"应用名关键字"}},"required":["query"]}` | `search::search_apps(query, State<Mutex<AppIndex>>) -> Vec<AppEntry>` | 前 5 条 `名称 | 路径` 摘要 |
| `search_files` | 在用户配置目录集合(默认仅桌面)内按文件名搜索 | `{"type":"object","properties":{"query":{"type":"string","description":"文件名关键字,可含扩展名如 pdf"},"dirs":{"type":"array","items":{"type":"string"},"description":"可选,限定搜索的子目录;越权目录被忽略"}},"required":["query"]}` | `commands::file_search::ai_search_files(query, dirs)`(3.3 实现,契约见 §3.2) | 前 5 条 `名称 | 路径`,或"未找到" |
| `set_wallpaper` | 把指定图片设为桌面壁纸(绝对路径,jpg/jpeg/png/bmp) | `{"type":"object","properties":{"file_path":{"type":"string","description":"图片绝对路径"}},"required":["file_path"]}` | `wallpaper_set_static_cmd(file_path) -> Result<(),String>`(复用其校验:绝对路径+扩展名白名单+文件存在) | `{"ok":true,"msg":"壁纸已更换"}` 或错误 |
| `get_system_status` | 查询当前 CPU 使用率/内存/网络速率 | `{"type":"object","properties":{}}` | `sys_get_status() -> SysStatus` | 格式化 `CPU 12% / 内存 5.8G/16G / 上行 1.2M/s 下行 3.4M/s` |
| `get_clipboard_history` | 查看剪贴板历史(最近文本,供"我复制了什么"类提问) | `{"type":"object","properties":{}}` | `clipboard_get_history() -> Vec<ClipboardItem>` | 前 20 条文本摘要(每条约 100 字符截断;图片路径条目标注"图片") |

- 工具集即白名单:固定 6 个,AI 只能在这 6 个里选;新增工具 = 改 tools.rs 清单 + 映射表,天然可审计;
- 与开发文档 §6.7 示例指令的对应:打开记事本 → `open_item`;把这张图设为壁纸 → `set_wallpaper`(前端可先把"当前选中/最近壁纸路径"作为上下文注入,3.2 不做拖拽传图,见 2.1 不做);查找桌面 pdf 发票 → `search_files`。

### 2.3 后端分发器(ai/tools.rs + ai.rs 执行分支)

文件:`src-tauri/src/ai/tools.rs`(3.2 独占,纯函数零 tauri 依赖,单测友好)

```rust
/// 工具规格:清单 + OpenAI tools 数组 JSON(给模型看)
pub struct ToolSpec { pub name: &'static str, pub description: &'static str, pub parameters: serde_json::Value }
pub const ALL_TOOLS: &[ToolSpec] = &[ /* 上表 6 个 */ ];

/// 把 ALL_TOOLS 序列化为请求体 tools 数组(serde_json::json! 包装即可,纯函数)
pub fn tools_json() -> serde_json::Value;

/// 解析模型回复中的 tool_calls 数组(OpenAI 格式):
/// choices[0].message.tool_calls[] → (id, name, arguments 字符串);解析失败/为空 → None
pub fn parse_tool_calls(reply: &str) -> Option<Vec<ParsedToolCall>>;
pub struct ParsedToolCall { pub id: String, pub name: String, pub arguments: serde_json::Value }

/// 路由:工具名 + 已解析参数 → 执行意图(纯函数,不碰系统调用)
pub fn route(name: &str, args: &serde_json::Value) -> Result<ToolAction, String>;
pub enum ToolAction {
    Open { path: String },
    SearchApps { query: String },
    SearchFiles { query: String, dirs: Vec<String> },
    SetWallpaper { file_path: String },
    GetSystemStatus,
    GetClipboardHistory,
}
// 校验规则:path/dirs 必须绝对路径;dirs 白名单校验(⊆ ai_search_roots ∪ 桌面)在
// file_search::ai_search_files 内完成(§3.2),route 只做结构与必填校验
```

命令层执行分支(在 ai.rs,3.1 拥有;对 ToolAction 做 match,调真实命令,把结果拼成回填 JSON):

```rust
// ai.rs 内部(工具循环核心,单次 ai_chat_stream 任务内)
async fn run_tool_loop(client, cfg, messages) -> String {
    let mut msgs = messages;
    for round in 0..cfg.ai_max_tool_rounds {
        let reply = client.chat(&msgs, &tools_json()).await?;      // 带 tools 请求
        let Some(calls) = tools::parse_tool_calls(&reply) else { return reply }; // 无工具 → 直接返回
        emit!("ai-event", {kind:"tool", tool:name, args:...});     // 动作条
        for call in calls {
            let result = match tools::route(&call.name, &call.arguments)? {   // 纯路由
                ToolAction::Open{path} => search::open_item(path),
                ToolAction::SearchApps{query} => search_apps(query, &index_state),
                ToolAction::SearchFiles{query, dirs} => file_search::ai_search_files(query, Some(dirs)),
                ToolAction::SetWallpaper{file_path} => wallpaper_set_static_cmd(file_path).map(|_| true).unwrap_or(false),
                ToolAction::GetSystemStatus => sys_get_status(),
                ToolAction::GetClipboardHistory => clipboard_get_history(),
            };   // 单测:route + 假执行器组合可测(执行分支薄,逻辑在 route)
            msgs.push({role:"tool", tool_call_id:call.id, content: 结果JSON});   // OpenAI 回填格式
        }
        emit!("ai-event", {kind:"chunk", delta: 空}); // 工具轮次后模型续答的增量继续流
    }
    format!("{最后回复}\n\n(已停止:工具调用超过 {ai_max_tool_rounds} 轮上限)")
}
```

- **防死循环上限**:`ai_max_tool_rounds`(默认 3);一轮 = 一次带 tools 的请求往返;超限终止并提示,绝不无限循环;
- **ai_execute_tool 命令**(开发文档 §5 契约,独立于流式循环):`(app, instruction: String) -> AiToolResult`——内部:组装单条 user 消息 + tools → 一次请求;有 tool_call → 执行**第一个**工具并返回 `{ok, action, msg: 结果摘要}`;无 tool_call → 规则兜底(见下);兜底失败 → `{ok:false, action:"none", msg: 模型原文}`;
- **规则兜底**(Ollama 模型不支持 tools 时保证基础指令可用,纯函数 `rule_match(instruction) -> Option<ToolAction>`,单测友好):关键词表——含"打开/启动/运行"→ `SearchApps{query=剩余词}` 再转 `Open{path=首个命中}`;含"找/查/搜"→ `SearchFiles{query=剩余词}`;其余 → None。仅两条,不扩大(避免与模型能力打架)。

### 2.4 前端(动作条,改 3.1 的 AIPanel.vue)

- useAiChat 收到 `tool` 事件 → 追加一条"工具动作"记录(tool 名 + args 摘要)到消息流内联展示(chip 样式,如 `🔧 正在打开 记事本`),模型续答内容照常追加;
- 不做工具确认弹窗(2.1 不做);工具结果是否再追问由模型自然处理。

### 2.5 依赖与数据流

```
AI 回复 ──parse_tool_calls──> tools.rs(纯函数)
    └──> route ──ToolAction──> ai.rs 执行分支 ──match──> 真实命令(open_item/search_apps/…)
    └──> 结果 JSON 回填 role:tool ──> 再次请求(≤3 轮)
ai_execute_tool(instruction) ──> 单轮同流程 + 规则兜底 ──> AiToolResult{ok,action,msg}
```

无新依赖(全部复用现有命令 + 3.3 的 ai_search_files)。

### 2.6 测试要点(3.2)

- Rust 单测(tools.rs 纯函数,核心可测性设计):
  - `tools_json()`:6 个工具,每个含 name/description/parameters,required 字段正确;
  - `parse_tool_calls`:标准 tool_calls 回复 → 3 个调用解析正确;content 为 null 纯工具回复;空/坏 JSON → None;arguments 含转义引号;
  - `route`:每个工具参数合法/缺必填/类型错误 → Ok/Err 正确;未知工具名 → Err;
  - `rule_match`:打开/查找/非指令文本 → 正确 ToolAction/None;
  - 循环上限:假 client(注入)下 4 轮 → 第 3 轮后终止并带提示(循环逻辑抽成 `run_tool_loop` 接受 `client` trait 对象即可单测);
  - AiToolResult 序列化字段契约;
- 手动验收:对话"打开记事本" → 动作条出现 + 记事本真的打开 + 模型续答;"内存还剩多少" → 状态数据入上下文;连续让 AI 调工具 5 次 → 第 3 轮被终止;`ai_tools_enabled=false` → 工具不再被调用;Ollama 无 tools 模型 → 规则兜底能打开应用。

### 2.7 技术决策(3.2)

| 决策点 | 选择 | 理由 |
|---|---|---|
| 工具循环位置 | **后端循环**(一次 invoke 内完成,前端只收事件) | 前端零循环状态机;单测可用假 client 覆盖;事件契约让前端只渲染 |
| 工具执行同步 vs 异步 | 命令执行同步(在 async 循环内直接 await/调用);仅 HTTP 是异步 | 工具本身都是毫秒级系统调用(open/wallpaper/scan),无需并发;同步顺序执行结果可预期 |
| 分发器形态 | 纯函数 route 返回 ToolAction,执行在命令层 match | 纯函数可单测、不碰系统;执行分支薄;新增工具只动两处 |
| 工具集边界 | 固定 6 个白名单,不开放 shell | 权限边界铁律;最小工具集覆盖开发文档 §6.7 三个示例指令 + 对话增强 |
| 规则兜底 | 仅 open/search 两条关键词规则 | Ollama 无 tools 模型保底;规则最小化避免与模型语义打架 |

---

## 3 自然语言文件搜索(3.3)

### 3.1 范围

**做**

| 功能 | 说明 |
|---|---|
| ai_search_files 命令 | `(query: String, dirs: Option<Vec<String>>) -> Vec<FileHit>`;开发文档 §5 `search_files{query, scope?}` 的实现者(命令名按 ai 前缀避与未来全局搜索冲突,契约见下) |
| 目录集合 | 默认仅桌面(drawer.rs 的 `desktop_dir()`,FOLDERID_Desktop);`ai_search_roots` 配置追加目录;**禁止全盘扫描**(铁律) |
| 匹配 | 文件名大小写不敏感子串;query 含扩展名词时叠加扩展名过滤(pdf/word/excel/图片/视频 映射表,纯函数) |
| 边界 | 递归深度 ≤3、命中上限 20、条目截断、单次扫描兜底保护(巨目录用数量上限先行截断) |
| 双重入口 | ① AI 工具(3.2 的 search_files 工具直接调本命令);② 前端可直接 invoke(开发文档 §5 契约) |

**不做**

- 文件内容全文检索(需索引,Phase4 或独立特性;开发文档 §6.3 明示文件按需搜索不预索引);
- 拼音/模糊匹配(Phase1 已推迟,全局搜索增强时统一做);
- 独立搜索 UI 窗口(结果经聊天呈现;搜索面板 Phase4 随全局搜索增强);
- 索引缓存/常驻 watcher(搜索是瞬态任务,不建索引——内存铁律)。

### 3.2 后端命令(file_search.rs)

文件:`src-tauri/src/commands/file_search.rs`(3.3 独占)

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileHit { pub name: String, pub full_path: String, pub is_dir: bool }
// 对齐开发文档 §5 的 {name, full_path, is_dir}[]

/// 搜索命令(供前端与 AI 工具两用)
#[tauri::command]
pub fn ai_search_files(query: String, dirs: Option<Vec<String>>) -> Vec<FileHit>
// 实现要点:
// 1. 白名单:dirs ⊆ ai_search_roots ∪ {桌面};越权/相对路径目录忽略(不报错,防 AI 越界);
//    dirs=None/空 → 用 ai_search_roots,仍空 → 仅桌面;
// 2. 扩展名映射:query 含 "pdf"→[".pdf"];"word"/"文档"→[".doc",".docx"];"excel"/"表格"→[".xls",".xlsx"];
//    "图片"/"照片"→图片扩展名;"视频"→视频扩展名;else 无过滤 —— 纯函数 ext_hint_from_query(query);
//    同时保留原词子串匹配(如 query="发票" 时按 "发票" 匹配名字,ext 无提示);
// 3. 遍历:每个根目录按深度优先,递归深度 ≤3;每层条目先按名字排序截断 500,再入下一层(巨目录保护);
// 4. 匹配:file_name.to_lowercase().contains(&query_lower)(扩展名过滤叠加时同时满足);
// 5. 结果全局去重 + 按名称排序 + 截断 20;
// 6. 目录读取错误(权限/不存在)跳过该目录,不中断整体;
// 7. 命令为 async fn + spawn_blocking 包裹扫描(防阻塞主线程;扫描目录耗时不可控)。
```

- 复用:not `drawer.rs` 的 watcher/缓存(搜索与抽屉互不相干),只复用其 `desktop_dir()`(pub,只读 import);
- 配置读取:直接 `config::load_from(config_path)` 拿 `ai_search_roots`(与 wallpaper.rs 的 `configured_wallpaper_dir()` 同模式);
- 权限校验函数 `is_allowed_dir(dir, roots)` 抽成纯函数(单测)。

### 3.3 前端

- 无独立组件;AIPanel 中 AI 使用 search_files 工具时,结果以工具返回摘要进入对话(3.2 的 `{ok, action, msg}` 格式),模型负责转述;
- 前端直接调用 `ai_search_files` 的入口本期不做 UI(契约已就绪,Phase4 全局搜索面板接)。

### 3.4 依赖与数据流

```
AI 工具调用 / 前端 invoke ──> ai_search_files(query, dirs?)
    ├─ dirs 白名单校验(⊆ ai_search_roots ∪ 桌面)
    ├─ ext_hint_from_query(query) ──> 扩展名过滤(纯函数)
    └─ 逐根目录 DFS(深度≤3,排序截断,上限保护) ──> Vec<FileHit>(≤20)
```

无新依赖(known-folders 已有)。

### 3.5 测试要点(3.3)

- Rust 单测:
  - `ext_hint_from_query`:pdf/word/excel/图片/视频/无提示词 → 正确映射;大小写不敏感;
  - `is_allowed_dir`:根目录内允许;根目录外拒绝;相对路径拒绝;空 roots 时仅桌面允许;
  - 临时目录树(深度 4 + 多扩展名 + 大小写混合)→ 命中正确、深度 3 截断正确、大小写不敏感命中、扩展名过滤叠加生效、结果 ≤20;
  - 越权 dirs 传入 → 被忽略,不 panic;
  - 巨目录(同层 1000+ 条目)→ 截断不卡死(数量上限先行);
- 手动验收:对 AI 说"查找桌面 pdf 发票"(桌面放 发票.pdf/发票.docx)→ 返回 pdf 命中;配置 ai_search_roots 加 D:\work → 能搜到 D:\work 下文件;传越权目录 → 忽略并正常返回;深目录第 4 层文件搜不到(深度边界)。

### 3.6 技术决策(3.3)

| 决策点 | 选择 | 理由 |
|---|---|---|
| 复用索引 vs 按需扫描 | **按需扫描,不建索引** | 开发文档 §6.3/§8 定死:文件按需搜索、禁止全盘预索引(IO 爆炸 + 内存铁律);桌面+少量配置目录量级下按需扫描毫秒~百毫秒,可接受 |
| 范围约束 | ai_search_roots ∪ 桌面,深度≤3,上限保护 | 铁律落地;深度+数量双上限把最坏耗时钉在可接受区间 |
| 扩展名自然语言映射 | 固定 5 组映射表(纯函数) | 覆盖"查找桌面 pdf 发票"示例;规则表可单测可扩展,不过度设计 NLP |
| 命令归属 | ai_search_files 独立文件、pub 函数 | 3.3 独占文件可最先合入;3.2 工具按契约调用,集成时对齐 |

---

## 4 并行开发指引

### 4.1 分工建议(并行度:3 模块 agent + 1 集成 agent)

| agent | 负责 | 首日产出 |
|---|---|---|
| 集成 agent | 0.3 骨架合并 + 共享文件维护 | 一次 commit:ai_panel 窗口、capabilities、Cargo.toml(reqwest/futures-util)、AppConfig AI 字段(serde(default)+密钥脱敏)、invoke_handler 占位、App.vue 分流、hotkey/托盘入口、Settings AI 区块、stores/config.ts 同步 |
| agent A(3.1) | commands/ai.rs、ai/client.rs、AIPanel.vue、useAiChat.ts | client.rs 双模式请求 + SSE 解析 + 错误分类纯函数 + 单测 |
| agent B(3.2) | ai/tools.rs | tools_json/parse_tool_calls/route/rule_match 纯函数 + 单测(零依赖,首日即可提交) |
| agent C(3.3) | commands/file_search.rs | ext_hint_from_query/is_allowed_dir + 扫描纯函数 + 单测(零依赖,首日即可提交) |

### 4.2 合并与验收顺序

1. 骨架合并(集成 agent)先落 main;
2. 3.2 tools.rs、3.3 file_search.rs 独立纯文件 → 完成后即合;3.1 ai.rs 按其契约开发(签名以本文档 §2.3/§3.2 为准,实现偏差由集成 agent 对齐);
3. 验收顺序建议:3.3 → 3.2 → 3.1(3.3 先行,3.2 的工具调用才有落点;3.1 的流式循环依赖 3.2 的类型,顺序仅为集成便利,可任意,文件不重叠);
4. 集成收尾:全量 `cargo test` + `pnpm build` + 手动验收串跑(§5)+ 空闲内存基线复测(<120MB 目标,开发文档 §8/§9);
5. 每模块完成即在看板对应行更新 ✅(开发进度.md 协作纪律)。

### 4.3 与 Phase1/2 的接口对接清单

- `open_item`/`search_apps`(Phase1)——3.2 工具执行复用(前者无参依赖,后者需 State);
- `config_load`/`config_save`(Phase1)——密钥/模型/搜索目录读写,config.rs 增加脱敏(集成 agent);
- `wallpaper_set_static`/`sys_get_status`/`clipboard_get_history`(Phase2)——3.2 工具执行复用;
- `desktop_dir()`(Phase2 2.2 drawer.rs pub 函数)——3.3 默认目录复用,只读不改;
- 事件——新增 `ai-event`(§0.3),现有事件不变;
- 热键/托盘——hotkey.rs/tray.rs 由集成 agent 追加 AI 入口。

---

## 5 测试要点汇总(对齐开发文档 §9)

| §9 条目 | Phase3 覆盖 |
|---|---|
| 空闲内存占用 | 集成收尾复测基线(<120MB);AI 零常驻线程(OnceLock Client + 瞬态任务),搜索无索引 |
| 长时内存泄漏 | 连续 50 轮对话流式(模拟循环)→ 内存平稳(收尾时跑) |
| **Ollama 离线模式断网不崩溃** | 断网/未启动 → `ai-event{kind:error}` + 错误条,进程不退出(3.1 手动验收必测项) |
| 托盘退出全部线程释放 | ai 无长驻线程,SSE 任务随窗口关闭/退出取消(drop 响应体即断连接) |
| 设置保存重启后生效 | AI 配置(provider/密钥/model/搜索目录)重启后生效(手动验收) |
| 热键重复注册/重启恢复 | ai_hotkey 沿用 Phase1 hotkey 机制(注册失败仅告警) |

---

## 6 技术决策与风险(AD 汇总)

| # | 决策点 | 选择 | 理由 | 风险/备注 |
|---|---|---|---|---|
| AD-1 | 流式 vs 非流式 | **流式(SSE)+ 非流式降级并存**;前端默认流式 | DeepSeek/Ollama 均原生支持 SSE,体验好;解析失败/非 SSE 响应自动降级一次性返回,单测覆盖两条路 | 流式中断丢尾 → 前端保留已收文本 + 错误条 |
| AD-2 | 工具执行同步阻塞 vs 异步 | 工具命令**同步执行**(在 async 循环内直接调用),HTTP 才异步 | 工具均为毫秒级系统调用,顺序执行结果可预期;无需工具并行 | 若未来有慢工具(如全盘搜索)改 async 即可,契约不变 |
| AD-3 | 密钥文件位置与权限 | `%APPDATA%\com.aurora.desktop\config.json` 的 `ai_api_key` 字段;命令层脱敏掩码,前端无明文 | 复用 Phase1 配置机制零新依赖;用户目录 ACL 保护;掩码契约双重保险 | 明文落盘是已知权衡(与剪贴板历史同策略);Phase4 可升级 Credential Manager |
| AD-4 | 工具循环位置 | 后端循环(≤3 轮),前端只消费事件 | 前端零状态机;假 client 注入可单测 | 长对话多轮循环延迟在 1 次 invoke 内,可接受 |
| AD-5 | 搜索复用 vs 新增 | 不建索引,按需扫描 ai_search_roots ∪ 桌面(深度≤3) | 铁律禁止预索引;量级可控 | 巨目录最坏情况由数量截断兜底 |
| AD-6 | 会话状态 | 前端维护,后端无状态 | 可测、无并发同步;40 条截断 | 上下文超长由截断兜底 |
| AD-7 | 双模式实现 | 统一 OpenAI 兼容格式,仅 endpoint/key/model 差异 | 一份解析代码走两端;Ollama 兼容端点成熟 | Ollama 旧版本不支持 tools → 规则兜底(§2.3) |
| AD-8 | Ollama 模型默认值 | `qwen2.5:7b` | 中文场景与 7B 资源占用平衡 | 用户按本机已装模型修改,UI 有提示 |

### 已知风险清单(实现时对照)

1. **prompt injection 越权**:AI 被诱导调用 open_item 打开任意 exe / search_files 传越权目录 → 缓解:工具集固定白名单、dirs 白名单校验(§3.2)、无 shell 工具;文档写明边界(开发文档 §8 权限边界);
2. **reqwest native-tls**:Windows 上 schannel 自动,无需 OpenSSL;若遇 TLS 1.2 证书问题检查系统证书更新(实现时按实测处理);
3. **SSE 解析鲁棒性**:Ollama/DeepSeek 行格式差异(空行/注释/多行 JSON)→ 跳过不中断(错误记录:先 grep 实测再定解析);
4. **流式任务生命周期**:窗口关闭时未完成的流任务需 drop(cancel),否则空转 → ai.rs 在窗口销毁时取消(实现时挂 app 生命周期,收尾验证);
5. **memory**:长对话 + 大文件搜索 → 截断(40 条消息/20 命中/深度 3)三处兜底;
6. **Ollama 模型未装**:配置的 ollama_model 不存在 → Ollama 返回 404 → 错误分类表提示"模型不存在,请安装"。

---

## 7 交付物清单

1. 3.1:commands/ai.rs(三命令)+ ai/client.rs(双模式 SSE 代理)+ AIPanel.vue + useAiChat.ts + ai_panel 窗口/热键/托盘入口;断网/Ollama 未启动/密钥缺失三种降级实测通过;
2. 3.2:ai/tools.rs(6 工具清单 + 解析 + 路由 + 规则兜底)+ ai.rs 工具循环(≤3 轮)+ ai_execute_tool 命令;"打开记事本/设壁纸/查找 pdf 发票"三个示例指令端到端通过;
3. 3.3:commands/file_search.rs(ai_search_files,目录白名单/扩展名映射/深度与上限保护);AI 工具调用与前端 invoke 双入口可用;
4. 密钥安全:config_load 脱敏 + config_save 掩码保留,日志无密钥,单测覆盖;
5. `cargo test` 全绿(新增单测:client 纯函数/SSE 解析/错误分类/tools 解析路由/循环上限/文件搜索纯函数/密钥掩码);
6. 手动验收清单(§1.7/2.6/3.5 + §5 对齐开发文档 §9)全部通过;
7. 空闲内存基线复测记录(对照 <120MB 目标);
8. 开发进度.md 看板 3.1~3.3 全部 ✅,错误记录.md 按纪律持续维护。
