# Aurora Phase1 MVP 设计

> 状态:已定稿(2026-08-11,用户全权授权推进)
> 规格依据:[docs/开发文档.md](./开发文档.md)
> 上级状态:[docs/开发进度.md](./开发进度.md)

## 1 背景与目标

Aurora 是 Windows 平台 AI 增强模块化桌面效率中心。Phase1 目标是**最小可用版本(MVP)**:一个能当"高级启动器"跑起来的桌面应用——全局热键呼出搜索、索引并打开开始菜单应用、常驻顶部灵动岛(时间 + CPU/内存)、基础配置持久化、托盘管理。

## 2 范围

### 做(Phase1 验收面)

| 功能 | 说明 |
|---|---|
| 项目脚手架 | Tauri2 + Vue3-TS + Tailwind,双窗口配置 |
| 系统托盘 | 显示全部 / 隐藏全部 / 退出 |
| 全局热键 | Ctrl+Shift+Space 切换搜索窗显隐 |
| 应用搜索 | 启动时索引开始菜单 .lnk;子串匹配;回车打开 |
| 基础设置 | 本地 json 配置保存/加载(模块开关骨架 + 热键显示) |
| 灵动岛雏形 | 常驻顶部 800×48:时间 + CPU/内存;点击唤起搜索 |

### 不做(明确排除,防范围蔓延)

- 文件搜索(search_files)——Phase3 再做,文档 §6.3 已定"不做全盘预索引"
- 拼音/模糊匹配——Phase2 跟 Dock 一起做,Phase1 用子串匹配
- 灵动岛媒体控制/展开动画——Phase2
- AI、剪贴板、壁纸、Dock、抽屉、自动化——各自阶段
- `ai_panel` 窗口——Phase3 再建

## 3 架构与窗口策略

**单进程模型**:Rust 侧一个 setup 注册托盘 + 热键 + 初始化索引;两个 WebView 窗口共享同一后端状态。

| 窗口 | 配置(label) | 关键属性 | 职责 |
|---|---|---|---|
| island | island | 800×48,x=0 y=0,无边框、透明、置顶、不可缩放、focus:false | 常驻显示时间+CPU/内存,点击唤起搜索 |
| search | search | 620×420,无边框、透明、置顶、不可缩放、默认隐藏 | 搜索输入+结果列表 |

**托盘**(Tauri tray-icon 内置功能):显示全部 / 隐藏全部 / 退出。图标先用 Tauri 默认,Phase4 换自定义。

**热键**:tauri-plugin-global-shortcut 注册 `Ctrl+Shift+Space`,按下时切换 search 窗口显隐;island 点击唤起搜索走前端 `emit` → 后端 `listen` 后 show search。

## 4 后端 Rust 模块设计(src-tauri/)

### 4.1 命令层(commands/)

Phase1 实现以下 Tauri command:

| 命令 | 签名 | 实现要点 |
|---|---|---|
| search_apps | (query: String) → Vec<AppEntry> | 内存索引中做大小写不敏感子串匹配,按名称排序取 top 20 |
| open_item | (path: String) → bool | ShellExecuteW 打开,失败返回 false |
| sys_get_status | () → SysStatus | GetSystemTimes 计算 CPU 使用率(两次调用间差商);GlobalMemoryStatusEx 取内存 |
| config_save | (cfg: AppConfig) → bool | JSON 写 `%APPDATA%\com.aurora.desktop\config.json` |
| config_load | () → AppConfig | JSON 读,文件不存在/损坏时返回默认值 |

数据结构(与开发文档 §5 一致):

```rust
struct AppEntry { name: String, path: String }
struct SysStatus { cpu: f32, mem_used_mb: u64, mem_total_mb: u64 }
struct AppConfig {
  hotkey_search: String,        // 默认 "Ctrl+Shift+Space"
  enable_island: bool,          // 默认 true
  enable_dock: bool,            // 默认 false(Phase2 用)
  enable_file_drawer: bool,     // 默认 false(Phase2 用)
  enable_clipboard_history: bool, // 默认 false(Phase2 用)
}
```

### 4.2 索引层(indexer/app_index.rs)

- 扫描目录:用户开始菜单 `%APPDATA%\Microsoft\Windows\Start Menu\Programs` + 公共 `C:\ProgramData\Microsoft\Windows\Start Menu\Programs`(递归,含子目录)
- 只收集 `.lnk` 文件,解析出显示名称(文件名去 .lnk)与完整路径;**不跟随 lnk 指向解析**,ShellExecuteW 直接打开 lnk 即可
- 增量策略:进程内记录两个目录的"最后修改时间",启动时对比,变化才重扫(轻量版,不做磁盘缓存)
- 索引结果:Vec<(name, path)>,排序后存内存

