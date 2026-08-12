# Phase5 设计(发布链路 + 多屏壁纸 + AI 工具扩展)

> 状态:2026-08-12 与用户确认范围(方案 A:三块一次交付,分批合入)。
> 热切换(模块开关即时生效)与高对比度/无障碍因风险最高,单独立项 Phase6,不在本文档范围。
> 代码签名因证书未采购,本文档只做接口预留与接入步骤说明,不实现假签名。

## 0 概述与耦合结论

| 块 | 内容 | 依赖 | 风险 |
|---|---|---|---|
| 5.1 发布链路 | NSIS 安装包 + 自动更新 + 签名预留 | 无(纯新增模块 updater) | 中(安装器交互、版本校验) |
| 5.2 多屏壁纸 | 多显示器枚举 + 每屏 WorkerW + 拼接/独立两模式 | 4.1 attach 机制(已稳定) | 中(热插拔、DPI 按屏) |
| 5.3 AI 工具扩展 | 新增 set_dynamic_wallpaper / stop_dynamic_wallpaper 两工具 | 5.2 的多屏 set 契约;4.1 set/clear 契约(已闭环) | 低 |
| 5.4 收口 | 清理全树 6 处历史 dead_code 警告 | 无 | 低 |

**耦合结论**:5.1 与 5.2/5.3 零耦合;5.3 依赖 4.1 现有契约 + 5.2 的多屏 set 扩展(设计上 5.3 只调 5.2 提供的命令,若 5.2 未合入则 5.3 先只调 4.1 原契约,两不阻塞);5.4 独立收尾。三块可并行,契约以本文档为准。

**AppConfig 增量铁律**:所有新增字段必须 `#[serde(default)]` 逐字段回退,老配置缺字段不整体失败(既有契约,见 commands/config.rs)。

## 1 5.1 发布链路

### 1.1 目标与边界

- 目标:非开发者用户可安装;已装用户自动获知并安装新版本;安装/升级/卸载全链路可走。
- 边界:
  - 更新源默认 GitHub Releases(项目已托管),支持配置自定义 URL(自建服务器场景);
  - 不依赖代码签名(证书未采购):自研更新链路用 SHA-256 校验防篡改,签名留接口;
  - 安装方式:NSIS 静默安装(currentUser,无需管理员);升级覆盖安装,配置保留(%APPDATA%\com.aurora.desktop 不受影响);
  - 与现有 MSI 并存(两种安装包各自独立,文档说明二选一,不建议混装)。

### 1.2 NSIS 安装包

- `tauri.conf.json` `bundle.targets` 增加 `"nsis"`,新增 `bundle.nsis` 配置:
  - `languages: ["SimpChinese"]`(tauri 2 NSIS 模板自带简体中文语言文件,实测确认;若模板不支持则回退英文向导+安装后首次启动显示中文说明);
  - `installMode: "currentUser"`(默认,免 UAC);
  - `installerIcon` 复用现有 icon;
  - 输出 `Aurora_<version>_x64-setup.exe`。
- 构建:`pnpm tauri build` 时**同时**产出 MSI 与 NSIS(每次发版两者都给)。WiX/NSIS 下载均需 `HTTPS_PROXY=http://127.0.0.1:7897`(错误记录 4.5 条,勿再踩)。
- 安装行为核对:安装目录默认 `%LOCALAPPDATA%\Aurora`;快捷方式(开始菜单/桌面)由 NSIS 模板默认行为决定,实测后按需配置。

### 1.3 自动更新(自研 updater 模块,零新依赖)

**不引入 tauri-plugin-updater**:该插件要求签名公钥与证书链,证书未采购前不可用;自研模块职责内聚、可单测。

新文件:`src-tauri/src/updater.rs`(纯逻辑 + 少量 FFI)+ `src-tauri/src/commands/updater_cmd.rs`(命令层)。

