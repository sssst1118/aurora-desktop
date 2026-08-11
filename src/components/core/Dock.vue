<script setup lang="ts">
// Dock 栏组件(Phase2 2.1) — 仅组件与样式,挂载接线待集成收尾:
// 集成 agent 把本组件挂进 label="dock" 的 800×64 窗口即可;命令在 dock.rs 已实现。
// 自动隐藏由后端线程处理(dock.rs,GetCursorPos 200ms + 1.5s 离开隐藏),本组件无需干预。
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, currentMonitor, PhysicalPosition } from "@tauri-apps/api/window";

interface DockItem {
  name: string;
  path: string;
}

interface AppEntry {
  name: string;
  path: string;
}

/** 本组件用到的 config 子集(config_load 返回完整对象,多取无妨) */
interface ConfigSubset {
  dock_items: DockItem[];
  dock_position: string;
  dock_auto_hide: boolean;
}

const items = ref<DockItem[]>([]);
const icons = ref<Map<string, string>>(new Map());
const running = ref<Set<string>>(new Set());
const position = ref<"top" | "bottom">("bottom");
const autoHide = ref(true);

// 右键菜单与添加面板
const menu = ref<{ x: number; y: number; item: DockItem | null } | null>(null);
const addOpen = ref(false);
const addQuery = ref("");
const addResults = ref<AppEntry[]>([]);

// DnD 排序:拖拽源下标 + 悬停目标下标(用于高亮)
const dragIdx = ref(-1);
const overIdx = ref(-1);

let runningTimer: number | undefined;
let cfgTimer: number | undefined;

function pad(s: string) {
  return s.length > 1 ? s : s + " ";
}

