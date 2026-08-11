# Aurora 极光

> **极光之下,桌面宇宙。**
> Windows 平台 AI 增强模块化桌面效率中心——启动器、灵动岛、Dock、全局搜索、文件收纳、剪贴板历史、壁纸引擎、AI 助手,一站集齐。

Aurora 是一个面向 Windows 10/11 的开源桌面效率中心,目标是**替代 / 增强 Windows 原生桌面体验**。顶部常驻的灵动岛如同一道极光带,极光之下是完整的桌面宇宙:所有功能模块独立可开关,按需启用、不臃肿;Rust 后端扛下系统调用与 AI 代理,前端只负责渲染交互。

## 功能全景

> ✅ 已实现 / 🚧 开发中 / 📋 规划中

| 模块 | 状态 | 说明 |
|---|---|---|
| **全局搜索启动器** | 📋 | 全局热键呼出极速搜索:应用优先、文件其次,拼音/模糊匹配,回车直达 |
| **灵动岛 Island** | 📋 | 常驻顶部悬浮面板,时间 / CPU / 内存 / 网络速率 / 媒体控制,置顶不抢焦点 |
| **Dock 栏** | 📋 | 自定义应用快捷栏,图标拖拽排序,运行中程序标记,上下边缘可切换 |
| **桌面文件抽屉** | 📋 | 桌面文件收纳:逻辑分组(文件不动)或物理归档(桌面清空),自动分类 |
| **剪贴板历史** | 📋 | 后台监听剪贴板,历史可搜索、可回贴,本地持久化 |
| **壁纸引擎** | 📋 | 静态壁纸一键切换 + 动态壁纸(网页/视频渲染到壁纸层),笔记本电池自动降载 |
| **系统状态监控** | 📋 | CPU / 内存 / 网络实时采集,灵动岛与系统托盘展示 |
| **AI 助手** | 📋 | 云端 DeepSeek + 本地 Ollama 双模式;自然语言指令直驱系统操作(打开程序、设壁纸、搜文件) |
| **UI 自动化** | 📋 | 键鼠模拟与 Windows UI Automation 控件操作(高阶特性,边界明确) |

## 界面示意

```
┌──────────────────────── 灵动岛(常驻顶部,极光带) ───────────────────┐
│  🔍  ▸ 14:32:08   CPU 12%   内存 5.8G/16G   ↑1.2M/s ↓3.4M/s  ♪     │
└─────────────────────────────────────────────────────────────────────┘

        ┌──────────── 全局搜索(Ctrl+Shift+Space) ────────────┐
        │  🔍  > 记                                                           │
        │  ────────────────────────────────────────────────── │
        │  🖥 记事本                    C:\Windows\notepad.exe │
        │  📄 记账.xlsx                  D:\财务\记账.xlsx    │
        │  ...                                                  │
        └──────────────────────────────────────────────────────┘

       ( Dock 栏 ──────────────── )( 文件抽屉 )( AI 对话面板 )
```

*界面示意为规划稿,最终效果以实际开发为准。*

## 技术栈

| 层级 | 选型 |
|---|---|
| 桌面外壳 | Tauri 2.0(多窗口、透明无边框、托盘、权限白名单) |
| 后端 | Rust 1.80+ / windows-rs(Win32 原生 API)、reqwest(AI 代理) |
| 前端 | Vue3 + TypeScript + Vite、TailwindCSS v3、Pinia |
| 插件 | global-shortcut(全局热键)、clipboard-next(剪贴板)、wallpaper(壁纸层) |
| 包管理 | pnpm / cargo |

## 快速开始

### 环境要求

- Windows 10 21H2 及以上
- Rust MSVC 工具链(含 Visual Studio Build Tools C++ 负载 + Windows SDK)
- Node.js LTS + pnpm

### 开发运行

```bash
pnpm install          # 安装前端依赖
pnpm tauri dev        # 启动开发模式(Rust 编译 + 前端热更新)
```

### 打包

```bash
pnpm tauri build      # 产出 MSI 安装包
```

## 项目结构

```
aurora-desktop/
├── src-tauri/            # Rust 后端
│   ├── commands/         # Tauri 命令:搜索/文件/壁纸/系统/剪贴板/AI/自动化
│   ├── indexer/          # 开始菜单应用索引、文件按需索引
│   └── automation/       # 键鼠模拟 + UI Automation 封装
└── src/                  # Vue3 前端
    ├── components/core/  # Island / Dock / SearchBar / FileDrawer / AIPanel / Settings
    └── stores/           # Pinia:模块开关、配置、剪贴板历史
```

## 路线图

| 阶段 | 内容 | 状态 |
|---|---|---|
| **Phase 1** MVP | 项目脚手架、多窗口 + 托盘、全局热键搜索、开始菜单索引、基础设置、灵动岛雏形——*可当高级启动器使用* | 📋 规划中 |
| **Phase 2** 效率增强 | Dock、文件抽屉、剪贴板历史、静态壁纸、完整系统状态 | 📋 |
| **Phase 3** AI 集成 | AI 对话面板、DeepSeek / Ollama 双模式、function-call 工具调用 | 📋 |
| **Phase 4** 高级特性 | 动态壁纸 + 电池降载、键鼠模拟、UI Automation、主题系统、MSI 打包 | 📋 |

## 设计原则

- **模块化可开关**:每个模块独立启用/停用,不臃肿;
- **分层架构**:系统调用 / IO / 搜索 / AI / 自动化全在 Rust 后端,前端只管渲染;
- **轻量优先**:空闲内存目标 <120MB,后台零轮询轰炸,索引懒加载;
- **密钥安全**:AI API Key 只存本地配置文件,不进前端、不上网络。

## 参考与致谢

架构与思路参考了这些优秀开源项目(按模块):

- **桌面美化**:Lively Wallpaper、Seelen-UI、dwall、Sapphire-EnhancedDesktop、SnowDesktop、Layouter
- **启动器**:ZeroLaunch-rs(拼音/模糊搜索)、Flow Launcher、Wox、QuickLauncher
- **剪贴板**:ClipVault、TieZ、EcoPaste、Copy Creator
- **AI 桌面**:Hope Agent、Abu-Cowork、Desktop-Claw、NAVI
- **Tauri 参考**:Yogu、健康办公助手、今序 (Torder)
- **任务栏**:ZenDesktop、Taskbar Widgets、ComposeWindowsTaskBar

## License

待定(规划使用 MIT)。
