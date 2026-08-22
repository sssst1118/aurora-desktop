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
/** 加载失败原因(非空时空态区显示真实错误,不再伪装成"应用被清空";2026-08-14 审计) */
const loadError = ref("");
/** 最近一次 addPaths 写盘失败原因(空 = 成功或全部已存在;父组件 await 后读此字段
 *  区分「已存在」与「保存失败」两种语义,见 defineExpose) */
const addFailReason = ref("");

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

/** 移除中集合:防连点(快速双击 ✕ 时第二次基于旧 items 过滤会重复写盘,
    复用 launch 的进行中集合模式,await 完成即释放——低1,2026-08-18) */
const removing = ref<Set<string>>(new Set());

/** launch 脉冲定时器集合(卸载时全部清理:多个条目并发启动时各持一个 timer,
    防卸载后 emit("launched") 打到已卸载父组件——低2,2026-08-18) */
const launchTimers = new Set<number>();

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

/**
 * 添加条目(去重;写后端持久化 + 后台取图标)。返回实际新增数(0 = 已存在 或 写盘失败)。
 * 语义区分:失败时置 addFailReason(父组件 await 后读 dockRef.addFailReason.value
 * 区分「已存在」与「保存失败」;返回结构不变,与旧调用方契约兼容)。
 */
async function addPaths(paths: string[]): Promise<number> {
  addFailReason.value = ""; // 每次尝试先复位,父组件 await 后读到的即本次结果
  // 中2(2026-08-14 波次 4):前置去重与父组件提示链同口径——Windows 路径大小写
  // 不敏感,统一 toLowerCase 比较(此前严格相等,已存 C:\A.exe 时拖入 c:\a.exe
  // 会写盘生成真实重复条目)
  const fresh = paths.filter(
    (p) => !items.value.some((it) => it.path.toLowerCase() === p.toLowerCase()),
  );
  if (fresh.length === 0) return 0; // 全部已存在(调用方按 0 提示"已在岛内")
  const next = [...items.value, ...fresh.map((p) => ({ name: nameOf(p), path: p }))];
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
    for (const p of fresh) void iconOf(p);
    return fresh.length;
  } catch (e) {
    console.error("dock_set_items failed", e);
    addFailReason.value = typeof e === "string" ? e : String(e);
    return 0;
  }
}

/** 悬停 ✕ 移除条目 */
async function removeItem(item: DockItem) {
  if (removing.value.has(item.path)) return; // 移除中,忽略连点
  removing.value.add(item.path);
  const next = items.value.filter((it) => it.path !== item.path);
  try {
    await invoke<boolean>("dock_set_items", { items: next });
    items.value = next;
  } catch (e) {
    console.error("dock_set_items failed", e);
  } finally {
    removing.value.delete(item.path);
  }
}

/** 点击:运行中 → dock_launch 聚焦;未运行 → 启动。脉冲反馈到期后通知父组件收回岛 */
async function launch(item: DockItem) {
  if (launching.value.has(item.path)) return; // 启动中,忽略连点
  launching.value.add(item.path);
  const t = window.setTimeout(() => {
    launchTimers.delete(t);
    launching.value.delete(item.path);
    emit("launched");
  }, LAUNCHING_MS);
  launchTimers.add(t);
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
    loadError.value = "";
    // 图标预热:挂载即后台发起全部提取(后端缓存幂等),展开零延迟
    for (const it of items.value) void iconOf(it.path);
  } catch (e) {
    console.error("dock_get_items failed", e);
    // 加载失败不吞错误:空态区显示真实原因(否则像"应用被清空"),并可点击重试
    loadError.value = typeof e === "string" ? e : String(e);
    emit("hint", `应用加载失败:${loadError.value}`);
  }
}

// ---- 溢出浮层:临时加高窗口(逻辑像素宽读当前实际值,不依赖父组件动画态) ----

/** 加高/恢复窗口的串行链:快速开合浮层时多次 setSize 并发(各自 await innerSize
    读同一旧高度)会互相覆盖,排队保证后一次基于前一次结果;
    父组件收岛动画前 await(见 onCollapse),确保高度已恢复 46 再读 innerSize——
    审计中3(2026-08-18):closeOverflow 的 setSize(46) 与 setWidthAnimated 读
    innerSize→curH 并发,动画会把加高后的高度当 curH 逐帧写回,浮层已关但窗口
    高度残留 46+extra 透明区 */
let extraHeightChain: Promise<void> = Promise.resolve();

async function setWindowExtraHeight(extra: number) {
  const task = extraHeightChain.then(() => applyExtraHeight(extra));
  extraHeightChain = task.catch(() => undefined); // 单次失败不阻断后续排队
  return task;
}

async function applyExtraHeight(extra: number) {
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

/** 岛收起动画前由父组件显式调用(审计中3):watch(expanded) 异步 flush,收岛动画
    读内高时 closeOverflow 可能尚未入链;这里同步发起关闭并等待高度恢复完成。
    浮层未开时链已空,零开销(收岛路径全覆盖:所有收岛都经父组件 toggleExpand) */
async function onCollapse(): Promise<void> {
  if (overflowOpen.value) closeOverflow();
  await extraHeightChain;
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

/** 浮层 Esc 键盘关闭(「…」按钮聚焦时 Esc 关浮层,与 mousedown 外点关闭并列,
    键盘可达闭环——低3,2026-08-18):浮层开着时 Esc 只关浮层并阻断冒泡,不与
    窗口级 Esc 递进叠加;浮层关闭后的下一次 Esc 由外层监听继续递进处理 */
function onWinKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape" || !overflowOpen.value) return;
  e.stopPropagation();
  closeOverflow();
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
  window.addEventListener("keydown", onWinKeydown);
  // 可视容量跟随宽度变化(展开动画/左段内容宽窄变化/DPI 缩放)
  dockResize = new ResizeObserver(recomputeVisible);
  if (dockEl.value) dockResize.observe(dockEl.value);
});

