<script setup lang="ts">
/**
 * Phase6 小桌面视图(DrawerPanel.vue 内容迁移,设计文档 §4.1 + 预览稿 renderDrawer)。
 * - 数据流与旧抽屉一致:挂载时 invoke drawer_list_files,订阅 "drawer-updated" 事件
 *   自动刷新(非轮询);点击条目 → drawer_open(逻辑收纳,文件原位不动)。
 * - 去掉了窗口级 header/footer/drag-region(由 MainPanel 壳统一),分类 tab/网格/
 *   收折/手动刷新兜底等能力原样保留。
 * - 分类 tab 样式移植预览稿 .drawer-side/.drawer-tab(无 emoji 图标,文字+计数);
 *   网格与条目复用 FileItem.vue(图标懒加载 + 缓存不变)。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { MAX_FILES, type DrawerGroup } from "../../FileDrawer/types";
import FileItem from "../../FileDrawer/FileItem.vue";
import AuroraIcon from "../../icons/AuroraIcon.vue";

defineOptions({ name: "SmallDesktopView" });

const groups = ref<DrawerGroup[]>([]);
const selected = ref("全部");
const collapsed = ref<Set<string>>(new Set());
const loading = ref(false);

/** 展示总数(后端已截断到 MAX_FILES) */
const total = computed(() => groups.value.reduce((sum, g) => sum + g.files.length, 0));

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
  <div class="h-full w-full flex flex-col min-h-0 text-[var(--aurora-text)]">
    <!-- 信息条:总数/截断提示 + 手动刷新兜底 -->
    <div
      class="flex items-center gap-2 px-4 py-1.5 border-b border-[var(--aurora-border)] text-[10.5px] text-[var(--aurora-text-dim)] shrink-0"
    >
      <span>共 <span class="num">{{ total }}</span> 项</span>
      <span v-if="truncated" class="text-[var(--aurora-warn)]">
        仅显示前 {{ MAX_FILES }} 项
      </span>
      <span class="flex-1"></span>
      <button
        class="px-2 py-0.5 rounded-md hover:bg-[var(--aurora-field)] hover:text-[var(--aurora-text)] transition-colors"
        title="重新扫描桌面"
        @click="refresh"
      >
        刷新
      </button>
    </div>

    <div class="flex flex-1 min-h-0">
      <!-- 左侧分类 tab(带计数;样式=预览稿 .drawer-side/.drawer-tab) -->
      <aside class="drawer-side">
        <button
          v-for="t in tabs"
          :key="t.category"
          class="drawer-tab"
          :class="{ on: selected === t.category }"
          :title="`${t.category}(${t.count} 项)`"
          @click="selectCategory(t.category)"
        >
          <span>{{ t.category }}</span>
          <span class="cnt num">{{ t.count }}</span>
        </button>
      </aside>

      <!-- 右侧文件列表 -->
      <main class="drawer-main">
        <!-- 加载中 -->
        <div
          v-if="loading"
          class="col-span-full flex items-center justify-center py-8 text-xs text-[var(--aurora-text-dim)]"
        >
          正在扫描桌面…
        </div>
        <!-- 空桌面空态 -->
        <div
          v-else-if="total === 0"
          class="col-span-full flex flex-col items-center justify-center gap-1 py-8 text-[var(--aurora-text-dim)]"
        >
          <div class="text-3xl mb-1">✦</div>
          <div class="text-sm">桌面空空如也</div>
          <div class="text-xs">把文件放到桌面,它们会自动出现在这里</div>
        </div>
        <!-- 分组列表(分组可收折) -->
        <template v-else>
          <section
            v-for="g in visibleGroups"
            :key="g.category"
            class="col-span-full shrink-0"
          >
            <button
              class="w-full flex items-center gap-2 px-1.5 py-1 rounded-lg hover:bg-[var(--aurora-field)] transition-colors"
              @click="toggleCollapse(g.category)"
            >
              <span
                class="text-[var(--aurora-text-dim)] transition-transform"
                :class="{ 'rotate-90': !collapsed.has(g.category) }"
              >
                <AuroraIcon name="chevron" :size="9" />
              </span>
              <span class="text-xs font-medium text-[var(--aurora-text)]">{{ g.category }}</span>
              <span
                class="text-[10px] px-1.5 rounded-full bg-[var(--aurora-field)] text-[var(--aurora-text-dim)]"
              >
                {{ g.files.length }}
              </span>
            </button>
            <!-- 图标网格(FileItem:真实图标懒加载 + 名称下注,点击打开) -->
            <div
              v-if="!collapsed.has(g.category)"
              class="mt-1.5 grid grid-cols-[repeat(auto-fill,minmax(84px,1fr))] gap-1.5"
            >
              <FileItem v-for="f in g.files" :key="f.path" :file="f" />
              <!-- 单分类视图下空组:明确提示,避免"点了分类没内容"的困惑 -->
              <div
                v-if="g.files.length === 0 && selected !== '全部'"
                class="col-span-full px-2 py-3 text-[11px] text-[var(--aurora-text-dim)]"
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

<style scoped>
/* 分类侧栏与 tab 样式移植自设计稿 aurora-v02-preview.html(.drawer-side/.drawer-tab) */
.drawer-side {
  width: 96px;
  flex: none;
  padding: 8px 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-right: 1px solid var(--aurora-border);
  overflow-y: auto;
}

.drawer-tab {
  position: relative; /* 选中态极光竖条定位基准(选中语言统一,设计文档 §5.3) */
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 7px 10px;
  border: none;
  border-radius: 9px;
  background: transparent;
  font-family: inherit;
  font-size: 13px;
  color: var(--aurora-text-dim);
  cursor: pointer;
  transition: all 0.13s ease;
  text-align: left;
}

.drawer-tab:hover {
  background: var(--aurora-field);
  color: var(--aurora-text);
}

/* 选中态:field 底 + 左侧极光渐变竖条(替代原 inset 描边,与 SearchView/剪贴板同款) */
.drawer-tab.on {
  background: var(--aurora-field);
  color: var(--aurora-text);
}

.drawer-tab.on::before {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  transform: translateY(-50%);
  height: 15px;
  width: 3px;
  border-radius: 0 99px 99px 0;
  background: linear-gradient(180deg, var(--aur-1), var(--aur-2));
  box-shadow: 0 0 8px var(--aur-2);
}

.drawer-tab .cnt {
  margin-left: auto;
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 99px;
  background: var(--aurora-field);
  color: var(--aurora-text-dim);
}

.drawer-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  padding: 12px;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(86px, 1fr));
  gap: 6px;
  align-content: start;
  overflow-y: auto;
}
</style>
