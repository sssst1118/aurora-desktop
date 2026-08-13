<script setup lang="ts">
// Dock 图标排组件(Phase2 2.1) — 2026-08-12 形态定调:独立 dock 窗口已废弃,
// 本组件嵌入 SearchBar 底部渲染。添加应用 = 从桌面/Explorer 拖拽 exe/lnk 进本区域
// (Tauri 2 dragDropEnabled 默认拦截 DOM 拖放,必须用 onDragDropEvent 官方 API);
// 删除 = 悬停图标 ✕;点击 = 运行中聚焦/未运行启动;绿点 = 运行指示(2s 轮询)。
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface DockItem {
  name: string;
  path: string;
}

const items = ref<DockItem[]>([]);
const icons = ref<Map<string, string>>(new Map());
const running = ref<Set<string>>(new Set());
// 文件拖入窗口悬停(高亮投放区;仅拖到 Dock 区内才点亮)
const fileDragOver = ref(false);
// "+" 占位点击轻提示(3s 自动消失;添加仍走文件拖入)
const addHint = ref(false);
let addHintTimer: number | undefined;

function showAddHint() {
  addHint.value = true;
  if (addHintTimer) window.clearTimeout(addHintTimer);
  addHintTimer = window.setTimeout(() => {
    addHint.value = false;
  }, 3000);
}
// 内部 DnD 排序:拖拽源下标 + 悬停目标下标(用于高亮)
const dragIdx = ref(-1);
const overIdx = ref(-1);

let runningTimer: number | undefined;
let dragLeaveTimer: number | undefined;
let unlistenHide: UnlistenFn | undefined;
let unlistenShow: UnlistenFn | undefined;

const win = getCurrentWindow();

/** 判断拖放位置是否落在 Dock 区(窗口底部 ~64px,图标排高度) */
function inDockZone(y: number): boolean {
  return y >= window.innerHeight - 64;
}

/** 拖拽进来的文件路径 → 条目名(去掉 .exe/.lnk 后缀) */
function nameOf(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? path;
  return base.replace(/\.(lnk|exe)$/i, "");
}

/** 过滤出可添加的快捷方式文件(exe/lnk) */
function pickAppPaths(paths: string[]): string[] {
  return paths.filter((p) => /\.(lnk|exe)$/i.test(p));
}

/** 添加条目(去重;写后端持久化 + 刷新图标) */
async function addItems(paths: string[]) {
  const fresh = paths.filter((p) => !items.value.some((it) => it.path === p));
  if (fresh.length === 0) return;
  const next = [...items.value, ...fresh.map((p) => ({ name: nameOf(p), path: p }))];
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
    for (const p of fresh) void iconOf(p);
  } catch (e) {
    console.error("dock_set_items failed", e);
  }
}

/** 悬停 ✕ 移除条目 */
async function removeItem(item: DockItem) {
  const next = items.value.filter((it) => it.path !== item.path);
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
  } catch (e) {
    console.error("dock_set_items failed", e);
  }
}

/** 图标 data URL(后端双缓存);失败返回 undefined → 占位首字符 */
async function iconOf(path: string): Promise<string | undefined> {
  const hit = icons.value.get(path);
  if (hit) return hit;
  try {
    const url = await invoke<string | null>("dock_get_icon", { path });
    if (url) {
      icons.value.set(path, url);
      return url;
    }
  } catch (e) {
    console.error("dock_get_icon failed", e);
  }
  return undefined;
}

async function refreshIcons() {
  for (const it of items.value) {
    if (!icons.value.has(it.path)) void iconOf(it.path);
  }
}

/** "启动中"反馈集合(仅视觉;真正的防抖在后端 LAUNCHING 集合,以窗口出现为准) */
const launching = ref<Set<string>>(new Set());
const LAUNCHING_FEEDBACK_MS = 1500;

/** 点击:运行中 → dock_launch 聚焦;未运行 → 启动 */
async function launch(item: DockItem) {
  if (launching.value.has(item.path)) return; // 启动中,忽略连点(后端亦有防抖)
  launching.value.add(item.path);
  setTimeout(() => {
    launching.value.delete(item.path);
  }, LAUNCHING_FEEDBACK_MS);
  try {
    await invoke<boolean>("dock_launch", { item });
    void pollRunning();
  } catch (e) {
    console.error("dock_launch failed", e);
  }
}

