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
// - 面板 Esc 递进收岛(设计文档 §3.1,2026-08-14 审计中项):expanded 翻转经 watch 统一
//   emit("island-expand-state") 广播,主面板据其判断 Esc 收岛还是关面板;面板发
//   "island-collapse-request" 事件(面板→岛,与 island-geometry 反向通道对称)请求收起。
import { ref, watch, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  availableMonitors,
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
} from "@tauri-apps/api/window";
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
/** 落盘防抖:写配置频率低,600ms 不变(审计项:与跟随节流拆成两条链路,互不重置) */
const MOVE_DEBOUNCE_MS = 600;
/** 拖动中跟随节流(设计文档 §4.2「岛被拖动且面板可见时,面板实时跟随」):
    80ms 固定间隔广播几何,拖动全程面板平滑跟随而不频繁触发 reposition */
const MOVE_FOLLOW_MS = 80;

const expanded = ref(false);
/** 拖动态:临时禁毛玻璃防透明窗口拖动重绘闪烁(2026-08-14 真机反馈) */
const dragging = ref(false);
/** 宽度动画进行中(双击判定门控:动画期间第二击按单击处理,不识别为双击,防连点收起被吞) */
let animating = false;
/** 动画期间实际窗口宽度(逻辑像素),下一次动画以它为起点 */
let curW = W_COLLAPSED;
/** 动画期间实际窗口高度(逻辑像素,初始 46)。宽度动画每帧 setSize 以它为高度而非常量
    WINDOW_H:展开动画中溢出浮层临时加高窗口(46+extra)时不被压回(审计项 2026-08-14) */
let curH = WINDOW_H;

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

