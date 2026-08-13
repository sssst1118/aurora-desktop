<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Settings from "./Settings.vue";
import Dock from "./Dock.vue";
import { useRecentApps, type RecentApp } from "../../composables/useRecentApps";

interface AppEntry {
  name: string;
  path: string;
  /**
   * 后端预留图标字段(约定为 base64 data URI)。
   * 注:当前后端 AppEntry 尚未返回该字段(2026-08-13 核对 indexer/app_index.rs 仅有
   * name/path),因此非 data URI / 缺失时统一走 dock_get_icon 取真实图标,再无才回退 emoji。
   */
  icon?: string | null;
}

/** 后端 search_files 返回的文件条目({ path, name, is_dir } 契约,is_dir 供类型角标区分) */
interface FileEntry {
  path: string;
  name: string;
  is_dir: boolean;
}

const query = ref("");
// 应用搜索结果与文件搜索结果分开存(§6.3「应用优先,其次文件」:两组分别渲染)
const appResults = ref<AppEntry[]>([]);
const fileResults = ref<FileEntry[]>([]);
const selected = ref(0);
const inputEl = ref<HTMLInputElement | null>(null);
const showSettings = ref(false);
// 2.1 Dock(并入搜索窗口形态):启动立即 + 每次窗口显示时重读开关,
// 设置页保存后经 aurora:config-saved 事件即时刷新(热生效)
const enableDock = ref(false);
// 显示方式:"glass" 毛玻璃(默认) | "solid" 不透明(2026-08-12 用户要求可选)
const searchStyle = ref("glass");
// 搜索进行中指示(invoke 期间;配合下方 150ms 防抖,只在真正发起搜索后亮起)
const searching = ref(false);
// 搜索错误态:invoke 失败时给明确提示,避免误导成"无匹配结果"
const searchError = ref("");
// 最近打开(空 query 时展示;localStorage 持久化,上限 10 条)
const { recents, loadRecents, saveRecent } = useRecentApps();
// 结果项真实图标 data URL 缓存(path → url;后端 dock_get_icon 自带内存+磁盘双缓存)
const icons = ref<Map<string, string>>(new Map());
// 结果项 DOM 引用(键盘选中后 scrollIntoView 用)
const itemEls: (HTMLElement | null)[] = [];

let debounceTimer: number | undefined;
let searchSeq = 0; // 请求序号:快速输入时丢弃过期响应,防旧结果覆盖新结果

const win = getCurrentWindow();

/** 可导航条目最小公共形状(应用/文件/最近打开;文件条目的 is_dir 供角标使用) */
type NavItem = { name: string; path: string; icon?: string | null; is_dir?: boolean };

// 最近使用的路径集合(有输入时给命中应用打"最近"标识;Set 保证 O(1) 判定)
const recentPaths = computed(() => new Set(recents.value.map((r) => r.path)));

/**
 * 应用结果展示序(搜索质量包 2026-08-13):有输入时把「命中结果中的最近打开应用」
 * 置顶(按最近打开倒序),其余保持后端返回的原序(后端已按匹配层级排好);
 * 最近打开但未命中的应用不额外塞入。纯前端排序,不改后端契约。
 */
const rankedAppResults = computed<AppEntry[]>(() => {
  const apps = appResults.value;
  if (apps.length === 0 || recentPaths.value.size === 0) return apps;
  const byPath = new Map(apps.map((a) => [a.path, a]));
  const boosted: AppEntry[] = [];
  for (const r of recents.value) {
    const hit = byPath.get(r.path);
    if (hit) boosted.push(hit);
  }
  if (boosted.length === 0) return apps;
  const seen = new Set(boosted.map((a) => a.path));
  return [...boosted, ...apps.filter((a) => !seen.has(a.path))];
});

/** 有输入时的合并展示:应用组在前(含最近置顶)、文件组在后(应用优先,其次文件) */
const allResults = computed<NavItem[]>(() => [
  ...rankedAppResults.value,
  ...fileResults.value,
]);

/** 当前键盘可导航列表:有输入 → 应用+文件合并(↑↓ 在两组间连续移动);无输入 → 最近打开 */
const navigableItems = computed<NavItem[] | RecentApp[]>(() =>
  query.value.trim() ? allResults.value : recents.value,
);

