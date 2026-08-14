<script setup lang="ts">
// 岛内 Dock(Phase6 Task 3,设计文档 §3.2):旧 Dock.vue 逻辑迁入岛的展开区。
// - 34px 图标瓦片(真实图标 dock_get_icon)/运行绿点(2s 轮询 dock_get_running)/
//   悬停 ✕ 删除徽章(dock_set_items 移除)/点击启动(dock_launch + 本地 launching 防连点
//   900ms + 脉冲 class,到期 emit("launched") 通知父组件收回岛)。
// - ＋ 虚线按钮:点击轻提示"拖拽应用进岛添加"(添加走拖入,由父组件 Island 处理 onDragDropEvent)。
// - 溢出:条目超过可视容量 → 渲染前 N 个 + 「…」按钮,点击弹浮层列全条目(浮层内可点启动/✕);
//   浮层 Teleport 出药丸(药丸 overflow:hidden 会裁剪)并用 .aurora-panel 玻璃样式,
//   弹出时临时加高窗口容纳(46px 药丸容不下,关闭恢复 —— 见错误记录 2026-08-11 弹层超窗裁剪)。
// - 可视容量动态计算(2026-08-14 真机观察项):静态 9 高于实际可视(~6 瓦片,378 药丸
//   左段时间/状态占位后剩 ~270px),第 7~9 个被 overflow:hidden 裁剪且不进「…」浮层直接
//   丢失;改为 ResizeObserver 按 mini-dock 实测宽度算容量(34px 瓦片 + 4px gap,预留
//   ＋ 与「…」按钮位),左段内容宽窄变化/DPI 自适应。
// - 图标预热:本组件随 Island 挂载(应用启动即挂载,不 v-if 懒加载)即后台发起全部
//   dock_get_icon(后端 LNK_TARGET_CACHE + 磁盘缓存幂等),用户展开岛时图标零延迟(设计文档 §6)。
// - 与旧 Dock 差异:去掉 HTML5 内部排序(设计稿无此交互);窗口常驻可见不暂停轮询。
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import AuroraIcon from "../icons/AuroraIcon.vue";

interface DockItem {
  name: string;
  path: string;
}

const props = defineProps<{ expanded: boolean }>();
const emit = defineEmits<{
  (e: "hint", text: string): void; // 轻提示(父组件在岛窗口内渲染气泡,避免超窗裁剪)
  (e: "launched"): void; // 启动脉冲到期 → 岛收回
}>();

const win = getCurrentWindow();

const items = ref<DockItem[]>([]);
const icons = ref<Map<string, string>>(new Map());
const running = ref<Set<string>>(new Set());

/** 溢出:条目超过可视容量 → 渲染前 N + 「…」浮层列全条目。
    N 由 mini-dock 实测宽度动态算出(见 recomputeVisible),默认 6 兜底首帧 */
const dockEl = ref<HTMLElement | null>(null);
const visibleCount = ref(6);
const overflow = computed(() => items.value.length > visibleCount.value);
const visibleItems = computed(() => items.value.slice(0, visibleCount.value));
/** 瓦片 34px + 间距 4px 的排布步长 */
const TILE_STEP = 38;

let dockResize: ResizeObserver | undefined;

/** 按当前宽度计算可视瓦片数:
    - 无溢出:瓦片 + 尾部 ＋ 按钮 → N ≤ (w-34)/38
    - 有溢出:额外腾出「…」按钮位 → N ≤ (w-72)/38,保底 1 */
function recomputeVisible() {
  const el = dockEl.value;
  if (!el) return;
  const w = el.clientWidth;
  const withoutMore = Math.max(1, Math.floor((w - 34) / TILE_STEP));
  const n =
    items.value.length <= withoutMore
      ? withoutMore
      : Math.max(1, Math.floor((w - 72) / TILE_STEP));
  if (n !== visibleCount.value) visibleCount.value = n;
}

/** 启动中集合:本地防连点(后端 dock_launch 亦有 LAUNCHING 防抖,双保险) */
const launching = ref<Set<string>>(new Set());
const LAUNCHING_MS = 900;

