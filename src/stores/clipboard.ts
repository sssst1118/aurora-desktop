import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";

/** 与 Rust 侧 ClipboardItem 对应(开发文档 §5 事件契约) */
export interface ClipboardItem {
  tp: "text" | "image";
  payload: string;
  ts: number;
}

/** 带原列表下标的条目(搜索过滤后回贴需要原始索引) */
export interface HistoryEntry {
  item: ClipboardItem;
  index: number;
}

export const useClipboardStore = defineStore("clipboard", {
  state: () => ({
    /** 历史全量(后端内存态,最新在前) */
    items: [] as ClipboardItem[],
    /** 搜索关键字(前端本地过滤,量小无需后端索引) */
    keyword: "",
    /** 当前选中项(过滤后列表中的下标) */
    selected: 0,
  }),
  getters: {
    /** 按关键字过滤,保留原始索引供回贴 */
    filtered(state): HistoryEntry[] {
      const k = state.keyword.trim().toLowerCase();
      if (!k) return state.items.map((item, index) => ({ item, index }));
      return state.items
        .map((item, index) => ({ item, index }))
        .filter((e) => e.item.payload.toLowerCase().includes(k));
    },
  },
  actions: {
    /** 拉全量历史(窗口显示时与收到 clipboard-updated 事件后调用) */
    async refresh() {
      this.items = (await invoke<ClipboardItem[]>("clipboard_get_history")) ?? [];
      this.selected = 0;
    },
    /** 清空历史(内存 + 本地文件) */
    async clear() {
      await invoke("clipboard_clear_history");
      this.items = [];
      this.selected = 0;
    },
    /** 回贴第 index 条到系统剪贴板(原始列表下标) */
    async copyBack(index: number) {
      await invoke("clipboard_copy_back", { index });
    },
  },
});
