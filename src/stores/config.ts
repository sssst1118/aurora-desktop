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
  // ---- Phase6(与 src-tauri/src/commands/config.rs 同步)----
  skin: string; // 皮肤包 "deep"|"midnight"|"dawn"|"verdant",默认 "deep"
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
  // ---- 稳定性包(与 src-tauri/src/commands/config.rs 同步)----
  launch_at_startup: boolean; // 开机自启;真值为注册表 Run 键(后端 launch_get_startup 读取)
  first_run_done: boolean; // 首次启动引导完成标记(后端自动置位)
}

export const useConfigStore = defineStore("config", {
  state: () => ({
    cfg: null as AppConfig | null,
  }),
  actions: {
    /**
     * 拉取后端配置。整对象替换会冲掉用户在加载期间的本地修改(P1 根因 2026-08-13:
     * 设置面板激活时 onActivated 发起 load,用户紧接着点击按钮,晚到的旧快照覆盖新值,
     * 导致"配置已落盘但界面没切换")。防覆盖规则:
     * - 已有更新的 load 发起 → 本结果过期,丢弃;
     * - 本次加载期间用户保存成功过(本地修改优先于旧快照)→ 丢弃;
     * - force=true 时无条件覆盖(saveSafe 失败回滚用,以磁盘实际落盘为准)。
     */
    async load(force = false) {
      const id = ++loadId;
      const fresh = await invoke<AppConfig>("config_load");
      if (!force && (id !== loadId || lastSaveId >= id)) return;
      this.cfg = fresh;
    },
    async save() {
      if (!this.cfg) return;
      // 标记在发起时而非成功时:用户点击到 save 发起是同一同步块,标记后任何
      // 更早发起的 load 快照(IPC 在飞)返回时都会因 lastSaveId >= id 被丢弃,
      // 不会覆盖保存期间的本地修改(若等保存成功才标记,保存 IPC 在飞时旧
      // load 返回仍会冲掉用户的新值)
      lastSaveId = loadId;
      const ok = await invoke<boolean>("config_save", { cfg: this.cfg });
      // 保存失败统一抛错,由调用方(Settings 等)捕获后回滚本地值并提示
      if (!ok) {
        throw new Error("config_save 返回失败");
      }
      // 热生效:保存成功即广播,依赖配置的组件(如 Dock 开关)监听后即时刷新,无需重启/下次呼出
      window.dispatchEvent(new CustomEvent("aurora:config-saved"));
    },
  },
});

/** load 请求单调序号(防旧 load 晚到覆盖新 load 结果) */
let loadId = 0;
/** 最近一次成功保存时的 loadId:所有 id ≤ lastSaveId 的 load 结果均视为过期 */
let lastSaveId = 0;
