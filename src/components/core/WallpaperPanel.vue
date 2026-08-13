<script setup lang="ts">
// 2.4 静态壁纸:目录输入 + 缩略图网格 + 点击应用 + 当前壁纸高亮。
// 挂载点:Settings.vue 壁纸区块(由集成 agent 接线),毛玻璃风格与 Island/Settings 一致。
// 预览不走 asset 协议——scope 只认 Tauri 变量集($HOME/$PICTURE 等,Windows 环境变量
// $USERPROFILE 不识别会当字面路径)且无运行时扩展授权 API,自定义壁纸目录(如
// C:\ProgramData\Lenovo\Themes)一律 403 预览全挂;改为后端缩略图命令
// wallpaper_thumbnail:任意目录均可读,缩到 480px JPEG base64 data URI 传回。
import { ref, watch, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useConfigStore } from "../../stores/config";

interface WallpaperEntry {
  name: string;
  path: string;
  size: number;
}

const store = useConfigStore();

const entries = ref<WallpaperEntry[]>([]);
const currentPath = ref<string | null>(null);
const dirInput = ref("");
const loading = ref(false);
const applying = ref<string | null>(null);
const error = ref("");
const notice = ref("");
// path → data URI(缩略图);加载中 = 未出现;失败 = thumbErrors
const thumbs = ref<Record<string, string>>({});
const thumbErrors = ref<Record<string, boolean>>({});
let disposed = false;

