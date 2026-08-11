# Aurora Phase4 设计(高级特性)

> 状态:已定稿(2026-08-11)
> 规格依据:[docs/开发文档.md](./开发文档.md) §2 技术栈、§5 命令接口、§6.6 壁纸模块、§6.8 系统自动化、§7 Phase4 里程碑、§8 风险约束、§9 测试要点、§11 参考项目清单
> 上级状态:[docs/开发进度.md](./开发进度.md)(Phase3 代码完成后启动 Phase4)
> 前置条件:Phase3 任务 3.1~3.3 全部 ✅(代码完成、142/142 测试全绿,手动验收待用户,不阻塞 Phase4 设计)
> 代码事实核对基准:本文档所有"复用命令/结构体/配置字段/windows-sys feature"均已对照 src-tauri 实际代码与 ~/.cargo/registry 的 windows-sys-0.59.0 源码(2026-08-11),不凭空写

---

## 0 总览与并行契约

### 0.1 目标

Phase4 在 Phase1(启动器)+ Phase2(效率五模块)+ Phase3(AI 集成)基础上做五个高级特性,其中 **4.2 键鼠模拟与 4.3 UI-Automation 是开发文档 §6.8 明示的风险最高模块,排在最后**:

| # | 模块 | 核心能力 | 验收一句话 |
|---|---|---|---|
| 4.1 | 动态壁纸(WorkerW)+ 电池降载 | 把 WebView 窗口注入 WorkerW 壁纸层,支持本地 mp4/HTML/静态图;笔记本电池模式自动暂停动态渲染 | 设置里选一个本地 mp4,桌面图标后出现视频壁纸;电池模式下视频自动暂停 |
| 4.2 | 键鼠模拟自动化 | SendInput 封装:鼠标移动/点击/滚轮、键盘按键/组合键/文本输入 | 自动化面板输入坐标点一下,目标应用收到点击;文本输入进记事本 |
| 4.3 | UI-Automation 控件操作 | windows-sys Uia* 客户端 API 封装:找窗口/遍历控件/读文本/点击控件;UWP/管理员边界写死 | 自动化面板列出记事本的按钮/输入框,点击触发动作;管理员窗口返回明确错误 |
| 4.4 | 主题系统 + 模块开关完善 | 深浅色切换(前端 CSS 变量 + Tailwind class 策略)、主题持久化、Settings 补全新模块开关 | 设置切浅色,面板即时变浅;重启保留;动态壁纸/自动化开关在设置可见可关 |
| 4.5 | MSI 打包 | `pnpm tauri build` 产出 MSI 安装程序,元数据/升级行为正确 | 构建出 MSI,双击安装,ARP 显示 Aurora 与公司信息;升级安装覆盖旧版 |

四条铁律贯穿本阶段(开发文档 §8,落地见 0.4):
1. **权限边界**:程序普通用户权限运行;自动化(4.2/4.3)对管理员权限窗口/UAC 提权程序直接返回失败,不硬来;
2. **动态壁纸 GPU 消耗**:提供开关;笔记本检测电池状态自动暂停动态渲染;
3. **内存 <120MB 与轮询有节制**:壁纸窗口仅开启时创建;电池检测 30s 一次轻量轮询;自动化零常驻线程;
4. **禁止全盘扫描**:壁纸只扫配置目录;自动化只枚举窗口/控件,不扫文件系统。

### 0.2 五任务一览

| # | 后端新文件(src-tauri/src/) | 前端新文件(src/) | 关键新依赖 / feature | 模块间耦合 |
|---|---|---|---|---|
| 4.1 | wallpaper_dynamic.rs(WorkerW 注入 + 电池检测)、commands/wallpaper_dynamic.rs | components/core/DynamicWallpaper.vue、composables/useDynamicWallpaper.ts | windows-sys 增 `Win32_System_Power`(GetSystemPowerStatus) | 复用 2.4 wallpaper.rs 的静态壁纸命令;复用 config;新事件 `wallpaper-power` |
| 4.2 | automation/input_sim.rs(SendInput 封装)、commands/automation.rs(sim 命令) | Settings.vue 自动化区块(集成 agent)+ AutomationPanel 测试 UI(可选) | 无新 feature(`Win32_UI_Input_KeyboardAndMouse` 已启用) | 纯独立;4.3 依赖其 `click_at`/`type_text` 契约 |
| 4.3 | automation/ui_automation_wrap.rs(Uia* 封装)、automation/uia_cmd.rs(UIA 命令层) | 同上区块内 UIA 测试 UI | windows-sys 增 `Win32_UI_Accessibility` + `Win32_System_Com` + `Win32_System_Variant` | 依赖 4.2 的 input_sim 坐标点击;**依赖 4.2 先行** |
| 4.4 | 无新后端(theme 存 AppConfig) | styles/global.css(CSS 变量)、theme.ts、Settings.vue 主题区块、组件迁移 | 无(tailwind.config.js 改 darkMode: "class") | 影响全部前端组件样式(迁移按组件归属各自执行,令牌与入口归 4.4) |
| 4.5 | 无(仅 tauri.conf.json bundle 配置 + 可选的 WiX 文件) | 无 | 无(tauri 自带 WiX) | 由集成 agent 收尾时执行,不占独立并行位 |

**耦合结论:4.1 只有「事件契约 + AppConfig 字段」对外;4.2 完全独立可最先合入;4.3 依赖 4.2 的 input_sim 公共函数(签名在本文档 §3.2 定死);4.4 与后端零耦合(纯前端+配置字段);4.5 是集成 agent 收尾动作。全部可并行,契约以本文档为准。**

### 0.3 并行开发契约(Phase4 开工必读)

#### 共享文件所有权

| 文件 | 维护者 | 约定 |
|---|---|---|
| src-tauri/src/lib.rs(invoke_handler、setup) | **集成 agent 独占** | 模块 agent 不得修改;命令注册/接线由集成 agent 合入 |
| src-tauri/src/commands/mod.rs | 集成 agent | 同上(mod 声明:wallpaper_dynamic、automation、automation/mod.rs 声明 input_sim/ui_automation_wrap/uia_cmd) |
| src-tauri/src/commands/config.rs(AppConfig) | 集成 agent | Phase4 字段见 0.3.4(wallpaper_/theme_/automation_ 前缀);沿用 `#[serde(default)]` 铁律,一次合入 |
| src-tauri/capabilities/default.json | 集成 agent | windows 数组追加 "wallpaper";严格 JSON,无注释(错误记录:解析器为严格 JSON) |
| src-tauri/tauri.conf.json | 集成 agent | 注册 wallpaper 窗口(0.3.5)+ assetProtocol scope 扩展(动态壁纸目录)+ **4.5 的 bundle 配置(集成 agent 在收尾阶段执行)** |
| src-tauri/Cargo.toml | 集成 agent | 一次加齐 Phase4 的 windows-sys feature(`Win32_System_Power`/`Win32_UI_Accessibility`/`Win32_System_Com`/`Win32_System_Variant`;`Win32_UI_Input_KeyboardAndMouse` 已启用无需重复) |
| src-tauri/src/commands/wallpaper_dynamic.rs、src/wallpaper_dynamic.rs | **4.1 模块 agent 独占** | WorkerW 注入 + 电池检测全在此 |
| src/components/core/DynamicWallpaper.vue、src/composables/useDynamicWallpaper.ts | 4.1 模块 agent | 独占 |
| src-tauri/src/automation/input_sim.rs、src-tauri/src/commands/automation.rs | **4.2 模块 agent 独占** | 键鼠模拟全在此;`click_at`/`type_text` 为 4.3 公共契约(pub) |
| src-tauri/src/automation/ui_automation_wrap.rs、src-tauri/src/automation/uia_cmd.rs | **4.3 模块 agent 独占** | UIA 封装与命令;automation/mod.rs 由集成 agent 创建声明 |
| src/styles/global.css、src/theme.ts、tailwind.config.js | **4.4 模块 agent 独占** | CSS 变量令牌 + dark 入口;集成 agent 不碰 style.css |
| src/components/core/Settings.vue、src/stores/config.ts | 集成 agent | Phase4 全部设置区块(动态壁纸/自动化/主题)由集成 agent 按各模块章节的"设置区块规格"合入;stores/config.ts 同步 TS 接口 |
| 其余前端组件(主题迁移) | 各自模块的拥有者 | 样式迁移在本组件内完成;核心面板(Island/Dock/Settings 等)的迁移清单见 §4.3,集成 agent 统筹验收 |

#### 协作流程

1. Phase3 全部 ✅(含手动验收,若用户仍未验收 Phase4 也可启动——Phase4 设计与实现不依赖 Phase3 验收结果)后,集成 agent 先做**骨架合并**(一次 commit):wallpaper 窗口、capabilities、Cargo.toml windows-sys features、AppConfig 字段(带 `#[serde(default)]`)、invoke_handler 占位、Settings 四区块占位、stores 同步、automation/mod.rs、tailwind.config.js darkMode;
2. 随后并行开工:**4.2 先合**(纯文件零依赖零新 feature,最先落地),4.1 按其契约开发,4.3 按 4.2 的 input_sim 契约开发(实现偏差由集成 agent 对齐),4.4 纯前端;
3. 4.2/4.3 风险最高,合入后单独手动验收(§2.5/§3.5),验收通过前不与其他模块验收混跑;
4. 集成收尾:全量 `cargo test` + `pnpm build` + 4.5 MSI 构建与安装验证 + 手动验收串跑(§7)+ 空闲内存基线复测;
5. 冲突预案同 Phase3:共享文件被他人改动先 pull 再提交,禁止 force push;验证用 `git worktree add` 隔离(错误记录:共享 checkout 半成品会拖垮整树编译)。

