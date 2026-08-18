<script lang="ts">
// 结果项真实图标 data URL 缓存(path → url;模块级共享,跨挂载保留)。
// SearchView 每次进入搜索视图都是全新挂载,实例级缓存会在切换视图后全部失效、
// 下次进入重复 IPC 拉取;提到模块级后仅首次拉取,后续挂载直接命中缓存
// (FileItem.vue 的 iconCache 同思路;后端 dock_get_icon 另有内存+磁盘双缓存;
// 波次5 G2 审计:原注释声称模块级,实际是组件作用域 reactive,已修正)
import { reactive } from "vue";

export const icons = reactive(new Map<string, string>());
</script>

<script setup lang="ts">
/**
 * Phase6 搜索视图(SearchBar.vue 结果区逻辑迁移,设计文档 §4.1 + 预览稿 renderSearch)。
 * - 输入状态由壳(MainPanel)持有,经 props.query 传入(打字即搜写入/切视图保留);
 *   本组件只做结果渲染 + 选中态,打开成功 emit("open") 由壳收窗。
 * - 150ms 防抖搜索 + seq 防过期(快速输入丢弃过期响应);
 * - 应用组在前、文件组在后(应用优先,其次文件,后端序);「最近打开」固定在下,
 *   与结果重复的项隐藏(预览稿行为);空输入 = 最近打开铺满,无占位文案。
 * - 最近打开含文件条目(设计文档 §6:open_item 打开文件同样记入);
 * - 图标缓存为模块级单例(见上方普通 script 块):跨挂载保留,避免重复 IPC;
 * - 键盘 ↑↓/Enter 窗口级监听(仅挂载期间;Esc/打字即搜由壳处理)。
 * 样式移植预览稿 .results/.group-label/.result-item/.app-icon/.item-name/.item-sub/.empty-state。
 */
import {
  computed,
  nextTick,
  onActivated,
  onMounted,
  onUnmounted,
  ref,
  watch,
} from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useRecentApps } from "../../../composables/useRecentApps";
import AuroraIcon from "../../icons/AuroraIcon.vue";

defineOptions({ name: "SearchView" });

const props = defineProps<{ query: string }>();
const emit = defineEmits<{ (e: "open"): void }>();

/** 后端 search_apps 返回的应用条目(name/path;icon 字段未落地,走 dock_get_icon 取真实图标) */
interface AppEntry {
  name: string;
  path: string;
  icon?: string | null;
}

/** 后端 search_files 返回的文件条目(is_dir 供文件夹角标区分) */
interface FileEntry {
  path: string;
  name: string;
  is_dir: boolean;
}

/** 可导航条目最小公共形状(应用/文件/最近打开) */
interface NavItem {
  name: string;
  path: string;
  kind: "app" | "file";
  icon?: string | null;
  is_dir?: boolean;
}

/** 渲染分组(应用/文件/最近打开;start = 全局选中序号偏移,动画 stagger 与键盘导航共用) */
interface ResultGroup {
  label: string;
  count: number;
  items: NavItem[];
  start: number;
  /** 最近打开组(条目带 recent-dot 标识,文件条目除外——与预览稿一致) */
  recent: boolean;
}

const appResults = ref<AppEntry[]>([]);
const fileResults = ref<FileEntry[]>([]);
const selected = ref(0);
const searching = ref(false);
const searchError = ref("");
const { recents, loadRecents, saveRecent } = useRecentApps();
// icons 为模块级缓存(上方普通 script 块声明,reactive 保持"晚到图标触发重渲染")
// 结果项 DOM 引用(键盘选中后 scrollIntoView 用)
const itemEls: (HTMLElement | null)[] = [];

let debounceTimer: number | undefined;
let searchSeq = 0;

/** 应用/文件结果 → 统一导航条目 */
const appItems = computed<NavItem[]>(() =>
  appResults.value.map((a) => ({ name: a.name, path: a.path, icon: a.icon, kind: "app" })),
);
const fileItems = computed<NavItem[]>(() =>
  fileResults.value.map((f) => ({ name: f.name, path: f.path, kind: "file", is_dir: f.is_dir })),
);

