<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useClipboardStore, type ClipboardItem, type HistoryEntry } from "../../stores/clipboard";
import { useClipboardHistory } from "../../composables/useClipboardHistory";

const store = useClipboardStore();
const win = getCurrentWindow();
const inputEl = ref<HTMLInputElement | null>(null);

const { start, stop } = useClipboardHistory();

const list = computed(() => store.filtered);

function pad(n: number) {
  return n.toString().padStart(2, "0");
}

function fmtTime(ts: number): string {
  const d = new Date(ts * 1000);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 内容摘要:折叠换行为空格,避免列表撑高 */
function summary(item: ClipboardItem): string {
  return item.payload.replace(/\s+/g, " ").trim();
}

async function copyBack(index: number) {
  try {
    await store.copyBack(index);
    // 回贴后自动隐藏窗口,贴近系统 Win+V 交互(焦点回到原应用)
    await win.hide();
  } catch (e) {
    console.error("copy back failed", e);
  }
}

// 单条删除(悬停 ✕):不弹确认(单条误删代价小,与全量清空的两层确认区分);
// 契约 clipboard_delete_item { index: 全量列表下标 },index 越界后端返回 Err,成功返回删除后条数
const deleteError = ref("");
let deleteErrorTimer: number | undefined;

async function deleteItem(entry: HistoryEntry) {
  try {
    await invoke<number>("clipboard_delete_item", { index: entry.index });
    // 后端删除成功,本地列表同步移除(index 口径与 copyBack 一致 = 全量列表下标)
    store.items.splice(entry.index, 1);
    if (store.selected >= list.value.length) {
      store.selected = Math.max(0, list.value.length - 1);
    }
    deleteError.value = "";
  } catch (e) {
    deleteError.value = `删除失败:${e}`;
    if (deleteErrorTimer) window.clearTimeout(deleteErrorTimer);
    deleteErrorTimer = window.setTimeout(() => {
      deleteError.value = "";
    }, 3000);
  }
}

// 清空确认:两层点击(第一击进入确认态,3s 未确认自动复原),成功后短暂显示"已清空"
const confirming = ref(false);
const cleared = ref(false);
let confirmTimer: number | undefined;
let clearedTimer: number | undefined;

async function clearAll() {
  try {
    await store.clear();
    cleared.value = true;
    if (clearedTimer) window.clearTimeout(clearedTimer);
    clearedTimer = window.setTimeout(() => {
      cleared.value = false;
    }, 1500);
  } catch (e) {
    console.error("clear history failed", e);
  }
}

function onClickClear() {
  if (!confirming.value) {
    confirming.value = true;
    if (confirmTimer) window.clearTimeout(confirmTimer);
    confirmTimer = window.setTimeout(() => {
      confirming.value = false;
    }, 3000);
    return;
  }
  if (confirmTimer) window.clearTimeout(confirmTimer);
  confirmTimer = undefined;
  confirming.value = false;
  void clearAll();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "ArrowDown") {
    e.preventDefault();
    if (list.value.length) store.selected = (store.selected + 1) % list.value.length;
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    if (list.value.length) {
      store.selected = (store.selected - 1 + list.value.length) % list.value.length;
    }
  } else if (e.key === "Enter") {
    e.preventDefault();
    const entry = list.value[store.selected];
    if (entry) void copyBack(entry.index);
  } else if (e.key === "Escape") {
    win.hide();
  }
}

onMounted(async () => {
  void store.refresh().catch((e) => console.error("load history failed", e));
  void start();
  void nextTick().then(() => inputEl.value?.focus());
  // 窗口显示(呼出)时拉最新历史并聚焦(不绑焦点事件,见上方说明)
  unlistenShow = await listen("tauri://show", () => {
    void store.refresh().catch((e) => console.error("load history failed", e));
    void nextTick().then(() => inputEl.value?.focus());
  });
});