/** 加载条目(后端内存缓存,首次自动从 config 读) */
async function loadItems() {
  try {
    items.value = (await invoke<DockItem[]>("dock_get_items")) ?? [];
    void refreshIcons();
  } catch (e) {
    console.error("dock_get_items failed", e);
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

/** 3s 轮询位置/自动隐藏配置(后端 auto-hide 线程每 2s 重读 config,写配置即时生效) */
async function pollConfig() {
  try {
    const cfg = (await invoke<ConfigSubset>("config_load")) as ConfigSubset;
    const p = cfg.dock_position === "top" ? "top" : "bottom";
    if (p !== position.value) {
      position.value = p;
      void applyPosition();
    }
    if (typeof cfg.dock_auto_hide === "boolean") autoHide.value = cfg.dock_auto_hide;
  } catch (e) {
    console.error("config_load failed", e);
  }
}

/** 按 dock_position 把窗口摆到对应边缘中央(物理坐标,多显示器取当前屏) */
async function applyPosition() {
  try {
    const mon = await currentMonitor();
    if (!mon) return;
    const win = getCurrentWindow();
    const outer = await win.outerSize();
    const x = Math.round(mon.position.x + (mon.size.width - outer.width) / 2);
    const y =
      position.value === "top"
        ? mon.position.y
        : mon.position.y + mon.size.height - outer.height;
    await win.setPosition(new PhysicalPosition(x, y));
  } catch (e) {
    console.error("setPosition failed", e);
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

/** 点击:运行中 → dock_launch 聚焦;未运行 → 启动 */
async function launch(item: DockItem) {
  try {
    await invoke<boolean>("dock_launch", { item });
    void pollRunning();
  } catch (e) {
    console.error("dock_launch failed", e);
  }
}

/** 右键菜单 */
function openMenu(ev: MouseEvent, item: DockItem) {
  menu.value = {
    x: Math.min(ev.clientX, window.innerWidth - 170),
    y: ev.clientY,
    item,
  };
  addOpen.value = false;
}

function closeMenu() {
  menu.value = null;
  addOpen.value = false;
  addQuery.value = "";
  addResults.value = [];
}

/** 移除条目 */
async function removeItem(item: DockItem) {
  const next = items.value.filter((it) => it.path !== item.path);
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
  } catch (e) {
    console.error("dock_set_items failed", e);
  }
  closeMenu();
}

/** 添加面板:首次打开扫全量,输入后模糊搜索(复用 Phase1 search_apps) */
async function openAdd() {
  addOpen.value = true;
  addQuery.value = "";
  await searchApps("");
}

async function searchApps(q: string) {
  try {
    addResults.value = (await invoke<AppEntry[]>("search_apps", { query: q })) ?? [];
  } catch (e) {
    console.error("search_apps failed", e);
    addResults.value = [];
  }
}

async function addApp(entry: AppEntry) {
  const next = [...items.value, { name: entry.name, path: entry.path }];
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
  } catch (e) {
    console.error("dock_set_items failed", e);
  }
  closeMenu();
}

/** 切换顶部/底部:写 config + 立即 reposition */
async function togglePosition() {
  const next = position.value === "top" ? "bottom" : "top";
  position.value = next;
  try {
    const cfg = (await invoke<ConfigSubset>("config_load")) as ConfigSubset;
    cfg.dock_position = next;
    await invoke<boolean>("config_save", { cfg });
  } catch (e) {
    console.error("config_save failed", e);
  }
  void applyPosition();
  closeMenu();
}

/** 自动隐藏开关(后端线程 2s 内感知) */
async function toggleAutoHide() {
  autoHide.value = !autoHide.value;
  try {
    const cfg = (await invoke<ConfigSubset>("config_load")) as ConfigSubset;
    cfg.dock_auto_hide = autoHide.value;
    await invoke<boolean>("config_save", { cfg });
  } catch (e) {
    console.error("config_save failed", e);
  }
  closeMenu();
}

// ---- DnD 排序(拖拽高亮目标位,松手写回) ----

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

onMounted(async () => {
  await loadItems();
  await pollRunning();
  await pollConfig();
  runningTimer = window.setInterval(pollRunning, 2000);
  cfgTimer = window.setInterval(pollConfig, 3000);
});

onUnmounted(() => {
  if (runningTimer) window.clearInterval(runningTimer);
  if (cfgTimer) window.clearInterval(cfgTimer);
});
</script>

<template>
  <div
    class="relative h-full w-full flex items-center gap-1 px-2 select-none bg-black/40 backdrop-blur-md"
    @click="closeMenu"
    @contextmenu.prevent
  >
    <div
      v-for="(it, i) in items"
      :key="it.path"
      class="group relative flex h-12 w-12 items-center justify-center rounded-xl transition-colors"
      :class="[
        i === overIdx && dragIdx >= 0 ? 'bg-white/20' : 'hover:bg-white/10',
        dragIdx === i ? 'opacity-40' : '',
      ]"
      :draggable="true"
      @dragstart="dragStart(i)"
      @dragover.prevent="dragOver(i)"
      @dragleave="dragLeave"
      @drop.prevent="dropAt(i)"
      @click.stop="launch(it)"
      @contextmenu.stop="openMenu($event, it)"
      :title="it.name"
    >
      <img
        v-if="icons.has(it.path) && icons.get(it.path)"
        :src="icons.get(it.path)"
        class="h-9 w-9 pointer-events-none"
        draggable="false"
        alt=""
      />
      <span
        v-else
        class="flex h-9 w-9 items-center justify-center rounded-lg bg-white/10 text-sm font-medium text-white/70"
        >{{ pad(it.name.slice(0, 1)) }}</span
      >
      <!-- 运行中指示点 -->
      <span
        v-if="running.has(it.path)"
        class="absolute bottom-1 h-1.5 w-1.5 rounded-full bg-green-400"
      ></span>
    </div>

    <div v-if="items.length === 0" class="text-xs text-white/40 select-none">
      右键添加应用
    </div>

    <!-- 右键菜单 -->
    <div
      v-if="menu"
      class="absolute z-10 w-40 overflow-hidden rounded-lg border border-white/10 bg-black/70 backdrop-blur-md text-xs text-white/90 shadow-xl"
      :class="position === 'top' ? 'top-full mt-1' : 'bottom-full mb-1'"
      :style="{ left: `${menu.x}px` }"
      @click.stop
    >
      <button class="block w-full px-3 py-2 text-left hover:bg-white/10" @click="openAdd">
        ＋ 添加应用
      </button>
      <button
        v-if="menu.item"
        class="block w-full px-3 py-2 text-left text-red-300 hover:bg-white/10"
        @click="removeItem(menu.item!)"
      >
        移除
      </button>
      <div class="my-1 border-t border-white/10"></div>
      <button class="block w-full px-3 py-2 text-left hover:bg-white/10" @click="togglePosition">
        {{ position === "top" ? "移到底部" : "移到顶部" }}
      </button>
      <button class="block w-full px-3 py-2 text-left hover:bg-white/10" @click="toggleAutoHide">
        {{ autoHide ? "自动隐藏:开" : "自动隐藏:关" }}
      </button>
    </div>

    <!-- 添加应用 mini 列表 -->
    <div
      v-if="addOpen"
      class="absolute left-1/2 z-10 w-72 -translate-x-1/2 overflow-hidden rounded-lg border border-white/10 bg-black/70 backdrop-blur-md text-xs text-white/90 shadow-xl"
      :class="position === 'top' ? 'top-full mt-1' : 'bottom-full mb-1'"
      @click.stop
    >
      <input
        v-model="addQuery"
        class="w-full border-b border-white/10 bg-transparent px-3 py-2 outline-none placeholder:text-white/30"
        placeholder="搜索应用…"
        @input="searchApps(addQuery)"
      />
      <div class="max-h-56 overflow-y-auto">
        <button
          v-for="a in addResults"
          :key="a.path"
          class="block w-full truncate px-3 py-1.5 text-left hover:bg-white/10"
          :title="a.path"
          @click="addApp(a)"
        >
          {{ a.name }}
        </button>
        <div v-if="addResults.length === 0" class="px-3 py-2 text-white/40">无匹配</div>
      </div>
    </div>
  </div>
</template>
