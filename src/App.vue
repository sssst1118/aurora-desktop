<script setup lang="ts">
// Phase6 一岛一窗:drawer/clipboard/ai_panel 窗口已删除(并入 search 主面板五视图),
// 本组件只剩 wallpaper 壁纸层、截图遮罩(capture-*)与未知 label 兜底。
// 入口分流(island/search)在 main.ts 完成。
import { getCurrentWindow } from "@tauri-apps/api/window";
import DynamicWallpaper from "./components/core/DynamicWallpaper.vue";
import CaptureView from "./components/core/CaptureView.vue";

const label = getCurrentWindow().label;
</script>

<template>
  <!-- Phase4 4.1 动态壁纸:wallpaper 主屏窗口 + Phase5 副屏窗口(wallpaper_<i> label)统一渲染壁纸层 -->
  <main v-if="label === 'wallpaper' || label.startsWith('wallpaper_')" class="w-full h-full">
    <DynamicWallpaper />
  </main>
  <!-- 截图遮罩(2026-08-18):capture-<i> label,透明全屏变暗 + 拖选截图 -->
  <main v-else-if="label.startsWith('capture-')" class="w-full h-full">
    <CaptureView />
  </main>
  <!-- 默认兜底壳:未知窗口 label 时挂载 -->
  <main
    v-else
    class="w-full h-full flex items-center justify-center bg-black/70 text-white/60 text-sm select-none"
  >
    Aurora(未知窗口)
  </main>
</template>
