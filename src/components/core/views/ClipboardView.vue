<script setup lang="ts">
/**
 * Phase6 剪贴板视图(ClipboardPanel.vue 内容迁移,设计文档 §4.1 + 预览稿 renderClip)。
 * - 去掉了窗口级根/标题栏/拖拽把手/底部提示条(由 MainPanel 壳统一);
 *   搜索过滤 + 清空(两层确认)+ 回贴 + 单条删除能力原样保留。
 * - 回贴成功后隐藏面板(贴近 Win+V 交互,焦点回到原应用);
 * - 数据刷新时机从"窗口显示(tauri://show)"改为 KeepAlive 激活(onActivated):
 *   窗口呼出且视图为剪贴板时刷新,视图切换切回时同样刷新,语义更准。
 * - 键盘 ↑↓/Enter 绑定在搜索框上(焦点在框内时生效);Esc 由壳统一处理。
 * 样式移植预览稿 .clip-list/.clip-item/.type-ico/.clip-del。
 */
import { computed, nextTick, onActivated, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useClipboardStore, type ClipboardItem, type HistoryEntry } from "../../../stores/clipboard";
import { useClipboardHistory } from "../../../composables/useClipboardHistory";
import AuroraIcon from "../../icons/AuroraIcon.vue";

defineOptions({ name: "ClipboardView" });

const store = useClipboardStore();
const win = getCurrentWindow();
const inputEl = ref<HTMLInputElement | null>(null);

const { start, stop } = useClipboardHistory();

const list = computed(() => store.filtered);

function pad(n: number) {
  return n.toString().padStart(2, "0");
}

