<script setup lang="ts">
import { onMounted } from "vue";
import { useConfigStore } from "../../stores/config";

const store = useConfigStore();
const emit = defineEmits<{ (e: "close"): void }>();

onMounted(() => {
  void store.load();
});

async function toggleIsland() {
  if (!store.cfg) return;
  store.cfg.enable_island = !store.cfg.enable_island;
  await store.save();
}

const phase2Modules = [
  { key: "enable_dock", label: "Dock 栏", note: "Phase2 开放" },
  { key: "enable_file_drawer", label: "文件抽屉", note: "Phase2 开放" },
  { key: "enable_clipboard_history", label: "剪贴板历史", note: "Phase2 开放" },
] as const;
</script>

<template>
  <div
    class="h-full w-full flex flex-col bg-black/70 backdrop-blur-xl rounded-xl overflow-hidden text-white"
  >
    <div class="flex items-center justify-between px-4 py-3 border-b border-white/10">
      <span class="text-sm">设置</span>
      <button class="text-white/40 hover:text-white/80 text-sm" title="关闭" @click="emit('close')">
        ✕
      </button>
    </div>
    <div class="flex-1 overflow-y-auto px-4 py-3 space-y-4">
      <div>
        <div class="text-xs text-white/50 mb-1">全局热键</div>
        <div class="text-sm font-mono bg-white/5 rounded px-2 py-1 inline-block">
          {{ store.cfg?.hotkey_search ?? "Ctrl+Shift+Space" }}
        </div>
        <p class="text-[10px] text-white/30 mt-1">呼出/隐藏搜索框(Phase1 固定,Phase2 支持自定义)</p>
      </div>

      <div class="flex items-center justify-between">
        <div>
          <div class="text-sm">灵动岛</div>
          <div class="text-[10px] text-white/30">顶部常驻:时间 + CPU/内存,重启后生效</div>
        </div>
        <button
          class="w-10 h-5 rounded-full relative transition-colors"
          :class="store.cfg?.enable_island ? 'bg-blue-500/80' : 'bg-white/10'"
          @click="toggleIsland"
        >
          <span
            class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
            :class="store.cfg?.enable_island ? 'left-[22px]' : 'left-0.5'"
          />
        </button>
      </div>

      <div
        v-for="item in phase2Modules"
        :key="item.key"
        class="flex items-center justify-between opacity-40"
      >
        <div>
          <div class="text-sm">{{ item.label }}</div>
          <div class="text-[10px] text-white/30">{{ item.note }}</div>
        </div>
        <div class="w-10 h-5 rounded-full bg-white/10 relative">
          <span class="absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white/60" />
        </div>
      </div>
    </div>
  </div>
</template>
