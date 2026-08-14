# Aurora 极光

[![License: MIT](https://img.shields.io/badge/License-MIT-blue)](./LICENSE)
[![Platform: Windows 10/11](https://img.shields.io/badge/Platform-Windows%2010%2F11-0078D6)](https://github.com/sssst1118/aurora-desktop)
[![Version: 0.2.3](https://img.shields.io/badge/Version-0.2.3-8b5cf6)](https://github.com/sssst1118/aurora-desktop/releases)
[![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8D8)](https://tauri.app)

[English](./README.md) | 中文

> **极光之下，桌面宇宙。**
> 一个悬浮在屏幕顶端的极光带，承载整个 Windows 桌面效率宇宙。

Aurora 是 Windows 10/11 上的开源桌面效率中枢——灵动岛（常驻药丸）是唯一入口：单击展开岛身，Dock 应用图标一并括入，快速启动；双击呼出主面板（小桌面/搜索/剪贴板/AI 助手/设置五合一）。每个模块独立可开关：不需要的关掉，需要的留下。

## 📸 界面预览

灵动岛（收起 / 展开带 Dock）与主面板五视图：

| 灵动岛 | 小桌面 | 搜索 | 剪贴板 | AI 助手 |
|---|---|---|---|---|
| ![灵动岛](docs/screenshots/aurora-island.png) | ![小桌面](docs/screenshots/aurora-panel-drawer.png) | ![搜索](docs/screenshots/aurora-panel-search.png) | ![剪贴板](docs/screenshots/aurora-panel-clip.png) | ![AI 助手](docs/screenshots/aurora-panel-ai.png) |

设置页与四套皮肤：

| 设置（深空极光） | 午夜 | 拂晓 | 翠野 |
|---|---|---|---|
| ![设置](docs/screenshots/aurora-panel-settings.png) | ![午夜](docs/screenshots/aurora-skin-midnight.png) | ![拂晓](docs/screenshots/aurora-skin-dawn.png) | ![翠野](docs/screenshots/aurora-skin-verdant.png) |

> 截图由 [交互式 UI 预览](./docs/aurora-v02-preview.html) 渲染生成（`tools/screenshot-preview.ps1`），与设计稿保持同步。

## ✨ 亮点特性

- **🌌 灵动岛（药丸形态）** — 常驻屏幕顶端：时间、CPU、内存、网速一眼尽览；**单击展开岛身，Dock 应用图标一并括入**（真实图标/运行绿点/悬停删除/拖拽添加），点图标即启动；可拖动到任意位置，主面板跟随
- **🪟 一岛一窗主面板** — 双击岛或 `Ctrl+Shift+Space` 呼出，从岛正下方展开：小桌面（默认）/搜索/剪贴板/AI 助手/设置五视图一键切换；**想搜就打字，结果与最近打开同屏**，搜索不再是界面，而是动作
- **📁 小桌面（文件收纳）** — 桌面文件按类型自动分类收纳，新增即时刷新，主面板默认视图，桌面从此整洁
- **⚡ 全局搜索** — 拼音搜索（`jsq` → 计算器）、应用+文件双结果组、最近打开智能置顶，回车直达
- **📋 剪贴板历史** — 自动记录每一次复制，可搜索、一键回贴、单条删除，重启不丢
- **🤖 AI 助手** — DeepSeek 云端 + Ollama 本地双模式；自然语言打开应用、搜索文件、更换壁纸，危险操作先确认
- **🖼️ 动态壁纸** — 视频注入桌面底层，笔记本自动降载，多屏热插拔自动重建，可逐屏配置
- **🎨 四套皮肤 + 极光缘** — 深空极光/午夜/拂晓/翠野一键切换，4 色强调色；每个面板顶部都有一道流动的极光带（签名视觉），保存即全窗口生效
- **🔄 自动更新** — 应用内检查、下载、一键升级，下载包 SHA-256 校验
- **🔒 隐私轻量** — 零遥测；API 密钥仅存本机；空闲内存 <120MB；模块按需启停，零后台浪费
- **🚀 主力工具级** — 开机自启、首次使用引导、配置一键导入导出、崩溃日志可查、配置原子写入

## 💻 系统要求

| 项目 | 要求 |
|---|---|
| 系统 | Windows 10 21H2 及以上（64 位） |
| 架构 | x64 |
| 内存 | 建议 4GB 以上（应用空闲占用 <120MB）；跑本地 AI 模型建议 8GB 以上 |
| 磁盘 | 安装包约 5MB（NSIS）/ 7MB（MSI） |
| 显卡 | 无独立要求（毛玻璃/动态壁纸可用核显） |
| AI 功能（可选） | DeepSeek API Key，或本地 Ollama 服务 |

## 🚀 快速开始

1. 从 [Releases](https://github.com/sssst1118/aurora-desktop/releases) 下载安装包（推荐 `Aurora_0.2.3_x64-setup.exe`，备选 `Aurora_0.2.3_x64_en-US.msi`）
2. 双击安装并启动 Aurora
3. 按 `Ctrl+Shift+Space` 呼出主面板，开始使用

> 安装包尚未代码签名——若 SmartScreen 弹警告，选择「更多信息 → 仍要运行」即可；自动更新仍有 SHA-256 校验兜底。

**配置 AI（可选）**：AI 助手默认开启，但需配置后才能回复——设置 → AI 助手 → 填入 DeepSeek API Key；或指向本地 Ollama（如 `qwen2.5:7b-instruct-q4_K_M`），云端/本地自由切换。

**想先看看长什么样？** 打开 [交互式 UI 预览](./docs/aurora-v02-preview.html)，无需安装。

## ⌨️ 快捷键

| 快捷键 | 功能 |
|---|---|
| `Ctrl+Shift+Space` | 呼出主面板（小桌面） |
| `Ctrl+Alt+D` | 呼出主面板并定位到小桌面 |
| `Ctrl+Alt+V` | 呼出主面板并定位到剪贴板 |
| `Ctrl+Alt+A` | 呼出主面板并定位到 AI 助手 |
| `Ctrl+Shift+H` | 显示/隐藏全部窗口 |
| 单击灵动岛 | 展开/收起 Dock |
| 双击灵动岛 | 呼出主面板 |

> 注：Dock/抽屉/剪贴板/AI/壁纸模块**默认全部开启**，不需要的在设置里关掉即可，热键随模块停用失效；三个 `Ctrl+Alt+*` 热键均可在设置中改键。

## 🛠️ 技术一览（开发者）

- **外壳**：Tauri 2（多窗口、透明无边框、托盘、权限白名单）
- **后端**：Rust + Win32 原生 API——系统调用、搜索索引、AI 代理全在后端
- **前端**：Vue3 + TypeScript + Vite + TailwindCSS
- 开发/打包说明见 [docs/开发文档.md](./docs/开发文档.md)，模块明细与路线图见 [docs/开发进度.md](./docs/开发进度.md)

## 📜 设计原则

- 模块化可开关，不臃肿
- 前端只管渲染，重活全在 Rust 后端
- 空闲内存 <120MB，后台低频采样，不做轮询轰炸
- 所有设置保存即生效，零重启
- AI 密钥只存本地，不进前端、不上网络

## ❓ 常见问题

- **热键没反应？** 可能被其他程序占用——到设置里改一个键位即可。
- **SmartScreen 弹警告？** 安装包尚未代码签名；选「更多信息 → 仍要运行」。更新包有 SHA-256 校验。
- **数据存在哪？** 全部在 `%APPDATA%\com.aurora.desktop\` 下：config.json、clipboard.json 和 logs\（崩溃日志为 panic-*.log）。

## 📬 反馈

发现 Bug 或有新想法？到 [GitHub Issues](https://github.com/sssst1118/aurora-desktop/issues) 提 issue——若是崩溃，请附上 logs 目录里最新的 `panic-*.log`。

## 致谢

架构与思路参考了 Lively Wallpaper、Seelen-UI、Flow Launcher、Wox、EcoPaste 等优秀开源项目。

## License

[MIT](./LICENSE) © 2026 [sssst1118](https://github.com/sssst1118)