- **版本元数据**:远端 `latest.json`,字段:
  ```json
  { "version": "0.2.0", "url": "https://github.com/sssst1118/aurora-desktop/releases/download/v0.2.0/Aurora_0.2.0_x64-setup.exe", "sha256": "<hex>", "notes": "更新说明(可选,前端展示)" }
  ```
  默认读取地址:`https://raw.githubusercontent.com/sssst1118/aurora-desktop/main/latest.json`(仓库内维护该文件,发版时更新);可用 AppConfig `update_feed_url` 覆盖为自建静态服务。
- **版本比较**:自实现语义化比较(parse `x.y.z` 逐段比较,忽略预发布后缀;不可解析按 `(0,0,0)` 处理不 panic)。
- **检查时机**:启动后延迟 15s(不抢启动资源,避免与首次索引竞争);此后每 6 小时(固定,不做配置项);托盘菜单提供"检查更新"手动触发。网络失败静默(日志 + 设置页可见错误,不弹窗打断)。
- **下载**:reqwest(已有依赖)流式下载到 `%LOCALAPPDATA%\Aurora\updates\Aurora_setup_<version>.exe`;完成后计算 SHA-256 与 `sha256` 字段比对,不匹配删除并报错;下载中进度经 emit 事件上报前端。
- **安装**:确认后,后端 spawn `cmd /c` 包装脚本(独立进程,与 app 生命周期解耦):
  `start /wait "" "<下载exe>" /S && start "" "<installDir>\Aurora.exe"`,然后 `app.exit(0)`。
  安装器静默完成 → 自动拉起新版本;安装失败(退出码非 0)→ 提示手动安装(打开下载目录)。
  - `/S` 为 NSIS 静默参数(实测确认 tauri NSIS 模板支持);
  - 新版本路径:同版本号变更,安装目录不变,直接取 `%LOCALAPPDATA%\Aurora\Aurora.exe`。

### 1.4 代码签名预留

- 设计上所有安装包(MSI/NSIS)构建产物位置统一为 `src-tauri/target/release/bundle/`,签名工具接入点 = 构建后对产物调用 signtool(文档步骤,不实现代码)。
- `tauri.conf.json` 签名配置键名以本机 tauri 2 配置 schema 实测为准(设计稿不确定,实现时 grep `~/.cargo/registry/src/*/tauri-bundler*/` 确认,勿凭记忆)。
- 文档新增 `docs/代码签名接入.md`(待办):证书采购后三步——① 获得代码签名证书(pfx/证书存储);② 配置签名(signtool 或 tauri 配置);③ 构建后验证签名(右键属性数字签名页/`signtool verify`)。

### 1.5 命令与事件契约

| 命令 | 签名 | 说明 |
|---|---|---|
| `update_check` | `() -> UpdateCheckResult` | 手动检查;`UpdateCheckResult { status: "latest"\|"available"\|"error", version?, notes?, error? }` |
| `update_download` | `() -> bool` | 下载新版(需先 check 到 available);进度经事件;完成 emit `update-downloaded` |
| `update_install` | `() -> bool` | 退出并静默安装;失败返回 false 由前端提示手动安装 |
| `update_open_folder` | `() -> bool` | 打开下载目录(手动安装兜底) |

事件:`update-progress { percent, bytes_done, bytes_total }`、`update-downloaded { version }`、`update-error { message }`。

AppConfig 新增(§5):
- `update_enabled: bool` 默认 true(总开关,false 不检查不提示);
- `update_feed_url: String` 默认 `https://raw.githubusercontent.com/sssst1118/aurora-desktop/main/latest.json`。

前端:设置页新增"更新"区块(当前版本号、检查按钮、状态文案);托盘菜单加"检查更新"项。

## 2 5.2 多屏壁纸

### 2.1 现状与边界

- 现状(4.1,已稳定):单窗口 `wallpaper`,set_size 主屏物理尺寸 → show → 关置顶 → `attach_to_workerw(hwnd, w, h)`(SetParent 到 Progman 子 WorkerW,Win11 实测算法,见错误记录)。
- 边界:
  - 多屏开关默认关 = 现状行为完全不变(回归零风险);
  - 只做"铺满所有屏"与"每屏独立素材"两模式,不做镜像/拉伸差异;
  - 显示器热插拔:2s 轮询检测布局变化 → 自动重建(事件驱动代价高,轮询 2s 足够,与 2.5 采样线程同风格);
  - DPI 按屏处理:每屏窗口 set_size 用该屏**物理像素**尺寸(与 4.1 主屏一致),SetParent 不动;