/** 2s 轮询运行状态(后端枚举可见窗口与条目路径匹配,返回被收录的运行条目 path) */
async function pollRunning() {
  try {
    const paths = (await invoke<string[]>("dock_get_running")) ?? [];
    running.value = new Set(paths);
  } catch (e) {
    console.error("dock_get_running failed", e);
  }
}

async function loadItems() {
  try {
    items.value = (await invoke<DockItem[]>("dock_get_items")) ?? [];
    void refreshIcons();
  } catch (e) {
    console.error("dock_get_items failed", e);
  }
}

// ---- 文件拖入添加(Tauri 拦截 DOM 拖放,官方 API 接管) ----

// 拖拽悬停期间 Tauri 会连续发 enter/over,leave 在移出窗口时触发;
// 用一个短延时兜底:enter/over 反复点亮时刷新,若突然断流(无 leave)也能熄灭
function markDragZone() {
  fileDragOver.value = true;
  if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
  dragLeaveTimer = window.setTimeout(() => {
    fileDragOver.value = false;
  }, 300);
}

win.onDragDropEvent((event) => {
  const payload = event.payload;
  if (payload.type === "enter" || payload.type === "over") {
    if (inDockZone(payload.position.y)) markDragZone();
  } else if (payload.type === "leave") {
    fileDragOver.value = false;
    if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
  } else if (payload.type === "drop") {
    fileDragOver.value = false;
    if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
    // 只有松手在 Dock 区才算添加(拖到搜索列表区 = 误拖,不处理)
    if (inDockZone(payload.position.y)) {
      void addItems(pickAppPaths(payload.paths));
    }
  }
});

// ---- 内部 DnD 排序(HTML5 元素拖拽,Tauri 只拦截文件拖入,不影响) ----

function dragStart(i: number) {
  dragIdx.value = i;
}

function dragOver(i: number) {
  overIdx.value = i;
}

function dragLeave() {
  overIdx.value = -1;
}

async function dropAt(i: number) {
  const from = dragIdx.value;
  dragIdx.value = -1;
  overIdx.value = -1;
  if (from < 0 || from === i) return;
  const next = [...items.value];
  const [moved] = next.splice(from, 1);
  next.splice(i, 0, moved);
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
  } catch (e) {
    console.error("dock_set_items failed", e);
  }
}

function pad(s: string) {
  return s.length > 1 ? s : s + " ";
}

/** 暂停运行态轮询(窗口隐藏时;后台不轮询,前台无感知) */
function pausePolling() {
  if (runningTimer) {
    window.clearInterval(runningTimer);
    runningTimer = undefined;
  }
}

/** 恢复运行态轮询:立即刷一次再起定时器(窗口重新显示时数据即时跟上) */
function resumePolling() {
  void pollRunning();
  if (runningTimer === undefined) {
    runningTimer = window.setInterval(pollRunning, 2000);
  }
}

onMounted(async () => {
  await loadItems();
  await pollRunning();
  // 窗口隐藏时暂停轮询、显示时立即刷新并恢复(避免后台空转 IPC)
  unlistenHide = await listen("tauri://hide", pausePolling);
  unlistenShow = await listen("tauri://show", resumePolling);
  runningTimer = window.setInterval(pollRunning, 2000);
});

onUnmounted(() => {
  if (runningTimer) window.clearInterval(runningTimer);
  if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
  if (addHintTimer) window.clearTimeout(addHintTimer);
  unlistenHide?.();
  unlistenShow?.();
});
</script>