/** 最近打开中的条目(kind 缺省按应用;老 localStorage 数据无 kind 字段) */
const recentItems = computed<NavItem[]>(() =>
  recents.value.map((r) => ({ name: r.name, path: r.path, kind: r.kind ?? "app" })),
);

/** 分组结构:应用组/文件组在前,最近打开组固定在下(与结果重复项隐藏;预览稿 renderSearch) */
const groups = computed<ResultGroup[]>(() => {
  const g: ResultGroup[] = [];
  let start = 0;
  if (!props.query.trim()) {
    const items = recentItems.value;
    if (items.length) g.push({ label: "最近打开", count: items.length, items, start, recent: true });
    return g;
  }
  if (appItems.value.length) {
    g.push({ label: "搜索结果 · 应用", count: appItems.value.length, items: appItems.value, start, recent: false });
    start += appItems.value.length;
  }
  if (fileItems.value.length) {
    g.push({ label: "搜索结果 · 文件", count: fileItems.value.length, items: fileItems.value, start, recent: false });
    start += fileItems.value.length;
  }
  // 固定「最近打开」栏:已出现在结果里的项隐藏,不重复展示
  const seenApp = new Set(appItems.value.map((a) => a.path));
  const seenFile = new Set(fileItems.value.map((f) => f.path));
  const merged = recentItems.value.filter((r) =>
    r.kind === "file" ? !seenFile.has(r.path) : !seenApp.has(r.path),
  );
  if (merged.length) {
    g.push({ label: "最近打开", count: merged.length, items: merged, start, recent: true });
  }
  return g;
});

/** 当前键盘可导航列表:全部可见条目按渲染序合并(↑↓ 在组间连续移动) */
const navigableItems = computed<NavItem[]>(() => groups.value.flatMap((g) => g.items));

// 可导航列表变化后:选中项回滚进视口 + 补齐图标
watch(
  navigableItems,
  () => {
    void scrollSelectedIntoView();
    refreshIcons();
  },
  { immediate: true },
);

async function scrollSelectedIntoView() {
  await nextTick();
  itemEls[selected.value]?.scrollIntoView({ block: "nearest" });
}

/** v-for 条目 ref 收集(函数 ref 每次渲染回调,卸载时 el 为 null) */
function setItemEl(el: unknown, i: number) {
  itemEls[i] = el instanceof HTMLElement ? el : null;
}

/** 条目图标:自带 data URI 直接用;否则取缓存图标;再无 → 首字母瓦片占位 */
function iconFor(item: NavItem): string | undefined {
  const ic = item.icon;
  if (ic && ic.startsWith("data:")) return ic;
  return icons.get(item.path);
}

/** 经后端 dock_get_icon 取真实图标 data URL;失败返回 undefined(回退首字母瓦片) */
async function iconOf(path: string): Promise<string | undefined> {
  const hit = icons.get(path);
  if (hit) return hit;
  try {
    const url = await invoke<string | null>("dock_get_icon", { path });
    if (url) {
      icons.set(path, url);
      return url;
    }
  } catch (e) {
    console.error("dock_get_icon failed", e);
  }
  return undefined;
}

/** 为当前可见的应用条目补齐图标(命中缓存即跳过;文件条目不取系统图标,
 * dock_get_icon 的 ExtractIconExW 只认 exe/ico,固定用类型角标,避免空发 IPC) */
function refreshIcons() {
  for (const it of groups.value.flatMap((g) => g.items)) {
    if (it.kind !== "app") continue;
    if (it.icon && it.icon.startsWith("data:")) continue;
    if (!icons.has(it.path)) void iconOf(it.path);
  }
}