#### 事件契约(模块间唯一共享通道)

Phase4 仅新增一个事件,其余模块均用 invoke 同步调用(4.2/4.3 是瞬态命令,无流式需求):

| 事件名 | 发布者 | payload | 消费者 | 语义 |
|---|---|---|---|---|
| `wallpaper-power` | 4.1 电池检测(仅**状态变化**时 emit,30s 检测一次) | `{on_battery: bool}` | DynamicWallpaper.vue(视频暂停/恢复)、Settings 动态壁纸区块(状态徽标) | 电池模式判定(§1.4)→ 变化即广播;不变化不广播,防轮询风暴 |

payload 结构属公共契约,不允许单方面改字段。现有 `sys-status`/`clipboard-updated`/`drawer-updated`/`ai-event` 全部不变。

#### AppConfig 扩展规则

Phase4 新增字段**全部带模块前缀**(wallpaper_ / theme_ / automation_),由集成 agent 一次合入(config.rs,沿用 `#[serde(default)]` 铁律——AppConfig 整体已声明,新字段自动获得回退,老配置不丢):

```rust
// ---- Phase4 4.1 动态壁纸(设计文档 §1)----
pub enable_dynamic_wallpaper: bool,    // 总开关,默认 false;关闭时不创建壁纸窗口、不启动电池检测
pub wallpaper_dynamic_dir: Option<String>, // 动态壁纸素材目录,默认 None = 与 2.4 wallpaper_dir 相同(仍为空则 %USERPROFILE%\Pictures)
pub wallpaper_scale_mode: String,      // "cover" | "contain" | "stretch",默认 "cover"(视频/图片填充方式)
pub wallpaper_battery_downshift: bool, // 电池降载开关,默认 true(§8 风险铁律,默认开)
pub wallpaper_battery_threshold_pct: u8, // 降载阈值:电池电量低于该百分比即暂停;默认 0 = 只要在用电池就暂停(最省电、语义最明确)
pub wallpaper_battery_check_sec: u32,  // 电池检测周期,默认 30(有节制轮询;与 2.5 采样同风格)
// ---- Phase4 4.2/4.3 自动化(设计文档 §2/§3;风险最高模块,总开关默认关)----
pub enable_automation: bool,           // 自动化总开关,默认 false;关闭时所有 automation_*/uia_* 命令直接返回错误
pub automation_uia_enable: bool,       // 4.3 UIA 控件操作独立开关,默认 false(比键鼠模拟风险更高,独立可关)
pub automation_click_delay_ms: u32,    // 连续点击最小间隔,默认 80(防连点风暴/误操作)
// ---- Phase4 4.4 主题(设计文档 §4)----
pub theme_mode: String,                // "system" | "dark" | "light",默认 "system"(跟随系统)
pub theme_accent: String,              // 强调色 token 名,默认 "blue"(前端 CSS 变量令牌,§4.2)
```

**铁律:AppConfig 结构体整体 `#[serde(default)]` 已存在,集成 agent 合入字段时不得移除;新字段全部有默认值,`theme_mode` 默认 "system" 保证老用户升级后外观不变(仍是深色)。**

#### 窗口注册与权限

- 新窗口 wallpaper 静态注册在 tauri.conf.json(与既有窗口同款,不运行时创建):