<template>
  <!-- 整窗可拖后显式禁拖:图标点击/文件投放/排序拖拽放行(tauri 2.11 drag-region false 阻断祖先判定) -->
  <div
    class="relative flex items-center gap-1 border-t border-[var(--aurora-border)] px-2 py-1.5 min-h-14 select-none transition-colors"
    :class="fileDragOver ? 'bg-[var(--aurora-accent)]/20' : ''"
    data-tauri-drag-region="false"
  >
    <!-- 文件拖入投放提示(悬停时浮在图标排上方) -->
    <div
      v-if="fileDragOver"
      class="pointer-events-none absolute -top-7 left-1/2 -translate-x-1/2 rounded-md bg-[var(--aurora-panel)] border border-[var(--aurora-border)] px-3 py-1 text-xs text-[var(--aurora-accent)] shadow-lg"
    >
      松手添加应用
    </div>

    <div
      v-for="(it, i) in items"
      :key="it.path"
      class="group relative flex h-10 w-10 items-center justify-center rounded-xl transition-colors"
      :class="[
        i === overIdx && dragIdx >= 0 ? 'bg-[var(--aurora-field)]' : 'hover:bg-[var(--aurora-field)]',
        dragIdx === i ? 'opacity-40' : '',
        launching.has(it.path) ? 'dock-launching' : '',
      ]"
      :draggable="true"
      @dragstart="dragStart(i)"
      @dragover.prevent="dragOver(i)"
      @dragleave="dragLeave"
      @drop.prevent="dropAt(i)"
      @click.stop="launch(it)"
      :title="it.name"
    >
      <img
        v-if="icons.has(it.path) && icons.get(it.path)"
        :src="icons.get(it.path)"
        class="h-7 w-7 pointer-events-none"
        draggable="false"
        alt=""
      />
      <span
        v-else
        class="flex h-7 w-7 items-center justify-center rounded-lg bg-[var(--aurora-field)] text-sm font-medium text-[var(--aurora-text)]"
        >{{ pad(it.name.slice(0, 1)) }}</span
      >
      <!-- 悬停 ✕ 删除(不触发启动:stop 掉点击) -->
      <button
        class="absolute -top-1.5 -right-1.5 hidden h-4 w-4 items-center justify-center rounded-full bg-[var(--aurora-danger)] text-[10px] leading-none text-white shadow group-hover:flex"
        :title="`移除 ${it.name}`"
        @click.stop="removeItem(it)"
      >
        ✕
      </button>
      <!-- 运行中指示点 -->
      <span
        v-if="running.has(it.path)"
        class="absolute bottom-0.5 h-1.5 w-1.5 rounded-full bg-[var(--aurora-success)]"
      ></span>
    </div>

    <!-- "+" 占位点击轻提示(添加只能走拖入,点击仅提示操作路径) -->
    <div
      v-if="addHint"
      class="pointer-events-none absolute -top-7 left-1/2 -translate-x-1/2 whitespace-nowrap rounded-md bg-[var(--aurora-panel)] border border-[var(--aurora-border)] px-3 py-1 text-xs text-[var(--aurora-text)] shadow-lg"
    >
      从资源管理器拖拽 exe/快捷方式到这里即可添加
    </div>

    <!-- 末尾固定 "+" 占位:有图标时可见,半透明提示可继续添加 -->
    <button
      v-if="items.length > 0"
      class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-dashed border-[var(--aurora-border)] text-lg text-[var(--aurora-text-dim)]/50 transition-colors hover:border-[var(--aurora-accent)] hover:text-[var(--aurora-accent)]/70"
      title="从资源管理器拖拽 exe/快捷方式到这里即可添加"
      @click="showAddHint"
    >
      +
    </button>

    <div
      v-if="items.length === 0"
      class="text-[10px] text-[var(--aurora-text-dim)] select-none"
      :class="fileDragOver ? 'text-[var(--aurora-accent)]' : ''"
    >
      拖拽应用到这里
    </div>
  </div>
</template>

<style scoped>
/* 启动中反馈:呼吸脉冲(点击后 1.5s,告知"正在启动",防用户急着连点) */
@keyframes aurora-launch-pulse {
  0%,
  100% {
    opacity: 0.55;
    transform: scale(0.94);
  }
  50% {
    opacity: 1;
    transform: scale(1);
  }
}
.dock-launching {
  animation: aurora-launch-pulse 0.8s ease-in-out infinite;
}
</style>