### 4.3 配置层

- 路径:`%APPDATA%\com.aurora.desktop\config.json`
- 读写用 serde_json;加载失败回退默认值;保存成功返回 true

### 4.4 依赖(Cargo.toml 主要项)

tauri 2.x、tauri-plugin-global-shortcut、serde/serde_json、windows(仅 ShellExecuteW 用 windows-sys 或 tauri 的 opener 能力;CPU 用 windows-sys GetSystemTimes/GlobalMemoryStatusEx)。**倾向:能少加 crate 就少加**,CPU 采集用 windows-sys。

## 5 前端 Vue3 设计(src/)

### 5.1 窗口与状态

- 两个窗口各挂独立入口(tauri 支持多 entry);共享 Pinia store(模块开关/配置)
- 跨窗口事件:island 点击 → `emit("open-search")` → 后端监听 → show search 窗口

### 5.2 组件

| 组件 | 职责 | 细节 |
|---|---|---|
| Island.vue | 时间(1s 刷新)+ CPU/内存(2s 轮询 sys_get_status) | 深色毛玻璃;点击整窗唤起搜索;不抢焦点由窗口配置保证 |
| SearchBar.vue | 输入框+结果列表 | 输入防抖 150ms 调 search_apps;↑↓ 选择、回车 open_item;Esc 隐藏;无结果空态 |
| Settings.vue | 设置页骨架 | 显示热键说明 + 模块开关(仅 island 有效,其余禁用态);保存调 config_save |
| ui-kit/GlassPanel.vue | 毛玻璃容器 | Tailwind backdrop-blur + 半透明深色 |
| ui-kit/useDraggable | 无边框窗口拖拽 | data-tauri-drag-region 优先,Tauri 原生支持 |

### 5.3 composables

- useTauriInvoke.ts:统一 invoke 封装 + 错误 toast
- useGlobalHotkey.ts:热键状态绑定(Phase1 后端控制窗口显隐,前端仅同步状态)

### 5.4 样式

- Tailwind + global.css;透明窗口背景 + backdrop-blur;深色主题优先(Phase4 主题系统)

## 6 测试与验收

### 6.1 Rust 单元测试(cargo test)

| 模块 | 用例 |
|---|---|
| indexer | 临时目录构造 .lnk 文件 → 解析数量/名称正确;目录 mtime 变化后重扫生效;非 lnk 文件被忽略 |
| search 匹配 | 大小写不敏感;子串命中;多结果按名称排序;空 query 返回空 |
| config | save→load 往返一致;损坏 JSON 回退默认值;缺文件回退默认值 |
| sys status | cpu/mem 返回合理区间(>0) |

### 6.2 手动验收清单(对应开发文档 §9)

1. 热键 Ctrl+Shift+Space 呼出/隐藏搜索框;重启程序后热键恢复有效
2. 搜索"记事本"回车打开;无匹配显示空态
3. 托盘隐藏全部/显示全部/退出;退出后任务管理器无残留进程
4. 灵动岛悬浮不抢焦点(记事本打字时灵动岛出现不影响输入)
5. 配置修改后重启生效
6. 记录空闲内存基线,与全局 120MB 目标对照(Phase1 只记录基线,不设硬门槛)

## 7 风险与约束

- 普通用户权限运行;不尝试管理员权限操作
- 索引只扫开始菜单目录,禁止全盘扫描
- 灵动岛窗口 focus:false,不得抢焦点
- 最低支持 Win10 21H2,不兼容更低版本
- 后台轮询仅限灵动岛 2s 状态刷新,无其他轮询

## 8 技术决策记录(本设计内)

| 决策 | 选择 | 理由 |
|---|---|---|
| Phase1 窗口数 | 2(island + search),不建 ai_panel | 范围最小化;AI 面板 Phase3 |
| 应用索引增量 | 进程内 mtime 对比,不落盘缓存 | MVP 从简;开始菜单量小(毫秒级扫描) |
| 搜索匹配 | 子串匹配,不做拼音 | 拼音匹配有工程量,进 Phase2 |
| lnk 解析 | 不解析指向,直接打开 lnk | ShellExecuteW 原生支持 lnk;少一个解析器依赖 |
| CPU 采集 | windows-sys 直接调 GetSystemTimes | 避免多余 crate 依赖 |

## 9 交付物

1. 可运行 dev 版:热键搜索 + 打开程序 + 灵动岛雏形 + 托盘 + 配置持久化
2. cargo test 全部通过
3. 手动验收 6 项清单执行通过
4. 内存基线记录
5. 进度文档 Phase1 任务全部 ✅
