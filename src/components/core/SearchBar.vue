<script setup lang="ts">
import { ref, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Settings from "./Settings.vue";
import Dock from "./Dock.vue";

interface AppEntry {
  name: string;
  path: string;
}

const query = ref("");
const results = ref<AppEntry[]>([]);
const selected = ref(0);
const inputEl = ref<HTMLInputElement | null>(null);
const showSettings = ref(false);
// 2.1 Dock(并入搜索窗口形态):每次窗口显示时重读开关,设置改动下次呼出生效
const enableDock = ref(false);
let debounceTimer: number | undefined;

const win = getCurrentWindow();

// Dock 开关立即读取(不等 onShown):Dock 组件在应用启动时即挂载并后台提取图标,
// 用户呼出搜索栏时图标已就绪(否则首次呼出才挂载,COM 初始化的 ~1.9s 成本
// 会压在"打开搜索栏之后"——实测首 lnk 图标提取独占 1.85s)
void loadDockFlag();

async function loadDockFlag() {
  try {
    const cfg = await invoke<{ enable_dock: boolean }>("config_load");
    enableDock.value = cfg.enable_dock ?? false;
  } catch {
    enableDock.value = false;
  }
}

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

/** 窗口每次显示时:关闭设置、清空输入、聚焦输入框、重读 Dock 开关 */
function onShown() {
  if (showSettings.value) showSettings.value = false;
  query.value = "";
  results.value = [];
  selected.value = 0;
  void loadDockFlag();
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
    <!-- 2.1 Dock 并入搜索窗口:底部图标排(开关在设置页,下次呼出生效) -->
    <Dock v-if="enableDock" />
  </div>
</template>