/** 溢出浮层 */
const overflowOpen = ref(false);
const OVERFLOW_ROW_H = 40;
const OVERFLOW_MAX_ROWS = 7;

let runningTimer: number | undefined;

/** 条目名(去 .exe/.lnk 后缀) */
function nameOf(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? path;
  return base.replace(/\.(lnk|exe)$/i, "");
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

/** 添加条目(去重;写后端持久化 + 后台取图标)。返回实际新增数(0 = 已存在) */
async function addPaths(paths: string[]): Promise<number> {
  const fresh = paths.filter((p) => !items.value.some((it) => it.path === p));
  if (fresh.length === 0) return 0;
  const next = [...items.value, ...fresh.map((p) => ({ name: nameOf(p), path: p }))];
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
    for (const p of fresh) void iconOf(p);
    return fresh.length;
  } catch (e) {
    console.error("dock_set_items failed", e);
    return 0;
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

/** 点击:运行中 → dock_launch 聚焦;未运行 → 启动。脉冲反馈到期后通知父组件收回岛 */
async function launch(item: DockItem) {
  if (launching.value.has(item.path)) return; // 启动中,忽略连点
  launching.value.add(item.path);
  window.setTimeout(() => {
    launching.value.delete(item.path);
    emit("launched");
  }, LAUNCHING_MS);
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
    // 图标预热:挂载即后台发起全部提取(后端缓存幂等),展开零延迟
    for (const it of items.value) void iconOf(it.path);
  } catch (e) {
    console.error("dock_get_items failed", e);
  }
}

// ---- 溢出浮层:临时加高窗口(逻辑像素宽读当前实际值,不依赖父组件动画态) ----

async function setWindowExtraHeight(extra: number) {
  try {
    const [size, sf] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    const w = Math.round(size.width / sf);
    await win.setSize(new LogicalSize(w, 46 + extra));
  } catch (e) {
    console.error("island dock setSize failed", e);
  }
}

function openOverflow() {
  overflowOpen.value = true;
  const extra = Math.min(items.value.length, OVERFLOW_MAX_ROWS) * OVERFLOW_ROW_H + 28;
  void setWindowExtraHeight(extra);
}

function closeOverflow() {
  if (!overflowOpen.value) return;
  overflowOpen.value = false;
  void setWindowExtraHeight(0);
}

function toggleOverflow() {
  if (overflowOpen.value) closeOverflow();
  else openOverflow();
}

/** 岛收起时同步关闭浮层(父组件 expanded prop 驱动;浮层只在展开态可达) */
watch(
  () => props.expanded,
  (v) => {
    if (!v) closeOverflow();
  },
);

/** 浮层外点击关闭(浮层 Teleport 到 body,用 document 级 mousedown 判定) */
function onDocMouseDown(e: MouseEvent) {
  if (!overflowOpen.value) return;
  const t = e.target as HTMLElement | null;
  if (!t || (!t.closest(".dock-overflow") && !t.closest(".dock-more"))) closeOverflow();
}

/** 占位首字符(与旧 Dock 同款:单字补空格避免字符宽度跳动) */
function pad(s: string) {
  return s.length > 1 ? s : s + " ";
}

function onAddClick() {
  emit("hint", "拖拽应用(.exe / .lnk)进岛即可添加");
}

onMounted(async () => {
  await loadItems();
  await pollRunning();
  runningTimer = window.setInterval(pollRunning, 2000);
  document.addEventListener("mousedown", onDocMouseDown);
  // 可视容量跟随宽度变化(展开动画/左段内容宽窄变化/DPI 缩放)
  dockResize = new ResizeObserver(recomputeVisible);
  if (dockEl.value) dockResize.observe(dockEl.value);
});

onUnmounted(() => {
  if (runningTimer) window.clearInterval(runningTimer);
  document.removeEventListener("mousedown", onDocMouseDown);
  dockResize?.disconnect();
});

// items 增删后溢出判定变化(是否腾出「…」按钮位),重算容量
watch(items, recomputeVisible);

defineExpose({ addPaths });
</script>

<template>
  <!-- 括入区根容器:整窗可拖下显式禁拖(tauri 2.11 drag-region="false" 阻断祖先判定,
  保证瓦片点击/✕/＋ 交互不被拖动接管;与旧 Dock.vue 同款处理)。
  expanded class = 岛展开态样式联动(收起时隐藏,2026-08-14 真机反馈:此前收起态
  括入区无隐藏样式,被静态内容挤成 34px 宽仍可见,表现为"Dock 只有一个图标") -->
  <div ref="dockEl" class="mini-dock" :class="{ expanded }" data-tauri-drag-region="false">
    <button
      v-for="it in visibleItems"
      :key="it.path"
      class="dock-tile"
      :class="{ launching: launching.has(it.path) }"
      :title="it.name"
      @click.stop="launch(it)"
    >
      <span class="dock-ico">
        <img
          v-if="icons.has(it.path) && icons.get(it.path)"
          :src="icons.get(it.path)"
          class="dock-ico-img"
          draggable="false"
          alt=""
        />
        <span v-else class="dock-ico-fallback">{{ pad(it.name.slice(0, 1)) }}</span>
      </span>
      <!-- 运行中指示点 -->
      <span v-if="running.has(it.path)" class="running-dot"></span>
      <!-- 悬停 ✕ 删除(不触发启动:stop 掉点击) -->
      <span
        class="del-badge"
        :title="`移除 ${it.name}`"
        :aria-label="`移除 ${it.name}`"
        @click.stop="removeItem(it)"
      >
        <AuroraIcon name="close" :size="8" />
      </span>
    </button>

    <!-- 溢出:超可视容量显示前 N + 「…」,点击弹浮层列全条目 -->
    <button
      v-if="overflow"
      class="dock-tile dock-more"
      title="全部应用"
      aria-label="全部应用"
      @click.stop="toggleOverflow"
    >
      <span class="dock-more-dots">…</span>
    </button>

    <!-- ＋ 添加:点击轻提示拖拽方式(添加本身走拖入) -->
    <button
      v-if="items.length > 0"
      class="dock-tile dock-add-mini"
      title="拖拽应用进岛添加"
      aria-label="添加应用"
      @click.stop="onAddClick"
    >
      <AuroraIcon name="plus" :size="12" />
    </button>

    <span v-if="items.length === 0" class="dock-empty">拖拽应用到这里</span>
  </div>

  <!-- 溢出浮层:Teleport 出药丸避免 overflow:hidden 裁剪;窗口已临时加高容纳 -->
  <Teleport to="body">
    <div v-if="overflowOpen" class="dock-overflow aurora-panel">
      <button
        v-for="it in items"
        :key="it.path"
        class="dock-overflow-row"
        :class="{ launching: launching.has(it.path) }"
        :title="it.name"
        @click.stop="launch(it)"
      >
        <span class="dock-ico">
          <img
            v-if="icons.has(it.path) && icons.get(it.path)"
            :src="icons.get(it.path)"
            class="dock-ico-img"
            draggable="false"
            alt=""
          />
          <span v-else class="dock-ico-fallback">{{ pad(it.name.slice(0, 1)) }}</span>
        </span>
        <span class="dock-overflow-name">{{ it.name }}</span>
        <span v-if="running.has(it.path)" class="running-dot dock-overflow-dot"></span>
        <span
          class="del-badge dock-overflow-del"
          :title="`移除 ${it.name}`"
          :aria-label="`移除 ${it.name}`"
          @click.stop="removeItem(it)"
        >
          <AuroraIcon name="close" :size="8" />
        </span>
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
/* 括入区(设计稿 .island .mini-dock 移植):
   收起态隐藏(淡出+左移),展开态淡入;防挤压——静态内容(时间/状态)占满窄岛时
   不把 Dock 区压成 34px 竖条(2026-08-14 真机反馈) */
.mini-dock {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  min-width: 0;
  flex-shrink: 0;
  opacity: 0;
  transform: translateX(-8px);
  transition:
    opacity 0.22s ease 0.08s,
    transform 0.22s ease 0.08s;
  pointer-events: none;
}
.mini-dock.expanded {
  opacity: 1;
  transform: translateX(0);
  pointer-events: auto;
}

/* 图标瓦片(设计稿 .dock-tile 移植,令牌换 --aurora-*) */
.dock-tile {
  position: relative;
  width: 34px;
  height: 34px;
  flex: none;
  display: grid;
  place-items: center;
  border: none;
  border-radius: 10px;
  background: transparent;
  color: var(--aurora-text);
  cursor: pointer;
  transition:
    transform 0.15s cubic-bezier(0.22, 1, 0.36, 1),
    background 0.14s ease;
}
.dock-tile:hover {
  transform: translateY(-2px) scale(1.08);
  background: var(--aurora-field);
}
.dock-tile:active {
  transform: scale(0.92);
}

.dock-ico {
  position: relative;
  width: 24px;
  height: 24px;
  display: grid;
  place-items: center;
  pointer-events: none;
}
.dock-ico-img {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  display: block;
}
.dock-ico-fallback {
  width: 24px;
  height: 24px;
  display: grid;
  place-items: center;
  border-radius: 6px;
  background: var(--aurora-field);
  font-size: 11px;
  font-weight: 600;
  color: var(--aurora-text);
}

.dock-empty {
  font-size: 11px;
  color: var(--aurora-text-dim);
  white-space: nowrap;
}

.running-dot {
  position: absolute;
  bottom: 1px;
  left: 50%;
  transform: translateX(-50%);
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--aurora-success);
  box-shadow: 0 0 5px var(--aurora-success);
  pointer-events: none;
}

