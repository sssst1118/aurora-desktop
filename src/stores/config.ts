import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";

/** 与 Rust 侧 AppConfig 字段一一对应 */
export interface AppConfig {
  hotkey_search: string;
  enable_island: boolean;
  enable_dock: boolean;
  enable_file_drawer: boolean;
  enable_clipboard_history: boolean;
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
