<script setup lang="ts">
/**
 * Phase6 主面板壳(设计文档 §4):search 窗口根组件,五视图合一。
 * - 结构参照设计稿 .main-panel:aurora-panel 根(整窗可拖)+ header(标题/输入框二态
 *   + 5 个视图切换按钮)+ view-body(KeepAlive 缓存 4 视图,search 不缓存)+ footer(快捷键+grip)。
 * - 视图 id 契约:small-desktop | search | clipboard | ai | settings(后端 panel-open-view
 *   事件仅 drawer/clipboard/ai 三种,drawer 映射 small-desktop)。
 * - 打字即搜:窗口 keydown 按可见字符(排除修饰键/IME 组合/可输入元素焦点)→ 切 search 态并输入。
 * - Esc 三级(设计文档 §3.1「先收岛展开 → 再关主面板」,2026-08-14 审计中项):
 *   ① search 态清空回小桌面 ② 岛展开时请求岛收起(面板保持打开)③ 再按隐藏窗口。
 *   焦点在可输入元素(INPUT/TEXTAREA/SELECT/富文本)时 Esc 放行不劫持(取消输入,
 *   与打字即搜同一豁免原则)。
 *   岛展开状态来自岛 watch 广播的 island-expand-state 事件,收岛走反向事件
 *   island-collapse-request(纯事件通道,与 island-geometry 对称,无后端命令)。
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
import { listen, emit, type UnlistenFn } from "@tauri-apps/api/event";
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

/** 隐藏主面板(header ✕ 按钮;与 Esc 同级入口,2026-08-14 真机反馈) */
function hidePanel() {
  void win.hide();
}

const activeView = ref<ViewId>("small-desktop");
/** 岛当前展开状态(岛 watch expanded 广播;Esc 二级「收岛」的判据) */
const islandExpanded = ref(false);
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

/** 空态引导 chip(SearchView emit):词填入搜索输入框,query 变化经 SearchView
    的 watch 触发打字即搜链路;聚焦输入框便于继续编辑 */
function onFillQuery(text: string) {
  query.value = text;
  void nextTick(() => inputEl.value?.focus());
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

// ---- 窗口级键盘:Esc 三级 + 打字即搜 ----

/** 焦点是否在可输入元素(INPUT/TEXTAREA/SELECT/富文本)。
    打字即搜豁免与 Esc 判定的共用基础(2026-08-14 波次 3 引入);
    Esc 侧波次 4 已降级为「有值或 IME 组合中才放行」,见 onWindowKeydown */
function isTypingTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return (
    t.tagName === "INPUT" ||
    t.tagName === "TEXTAREA" ||
    t.tagName === "SELECT" ||
    t.isContentEditable
  );
}

/** 焦点可输入元素的当前值(输入控件取 .value,富文本取 textContent)。
    Esc 豁免降级判据:有值或 IME 组合中才放行(2026-08-14 波次 4 中1) */
