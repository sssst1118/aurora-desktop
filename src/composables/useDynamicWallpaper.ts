import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 与 Rust 侧 PerMonitorState 对应(get_state.monitors 元素) */
export interface PerMonitorState {
  index: number;
  kind: "none" | "video" | "html";
  path: string | null;
  url: string | null;
}

/** 与 Rust 侧 DynamicWallpaperState 对应(wallpaper_dynamic_get_state 返回) */
export interface DynamicWallpaperState {
  enabled: boolean;
  kind: "none" | "image" | "video" | "html";
  path: string | null;
  /** video/html 素材:目录外 = data URL;默认 Pictures 内 = null(前端 convertFileSrc) */
  url: string | null;
  on_battery: boolean;
  downshift_active: boolean;
  /** Phase5 多屏逐屏状态;多屏关 = 空数组 */
  monitors: PerMonitorState[];
}

/** `wallpaper-power` 事件 payload(事件契约见 docs/Phase4-设计.md §0.3) */
export interface WallpaperPowerPayload {
  on_battery: boolean;
}

/**
 * 动态壁纸状态封装:wallpaper 窗口挂载时 start() 拉取初始状态并订阅电池事件,
 * 卸载时 stop()。on_battery 由后端电池检测线程驱动(30s 一次,仅状态翻转时广播,
 * 不变化不广播,防轮询风暴)。
 */
export function useDynamicWallpaper() {
  const state = ref<DynamicWallpaperState>({
    enabled: false,
    kind: "none",
    path: null,
    url: null,
    on_battery: false,
    downshift_active: false,
    monitors: [],
  });
  let unlisten: UnlistenFn | null = null;

  /** 拉取后端最新状态(窗口挂载/恢复时调用) */
  async function refresh(): Promise<void> {
    try {
      state.value = await invoke<DynamicWallpaperState>("wallpaper_dynamic_get_state");
    } catch (e) {
      console.error("wallpaper_dynamic_get_state failed", e);
    }
  }

  /** 订阅电池事件 + 拉初始状态(幂等) */
  async function start(): Promise<void> {
    await refresh();
    if (unlisten) return;
    try {
      unlisten = await listen<WallpaperPowerPayload>("wallpaper-power", (ev) => {
        state.value.on_battery = ev.payload.on_battery;
      });
    } catch (e) {
      console.error("listen wallpaper-power failed", e);
    }
  }

  function stop(): void {
    unlisten?.();
    unlisten = null;
  }

  return { state, refresh, start, stop };
}