/** 条目位置提示:路径去掉文件名后的目录段,过长只保留最末两级并前置省略号 */
function dirLabelOf(item: NavItem): string {
  const dir = item.path.endsWith(item.name)
    ? item.path.slice(0, item.path.length - item.name.length)
    : "";
  if (!dir) return "";
  const segs = dir.split(/[\\/]+/).filter(Boolean);
  const tail = segs.slice(-2).join(" / ");
  return segs.length > 2 ? `… ${tail}` : tail;
}

/** 首字母瓦片占位(应用无图标时;与 FileItem 同款:单字符补空格居中) */
function padLetter(s: string): string {
  return s.length > 1 ? s : s + " ";
}

async function doSearch() {
  const q = props.query.trim();
  const seq = ++searchSeq;
  if (!q) {
    // 空输入回到最近打开列表(结果清空,进行中指示熄灭)
    appResults.value = [];
    fileResults.value = [];
    selected.value = 0;
    searching.value = false;
    searchError.value = "";
    return;
  }
  searching.value = true;
  searchError.value = "";
  // 应用与文件搜索并行发起;文件搜索失败静默降级,应用搜索失败才提示错误
  const [apps, files] = await Promise.all([
    invoke<AppEntry[]>("search_apps", { query: q }).catch((e) => {
      console.error(e);
      return undefined;
    }),
    invoke<FileEntry[]>("search_files", { query: q, maxResults: 8 }).catch((e) => {
      console.error(e);
      return undefined;
    }),
  ]);
  if (seq !== searchSeq) return; // 已有更新的请求,丢弃过期结果
  appResults.value = apps ?? [];
  fileResults.value = files ?? [];
  selected.value = 0;
  searching.value = false;
  if (apps === undefined) searchError.value = "搜索失败,请稍后重试";
}

// 输入变化(壳打字即搜/清空)→ 150ms 防抖搜索;immediate 覆盖打字即搜挂载即搜的场景
watch(
  () => props.query,
  () => {
    if (debounceTimer) window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(() => {
      debounceTimer = undefined;
      void doSearch();
    }, 150);
  },
  { immediate: true },
);

function selectNext() {
  if (navigableItems.value.length === 0) return;
  selected.value = (selected.value + 1) % navigableItems.value.length;
  void scrollSelectedIntoView();
}

function selectPrev() {
  if (navigableItems.value.length === 0) return;
  selected.value =
    (selected.value - 1 + navigableItems.value.length) % navigableItems.value.length;
  void scrollSelectedIntoView();
}

/** 轻提示(打开失败等非阻断反馈;3s 自动消退,与 Island showHint 同型) */
const hint = ref("");
let hintTimer: number | undefined;

function showHint(text: string) {
  hint.value = text;
  if (hintTimer) window.clearTimeout(hintTimer);
  hintTimer = window.setTimeout(() => {
    hint.value = "";
  }, 3000);
}

async function openSelected() {
  // 应用与文件走同一 open_item(ShellExecute 语义,文件/目录/应用通吃)
  const item = navigableItems.value[selected.value];
  if (!item) return;
  let ok = false;
  try {
    ok = await invoke<boolean>("open_item", { path: item.path });
  } catch (e) {
    console.error("open_item failed", e);
  }
  // 打开失败(返回 false 或 IPC 异常):轻提示并保留面板,结果列表仍在可改选重试;
  // 不再无条件 emit("open") 关面板(2026-08-14 波次 3 审计)
  if (!ok) {
    showHint("打开失败,请重试");
    return;
  }
  // 打开成功即写入最近使用(置顶去重;文件条目同样记入,kind 区分展示)
  saveRecent({ name: item.name, path: item.path, kind: item.kind });
  emit("open");
}

/**
 * 窗口级键盘(仅本视图挂载期间):↑↓ 选择 / Enter 打开;
 * Esc 与打字即搜由壳统一处理;IME 组合中不劫持方向键;按钮焦点放行原生行为。
 */
