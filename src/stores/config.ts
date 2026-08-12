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
  // 2.1 Dock 并入搜索窗口后仅剩条目;dock_position/dock_auto_hide 已废弃 2026-08-12
  dock_items: DockItem[];
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
  // ---- Phase4(与 src-tauri/src/commands/config.rs 同步)----
  enable_dynamic_wallpaper: boolean;
  wallpaper_dynamic_dir: string | null;
  wallpaper_scale_mode: string;
  wallpaper_battery_downshift: boolean;
  wallpaper_battery_threshold_pct: number;
  wallpaper_battery_check_sec: number;
  enable_automation: boolean;
  automation_uia_enable: boolean;
  automation_click_delay_ms: number;
  theme_mode: string;
  theme_accent: string;
  // ---- Phase5(与 src-tauri/src/commands/config.rs 同步)----
  update_enabled: boolean;
  update_feed_url: string;
  wallpaper_multi_monitor: boolean;
  wallpaper_span_mode: boolean;
  // ---- 搜索框外观与几何记忆(2026-08-12;search_x/y/w/h 由后端记忆,前端不直接改)----
  search_style: string; // "glass" 毛玻璃(默认) | "solid" 不透明
  search_x: number | null;
  search_y: number | null;
  search_width: number | null;
  search_height: number | null;
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
      if (!this.cfg) return;
      const ok = await invoke<boolean>("config_save", { cfg: this.cfg });
      // 热生效:保存成功即广播,依赖配置的组件(如 Dock 开关)监听后即时刷新,无需重启/下次呼出
      if (ok) window.dispatchEvent(new CustomEvent("aurora:config-saved"));
    },
  },
});
