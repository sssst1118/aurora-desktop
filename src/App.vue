<script setup lang="ts">
// Phase2 新窗口( dock / drawer / clipboard )按 label 分流渲染。
// 入口分流(island/search)在 main.ts 完成,其余 label 落到本组件。
// 【模块实现后替换】组件文件由各模块 agent 创建后,在此 import 并替换占位分支:
//   2.1 dock       -> components/core/Dock.vue
//   2.2 drawer     -> components/core/FileDrawer.vue
//   2.3 clipboard  -> components/core/ClipboardPanel.vue
import { getCurrentWindow } from "@tauri-apps/api/window";

const label = getCurrentWindow().label;
</script>

<template>
  <!-- 2.1 Dock 占位(模块实现后替换为 Dock.vue) -->
  <main
    v-if="label === 'dock'"
    class="w-full h-full flex items-center justify-center bg-black/70 text-white/60 text-sm select-none"
  >
    Aurora-Dock(骨架占位)
  </main>
  <!-- 2.2 FileDrawer 占位(模块实现后替换为 FileDrawer.vue) -->
  <main
    v-else-if="label === 'drawer'"
    class="w-full h-full flex items-center justify-center bg-black/70 text-white/60 text-sm select-none"
  >
    Aurora-FileDrawer(骨架占位)
  </main>
  <!-- 2.3 剪贴板历史占位(模块实现后替换为 ClipboardPanel.vue) -->
  <main
    v-else-if="label === 'clipboard'"
    class="w-full h-full flex items-center justify-center bg-black/70 text-white/60 text-sm select-none"
  >
    Aurora-Clipboard(骨架占位)
  </main>
  <!-- 默认兜底壳:未知窗口 label 时挂载 -->
  <main
    v-else
    class="w-full h-full flex items-center justify-center bg-black/70 text-white/60 text-sm select-none"
  >
    Aurora(未知窗口)
  </main>
</template>
