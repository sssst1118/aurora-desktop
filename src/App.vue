<script setup lang="ts">
// Phase2 新窗口( dock / drawer / clipboard )按 label 分流渲染。
// 入口分流(island/search)在 main.ts 完成,其余 label 落到本组件。
// 组件根元素不再加背景:各面板组件自带毛玻璃背景(Dock/DrawerPanel/ClipboardPanel)。
import { getCurrentWindow } from "@tauri-apps/api/window";
import Dock from "./components/core/Dock.vue";
import DrawerPanel from "./components/FileDrawer/DrawerPanel.vue";
import ClipboardPanel from "./components/core/ClipboardPanel.vue";
import AIPanel from "./components/core/AIPanel.vue";
import DynamicWallpaper from "./components/core/DynamicWallpaper.vue";

const label = getCurrentWindow().label;
</script>

<template>
  <!-- 2.1 Dock 栏 -->
  <main v-if="label === 'dock'" class="w-full h-full">
    <Dock />
  </main>
  <!-- 2.2 FileDrawer 桌面文件抽屉 -->
  <main v-else-if="label === 'drawer'" class="w-full h-full">
    <DrawerPanel />
  </main>
  <!-- 2.3 剪贴板历史 -->
  <main v-else-if="label === 'clipboard'" class="w-full h-full">
    <ClipboardPanel />
  </main>
  <!-- 3.1 AI 对话面板 -->
  <main v-else-if="label === 'ai_panel'" class="w-full h-full">
    <AIPanel />
  </main>
  <!-- Phase4 4.1 动态壁纸:wallpaper 主屏窗口 + Phase5 副屏窗口(wallpaper_<i> label)统一渲染壁纸层 -->
  <main v-else-if="label === 'wallpaper' || label.startsWith('wallpaper_')" class="w-full h-full">
    <DynamicWallpaper />
  </main>
  <!-- 默认兜底壳:未知窗口 label 时挂载 -->
  <main
    v-else
    class="w-full h-full flex items-center justify-center bg-black/70 text-white/60 text-sm select-none"
  >
    Aurora(未知窗口)
  </main>
</template>