onUnmounted(() => {
  stop();
  unlistenShow?.();
  if (confirmTimer) window.clearTimeout(confirmTimer);
  if (clearedTimer) window.clearTimeout(clearedTimer);
  if (deleteErrorTimer) window.clearTimeout(deleteErrorTimer);
});

// 窗口真正显示时(tauri://show)拉最新历史并聚焦搜索框;不绑焦点事件——
// 焦点在窗口激活抖动(拖拽/缩放/点击回窗)时也会触发,会打断面板内正在进行的搜索
let unlistenShow: UnlistenFn | undefined;
</script>

<template>
  <div
    class="h-full w-full flex flex-col bg-[var(--aurora-panel)] backdrop-blur-xl rounded-xl overflow-hidden text-[var(--aurora-text)] select-none"
  >
    <!-- 标题栏:图标 + 搜索框 + 清空 -->
    <div class="flex items-center gap-2 px-4 py-3 border-b border-[var(--aurora-border)]">
      <span class="text-sm">📋</span>
      <input
        ref="inputEl"
        v-model="store.keyword"
        class="flex-1 bg-transparent outline-none text-sm placeholder:text-[var(--aurora-text-dim)]"
        placeholder="搜索剪贴板历史…"
        @keydown="onKeydown"
      />
      <button
        class="text-xs shrink-0 transition-colors"
        :class="
          cleared
            ? 'text-[var(--aurora-success)]'
            : confirming
              ? 'text-[var(--aurora-danger)]'
              : 'text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)]'
        "
        :title="confirming ? '再次点击确认清空(本地文件一并删除)' : '清空历史(本地文件一并删除)'"
        @click="onClickClear"
      >
        {{ cleared ? "已清空" : confirming ? "确认清空?" : "清空" }}
      </button>
    </div>
    <!-- 历史列表 -->
    <div class="flex-1 overflow-y-auto py-1">
      <!-- 单条删除失败提示(3s 自动消失) -->
      <div
        v-if="deleteError"
        class="mx-4 mt-1 rounded-lg bg-[var(--aurora-danger-bg)] px-3 py-1 text-xs text-[var(--aurora-danger)]"
      >
        {{ deleteError }}
      </div>
      <div v-if="store.items.length === 0" class="px-4 py-6 text-center text-xs text-[var(--aurora-text-dim)]">
        暂无历史,复制文本后自动记录
      </div>
      <div v-else-if="list.length === 0" class="px-4 py-6 text-center text-xs text-[var(--aurora-text-dim)]">
        无匹配结果
      </div>
      <div
        v-for="(entry, i) in list"
        :key="`${entry.item.ts}-${i}`"
        class="group px-4 py-2 flex items-center gap-3 cursor-pointer"
        :class="i === store.selected ? 'bg-[var(--aurora-field)]' : ''"
        @mouseenter="store.selected = i"
        @click="copyBack(entry.index)"
      >
        <span class="text-base shrink-0">{{ entry.item.tp === "image" ? "🖼️" : "📄" }}</span>
        <span class="flex-1 min-w-0 block text-sm truncate">
          {{ entry.item.tp === "image" ? entry.item.payload : summary(entry.item) }}
        </span>
        <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">{{ fmtTime(entry.item.ts) }}</span>
        <!-- 悬停 ✕ 单条删除(对标 Win+V;stop 阻止冒泡触发回贴) -->
        <button
          class="hidden h-4 w-4 shrink-0 items-center justify-center rounded-full text-[10px] leading-none text-[var(--aurora-text-dim)] group-hover:flex hover:bg-[var(--aurora-danger)] hover:text-white"
          title="删除该条"
          @click.stop="deleteItem(entry)"
        >
          ✕
        </button>
      </div>
    </div>
    <div class="px-4 py-2 text-[10px] text-[var(--aurora-text-dim)] border-t border-[var(--aurora-border)]">
      ↑↓ 选择 · Enter 回贴 · Esc 关闭
    </div>
  </div>
</template>
