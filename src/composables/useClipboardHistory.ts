import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useClipboardStore } from "../stores/clipboard";

/**
 * 剪贴板历史事件订阅:后端每条新记录入库后广播 `clipboard-updated`(payload = 最新一条),
 * 前端收到后拉全量刷新(事件契约见 docs/Phase2-设计.md §0.3)。
 * 组件挂载时 start、卸载时 stop。
 */
export function useClipboardHistory() {
  const store = useClipboardStore();
  let unlisten: UnlistenFn | null = null;

  async function start(): Promise<void> {
    if (unlisten) return;
    unlisten = await listen("clipboard-updated", () => {
      void store.refresh().catch((e) => console.error("clipboard refresh failed", e));
    });
  }

  function stop(): void {
    unlisten?.();
    unlisten = null;
  }

  return { start, stop };
}
