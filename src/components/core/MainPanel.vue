<script setup lang="ts">
/**
 * Phase6 主面板壳(设计文档 §4):search 窗口根组件,五视图合一。
 * - 结构参照设计稿 .main-panel:aurora-panel 根(整窗可拖)+ header(标题/输入框二态
 *   + 5 个视图切换按钮)+ view-body(KeepAlive 缓存 4 视图,search 不缓存)+ footer(快捷键+grip)。
 * - 视图 id 契约:small-desktop | search | clipboard | ai | settings(后端 panel-open-view
 *   事件仅 drawer/clipboard/ai 三种,drawer 映射 small-desktop)。
 * - 打字即搜:窗口 keydown 按可见字符(排除修饰键/IME 组合/可输入元素焦点)→ 切 search 态并输入。
 * - Esc 两级:search 态清空回小桌面;非 search 态隐藏窗口。
 * - 位置跟随岛:监听 island-geometry(Task 3 岛拖动防抖后 emit,逻辑像素)→ 面板可见时
 *   水平居中于岛、垂直岛底 +12px,clamp 回工作区;面板自身拖动位置不落盘。
 * - 尺寸记忆:onResized 防抖 600ms → search_save_geometry(x/y 占位 0,后端 setup 不恢复位置)。
 * - 呼出重置:tauri://show 时无 pending view 则回默认视图小桌面并清空输入;pop-in 动画重放。
 */
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  LogicalPosition,
  LogicalSize,
  availableMonitors,
  getCurrentWindow,
} from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import AuroraIcon from "../icons/AuroraIcon.vue";
import SmallDesktopView from "./views/SmallDesktopView.vue";
import ClipboardView from "./views/ClipboardView.vue";
import AIView from "./views/AIView.vue";
import SearchView from "./views/SearchView.vue";
import Settings from "./Settings.vue";

/** 视图 id 契约(后端 panel-open-view 映射到此处) */
type ViewId = "small-desktop" | "search" | "clipboard" | "ai" | "settings";

/** header 视图切换按钮(顺序=设计稿:小桌面/搜索/剪贴板/AI/设置) */
const VIEW_BUTTONS: { id: ViewId; icon: string; title: string }[] = [
  { id: "small-desktop", icon: "drawer", title: "小桌面" },
  { id: "search", icon: "search", title: "搜索" },
  { id: "clipboard", icon: "clipboard", title: "剪贴板" },
  { id: "ai", icon: "ai", title: "AI 助手" },
  { id: "settings", icon: "settings", title: "设置" },
];

/** 视图标题(search 态显示输入框,无标题) */
const VIEW_TITLES: Record<ViewId, string> = {
  "small-desktop": "小桌面",
  search: "",
  clipboard: "剪贴板",
  ai: "AI 助手",
  settings: "设置",
};

/** KeepAlive 缓存名单(按组件 name 匹配;search 不缓存=每次进入全新挂载) */
const CACHED_VIEWS = ["SmallDesktopView", "ClipboardView", "AIView", "Settings"];

const win = getCurrentWindow();

const activeView = ref<ViewId>("small-desktop");
/** 搜索输入状态由壳持有(打字即搜写入、视图切换保留、呼出重置清空),SearchView 经 prop 消费 */
const query = ref("");
const inputEl = ref<HTMLInputElement | null>(null);
const rootEl = ref<HTMLDivElement | null>(null);

const viewTitle = computed(() => VIEW_TITLES[activeView.value]);

/**
 * 本次呼出是否携带目标视图(panel-open-view 事件在 show 之后 emit,但两事件跨通道、
 * 到达顺序不保证:pending 标记 + show 时读后即清,两种到达顺序都收敛到目标视图)。
 */
let pendingView: ViewId | null = null;
/** 面板当前可见状态(tauri://show / tauri://hide 驱动;位置跟随只在可见时执行) */
let visible = false;

const unlisteners: UnlistenFn[] = [];

/** 搜索视图打开条目成功(SearchView emit):关闭面板,与旧 SearchBar 打开后收窗行为一致 */
function onSearchOpen() {
  void win.hide();
}

function showView(id: ViewId, queryText?: string) {
  activeView.value = id;
  if (id === "search") {
    // 打字即搜:字符覆盖旧 query;点搜索图标:保留上次输入
    if (queryText !== undefined) query.value = queryText;
    void nextTick(() => inputEl.value?.focus());
  }
}

