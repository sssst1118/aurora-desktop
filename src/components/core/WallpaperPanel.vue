<script setup lang="ts">
// 2.4 静态壁纸:目录输入 + 缩略图网格 + 点击应用 + 当前壁纸高亮。
// 挂载点:Settings.vue 壁纸区块(由集成 agent 接线),毛玻璃风格与 Island/Settings 一致。
// 预览走 Tauri asset 协议(tauri.conf.json security.assetProtocol.scope 限定
// $USERPROFILE/Pictures/**);自定义目录在 scope 外时缩略图显示占位,设置仍可用。
import { ref, onMounted } from "vue";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
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
const imgErrors = ref<Record<string, boolean>>({});

function fmtSize(n: number): string {
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)}MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)}KB`;
  return `${n}B`;
}

function isCurrent(path: string): boolean {
  const cur = currentPath.value;
  return cur !== null && path.toLowerCase() === cur.toLowerCase();
}

async function refresh() {
  loading.value = true;
  error.value = "";
  try {
    entries.value = await invoke<WallpaperEntry[]>("wallpaper_list_local");
    currentPath.value = await invoke<string | null>("wallpaper_get_current");
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

onMounted(async () => {
  await store.load();
  dirInput.value = store.cfg?.wallpaper_dir ?? "";
  void refresh();
});
</script>

<template>
  <div class="space-y-3">
    <div>
      <div class="text-xs text-white/50 mb-1">壁纸目录(留空 = 默认图片目录)</div>
      <div class="flex gap-2">
        <input
          v-model="dirInput"
          class="flex-1 min-w-0 text-sm bg-white/5 rounded-lg px-3 py-1.5 outline-none focus:bg-white/10 placeholder:text-white/25"
          placeholder="如 D:\Wallpapers"
          spellcheck="false"
        />
        <button
          class="text-xs px-3 py-1.5 rounded-lg bg-blue-500/80 hover:bg-blue-500 text-white shrink-0"
          @click="saveDir"
        >
          应用目录
        </button>
        <button
          class="text-xs px-3 py-1.5 rounded-lg bg-white/10 hover:bg-white/20 text-white/80 shrink-0"
          :disabled="loading"
          @click="refresh"
        >
          {{ loading ? "加载中…" : "刷新" }}
        </button>
      </div>
    </div>

    <div v-if="error" class="text-xs text-red-300 bg-red-500/10 rounded-lg px-3 py-1.5">
      {{ error }}
    </div>
    <div v-else-if="notice" class="text-xs text-emerald-300 bg-emerald-500/10 rounded-lg px-3 py-1.5">
      {{ notice }}
    </div>

    <div v-if="entries.length > 0" class="grid grid-cols-4 gap-2">
      <button
        v-for="entry in entries"
        :key="entry.path"
        class="relative rounded-lg overflow-hidden bg-white/5 hover:bg-white/10 focus:outline-none focus:ring-1 focus:ring-blue-400/60 group text-left"
        :class="[
          isCurrent(entry.path) ? 'ring-2 ring-blue-400/90' : '',
          applying === entry.path ? 'opacity-60' : '',
        ]"
        :title="entry.path"
        @click="apply(entry.path)"
      >
        <div class="aspect-square">
          <img
            v-if="!imgErrors[entry.path]"
            :src="convertFileSrc(entry.path)"
            class="w-full h-full object-cover"
            loading="lazy"
            draggable="false"
            @error="imgErrors[entry.path] = true"
          />
          <div
            v-else
            class="w-full h-full flex items-center justify-center text-white/30 text-[10px] px-1 text-center"
          >
            预览不可用
          </div>
        </div>
        <span
          v-if="isCurrent(entry.path)"
          class="absolute top-1 right-1 text-[9px] px-1.5 py-0.5 rounded bg-blue-500/90 text-white"
        >
          当前
        </span>
        <div
          class="absolute inset-x-0 bottom-0 bg-black/60 px-1.5 py-1 text-[10px] text-white/85 truncate"
        >
          {{ entry.name }}
        </div>
        <div class="px-1.5 pt-0.5 pb-1 text-[9px] text-white/35">
          {{ fmtSize(entry.size) }}
        </div>
      </button>
    </div>
    <div v-else-if="!loading && !error" class="text-xs text-white/30 py-2 text-center">
      暂无壁纸图片
    </div>
  </div>
</template>
