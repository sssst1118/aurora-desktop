<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import type { DrawerFile } from "./types";

const props = defineProps<{ file: DrawerFile; icon: string }>();

/** 点击条目 → drawer_open(仅桌面目录内路径,后端已校验),文件/文件夹通吃 */
function open() {
  invoke<boolean>("drawer_open", { path: props.file.path }).catch((e) =>
    console.error("drawer_open failed", e),
  );
}
</script>

<template>
  <button
    class="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-white/80 hover:bg-white/10 hover:text-white transition-colors group"
    :title="file.path"
    @click="open"
  >
    <span class="text-base w-5 text-center shrink-0 leading-none">{{ icon }}</span>
    <span class="flex-1 min-w-0 truncate text-left text-[13px]">{{ file.name }}</span>
    <span
      v-if="!file.is_dir && file.ext"
      class="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-white/50 group-hover:text-white/70"
    >
      {{ file.ext.toUpperCase() }}
    </span>
  </button>
</template>