function onWindowKeydown(e: KeyboardEvent) {
  if (e.isComposing) return;
  if (e.target instanceof HTMLButtonElement) return;
  if (e.key === "ArrowDown") {
    e.preventDefault();
    selectNext();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    selectPrev();
  } else if (e.key === "Enter") {
    e.preventDefault();
    void openSelected();
  }
}

// 最近打开拉取:挂载时执行;onActivated 兜底(当前 SearchView 不在 KeepAlive 缓存名单,
// 每次进入全新挂载,onMounted 已覆盖;若未来加入缓存,激活时需重读 localStorage。
// keydown 监听只在 onMounted 注册一次,onActivated 不重复注册)
onMounted(() => {
  loadRecents(); // 最近打开列表(空 query 态展示)
  window.addEventListener("keydown", onWindowKeydown);
});

onActivated(loadRecents);

onUnmounted(() => {
  window.removeEventListener("keydown", onWindowKeydown);
  if (debounceTimer) window.clearTimeout(debounceTimer);
  if (hintTimer) window.clearTimeout(hintTimer);
});
</script>

<template>
  <div class="relative h-full w-full flex flex-col min-h-0 select-none">
    <!-- 进行中指示(150ms 防抖后真正发起搜索期间) -->
    <div
      v-if="searching"
      class="px-4 py-3 flex items-center gap-2 text-xs text-[var(--aurora-text-dim)]"
    >
      <span
        class="aurora-spin inline-block h-3 w-3 rounded-full border border-[var(--aurora-text-dim)] border-t-transparent"
      ></span>
      搜索中…
    </div>
    <!-- 错误态:invoke 失败与"无匹配结果"区分开,避免误导 -->
    <div v-else-if="searchError" class="px-4 py-3 text-xs text-[var(--aurora-danger)]">
      {{ searchError }}
    </div>
    <!-- 有输入且无结果:空态(✦ 装饰) -->
    <div
      v-else-if="props.query.trim() && navigableItems.length === 0"
      class="empty-state"
    >
      <div class="big">✦</div>
      无匹配结果
      <br />
      <span class="sub">换个关键词试试</span>
    </div>
    <!-- 空输入且最近打开为空:引导空态(与"无匹配结果"区分;2026-08-14 真机反馈:
         空态全空白像"功能没了",补引导文案) -->
    <div
      v-else-if="!props.query.trim() && navigableItems.length === 0"
      class="empty-state"
    >
      <div class="big">✦</div>
      输入以搜索
      <br />
      <span class="sub">打开过的应用与文件会出现在这里</span>
    </div>
    <!-- 结果列表:应用/文件在上,最近打开固定在下 -->
    <div v-else class="results flex-1 min-h-0" role="listbox">
      <template v-for="(g, gi) in groups" :key="gi">
        <div class="group-label">
          {{ g.label }}
          <span class="count num">{{ g.count }}</span>
        </div>
        <div
          v-for="(item, ii) in g.items"
          :key="item.path + ':' + ii"
          :ref="(el) => setItemEl(el, g.start + ii)"
          class="result-item"
          :class="{ selected: selected === g.start + ii }"
          role="option"
          :aria-selected="selected === g.start + ii"
          :style="{ animationDelay: (g.start + ii) * 28 + 'ms' }"
          @mouseenter="selected = g.start + ii"
          @click="openSelected"
        >
          <!-- 文件条目:类型角标(目录/文件);应用条目:真实图标或首字母瓦片 -->
          <span v-if="item.kind === 'file'" class="app-icon app-icon-type" :title="item.is_dir ? '文件夹' : '文件'">
            <AuroraIcon :name="item.is_dir ? 'folder' : 'file'" :size="15" />
          </span>
          <img
            v-else-if="iconFor(item)"
            :src="iconFor(item)"
            class="app-icon"
            draggable="false"
            alt=""
          />
          <span v-else class="app-icon app-icon-fallback">{{ padLetter(item.name.slice(0, 1)) }}</span>
          <span class="item-name">{{ item.name }}</span>
          <span v-if="g.recent && item.kind !== 'file'" class="recent-dot" title="最近使用"></span>
          <span v-if="dirLabelOf(item)" class="item-sub">{{ dirLabelOf(item) }}</span>
        </div>
      </template>
    </div>
    <!-- 打开失败等非阻断轻提示:视图内居中浮层,3s 自动消退(不占用结果列表空间) -->
    <Transition name="hint-fade">
      <div v-if="hint" class="view-hint">{{ hint }}</div>
    </Transition>
  </div>
