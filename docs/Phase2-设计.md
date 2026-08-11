# Aurora Phase2 设计(效率增强五模块)

> 状态:已定稿(2026-08-11)
> 规格依据:[docs/开发文档.md](./开发文档.md) §2 技术栈、§5 命令接口、§6.2/6.4/6.5/6.6/6.1 模块需求、§8 风险约束
> 上级状态:[docs/开发进度.md](./开发进度.md)(Phase1 完成后启动 Phase2)
> 前置条件:Phase1 任务 1.1~1.5 全部 ✅(本文档定稿时 Phase1 仍在实现中,不阻塞 Phase2 设计)

---

## 0 总览与并行契约

### 0.1 目标

Phase2 在 Phase1(高级启动器)基础上做四个独立功能模块 + 一个系统服务模块,目标是"每个模块独立可验收、并行可开发":

| # | 模块 | 核心能力 | 验收一句话 |
|---|---|---|---|
| 2.1 | Dock 栏 | 可自定义应用快捷栏,图标拖拽排序、运行中指示、顶部/底部边缘切换 | Dock 上点图标能启动/聚焦应用,重启后条目与顺序保留 |
| 2.2 | FileDrawer | 桌面文件抽屉,逻辑收纳(原位不动),按扩展名自动分类 | 打开抽屉看到桌面文件自动归类,文件仍在桌面 |
| 2.3 | 剪贴板历史 | 后台事件驱动监听,历史可搜索/回贴/持久化 | 复制文本后历史出现,重启后仍在,点击可回贴 |
| 2.4 | 静态壁纸 | 一键切换壁纸 | 壁纸列表点一下,桌面壁纸立即更换 |
| 2.5 | 系统状态 | CPU/内存/网络完整实时采集,供灵动岛与托盘展示 | 灵动岛显示实时网络速率,托盘 tooltip 显示完整状态 |

### 0.2 五模块一览

| # | 后端新文件(src-tauri/src/) | 前端新文件(src/) | 关键新依赖 | 模块间耦合 |
|---|---|---|---|---|
| 2.1 Dock | commands/dock.rs、dock_icon.rs | components/core/Dock.vue | windows-sys(窗口枚举/图标)、png | 复用 open_item、search_apps、config |
| 2.2 FileDrawer | commands/files.rs、classify.rs | components/core/FileDrawer.vue | known-folders、notify | 复用 open_item、config |
| 2.3 剪贴板 | commands/clipboard.rs | components/core/ClipboardPanel.vue、composables/useClipboardHistory.ts、stores/clipboard.ts | tauri-plugin-clipboard | 事件契约(clipboard-updated) |
| 2.4 壁纸 | commands/wallpaper.rs | components/core/WallpaperPanel.vue | windows-sys(WindowsAndMessaging) | 复用 config |
| 2.5 系统状态 | system_sampler.rs、commands/system.rs(扩展) | components/core/SysStatusWidget.vue、Island.vue(改造) | windows-sys(NetworkManagement_IpHelper) | 事件契约(sys-status);服务其他模块 |

**耦合结论:五模块之间无直接函数调用,唯一共享通道是两个全局事件 + 全局配置,其余都是复用 Phase1 已有命令。全部可并行。**

### 0.3 并行开发契约(Phase2 开工必读)

#### 共享文件所有权

| 文件 | 维护者 | 约定 |
|---|---|---|
| src-tauri/src/lib.rs(invoke_handler、setup、插件注册) | **集成 agent 独占** | 模块 agent 不得修改;需要注册命令/插件时在模块文档标注,由集成 agent 统一合入 |
| src-tauri/src/commands/mod.rs | 集成 agent | 同上(mod 声明) |
| src-tauri/src/commands/config.rs(AppConfig) | 集成 agent | 各模块字段以模块前缀命名,见 0.3.3;一次合入 |
| src-tauri/capabilities/default.json | 集成 agent | 按模块分块注释追加权限与窗口 label |
| src-tauri/tauri.conf.json | 集成 agent | 一次性注册三个新窗口(见 0.3.4) |
| src-tauri/Cargo.toml、package.json | 集成 agent | 一次性加齐 Phase2 全部依赖 |
| src-tauri/src/hotkey.rs | 集成 agent | 2.2/2.3 的呼出热键注册统一在此追加 |
| src/App.vue(label 分流) | 集成 agent | 加三个窗口渲染分支 |
| src-tauri/src/tray.rs | 集成 agent | 2.2/2.3 托盘入口、2.5 tooltip 更新统一合入 |
| 其余全部新文件(见 0.2 表) | 各自模块 agent | 独占,零重叠 |

#### 协作流程

1. Phase1 全部 ✅ 后,集成 agent 先做**骨架合并**(一次 commit):新窗口、capabilities、依赖、AppConfig 字段、invoke_handler 占位、App.vue 分流分支、hotkey 与托盘入口,全部到位;
2. 随后 5 个模块 agent 并行开工,各写各的新文件,不碰共享文件;
3. 各模块完成后 merge 到 main;集成 agent 负责最终全量编译 + 回归;
4. 冲突预案:任何 agent 发现共享文件被他人改动,先 pull 再基于最新版提交,禁止 force push。