// 可导航列表变化(新搜索结果/回到最近打开)后:选中项回滚进视口 + 补齐图标
watch(
  navigableItems,
  () => {
    void scrollSelectedIntoView();
    refreshIcons();
  },
  { immediate: true },
);

/** 选中项跟随滚动:block nearest 最小滚动,消除"选中项滚出视口看不见" */
async function scrollSelectedIntoView() {
  await nextTick();
  itemEls[selected.value]?.scrollIntoView({ block: "nearest" });
}

/** v-for 条目 ref 收集(函数 ref 每次渲染都会回调,卸载时 el 为 null) */
function setItemEl(el: unknown, i: number) {
  itemEls[i] = el instanceof HTMLElement ? el : null;
}

/** 条目图标:自带 data URI 直接用;否则取缓存图标;再无 → 模板回退 emoji */
function iconFor(item: AppEntry | RecentApp): string | undefined {
  const ic = (item as AppEntry).icon;
  if (ic && ic.startsWith("data:")) return ic;
  return icons.value.get(item.path);
}

/** 经后端 dock_get_icon 取真实图标 data URL(与 Dock.vue 同款);失败返回 undefined */
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

/** 为当前可见条目补齐图标(命中缓存即跳过;图标未就绪期间显示 emoji 占位)。
 * 文件条目不取系统图标(dock_get_icon 的 ExtractIconExW 只认 exe/ico,
 * 普通文件/目录必失败),固定用类型角标,避免每次搜索空发一串 IPC。 */
function refreshIcons() {
  for (const it of query.value.trim() ? rankedAppResults.value : recents.value) {
    const ic = (it as AppEntry).icon;
    if (ic && ic.startsWith("data:")) continue;
    if (!icons.value.has(it.path)) void iconOf(it.path);
  }
}

/** 文件条目的位置提示(相对路径省略):取路径去掉文件名后的目录段,
 * 过长时只保留最末两级目录并前置省略号;路径即文件名(罕见)时为空 */
function dirLabelOf(item: FileEntry): string {
  const dir = item.path.endsWith(item.name)
    ? item.path.slice(0, item.path.length - item.name.length)
    : "";
  if (!dir) return "";
  const segs = dir.split(/[\\/]+/).filter(Boolean);
  const tail = segs.slice(-2).join(" / ");
  return segs.length > 2 ? `… ${tail}` : tail;
}

// Dock 开关立即读取(不等 onShown):Dock 组件在应用启动时即挂载并后台提取图标,
// 用户呼出搜索栏时图标已就绪(否则首次呼出才挂载,COM 初始化的 ~1.9s 成本
// 会压在"打开搜索栏之后"——实测首 lnk 图标提取独占 1.85s)
void loadDockFlag();
void loadStyleFlag();
loadRecents(); // 最近打开列表(空 query 态展示)

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
  const seq = ++searchSeq;
  if (!q) {
    // 空输入回到最近打开列表(应用/文件结果清空,进行中指示熄灭)
    appResults.value = [];
    fileResults.value = [];
    selected.value = 0;
    searching.value = false;
    searchError.value = "";
    return;
  }
  searching.value = true;
  searchError.value = "";
  // 应用与文件搜索并行发起;文件搜索失败静默降级(只缺文件组,不影响应用结果),
  // 应用搜索失败才提示错误(与旧行为一致,避免误导成"无匹配结果")
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

function onInput() {
  if (debounceTimer) window.clearTimeout(debounceTimer);
  debounceTimer = window.setTimeout(doSearch, 150);
}

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

async function openSelected() {
  // 应用与文件走同一 open_item(ShellExecute 语义,文件/目录/应用通吃)
  const item = navigableItems.value[selected.value];
  if (!item) return;
  // 打开成功即写入最近使用(置顶去重;文件条目同样记入);失败也照旧关窗,与既有行为一致
  const ok = await invoke<boolean>("open_item", { path: item.path });
  if (ok) saveRecent(item);
  await win.hide();
}

/** 清空输入:取消挂起的防抖,并让进行中的搜索结果过期,回到最近打开列表 */
function clearQuery() {
  if (debounceTimer) {
    window.clearTimeout(debounceTimer);
    debounceTimer = undefined;
  }
  query.value = "";
  void doSearch();
}

