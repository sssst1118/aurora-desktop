# Aurora

[English](./README.md) | [中文](./README.zh-CN.md)

> **The desktop universe beneath an aurora.**
> A floating aurora band at the top of your screen, holding an entire Windows desktop productivity universe.

Aurora is an open-source desktop productivity hub for Windows 10/11 — global search, island, file drawer, clipboard history, AI assistant, and dynamic wallpapers, all in one place. Every module is independently toggleable: enable only what you need, nothing more.

## ✨ Highlights

- **⚡ Global Search** — Summon with `Ctrl+Shift+Space` and launch any Start-menu app in seconds, Enter to go
- **🌌 Island** — A persistent panel at the top of your screen: time, CPU, memory, network speed at a glance; always on top without stealing focus
- **📁 File Drawer** — Desktop files auto-sorted by type, refreshed on change — a tidy desktop, always
- **📋 Clipboard History** — Every copy auto-recorded, searchable, one-click re-paste, persisted across reboots
- **🤖 AI Assistant** — Dual mode: cloud DeepSeek + local Ollama; drive your system with natural language — open apps, find files, switch wallpapers
- **🖼️ Dynamic Wallpapers** — Video / HTML / WebGL injected beneath the desktop; auto-downshift on battery; hot-plug multi-monitor rebuild
- **📊 System Monitor** — Real-time CPU / memory / network sampling, visible on the Island and in the tray
- **🔒 Private & Light** — API keys stored locally only; idle memory under 120MB; zero background waste

## 💻 System Requirements

| Item | Requirement |
|---|---|
| OS | Windows 10 21H2 or later (64-bit) |
| Architecture | x64 |
| RAM | 4GB+ recommended (idle footprint <120MB) |
| Disk | ~7MB installer |
| GPU | No dedicated GPU required (glass blur & dynamic wallpapers run on iGPUs) |
| AI features (optional) | A DeepSeek API key, or a local Ollama service |

## 🚀 Quick Start

1. Download the installer from [Releases](https://github.com/sssst1118/aurora-desktop/releases) (`Aurora_0.1.0_x64.msi` or `setup.exe`)
2. Install and launch Aurora
3. Press `Ctrl+Shift+Space` to summon global search and start

**Optional AI setup**: Settings → AI Assistant → enter your DeepSeek API key, or point to a local Ollama (e.g. `qwen2.5:7b-instruct-q4_K_M`) — switch freely between cloud and local.

## ⌨️ Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+Space` | Global search |
| `Ctrl+Alt+D` | File drawer |
| `Ctrl+Alt+V` | Clipboard history |
| `Ctrl+Alt+A` | AI assistant |

## 🛠️ For Developers

- **Shell**: Tauri 2 (multi-window, transparent frameless, tray, permission allowlist)
- **Backend**: Rust + native Win32 APIs — system calls, search indexing, and the AI proxy all live here
- **Frontend**: Vue3 + TypeScript + Vite + TailwindCSS
- Dev / packaging docs: [docs/开发文档.md](./docs/开发文档.md) · module details & roadmap: [docs/开发进度.md](./docs/开发进度.md)

## 📜 Design Principles

- Modular and toggleable — no bloat
- The frontend only renders; the heavy lifting stays in the Rust backend
- Idle memory under 120MB, zero background polling storms
- AI keys live in local config only — never in the frontend, never on the network

## Credits

Architecture and ideas inspired by Lively Wallpaper, Seelen-UI, Flow Launcher, Wox, EcoPaste, and other great open-source projects.

## License

[MIT](./LICENSE) © 2026 [sssst1118](https://github.com/sssst1118)