#### 事件契约(模块间唯一共享通道)

| 事件名 | 发布者 | payload | 消费者 | 语义 |
|---|---|---|---|---|
| `sys-status` | 2.5 采样线程(每 2s) | `SysStatus{cpu,f32,mem_used_mb,mem_total_mb,net_rx_bps,net_tx_bps}` | island、Dock、后续模块 | 全局广播,窗口用 `listen` 订阅,无需轮询 invoke |
| `clipboard-updated` | 2.3 监听线程 | `ClipboardItem`(最新一条) | 剪贴板窗口 | 收到后前端调 `clipboard_get_history` 拉全量刷新 |

两个事件的 payload 结构在本文档 §3/§5 定义,属公共契约,不允许单方面改字段。

#### AppConfig 扩展规则

- Phase1 已含开关:`enable_dock`、`enable_file_drawer`、`enable_clipboard_history`(默认 false);
- Phase2 新增字段**全部带模块前缀**,由集成 agent 一次合入:
  - `dock_items: Vec<DockItem>`、`dock_position: String`("top"/"bottom",默认 "bottom")、`dock_auto_hide: bool`
  - `drawer_hotkey: String`(默认 "ctrl+alt+d")、`drawer_open_on_launch: bool`
  - `clipboard_max_items: u32`(默认 200)、`hotkey_clipboard: String`(默认 "ctrl+alt+v")
  - `wallpaper_dir: Option<String>`(默认空 = `%USERPROFILE%\Pictures`)
- **铁律:AppConfig 结构体整体加 `#[serde(default)]`**,否则 Phase1 已生成的旧 config.json 缺新字段时整个反序列化失败,用户配置会被 `load_from` 回退默认值丢失(Phase1 现有 load_from 对损坏 JSON 直接回退,没有字段级容错)。

#### 窗口注册与权限

- 新窗口统一**静态注册在 tauri.conf.json**(与 island/search 同款配置),不运行时创建,避免动态窗口的 capability 匹配坑:

| label | 尺寸 | 特性 | 默认可见 |
|---|---|---|---|
| dock | 800×64 | 无边框/透明/置顶/不可缩放/skipTaskbar/focus:false | 随 enable_dock |
| drawer | 720×520 | 无边框/透明/置顶/不可缩放/skipTaskbar | 隐藏,热键/托盘呼出 |
| clipboard | 480×560 | 无边框/透明/置顶/不可缩放/skipTaskbar | 隐藏,热键/托盘呼出 |

- 三个 label 一并加入 capabilities/default.json 的 `windows` 数组(否则窗口内前端 invoke 权限受限);
- 多入口沿用 Phase1 单入口模式:新窗口 url 都指向 index.html,前端 App.vue 按 `getCurrentWindow().label` 分流渲染对应组件。

### 0.4 全局铁律在 Phase2 的落地

| 铁律 | Phase2 落地 |
|---|---|
| 普通用户权限 | 本阶段全部 API 均无需管理员:SystemParametersInfoW、GetIfTable2、EnumWindows、SHGetKnownFolderPath、剪贴板 API 均为普通权限可用(已查证) |
| 禁止全盘扫描 | 2.2 只扫用户桌面目录(SHGetKnownFolderPath FOLDERID_Desktop),不扫公共桌面、不递归系统目录;2.4 只扫用户配置的壁纸目录 |
| 后台轮询有节制 | 全应用后台循环清单:2.5 采样 2s 一次(GetIfTable2 + GetSystemTimes,极轻);2.1 运行检测 2s 一次、auto-hide 的 GetCursorPos 200ms 一次;2.2 桌面目录用 notify(ReadDirectoryChangesW 事件驱动);2.3 剪贴板用 AddClipboardFormatListener/WM_CLIPBOARDUPDATE 事件驱动。**2.2/2.3 明确不是轮询** |
| API Key 不进前端 | Phase2 无 AI 相关代码,无影响(Phase3 沿用 Rust 代理) |

### 0.5 与 Phase1 的衔接

- 复用 Phase1 命令:`open_item`(打开应用/文件/文件夹)、`search_apps`(Dock 添加应用的挑选列表)、`config_save`/`config_load`;
- `sys_get_status` 命令签名兼容保留,返回值新增 `net_rx_bps`/`net_tx_bps` 两个字段(前端 TS 类型同步);
- 灵动岛 Phase1 是 2s 前端轮询,Phase2 由 2.5 改为后端 2s 采样 + 事件推送,island 改订阅(改动仅限 Island.vue 内部)。

---

## 1 Dock 栏(2.1)

