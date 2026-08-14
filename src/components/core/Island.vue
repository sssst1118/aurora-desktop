<script setup lang="ts">
// 灵动岛(Phase6 Task 3,设计文档 §3 + 设计稿 .island):药丸三态。
// - 常态:呼吸点 + 时间(1s tick)+ CPU/内存/网络(sys-status 事件订阅,后端 2s 采样广播)。
// - 单击空白处 240ms 定时(期间第二次 click = 双击)→ 双击 invoke("open_search") 呼出主面板
//   (后端 show_search_window 定位主面板于岛正下方 12px);单击到期 = 展开/收起。
// - 展开动画:窗口宽度 378↔648 用 rAF 步进 setSize(~280ms,每帧插值;仿 SearchBar resizeMove
//   的 rAF 节流思路),同时 .expanded class 驱动 CSS 内容过渡(括入区淡入)。
// - 拖动:根容器 data-tauri-drag-region(现版同款 bare 语义:直接命中根元素才拖,子元素照常
//   点击;按钮由 drag.js 自动豁免,Dock 区显式 ="false");click 时指针位移 >4px 判为拖动不触发单击。
// - 几何记忆:onMoved(tauri://move)防抖 600ms → invoke("island_save_geometry", {x,y}) 落盘
//   + emit("island-geometry", {x,y,w,h})(逻辑像素,主面板 Task 4 跟随消费,取 x+w/2 对齐)。
// - 拖放入岛:onDragDropEvent → .exe/.lnk 调 Dock addPaths 追加(去重)并轻提示;
//   非目标类型忽略+提示;常态拖入先展开再接收(enter 携带有效路径即展开,drop 兜底)。
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { listen, emit, type UnlistenFn } from "@tauri-apps/api/event";
import IslandDock from "./IslandDock.vue";
import AuroraIcon from "../icons/AuroraIcon.vue";
import { apply_theme, apply_panel_style } from "../../theme";

interface SysStatus {
  cpu: number;
  mem_used_mb: number;
  mem_total_mb: number;
  /** 聚合接收速率 bytes/s */
  net_rx_bps: number;
  /** 聚合发送速率 bytes/s */
  net_tx_bps: number;
}

const win = getCurrentWindow();

// ---- 窗口尺寸(逻辑像素;tauri.conf island 378×46,展开 648×46) ----
const W_COLLAPSED = 378;
const W_EXPANDED = 648;
const WINDOW_H = 46;
const EXPAND_MS = 280;
const DOUBLE_CLICK_MS = 240;
const MOVE_DEBOUNCE_MS = 600;

const expanded = ref(false);
/** 拖动态:临时禁毛玻璃防透明窗口拖动重绘闪烁(2026-08-14 真机反馈) */
const dragging = ref(false);
/** 动画期间实际窗口宽度(逻辑像素),下一次动画以它为起点 */
let curW = W_COLLAPSED;

// ---- 时间 / 系统状态(沿用旧岛逻辑;net 拆 rx/tx 双 b 展示对齐设计稿) ----
const timeStr = ref("");
const cpu = ref("--");
const mem = ref("");
const netRx = ref("");
const netTx = ref("");

let timeTimer: number | undefined;
let unlistenSys: UnlistenFn | undefined;
let unlistenCfg: UnlistenFn | undefined;

function pad(n: number) {
  return n.toString().padStart(2, "0");
}

