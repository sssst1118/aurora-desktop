<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { MAX_FILES, iconOf, type DrawerGroup } from "./types";
import FileItem from "./FileItem.vue";

/**
 * 2.2 FileDrawer 桌面文件抽屉主面板。
 * 毛玻璃风格与 Island 一致;接线(挂载进 App.vue drawer 分支)由集成 agent 完成。
 * 数据流:显示时先 invoke drawer_list_files,再订阅 "drawer-updated" 事件自动刷新;
 * 点击条目 → drawer_open(逻辑收纳,文件原位不动)。
 */

const groups = ref<DrawerGroup[]>([]);
const selected = ref("全部");
const collapsed = ref<Set<string>>(new Set());
const loading = ref(false);

/** 展示总数(后端已截断到 MAX_FILES) */
const total = computed(() =>
  groups.value.reduce((sum, g) => sum + g.files.length, 0),
);

/** 是否被后端截断(总数到上限时提示条说明) */
const truncated = computed(() => total.value >= MAX_FILES);

/** 左侧分类 tab:全部 + 各组(带计数) */
const tabs = computed(() => [
  { category: "全部", count: total.value },
  ...groups.value.map((g) => ({ category: g.category, count: g.files.length })),
]);

/** 右侧内容:选中"全部"时展示全部分组(可收折),否则只展示选中分组 */
const visibleGroups = computed(() =>
  selected.value === "全部"
    ? groups.value
    : groups.value.filter((g) => g.category === selected.value),
);

async function load() {
  loading.value = true;
  try {
    groups.value = await invoke<DrawerGroup[]>("drawer_list_files");
  } catch (e) {
    console.error("drawer_list_files failed", e);
  } finally {
    loading.value = false;
  }
}

/** 手动刷新兜底按钮 */
async function refresh() {
  loading.value = true;
  try {
    groups.value = await invoke<DrawerGroup[]>("drawer_refresh");
  } catch (e) {
    console.error("drawer_refresh failed", e);
  } finally {
    loading.value = false;
  }
}

/** 分组收折切换 */
function toggleCollapse(category: string) {
  const next = new Set(collapsed.value);
  if (next.has(category)) {
    next.delete(category);
  } else {
    next.add(category);
  }
  collapsed.value = next;
}

/** 左侧分类 tab 点击:切换选中;若该分组曾在"全部"视图下被折叠,
 *  切换时自动展开,避免折叠状态残留导致右侧看不到文件 */
function selectCategory(category: string) {
  selected.value = category;
  if (category !== "全部" && collapsed.value.has(category)) {
    const next = new Set(collapsed.value);
    next.delete(category);
    collapsed.value = next;
  }
}

/** 右上角关闭按钮:隐藏窗口(热键/托盘可再次呼出) */
function closeWindow() {
  getCurrentWindow()
    .hide()
    .catch((e) => console.error("drawer hide failed", e));
}

let unlisten: UnlistenFn | undefined;

onMounted(async () => {
  await load();
  // watcher 事件驱动刷新:后端重扫后 emit 信号,前端拉取(非轮询)
  unlisten = await listen("drawer-updated", () => {
    void load();
  });
});

onUnmounted(() => {
  unlisten?.();
});
</script>