function fmtTime(ts: number): string {
  const d = new Date(ts * 1000);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 内容摘要:折叠换行为空格,避免列表撑高 */
function summary(item: ClipboardItem): string {
  return item.payload.replace(/\s+/g, " ").trim();
}

async function copyBack(index: number) {
  try {
    await store.copyBack(index);
    // 回贴后自动隐藏面板,贴近系统 Win+V 交互(焦点回到原应用)
    await win.hide();
  } catch (e) {
    console.error("copy back failed", e);
  }
}

// 单条删除(悬停 ✕):不弹确认(单条误删代价小,与全量清空的两层确认区分);
// 契约 clipboard_delete_item { index: 全量列表下标 },index 越界后端返回 Err,成功返回删除后条数
const deleteError = ref("");
let deleteErrorTimer: number | undefined;

async function deleteItem(entry: HistoryEntry) {
  try {
    await invoke<number>("clipboard_delete_item", { index: entry.index });
    // 后端删除成功,本地列表同步移除(index 口径与 copyBack 一致 = 全量列表下标)
    store.items.splice(entry.index, 1);
    if (store.selected >= list.value.length) {
      store.selected = Math.max(0, list.value.length - 1);
    }
    deleteError.value = "";
  } catch (e) {
    deleteError.value = `删除失败:${e}`;
    if (deleteErrorTimer) window.clearTimeout(deleteErrorTimer);
    deleteErrorTimer = window.setTimeout(() => {
      deleteError.value = "";
    }, 3000);
  }
}

// 清空确认:两层点击(第一击进入确认态,3s 未确认自动复原),成功后短暂显示"已清空"
const confirming = ref(false);
const cleared = ref(false);
let confirmTimer: number | undefined;
let clearedTimer: number | undefined;

async function clearAll() {
  try {
    await store.clear();
    cleared.value = true;
    if (clearedTimer) window.clearTimeout(clearedTimer);
    clearedTimer = window.setTimeout(() => {
      cleared.value = false;
    }, 1500);
  } catch (e) {
    console.error("clear history failed", e);
  }
}

function onClickClear() {
  if (!confirming.value) {
    confirming.value = true;
    if (confirmTimer) window.clearTimeout(confirmTimer);
    confirmTimer = window.setTimeout(() => {
      confirming.value = false;
    }, 3000);
    return;
  }
  if (confirmTimer) window.clearTimeout(confirmTimer);
  confirmTimer = undefined;
  confirming.value = false;
  void clearAll();
}

/** 搜索框键盘(焦点在框内时):↑↓ 选择 / Enter 回贴;Esc 由壳统一处理 */
function onKeydown(e: KeyboardEvent) {
  if (e.key === "ArrowDown") {
    e.preventDefault();
    if (list.value.length) store.selected = (store.selected + 1) % list.value.length;
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    if (list.value.length) {
      store.selected = (store.selected - 1 + list.value.length) % list.value.length;
    }
  } else if (e.key === "Enter") {
    e.preventDefault();
    const entry = list.value[store.selected];
    if (entry) void copyBack(entry.index);
  }
}

/** 激活(窗口呼出且剪贴板为当前视图 / 切回本视图)时:拉最新历史并聚焦 */
function onActivatedView() {
  void store.refresh().catch((e) => console.error("load history failed", e));
  void nextTick().then(() => inputEl.value?.focus());
}

onMounted(() => {
  void start(); // 事件订阅一次即可(组件销毁时 stop)
});

// 首次挂载后 activated 必触发一次,数据拉取与聚焦统一走这里
onActivated(onActivatedView);

onUnmounted(() => {
  stop();
  if (confirmTimer) window.clearTimeout(confirmTimer);
  if (clearedTimer) window.clearTimeout(clearedTimer);
  if (deleteErrorTimer) window.clearTimeout(deleteErrorTimer);
});
</script>

<template>
  <div class="h-full w-full flex flex-col min-h-0 select-none">
    <!-- 工具行:搜索框 + 清空(两层确认) -->
    <div class="flex items-center gap-2 px-4 py-2 border-b border-[var(--aurora-border)] shrink-0">
      <AuroraIcon name="search" :size="13" class="shrink-0 text-[var(--aurora-text-dim)]" />
      <input
        ref="inputEl"
        v-model="store.keyword"
        class="flex-1 min-w-0 bg-transparent outline-none text-[13px] placeholder:text-[var(--aurora-text-dim)]"
        placeholder="搜索剪贴板历史…"
        @keydown="onKeydown"
      />
      <button
        class="text-xs shrink-0 transition-colors"
        :class="
          cleared
            ? 'text-[var(--aurora-success)]'
            : confirming
              ? 'text-[var(--aurora-danger)]'
              : 'text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)]'
        "
        :title="confirming ? '再次点击确认清空(本地文件一并删除)' : '清空历史(本地文件一并删除)'"
        @click="onClickClear"
      >
        {{ cleared ? "已清空" : confirming ? "确认清空?" : "清空" }}
      </button>
    </div>

    <!-- 历史列表 -->
    <div class="clip-list flex-1 min-h-0">
      <!-- 单条删除失败提示(3s 自动消失) -->
      <div
        v-if="deleteError"
        class="mx-2 mt-1 rounded-lg bg-[var(--aurora-danger-bg)] px-3 py-1 text-xs text-[var(--aurora-danger)]"
      >
        {{ deleteError }}
      </div>
      <div
        v-if="store.items.length === 0"
        class="px-4 py-6 text-center text-xs text-[var(--aurora-text-dim)]"
      >
        暂无历史,复制文本后自动记录
      </div>
      <div
        v-else-if="list.length === 0"
        class="px-4 py-6 text-center text-xs text-[var(--aurora-text-dim)]"
      >
        无匹配结果
      </div>
      <div
        v-for="(entry, i) in list"
        :key="`${entry.item.ts}-${i}`"
        class="clip-item"
        :class="{ selected: i === store.selected }"
        :style="{ animationDelay: i * 28 + 'ms' }"
        @mouseenter="store.selected = i"
        @click="copyBack(entry.index)"
      >
        <span class="type-ico">
          <AuroraIcon :name="entry.item.tp === 'image' ? 'file' : 'copy'" :size="13" />
        </span>
        <span class="txt">
          <span class="summary block">
            {{ entry.item.tp === "image" ? entry.item.payload : summary(entry.item) }}
          </span>
          <span class="meta num">{{ fmtTime(entry.item.ts) }}</span>
        </span>
        <!-- 悬停 ✕ 单条删除(对标 Win+V;stop 阻止冒泡触发回贴) -->
        <button
          class="clip-del"
          title="删除该条"
          @click.stop="deleteItem(entry)"
        >
          <AuroraIcon name="close" :size="10" />
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 样式移植自设计稿 aurora-v02-preview.html(.clip-list/.clip-item/.type-ico/.clip-del) */
.clip-list {
  padding: 6px 8px;
  overflow-y: auto;
}

.clip-item {
  position: relative; /* 选中态极光竖条定位基准(选中语言统一,设计文档 §5.3) */
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 9px 12px;
  border-radius: 10px;
  cursor: pointer;
  animation: rise-in 0.22s ease both;
  transition: background 0.1s ease;
}

.clip-item:hover,
.clip-item.selected {
  background: var(--aurora-field);
}

/* 选中态:左侧极光渐变竖条 + 光晕(与 SearchView .result-item.selected 同款) */
.clip-item.selected::before {
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

.type-ico {
  flex: none;
  width: 24px;
  height: 24px;
  display: grid;
  place-items: center;
  border-radius: 7px;
  background: var(--aurora-field);
  color: var(--aurora-text-dim);
}

.txt {
  flex: 1;
  min-width: 0;
}

.summary {
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.meta {
  font-size: 11px;
  color: var(--aurora-text-dim);
  margin-top: 2px;
}

.clip-del {
  flex: none;
  width: 22px;
  height: 22px;
  display: grid;
  place-items: center;
  border: none;
  border-radius: 99px;
  background: transparent;
  color: var(--aurora-text-dim);
  cursor: pointer;
  opacity: 0;
  transition: all 0.14s ease;
}

.clip-item:hover .clip-del {
  opacity: 1;
}

.clip-del:hover {
  background: var(--aurora-danger);
  color: #fff;
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
  .clip-item {
    animation-duration: 0.01s;
  }
}
</style>