function tickTime() {
  const d = new Date();
  timeStr.value = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** 字节/秒 → 紧凑速率(设计稿格式:12K / 1.2M,无 /s 后缀以适配 378px 药丸宽度) */
function formatRate(bps: number): string {
  if (bps >= 1024 * 1024) return `${(bps / 1024 / 1024).toFixed(1)}M`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(0)}K`;
  return `${bps}B`;
}

/** 应用后端快照(事件推送或 invoke 兜底均走这里;?? 0 兼容旧后端缺字段) */
function applyStatus(s: SysStatus) {
  cpu.value = `${Math.round(s.cpu)}%`;
  mem.value = `${(s.mem_used_mb / 1024).toFixed(1)}G`;
  netRx.value = `↓${formatRate(s.net_rx_bps ?? 0)}`;
  netTx.value = `↑${formatRate(s.net_tx_bps ?? 0)}`;
}

// ---- 展开/收起:窗口宽度 rAF 步进动画(~280ms;ease-out 近似设计文档 §5.4 曲线) ----

let animRaf = 0;
let animResolve: (() => void) | null = null;

function expandEase(t: number): number {
  // 近似 cubic-bezier(0.22, 1, 0.36, 1):先快后慢
  return 1 - Math.pow(1 - t, 3);
}

function setWidthAnimated(target: number): Promise<void> {
  // 新动画打断旧动画(释放等待者,避免 drop 流程的 await 悬挂)
  if (animRaf) {
    cancelAnimationFrame(animRaf);
    animRaf = 0;
    animResolve?.();
    animResolve = null;
  }
  return new Promise((resolve) => {
    const from = curW;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const finish = () => {
      animRaf = 0;
      animResolve = null;
      resolve();
    };
    if (reduced || Math.abs(target - from) < 1) {
      // 降级:瞬时切换
      curW = target;
      win
        .setSize(new LogicalSize(target, WINDOW_H))
        .catch((e) => console.error("island setSize failed", e));
      finish();
      return;
    }
    const start = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / EXPAND_MS);
      curW = Math.round(from + (target - from) * expandEase(t));
      win
        .setSize(new LogicalSize(curW, WINDOW_H))
        .catch((e) => console.error("island setSize failed", e));
      if (t < 1) {
        animRaf = requestAnimationFrame(step);
      } else {
        finish();
      }
    };
    animResolve = finish;
    animRaf = requestAnimationFrame(step);
  });
}

function toggleExpand() {
  expanded.value = !expanded.value;
  void setWidthAnimated(expanded.value ? W_EXPANDED : W_COLLAPSED);
}

let autoCollapseTimer: number | undefined;

/** 呼出主面板(open_search 为已有命令;双击时同步展开,设计文档 §3.1)。
    2026-08-14 真机反馈:双击后岛保持展开态,Dock 区禁拖导致"换个地方再双击就拖不动"
    → 面板弹出后岛自动收回(展开=看 Dock 的瞬态,呼出面板任务结束即恢复窄态,拖动能恢复) */
function openSearchPanel() {
  if (!expanded.value) {
    expanded.value = true;
    void setWidthAnimated(W_EXPANDED);
  }
  invoke("open_search").catch((e) => console.error("open_search failed", e));
  if (autoCollapseTimer) window.clearTimeout(autoCollapseTimer);
  autoCollapseTimer = window.setTimeout(() => {
    autoCollapseTimer = undefined;
    if (expanded.value) toggleExpand();
  }, 450);
}

// ---- 单击/双击判定(pointerdown 驱动,2026-08-14 真机反馈修复):
// 单击=展开/收起;240ms 内第二次按下=双击呼出主面板;按下后移动 >4px=拖动,取消单击。
// 改用 pointerdown 计数而非 click/dblclick 事件:data-tauri-drag-region 元素上
// 原生拖动会吞掉 click,导致"只有特定区域能双击"——按下事件不受影响,全岛判定一致。 ----

let clickTimer: number | undefined;
let downPos: { x: number; y: number } | null = null;
let lastDownAt = 0;
let dragEndTimer: number | undefined;

/** 标记拖动态(禁 blur 防闪烁);拖动结束由 pointerup 立即清 + 移动断流 150ms 兜底
    (系统拖动期间 DOM 可能收不到 pointerup,onMoved 断流兜底恢复毛玻璃) */
function markDragging() {
  if (!dragging.value) dragging.value = true;
  if (dragEndTimer) window.clearTimeout(dragEndTimer);
  dragEndTimer = window.setTimeout(() => {
    dragEndTimer = undefined;
    dragging.value = false;
  }, 150);
}

function onPointerDown(e: PointerEvent) {
  const target = e.target as HTMLElement | null;
  // 括入区(Dock 图标/✕/＋)与搜索入口的按下不参与岛判定(设计稿同)
  if (!target || target.closest(".mini-dock, .search-entry")) return;
  downPos = { x: e.clientX, y: e.clientY };
  const now = performance.now();
  if (now - lastDownAt < DOUBLE_CLICK_MS) {
    // 双击:呼出主面板(同时展开)
    lastDownAt = 0;
    downPos = null;
    if (clickTimer !== undefined) {
      window.clearTimeout(clickTimer);
      clickTimer = undefined;
    }
    openSearchPanel();
    return;
  }
  lastDownAt = now;
  clickTimer = window.setTimeout(() => {
    clickTimer = undefined;
    toggleExpand();
  }, DOUBLE_CLICK_MS);
}

/** 按下后移动 >4px 判定为拖动:取消 pending 单击(原生拖动接管) */
function onPointerMove(e: PointerEvent) {
  if (!downPos) return;
  if (Math.hypot(e.clientX - downPos.x, e.clientY - downPos.y) > 4) {
    downPos = null;
    if (clickTimer !== undefined) {
      window.clearTimeout(clickTimer);
      clickTimer = undefined;
    }
    markDragging();
  }
}

/** 松开立即恢复毛玻璃(系统拖动收不到 pointerup 时由 onMoved 断流定时器兜底) */
function onPointerUp() {
  if (dragging.value) {
    dragging.value = false;
    if (dragEndTimer) {
      window.clearTimeout(dragEndTimer);
      dragEndTimer = undefined;
    }
  }
}

// ---- 几何记忆 + 主面板跟随事件 ----

let unMoved: UnlistenFn | undefined;
let geometryTimer: number | undefined;

/** 读当前几何(innerPosition/innerSize 物理像素 → 逻辑像素),落盘 + 广播 island-geometry */
async function persistGeometry() {
  try {
    const [pos, size, sf] = await Promise.all([
      win.innerPosition(),
      win.innerSize(),
      win.scaleFactor(),
    ]);
    const x = Math.round(pos.x / sf);
    const y = Math.round(pos.y / sf);
    const w = Math.round(size.width / sf);
    const h = Math.round(size.height / sf);
    invoke("island_save_geometry", { x, y }).catch((e) =>
      console.error("island_save_geometry failed", e),
    );
    emit("island-geometry", { x, y, w, h }).catch((e) =>
      console.error("emit island-geometry failed", e),
    );
  } catch (e) {
    console.error("persistGeometry failed", e);
  }
}

/** 拖动/尺寸变化后防抖 600ms 落盘并通知主面板跟随 */
function schedulePersistGeometry() {
  if (geometryTimer) window.clearTimeout(geometryTimer);
  geometryTimer = window.setTimeout(() => {
    geometryTimer = undefined;
    void persistGeometry();
  }, MOVE_DEBOUNCE_MS);
}

// ---- 拖放入岛(Tauri 2 拦截 DOM 拖放,官方 onDragDropEvent 接管) ----

const dockRef = ref<InstanceType<typeof IslandDock> | null>(null);
const fileDragOver = ref(false);
const hint = ref("");
let hintTimer: number | undefined;
let dragLeaveTimer: number | undefined;

/** 仅接收应用类文件(exe/lnk) */
function isAppPath(p: string): boolean {
  return /\.(lnk|exe)$/i.test(p);
}

/** 轻提示气泡(岛窗口 46px 高,气泡渲染在药丸内部居中,避免超窗裁剪) */
function showHint(text: string) {
  hint.value = text;
  if (hintTimer) window.clearTimeout(hintTimer);
  hintTimer = window.setTimeout(() => {
    hint.value = "";
  }, 3000);
}

/** 拖拽悬停高亮 + 熄灭看门狗(enter/over 连续点亮;无 leave 断流也能熄灭) */
function markFileDrag() {
  fileDragOver.value = true;
  if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
  dragLeaveTimer = window.setTimeout(() => {
    fileDragOver.value = false;
  }, 300);
}

function clearFileDrag() {
  fileDragOver.value = false;
  if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
}

win.onDragDropEvent((event) => {
  const payload = event.payload as {
    type: string;
    paths?: string[];
    position?: { x: number; y: number };
  };
  if (payload.type === "enter") {
    // 常态拖入:enter 携带有效应用路径即先展开(露出投放区)再点亮高亮
    const apps = (payload.paths ?? []).filter(isAppPath);
    if (apps.length === 0) return;
    if (!expanded.value) {
      expanded.value = true;
      void setWidthAnimated(W_EXPANDED);
    }
    markFileDrag();
  } else if (payload.type === "over") {
    // 连续 over:enter 已点亮时续灯(over 事件不携带路径)
    if (fileDragOver.value) markFileDrag();
  } else if (payload.type === "leave") {
    clearFileDrag();
  } else if (payload.type === "drop") {
    clearFileDrag();
    const apps = (payload.paths ?? []).filter(isAppPath);
    if (apps.length === 0) {
      showHint("仅支持拖入应用(.exe / .lnk)");
      return;
    }
    void (async () => {
      // 常态拖入兜底:先展开再接收(窗口变宽露出投放区)
      if (!expanded.value) {
        expanded.value = true;
        await setWidthAnimated(W_EXPANDED);
      }
      const added = await dockRef.value?.addPaths(apps);
      showHint(added ? `已添加 ${added} 个应用到岛` : "应用已在岛内");
    })();
  }
});

/** Dock 应用启动脉冲到期:收回岛(设计稿 900ms 后收起) */
function onLaunched() {
  if (expanded.value) toggleExpand();
}

onMounted(async () => {
  tickTime();
  // 展开动画起点:以实际窗口宽度为准(配置 378;读取失败保留默认)
  try {
    const [size, sf] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    curW = Math.max(1, Math.round(size.width / sf));
  } catch {
    /* 保留 378 兜底 */
  }
  // 系统状态:后端 2s 采样线程广播,取代前端轮询(Phase2 2.5 机制)
  try {
    unlistenSys = await listen<SysStatus>("sys-status", (e) => applyStatus(e.payload));
  } catch (e) {
    console.error("listen sys-status failed", e);
  }
  // 首帧兜底:invoke 一次立即拿最近快照(同时幂等触发后端采样线程启动)
  try {
    applyStatus(await invoke<SysStatus>("sys_get_status"));
  } catch (e) {
    console.error("sys_get_status failed", e);
  }
  // 拖动结束(tauri://move)→ 防抖落盘 + 广播几何(主面板跟随)
  unMoved = await win.onMoved(() => schedulePersistGeometry());
  // Phase6 皮肤跨窗口热生效:设置页保存后后端全局广播 config-saved,
  // 岛窗口重载配置并应用主题(皮肤/强调色/显示方式即时生效,无需重启)
  try {
    unlistenCfg = await listen("config-saved", async () => {
      try {
        const cfg = await invoke<{
          theme_mode: string;
          theme_accent: string;
          skin?: string;
          search_style?: string;
        }>("config_load");
        apply_theme(cfg);
        apply_panel_style(cfg.search_style ?? "solid");
      } catch (e) {
        console.error("config reload for theme failed", e);
      }
    });
  } catch (e) {
    console.error("listen config-saved failed", e);
  }
  // 启动广播一次初始几何:落盘幂等(setup 已恢复的位置写回原值),主面板可得初始同步
  void persistGeometry();
  timeTimer = window.setInterval(tickTime, 1000);
});

onUnmounted(() => {
  if (timeTimer) window.clearInterval(timeTimer);
  if (clickTimer) window.clearTimeout(clickTimer);
  if (geometryTimer) window.clearTimeout(geometryTimer);
  if (hintTimer) window.clearTimeout(hintTimer);
  if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
  if (dragEndTimer) window.clearTimeout(dragEndTimer);
  if (autoCollapseTimer) window.clearTimeout(autoCollapseTimer);
  if (animRaf) cancelAnimationFrame(animRaf);
  unlistenSys?.();
  unMoved?.();
  unlistenCfg?.();
});
</script>

<template>
  <div
    class="island"
    :class="{ expanded, dragover: fileDragOver, dragging }"
    data-tauri-drag-region
    title="单击展开 Dock · 双击呼出主面板"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
  >
    <!-- 静态展示元素全部标 drag-region(bare 语义下命中它们也能拖动窗口,
         2026-08-14 真机反馈:此前只有根元素空白处能拖) -->
    <span class="pulse-dot" data-tauri-drag-region></span>
    <span class="time num" data-tauri-drag-region>{{ timeStr }}</span>
    <span class="stats" data-tauri-drag-region>
      <span>CPU <b class="num">{{ cpu }}</b></span>
      <span>内存 <b class="num">{{ mem }}</b></span>
      <span class="net"><b class="num">{{ netRx }}</b> <b class="num">{{ netTx }}</b></span>
    </span>
    <span class="divider" data-tauri-drag-region></span>
    <IslandDock
      ref="dockRef"
      :expanded="expanded"
      @hint="showHint"
      @launched="onLaunched"
    />
    <button
      class="search-entry"
      title="呼出主面板"
      aria-label="呼出主面板"
      @click.stop="openSearchPanel"
    >
      <AuroraIcon name="search" :size="15" />
    </button>
    <Transition name="hint-fade">
      <div v-if="hint" class="island-hint">{{ hint }}</div>
    </Transition>
  </div>
</template>

<style scoped>
/* 药丸(设计稿 .island 移植,令牌换 --aurora-*;宽度由窗口 setSize 驱动,不做 CSS 宽度过渡) */
.island {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  height: 46px;
  width: 100%;
  padding: 0 18px;
  border-radius: 999px;
  border: 1px solid var(--aurora-border);
  background: linear-gradient(180deg, var(--glass-hi), transparent 45%), var(--aurora-panel);
  backdrop-filter: blur(28px) saturate(165%);
  -webkit-backdrop-filter: blur(28px) saturate(165%);
  /* 2026-08-14 真机反馈:外阴影在透明矩形窗口内被裁剪,圆角药丸外露出
     "方块"残影 → 去掉外阴影,只保留内高光;层次靠边框+顶部极光带 */
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.07);
  cursor: pointer;
  user-select: none;
  overflow: hidden;
  /* 2026-08-14 真机反馈:四角可见 backdrop-filter 矩形模糊(圆角外仍生效,
     表现为"外面的方角+内部的圆角并存")→ clip-path 把模糊输出裁进圆角,四角干净 */
  clip-path: inset(0 round 999px);
}
/* 拖动中:透明窗口+毛玻璃每帧重绘闪烁(右侧明显)→ 临时实心面板禁 blur,静止恢复 */
.island.dragging {
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  background: var(--aurora-panel-solid);
}
.island:active {
  transform: scale(0.988);
}
/* 极光缘短带(签名元素:岛顶 14%~86% 短带,设计稿 .island::before) */
.island::before {
  content: "";
  position: absolute;
  top: 0;
  left: 14%;
  right: 14%;
  height: 1.5px;
  border-radius: 999px;
  background: linear-gradient(90deg, var(--aur-1), var(--aur-2), var(--aur-3));
  background-size: 200% 100%;
  animation: aurora-flow 9s linear infinite;
  opacity: 0.9;
  pointer-events: none;
}
/* 拖入有效应用文件悬停高亮(设计稿 .island.dragover;外描边在透明窗口会被裁,
   改边框变色提示) */
.island.dragover {
  border-color: var(--aurora-accent);
}

/* 呼吸点 */
.pulse-dot {
  width: 7px;
  height: 7px;
  flex: none;
  border-radius: 50%;
  background: var(--aurora-accent);
  box-shadow: 0 0 10px 1px var(--aurora-accent);
  animation: island-breathe 3.2s ease-in-out infinite;
}
@keyframes island-breathe {
  0%,
  100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.45;
    transform: scale(0.82);
  }
}

.time {
  font-size: 15px;
  font-weight: 650;
  letter-spacing: 0.02em;
  flex: none;
  color: var(--aurora-text);
}
.stats {
  display: flex;
  gap: 11px;
  font-size: 11px;
  color: var(--aurora-text-dim);
  flex: none;
  white-space: nowrap;
}
.stats b {
  font-weight: 600;
  color: var(--aurora-text);
}

.divider {
  width: 1px;
  height: 20px;
  flex: none;
  background: var(--aurora-border);
  opacity: 0;
  transition: opacity 0.2s ease 0.1s;
}

/* 括入的 Dock 区:常态隐藏,展开淡入(子组件根元素携带本组件 scope 属性,选择器可达) */
.mini-dock {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  min-width: 0;
  opacity: 0;
  transform: translateX(-8px);
  transition:
    opacity 0.22s ease 0.08s,
    transform 0.22s ease 0.08s;
  pointer-events: none;
}
.island.expanded .mini-dock {
  opacity: 1;
  transform: translateX(0);
  pointer-events: auto;
}
.island.expanded .divider {
  opacity: 1;
}

/* 展开态搜索入口(设计稿 .search-entry) */
.search-entry {
  flex: none;
  width: 34px;
  height: 34px;
  display: grid;
  place-items: center;
  border: none;
  border-radius: 10px;
  background: var(--aurora-field);
  color: var(--aurora-accent);
  cursor: pointer;
  opacity: 0;
  transition:
    opacity 0.22s ease 0.1s,
    background 0.14s ease;
  pointer-events: none;
}
.island.expanded .search-entry {
  opacity: 1;
  pointer-events: auto;
}
.search-entry:hover {
  background: var(--aurora-field-hover);
}

/* 轻提示气泡:药丸内部居中浮层(岛窗口 46px 高,超出窗口会被裁剪,故不下挂) */
.island-hint {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  padding: 4px 12px;
  border-radius: 999px;
  background: var(--aurora-panel-solid);
  border: 1px solid var(--aurora-border);
  color: var(--aurora-text);
  font-size: 11px;
  white-space: nowrap;
  pointer-events: none;
  z-index: 30;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.35);
}
.hint-fade-enter-active,
.hint-fade-leave-active {
  transition: opacity 0.18s ease;
}
.hint-fade-enter-from,
.hint-fade-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .island::before,
  .pulse-dot {
    animation: none;
  }
}
</style>