### 1.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 常驻 Dock 窗口 | label=dock,800×64 无边框透明置顶,默认显示在屏幕底部中央(x=(屏幕宽-800)/2,y=屏高-64);`dock_position` 切换顶部/底部(运行时 `set_position`) |
| 自定义快捷方式 | 右键 Dock → "添加应用"打开内嵌 mini 列表(复用 `search_apps` 索引,模糊匹配按 Phase1 子串即可),选中加入;右键图标 → 移除 |
| 图标显示 | 从应用路径提取图标(见 1.6),提取失败用占位图标 + 名称 |
| 拖拽排序 | 前端原生 HTML5 DnD,松手后调 `dock_set_items` 持久化 |
| 运行中指示 | 后端 2s 一次枚举可见顶层窗口,按 PID 去重,与 Dock 项匹配者显示小圆点指示 |
| 点击行为 | 未运行 → 启动(open_item);已运行 → 聚焦已有窗口(恢复最小化 + 置前台),一个应用多个窗口时聚焦其任一可见窗口 |
| 自动隐藏 | `dock_auto_hide=true` 时:鼠标移出 Dock 区域 1.5s 后隐藏,GetCursorPos 200ms 轻量检测到鼠标到达边缘区域(距对应边缘 8px 内)立即浮现 |
| 配置持久化 | 条目+顺序+位置+自动隐藏开关存 config(见 0.3.3) |

**不做**

- 多显示器边缘选择、窗口平铺/管理(Phase4);
- 图标放大动画、文件夹展开、应用分组、右键富菜单;
- 拼音/模糊匹配增强(Phase1 已推迟,不属于本模块验收面,后续随搜索增强单列);
- 替换 Windows 任务栏(产品定位是增强不是替代)。

### 1.2 后端 Rust 模块与命令

文件:`src-tauri/src/commands/dock.rs`、`src-tauri/src/dock_icon.rs`

| 命令 | 签名 | 实现要点 |
|---|---|---|
| dock_get_items | `() -> Vec<DockItem>` | 从内存缓存读(启动时从 config 加载一次);`DockItem{name:String, path:String}` |
| dock_set_items | `(items: Vec<DockItem>) -> bool` | 写回 config 的 `dock_items` 字段(复用 save_to) |
| dock_launch | `(item: DockItem) -> bool` | 先查运行检测缓存:命中 → 聚焦窗口;未命中 → 复用 `open_item(path)` |
| dock_get_running | `() -> Vec<String>` | 返回运行中且被 Dock 收录的应用路径集合(小圆点渲染用) |
| dock_get_icon | `(path: String) -> Option<String>` | 图标 base64 data URL,内存 HashMap 缓存 + 磁盘缓存 `%APPDATA%\com.aurora.desktop\icons\{hash}.png` |

运行检测(独立纯函数,便于单测):`EnumWindows` 回调中 `IsWindowVisible` 过滤 → `GetWindowThreadProcessId` 拿 PID → `GetWindowTextW` 跳过空标题窗口(内部/系统窗口)→ 按 PID 去重 → `QueryFullProcessImageNameW` 拿 exe 路径。与 Dock 项的匹配:**lnk 需解析指向**(用 COM `IShellLinkW`,Phase1 有意没做,Phase2 引入)得到目标 exe 路径后与进程路径比较;直接 exe 条目直接比较路径(大小写不敏感)。聚焦:`ShowWindow(hwnd, SW_RESTORE)` + `SetForegroundWindow`。

### 1.3 前端组件

- `src/components/core/Dock.vue`:图标行 + 运行指示点 + 右键菜单(添加/移除)+ mini 添加列表(调 search_apps)+ DnD 排序(拖拽时高亮目标位,松手调 dock_set_items);
- 位置切换/自动隐藏开关在 Settings.vue 对应区块(设置页由集成 agent 统一接线);
- 订阅 `sys-status` 事件(可选展示网络速率于 Dock 侧)与运行指示数据(`dock_get_running` 2s 拉一次,与后端采样同步周期即可,不额外开轮询)。

### 1.4 依赖与数据流

```
前端 Dock.vue ──invoke──> dock.rs ──> config.rs(save_to)     持久化
    │                        │──> open_item(复用 Phase1)       启动
    │                        └──> dock_icon.rs ──> 磁盘缓存     图标
    └──listen("sys-status") <── system_sampler.rs(2.5 发布)  可选展示
```

新增依赖:`png = "0.17"`;windows-sys 增加 features:`Win32_UI_WindowsAndMessaging`(EnumWindows/GetWindowTextW/GetWindowThreadProcessId/GetCursorPos/ShowWindow/SetForegroundWindow/SystemParametersInfoW)、`Win32_UI_Shell`(ExtractIconExW、IShellLinkW)、`Win32_System_Com`(COM 初始化)。

### 1.5 测试要点

- Rust 单测:运行检测的过滤纯函数(构造 hwnd/pid/title 三元组:可见+有标题→保留;空标题→剔除;同 PID 多窗口→去重);lnk 目标解析(临时 .lnk 指向假 exe,验证路径提取);dock_items 保存加载往返;图标缓存命中/回退占位;
- 手动验收:添加应用→图标正确显示;拖拽换序→重启顺序保留;启动应用→小圆点出现,再点→窗口聚焦;切换底部/顶部→位置立即变化且重启保留;自动隐藏开→鼠标离开隐藏、移到边缘弹出;dock 开关关闭→窗口销毁不占资源。