/**
 * 窗口级 keydown(原绑定在 input 上,切到设置页 input 卸载后 Esc 断流,故移到这里兜底全态):
 * - 设置态:Esc 先关设置页返回搜索态,其余按键交给设置页内部处理;
 * - 搜索态:↑↓ 选择 / Enter 打开 / 二级 Esc(有输入先清空,再按才关窗,对标 Raycast);
 * - IME 组合输入中不劫持方向键(中文输入法选词不误触)。
 */
function onWindowKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    if (showSettings.value) {
      showSettings.value = false;
      void nextTick().then(() => inputEl.value?.focus());
    } else if (query.value) {
      clearQuery();
    } else {
      void win.hide();
    }
    return;
  }
  if (showSettings.value) return;
  if (e.isComposing) return;
  // 焦点在按钮上(如 ⚙ 设置按钮)时放行,让按钮原生响应 Enter/方向键
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

/** 窗口每次显示时:关闭设置、清空输入、聚焦输入框、重读 Dock 开关 */
function onShown() {
  if (showSettings.value) showSettings.value = false;
  query.value = "";
  appResults.value = [];
  fileResults.value = [];
  selected.value = 0;
  searching.value = false;
  searchError.value = "";
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
  // 窗口级键盘:搜索态方向键/回车/二级 Esc,设置态 Esc 关设置返回搜索态
  window.addEventListener("keydown", onWindowKeydown);
});

