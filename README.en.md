# Aurora

[English](./README.en.md) | [中文](./README.md)

> **The desktop universe beneath an aurora.**
> An AI-enhanced, modular desktop productivity hub for Windows — launcher, island, dock, global search, file drawer, clipboard history, wallpaper engine, and AI assistant, all in one place.

Aurora is an open-source desktop productivity hub for Windows 10/11, built to **replace or enhance the native Windows desktop experience**. The island that lives at the top of your screen is like a band of aurora — beneath it lies a complete desktop universe. Every feature module can be toggled on or off independently, so you enable only what you need and nothing more. The Rust backend handles system calls and AI orchestration, while the frontend is responsible purely for rendering and interaction.

## Feature Overview

> ✅ Implemented / 🚧 In progress / 📋 Planned

| Module | Status | Description |
|---|---|---|
| **Global Search Launcher** | 📋 | Summon with a global hotkey for blazing-fast search: apps first, files second, Pinyin / fuzzy matching, Enter to launch |
| **Island** | 📋 | A persistent floating panel pinned to the top of the screen — time / CPU / memory / network speed / media controls, always on top without stealing focus |
| **Dock** | 📋 | A customizable app shortcut bar with drag-to-reorder icons, running-application indicators, and switchable top/bottom edges |
| **Desktop File Drawer** | 📋 | Desktop file organization: logical grouping (files stay put) or physical archiving (desktop cleared), with auto-classification |
| **Clipboard History** | 📋 | Monitors the clipboard in the background; history is searchable, re-pasteable, and persisted locally |
| **Wallpaper Engine** | 📋 | One-click static wallpaper switching + dynamic wallpapers (web/video rendered to the wallpaper layer), with automatic battery-aware downshift on laptops |
| **System Monitor** | 📋 | Real-time CPU / memory / network collection, surfaced in the Island and the system tray |
| **AI Assistant** | 📋 | Dual-mode: cloud DeepSeek + local Ollama; natural-language commands drive system actions directly (open apps, set wallpapers, find files) |
| **UI Automation** | 📋 | Keyboard/mouse simulation and Windows UI Automation control manipulation (an advanced feature with well-defined boundaries) |

## Interface Preview

```
┌─────────────────────────  Island (always on top, aurora band)  ────────────────────────┐
│  🔍  ▸ 14:32:08   CPU 12%   Mem 5.8G/16G   ↑1.2M/s ↓3.4M/s  ♪     │
└──────────────────────────────────────────────────────────────────────────────────────┘

        ┌───────────────  Global Search (Ctrl+Shift+Space)  ────────────────┐
        │  🔍  > not                                                    │
        │  ─────────────────────────────────────────────────────────── │
        │  🖥  Notepad                    C:\Windows\notepad.exe       │
        │  📄  ledger.xlsx                D:\Finance\ledger.xlsx       │
        │  ...                                                           │
        └──────────────────────────────────────────────────────────────┘

       ( Dock ───────────────── )( File Drawer )( AI Chat Panel )
```

*The interface sketch is a design draft; the final look depends on actual development.*

## Tech Stack

| Layer | Choice |
|---|---|
| Desktop Shell | Tauri 2.0 (multi-window, transparent frameless, tray, permission allowlist) |
| Backend | Rust 1.80+ / windows-rs (native Win32 APIs), reqwest (AI proxy) |
| Frontend | Vue3 + TypeScript + Vite, TailwindCSS v3, Pinia |
| Plugins | global-shortcut (global hotkeys), clipboard-next (clipboard), wallpaper (wallpaper layer) |
| Package Managers | pnpm / cargo |

## Quick Start

### Prerequisites

- Windows 10 21H2 or later
- Rust MSVC toolchain (Visual Studio Build Tools with C++ workload + Windows SDK)
- Node.js LTS + pnpm

### Development

```bash
pnpm install          # Install frontend dependencies
pnpm tauri dev        # Launch dev mode (Rust compilation + frontend hot reload)
```

### Build

```bash
pnpm tauri build      # Produce an MSI installer
```

## Project Structure

```
aurora-desktop/
├── src-tauri/            # Rust backend
│   ├── commands/         # Tauri commands: search / files / wallpaper / system / clipboard / AI / automation
│   ├── indexer/          # Start-menu app index, on-demand file indexing
│   └── automation/       # Keyboard/mouse simulation + UI Automation wrapper
└── src/                  # Vue3 frontend
    ├── components/core/  # Island / Dock / SearchBar / FileDrawer / AIPanel / Settings
    └── stores/           # Pinia: module toggles, configuration, clipboard history
```

## Roadmap

| Phase | Scope | Status |
|---|---|---|
| **Phase 1** MVP | Project scaffolding, multi-window + tray, global-hotkey search, Start-menu indexing, basic settings, Island prototype — *usable as an advanced launcher* | 📋 Planned |
| **Phase 2** Productivity | Dock, file drawer, clipboard history, static wallpapers, full system status | 📋 |
| **Phase 3** AI Integration | AI chat panel, DeepSeek / Ollama dual mode, function-call tool use | 📋 |
| **Phase 4** Advanced | Dynamic wallpapers + battery downshift, keyboard/mouse simulation, UI Automation, theming, MSI packaging | 📋 |

## Design Principles

- **Modular and toggleable**: every module can be enabled or disabled independently — no bloat;
- **Layered architecture**: system calls / IO / search / AI / automation all live in the Rust backend; the frontend only renders;
- **Lightweight first**: idle memory target under 120MB, zero background polling storms, lazy-loaded indexing;
- **Secret safety**: AI API keys are stored only in local config files — never in the frontend, never on the network.

## Credits

Architecture and ideas are inspired by these great open-source projects (grouped by module):

- **Desktop beautification**: Lively Wallpaper, Seelen-UI, dwall, Sapphire-EnhancedDesktop, SnowDesktop, Layouter
- **Launchers**: ZeroLaunch-rs (Pinyin/fuzzy search), Flow Launcher, Wox, QuickLauncher
- **Clipboard**: ClipVault, TieZ, EcoPaste, Copy Creator
- **AI desktops**: Hope Agent, Abu-Cowork, Desktop-Claw, NAVI
- **Tauri references**: Yogu, Healthy Office Assistant, Jinsu (Torder)
- **Taskbars**: ZenDesktop, Taskbar Widgets, ComposeWindowsTaskBar

## License

[MIT](./LICENSE) © 2026 [sssst1118](https://github.com/sssst1118)