<template>
  <div
    class="w-full h-full flex flex-col bg-[var(--aurora-panel)] backdrop-blur-xl rounded-2xl border border-[var(--aurora-border)] overflow-hidden text-[var(--aurora-text)] shadow-2xl"
  >
    <!-- 头部(可拖动区域) -->
    <header
      class="flex items-center gap-2.5 px-4 py-3 border-b border-[var(--aurora-border)] shrink-0 cursor-move"
      data-tauri-drag-region
    >
      <span class="text-lg leading-none" data-tauri-drag-region>🗂️</span>
      <h1 class="text-sm font-medium tracking-wide" data-tauri-drag-region>
        桌面文件抽屉
      </h1>
      <span class="text-xs text-[var(--aurora-text-dim)]" data-tauri-drag-region>
        共 {{ total }} 项
      </span>
      <span v-if="truncated" class="text-[10px] text-amber-300/80">
        仅显示前 {{ MAX_FILES }} 项
      </span>
      <div class="ml-auto flex items-center gap-1">
        <button
          class="w-7 h-7 rounded-lg text-sm text-[var(--aurora-text-dim)] hover:bg-[var(--aurora-field)] hover:text-[var(--aurora-text)] transition-colors"
          title="刷新"
          @click="refresh"
        >
          🔄
        </button>
        <button
          class="w-7 h-7 rounded-lg text-sm text-[var(--aurora-text-dim)] hover:bg-[var(--aurora-field)] hover:text-[var(--aurora-text)] transition-colors"
          title="隐藏"
          @click="closeWindow"
        >
          ✕
        </button>
      </div>
    </header>

    <div class="flex flex-1 min-h-0">
      <!-- 左侧分类 tab(带计数) -->
      <aside
        class="w-32 shrink-0 border-r border-[var(--aurora-border)] p-2 space-y-1 overflow-y-auto"
      >
        <button
          v-for="t in tabs"
          :key="t.category"
          class="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs transition-colors"
          :class="
            selected === t.category
              ? 'bg-[var(--aurora-field)] text-[var(--aurora-text)]'
              : 'text-[var(--aurora-text-dim)] hover:bg-[var(--aurora-field)]'
          "
          @click="selectCategory(t.category)"
        >
          <span class="text-sm leading-none">{{ iconOf(t.category, false) }}</span>
          <span class="flex-1 min-w-0 text-left truncate">{{ t.category }}</span>
          <span
            class="shrink-0 text-[10px] px-1.5 rounded-full bg-[var(--aurora-field)] text-[var(--aurora-text-dim)]"
          >
            {{ t.count }}
          </span>
        </button>
      </aside>

      <!-- 右侧文件列表 -->
      <main class="flex-1 min-w-0 flex flex-col p-3 gap-3 overflow-y-auto">
        <!-- 加载中 -->
        <div
          v-if="loading"
          class="flex-1 flex items-center justify-center text-xs text-[var(--aurora-text-dim)]"
        >
          正在扫描桌面…
        </div>
        <!-- 空桌面空态 -->
        <div
          v-else-if="total === 0"
          class="flex-1 flex flex-col items-center justify-center gap-1 text-[var(--aurora-text-dim)]"
        >
          <div class="text-3xl mb-1">✨</div>
          <div class="text-sm">桌面空空如也</div>
          <div class="text-xs text-[var(--aurora-text-dim)]">
            把文件放到桌面,它们会自动出现在这里
          </div>
        </div>
        <!-- 分组列表(分组可收折) -->
        <template v-else>
          <section v-for="g in visibleGroups" :key="g.category" class="shrink-0">
            <button
              class="w-full flex items-center gap-2 px-1.5 py-1 rounded-lg hover:bg-[var(--aurora-field)] transition-colors"
              @click="toggleCollapse(g.category)"
            >
              <span
                class="text-[9px] text-[var(--aurora-text-dim)] transition-transform"
                :class="{ 'rotate-90': !collapsed.has(g.category) }"
              >
                ▶
              </span>
              <span class="text-sm leading-none">{{ iconOf(g.category, false) }}</span>
              <span class="text-xs font-medium text-[var(--aurora-text)]">{{ g.category }}</span>
              <span
                class="text-[10px] px-1.5 rounded-full bg-[var(--aurora-field)] text-[var(--aurora-text-dim)]"
              >
                {{ g.files.length }}
              </span>
            </button>
            <div v-if="!collapsed.has(g.category)" class="mt-1 space-y-0.5">
              <FileItem
                v-for="f in g.files"
                :key="f.path"
                :file="f"
                :icon="iconOf(g.category, f.is_dir)"
              />
              <!-- 单分类视图下空组:明确提示,避免"点了分类没内容"的困惑 -->
              <div
                v-if="g.files.length === 0 && selected !== '全部'"
                class="px-2 py-2 text-[11px] text-[var(--aurora-text-dim)]"
              >
                该分类暂无文件
              </div>
            </div>
          </section>
        </template>
      </main>
    </div>
  </div>
</template>