### 2.2 架构:每屏一个壁纸窗口

- 多屏启用时,运行时创建 n 个壁纸窗口 `wallpaper_0..n-1`(WebviewWindowBuilder,加载现有 wallpaper 页面),每屏一个:
  - 窗口 `wallpaper_0` = 现窗口沿用(保持 URL 契约/前端初始渲染逻辑不变);
  - 新窗口按显示器枚举顺序创建,注入各自显示器的 WorkerW(枚举该屏的 Progman/WorkerW 对,复用 4.1 算法,注意 EnumDisplayMonitors 顺序与 WorkerW 实例的对应需按 monitor rect 匹配)。
- **拼接模式(span)**:一个素材铺满虚拟桌面。每屏窗口加载同一素材,前端按 `?monitor=<i>&span=1` 参数 + 各屏在虚拟桌面中的坐标计算切片(素材坐标系 = 虚拟桌面 rect,窗口显示本屏对应的矩形区域)。图片/视频切片用 CSS object-position + transform 实现;html 素材按坐标裁剪。
- **独立模式**:每屏窗口各挂不同素材;未设置素材的屏显示系统壁纸(不注入)。
- 素材状态:`current_state()` 单值 → 扩展为 `Vec<PerMonitorState>`(每屏 kind/path/url);`get_state` 返回聚合(主屏字段兼容 + 多屏列表)。

### 2.3 命令契约(增量,兼容 4.1)

| 命令 | 签名 | 说明 |
|---|---|---|
| `wallpaper_multi_apply` | `() -> Result<(), String>` | 按当前配置重建各屏 attach(开关/模式/素材变更后调用) |
| `wallpaper_multi_monitors` | `() -> Vec<MonitorInfo>` | 枚举显示器:`{ index, x, y, width, height, primary }`(物理像素,前端设置区展示用) |
| `wallpaper_dynamic_set` | 不变 `(path)` | 拼接模式 → 应用到全部屏;独立模式 → 只设主屏(wallpaper_0,指定屏用 set_monitor);多屏关 → 现状单屏行为 |
| `wallpaper_dynamic_set_monitor` | `(path, index) -> Result<WallpaperSetInfo, String>` | 独立模式:只设指定屏;越界/非独立模式报错 |
| `wallpaper_dynamic_clear` | 不变 | 多屏开 → 撤下全部屏注入;关 → 现状 |

`MonitorInfo`/`PerMonitorState` 数据结构随实现定义在 wallpaper_dynamic 模块(实现层),命令层照 4.1 模式转发。

AppConfig 新增(§5):
- `wallpaper_multi_monitor: bool` 默认 false(= 现状,只铺主屏);
- `wallpaper_span_mode: bool` 默认 true(拼接;false = 每屏独立素材)。

前端:设置区壁纸区块新增"多显示器"小节(启用开关、模式单选、独立模式下的每屏素材选择列表、显示器信息只读展示)。

## 3 5.3 AI 工具扩展

### 3.1 新工具契约(ALL_TOOLS 白名单 +2,OpenAI 格式)

| 工具名 | 描述 | 参数 |
|---|---|---|
| `set_dynamic_wallpaper` | 把图片/视频/网页设为动态壁纸(绝对路径;图片走系统壁纸,视频/html 走 WorkerW) | `{ "file_path": "绝对路径", "url": "可选,http(s) 网页素材" }`(二者至少一个) |
| `stop_dynamic_wallpaper` | 恢复系统壁纸(撤下动态壁纸注入) | `{}` |