/** 面板呼出动画重放(pop-in;class 移除→强制 reflow→重加,让动画可重复触发) */
function replayPopIn() {
  const el = rootEl.value;
  if (!el) return;
  el.classList.remove("panel-pop");
  void el.offsetWidth;
  el.classList.add("panel-pop");
}

// ---- 窗口级键盘:Esc 两级 + 打字即搜 ----

function onWindowKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    if (activeView.value === "search") {
      // 一级:搜索态 → 清空输入退回小桌面(对标 Raycast,面板保持打开)
      query.value = "";
      showView("small-desktop");
    } else {
      // 二级:非搜索态 → 关闭面板
      void win.hide();
    }
    return;
  }
  // 打字即搜:可见字符且无修饰键且非 IME 组合且当前非 search 态;
  // 焦点在可输入元素(AI 输入框/剪贴板搜索框/设置输入框等)时不劫持,正常打字
  if (e.isComposing) return;
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key.length !== 1) return;
  if (activeView.value === "search") return;
  const t = e.target as HTMLElement | null;
  if (
    t &&
    (t.tagName === "INPUT" ||
      t.tagName === "TEXTAREA" ||
      t.tagName === "SELECT" ||
      t.isContentEditable)
  ) {
    return;
  }
  e.preventDefault();
  showView("search", e.key);
}

// ---- 位置跟随岛(island-geometry 事件,逻辑像素) ----

/** 枚举显示器工作区(物理像素 → 逻辑像素四元组) */
async function logicalMonitors(): Promise<{ x: number; y: number; w: number; h: number }[]> {
  const ms = await availableMonitors();
  return ms.map((m) => ({
    x: m.position.x / m.scaleFactor,
    y: m.position.y / m.scaleFactor,
    w: m.size.width / m.scaleFactor,
    h: m.size.height / m.scaleFactor,
  }));
}

/**
 * 面板定位:水平中心对齐岛中心,垂直岛底 +12px;
 * clamp 到岛所在显示器工作区内(面板完整可见,与设计稿 positionPanel 同语义)。
 */
async function followIsland(p: { x: number; y: number; w: number; h: number }) {
  try {
    const size = await win.outerSize();
    const sf = await win.scaleFactor();
    const w = size.width / sf;
    const h = size.height / sf;
    const monitors = await logicalMonitors();
    if (monitors.length === 0) return;
    // 选岛水平中心所在显示器(多屏时岛可能不在主屏)
    const cx = p.x + p.w / 2;
    const m =
      monitors.find((mm) => cx >= mm.x && cx < mm.x + mm.w) ?? monitors[0];
    const px = Math.min(Math.max(Math.round(p.x + p.w / 2 - w / 2), m.x), m.x + m.w - w);
    const py = Math.min(Math.max(Math.round(p.y + p.h + 12), m.y), m.y + m.h - h);
    await win.setPosition(new LogicalPosition(px, py));
  } catch (e) {
    console.error("follow island geometry failed", e);
  }
}

// ---- 尺寸记忆(位置不落盘) ----

let sizeTimer: number | undefined;

async function saveSize() {
  try {
    const size = await win.innerSize();
    const sf = await win.scaleFactor();
    await invoke("search_save_geometry", {
      x: 0,
      y: 0,
      w: size.width / sf,
      h: size.height / sf,
    });
  } catch (e) {
    console.error("search_save_geometry failed", e);
  }
}

function scheduleSaveSize() {
  if (sizeTimer) window.clearTimeout(sizeTimer);
  sizeTimer = window.setTimeout(() => {
    sizeTimer = undefined;
    void saveSize();
  }, 600);
}

// ---- 右下角缩放手柄(无边框窗口无系统 resize 边框,自绘手柄 + setSize;
//      移植自 SearchBar 的 rAF 节流模式,最小尺寸下限防布局挤崩) ----

let resizeState: { sx: number; sy: number; w: number; h: number } | null = null;
let lastDx = 0;
let lastDy = 0;
let resizeRaf = 0;

