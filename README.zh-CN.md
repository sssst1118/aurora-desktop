# Aurora 极光

[English](./README.md) | 中文

> **极光之下,桌面宇宙。**
> 一个悬浮在屏幕顶端的极光带,承载整个 Windows 桌面效率宇宙。

Aurora 是 Windows 10/11 上的开源桌面效率中枢——全局搜索、灵动岛、文件收纳、剪贴板历史、AI 助手、动态壁纸,一站集齐。每个模块独立可开关:只装你需要的,不多一点。

## ✨ 亮点特性

- **⚡ 全局搜索** — `Ctrl+Shift+Space` 即呼即搜,开始菜单应用秒级定位,回车直达
- **🌌 灵动岛** — 常驻屏幕顶端:时间、CPU、内存、网速一眼尽览,置顶不抢焦点
- **📁 文件抽屉** — 桌面文件按类型自动分类收纳,新增即时刷新,桌面从此整洁
- **📋 剪贴板历史** — 自动记录每一次复制,可搜索、一键回贴,重启不丢
- **🤖 AI 助手** — DeepSeek 云端 + Ollama 本地双模式;自然语言打开应用、搜索文件、更换壁纸
- **🖼️ 动态壁纸** — 视频 / HTML / WebGL 注入桌面底层,笔记本自动降载,多屏热插拔自动重建
- **📊 系统监控** — CPU / 内存 / 网络实时采样,灵动岛与托盘双处可见
- **🔒 隐私轻量** — API 密钥仅存本机;空闲内存 <120MB;模块按需启停,零后台浪费

## 💻 系统要求

| 项目 | 要求 |
|---|---|
| 系统 | Windows 10 21H2 及以上(64 位) |
| 架构 | x64 |
| 内存 | 建议 4GB 以上(应用空闲占用 <120MB) |
| 磁盘 | 安装包约 7MB |
| 显卡 | 无独立要求(毛玻璃/动态壁纸可用核显) |
| AI 功能(可选) | DeepSeek API Key,或本地 Ollama 服务 |

## 🚀 快速开始

1. 从 [Releases](https://github.com/sssst1118/aurora-desktop/releases) 下载安装包(`Aurora_0.1.0_x64.msi` 或 `setup.exe`)
2. 双击安装并启动 Aurora
3. 按 `Ctrl+Shift+Space` 呼出全局搜索,开始使用

**配置 AI(可选)**:设置 → AI 助手 → 填入 DeepSeek API Key;或指向本地 Ollama(如 `qwen2.5:7b-instruct-q4_K_M`),云端 / 本地自由切换。

## ⌨️ 快捷键

| 快捷键 | 功能 |
|---|---|
| `Ctrl+Shift+Space` | 全局搜索 |
| `Ctrl+Alt+D` | 文件抽屉 |
| `Ctrl+Alt+V` | 剪贴板历史 |
| `Ctrl+Alt+A` | AI 助手 |

## 🛠️ 技术一览(开发者)

- **外壳**:Tauri 2(多窗口、透明无边框、托盘、权限白名单)
- **后端**:Rust + Win32 原生 API——系统调用、搜索索引、AI 代理全在后端
- **前端**:Vue3 + TypeScript + Vite + TailwindCSS
- 开发 / 打包说明见 [docs/开发文档.md](./docs/开发文档.md),模块明细与路线图见 [docs/开发进度.md](./docs/开发进度.md)

## 📜 设计原则

- 模块化可开关,不臃肿
- 前端只管渲染,重活全在 Rust 后端
- 空闲内存 <120MB,后台零轮询
- AI 密钥只存本地,不进前端、不上网络

## 致谢

架构与思路参考了 Lively Wallpaper、Seelen-UI、Flow Launcher、Wox、EcoPaste 等优秀开源项目。

## License

[MIT](./LICENSE) © 2026 [sssst1118](https://github.com/sssst1118)