onUnmounted(() => {
  // 卸载时清理全部 launch 脉冲定时器(防卸载后 emit 打到已卸载父组件)
  launchTimers.forEach((t) => window.clearTimeout(t));
  launchTimers.clear();
  if (runningTimer) window.clearInterval(runningTimer);
  document.removeEventListener("mousedown", onDocMouseDown);
  window.removeEventListener("keydown", onWinKeydown);
  dockResize?.disconnect();
});

// items 增删后溢出判定变化(是否腾出「…」按钮位),重算容量
watch(items, recomputeVisible);

defineExpose({ addPaths, addFailReason, onCollapse });
</script>

<template>
  <!-- 括入区根容器:2026-08-19 真机反馈「展开后有概率拖不动」——此前显式禁拖,
  展开态约 40% 宽度(括入区)完全不可拖。本区全部交互件均为 <button>(drag.js
  自动豁免,点击/悬停 ✕/＋ 仍正常),故根容器标 true 只让其自身空白区域可拖。
  expanded class = 岛展开态样式联动(收起时隐藏,2026-08-14 真机反馈:此前收起态
  括入区无隐藏样式,被静态内容挤成 34px 宽仍可见,表现为"Dock 只有一个图标") -->
  <div ref="dockEl" class="mini-dock" :class="{ expanded }" data-tauri-drag-region="true">
    <button
      v-for="it in visibleItems"
      :key="it.path"
      class="dock-tile"
      :class="{ launching: launching.has(it.path) }"
      :title="it.name"
      :aria-label="`启动 ${it.name}`"
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
      <!-- 悬停 ✕ 删除(不触发启动:stop 掉点击;瓦片本身是 button,徽章不能嵌套 button,
           用 tabindex+role 提供键盘可达,Enter/Space 触发删除) -->
      <span
        class="del-badge"
        role="button"
        tabindex="0"
        :title="`移除 ${it.name}`"
        :aria-label="`移除 ${it.name}`"
        @click.stop="removeItem(it)"
        @keydown.enter.stop.prevent="removeItem(it)"
        @keydown.space.stop.prevent="removeItem(it)"
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

    <!-- 空态:加载失败显示真实错误并可点击重试(否则像"应用被清空"误导) -->
    <button
      v-if="items.length === 0"
      class="dock-empty"
      :class="{ err: loadError }"
      :title="loadError ? '点击重试加载应用' : undefined"
      @click="loadItems"
    >
      {{ loadError ? "应用加载失败,点击重试" : "拖拽应用到这里" }}
    </button>
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
        :aria-label="`启动 ${it.name}`"
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
          role="button"
          tabindex="0"
          :title="`移除 ${it.name}`"
          :aria-label="`移除 ${it.name}`"
          @click.stop="removeItem(it)"
          @keydown.enter.stop.prevent="removeItem(it)"
          @keydown.space.stop.prevent="removeItem(it)"
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
  /* 占位瓦片与真实图标统一 34px 满瓦片(圆角 22% ≈ 7.5px),尺寸/圆角同款 */
  width: 34px;
  height: 34px;
  display: grid;
  place-items: center;
  pointer-events: none;
}
.dock-ico-img {
  width: 34px;
  height: 34px;
  border-radius: 22%;
  display: block;
  /* contain + 2px 内边距:RSView 类满幅图标不再贴边被裁 */
  object-fit: contain;
  padding: 2px;
  box-sizing: border-box;
}
.dock-ico-fallback {
  width: 34px;
  height: 34px;
  display: grid;
  place-items: center;
  border-radius: 22%;
  background: var(--aurora-field);
  box-shadow: var(--aurora-card-shadow);
  font-size: 12px;
  font-weight: 600;
  color: var(--aurora-text);
}
/* 溢出浮层行高 40px:行内图标回退 24px,不撑破行 */
.dock-overflow-row .dock-ico,
.dock-overflow-row .dock-ico-img,
.dock-overflow-row .dock-ico-fallback {
  width: 24px;
  height: 24px;
}

/* 空态(button:加载失败时可点击重试;err 态用危险色提示真实错误) */
.dock-empty {
  border: none;
  background: transparent;
  font-family: inherit;
  font-size: 11px;
  color: var(--aurora-text-dim);
  white-space: nowrap;
  cursor: pointer;
  padding: 0;
}

.dock-empty.err {
  color: var(--aurora-danger);
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
  color: #fff; /* 危险红底白字:四套皮肤 danger 均为 300/400 系亮色,白字对比足够;
                  与项目内 ai-confirm-ok/ai-btn-stop 的 on-accent 白字同一约定,故不抽令牌 */
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
}
/* 删除徽章默认 hover 显示;键盘可达(2026-08-14 审计):瓦片聚焦或徽章自身聚焦时同样显示 */
.dock-tile:hover .del-badge,
.dock-tile:focus-within .del-badge,
.dock-tile:focus .del-badge {
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

@media (prefers-reduced-motion: reduce) {
  /* 启动脉冲:归零动画 */
  .dock-tile.launching,
  .dock-overflow-row.launching {
    animation: none;
  }
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
.dock-overflow-row:hover .dock-overflow-del,
.dock-overflow-row:focus-within .dock-overflow-del {
  display: grid;
}
</style>