### 1.6 技术决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| Dock 窗口方式 | tauri.conf.json 静态注册 + setup 按 enable_dock 显隐 | 与 Phase1 一致;capability 匹配简单可靠 |
| 默认位置 | 底部,顶部为可选项 | 顶部已有 island(800×48 x=0),两者同开顶部会重叠;文档写明:顶部模式与岛同开时由用户自行取舍(自动避让 Phase4 再议) |
| 运行检测周期 | 2s 一次 EnumWindows | 系统调用极轻;与 2.5 采样同频,体验无感知;不引入 SetWinEventHook 复杂度 |
| lnk 解析 | IShellLinkW(COM)解析目标 | 运行匹配需要真实 exe 路径,字符串比较不可靠(如 chrome 多窗口);Phase1 不解析是因为打开 lnk 不需要,两阶段诉求不同 |
| 图标提取 | ExtractIconExW → GetDIBits → RGBA → png 编码 → base64,内存+磁盘双缓存 | HICON 不能跨进程长期持有,必须转像素;缓存避免每次启动重复提取 |
| 自动隐藏探测 | GetCursorPos 200ms 轮询 | 与铁律兼容(极轻系统调用,非磁盘/网络);事件钩子方案(SetWindowsHookEx)复杂度不值得 |

---

## 2 FileDrawer 桌面文件抽屉(2.2)

### 2.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 抽屉窗口 | label=drawer,720×520 无边框透明置顶悬浮;全局热键 `drawer_hotkey`(默认 ctrl+alt+d,注册失败仅告警)+ 托盘菜单入口呼出/隐藏 |
| 逻辑收纳 | 文件原位不动,仅读取展示;无删除/移动/重命名入口(防误操作) |
| 自动分类 | 按扩展名分组:文件夹 / 文档 / 图片 / 视频 / 音频 / 压缩包 / 安装包 / 其他(规则表见 classify.rs,可测试) |
| 实时刷新 | notify watcher 监听桌面目录(ReadDirectoryChangesW,事件驱动非轮询),变化 200ms 防抖后重扫;另提供手动刷新按钮兜底 |
| 打开 | 点击文件/文件夹 → 复用 `open_item` |
| 上限保护 | 桌面文件 >1500 时只展示前 1500(按名称排序)+ 总数提示,防渲染卡顿 |
| 设置开关 | `enable_file_drawer` 关闭时不启动 watcher、不注册热键 |

**不做**

- 物理收纳(移动文件到归档目录)——开发文档 §6.4 明确 Phase2 只做逻辑收纳,物理收纳 Phase4;
- 文件内容搜索/预览(Phase3 的 search_files 范围);
- 每文件图标提取(用类别图标,与 2.1 图标模块解耦)、桌面图标布局管理(Layouter 式,Phase4)。

### 2.2 后端 Rust 模块与命令

文件:`src-tauri/src/commands/files.rs`(命令+watcher)、`src-tauri/src/classify.rs`(纯分类函数,无系统调用,单测友好)

| 命令 | 签名 | 实现要点 |
|---|---|---|
| drawer_list_files | `() -> Vec<DrawerGroup>` | 扫 `SHGetKnownFolderPath(FOLDERID_Desktop)`,逐文件 classify 分组,按名称排序截断 1500;`DrawerGroup{category:String, files:Vec<DrawerFile>}`、`DrawerFile{name:String, path:String, ext:String, is_dir:bool}` |
| drawer_open | `(path: String) -> bool` | 直接转发 open_item(文件/文件夹通吃) |
| drawer_refresh | `() -> Vec<DrawerGroup>` | 手动刷新入口,逻辑同 drawer_list_files |

watcher 生命周期:setup 中 enable_file_drawer=true 时创建 `notify::recommended_watcher` 监听桌面目录,收到事件 → 防抖重扫 → 更新内存缓存 → emit `drawer-updated`(payload:空,信号用途)→ 前端拉取;退出时 drop watcher。

### 2.3 前端组件

- `src/components/core/FileDrawer.vue`:左侧分类 tab(带计数)+ 右侧文件网格/列表;空桌面空态;总文件数提示条;
- 显示时先 invoke `drawer_list_files`,再订阅 `drawer-updated` 刷新;
- 分类图标用前端内置(emoji/字体图标),不走后端。

### 2.4 依赖与数据流

```
桌面目录变化 ──ReadDirectoryChangesW──> notify watcher ──200ms防抖──> files.rs 重扫
      │                                                                  │
      └──────────────────────────────────────────────────────────────────┘ emit "drawer-updated"
FileDrawer.vue ──invoke──> files.rs ──> open_item(复用)                   打开
```

新增依赖:`known-folders = "1.4"`(SHGetKnownFolderPath 安全封装,内部处理 CoTaskMemFree,避免手写泄漏)、`notify = "6"`(版本以 `cargo add` 时最新稳定为准)。windows-sys 无需新增 feature。

### 2.5 测试要点