onUnmounted(() => {
  unMoved?.();
  unResized?.();
  unlistenShow?.();
  window.removeEventListener("keydown", onWindowKeydown);
  if (geometryTimer) window.clearTimeout(geometryTimer);
  if (debounceTimer) window.clearTimeout(debounceTimer);
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
          placeholder="搜索应用、文件…"
          @input="onInput"
        />
        <button
          class="text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] text-sm"
          title="设置"
          @click="toggleSettings"
        >
          ⚙
        </button>
        <!-- 拖拽把手(P2 修复 2026-08-13):header 空白/把手处可拖窗口,悬停提亮提示可拖 -->
        <span
          class="aurora-drag-hint text-xs px-1 cursor-grab"
          title="拖动窗口移动(输入框/按钮除外)"
        >
          ⠿
        </span>
      </div>
      <!-- 结果列表禁拖:滚动条拖动/列表项点击放行 -->
      <div class="flex-1 overflow-y-auto py-1" data-tauri-drag-region="false">
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
        <div v-else-if="searchError" class="px-4 py-3 text-xs text-red-400">
          {{ searchError }}
        </div>
        <!-- 有输入且无结果:空态 -->
        <div
          v-else-if="query.trim() && allResults.length === 0"
          class="px-4 py-3 text-xs text-[var(--aurora-text-dim)]"
        >
          无匹配结果
        </div>
        <!-- 空 query:引导文案,下方展示最近打开 -->
        <div v-else-if="!query.trim()" class="px-4 py-3 text-xs text-[var(--aurora-text-dim)]">
          输入关键词搜索应用、文件
        </div>
        <!-- 有输入:应用组在前(最近打开的应用置顶带标识)、文件组在后(应用优先,其次文件;
             键盘 ↑↓ 在两组间连续移动) -->
        <template v-if="query.trim()">
          <div
            v-for="(item, i) in rankedAppResults"
            :key="item.path"
            :ref="(el) => setItemEl(el, i)"
            class="px-4 py-2 flex items-center gap-3 text-sm cursor-pointer relative"
            :class="i === selected ? 'bg-[var(--aurora-field)]' : 'hover:bg-[var(--aurora-field)]'"
            @mouseenter="selected = i"
            @click="openSelected"
          >
            <!-- 选中指示:左侧强调色竖条,键盘/鼠标选中一目了然 -->
            <span
              v-if="i === selected"
              class="absolute left-0 top-1/2 -translate-y-1/2 h-4 w-[3px] rounded-r-full bg-[var(--aurora-accent)]"
            ></span>
            <!-- 真实图标:后端 icon 字段(data URI)或 dock_get_icon 提取;未就绪回退 emoji -->
            <img
              v-if="iconFor(item)"
              :src="iconFor(item)"
              class="h-5 w-5 object-contain pointer-events-none"
              draggable="false"
              alt=""
            />
            <span v-else class="text-base">🖥️</span>
            <span class="flex-1 min-w-0 truncate">{{ item.name }}</span>
            <!-- 最近标识(搜索质量包 2026-08-13):命中结果中的最近打开应用 -->
            <span
              v-if="recentPaths.has(item.path)"
              class="shrink-0 flex items-center gap-1 text-[10px] text-[var(--aurora-text-dim)]"
            >
              <span class="inline-block h-1.5 w-1.5 rounded-full bg-[var(--aurora-accent)]"></span>
              最近
            </span>
          </div>
          <!-- 文件组(最多 8 条,后端默认一致):空结果不渲染标题与列表 -->
          <template v-if="fileResults.length">
            <div class="px-4 pt-3 pb-1 text-[10px] tracking-wide text-[var(--aurora-text-dim)]">
              文件
            </div>
            <div
              v-for="(item, i) in fileResults"
              :key="'file:' + item.path"
              :ref="(el) => setItemEl(el, rankedAppResults.length + i)"
              class="px-4 py-2 flex items-center gap-3 text-sm cursor-pointer relative"
              :class="rankedAppResults.length + i === selected ? 'bg-[var(--aurora-field)]' : 'hover:bg-[var(--aurora-field)]'"
              @mouseenter="selected = rankedAppResults.length + i"
              @click="openSelected"
            >
              <span
                v-if="rankedAppResults.length + i === selected"
                class="absolute left-0 top-1/2 -translate-y-1/2 h-4 w-[3px] rounded-r-full bg-[var(--aurora-accent)]"
              ></span>
              <!-- 类型角标:目录 📁 / 文件 📄(dock_get_icon 只认 exe/ico,普通文件不取图标) -->
              <span class="text-base">{{ item.is_dir ? "📁" : "📄" }}</span>
              <span class="max-w-[60%] truncate">{{ item.name }}</span>
              <span class="flex-1 min-w-0 truncate text-[10px] text-[var(--aurora-text-dim)]">
                {{ dirLabelOf(item) }}
              </span>
            </div>
          </template>
        </template>
        <!-- 无输入:最近打开 -->
        <template v-else>
          <div
            v-for="(item, i) in recents"
            :key="item.path"
            :ref="(el) => setItemEl(el, i)"
            class="px-4 py-2 flex items-center gap-3 text-sm cursor-pointer relative"
            :class="i === selected ? 'bg-[var(--aurora-field)]' : 'hover:bg-[var(--aurora-field)]'"
            @mouseenter="selected = i"
            @click="openSelected"
          >
            <!-- 选中指示:左侧强调色竖条,键盘/鼠标选中一目了然 -->
            <span
              v-if="i === selected"
              class="absolute left-0 top-1/2 -translate-y-1/2 h-4 w-[3px] rounded-r-full bg-[var(--aurora-accent)]"
            ></span>
            <img
              v-if="iconFor(item)"
              :src="iconFor(item)"
              class="h-5 w-5 object-contain pointer-events-none"
              draggable="false"
              alt=""
            />
            <span v-else class="text-base">🖥️</span>
            <span>{{ item.name }}</span>
          </div>
        </template>
      </div>
      <div
        class="px-4 py-2 flex items-center justify-between text-[10px] text-[var(--aurora-text-dim)] border-t border-[var(--aurora-border)]"
      >
        <span>↑↓ 选择 · Enter 打开 · Esc 清空/关闭</span>
        <!-- P2:拖拽提示(底部条空白处可拖窗口;右下角手柄调整大小) -->
        <span class="aurora-drag-hint flex items-center gap-0.5" title="底部空白处拖动窗口移动">
          <span>⠿ 拖动窗口</span>
        </span>
      </div>
    </template>
    <!-- KeepAlive 缓存设置页(2026-08-13):关闭再开保持滚动位置与组件状态,避免每次重建重复拉取 -->
    <KeepAlive>
      <Settings v-if="showSettings" @close="toggleSettings" />
    </KeepAlive>
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

<style scoped>
/* 搜索进行中指示:旋转小圆环 */
@keyframes aurora-spin {
  to {
    transform: rotate(360deg);
  }
}
.aurora-spin {
  animation: aurora-spin 0.8s linear infinite;
}
</style>