/* 悬停 ✕ 删除徽章(设计稿 .del-badge 移植) */
.del-badge {
  position: absolute;
  top: -3px;
  right: -3px;
  width: 15px;
  height: 15px;
  display: none;
  place-items: center;
  border-radius: 99px;
  background: var(--aurora-danger);
  color: #fff;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
}
.dock-tile:hover .del-badge {
  display: grid;
}
.del-badge:hover {
  background: color-mix(in srgb, var(--aurora-danger) 80%, #000);
}

/* ＋ 虚线按钮(设计稿 .dock-add-mini 移植) */
.dock-add-mini {
  border: 1.5px dashed var(--aurora-border) !important;
  color: var(--aurora-text-dim);
  font-size: 16px;
}
.dock-add-mini:hover {
  color: var(--aurora-accent);
  border-color: var(--aurora-accent) !important;
  background: transparent !important;
  transform: none !important;
}

/* 「…」溢出按钮 */
.dock-more-dots {
  font-size: 15px;
  letter-spacing: 2px;
  color: var(--aurora-text-dim);
  transform: translateY(-2px);
}
.dock-more:hover .dock-more-dots {
  color: var(--aurora-text);
}

/* 启动脉冲(设计稿 launch-pulse:0.7s × 2 呼吸缩放) */
@keyframes dock-launch-pulse {
  0%,
  100% {
    transform: scale(1);
    filter: brightness(1);
  }
  50% {
    transform: scale(0.88);
    filter: brightness(1.5);
  }
}
.dock-tile.launching {
  animation: dock-launch-pulse 0.7s ease-in-out 2;
}

/* ---- 溢出浮层(Teleport 到 body,固定定位贴药丸下缘右侧) ---- */
.dock-overflow {
  position: fixed;
  top: 40px;
  right: 12px;
  width: 248px;
  max-height: 300px;
  overflow-y: auto;
  padding: 6px;
  z-index: 60;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.dock-overflow-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 40px;
  flex: none;
  padding: 0 8px;
  border: none;
  border-radius: 9px;
  background: transparent;
  color: var(--aurora-text);
  font-family: inherit;
  font-size: 13px;
  cursor: pointer;
  text-align: left;
  transition: background 0.12s ease;
}
.dock-overflow-row:hover {
  background: var(--aurora-field);
}
.dock-overflow-row.launching {
  animation: dock-launch-pulse 0.7s ease-in-out 2;
}
.dock-overflow-name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 行内形态的绿点/删除徽章(覆盖瓦片上的 absolute 定位) */
.dock-overflow-dot {
  position: static;
  transform: none;
  flex: none;
}
.dock-overflow-del {
  position: static;
  flex: none;
}
.dock-overflow-row:hover .dock-overflow-del {
  display: grid;
}
</style>