function typingValue(t: EventTarget | null): string {
  if (!(t instanceof HTMLElement)) return "";
  if (
    t.tagName === "INPUT" ||
    t.tagName === "TEXTAREA" ||
    t.tagName === "SELECT"
  ) {
    return (t as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value ?? "";
  }
  if (t.isContentEditable) return t.textContent ?? "";
  return "";
}

/** 本地过滤类输入框判定:剪贴板 keyword 搜索框(视图内唯一 INPUT,placeholder 含
    「搜索」)。Esc 对这类输入执行与主搜索框同款的一级清空(审计中2,2026-08-18):
    波次 4「有值即放行」依赖 WebView 的 INPUT 有原生 Esc 取消动作,但实际没有,
    焦点在此类过滤框时按 Esc 完全无响应(不关面板不清输入),与主搜索框行为割裂 */
function isFilterInput(t: HTMLElement): boolean {
  return (
    activeView.value === "clipboard" &&
    t.tagName === "INPUT" &&
    (t.getAttribute("placeholder") ?? "").includes("搜索")
  );
}

/** 一级清空过滤类输入框(与主搜索框 query 清空同语义):置空 value 并派发 input
    事件,让组件内 v-model 同步(剪贴板 keyword 清空后历史列表恢复全量) */
function clearTypingValue(t: HTMLElement) {
  const input = t as HTMLInputElement;
  input.value = "";
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/** SELECT 下拉 Esc 放行的最近时间戳(performance.now,0 = 从未放行):
    下拉打开时放行原生 Esc 关闭下拉;页面无 API 探测下拉打开状态,以「放行后
    短窗口内再次 Esc = 下拉已关」近似,再按递进收岛/关面板(审计中2) */
let selectEscAt = 0;
const SELECT_ESC_REPEAT_MS = 400;

function onWindowKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    const typing = isTypingTarget(e.target);
    // 高1(2026-08-14 波次 4):search 态焦点在主搜索输入框 → 执行一级
    // (清空 query 回小桌面)。波次 3 的 isTypingTarget 豁免把主输入框一并放行,
    // 最常见路径(打字后按 Esc 清空回小桌面)退化成无动作;
    // IME 组合中仍放行原生 Esc 取消输入法组合
    if (typing && activeView.value === "search" && e.target === inputEl.value) {
      if (e.isComposing) return;
      e.preventDefault();
      query.value = "";
      showView("small-desktop");
      return;
    }
    // 中2(2026-08-18 波次 5):按输入框语义细分,替换波次 4「有值即放行」——
    // 放行依赖原生 Esc 取消动作,但 WebView 的 INPUT 对 Esc 无动作,焦点在
    // 剪贴板 keyword 等过滤框时 Esc 完全无响应(不关面板不清输入),与主搜索框割裂
    if (typing) {
      if (e.isComposing) return; // IME 组合中:放行原生 Esc 取消输入法组合
      const t = e.target as HTMLElement;
      if (t.tagName === "SELECT") {
        // SELECT:下拉打开时放行原生 Esc 关闭下拉;短窗口内再次 Esc 视为下拉
        // 已关,继续递进(收岛/关面板)——见 selectEscAt 注释
        const now = performance.now();
        if (now - selectEscAt > SELECT_ESC_REPEAT_MS) {
          selectEscAt = now;
          return;
        }
      } else if (isFilterInput(t)) {
        // 本地过滤类输入框(剪贴板 keyword):与主搜索框同款一级清空,面板保持打开
        e.preventDefault();
        clearTypingValue(t);
        return;
      } else if (typingValue(t) !== "") {
        // 正文输入(AI 消息)/设置配置输入:有值放行(取消输入是原生预期;配置输入
        // 不清空,置空会经 change 保存空值破坏配置);空值递进收岛/关面板
        return;
      }
      // SELECT 下拉已关 / 空值输入框:继续递进
    }
    e.preventDefault();
    if (activeView.value === "search") {
      // 一级:搜索态 → 清空输入退回小桌面(对标 Raycast,面板保持打开)
      query.value = "";
      showView("small-desktop");
    } else if (islandExpanded.value) {
      // 二级:岛展开 → 请求岛收起,面板保持打开(设计文档 §3.1 层级递进;
      // 焦点在面板、岛 focus:false 收不到 Esc,只能由面板经事件通道转达)。
      // 本地乐观置 false:岛 watch 广播会兜底同步,连按 Esc 不被 280ms 收岛动画卡出多一次按键
      islandExpanded.value = false;
      emit("island-collapse-request").catch((err) =>
        console.error("emit island-collapse-request failed", err),
      );
    } else {
      // 三级:岛已收起 → 关闭面板
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
  if (isTypingTarget(e.target)) return;
  // 低6(2026-08-14 波次 4):按钮聚焦时按 Space 激活是原生行为,打字即搜不得劫持
  // (与 SearchView 同款豁免;此前 Tab 到视图切换按钮按空格会被切到搜索视图)
  if (e.target instanceof HTMLButtonElement) return;
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
    // 窄显示器(m.w < 面板宽 w)时上限 m.x+m.w-w < 下限 m.x,Math.max 会产生
    // 下限大于上限的非法区间,右缘直接出屏(审计项 2026-08-14):下限取 min 兜底
    // 使下限=上限,面板右缘贴屏(左侧超屏),y 方向同理
    const loX = Math.min(m.x, m.x + m.w - w);
    const loY = Math.min(m.y, m.y + m.h - h);
    const px = Math.min(Math.max(Math.round(p.x + p.w / 2 - w / 2), loX), m.x + m.w - w);
    const py = Math.min(Math.max(Math.round(p.y + p.h + 12), loY), m.y + m.h - h);
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
//      移植自 SearchBar 的 rAF 节流模式,尺寸夹在 360×260 ~ 2 倍显示器之间,
//      与后端 clamp_search_size 同口径) ----

/** 缩放手柄 aria 播报(role="slider" 单值语义,主值取宽,valuetext 报 宽×高;
    onResized 同步当前逻辑尺寸;上限挂载时按显示器计算,下限固定 360,
    与后端 clamp_search_size 同口径——审计低5,2026-08-18) */
const handleA11y = ref({ w: 0, h: 0, maxW: 4096 });

/** 同步缩放手柄 aria-valuenow(当前逻辑尺寸) */
async function syncHandleA11y() {
  try {
    const [size, sf] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    handleA11y.value.w = Math.round(size.width / sf);
    handleA11y.value.h = Math.round(size.height / sf);
  } catch {
    /* 播报失败忽略,保留上次值 */
  }
}

let resizeState: {
  sx: number;
  sy: number;
  w: number;
  h: number;
  /** 尺寸上限(逻辑像素)= 显示器尺寸 × 2,与后端 clamp_search_size 对齐 */
  maxW: number;
  maxH: number;
} | null = null;
let lastDx = 0;
let lastDy = 0;
let resizeRaf = 0;

async function resizeStart(e: PointerEvent) {
  e.preventDefault();
  // 指针拖动开始:键盘期望值作废,下次按键重新读当前尺寸(防连按期望与真实尺寸脱节)
  keyResizeW = undefined;
  try {
    const [size, sf] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    // 上限与后端 clamp_search_size 同口径:全部显示器取最大宽/高 × 2;
    // 无显示器时后端回退 4096(2026-08-14 波次 3 审计:此前仅 Math.max 下限,
    // 拖大无上限,巨窗口 setSize 卡渲染且与后端落盘 clamp 不一致)
    const monitors = await logicalMonitors();
    const maxW = monitors.length ? Math.max(...monitors.map((m) => m.w)) * 2 : 4096;
    const maxH = monitors.length ? Math.max(...monitors.map((m) => m.h)) * 2 : 4096;
    resizeState = {
      sx: e.screenX,
      sy: e.screenY,
      w: size.width / sf,
      h: size.height / sf,
      maxW,
      maxH,
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
  if (resizeRaf) return;
  resizeRaf = requestAnimationFrame(applyResize);
}

function applyResize() {
  resizeRaf = 0;
  if (!resizeState) return;
  // 下限 360×260 与后端 clamp_search_size 的 MIN 一致;上限 2 倍显示器
  const w = Math.min(resizeState.maxW, Math.max(360, Math.round(resizeState.w + lastDx)));
  const h = Math.min(resizeState.maxH, Math.max(260, Math.round(resizeState.h + lastDy)));
  win.setSize(new LogicalSize(w, h)).catch((err) => console.error("setSize failed", err));
}

function resizeEnd() {
  resizeState = null;
  window.removeEventListener("pointermove", resizeMove);
  window.removeEventListener("pointerup", resizeEnd);
  scheduleSaveSize(); // 缩放结束落一次尺寸(防抖保存兜底)
}

/** 缩放手柄键盘调节步长(逻辑像素;右下角手柄方向语义:右/下放大,左/上缩小) */
const RESIZE_KEY_STEP = 20;

/** 键盘缩放期望尺寸(逻辑像素,undefined = 尚未建立):
    方向键快速连按(keyboard repeat ~30ms/次)时多个并发 setSize 都基于 await
    innerSize() 读到的同一旧尺寸,连按丢步进(审计中1,2026-08-18);改为期望值
    累加——首次按键读一次当前尺寸作起点,后续按键在期望值上累加并 clamp,
    不重复读窗口尺寸;指针拖动开始(resizeStart)重置,防与真实尺寸脱节 */
let keyResizeW: number | undefined;
let keyResizeH: number | undefined;
let keyResizeMaxW = 4096;
let keyResizeMaxH = 4096;

/** 键盘缩放期望值起点:读当前窗口尺寸 + 显示器上限(与 applyResize clamp 同口径) */
async function initKeyResizeBase() {
  try {
    const [size, sf] = await Promise.all([win.innerSize(), win.scaleFactor()]);
    const monitors = await logicalMonitors();
    keyResizeMaxW = monitors.length ? Math.max(...monitors.map((m) => m.w)) * 2 : 4096;
    keyResizeMaxH = monitors.length ? Math.max(...monitors.map((m) => m.h)) * 2 : 4096;
    keyResizeW = size.width / sf;
    keyResizeH = size.height / sf;
  } catch (err) {
    console.error("init key resize base failed", err);
  }
}

/** 缩放手柄键盘调节(role="slider" + tabindex 键盘可达,无指针时方向键等效拖动;
    clamp 与 applyResize 同口径 360×260 ~ 2 倍显示器) */
async function resizeByKey(dx: number, dy: number) {
  try {
    // 首次按键(或指针拖动后)建立期望值;后续按键在期望值上累加,不再重新读窗口
    if (keyResizeW === undefined) await initKeyResizeBase();
    const baseW = keyResizeW;
    const baseH = keyResizeH;
    if (baseW === undefined || baseH === undefined) return; // 起点读取失败,丢本次按键
    keyResizeW = Math.min(keyResizeMaxW, Math.max(360, baseW + dx));
    keyResizeH = Math.min(keyResizeMaxH, Math.max(260, baseH + dy));
    await win.setSize(new LogicalSize(Math.round(keyResizeW), Math.round(keyResizeH)));
    scheduleSaveSize(); // 与指针缩放结束同口径:调整后落一次尺寸
  } catch (err) {
    console.error("resize by key failed", err);
  }
}

function onResizeHandleKeydown(e: KeyboardEvent) {
  if (e.key === "ArrowRight") {
    e.preventDefault();
    void resizeByKey(RESIZE_KEY_STEP, 0);
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    void resizeByKey(0, RESIZE_KEY_STEP);
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    void resizeByKey(-RESIZE_KEY_STEP, 0);
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    void resizeByKey(0, -RESIZE_KEY_STEP);
  } else if (e.key === " ") {
    // role="slider" 无 Space 激活语义:吞掉,防止被窗口级打字即搜误判切到搜索视图
    e.preventDefault();
    e.stopPropagation();
  }
}

onMounted(async () => {
  // 缩放手柄 aria 上限:显示器最大宽 × 2(与 applyResize clamp 同口径;低5)
  try {
    const ms = await logicalMonitors();
    handleA11y.value.maxW = ms.length ? Math.max(...ms.map((m) => m.w)) * 2 : 4096;
  } catch {
    /* 保留默认 4096 兜底 */
  }
  void syncHandleA11y(); // 初始 aria-valuenow
  // 尺寸变化 → 防抖落盘(仅尺寸,位置不记忆;onMoved 不监听)+ 同步手柄 aria
  unlisteners.push(
    await win.onResized(() => {
      void syncHandleA11y();
      scheduleSaveSize();
    }),
  );

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

  // 岛展开状态(岛 watch expanded 广播 + 挂载时初始广播):Esc 二级收岛判据;
  // 面板隐藏期间监听常驻,状态持续对齐,无需每次 Esc 查询
  unlisteners.push(
    await listen("island-expand-state", (ev) => {
      const p = ev.payload as { expanded?: boolean } | null;
      islandExpanded.value = p?.expanded === true;
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
        <!-- 隐藏按钮(2026-08-14 真机反馈:主界面要可见的关闭入口,Esc 之外再给一个) -->
        <button
          class="view-btn close-btn"
          title="隐藏主面板"
          aria-label="隐藏主面板"
          @click="hidePanel"
        >
          <AuroraIcon name="close" :size="14" />
        </button>
      </div>
    </header>

    <!-- view-body:Settings 常驻挂载(v-show)预热——"配置界面打开卡一会儿"修复:
         启动即拉取数据,点击秒开;其余视图 KeepAlive 缓存 -->
    <div class="view-body" data-tauri-drag-region="false">
      <Settings v-show="activeView === 'settings'" :active="activeView === 'settings'" />
      <KeepAlive :include="CACHED_VIEWS">
        <SmallDesktopView v-if="activeView === 'small-desktop'" />
        <SearchView
          v-else-if="activeView === 'search'"
          :query="query"
          @open="onSearchOpen"
          @fill="onFillQuery"
        />
        <ClipboardView v-else-if="activeView === 'clipboard'" />
        <AIView v-else-if="activeView === 'ai'" @open-settings="showView('settings')" />
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

    <!-- 右下角缩放手柄(自绘;无边框窗口无系统 resize 边框;
         键盘可达:role="slider" + tabindex + 方向键调节,2026-08-14 波次 4;
         aria-valuenow/min/max 补全:当前逻辑尺寸(宽)+ 下限 360 + 上限显示器×2,
         与后端 clamp 同口径,valuetext 报 宽×高——2026-08-18 审计低5) -->
    <div
      class="resize-handle"
      role="slider"
      tabindex="0"
      aria-label="调整窗口大小"
      :aria-valuenow="handleA11y.w"
      :aria-valuemin="360"
      :aria-valuemax="handleA11y.maxW"
      :aria-valuetext="`${handleA11y.w}×${handleA11y.h}`"
      title="拖动调整大小"
      @pointerdown="resizeStart"
      @keydown="onResizeHandleKeydown"
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
  /* 字号档归并(设计文档 §5.3 四档):标题档 15px,原 13.5 混档(2026-08-14 审计) */
  font-size: 15px;
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
  /* 字号档归并(设计文档 §5.3 四档):输入档 17px,原 16.5 混档(2026-08-14 审计) */
  font-size: 17px;
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
  /* 字号档归并(设计文档 §5.3 四档):caption 档 11px,原 10.5 混档(2026-08-14 审计) */
  font-size: 11px;
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
  /* 字号档归并(设计文档 §5.3 四档):caption 档 11px,原 10 混档(2026-08-14 审计) */
  font-size: 11px;
  color: var(--aurora-text);
  /* 键帽投影固定黑 0.15(压暗通用语义,深浅皮肤通用,拂晓下观感正常) */
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