async function resizeStart(e: PointerEvent) {
  e.preventDefault();
  try {
    const size = await win.innerSize();
    const sf = await win.scaleFactor();
    resizeState = { sx: e.screenX, sy: e.screenY, w: size.width / sf, h: size.height / sf };
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
  if (resizeRaf) return;
  resizeRaf = requestAnimationFrame(applyResize);
}

function applyResize() {
  resizeRaf = 0;
  if (!resizeState) return;
  const w = Math.max(360, Math.round(resizeState.w + lastDx));
  const h = Math.max(260, Math.round(resizeState.h + lastDy));
  win.setSize(new LogicalSize(w, h)).catch((err) => console.error("setSize failed", err));
}

function resizeEnd() {
  resizeState = null;
  window.removeEventListener("pointermove", resizeMove);
  window.removeEventListener("pointerup", resizeEnd);
  scheduleSaveSize(); // 缩放结束落一次尺寸(防抖保存兜底)
}

onMounted(async () => {
  // 尺寸变化 → 防抖落盘(仅尺寸,位置不记忆;onMoved 不监听)
  unlisteners.push(await win.onResized(() => scheduleSaveSize()));

  // 窗口显示:恢复默认视图小桌面(除非本次呼出带目标视图)+ 清空输入 + 呼出动画
  unlisteners.push(
    await listen("tauri://show", () => {
      visible = true;
      const target = pendingView;
      pendingView = null; // 读后即清,防残留影响下次普通呼出
      if (target) {
        showView(target);
      } else {
        query.value = "";
        showView("small-desktop");
      }
      replayPopIn();
    }),
  );
  unlisteners.push(
    await listen("tauri://hide", () => {
      visible = false;
      // 清残留:panel-open-view 在 show 事件之后到达时,它的 pendingView 未被
      // show 消耗;若不清,下次普通呼出(双击岛/open_search)会误读残留切错视图
      pendingView = null;
    }),
  );

  // 热键/托盘呼出对应视图:后端在 show 之后 emit(仅面板呼出后有效,到达即切)
  unlisteners.push(
    await listen("panel-open-view", (ev) => {
      const v = (ev.payload as { view?: string } | null)?.view;
      const mapped: ViewId =
        v === "clipboard" ? "clipboard" : v === "ai" ? "ai" : "small-desktop";
      pendingView = mapped;
      showView(mapped);
    }),
  );

  // 岛拖动(防抖后)emit 的几何:面板可见时实时跟随
  unlisteners.push(
    await listen("island-geometry", (ev) => {
      const p = ev.payload as { x?: number; y?: number; w?: number; h?: number } | null;
      if (!p || typeof p.x !== "number" || typeof p.y !== "number") return;
      if (!visible) return;
      void followIsland({ x: p.x, y: p.y, w: p.w ?? 0, h: p.h ?? 0 });
    }),
  );

  window.addEventListener("keydown", onWindowKeydown);
});

onUnmounted(() => {
  unlisteners.forEach((u) => u());
  unlisteners.length = 0;
  window.removeEventListener("keydown", onWindowKeydown);
  if (sizeTimer) window.clearTimeout(sizeTimer);
});
</script>

<template>
  <!-- 整窗拖动:根容器 drag-region=true,点任意空白处直接拖动;
       INPUT/BUTTON/TEXTAREA 等可点击元素自动豁免;view-body 显式 false 保护滚动与交互 -->
  <div
    ref="rootEl"
    class="aurora-panel main-panel-root"
    data-tauri-drag-region="true"
  >
    <!-- header:标题/输入框二态互斥 + 五视图切换 -->
    <header class="main-head">
      <div class="head-left">
        <span v-show="activeView !== 'search'" class="head-title">{{ viewTitle }}</span>
        <input
          v-show="activeView === 'search'"
          ref="inputEl"
          v-model="query"
          class="head-input"
          placeholder="搜索应用、文件…"
        />
      </div>
      <div class="view-switch">
        <button
          v-for="b in VIEW_BUTTONS"
          :key="b.id"
          class="view-btn"
          :class="{ on: activeView === b.id }"
          :title="b.title"
          :aria-label="`切换到${b.title}视图`"
          @click="showView(b.id)"
        >
          <AuroraIcon :name="b.icon" :size="15" />
        </button>
      </div>
    </header>

    <!-- view-body:KeepAlive 缓存小桌面/剪贴板/AI/设置;search 不缓存(每次全新) -->
    <div class="view-body" data-tauri-drag-region="false">
      <KeepAlive :include="CACHED_VIEWS">
        <SmallDesktopView v-if="activeView === 'small-desktop'" />
        <SearchView
          v-else-if="activeView === 'search'"
          :query="query"
          @open="onSearchOpen"
        />
        <ClipboardView v-else-if="activeView === 'clipboard'" />
        <AIView v-else-if="activeView === 'ai'" @open-settings="showView('settings')" />
        <Settings v-else />
      </KeepAlive>
    </div>

    <!-- footer:快捷键速查 + 拖拽把手(整窗可拖,把手为提示) -->
    <footer class="main-foot">
      <div class="keys">
        <span class="grp"><kbd>↑</kbd><kbd>↓</kbd>选择</span>
        <span class="grp"><kbd>Enter</kbd>打开</span>
        <span class="grp"><kbd>Esc</kbd>关闭</span>
      </div>
      <span class="foot-note">
        <AuroraIcon name="grip" :size="11" />
        拖动窗口
      </span>
    </footer>

    <!-- 右下角缩放手柄(自绘;无边框窗口无系统 resize 边框) -->
    <div
      class="resize-handle"
      @pointerdown="resizeStart"
      title="拖动调整大小"
    >
      <span class="resize-corner"></span>
    </div>
  </div>
</template>

<style scoped>
/* 结构样式移植自设计稿 aurora-v02-preview.html(.main-panel/.main-head/.view-switch/
   .main-foot);背景/圆角/极光缘由 global.css 的 .aurora-panel 类提供 */
.main-panel-root {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  user-select: none;
}

.main-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px 10px 18px;
  border-bottom: 1px solid var(--aurora-border);
  flex: none;
}