async function setWidthAnimated(target: number): Promise<void> {
  // 新动画打断旧动画(释放等待者,避免 drop 流程的 await 悬挂)
  if (animRaf) {
    cancelAnimationFrame(animRaf);
    animRaf = 0;
    animResolve?.();
    animResolve = null;
  }
  animating = true;
  // 动画开始前记录当前实际高度(而非常量 WINDOW_H):展开动画期间溢出浮层临时加高
  // 窗口(46+extra)时,每帧 setSize 不再把高度写回 46(审计项:浮层被宽度动画拦腰裁)
  try {
    const [size, sf] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    curH = Math.max(1, Math.round(size.height / sf));
  } catch {
    /* 读取失败保留上次高度,不阻断动画 */
  }
  await new Promise<void>((resolve) => {
    const from = curW;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const finish = () => {
      animRaf = 0;
      animResolve = null;
      animating = false;
      resolve();
    };
    if (reduced || Math.abs(target - from) < 1) {
      // 降级:瞬时切换
      curW = target;
      win
        .setSize(new LogicalSize(target, curH))
        .catch((e) => console.error("island setSize failed", e));
      finish();
      return;
    }
    const start = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / EXPAND_MS);
      curW = Math.round(from + (target - from) * expandEase(t));
      win
        .setSize(new LogicalSize(curW, curH))
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

/** 展开前右缘越界补偿:系统拖动无 clamp、启动恢复的 clamp 按 378 宽计算,岛拖到
    屏幕右缘后展开(378→648 左锚点右扩)会把右端裁出屏。检测岛所在显示器右缘,
    超界先左移补偿(与宽度动画同帧起步,setPosition 先于 setSize 动画执行) */
async function ensureExpandFits(targetW: number) {
  if (targetW <= curW) return;
  try {
    const [pos, sf] = await Promise.all([win.innerPosition(), win.scaleFactor()]);
    const x = pos.x / sf;
    const y = pos.y / sf;
    const cx = x + curW / 2; // 岛中心 → 选岛所在显示器
    const ms = await availableMonitors();
    const m =
      ms.find((mm) => {
        const mx = mm.position.x / mm.scaleFactor;
        const mw = mm.size.width / mm.scaleFactor;
        return cx >= mx && cx < mx + mw;
      }) ?? ms[0];
    const mx = m.position.x / m.scaleFactor;
    const mw = m.size.width / m.scaleFactor;
    const over = x + targetW - (mx + mw);
    if (over > 0) {
      await win.setPosition(new LogicalPosition(Math.round(x - over), Math.round(y)));
    }
  } catch (e) {
    console.error("ensureExpandFits failed", e);
  }
}

/** 双击呼出后的自动收回定时器(openSearchPanel 设置;手动操作即取消,见 toggleExpand) */
let autoCollapseTimer: number | undefined;

function toggleExpand() {
  // 低5(2026-08-14 波次 4):手动收起/展开即取消 450ms 自动收回定时器——定时器与
  // 手动操作竞态时,到期会把用户刚展开/刚收起的岛反向弹回
  // (与 island-collapse-request 分支同款处理;定时器自身到期后已置 undefined,
  //  此处清的是「用户操作时仍挂着的」那个)
  if (autoCollapseTimer) {
    window.clearTimeout(autoCollapseTimer);
    autoCollapseTimer = undefined;
  }
  expanded.value = !expanded.value;
  void (async () => {
    // 收岛前先等 Dock 浮层加高恢复(审计中3,2026-08-18):closeOverflow 的
    // setSize(46) 与 setWidthAnimated 读 innerSize→curH 并发时,动画会把加高后
    // 的高度当 curH 逐帧写回,浮层已关但窗口高度残留 46+extra 透明区;
    // watch(expanded) 异步触发,须经 Dock 暴露的 onCollapse 显式等待高度链
    if (!expanded.value) await dockRef.value?.onCollapse();
    if (expanded.value) await ensureExpandFits(W_EXPANDED);
    await setWidthAnimated(expanded.value ? W_EXPANDED : W_COLLAPSED);
  })();
}

/** 呼出主面板(open_search 为已有命令;双击时同步展开,设计文档 §3.1)。
    2026-08-14 真机反馈:双击后岛保持展开态,Dock 区禁拖导致"换个地方再双击就拖不动"
    → 面板弹出后岛自动收回(展开=看 Dock 的瞬态,呼出面板任务结束即恢复窄态,拖动能恢复) */
function openSearchPanel() {
  if (!expanded.value) {
    expanded.value = true;
    void (async () => {
      await ensureExpandFits(W_EXPANDED);
      await setWidthAnimated(W_EXPANDED);
    })();
  }
  invoke("open_search").catch((e) => console.error("open_search failed", e));
  if (autoCollapseTimer) window.clearTimeout(autoCollapseTimer);
  autoCollapseTimer = window.setTimeout(() => {
    autoCollapseTimer = undefined;
    if (expanded.value) toggleExpand();
  }, 450);
}

// ---- 面板 Esc 递进收岛:展开状态广播 + 收岛请求(与 island-geometry 同构的纯事件通道) ----

let unlistenCollapse: UnlistenFn | undefined;

/** 所有 expanded 翻转点(单击 toggle/双击呼出/450ms 自动收回/Dock 启动脉冲/拖入展开)
    经 watch 统一广播,主面板据此判断 Esc 是收岛还是关面板(岛窗口 focus:false 收不到
    键盘事件,面板是独立窗口也收不到岛的 keydown——展开判据必须广播到面板侧) */
watch(expanded, (v) => {
  emit("island-expand-state", { expanded: v }).catch((e) =>
    console.error("emit island-expand-state failed", e),
  );
});

// ---- 单击/双击判定(pointerdown 驱动,2026-08-14 真机反馈修复):
// 单击=展开/收起;240ms 内第二次按下=双击呼出主面板;按下后移动 >4px=拖动,取消单击。
// 改用 pointerdown 计数而非 click/dblclick 事件:data-tauri-drag-region 元素上
// 原生拖动会吞掉 click,导致"只有特定区域能双击"——按下事件不受影响,全岛判定一致。 ----

let clickTimer: number | undefined;
let downPos: { x: number; y: number } | null = null;
let lastDownAt = 0;
let dragEndTimer: number | undefined;

/** 标记拖动态(禁 blur 防闪烁);拖动结束由 pointerup 立即清 + 移动断流 150ms 兜底。
    长拖续灯(2026-08-14 波次 4):系统拖动期间 DOM 收不到 pointermove/pointerup,
    onMoved(见 onMounted 注册处)每帧续灯,事件流断流(拖动结束)后此 150ms
    定时器兜底恢复毛玻璃 */
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
  // 展开动画进行中第二击按单击处理(审计项 2026-08-14:240ms 判定窗吞「快速连点收起」,
  // 动画期间单击展开中,第二击落窗内会被误判双击呼出面板)
  if (!animating && now - lastDownAt < DOUBLE_CLICK_MS) {
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
  // 取消上一击 pending 单击(双击判定被禁用时,连点的第二击接管为「单击」语义,
  // 不清的话上一击 timer 到期会多触发一次 toggleExpand,展开又被立即收起)
  if (clickTimer !== undefined) {
    window.clearTimeout(clickTimer);
    clickTimer = undefined;
  }
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
let followTimer: number | undefined;
let lastFollowAt = 0;

/** 读当前几何(innerPosition/innerSize 物理像素 → 逻辑像素),仅广播 island-geometry。
    拖动中 80ms 节流调用,供主面板实时跟随(设计文档 §4.2) */
async function emitGeometry() {
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
    emit("island-geometry", { x, y, w, h }).catch((e) =>
      console.error("emit island-geometry failed", e),
    );
  } catch (e) {
    console.error("emitGeometry failed", e);
  }
}

/** 读当前几何,落盘 + 广播(600ms 防抖后执行一次:写配置频率低,最终位置兜底一致) */
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

/** 拖动/尺寸变化:两条独立链路(审计项 2026-08-14:原 600ms 防抖里 emit 被拖动期间
    反复重置,面板全程不动、松手 600ms 才跳一次位)。
    - 跟随:80ms 节流(首帧立即 + 固定间隔 + 拖尾兜底),拖动全程面板实时跟随;
    - 落盘:600ms 防抖不变,写配置频率低。 */
function schedulePersistGeometry() {
  const now = performance.now();
  if (followTimer) {
    window.clearTimeout(followTimer);
    followTimer = undefined;
  }
  if (now - lastFollowAt >= MOVE_FOLLOW_MS) {
    lastFollowAt = now;
    void emitGeometry();
  } else {
    followTimer = window.setTimeout(() => {
      followTimer = undefined;
      lastFollowAt = performance.now();
      void emitGeometry();
    }, MOVE_FOLLOW_MS - (now - lastFollowAt));
  }
  if (geometryTimer) window.clearTimeout(geometryTimer);
  geometryTimer = window.setTimeout(() => {
    geometryTimer = undefined;
    void persistGeometry();
  }, MOVE_DEBOUNCE_MS);
}

// ---- 拖放入岛(Tauri 2 拦截 DOM 拖放,官方 onDragDropEvent 接管) ----

/** Dock 模块开关(设置页 enable_dock,热生效;false 时整个括入区不渲染,
    修复 2026-08-14 文档审计:此前无条件渲染,开关空操作) */
const dockEnabled = ref(true);

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
    if (!dockEnabled.value) return; // Dock 关闭:不展开不点亮
    // 常态拖入:enter 携带有效应用路径即先展开(露出投放区)再点亮高亮
    const apps = (payload.paths ?? []).filter(isAppPath);
    if (apps.length === 0) return;
    if (!expanded.value) {
      expanded.value = true;
      void (async () => {
        await ensureExpandFits(W_EXPANDED);
        await setWidthAnimated(W_EXPANDED);
      })();
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
    if (!dockEnabled.value) {
      showHint("Dock 已关闭,请在设置中开启后再拖入");
      return;
    }
    void (async () => {
      // 常态拖入兜底:先展开再接收(窗口变宽露出投放区)
      if (!expanded.value) {
        expanded.value = true;
        await setWidthAnimated(W_EXPANDED);
      }
      const added = await dockRef.value?.addPaths(apps);
      if (added) {
        showHint(`已添加 ${added} 个应用到岛`);
        return;
      }
      // 0 = 全部已存在 或 写盘失败:拉一次真实条目区分,失败提示具体原因
      // (参考剪贴板 deleteError 模式,2026-08-14 审计:此前一律提示"应用已在岛内"误导)
      try {
        const items = (await invoke<{ path: string }[]>("dock_get_items")) ?? [];
        // Windows 路径大小写不敏感:统一小写再比较,同路径不同大小写也应判为已存在
        // (2026-08-14 波次 3 审计:此前严格相等,大小写差异会被误判为新应用)
        const allExist = apps.every((p) =>
          items.some((it) => it.path.toLowerCase() === p.toLowerCase()),
        );
        showHint(allExist ? "应用已在岛内" : "添加失败:写入配置出错,请重试");
      } catch (e) {
        console.error("dock_get_items failed", e);
        showHint("添加失败:无法读取 Dock 状态");
      }
    })();
  }
});

