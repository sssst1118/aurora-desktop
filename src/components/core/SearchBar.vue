<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
// 2.1 Dock(并入搜索窗口形态):启动立即 + 每次窗口显示时重读开关,
// 设置页保存后经 aurora:config-saved 事件即时刷新(热生效)
const enableDock = ref(false);
// 显示方式:"glass" 毛玻璃(默认) | "solid" 不透明(2026-08-12 用户要求可选)
const searchStyle = ref("glass");
let debounceTimer: number | undefined;

const win = getCurrentWindow();

// Dock 开关立即读取(不等 onShown):Dock 组件在应用启动时即挂载并后台提取图标,
// 用户呼出搜索栏时图标已就绪(否则首次呼出才挂载,COM 初始化的 ~1.9s 成本
// 会压在"打开搜索栏之后"——实测首 lnk 图标提取独占 1.85s)
void loadDockFlag();
void loadStyleFlag();

// 热生效:设置页保存成功 → 立即重读 Dock 开关与显示方式(无需重启/下次呼出)
window.addEventListener("aurora:config-saved", () => {
  void loadDockFlag();
  void loadStyleFlag();
});

async function loadDockFlag() {
  try {
    const cfg = await invoke<{ enable_dock: boolean }>("config_load");
    enableDock.value = cfg.enable_dock ?? false;
  } catch {
    enableDock.value = false;
  }
}

async function loadStyleFlag() {
  try {
    const cfg = await invoke<{ search_style?: string }>("config_load");
    searchStyle.value = cfg.search_style === "solid" ? "solid" : "glass";
  } catch {
    searchStyle.value = "glass";
  }
}

/** 容器背景样式:毛玻璃(半透明+模糊) / 不透明 */
const panelClass = computed(() =>
  searchStyle.value === "solid"
    ? "bg-[var(--aurora-panel-solid)]"
    : "bg-[var(--aurora-panel)] backdrop-blur-xl",
);

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

// 状态重置只绑"窗口真正显示"事件(tauri://show):聚焦不重置。
// 根因(2026-08-12 用户实测):拖缩放手柄时窗口被反复 setSize,透明置顶窗口
// 激活状态抖动触发 focused=true → 设置被强制关闭跳回搜索框;
// show 事件仅在 hide→show 时触发,点击回窗口/拖拽缩放均不再影响面板状态
let unlistenShow: UnlistenFn | undefined;

function toggleSettings() {
  showSettings.value = !showSettings.value;
}

// ---- 拖动移动(header 左端手柄,data-tauri-drag-region 原生拖动) ----

// ---- 右下角缩放手柄(无边框窗口无系统 resize 边框,自绘手柄 + setSize) ----

/** 缩放进行中:起始鼠标位置 + 起始窗口逻辑尺寸 */
let resizeState: { sx: number; sy: number; w: number; h: number } | null = null;
let lastDx = 0;
let lastDy = 0;
let resizeRaf = 0;

async function resizeStart(e: PointerEvent) {
  e.preventDefault();
  try {
    const size = await win.innerSize(); // PhysicalSize
    const sf = await win.scaleFactor();
    resizeState = {
      sx: e.screenX,
      sy: e.screenY,
      w: size.width / sf,
      h: size.height / sf,
    };
    window.addEventListener("pointermove", resizeMove);
    window.addEventListener("pointerup", resizeEnd);
  } catch (err) {
    console.error("resize start failed", err);
  }
}

function resizeMove(e: PointerEvent) {
  if (!resizeState) return;
  lastDx = e.screenX - resizeState.sx;
  lastDy = e.screenY - resizeState.sy;
  if (resizeRaf) return; // rAF 节流,避免高频 setSize IPC
  resizeRaf = requestAnimationFrame(applyResize);
}

function applyResize() {
  resizeRaf = 0;
  if (!resizeState) return;
  // 最小尺寸下限:内容(输入框/列表/Dock)不至于挤崩布局
  const w = Math.max(360, Math.round(resizeState.w + lastDx));
  const h = Math.max(260, Math.round(resizeState.h + lastDy));
  win
    .setSize(new LogicalSize(w, h))
    .catch((err) => console.error("setSize failed", err));
}

function resizeEnd() {
  resizeState = null;
  window.removeEventListener("pointermove", resizeMove);
  window.removeEventListener("pointerup", resizeEnd);
  // 缩放结束落一次几何(移动/缩放事件的防抖保存兜底)
  scheduleSaveGeometry();
}

// ---- 几何记忆:拖动/缩放后防抖写回配置文件(下次启动恢复) ----

let geometryTimer: number | undefined;

async function saveGeometry() {
  try {
    const pos = await win.innerPosition(); // PhysicalPosition
    const size = await win.innerSize(); // PhysicalSize
    const sf = await win.scaleFactor();
    await invoke("search_save_geometry", {
      x: Math.round(pos.x / sf),
      y: Math.round(pos.y / sf),
      w: size.width / sf,
      h: size.height / sf,
    });
  } catch (e) {
    console.error("search_save_geometry failed", e);
  }
}

function scheduleSaveGeometry() {
  if (geometryTimer) window.clearTimeout(geometryTimer);
  geometryTimer = window.setTimeout(() => {
    geometryTimer = undefined;
    void saveGeometry();
  }, 600);
}

let unMoved: UnlistenFn | undefined;
let unResized: UnlistenFn | undefined;

onMounted(async () => {
  // 移动/缩放事件 → 防抖保存几何(用户拖动/缩放后重启不丢位置)
  unMoved = await win.onMoved(() => scheduleSaveGeometry());
  unResized = await win.onResized(() => scheduleSaveGeometry());
  // 窗口显示(呼出)时重置面板状态(见上方 onShown 说明,不绑焦点事件)
  unlistenShow = await listen("tauri://show", () => onShown());
});

onUnmounted(() => {
  unMoved?.();
  unResized?.();
  unlistenShow?.();
  if (geometryTimer) window.clearTimeout(geometryTimer);
});
</script>

<template>
  <!-- 整窗拖动(2026-08-12 用户需求:左上角小手柄反常识):根容器 data-tauri-drag-region="true"
        = 点任意空白处直接拖动;Tauri 2.11 判定沿事件路径上溯,INPUT/BUTTON/TEXTAREA 等
        可点击元素自动豁免(点输入框输入、点按钮点击不受影响);
        滚动列表/Dock/Settings 内容区显式 "false" 禁拖(保护滚动条与交互控件) -->
  <div
    class="h-full w-full flex flex-col rounded-xl overflow-hidden text-[var(--aurora-text)] relative"
    :class="panelClass"
    data-tauri-drag-region="true"
  >
    <template v-if="!showSettings">
      <div class="flex items-center gap-2 pl-4 pr-4 py-3 border-b border-[var(--aurora-border)]">
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
      <!-- 结果列表禁拖:滚动条拖动/列表项点击放行 -->
      <div class="flex-1 overflow-y-auto py-1" data-tauri-drag-region="false">
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
    <!-- 右下角缩放手柄(自绘;无边框窗口无系统 resize 边框) -->
    <div
      class="absolute bottom-0 right-0 z-20 w-4 h-4 cursor-nwse-resize"
      @pointerdown="resizeStart"
      title="拖动调整大小"
    >
      <span
        class="absolute bottom-0.5 right-0.5 w-2.5 h-2.5 border-r-2 border-b-2 border-[var(--aurora-text-dim)] opacity-50 pointer-events-none"
      ></span>
    </div>
  </div>
</template>
