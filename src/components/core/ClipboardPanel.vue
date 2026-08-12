<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useClipboardStore, type ClipboardItem } from "../../stores/clipboard";
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

async function clearAll() {
  try {
    await store.clear();
  } catch (e) {
    console.error("clear history failed", e);
  }
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

onMounted(() => {
  void store.refresh().catch((e) => console.error("load history failed", e));
  void start();
  void nextTick().then(() => inputEl.value?.focus());
});

onUnmounted(() => {
  stop();
});

// 窗口每次显示时拉最新历史并聚焦搜索框
win.onFocusChanged(({ payload: focused }) => {
  if (focused) {
    void store.refresh().catch((e) => console.error("load history failed", e));
    void nextTick().then(() => inputEl.value?.focus());
  }
});
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
        class="text-xs text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] shrink-0"
        title="清空历史(本地文件一并删除)"
        @click="clearAll"
      >
        清空
      </button>
    </div>
    <!-- 历史列表 -->
    <div class="flex-1 overflow-y-auto py-1">
      <div v-if="store.items.length === 0" class="px-4 py-6 text-center text-xs text-[var(--aurora-text-dim)]">
        暂无历史,复制文本后自动记录
      </div>
      <div v-else-if="list.length === 0" class="px-4 py-6 text-center text-xs text-[var(--aurora-text-dim)]">
        无匹配结果
      </div>
      <div
        v-for="(entry, i) in list"
        :key="`${entry.item.ts}-${i}`"
        class="px-4 py-2 flex items-center gap-3 cursor-pointer"
        :class="i === store.selected ? 'bg-[var(--aurora-field)]' : ''"
        @mouseenter="store.selected = i"
        @click="copyBack(entry.index)"
      >
        <span class="text-base shrink-0">{{ entry.item.tp === "image" ? "🖼️" : "📄" }}</span>
        <span class="flex-1 min-w-0 block text-sm truncate">
          {{ entry.item.tp === "image" ? entry.item.payload : summary(entry.item) }}
        </span>
        <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">{{ fmtTime(entry.item.ts) }}</span>
      </div>
    </div>
    <div class="px-4 py-2 text-[10px] text-[var(--aurora-text-dim)] border-t border-[var(--aurora-border)]">
      ↑↓ 选择 · Enter 回贴 · Esc 关闭
    </div>
  </div>
</template>