/** Dock 应用启动脉冲到期:收回岛(设计稿 900ms 后收起) */
function onLaunched() {
  if (expanded.value) toggleExpand();
}

/** 配置热生效应用:皮肤/显示方式/Dock 开关(启动与 config-saved 各走一次;
    enable_dock 门控 IslandDock 渲染,设置页改动保存后即时生效) */
async function applyConfig() {
  try {
    const cfg = await invoke<{
      theme_mode: string;
      theme_accent: string;
      skin?: string;
      search_style?: string;
      enable_dock?: boolean;
    }>("config_load");
    apply_theme(cfg);
    apply_panel_style(cfg.search_style ?? "solid");
    if (typeof cfg.enable_dock === "boolean") dockEnabled.value = cfg.enable_dock;
  } catch (e) {
    console.error("config reload failed", e);
  }
}

onMounted(async () => {
  tickTime();
  // 展开动画起点:以实际窗口宽高为准(配置 378×46;读取失败保留默认)
  try {
    const [size, sf] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    curW = Math.max(1, Math.round(size.width / sf));
    curH = Math.max(1, Math.round(size.height / sf));
  } catch {
    /* 保留 378×46 兜底 */
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
  // 拖动结束(tauri://move)→ 跟随节流广播 + 防抖落盘(主面板跟随)
  // 长拖续灯(低1,2026-08-14 波次 4;2026-08-18 审计低4 核对注释与实现相符,
  // 仅措辞修正):系统拖动(drag-region)期间 DOM 收不到 pointermove,拖动态
  // (禁毛玻璃防闪烁)靠 onMoved 每帧续灯——markDragging 每次触发都重置其内部
  // 150ms 兜底定时器,拖动结束事件流断流后定时器到期自动熄灭。
  // 依据(按代码推断,tao 0.35 源码):拖动由 PostMessage(WM_NCLBUTTONDOWN,HTCAPTION)
  // 启动,DefWindowProc 模态拖动循环泵送消息期间,WM_WINDOWPOSCHANGED 每帧派发到
  // 子类窗口过程并同步 send_event(Moved) → tauri://move 持续触发(波次 1 高1 原
  // bug「onMoved 600ms 防抖被反复重置」同源旁证);程序性 set_position 被 tao 抑制
  // 不触发 Moved(波次 3 踩坑记录),故此续灯仅真实拖动生效,无程序移动副作用
  unMoved = await win.onMoved(() => {
    markDragging();
    schedulePersistGeometry();
  });
  // 皮肤/Dock 开关跨窗口热生效:设置页保存后后端全局广播 config-saved
  try {
    unlistenCfg = await listen("config-saved", applyConfig);
  } catch (e) {
    console.error("listen config-saved failed", e);
  }
  void applyConfig();
  // 启动只广播一次初始几何(主面板可得初始同步),不落盘:setup 的 setPosition 经
  // Windows 消息循环异步生效,前端此刻读 innerPosition 可能早于移动完成,会把
  // 未恢复到位的位置写进 config(漂移缺陷 2,2026-08-14 修复);
  // 落盘仅由用户拖动(onMoved → schedulePersistGeometry)驱动
  void emitGeometry();
  // 面板 Esc 二级收岛请求(面板→岛):展开才收起;同步清自动收回定时器,
  // 防 450ms 到期重复触发(if expanded 兜底存在,主动清更干净)
  try {
    unlistenCollapse = await listen("island-collapse-request", () => {
      if (!expanded.value) return;
      if (autoCollapseTimer) {
        window.clearTimeout(autoCollapseTimer);
        autoCollapseTimer = undefined;
      }
      toggleExpand();
    });
  } catch (e) {
    console.error("listen island-collapse-request failed", e);
  }
  // 初始展开状态广播(启动必收起态):面板晚于岛挂载时也能对齐,无需查询命令
  emit("island-expand-state", { expanded: false }).catch((e) =>
    console.error("emit island-expand-state initial failed", e),
  );
  timeTimer = window.setInterval(tickTime, 1000);
});

onUnmounted(() => {
  if (timeTimer) window.clearInterval(timeTimer);
  if (clickTimer) window.clearTimeout(clickTimer);
  if (geometryTimer) window.clearTimeout(geometryTimer);
  if (followTimer) window.clearTimeout(followTimer);
  if (hintTimer) window.clearTimeout(hintTimer);
  if (dragLeaveTimer) window.clearTimeout(dragLeaveTimer);
  if (dragEndTimer) window.clearTimeout(dragEndTimer);
  if (autoCollapseTimer) window.clearTimeout(autoCollapseTimer);
  if (animRaf) cancelAnimationFrame(animRaf);
  unlistenSys?.();
  unMoved?.();
  unlistenCfg?.();
  unlistenCollapse?.();
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
      v-if="dockEnabled"
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
  /* 2026-08-18 用户反馈:透明融合诉求下 1px 描边形成"框",删掉;
     层次仅靠内高光+顶部极光带(药丸背景色/模糊本身已提供与桌面区分度) */
  background: linear-gradient(180deg, var(--glass-hi), transparent 45%), var(--aurora-panel);
  backdrop-filter: blur(28px) saturate(165%);
  -webkit-backdrop-filter: blur(28px) saturate(165%);
  /* 2026-08-14 真机反馈:外阴影在透明矩形窗口内被裁剪,圆角药丸外露出
     "方块"残影 → 去掉外阴影,只保留内高光。
     内高光用 --aurora-text 低浓度自适应(2026-08-14 审计:硬编码白高光在拂晓
     浅色皮肤上失效,color-mix 随皮肤变深色低对比提亮) */
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--aurora-text) 7%, transparent);
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
   2026-08-18 常态描边已删,高亮改内圈发光,不依赖 border) */
.island.dragover {
  box-shadow:
    inset 0 0 0 1.5px var(--aurora-accent),
    inset 0 1px 0 color-mix(in srgb, var(--aurora-text) 7%, transparent);
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
  /* 气泡投影固定黑(投影=压暗通用语义);拂晓浅色皮肤下 0.35 略重但浮层层次仍合理 */
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
