<script lang="ts">
// 图标缓存(模块级:抽屉刷新重建 FileItem 时复用,避免重复 IPC;
// 后端亦有内存+磁盘双缓存,这里省的是前端往返)
export const iconCache = new Map<string, string>();
</script>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { DrawerFile } from "./types";

const props = defineProps<{ file: DrawerFile }>();

const iconUrl = ref<string | undefined>(iconCache.get(props.file.path));

onMounted(async () => {
  if (iconUrl.value) return;
  try {
    const url = await invoke<string | null>("dock_get_icon", { path: props.file.path });
    if (url) {
      iconCache.set(props.file.path, url);
      iconUrl.value = url;
    }
  } catch (e) {
    console.error("dock_get_icon failed", e);
  }
});

/** 点击条目 → drawer_open(仅桌面目录内路径,后端已校验),文件/文件夹通吃 */
function open() {
  invoke<boolean>("drawer_open", { path: props.file.path }).catch((e) =>
    console.error("drawer_open failed", e),
  );
}

/** 首字符占位(无图标/图标提取失败时,Dock 同款) */
function pad(s: string) {
  return s.length > 1 ? s : s + " ";
}
</script>

<template>
  <button
    class="group flex flex-col items-center gap-1 rounded-xl px-1 py-2 transition-colors hover:bg-[var(--aurora-field)]"
    :title="`${file.path}${file.is_dir ? '（文件夹）' : ''}`"
    @click="open"
  >
    <!-- 真实图标(lnk/exe 提取目标应用图标;失败回退首字符占位) -->
    <img
      v-if="iconUrl"
      :src="iconUrl"
      class="h-10 w-10 pointer-events-none select-none"
      draggable="false"
      alt=""
    />
    <span
      v-else
      class="flex h-10 w-10 items-center justify-center rounded-lg bg-[var(--aurora-field)] text-sm font-medium text-[var(--aurora-text)]"
      >{{ pad(file.name.slice(0, 1)) }}</span
    >
    <!-- 名称(单行截断;目录带文件夹后缀提示) -->
    <span class="w-full max-w-[76px] truncate text-center text-[11px] leading-tight text-[var(--aurora-text)]">
      {{ file.is_dir ? `${file.name}/` : file.name }}
    </span>
  </button>
</template>