- Rust 单测:classify 分类器(文档/图片/视频/音频/压缩包/安装包/未知扩展名/大写扩展名/无扩展名/隐藏文件);临时目录构造多类型文件 → 分组数量与归类正确;非桌面目录路径一律拒绝;上限截断逻辑;
- 手动验收:桌面放几个不同扩展名文件 → 抽屉自动归类显示;在桌面新建文件 → 抽屉自动出现(不需手动刷新);点击文件以系统默认方式打开;热键呼出/隐藏;开关关闭后 watcher 不启动(任务管理器无句柄泄漏)。

### 2.6 技术决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 桌面目录获取 | known-folders crate(封装 SHGetKnownFolderPath) | 桌面可被库重定向,不能硬编码 C:\Users\...\Desktop;crate 处理 CoTaskMemFree 防泄漏 |
| 刷新机制 | notify 事件驱动 + 手动刷新兜底 | 符合"不轮询"铁律;仅监听桌面目录一个目录,无全盘扫描 |
| 分类粒度 | 8 类固定规则表 | 满足"文档、图片、安装包等"需求;规则表纯函数化,后续可扩展 |
| 文件上限 1500 | 截断展示+计数提示 | 桌面场景极少超限,防御性截断防 WebView 渲染卡死 |

---

## 3 剪贴板历史(2.3)

### 3.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 后台监听 | 事件驱动监听剪贴板变化(首选 tauri-plugin-clipboard v2 的 monitor,Windows 端基于剪贴板变更通知;备选自研 AddClipboardFormatListener + WM_CLIPBOARDUPDATE 消息窗口,见 3.6);**不是轮询** |
| 记录范围 | 文本(去空白、去重);剪贴板中的**图片文件路径**(复制文件场景);>64KB 的文本不记(文档"不保存超大二进制"落地) |
| 去重 | 与最近一条内容完全相同不记(内容哈希,防 Office/资源管理器多次设置剪贴板刷屏) |
| 持久化 | `%APPDATA%\com.aurora.desktop\history\clipboard.json`,上限 `clipboard_max_items`(默认 200),超限淘汰最旧;启动时加载入内存 |
| 历史窗口 | label=clipboard,480×560;呼出 = `hotkey_clipboard`(默认 ctrl+alt+v,注册冲突仅告警)+ 托盘菜单入口 |
| 搜索 | 前端本地过滤(量小,无需后端索引) |
| 回贴 | 点击条目 → 写回剪贴板(插件 writeText),随后自动隐藏窗口并聚焦原应用(隐藏窗口 + 恢复前台) |
| 清空 | `clipboard_clear_history` 清内存+文件 |
| 设置开关 | `enable_clipboard_history` 关闭时监听线程不启动 |

**不做**

- 剪贴板内嵌位图的快照/预览(复制截图场景,Phase4 配合图片保存);
- 密码/敏感内容自动过滤(依赖 UI Automation 识别密码框,Phase4;本期文档写明隐私边界:历史明文存本地文件,用户可一键清空);
- 云同步、加密、标签、置顶锁定(均 Phase4)。

### 3.2 后端 Rust 模块与命令

文件:`src-tauri/src/commands/clipboard.rs`

| 命令 | 签名 | 实现要点 |
|---|---|---|
| clipboard_get_history | `() -> Vec<ClipboardItem>` | 内存 `Mutex<Vec<ClipboardItem>>`(启动时从 json 加载);`ClipboardItem{tp:"text"/"image", payload:String, ts:u64}` 与开发文档 §5 一致 |
| clipboard_clear_history | `() -> ()` | 清内存 + 删除历史文件 |
| clipboard_copy_back | `(index: usize) -> Result<(),String>` | 取历史第 index 条写回剪贴板(插件 writeText),越界返回错误 |

监听线程:setup 中 enable_clipboard_history=true 时启动;收到剪贴板更新 → 读取内容(仅 text;image 场景读文件列表第一项路径)→ 校验(非空/非超长/去重)→ 入队头部 → 裁剪上限 → 异步落盘 → emit `clipboard-updated`。托盘退出流程负责销毁消息窗口/停止监听,不留后台线程。

### 3.3 前端组件

- `src/composables/useClipboardHistory.ts`:封装 invoke + 订阅 `clipboard-updated`;
- `src/stores/clipboard.ts`:Pinia 存历史数组、搜索关键字、选中项;
- `src/components/core/ClipboardPanel.vue`:搜索框 + 列表(时间 + 内容摘要)+ 点击回贴 + 清空按钮;回贴后调用窗口隐藏。

### 3.4 依赖与数据流

```
剪贴板变化 ──事件驱动──> tauri-plugin-clipboard monitor / 自研消息窗口
        └──> clipboard.rs: 校验→去重→入队→裁剪→落盘
        └──> emit "clipboard-updated" ──> ClipboardPanel.vue 刷新
ClipboardPanel ──invoke──> clipboard_get_history / copy_back / clear
```

新增依赖:`tauri-plugin-clipboard = "2"`(Rust crate,CrossCopy 维护,v2 支持 Tauri2)+ 前端 `tauri-plugin-clipboard-api`;capabilities 增 `clipboard:default`(或按需 allow-*);备选自研路径需 windows-sys `Win32_UI_WindowsAndMessaging`(AddClipboardFormatListener/RemoveClipboardFormatListener)。

