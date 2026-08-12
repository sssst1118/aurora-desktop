<script setup lang="ts">
import { ref, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Settings from "./Settings.vue";

interface AppEntry {
  name: string;
  path: string;
}

const query = ref("");
const results = ref<AppEntry[]>([]);
const selected = ref(0);
const inputEl = ref<HTMLInputElement | null>(null);
const showSettings = ref(false);
let debounceTimer: number | undefined;

const win = getCurrentWindow();

async function doSearch() {
  const q = query.value.trim();
  if (!q) {
    results.value = [];
    return;
  }
  try {
    results.value = (await invoke<AppEntry[]>("search_apps", { query: q })) ?? [];
    selected.value = 0;
  } catch (e) {
    results.value = [];
    console.error(e);
  }
}

function onInput() {
  if (debounceTimer) window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(doSearch, 150);
}

function selectNext() {
  if (results.value.length === 0) return;
  selected.value = (selected.value + 1) % results.value.length;
}

function selectPrev() {
  if (results.value.length === 0) return;
  selected.value = (selected.value - 1 + results.value.length) % results.value.length;
}

async function openSelected() {
  const item = results.value[selected.value];
  if (!item) return;
  await invoke("open_item", { path: item.path });
  await win.hide();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "ArrowDown") {
    e.preventDefault();
    selectNext();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    selectPrev();
  } else if (e.key === "Enter") {
    e.preventDefault();
    openSelected();
  } else if (e.key === "Escape") {
    win.hide();
  }
}

/** 窗口每次显示时:关闭设置、清空输入、聚焦输入框 */
function onShown() {
  if (showSettings.value) showSettings.value = false;
  query.value = "";
  results.value = [];
  selected.value = 0;
  void nextTick().then(() => inputEl.value?.focus());
}

win.onFocusChanged(({ payload: focused }) => {
  if (focused) onShown();
});

function toggleSettings() {
  showSettings.value = !showSettings.value;
}
</script>

<template>
  <div
    class="h-full w-full flex flex-col bg-[var(--aurora-panel)] backdrop-blur-xl rounded-xl overflow-hidden text-[var(--aurora-text)]"
  >
    <template v-if="!showSettings">
      <div class="flex items-center gap-2 px-4 py-3 border-b border-[var(--aurora-border)]">
        <span>🔍</span>
        <input
          ref="inputEl"
          v-model="query"
          class="flex-1 bg-transparent outline-none text-sm placeholder:text-[var(--aurora-text-dim)]"
          placeholder="搜索应用…"
          @input="onInput"
          @keydown="onKeydown"
        />
        <button
          class="text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] text-sm"
          title="设置"
          @click="toggleSettings"
        >
          ⚙
        </button>
      </div>
      <div class="flex-1 overflow-y-auto py-1">
        <div v-if="query.trim() && results.length === 0" class="px-4 py-3 text-xs text-[var(--aurora-text-dim)]">
          无匹配结果
        </div>
        <div
          v-for="(item, i) in results"
          :key="item.path"
          class="px-4 py-2 flex items-center gap-3 text-sm cursor-pointer"
          :class="i === selected ? 'bg-[var(--aurora-field)]' : ''"
          @mouseenter="selected = i"
          @click="openSelected"
        >
          <span class="text-base">🖥️</span>
          <span>{{ item.name }}</span>
        </div>
      </div>
      <div class="px-4 py-2 text-[10px] text-[var(--aurora-text-dim)] border-t border-[var(--aurora-border)]">
        ↑↓ 选择 · Enter 打开 · Esc 关闭
      </div>
    </template>
    <Settings v-else @close="toggleSettings" />
  </div>
</template>