| label | 尺寸 | 特性 | 默认可见 |
|---|---|---|---|
| wallpaper | 全屏(启动时按主显示器尺寸 `set_size`,多显示器仅主屏,Phase4 不做多屏) | 无边框/**不透明**(壁纸层无需透明)/**不置顶**(注入 WorkerW 后置顶无意义)/不可缩放/skipTaskbar/focus:false | 不可见;仅 `enable_dynamic_wallpaper=true` 且素材为 html/mp4 时 attach 到 WorkerW(静态图走 2.4 系统壁纸,不占 WorkerW) |

```json
{ "label": "wallpaper", "title": "Aurora-Wallpaper", "width": 1920, "height": 1080,
  "decorations": false, "transparent": false, "alwaysOnTop": false,
  "resizable": false, "visible": false, "skipTaskbar": true, "focus": false }
```

- capabilities/default.json 的 `windows` 数组追加 `"wallpaper"`(否则该窗口内前端 invoke/事件受限;严格 JSON,不写注释);
- **Phase4 不新增任何 capability 权限**:事件订阅(`core:default` 含 event listen)、invoke 均已具备;视频/图片素材走 Rust 后端(asset 协议或 base64,见 §1.5),前端不直接联网读本地文件;
- 多入口沿用单入口模式:wallpaper 的 url 指向 index.html,App.vue 加 `label === "wallpaper"` 分支挂 DynamicWallpaper.vue(集成 agent);
- ⚠️ **壁纸窗口不进 show_all/hide_all**:win_utils.rs 的 show_all/hide_all 只遍历 ["island", "search"],壁纸窗口 attach 到 WorkerW 后由系统桌面统一管理,集成 agent 不要把它加进这两个函数(否则"隐藏全部窗口"会扯掉壁纸层)。

#### 全局铁律在 Phase4 的落地

| 铁律 | Phase4 落地 |
|---|---|
| 权限边界(普通用户,§8) | 4.2 SendInput 注入前台窗口(UIPI:低权限进程不能向 UAC 提权窗口注入输入,检测到失败返回明确错误,不重试风暴);4.3 对 UWP/管理员进程返回"权限受限"错误(§3.4);不尝试任何提权路径 |
| 动态壁纸 GPU 消耗(§8) | `wallpaper_battery_downshift` 默认开;电池模式(ACLineStatus=0)→ 暂停视频渲染(§1.4 事件暂停);另有总开关 `enable_dynamic_wallpaper` 可完全关闭 |
| 内存 <120MB | wallpaper 窗口仅 enable 时创建;视频暂停时 WebView 不渲染帧;电池检测 30s 一次(GetSystemPowerStatus 极轻);自动化零常驻线程;收尾复测基线 |
| 后台轮询有节制 | 新增轮询仅一处:电池检测 30s/次(状态变化才 emit);4.2/4.3 全为瞬态 invoke,无监听线程 |
| 禁止全盘扫描 | 壁纸只扫 `wallpaper_dynamic_dir`(单个配置目录);自动化只枚举窗口/控件树,不碰文件系统 |
| API Key 不进前端 | Phase4 无新密钥;4.4 主题无敏感数据 |

### 0.4 与 Phase1~3 的接口对接清单

- 复用命令(真实签名已核实):
  - `config_load` / `config_save`(Phase1)——全部 Phase4 模块的配置读写(theme/wallpaper/automation 字段);
  - `wallpaper::wallpaper_set_static_cmd(file_path) -> Result<(), String>`(Phase2 2.4,invoke 名 `wallpaper_set_static`)——4.1 的**静态图素材**直接走它(不占 WorkerW);
  - `wallpaper::wallpaper_dir()`(2.4 内部目录解析,若为 pub 可复用为 `wallpaper_dynamic_dir` 缺省值;4.1 agent 只读复用,不改 wallpaper.rs);
  - `open_item(path)`(Phase1)——4.3 的"打开窗口"前置动作用(可选);
- 新增命令:4.1 `wallpaper_dynamic_*` 四命令 + 4.2 `automation_sim_*` 五命令 + 4.3 `uia_*` 五命令(契约见各章);
- 事件:新增 `wallpaper-power`(§0.3),与现有四事件并存;
- 主题:App.vue 根布局与全部组件样式受 4.4 影响(令牌迁移清单 §4.3);
- 热键/托盘:Phase4 无新增热键与托盘入口(动态壁纸/自动化都不需要;设置页入口已存在)。

---

## 1 动态壁纸(WorkerW)+ 电池降载(4.1)

### 1.1 范围

**做**

| 功能 | 说明 |
|---|---|
| WorkerW 壁纸层注入 | 自研:FindWindowW("Progman") → SendMessageTimeoutW(0x052C) → EnumWindows 找 WorkerW → `SetParent(webview_hwnd, workerw)` → SetWindowPos 铺满主屏(实现细节见 1.6 技术决策 AD-1 与 1.3) |
| 素材类型 | ① 静态图片(jpg/png/webp 等)→ 复用 2.4 系统壁纸,不占 WorkerW;② 本地 mp4 → 壁纸 WebView 内 HTML5 `<video autoplay muted loop playsinline>`;③ 本地 html 文件 → WebView 直接渲染(iframe/内嵌) |
| 素材目录 | `wallpaper_dynamic_dir`(默认回退 2.4 wallpaper_dir → %USERPROFILE%\Pictures);列表命令扫描 mp4/webm/avi/mov/jpg/png/webp/bmp/gif/html,按名称排序截 100 |
| 应用/恢复 | 选择素材 → 应用(静态图 → system API;html/mp4 → attach WorkerW);"恢复系统壁纸" → detach + 恢复原壁纸 |
| 电池降载 | 30s 检测 `GetSystemPowerStatus`;电池模式(ACLineStatus=0,阈值 `wallpaper_battery_threshold_pct` 默认 0=仅电池即暂停)→ emit `wallpaper-power{on_battery:true}` → 前端暂停视频/停动画;状态变化才广播;插电恢复 |
| 开关联动 | `enable_dynamic_wallpaper=false`:不创建窗口、不启动电池检测、设置区显示灰色;`wallpaper_battery_downshift=false`:降载逻辑整体关闭(用户自行承担耗电) |

**不做**

- 多显示器分别壁纸、多屏拼接(只铺主屏;多屏 Phase5/后续);
- 视频壁纸的音轨播放(永远 muted,壁纸不允许出声)、GIF 用 <img> 直放(仅 mp4/html 走 WorkerW);
- 壁纸轮播/定时切换、素材在线下载、壁纸编辑器;
- 沿用 tauri-plugin-wallpaper 插件(存在性核实与结论见 AD-1:可自研实现,插件小众且能力等同,不引入);
- 动态壁纸与 AI 联动(如"设这个视频为壁纸"指令——Phase3 工具集固定 6 个,新增工具 Phase5)。

### 1.2 后端命令签名

文件:`src-tauri/src/commands/wallpaper_dynamic.rs`(命令层,4.1 独占)+ `src-tauri/src/wallpaper_dynamic.rs`(WorkerW 注入与电池检测实现,4.1 独占)

| 命令 | 签名 | 实现要点 |
|---|---|---|
| wallpaper_dynamic_list | `() -> Vec<WallpaperEntry>` | 扫 `wallpaper_dynamic_dir`(回退链见 1.1),按扩展名白名单过滤,排序截 100;复用 2.4 的 `WallpaperEntry{name,path}`(同构,可与 2.4 共用类型或独立,不冲突即可) |
| wallpaper_dynamic_set | `(path: String) -> Result<(), String>` | 校验绝对路径+存在+扩展名白名单;按类型分派:图片 → 复用 `wallpaper_set_static_cmd`;mp4/html → 记录素材路径入内存 → 调 `wallpaper_attach()`(1.3)→ 返回;重复 set 同路径为幂等 |
| wallpaper_dynamic_clear | `() -> Result<(), String>` | 调用 `wallpaper_detach()`(恢复原系统壁纸;若当前是系统壁纸则无操作);清内存素材记录 |
| wallpaper_dynamic_get_state | `() -> DynamicWallpaperState` | `{enabled: bool, kind: String("none"/"image"/"video"/"html"), path: Option<String>, on_battery: bool, downshift_active: bool}`;设置区状态徽标用 |

电池检测(内部,非命令):setup 中 `enable_dynamic_wallpaper && wallpaper_battery_downshift` 时 spawn 线程,每 `wallpaper_battery_check_sec`(默认 30)s 调 `GetSystemPowerStatus`;判定 = `ACLineStatus==0 && BatteryLifePercent <= threshold`(threshold=0 时仅看 ACLineStatus);**状态翻转时**才 `emit("wallpaper-power", {on_battery})`;应用退出时线程随进程结束(无句柄/线程泄漏风险,托盘退出验证见 §7)。

### 1.3 WorkerW 注入实现要点(4.1 模块 agent 必读)

经典三连,全部在 `Win32_UI_WindowsAndMessaging`(**已启用,零新 feature**):

```rust
// 1) 拿桌面窗口
let progman = FindWindowW(w!("Progman"), null());
// 2) 触发系统创建 WorkerW(0x052C = WM_SPAWN_WORKERW;第一次发通常让 Progman 直接持有 SHELLDLL_DefView)
SendMessageTimeoutW(progman, 0x052C, 0, 0, SMTO_NORMAL, 1000, &mut result);
// 3) EnumWindows 回调:找类名 == "SHELLDLL_DefView" 的窗口;取其父窗口 GetParent
//    - 若父窗口 != progman → 该父窗口即 WorkerW
//    - 若父窗口 == progman(说明 DefView 直接在 Progman 下)→ 再发一次 0x052C,重枚举
// 4) SetParent(webview_hwnd, workerw);SetWindowPos(webview_hwnd, null(), 0, 0, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW)
```

要点与坑(已对照错误记录):

- **webview 窗口的 HWND 获取**:`app.get_webview_window("wallpaper")?.hwnd()`(tauri 2 返回 `RawWindowHandle`,取 `hwnd.0` 转 `*mut c_void`,hotkey.rs 已有同款写法);
- **SetParent 前**:窗口先 `show()` 再注入,注入后 `set_always_on_top(false)`(防 WorkerW 层被顶穿);
- **尺寸同步**:注入后监听屏幕分辨率变化(可后端 `RegisterPowerSettingNotification` 不做,先简化:每次 `wallpaper_dynamic_set` 时重查 `primary_monitor` 尺寸 `set_size`);壁纸层不随窗口焦点变化;
- **detach 恢复**:`SetParent(webview_hwnd, null())` + `hide()`;原系统壁纸不用动(WorkerW 覆盖层本来就只是"盖在图标后面的一层",撤掉即恢复原壁纸显示)——**不需要动 SystemParametersInfoW**;
- **多屏**:只铺主屏(primary_monitor),副屏留系统壁纸(Phase4 边界);
- 纯函数抽取:`find_workerw(progman) -> Option<HWND>`、`is_battery_mode(&SYSTEM_POWER_STATUS, threshold) -> bool` 可单测(注入本身无法单测,手动验收覆盖);
- 全部 unsafe 调用遵循项目风格(逐调用 `unsafe {}` + 注释,参考 dock_icon.rs)。

### 1.4 电池降载判定与事件

```rust
// 判定纯函数(可单测):
// ACLineStatus: 1 = AC 供电,0 = 电池
// BatteryFlag: 8 = 充电中(拔电源但插着电充也算充电 → 不降载),255 = 未知
// BatteryLifePercent: 0-100,255 = 未知
pub fn is_battery_mode(st: &SYSTEM_POWER_STATUS, threshold_pct: u8) -> bool {
    if st.ACLineStatus == 1 { return false; }          // 接电源
    if st.BatteryFlag == 8 { return false; }            // 充电中
    if threshold_pct == 0 { return true; }              // 默认:用电池即降载
    st.BatteryLifePercent <= threshold_pct              // 阈值模式:电量低于阈值
}
```

- 前端收到 `wallpaper-power{on_battery:true}` → DynamicWallpaper.vue:视频 `video.pause()` + 展示"电池模式,动态壁纸已暂停"遮罩(半透明提示,可点"恢复播放")→ `on_battery:false` → `video.play()` 恢复;
- 纯 mp4 场景 pause 后 WebView 不再解码渲染,GPU 占用归零(验证方式见 §1.6 手动验收);
- html 素材:收到 on_battery → 隐藏元素 + 停 requestAnimationFrame(由素材 html 内 JS 订阅同一事件,模板给出示例)。

### 1.5 素材渲染与数据流

```
DynamicWallpaper.vue ──invoke wallpaper_dynamic_list──> 目录扫描(仅配置目录,≤100)
    └──invoke wallpaper_dynamic_set(path)──> 类型分派
         ├─ 图片 → wallpaper_set_static_cmd(2.4 复用,系统壁纸)
         └─ mp4/html → wallpaper_attach() → WebView 窗口 SetParent 到 WorkerW
WebView 内渲染:mp4 → <video src=assetUrl autoplay muted loop playsinline>
              html → 直接渲染页面内容(素材 html 由用户提供)
    └──listen("wallpaper-power") <── 电池检测线程(30s,状态变化才 emit)
电池检测线程(GetSystemPowerStatus,Win32_System_Power)──> is_battery_mode ──> emit
```

- **mp4 的 URL 方案(已定)**:asset 协议 `convertFileSrc(path)`(Phase2 2.4 同款);`tauri.conf.json` 的 `security.assetProtocol.scope` 追加 `$USERPROFILE/Pictures/**` 与 `wallpaper_dynamic_dir`(若配置了其他目录,scope 需覆盖——集成 agent 骨架合并时把 scope 扩为 `["$USERPROFILE/Pictures/**", "$WALLPAPER_DIR/**"]`?**tauri scope 不支持运行时变量目录**,处理方式:骨架合并时 scope 写 `["$USERPROFILE/Pictures/**", "$HOME/**"]`?过宽。**更稳**:素材目录固定取 `wallpaper_dynamic_dir` 配置的路径,若用户配置了非 Pictures 目录,则视频 URL 改走 **Rust 后端读文件转 data URL**(路径不经前端,`wallpaper_dynamic_set` 返回素材的 data URL,≤50MB 上限,视频壁纸素材普遍 <20MB,可接受;大文件提示用户放 Pictures 下)。设计如此两段式:**Pictures 内走 asset 协议,目录外走后端 data URL**。此决策写入 AD-2 备选。);
- 新增依赖:无新 crate;windows-sys 增 `Win32_System_Power`(GetSystemPowerStatus/SYSTEM_POWER_STATUS,已 grep 源码确认在 `Win32_System_Power` 模块,kernel32.dll;**不要用 PowerGetActiveScheme 判断电源计划**——它在同模块但引用 `Registry::HKEY` 类型,可能连带要求 `Win32_System_Registry`,收益低,放弃)。

### 1.6 测试要点(4.1)

- Rust 单测:
  - `is_battery_mode` 纯函数:AC 供电→false;电池+充电中→false;电池+阈值 0→true;电池+电量≤阈值→true;未知电量(255)→按阈值 0 规则;阈值边界(==threshold 降载);
  - 素材列表过滤:临时目录造 mp4/webm/html/jpg/txt/隐藏文件 → 只返回白名单;目录不存在 → 空列表不 panic;排序与截断 100;
  - `wallpaper_dynamic_dir` 回退链:配置空 → 2.4 wallpaper_dir → %USERPROFILE%\Pictures;
  - set 校验:相对路径/不存在/非白名单扩展名 → Err;
- 手动验收(§9 必测):设置里选 mp4 → 桌面图标后出现视频壁纸、视频循环播放、图标可点击(壁纸在图标**后面**);选图片 → 走系统壁纸;切 html → 页面渲染;电池降载:断开电源(笔记本)或模拟 `ACLineStatus=0`(临时改判定单测/日志)→ 30s 内视频暂停 + 遮罩出现,插电恢复播放;`wallpaper_battery_downshift=false` → 电池下不停;`enable_dynamic_wallpaper=false` → 重启后无壁纸窗口(任务管理器无该进程 WebView,进程内存无增长);托盘退出 → 桌面恢复原壁纸、无残留窗口。

### 1.7 技术决策(4.1)

| 决策点 | 选择 | 理由 |
|---|---|---|
| WorkerW 方案 | **自研**(FindWindowW + 0x052C + EnumWindows + SetParent) | 能力与插件等价、零新 crate、注入逻辑 ~80 行可控可调;`Win32_UI_WindowsAndMessaging` 已启用零新 feature;符合项目 2.4"不引维护度参差 crate,手写 FFI 可控"既定路线 |
| tauri-plugin-wallpaper | **不采用,记录为备选** | 已核实存在:v3.0.0(2026-01-23,MIT,Meslzy),crates.io 总下载 4709(小众);依赖 `windows ^0.61`(大 crate,编译开销显著);API 固定 attach/detach/reset/pin/unpin,无电池降载/尺寸同步能力;若自研遇不可解问题,切换成本低(其 Rust API 仅 `handle.wallpaper().attach(...)` 几行) |
| 动态壁纸渲染方式 | WebView 窗口承载(mp4 用 HTML5 video,html 直接渲染) | 与项目前端栈一致,零新渲染引擎;视频走系统解码(WebView2/Chromium,支持 H.264),无需自写解码;mp4 必须 muted(壁纸不出声) |
| 电池降载判定 | `GetSystemPowerStatus` + 阈值字段,30s 检测 | 轻量(kernel32 调用,无窗口/无 COM);`PowerGetActiveScheme` 需要额外 feature 且语义复杂(判断"节能计划"),放弃;30s 周期符合"有节制";状态变化才 emit,事件零浪费 |
| 素材访问 | Pictures 内走 asset 协议;目录外走后端 data URL | asset scope 是静态配置,不能含运行时变量目录;data URL 路径绕开 scope 限制,≤50MB 上限防内存(与视频壁纸素材量级匹配) |

---

## 2 键鼠模拟自动化(4.2)

> ⚠️ 开发文档 §6.8 风险最高模块(第一阶段),**最后一个开发批次**,与 4.3 一起最后合并、单独验收。

### 2.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 鼠标模拟 | `automation_sim_click(x, y, button)`(左/右/中键点击,按下+抬起一次注入)、`automation_sim_move(x, y)`(绝对坐标移动)、`automation_sim_scroll(delta, x?, y?)`(滚轮,delta 正=向下) |
| 键盘模拟 | `automation_sim_key(key, modifiers[])`(单键/组合键,如 ["ctrl","c"]、"enter"、"F5"、"a");`automation_sim_type(text)`(Unicode 文本输入,非 ASCII 安全) |
| 兼容契约 | `automation_sim_input(text)`(开发文档 §5 契约名,`automation_sim_type` 的别名,统一实现) |
| 总开关 | `enable_automation=false` → 全部命令立即返回 Err("自动化未启用");`automation_click_delay_ms`(默认 80)强制相邻点击最小间隔(防连点风暴) |
| 边界返回 | UIPI 注入失败(目标为 UAC 提权窗口)、SendInput 返回 0 → 返回明确中文错误,不重试 |

**不做**

- 宏录制/回放、脚本化序列、定时自动化;
- 相对移动、拖拽、多键同时按下的完整组合矩阵(仅 modifiers 组合,不做任意 N 键同按);
- 全局键盘钩子(SetWindowsHookEx)、防检测/注入隐身(这是辅助功能不是外挂);
- 4.3 之前的任何控件级操作(那是 4.3 的活,4.2 只做裸坐标/裸按键);
- 管理员权限规避(铁律:普通用户权限,碰不到的窗口直接失败)。

### 2.2 后端命令签名

文件:`src-tauri/src/automation/input_sim.rs`(SendInput 封装,4.2 独占)+ `src-tauri/src/commands/automation.rs`(命令层,4.2 独占)

```rust
// ---- input_sim.rs(公共契约,4.3 复用 click_at/type_text)----
/// 移动鼠标到绝对屏幕坐标(单位:像素)
pub fn move_to(x: i32, y: i32) -> Result<(), String>;
/// 在坐标处点击(默认左键;button: "left"|"right"|"middle")
pub fn click_at(x: i32, y: i32, button: &str) -> Result<(), String>;
/// 滚轮:delta>0 向下滚动(Windows 语义),可选先移动鼠标
pub fn scroll(delta: i32, x: Option<i32>, y: Option<i32>) -> Result<(), String>;
/// 按键:key 见键盘表,modifiers = ["ctrl","shift","alt","win"] 子集(按下顺序注入,全部抬起后收尾)
pub fn press_key(key: &str, modifiers: &[&str]) -> Result<(), String>;
/// Unicode 文本输入(逐字符 SendInput KEYEVENTF_UNICODE,非 ASCII 安全;不依赖剪贴板)
pub fn type_text(text: &str) -> Result<(), String>;
```

```rust
// ---- commands/automation.rs 命令层(全部 #[tauri::command],签名与上面一一对应)----
// 命令统一入口校验 enable_automation(纯函数 can_use_automation(cfg) -> Result<(), String>),开关关闭即 Err
automation_sim_click  {x: i32, y: i32, button: Option<String>} -> Result<(), String>
automation_sim_move   {x: i32, y: i32} -> Result<(), String>
automation_sim_scroll {delta: i32, x: Option<i32>, y: Option<i32>} -> Result<(), String>
automation_sim_key    {key: String, modifiers: Option<Vec<String>>} -> Result<(), String>
automation_sim_type   {text: String} -> Result<(), String>
automation_sim_input  {text: String} -> Result<(), String>   // 开发文档 §5 兼容别名
```

**SendInput 实现要点(已核实 windows-sys 0.59 源码)**:

- `SendInput` / `INPUT` / `MOUSEINPUT` / `KEYBDINPUT` 均在 `Win32_UI_Input_KeyboardAndMouse` 模块,**该 feature 已在 Cargo.toml 启用**(Phase1 热键就用了同模块 keybd_event),零新 feature;
- `INPUT` 是 union:`union { mi: MOUSEINPUT, ki: KEYBDINPUT, hi: HIDINPUT }`——构造时先 `INPUT { r#type: INPUT_MOUSE, ..zeroed() }` 再填对应成员(参考 hotkey.rs 既有 unsafe 风格);
- 鼠标:`MOUSEINPUT{ dx, dy, dwFlags: MOUSEEVENTF_MOVE|MOUSEEVENTF_ABSOLUTE|MOUSEEVENTF_VIRTUALDESK }` 移动;点击 = 按下(MOUSEEVENTF_LEFTDOWN)+ 抬起(LEFTUP)两次注入或一次数组注入(SendInput 支持 INPUT 数组,一次提交多点动作更稳);
- 键盘:`KEYBDINPUT{ wVk: VK_*, wScan: 0, dwFlags: 0/KEYEVENTF_KEYUP }`(按键);文本用 `KEYEVENTF_UNICODE` + wScan=code unit(UTF-16,逐字符两次注入);
- 调用前 `SetLastError(0)` 清错误 → 调用后 `GetLastError()`;返回 0 = 全部被系统拒绝(最常见:目标窗口是 UIPI 隔离的高权限窗口)→ 返回 "输入被系统拒绝(目标窗口可能为管理员权限,UIPI 限制)"(铁律边界文案);
- 坐标:屏幕绝对坐标(与 GetCursorPos 同一坐标系);虚拟桌面关(不做多显示器定位,Phase4 仅主屏);
- 组合键顺序:modifiers 逐个按下 → 主键按下/抬起 → modifiers 逐个抬起(标准注入序,避免粘滞键状态)。

**键盘表(实现时写死,单测覆盖)**:字母/数字直传 VK;特殊键映射:enter→VK_RETURN、tab→VK_TAB、space→VK_SPACE、backspace→VK_BACK、delete→VK_DELETE、esc→VK_ESCAPE、arrow 四向、F1~F12、home/end/pageup/pagedown;未知键 → Err("不支持的按键")。modifiers:ctrl→VK_CONTROL、shift→VK_SHIFT、alt→VK_MENU、win→VK_LWIN。

### 2.3 前端(自动化区块,集成 agent 实现)

Settings.vue 新增"自动化"区块(集成 agent 按此规格实现,4.2/4.3 合入后生效):

- 总开关(enable_automation)+ UIA 子开关(automation_uia_enable,4.3 用)——重启后生效(同现有开关模式);
- 测试区(仅 4.2 能力,不引 4.3):输入框(x,y) + "点击"按钮 → `automation_sim_click`;输入框(text)+ "输入文本"按钮 → `automation_sim_type`;显示后端错误信息(红色小字);
- 警示文案(固定显示):"自动化为高风险模块:普通用户权限下无法操作管理员窗口/UWP 应用;坐标点击依赖前台窗口位置,请确认目标可见"。

### 2.4 依赖与数据流

```
AutomationPanel(设置区) ──invoke automation_sim_click/type/key──> commands/automation.rs
    └─ enable_automation 校验(纯函数) ──> input_sim.rs(SendInput 注入系统输入队列)
    └─ 失败 → 中文错误 → 前端红色提示(应用不崩溃)
4.3 uia_click_control ──> input_sim::click_at(坐标点击复用,公共契约)
```

无新依赖、无新 feature(已核实)。

### 2.5 测试要点(4.2)

- Rust 单测(input_sim 纯函数部分):
  - `can_use_automation`:enable 开/关 → Ok/Err 正确;
  - 键盘映射表:字母/数字/特殊键/未知键 → 正确 VK 或 Err;
  - modifiers 规范化:去重、非法 modifier 拒绝;
  - INPUT 构造:mouse click 的 dwFlags 组合正确、key 的 vk/scan 正确、unicode 文本的 wScan 正确(构造纯函数 `build_inputs(...) -> Vec<INPUT>` 抽出来测,注入本身不测);
  - 点击间隔:连续两次 click 时间差 < automation_click_delay_ms → 拒绝(间隔守卫纯函数);
- 手动验收(风险模块必测):设置开总开关 → 面板"点击 (100,100)" → 鼠标真的点到(用截图/肉眼验证);"输入文本 你好abc" → 焦点在记事本时内容正确(中文 OK);组合键 ctrl+a 全选;UAC 提权窗口(管理员记事本)→ 返回权限错误文案,进程不崩;开关关闭 → 全部命令 Err;重启后开关保留。

### 2.6 技术决策(4.2)

| 决策点 | 选择 | 理由 |
|---|---|---|
| SendInput vs keybd_event/mouse_event | **SendInput**(单一入口,INPUT 数组一次提交) | 现代推荐 API,支持 UNICODE 文本、绝对坐标、一次注入多点动作;keybd_event/mouse_event 已弃用且无文本注入;同 feature 零成本 |
| 文本输入法 | KEYEVENTF_UNICODE 逐字符注入(不碰剪贴板) | 剪贴板法(复制-粘贴)会污染用户剪贴板(与 2.3 剪贴板历史互相干扰),UNICODE 注入对非 ASCII 安全 |
| 权限边界处理 | 注入失败/被拒 → 明确中文错误返回 | 铁律落地:UIPI 是系统级保护,低权限进程无法突破,文档与 UI 都写明;不做任何提权尝试 |
| 命令粒度 | 独立小命令(sim_click/move/scroll/key/type),不做"动作脚本" | 前端与未来 AI 工具都可组合;单命令可测、失败定位准 |

---

## 3 UI-Automation 控件操作(4.3)

> ⚠️ 开发文档 §6.8 第二阶段,**依赖 4.2 的 input_sim(坐标点击)**,与 4.2 同批最后开发。

### 3.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 窗口枚举 | `uia_find_window(title)` 按标题子串匹配顶层可见窗口 → 列表(hwnd/标题/类名/PID);`uia_get_window_info(hwnd)` 单窗口详情 |
| 控件遍历 | `uia_find_controls(hwnd, control_type?, name?)` 从指定窗口根遍历其 UIA 控件树(限深 3、上限 200,防卡死)→ 控件列表(id/名称/类型/坐标) |
| 读文本 | `uia_get_control_text(hwnd, control_id)` 读控件 Name/Value 文本 |
| 控件操作 | `uia_click_control(hwnd, control_id)`(取 BoundingRectangle 中心 → 调 4.2 `input_sim::click_at`);`uia_type_into(hwnd, control_id, text)`(SetFocus/Select → 4.2 `type_text`) |
| 边界写死 | UWP 应用、管理员权限进程、无 UIA 提供者的旧控件 → 返回固定错误文案(见 3.4),文档/UI 明示 |
| 开关 | `enable_automation` + `automation_uia_enable`(UIA 能力总开关,默认 false)双开关都开才可用 |

**不做**

- UIA 事件监听(UiaAddEvent,实时界面监听是下一阶段)、Control Patterns(InvokePattern 等模式接口)、缓存请求精调;
- 跨进程自动化测试框架、录制回放;
- MSAA/LegacyIAccessible 回退通道(旧控件不支持 UIA 时直接报错,不做兼容层);
- 管理员提权运行本程序(UIPI 限制是铁律,不做);
- **UI 自动化深度遍历性能优化**:限深 3/限 200 已是最保守防线,不做懒加载缓存。

### 3.2 后端命令签名

文件:`src-tauri/src/automation/ui_automation_wrap.rs`(Uia* 封装,4.3 独占)+ `src-tauri/src/automation/uia_cmd.rs`(命令层,4.3 独占)

```rust
// ---- ui_automation_wrap.rs 公共类型与函数 ----
#[derive(Clone, Serialize)]
pub struct UiaWindow { pub hwnd: i64, pub title: String, pub class: String, pub pid: u32, pub visible: bool }
#[derive(Clone, Serialize)]
pub struct UiaControl { pub id: String,          // 稳定标识:从根到该控件的遍历路径(如 "0.2.1"),命令层用它定位
                        pub name: String, pub control_type: String, pub bounds: (i32,i32,i32,i32) }
pub fn find_top_windows(title_like: &str) -> Result<Vec<UiaWindow>, String>;   // EnumWindows(普通 API,不依赖 UIA)
pub fn find_controls(hwnd: i64, control_type: &str, name_like: &str) -> Result<Vec<UiaControl>, String>;
pub fn get_control_text(hwnd: i64, id: &str) -> Result<String, String>;
pub fn get_control_bounds(hwnd: i64, id: &str) -> Result<(i32,i32,i32,i32), String>;
```

```rust
// ---- uia_cmd.rs 命令层(全部 #[tauri::command];入口校验 enable_automation && automation_uia_enable)----
uia_find_window      {title: String} -> Result<Vec<UiaWindow>, String>
uia_get_window_info  {hwnd: i64} -> Result<UiaWindow, String>
uia_find_controls    {hwnd: i64, control_type: Option<String>, name: Option<String>} -> Result<Vec<UiaControl>, String>
uia_get_control_text {hwnd: i64, control_id: String} -> Result<String, String>
uia_click_control    {hwnd: i64, control_id: String} -> Result<(), String>   // 内部:get_control_bounds → input_sim::click_at(中心点)
uia_type_into        {hwnd: i64, control_id: String, text: String} -> Result<(), String> // 内部:焦点定位 → input_sim::type_text
```

**Uia* 客户端 API 实现要点(已核实 windows-sys 0.59 源码,本机 grep 结论)**:

- 需要新增 feature:`Win32_UI_Accessibility`(Uia* 全部函数与 UiaFindParams/UiaCacheRequest/UiaCondition/HUIANODE/TreeScope 等结构枚举)+ `Win32_System_Com`(UiaFind/UiaGetRuntimeId/UiaGetUpdatedCache 的 SAFEARRAY 参数被该 feature 门控)+ `Win32_System_Variant`(UiaGetPropertyValue 的 VARIANT 参数被 Com+Variant 双门控)——**三个 feature 集成 agent 一次加齐**(Cargo.toml 已核实当前不含这三个);
- windows-sys **不含 IUIAutomation COM 接口**(只含 Uia* 句柄式函数——这是 UIAutomationCore 的原生客户端 API,无 COM vtable 手写,正合项目规避手写大接口的教训),使用流程:
  1. `UiaGetRootNode(&mut root)` → 根句柄;
  2. 根上 `UiaFind(&root, &mut UiaFindParams{ condition: UiaCondition{ type_: UiaConditionType_Property, property: UIA_NamePropertyId/UIA_ClassNamePropertyId, value: VARIANT }, ..., }, &mut cache_req, &mut data_sa, ...)`(TreeScope 用 subtree)——返回 SAFEARRAY 包的元素句柄数组;**SAFEARRAY 遍历需手写**(windows-sys 只给结构不给封装:按 SafeArrayGetLBound/UBound/Element 读指针,或直接按 `*(ptr as *mut HUIANODE).add(i)` 读——实现时以实际布局为准,单测覆盖空结果);
  3. 每元素 `UiaGetPropertyValue(node, UIA_NamePropertyId/UIA_ControlTypePropertyId/UIA_BoundingRectanglePropertyId, &mut VARIANT)` 读属性(VARIANT 读写也手写:类型 VT_BSTR/VT_R4/VT_I4 分支);
  4. 用完 `UiaDisconnectProvider`?——不需要,客户端句柄用 `UiaRelease`?——**核对**:客户端 API 的元素句柄不需要释放(UIAutomationCore 托管),但 UiaCacheRequest 需要释放(`UiaCacheRequest` 由 UiaGetUpdatedCache 用……实际用 UiaFind 的 prequest 参数传构造好的缓存请求,无需释放)。**实现时以 UIAutomationCore 文档与实测为准,本文档标注为风险点**(见 §8 风险 5);
  5. 坐标换算:拿到的 BoundingRectangle 是屏幕绝对坐标(与 input_sim 坐标系一致,直接可用);
- 控件 id 设计:`find_controls` 按"根→子"遍历时记录路径 "0.2.1";后续 get/click 命令用路径重新定位(每次命令都重新遍历定位——控件树变化小、成本低,不做缓存句柄,避免句柄生命周期管理)。

### 3.3 前端(自动化区块 UIA 部分,集成 agent 实现)

Settings.vue 自动化区块内追加 UIA 测试区(enable_automation && automation_uia_enable 才显示):

- 窗口搜索:输入标题关键字 → `uia_find_window` → 下拉列表(标题+hwnd);
- 控件列表:选中窗口 → "遍历控件" → `uia_find_controls` → 表格(名称/类型/坐标/路径 id);
- 操作按钮:选中控件 → "读取文本"(`uia_get_control_text`)/"点击"(`uia_click_control`)/"输入文本"(`uia_type_into` + 输入框);
- 边界提示(固定文案):"UWP 应用与管理员权限程序的控件无法访问(系统安全限制)";
- 错误显示:后端中文错误红字展示。

### 3.4 边界与错误文案(写死,文档与 UI 一致)

| 场景 | 判定 | 错误文案 |
|---|---|---|
| 目标窗口为 UWP | UWP 窗口由应用容器隔离,UIA 客户端需系统权限 | "UWP 应用不支持第三方控件操作(系统安全限制)" |
| 目标窗口管理员权限 | UIPI 隔离:普通进程 UIA 访问返回 E_ACCESSDENIED | "目标窗口为管理员权限,当前程序无权限访问" |
| 控件无 UIA 提供者 | UiaFind 无结果 / 属性全空 | "该控件不支持 UI 自动化,请换用键鼠模拟或系统原生操作" |
| 找不到窗口/控件 | 无匹配 | "未找到匹配的窗口/控件" |
| 开关未开 | can_use_automation 校验 | "自动化未启用,请在设置中开启" |

### 3.5 测试要点(4.3)

- Rust 单测:
  - `find_top_windows` 过滤纯函数(可见性/标题匹配);
  - 控件树遍历路径编码与解码(id → 路径数组,往返一致);
  - 深度≤3/数量≤200 截断(假树注入遍历函数测);
  - VARIANT 读写辅助函数(纯函数,用构造的 VARIANT 值测 VT_BSTR/VT_I4/VT_R8 分支);
  - 边界文案映射纯函数(UIPI/无提供者/未找到 → 文案);
  - 坐标取中心点计算;
- 手动验收(风险模块必测):记事本窗口 → 遍历控件出现 菜单栏/编辑区/状态栏;点击"编辑"菜单 → 菜单真的弹出(肉眼);读取编辑区文本 → 内容一致;"输入文本"写入成功;管理员记事本(提权启动)→ 返回权限错误文案,进程不崩;UWP 应用(计算器)→ UWP 文案;开关关闭 → 全部 Err;`automation_uia_enable=false` 但总开关开 → UIA 命令 Err、sim 命令可用。

### 3.6 技术决策(4.3)

| 决策点 | 选择 | 理由 |
|---|---|---|
| UIA 封装深度 | windows-sys **Uia* 句柄式原生客户端 API**,不做 IUIAutomation COM 接口 | windows-sys 只暴露 Uia* 函数族(已 grep 源码证实),**无 COM 接口**,天然规避手写大 vtable(错误记录:COM vtable 须先 grep 头文件,IShellLinkW 21 槽教训);所需 3 个 feature 已核实 |
| 备选(windows crate) | 若 Uia* 手写 SAFEARRAY/VARIANT 成本失控,切换 `windows = { version = "0.6x", features = ["Win32_UI_Accessibility", "Win32_System_Com"] }` 获得完整 IUIAutomation | windows crate 有接口类型与包装,代码量小但引入大依赖(编译时间/二进制体积);**切换决策权在集成 agent,前提是 4.3 先按 Uia* 方案实现并记录阻塞点** |
| 控件定位方式 | 遍历路径 id,每次命令重新遍历定位 | 无句柄生命周期管理、无缓存失效问题;控件树量级小(200 上限),成本可忽略 |
| 边界策略 | 直接返回错误文案,不做 MSAA 回退、不提权 | 铁律落地;边界明示(开发文档 §6.8 要求文档写明边界) |
| 依赖 4.2 | 点击/输入复用 input_sim 公共函数 | 同一注入通道保证行为一致(UIPI 限制同样生效);契约 `click_at(x,y,button)`/`type_text(text)` 在 §2.2 定死 |

---

## 4 主题系统 + 模块开关完善(4.4)

### 4.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 深浅色切换 | 前端 CSS 变量令牌 + Tailwind v3 **class 策略**(`darkMode: "class"`);`theme_mode` = "system"/"dark"/"light";system 跟随 `prefers-color-scheme` |
| 主题持久化 | `theme_mode`/`theme_accent` 存 AppConfig(0.3.4);**即时生效 + 重启保留**(与"重启生效"不同——主题必须即时生效,这是 4.4 的明确改进点) |
| 主题入口 | Settings.vue 新增"主题"区块:三态切换(浅色/深色/跟随系统)+ 强调色选择;main.ts 启动时按配置应用 |
| 组件迁移 | 现有组件硬编码深色(bg-black/70、text-white 等,已核实 Settings.vue 全部为硬编码)→ 迁移到语义令牌(§4.2),迁移清单见 §4.3 |
| 模块开关完善 | Settings 补全 Phase4 新模块开关:动态壁纸(含电池降载子开关)、自动化(含 UIA 子开关);现有"Phase2 开放,重启后生效"文案统一规范 |

**不做**

- 主题系统做全套设计令牌体系(Dark/Light 双套 20+ token 起步的完整 design system)——本次只建**最小可用令牌集**(面板背景/文字/边框/强调色/悬浮层),足够覆盖现有组件;
- 自定义主题导入/导出、跟随系统之外的自动切换(日出日落);
- 组件级动画/过渡效果定制;
- 高对比度模式、无障碍增强(Phase5)。

### 4.2 实现方案

**CSS 变量令牌(global.css,4.4 独占)**:

```css
:root {            /* 浅色主题(Light) */
  --aurora-panel: rgba(255,255,255,.72);      /* 面板毛玻璃底色 */
  --aurora-panel-solid: #f8fafc;              /* 不透明面板(AI 面板等) */
  --aurora-text: #0f172a;
  --aurora-text-dim: rgba(15,23,42,.55);
  --aurora-border: rgba(15,23,42,.12);
  --aurora-accent: #3b82f6;                    /* 默认蓝,theme_accent 换 token 名映射 */
  --aurora-field: rgba(15,23,42,.05);          /* 输入框底色 */
}
:root.dark {       /* 深色主题(当前视觉,原样迁入) */
  --aurora-panel: rgba(0,0,0,.70);
  --aurora-panel-solid: #020617;
  --aurora-text: #ffffff;
  --aurora-text-dim: rgba(255,255,255,.30);
  --aurora-border: rgba(255,255,255,.10);
  --aurora-accent: #3b82f6;
  --aurora-field: rgba(255,255,255,.05);
}
```

**入口逻辑(theme.ts,4.4 独占)**:`apply_theme(cfg)` 把 `theme_mode` 映射到 `<html>` 的 `.dark` class + `color-scheme`(system 时用 `matchMedia("(prefers-color-scheme: dark)")` + change 监听);main.ts 启动时 config_load 后调用;Settings 切换时调用 + `config_save`(即时生效)。

**Tailwind 迁移方式(重要,工作量控制)**:不改所有 tailwind 颜色类(那是全局大改),只改**语义化核心类**:

- 面板容器:`bg-black/70` → `bg-[var(--aurora-panel)]`;
- 文字:`text-white` → `text-[var(--aurora-text)]`;`text-white/30` → `text-[var(--aurora-text-dim)]`;
- 边框:`border-white/10` → `border-[var(--aurora-border)]`;
- 强调色:`bg-blue-500/80` → `bg-[var(--aurora-accent)]`(开关/选中态);
- 输入框:`bg-white/5` → `bg-[var(--aurora-field)]`;
- `dark:` 变体**不引入**(现有组件都是"深色即默认",用变量直接替代即可——dark 变量值就是现在的硬编码值,Light 值才是新增的)。**此策略把迁移成本从"每个元素双份样式"降到"机械替换颜色类"**,与 Tailwind class 策略共存(组件内无 dark: 变体,但 html.dark 类驱动 :root.dark 变量,纯 CSS 层面生效)。

**设置区块规格(集成 agent 实现)**:主题区块 = 三态选择(segmented 按钮)+ 强调色 4 色圆点(blue/green/purple/orange);模块开关区块 = 动态壁纸(enable_dynamic_wallpaper)+ 电池降载(wallpaper_battery_downshift,受 enable 控制)+ 自动化(enable_automation)+ UIA(automation_uia_enable,受 enable_automation 控制)。全部即时 save、重启后保留(开关仍重启生效,与现状一致,4.4 不改变开关热切换)。

### 4.3 组件迁移清单(集成 agent 统筹验收)

| 组件 | 迁移内容 | 归属 |
|---|---|---|
| Settings.vue | 容器/文字/边框/开关类 → 变量 | 集成 agent(本就独占) |
| Island.vue / Dock.vue / DrawerPanel.vue / ClipboardPanel.vue / AIPanel.vue / WallpaperPanel.vue / DynamicWallpaper.vue / SearchBar | 同上机械替换 | 各自模块拥有者执行,集成 agent 验收 |
| style.css / theme.ts / main.ts | 令牌定义 + 入口 | 4.4 agent 独占 |
| index.html | 无(html.dark 由 JS 加) | - |

### 4.4 测试要点(4.4)

- 前端:三态切换 → 各面板即时变色(浅色下文字可读);重启 → 上次选择保留;system 模式下切换系统深浅色 → 跟随(主题可测项主要是手动验收);
- 配置往返:theme_mode/theme_accent 存取正确(Rust 单测在 config.rs,集成 agent 合入字段时带);
- 回归:深色视觉与 Phase3 完全一致(变量值=原硬编码值,机械替换后截图对比);
- 模块开关:4.1/4.2/4.3 各开关开/关 → 对应能力禁用(联动 4.1~4.3 手动验收)。

### 4.5 技术决策(4.4)

| 决策点 | 选择 | 理由 |
|---|---|---|
| 主题方案 | CSS 变量令牌 + html.dark 类(Tailwind v3 class 策略) | 组件全是 tailwind 任意值硬编码,变量替换是唯一机械可行路径;`darkMode:"class"` 是 Tailwind v3 标准做法(当前未配置,默认 media,需改 tailwind.config.js) |
| 迁移策略 | 变量替换(不做 dark: 双份变体) | 现有组件深色即默认,Light 才需要新值;双份变体迁移成本 ×2 且易遗漏,机械替换可批量完成 |
| 即时生效 vs 重启生效 | 主题即时生效;模块开关保持重启生效 | 主题不即时生效没有意义;开关即时生效涉及窗口重建/线程启停,风险高,Phase4 不做热切换(4.1~4.3 的开关语义均为启动时初始化,改热切换属 Phase5) |
| 强调色 | 固定 4 色 token 映射 | 不过度设计;存 token 名不存色值(防未来扩展) |

---

## 5 MSI 打包(4.5)

### 5.1 范围

**做**

| 功能 | 说明 |
|---|---|
| MSI 产出 | `pnpm tauri build` 产出 `.msi`(tauri 2 默认 WiX 工具链,targets 已配 "msi") |
| 元数据 | publisher/**companyName**/copyright/版本号/图标,ARP(卸载或更改程序)正确显示 |
| 升级安装 | 新版本(version 递增)直接覆盖升级安装(单条 ARP 记录);同版本重复安装行为验证 |
| 中文环境验证 | 中文用户名/中文路径安装、中文系统下 ARP 显示正常 |
| 安装后验证 | 安装目录结构、exe 启动、托盘/热键正常(与开发版行为一致) |

**不做**

- NSIS exe 安装包(开发文档 §4.1 契约 targets 只有 msi;NSIS Phase5 可加);
- WiX 深度定制(自定义 UI/横幅/许可协议页、每用户/每机器安装策略调优——用 tauri 默认行为);
- 代码签名(证书未就绪,Phase5 采购后做)、自动更新通道。

### 5.2 配置(tauri.conf.json,集成 agent 在 4.5 阶段执行)

当前 bundle 块只有 `{"active": true, "targets": ["msi"], "icon": [...]}`(已核实),需补:

```json
"bundle": {
  "active": true,
  "targets": ["msi"],
  "icon": ["icons/icon.ico"],
  "publisher": "Aurora Desktop",                    // 必填!WiX Manufacturer,缺失会导致构建报错(tauri 2 已知行为,构建时验证)
  "copyright": "Copyright © 2026 sssst1118",
  "category": "Utility",
  "shortDescription": "Aurora - AI 增强模块化桌面效率中心",
  "longDescription": "灵动岛 / Dock / 全局搜索 / 文件抽屉 / 剪贴板历史 / 壁纸引擎 / AI 助手"
}
```

- 版本:沿用 `"version": "0.1.0"`(与 Cargo.toml/package.json 一致,升级时三处同步改);
- 公司名显示:`publisher` → ARP 的"发布者";productName "Aurora" 已定(4.5 不更名,README/记忆均有更名教训);
- **注意:升级安装依赖 version 递增 + WiX MajorUpgrade(tauri 默认开启)**:0.1.0 → 0.1.1/0.2.0 直接安装覆盖;若安装 0.1.0 后改回 0.1.0 再装 → 行为为修复/重装(ARP 仍单条);**同版本不可降级覆盖测试项**(降级安装可能被 WiX 拒——验证记录即可,不修);
- 中文路径:MSI 支持中文安装路径(默认 %ProgramFiles%\Aurora);中文用户名(用户目录含中文)不影响 MSI 本体,但 app config 目录(%APPDATA%\com.aurora.desktop)跟随用户目录,Phase1 已验证读写无碍(4.5 顺手复测);
- 构建命令:`pnpm tauri build`(前端 pnpm build + Rust release + WiX 打包,首次 WiX 下载需要网络,代理注意项同 Phase1);
- 图标:已配 icons/icon.ico(MSI 显示在 ARP/资源管理器,可选用).

### 5.3 测试要点(4.5)

- 构建:release 全量构建成功产出 `target/release/bundle/msi/*.msi`;
- 安装:双击安装 → 快捷方式/开始菜单项出现(tauri 默认);启动应用正常(托盘/热键/窗口);
- ARP:卸载或更改程序显示 "Aurora" + 发布者 "Aurora Desktop" + 版本号 + 图标;
- 升级:装 0.1.0 → 改 version 0.1.1 重构建 → 直接安装覆盖 → ARP 单条记录版本 0.1.1,配置(theme/wallpaper/ai key)保留(%APPDATA% 不动);
- 卸载:卸载后 %ProgramFiles%\Aurora 清空、托盘/热键不再存在(无残留进程);
- 中文环境:中文用户名安装、ARP 中文显示正常、应用内中文不受影响。

### 5.4 技术决策(4.5)

| 决策点 | 选择 | 理由 |
|---|---|---|
| 打包工具 | tauri 内置 WiX(targets: ["msi"]) | 开发文档 §4.1 契约已定 msi;tauri 默认配置零额外脚本;NSIS Phase5 再议 |
| publisher 元数据 | 明确填写(publisher/companyName/copyright) | 缺失 publisher 是 tauri msi 构建的已知报错点;ARP 信息完整性是交付物 |
| 升级策略 | 依赖 WiX MajorUpgrade(version 递增) | 零配置;升级不碰 %APPDATA% 配置,体验正确 |

---

## 6 并行开发指引

### 6.1 分工建议(并行度:4 模块 agent + 1 集成 agent)

| agent | 负责 | 首日产出 |
|---|---|---|
| 集成 agent | 0.3 骨架合并 + 共享文件维护 + **4.5 MSI(收尾阶段)** | 一次 commit:wallpaper 窗口、capabilities、Cargo.toml 三个新 feature、AppConfig 字段(serde(default))、invoke_handler 占位、automation/mod.rs、tailwind darkMode、Settings 四区块占位、stores/config.ts 同步 |
| agent A(4.1) | wallpaper_dynamic.rs、commands/wallpaper_dynamic.rs、DynamicWallpaper.vue、useDynamicWallpaper.ts | is_battery_mode + 素材过滤纯函数 + WorkerW 查找纯函数 + 单测 |
| agent B(4.2) | automation/input_sim.rs、commands/automation.rs | INPUT 构造纯函数 + 键盘映射表 + 间隔守卫 + 单测(零依赖,首日即可提交) |
| agent C(4.3) | automation/ui_automation_wrap.rs、automation/uia_cmd.rs | 窗口枚举过滤 + 路径编解码 + VARIANT 辅助纯函数 + 单测(**依赖 4.2 的 input_sim 契约,可先写纯函数后接**;实现用 worktree 隔离,不动共享文件) |
| agent D(4.4) | global.css 令牌、theme.ts、tailwind.config.js、组件迁移 | 令牌定义 + apply_theme 纯函数 + 入口接线(前端可独立跑 pnpm build 验证) |

### 6.2 合并与验收顺序

1. 骨架合并(集成 agent)先落 main;
2. **4.2 → 4.4 → 4.1 顺序合入**(4.2 零依赖最先;4.4 纯前端随时可合;4.1 涉及新窗口/事件,合入后即可手动验收);4.3 依赖 4.2 的契约,实现可并行、合入需在 4.2 之后;
3. **4.2/4.3 风险最高,最后一批验收**:合并后先跑 §2.5/§3.5 手动验收(注入行为必须真机验证),通过后才算模块完成;
4. 集成收尾:全量 `cargo test` + `pnpm build` + 4.5 MSI 构建与安装/升级/卸载验证(§5.3)+ 手动验收串跑(§7)+ 空闲内存基线复测(<120MB);
5. 每模块完成即在看板对应行更新 ✅。

### 6.3 与 Phase1~3 的接口对接清单

- `config_load`/`config_save`(Phase1)——四模块配置读写(集成 agent 扩展字段);
- `wallpaper_set_static`/`wallpaper_dir()`(Phase2 2.4)——4.1 静态图复用与目录回退,只读不改;
- `open_item`(Phase1)——4.3 可选前置(打开目标应用),不强制;
- 事件——新增 `wallpaper-power`(§0.3);现有 `sys-status`/`clipboard-updated`/`drawer-updated`/`ai-event` 不变;
- 前端——App.vue 加 wallpaper 分支(集成 agent);Settings.vue 四区块(集成 agent);其余组件主题迁移(§4.3 清单);
- 热键/托盘——**无新增**(Phase4 不需要);⚠️ win_utils.rs 的 show_all/hide_all 不加 wallpaper(§0.3.5)。

---

## 7 测试要点汇总(对齐开发文档 §9)

| §9 条目 | Phase4 覆盖 |
|---|---|
| 空闲内存占用 | 集成收尾复测基线(<120MB);壁纸窗口仅 enable 时创建、视频暂停不渲染;电池检测线程 30s 轻量轮询;自动化零常驻线程 |
| 长时内存泄漏 | 视频壁纸连续挂机 30min → 内存平稳(收尾时跑);自动化连发 100 次注入 → 无增长 |
| **电池降载** | 4.1 手动验收必测:电池模式 → 30s 内视频暂停 + 遮罩;插电恢复;降载开关关闭不降载 |
| **托盘退出全部线程释放** | 4.1:退出后桌面恢复原壁纸、无残留 WorkerW 子窗口(EnumWindows 检查无 aurora 窗口残留);电池检测线程随进程结束;自动化无后台线程 |
| 设置保存重启后生效 | 4.4:theme_mode 即时生效+重启保留;4.1/4.2/4.3 开关重启后生效(对应能力禁用/启用) |
| 热键重复注册/重启恢复 | Phase4 无新热键(回归:Phase1~3 热键不受影响) |
| 权限边界回归 | 4.2/4.3 对 UAC 提权窗口全部返回明确错误,进程不崩(§2.5/§3.5 手动验收必测) |

---

## 8 技术决策与风险(AD 汇总)

| # | 决策点 | 选择 | 理由 | 风险/备注 |
|---|---|---|---|---|
| AD-1 | WorkerW 方案 | **自研**(FindWindowW + 0x052C + EnumWindows + SetParent) | 能力与插件等价、零新依赖、~80 行可控;feature 已启用 | 注入失败(罕见系统组合)→ 返回错误并提示重启资源管理器;检测函数纯化可测 |
| AD-2 | tauri-plugin-wallpaper | **不采用,备选记录** | 已核实存在:v3.0.0(2026-01-23),crates.io 总下载 4709,依赖 windows ^0.61;能力与自研重叠且无电池降载 | 自研受阻时切换成本低(插件 API 仅几行,`handle.wallpaper().attach(...)`) |
| AD-3 | 动态壁纸渲染方式 | WebView 窗口承载(HTML5 video / 直接渲染 html) | 零新渲染引擎;系统解码;与前端栈一致 | mp4 须 muted;素材目录外文件走 data URL(≤50MB) |
| AD-4 | 电池降载阈值与周期 | `GetSystemPowerStatus`,默认阈值 0(用电池即暂停),30s 检测 | ACLineStatus 判定最简单明确;30s 符合有节制;状态变化才 emit | 电池插电瞬间 Flag=充电中 → 不降载;阈值模式(>0)为可选增强 |
| AD-5 | SendInput vs keybd_event | **SendInput**(INPUT 数组、UNICODE 文本、绝对坐标) | 现代 API;同 feature 零成本;一次提交多点动作 | 注入被 UIPI 拒绝 → 明确错误返回(铁律) |
| AD-6 | UIA 封装深度 | windows-sys **Uia* 句柄式 API**(3 feature) | 已 grep 源码:windows-sys 无 IUIAutomation COM 接口,Uia* 函数族齐全;规避手写大 vtable | SAFEARRAY/VARIANT 手写解析是主要工作量;备选切换 windows crate(集成 agent 裁决) |
| AD-7 | 主题迁移策略 | CSS 变量令牌 + 机械替换(不做 dark: 双份变体) | 现有组件深色即默认;变量值=原硬编码值,Light 是新增 | 浅色下个别组件对比度需人工抽查(手动验收项) |
| AD-8 | MSI 打包 | tauri 内置 WiX(targets: ["msi"]),补齐元数据 | 开发文档契约已定;零额外脚本;MajorUpgrade 默认支持升级 | publisher 缺失是已知构建报错点;首次 WiX 下载需网络(代理注意) |

### 已知风险清单(实现时对照)

1. **WorkerW 注入的系统兼容性**:Win10/11 桌面窗口结构一致(Progman/WorkerW 模式自 Vista 起稳定),但个别 DPI 缩放/多屏组合下 SetWindowPos 尺寸失真 → 每次 set 时按 primary_monitor 重算,手动验收覆盖 100%/125% 缩放;
2. **电池状态误判**:虚拟机/台式机无电池 → ACLineStatus 恒 1(AC),永不降载(正确);BatteryLifePercent=255(未知)时按阈值 0 规则(仅电池即降载)——单测覆盖;
3. **UIPI 边界误伤**:管理员窗口在 4.2 注入失败是**预期行为**(铁律),错误文案要能区分"被拒"与"其他失败"(GetLastError 辅助);
4. **Uia* SAFEARRAY 布局**:windows-sys 只给结构不给封装,SAFEARRAY 元素读取按 Com 模块布局手写,若与实测不符 → 切换 windows crate(AD-6 备选),集成 agent 裁决;
5. **UIA 客户端句柄生命周期**:UiaGetRootNode 等返回的 HUIANODE 生命周期由 UIAutomationCore 托管,无需手动释放(以实测为准,若需释放则统一收口在 wrap 层,勿散落);
6. **主题迁移遗漏**:机械替换可能漏掉个别硬编码色(如阴影/渐变)→ 手动验收浅色模式全窗口截图检查;
7. **MSI 构建环境**:首次构建需下载 WiX(网络代理);构建机需 VS Build Tools(已具备);publisher 未配 → 构建直接失败(预配置避免)。

---

## 9 交付物清单

1. 4.1:wallpaper_dynamic.rs(WorkerW 注入 + 电池检测)+ commands/wallpaper_dynamic.rs(四命令)+ DynamicWallpaper.vue + useDynamicWallpaper.ts + wallpaper 窗口/事件接线;mp4 壁纸端到端与电池降载实测通过;
2. 4.2:automation/input_sim.rs + commands/automation.rs(六命令,含开发文档 §5 兼容契约 automation_sim_input);真机注入(点击/文本/组合键)实测通过,提权窗口返回明确错误;
3. 4.3:automation/ui_automation_wrap.rs + uia_cmd.rs(六命令);记事本控件遍历/点击/读文本实测通过,UWP/管理员边界文案验证;
4. 4.4:global.css 令牌 + theme.ts + tailwind darkMode class + Settings 主题区块 + 全组件迁移;深浅色切换与持久化实测;
5. 4.5:tauri.conf.json bundle 元数据 + MSI 构建产出 + 安装/升级/卸载/ARP 验证记录;
6. `cargo test` 全绿(新增单测:is_battery_mode/素材过滤/INPUT 构造/键盘映射/窗口过滤/路径编解码/VARIANT 辅助/边界文案/主题配置往返);
7. 手动验收清单(§1.6/2.5/3.5/4.4/5.3 + §7 对齐开发文档 §9)全部通过;
8. 空闲内存基线复测记录(对照 <120MB 目标,视频壁纸启用与停用两态);
9. 开发进度.md 看板 4.1~4.5 全部 ✅,错误记录.md 按纪律持续维护。