### 3.5 测试要点

- Rust 单测:去重逻辑(连续相同文本只记一条;不同文本都记);上限裁剪(200 条满后淘汰最旧);超长/空白过滤(>64KB 拒绝、纯空白拒绝);序列化往返;`copy_back` 越界返回 Err;
- 手动验收:复制"你好"→ 历史出现;连续复制相同文本两次 → 只记一条;复制 250 条 → 只保留最新 200;重启应用 → 历史仍在;点击条目 → 在记事本粘贴成功;清空 → 列表空且文件删除;开关关闭 → 复制内容不入历史。

### 3.6 技术决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 监听实现 | 首选 tauri-plugin-clipboard v2(start_monitor + onClipboardUpdate,Windows 端事件驱动) | 社区成熟(每周数千下载),文本/文件事件按类型分发的 API 恰好匹配需求;避免手写消息窗口 + 消息循环线程 |
| 备选方案 | 自研 `AddClipboardFormatListener(hwnd)` + 消息窗口收 `WM_CLIPBOARDUPDATE(0x031D)` | 插件如遇问题时的退路,Win Vista+ 全版本可用,官方推荐的事件驱动方案;HWND_MESSAGE 消息窗口不需要 UI |
| 去重策略 | 与最近一条内容哈希比较 | 系统把剪贴板视为被反复 set 的共享区(Office/资源管理器多次设置),计数消息不可靠,内容比对最稳(已查证) |
| 持久化格式 | 单 json 文件 + 内存全量 | 200 条文本量级(约几百 KB),json 简单可靠,与 Phase1 配置同一技术路线;不引 SQLite |
| 回贴后行为 | 写回剪贴板 + 隐藏窗口 + 恢复原前台窗口 | 贴近系统 Win+V 交互,减少打断 |

---

## 4 静态壁纸切换(2.4)

### 4.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 一键切换 | 点选壁纸 → `SystemParametersInfoW(SPI_SETDESKWALLPAPER)`,立即生效;`SPIF_UPDATEINIFILE|SPIF_SENDCHANGE` 使设置跨重启保持(系统注册表持久化,不需自己存) |
| 壁纸列表 | 扫描配置的壁纸目录(默认 `%USERPROFILE%\Pictures`)下 jpg/jpeg/png/bmp/webp,按名称排序,最多展示 200 张 |
| 预览 | 前端缩略图走 Tauri asset 协议(`convertFileSrc`),capabilities 配 `core:asset:default` + `security.assetProtocol` scope 限定壁纸目录(见 4.4) |
| 当前壁纸标识 | `SystemParametersInfoW(SPI_GETDESKWALLPAPER)` 读当前壁纸路径,列表高亮 |
| 目录配置 | Settings 壁纸区文本输入(留空 = 默认目录),存 `wallpaper_dir`;目录无效时列表区显示错误提示 |

**不做**

- 动态壁纸(WorkerW 网页/视频、电池降载)——Phase4,开发文档 §6.6 已定;
- 多显示器分别设置、壁纸随机/定时轮播、壁纸裁剪编辑(Phase4);
- 壁纸历史/一键还原(SPI_GETDESKWALLPAPER 可读当前值,本期只做"当前高亮",不做多级回滚)。

### 4.2 后端 Rust 模块与命令

文件:`src-tauri/src/commands/wallpaper.rs`

| 命令 | 签名 | 实现要点 |
|---|---|---|
| wallpaper_set_static | `(file_path: String) -> Result<(),String>` | 校验:绝对路径 + 文件存在 + 扩展名白名单;`SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, utf16_path_ptr, SPIF_UPDATEINIFILE\|SPIF_SENDCHANGE)`;失败返回 `GetLastError` 信息 |
| wallpaper_list_local | `() -> Vec<WallpaperEntry>` | 读 `wallpaper_dir`(空→默认 Pictures),过滤图片扩展名 + 常规文件,按名称排序截 200;`WallpaperEntry{name:String, path:String}` |
| wallpaper_get_current | `() -> Option<String>` | `SPI_GETDESKWALLPAPER` 读当前壁纸路径 |

**实现要点(已查证的关键坑)**:`pvParam` 必须是 **UTF-16 编码 + 末尾 NUL** 的绝对路径指针(`encode_wide().chain(once(0))`),传 Rust `&str`(胖指针)会得到黑色壁纸;路径必须绝对,相对路径静默失败。

### 4.3 前端组件

- `src/components/core/WallpaperPanel.vue`(挂进 Settings.vue 壁纸区块):目录输入 + 网格缩略图(convertFileSrc 生成 asset URL)+ 点击应用 + 当前项高亮 + 失败 toast(后端错误信息)。

### 4.4 依赖与数据流

```
WallpaperPanel ──invoke──> wallpaper.rs ──SystemParametersInfoW──> 系统壁纸
    │        └──list──> 目录扫描(仅配置目录)
    └──asset 协议预览 <── security.assetProtocol.scope = [壁纸目录]
```