</template>

<style scoped>
/* 样式移植自设计稿 aurora-v02-preview.html(.results/.group-label/.result-item/
   .app-icon/.item-name/.item-sub/.recent-dot/.empty-state) */
.results {
  padding: 6px 8px;
  overflow-y: auto;
}

.group-label {
  display: flex;
  align-items: baseline;
  gap: 7px;
  padding: 9px 12px 5px;
  font-size: 11px;
  font-weight: 650;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--aurora-text-dim);
}

.group-label .count {
  font-weight: 500;
  letter-spacing: 0.02em;
  opacity: 0.75;
}

.result-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 11px;
  height: 42px;
  padding: 0 12px;
  border-radius: 10px;
  font-size: 13px;
  cursor: pointer;
  animation: rise-in 0.22s ease both;
  transition: background 0.1s ease;
}

.result-item:hover,
.result-item.selected {
  background: var(--aurora-field);
}

/* 选中态:左侧极光渐变竖条 + 光晕(选中语言统一,设计文档 §5.3) */
.result-item.selected::before {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  transform: translateY(-50%);
  height: 18px;
  width: 3px;
  border-radius: 0 99px 99px 0;
  background: linear-gradient(180deg, var(--aur-1), var(--aur-2));
  box-shadow: 0 0 8px var(--aur-2);
}

.app-icon {
  flex: none;
  width: 24px;
  height: 24px;
  border-radius: 7px;
  overflow: hidden;
  object-fit: contain;
  pointer-events: none;
}

/* 文件类型角标瓦片(目录/文件统一灰色系,与预览稿 FILE_ICO 同观感) */
.app-icon-type {
  display: grid;
  place-items: center;
  background: var(--aurora-field);
  color: var(--aurora-text-dim);
  border-radius: 7px;
  flex: none;
}

/* 应用无图标回退:首字母瓦片(强调色系,与 Dock 占位同款观感) */
.app-icon-fallback {
  display: grid;
  place-items: center;
  background: color-mix(in srgb, var(--aurora-accent) 22%, transparent);
  color: var(--aurora-accent);
  font-size: 12px;
  font-weight: 600;
  flex: none;
}

.item-name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-sub {
  font-size: 11px;
  color: var(--aurora-text-dim);
  flex: none;
  max-width: 38%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.recent-dot {
  flex: none;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--aur-2);
  box-shadow: 0 0 6px var(--aur-2);
}

.empty-state {
  padding: 26px 16px 30px;
  text-align: center;
  font-size: 11px;
  color: var(--aurora-text-dim);
}

.empty-state .big {
  font-size: 26px;
  margin-bottom: 10px;
}

.empty-state .sub {
  font-size: 11px;
  opacity: 0.7;
}

@keyframes rise-in {
  from {
    opacity: 0;
    transform: translateY(7px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  .result-item {
    animation-duration: 0.01s;
  }
  /* 搜索旋转指示:归零动画 */
  .aurora-spin {
    animation: none;
  }
}

/* 搜索进行中指示:旋转小圆环 */
@keyframes aurora-spin {
  to {
    transform: rotate(360deg);
  }
}
.aurora-spin {
  animation: aurora-spin 0.8s linear infinite;
}

/* 轻提示气泡(打开失败等):视图内居中浮层,与 Island .island-hint 同观感 */
.view-hint {
  position: absolute;
  left: 50%;
  bottom: 14px;
  transform: translateX(-50%);
  padding: 5px 14px;
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
</style>