function fmtSize(n: number): string {
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)}MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)}KB`;
  return `${n}B`;
}

function isCurrent(path: string): boolean {
  const cur = currentPath.value;
  return cur !== null && path.toLowerCase() === cur.toLowerCase();
}

/** 逐张生成缩略图(后台 invoke;组件卸载后丢弃结果) */
async function loadThumb(path: string) {
  try {
    const uri = await invoke<string>("wallpaper_thumbnail", { filePath: path });
    if (!disposed) thumbs.value[path] = uri;
  } catch {
    if (!disposed) thumbErrors.value[path] = true;
  }
}

/** 缩略图拉取并发上限:数百张全量并发会拥堵 IPC,信号量限流分批 */
const THUMB_CONCURRENCY = 4;

/** 固定并发拉取缩略图:N 个 worker 各自取下一个路径直到取完(失败不阻塞后续) */
async function loadThumbs(paths: string[]) {
  const total = paths.length;
  if (total === 0) return;
  let next = 0;
  const worker = async () => {
    while (next < total) {
      await loadThumb(paths[next++]);
    }
  };
  const n = Math.min(THUMB_CONCURRENCY, total);
  await Promise.all(Array.from({ length: n }, () => worker()));
}

async function refresh() {
  loading.value = true;
  error.value = "";
  try {
    entries.value = await invoke<WallpaperEntry[]>("wallpaper_list_local");
    currentPath.value = await invoke<string | null>("wallpaper_get_current");
    thumbs.value = {};
    thumbErrors.value = {};
    // 缩略图后台 4 路并发拉取,不阻塞列表展示(loading 只管列表本身)
    void loadThumbs(entries.value.map((e) => e.path));
    if (entries.value.length === 0) {
      error.value = `目录中没有可用图片:${store.cfg?.wallpaper_dir ?? "默认目录"}`;
    }
  } catch (e) {
    error.value = `读取壁纸列表失败:${e}`;
  } finally {
    loading.value = false;
  }
}

async function saveDir() {
  error.value = "";
  notice.value = "";
  if (!store.cfg) return;
  const v = dirInput.value.trim();
  store.cfg.wallpaper_dir = v === "" ? null : v;
  try {
    await store.save();
    await refresh();
  } catch (e) {
    error.value = `保存目录失败:${e}`;
  }
}

async function apply(path: string) {
  if (applying.value) return;
  applying.value = path;
  error.value = "";
  notice.value = "";
  try {
    await invoke("wallpaper_set_static", { filePath: path });
    currentPath.value = path;
    notice.value = "壁纸已更换";
  } catch (e) {
    error.value = String(e);
  } finally {
    applying.value = null;
  }
}

onMounted(() => {
  // 配置由 Settings 统一加载(本组件不再重复 store.load());
  // 子组件 mounted 先于 Settings,cfg 可能尚未就绪,watch 等它回填目录输入框一次。
  // ⚠️ stop 用 let + 可选调用:immediate 回调在 watch 返回前同步执行,若回调内直接
  // 引用 const stopSync 会命中 TDZ 抛 ReferenceError(P1 根因 2026-08-13:dev 下打开
  // 设置页时 cfg 已就绪,回调走到底即崩,中断当次渲染,表现为"按钮点击落盘但界面不切换")
  let stop: (() => void) | null = null;
  stop = watch(
    () => store.cfg?.wallpaper_dir,
    (v) => {
      if (v === undefined) return; // cfg 未加载完成,继续等
      dirInput.value = v ?? "";
      stop?.();
    },
    { immediate: true },
  );
  void refresh();
});

onUnmounted(() => {
  disposed = true;
});
</script>

<template>
  <div class="space-y-3">
    <div>
      <div class="text-xs text-[var(--aurora-text-dim)] mb-1">壁纸目录(留空 = 默认图片目录)</div>
      <div class="flex gap-2">
        <input
          v-model="dirInput"
          class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-3 py-1.5 outline-none focus:bg-[var(--aurora-field)] placeholder:text-[var(--aurora-text-dim)]"
          placeholder="如 D:\Wallpapers"
          spellcheck="false"
        />
        <button
          class="text-xs px-3 py-1.5 rounded-lg bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)] text-white shrink-0"
          @click="saveDir"
        >
          应用目录
        </button>
        <button
          class="text-xs px-3 py-1.5 rounded-lg bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] text-[var(--aurora-text)] shrink-0"
          :disabled="loading"
          @click="refresh"
        >
          {{ loading ? "加载中…" : "刷新" }}
        </button>
      </div>
    </div>

    <div v-if="error" class="wp-msg wp-msg-danger text-xs rounded-lg px-3 py-1.5">
      {{ error }}
    </div>
    <div v-else-if="notice" class="wp-msg wp-msg-success text-xs rounded-lg px-3 py-1.5">
      {{ notice }}
    </div>

    <div v-if="entries.length > 0" class="grid grid-cols-4 gap-2">
      <button
        v-for="entry in entries"
        :key="entry.path"
        class="relative rounded-lg overflow-hidden bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)] group text-left"
        :class="[
          isCurrent(entry.path) ? 'ring-2 ring-[var(--aurora-accent)]' : '',
          applying === entry.path ? 'opacity-60' : '',
        ]"
        :title="entry.path"
        @click="apply(entry.path)"
      >
        <div class="aspect-square">
          <img
            v-if="thumbs[entry.path]"
            :src="thumbs[entry.path]"
            class="w-full h-full object-cover"
            draggable="false"
          />
          <div
            v-else-if="thumbErrors[entry.path]"
            class="w-full h-full flex items-center justify-center text-[var(--aurora-text-dim)] text-[10px] px-1 text-center"
          >
            预览不可用
          </div>
          <div
            v-else
            class="w-full h-full flex items-center justify-center text-[var(--aurora-text-dim)] text-[10px] animate-pulse"
          >
            加载中…
          </div>
        </div>
        <span
          v-if="isCurrent(entry.path)"
          class="absolute top-1 right-1 text-[9px] px-1.5 py-0.5 rounded bg-[var(--aurora-accent)] text-white"
        >
          当前
        </span>
        <div
          class="absolute inset-x-0 bottom-0 bg-black/60 px-1.5 py-1 text-[10px] text-white/85 truncate"
        >
          {{ entry.name }}
        </div>
        <div class="px-1.5 pt-0.5 pb-1 text-[9px] text-[var(--aurora-text-dim)]">
          {{ fmtSize(entry.size) }}
        </div>
      </button>
    </div>
    <div v-else-if="!loading && !error" class="text-xs text-[var(--aurora-text-dim)] py-2 text-center">
      暂无壁纸图片
    </div>
  </div>
</template>

<style scoped>
/* 语义色令牌透明底变体:令牌值由 global.css 提供(hex/rgba 均兼容);
 * Tailwind 的 /opacity 修饰对 var() 任意值不生成样式,用 color-mix 兜底 */
.wp-msg-danger {
  background: color-mix(in srgb, var(--aurora-danger) 10%, transparent);
  color: var(--aurora-danger);
}
.wp-msg-success {
  background: color-mix(in srgb, var(--aurora-success) 10%, transparent);
  color: var(--aurora-success);
}
</style>