新增依赖:无新 crate(纯 windows-sys)。windows-sys 增加 feature:`Win32_UI_WindowsAndMessaging`(SystemParametersInfoW 与其常量,该 feature 与 2.1 复用同一声明)。tauri.conf.json `security.assetProtocol` 启用并限定 scope 到壁纸目录;capabilities 增 `core:asset:default`。

### 4.5 测试要点

- Rust 单测:列表过滤(临时目录造 jpg/png/bmp/webp/txt/隐藏文件 → 只返回图片);相对路径/不存在路径 → set 返回 Err;`wallpaper_dir` 为空回退默认目录逻辑;
- 手动验收:设置页显示壁纸目录缩略图;点击后桌面壁纸立即更换;重启系统后壁纸保持(SPIF_UPDATEINIFILE 持久化);目录无效时提示;开关关闭不显示壁纸区块。

### 4.6 技术决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 设置壁纸 API | SystemParametersInfoW(SPI_SETDESKWALLPAPER) | 微软官方 API,零依赖,普通权限可用;动态壁纸 WorkerW 留给 Phase4 |
| 预览方案 | Tauri asset 协议 + convertFileSrc | WebView 无法直接 file:// 任意路径;asset 协议 scope 限定壁纸目录,不放开全盘访问 |
| 目录选择 UI | 文本输入,不引 dialog 插件 | 少一个依赖与 capability;对话框 Phase4 与设置完善一起做 |
| 不引入壁纸 crate | 手写 ~30 行 FFI | wallpaper/wall-rs crate 维护度参差,windows-sys 直调可控且 Phase1 已用同款模式 |

---

## 5 系统状态完整采集(2.5)

### 5.1 范围

**做**

| 功能 | 说明 |
|---|---|
| 常驻采样线程 | `system_sampler.rs` 后台线程,每 2s 采样一次,结果广播 `sys-status` 事件;采样内容:CPU / 内存 / 网络收发速率 |
| CPU | 复用 Phase1 `cpu_percent`(GetSystemTimes 相邻两次采样差商),2s 间隔天然满足差商需求 |
| 内存 | 复用 GlobalMemoryStatusEx(Phase1 已有) |
| 网络 | `GetIfTable2` 遍历 `MIB_IF_TABLE2`,取各接口 `MIB_IF_ROW2.InOctets/OutOctets` 与上次采样差商 → 各接口 bps → 聚合(排除 loopback 与非 Up 接口)为总 rx_bps/tx_bps;`FreeMibTable` 释放 |
| 灵动岛展示 | Island.vue 订阅 `sys-status`(替换 Phase1 的 2s 前端轮询),新增网络速率显示;抽通用组件 SysStatusWidget.vue(岛与后续模块共用) |
| 托盘 tooltip | 每 2s 更新 tray `set_tooltip`,展示 CPU/内存/网络(走后端,不经前端) |
| 命令兼容 | `sys_get_status` 保留,返回最近一次快照(不再命令内 sleep 采样);返回值新增 `net_rx_bps`/`net_tx_bps` |

**不做**

- GPU 使用率、磁盘读写、温度(Phase4);
- 每进程流量、历史曲线图(Phase4);
- 采样频率配置化(Phase4,本期固定 2s 符合"有节制")。

### 5.2 后端 Rust 模块与命令

文件:`src-tauri/src/system_sampler.rs`(常驻线程)、`src-tauri/src/commands/system.rs`(扩展)

| 命令 | 签名 | 实现要点 |
|---|---|---|
| sys_get_status | `() -> SysStatus` | 返回 `Arc<Mutex<Option<SysStatus>>>` 中最近快照(无快照时返回默认 0 值) |

数据契约(事件 payload 与命令返回共用):

```rust
#[derive(Clone, Serialize)]
struct SysStatus {
    cpu: f32,            // 0.0-100.0
    mem_used_mb: u64,
    mem_total_mb: u64,
    net_rx_bps: u64,     // 新增:聚合接收速率 bytes/s
    net_tx_bps: u64,     // 新增:聚合发送速率 bytes/s
}
```

采样线程实现:setup 中 spawn(`tauri::async_runtime` 或 std thread + sleep 2s);循环:采样 → 更新共享快照 → `app.emit("sys-status", &status)`(失败忽略)→ 更新托盘 tooltip;采样两轮间的网络差商需记录上一轮 InOctets/OutOctets 与时间戳。

网络采样要点(已查证):`GetIfTable2` 需 windows-sys feature `Win32_NetworkManagement_IpHelper`;`MIB_IF_ROW2` 过滤条件 `OperStatus == IfOperStatusUp` 且类型非 `IF_TYPE_SOFTWARE_LOOPBACK`;内核计数器约 200-500ms 刷新一次,2s 采样间隔足够;虚拟网卡(WSL2/Docker)计数可能偏大,Phase2 不做智能过滤,文档标注已知现象。

### 5.3 前端组件

- `src/components/core/SysStatusWidget.vue`:CPU/内存/网络速率的展示单元(网络格式化 KB/s、MB/s),供 Island 与后续模块复用;
- `Island.vue` 改造:去掉 2s 轮询 invoke,改 `listen("sys-status")` 更新(监听对象记得 unlisten,窗口隐藏时暂停 UI 更新);
- TS 类型 `SysStatus` 同步新增两个字段。