.head-left {
  flex: 1;
  display: flex;
  align-items: center;
  min-width: 0;
}

.head-title {
  font-size: 13.5px;
  font-weight: 650;
  letter-spacing: 0.02em;
}

.head-input {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  font-family: inherit;
  font-size: 16.5px;
  font-weight: 480;
  color: var(--aurora-text);
  caret-color: var(--aurora-accent);
  letter-spacing: 0.01em;
}

.head-input::placeholder {
  color: var(--aurora-text-dim);
}

.view-switch {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: none;
}

.view-btn {
  position: relative;
  width: 32px;
  height: 32px;
  display: grid;
  place-items: center;
  border: none;
  border-radius: 9px;
  background: transparent;
  color: var(--aurora-text-dim);
  cursor: pointer;
  transition:
    background 0.14s ease,
    color 0.14s ease;
}

.view-btn:hover {
  background: var(--aurora-field);
  color: var(--aurora-text);
}

.view-btn.on {
  color: var(--aurora-accent);
  background: var(--aurora-field);
}

/* 当前视图高亮:底部 accent 短杠(压在 header 底边上) */
.view-btn.on::after {
  content: "";
  position: absolute;
  bottom: -11px;
  left: 50%;
  transform: translateX(-50%);
  width: 14px;
  height: 2px;
  border-radius: 99px;
  background: linear-gradient(90deg, var(--aur-1), var(--aur-2));
  box-shadow: 0 0 6px var(--aur-2);
}

.view-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.main-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 14px 9px;
  border-top: 1px solid var(--aurora-border);
  font-size: 10.5px;
  color: var(--aurora-text-dim);
  flex: none;
}

.keys {
  display: flex;
  align-items: center;
  gap: 9px;
}

.keys .grp {
  display: flex;
  align-items: center;
  gap: 3px;
}

kbd {
  display: inline-grid;
  place-items: center;
  min-width: 20px;
  height: 19px;
  padding: 0 6px;
  border-radius: 5px;
  border: 1px solid var(--aurora-border);
  background: var(--aurora-field);
  font-family: inherit;
  font-size: 10px;
  color: var(--aurora-text);
  box-shadow: 0 1px 0 rgba(0, 0, 0, 0.15);
}

.foot-note {
  display: flex;
  align-items: center;
  gap: 5px;
  opacity: 0.8;
}

.foot-note:hover {
  opacity: 1;
}

/* 呼出动画:pop-in(scale 0.96→1 + 上移,spring 曲线),tauri://show 时重放 */
.panel-pop {
  animation: pop-in 0.26s cubic-bezier(0.22, 1.28, 0.5, 1) both;
}

@keyframes pop-in {
  0% {
    opacity: 0;
    transform: scale(0.96) translateY(-8px);
  }
  60% {
    opacity: 1;
  }
  100% {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  .panel-pop {
    animation-duration: 0.01s;
  }
}

/* 右下角缩放手柄 */
.resize-handle {
  position: absolute;
  bottom: 0;
  right: 0;
  z-index: 20;
  width: 16px;
  height: 16px;
  cursor: nwse-resize;
  display: grid;
  place-items: end center;
}

.resize-corner {
  width: 10px;
  height: 10px;
  border-right: 2px solid var(--aurora-text-dim);
  border-bottom: 2px solid var(--aurora-text-dim);
  opacity: 0.5;
  pointer-events: none;
}
</style>
