import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";

/** Dock 快捷方式条目(与 Rust 侧 DockItem 对应) */
export interface DockItem {
  name: string;
  path: string;
}

/** 与 Rust 侧 AppConfig 字段一一对应 */
export interface AppConfig {
  hotkey_search: string;
  enable_island: boolean;
  enable_dock: boolean;
  enable_file_drawer: boolean;
  enable_clipboard_history: boolean;
  // ---- Phase2 新增(与 src-tauri/src/commands/config.rs 同步)----
  dock_items: DockItem[];
  dock_position: string;
  dock_auto_hide: boolean;
  drawer_hotkey: string;
  drawer_open_on_launch: boolean;
  clipboard_max_items: number;
  hotkey_clipboard: string;
  wallpaper_dir: string | null;
  // ---- Phase3 AI 集成(与 src-tauri/src/commands/config.rs 同步;ai_api_key 只可能是 null 或掩码 "******")----
  enable_ai: boolean;
  ai_provider: string;
  ai_api_key: string | null;
  ai_model: string;
  ai_base_url: string;
  ai_ollama_url: string;
  ai_ollama_model: string;
  ai_tools_enabled: boolean;
  ai_search_roots: string[];
  ai_max_tool_rounds: number;
  ai_hotkey: string;
}

export const useConfigStore = defineStore("config", {
  state: () => ({
    cfg: null as AppConfig | null,
  }),
  actions: {
    async load() {
      this.cfg = await invoke<AppConfig>("config_load");
    },
    async save() {
      if (this.cfg) await invoke<boolean>("config_save", { cfg: this.cfg });
    },
  },
});