### 5.4 依赖与数据流

```
system_sampler.rs(2s) ──GetSystemTimes/GlobalMemoryStatusEx/GetIfTable2──> SysStatus
      ├──> emit "sys-status" ──> Island.vue / SysStatusWidget.vue / Dock(可选)
      └──> tray.set_tooltip(后端直更)
sys_get_status(兼容) <── 最近快照
```

新增依赖:无新 crate。windows-sys 增加 feature:`Win32_NetworkManagement_IpHelper`。

### 5.5 测试要点

- Rust 单测:`net_rate` 差商纯函数(构造两次快照的 InOctets/OutOctets + 时间差 → 速率正确;计数器回绕 saturating_sub 不 panic);接口过滤纯函数(loopback/OperStatus Down/混合列表 → 只聚合有效接口);`cpu_percent` 现有测试保留;
- 手动验收:灵动岛三秒内显示网络速率且变化平滑(下载大文件时明显上升);CPU/内存数值与任务管理器偏差在合理范围;托盘悬浮 tooltip 显示完整状态;灵动岛开关关闭后采样线程停止(不再产生事件);长时间挂机内存不增长。

### 5.6 技术决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 采集方式 | 后端常驻线程 2s 采样 + 事件广播 | 多消费者(岛/托盘/Dock)共享一份数据,杜绝各自轮询 invoke;Phase1 的命令内 sleep 采样方案在常驻采样器下不再需要 |
| 网络 API | GetIfTable2(64 位计数) | 现代替代方案,InOctets/OutOctets 覆盖硬件卸载流量;GetIfTable(32 位)有 ~4Gbps 上限,不用;无需管理员权限 |
| 事件广播替代多窗口共享状态 | Tauri 全局 emit | 窗口进程内共享,`listen` 零成本,避免引入跨窗口 store 同步复杂度 |
| 采样周期固定 2s | 不配置化 | 符合"有节制";与 Phase1 前端轮询同频,行为等价 |

---

## 6 并行开发指引

### 6.1 分工建议(并行度:5 模块 agent + 1 集成 agent)

| agent | 负责 | 首日产出 |
|---|---|---|
| 集成 agent | 0.3 骨架合并 + 共享文件维护 | 一次 commit 完成:3 新窗口、capabilities、Cargo.toml/package.json 依赖、AppConfig 新字段(带 #[serde(default)])、invoke_handler 全量注册、App.vue 分流、hotkey/托盘入口 |
| agent A | 2.1 Dock(commands/dock.rs、dock_icon.rs、Dock.vue) | dock.rs 命令 + 运行检测纯函数 + 单测 |
| agent B | 2.2 FileDrawer(commands/files.rs、classify.rs、FileDrawer.vue) | classify.rs + 单测 |
| agent C | 2.3 剪贴板(commands/clipboard.rs、useClipboardHistory.ts、stores/clipboard.ts、ClipboardPanel.vue) | clipboard.rs + 单测 |
| agent D | 2.4 壁纸(commands/wallpaper.rs、WallpaperPanel.vue) | wallpaper.rs + 单测 |
| agent E | 2.5 系统状态(system_sampler.rs、system.rs 扩展、SysStatusWidget.vue、Island.vue 改造) | net_rate 纯函数 + 单测 |

### 6.2 合并与验收顺序

1. 骨架合并(集成 agent)先落 main,五个 agent 的代码位置从那一刻起互相不可见(文件不重叠,直接同 main 并行);
2. 验收顺序建议:2.5 → 2.1 → 2.3 → 2.4 → 2.2(2.5 先行,其他模块无依赖,顺序仅为集成便利,可任意);
3. 集成收尾:全量 `cargo test` + `pnpm build` + 一次完整手动验收串跑 + 空闲内存基线记录;
4. 每模块完成即在看板对应行更新 ✅(状态图例与协作纪律见开发进度.md)。

### 6.3 与 Phase1 的接口对接清单

- `open_item`(打开应用/文件/文件夹)——2.1/2.2 直接复用;
- `search_apps`——2.1 添加应用的内嵌列表复用;
- `config_save`/`config_load`——全部模块复用,AppConfig 字段扩展见 0.3.3;
- `sys_get_status`——2.5 扩展字段,1.5 现有调用方(island)同步改造;
- 托盘——2.2/2.3 入口 + 2.5 tooltip,统一由集成 agent 改 tray.rs。

---

## 7 交付物

1. 五个模块全部可独立开关、独立验收(开关默认 false,与 Phase1 配置结构兼容);
2. `cargo test` 全绿(新增单测覆盖各模块纯函数与配置往返);
3. 手动验收清单(本文档各模块 1.5/2.5/3.5/4.5/5.5)全部通过;
4. 空闲内存基线记录,对照全局 <120MB 目标;
5. 开发进度.md 看板 2.1~2.5 全部 ✅,错误记录.md 按纪律持续维护。