- `ToolAction` 新增变体:`SetDynamicWallpaper { path: String, url: Option<String> }`、`StopDynamicWallpaper`;
- `route()` 新增分支(参数必填校验:file_path/url 至少一个;绝对路径校验同现有工具);
- `exec_tool_action`(commands/ai.rs)新增 match 分支:
  - SetDynamicWallpaper → 调 `wallpaper_dynamic_set` 命令逻辑(图片/视频/html 分派复用;开关关 → 透传"动态壁纸未启用"错误,失败隔离 ok:false 回填);
  - StopDynamicWallpaper → 调 `wallpaper_dynamic_clear` 逻辑。

### 3.2 rule_match 关键词扩展(模型不支持 tools 时的兜底)

| 意图 | 关键词(长度降序,命中删全部命中词) | 产出 |
|---|---|---|
| 设壁纸 | "设置壁纸" / "换成壁纸" / "设为壁纸" / "换壁纸" / "改成壁纸" / "设壁纸"(保守词表,宁缺勿误匹配,如"做壁纸"易误伤"生成壁纸"故不收) | SetDynamicWallpaper(路径从目录内匹配素材,找不到返回错误文案) |
| 停壁纸 | "停止壁纸" / "关掉壁纸" / "关闭壁纸" / "停壁纸" | StopDynamicWallpaper |

单测:route 必填校验、rule_match 关键词(含组合词子串防回归,3.2 老坑)、工具 JSON schema 断言、exec 门控错误。

## 4 5.4 收口:dead_code 警告清理

- 目标:全树 cargo build 0 警告(4.3 合入时记录"另行立项"的 6 处 Phase1/2/3 历史 dead_code)。
- 做法:逐处定位后按类型处理——已无用代码删除、被接线遗漏的补引用、确需保留的加最小范围 `#[allow]` 并注释理由(不追求"必须删干净",以行为不变为铁律);每处处理跑 cargo test 防回归。

## 5 AppConfig 增量汇总(全部 #[serde(default)] + Default 实现 + 老配置回退单测)

| 字段 | 类型 | 默认 | 归属 |
|---|---|---|---|
| `update_enabled` | bool | true | 5.1 |
| `update_feed_url` | String | `https://raw.githubusercontent.com/sssst1118/aurora-desktop/main/latest.json` | 5.1 |
| `wallpaper_multi_monitor` | bool | false | 5.2 |
| `wallpaper_span_mode` | bool | true | 5.2 |

## 6 测试计划

| 模块 | 单测覆盖(纯逻辑层,零 Win32) | 真机验证(手动验收清单) |
|---|---|---|
| updater | 版本比较(semver 逐段/不可解析回退)、latest.json 解析(缺字段/坏 JSON)、sha256 计算比对、下载路径构造 | 检查(新/旧/网络失败)、下载进度、安装升级全链路、配置保留 |
| 多屏壁纸 | 显示器枚举排序、虚拟桌面坐标切片计算、每屏状态聚合 | 双屏拼接对齐、独立模式逐屏设、热插拔重建、DPI 125% |
| AI 工具 | route/rule_match/schema 断言、exec 门控 | 对话"设这个视频为壁纸"全链路(4.1 契约已闭环) |
| 5.4 | 既有测试全绿(行为不变) | — |

## 7 质量门与验收顺序

1. 每块合入前:`cargo test` 全绿(存量 + 新增)+ `pnpm build` 通过;
2. 5.1 完成:release 构建产出 MSI + NSIS 双产物;安装→升级→卸载真机验证;空闲内存复测 <120MB;
3. 5.2 完成:单屏回归(多屏关 = 现状)+ 双屏实测;
4. 5.3 完成:Ollama 本地连测(非流式/流式/工具分片三通道,3.1 老坑);
5. 5.4 完成:0 警告 + 全测试绿;
6. 全部合入后:更新开发进度.md 看板、README 功能清单;错误记录.md 补本次新踩坑。

## 8 遗留(Phase6,不在本次)

- 模块开关热切换(窗口重建/线程启停,4.4 明确标注 Phase5 后做);
- 高对比度模式与无障碍增强(多窗口横向铺开);
- 代码签名(证书采购后,按 §1.4 接入)。
