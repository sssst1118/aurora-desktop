<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

interface SysStatus {
  cpu: number;
  mem_used_mb: number;
  mem_total_mb: number;
}

const timeStr = ref("");
const cpu = ref("--");
const mem = ref("");

let timeTimer: number | undefined;
let sysTimer: number | undefined;

function pad(n: number) {
  return n.toString().padStart(2, "0");
}

function tickTime() {
  const d = new Date();
  timeStr.value = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

async function tickSys() {
  try {
    const s = (await invoke<SysStatus>("sys_get_status")) as SysStatus;
    cpu.value = `${Math.round(s.cpu)}%`;
    mem.value = `${(s.mem_used_mb / 1024).toFixed(1)}G / ${(s.mem_total_mb / 1024).toFixed(0)}G`;
  } catch (e) {
    console.error("sys_get_status failed", e);
  }
}

function openSearch() {
  invoke("open_search").catch((e) => console.error("open_search failed", e));
}

onMounted(() => {
  tickTime();
  tickSys();
  timeTimer = window.setInterval(tickTime, 1000);
  sysTimer = window.setInterval(tickSys, 2000);
});

onUnmounted(() => {
  if (timeTimer) window.clearInterval(timeTimer);
  if (sysTimer) window.clearInterval(sysTimer);
});
</script>

<template>
  <div
    class="h-full w-full flex items-center gap-4 px-4 text-white/90 select-none bg-black/40 backdrop-blur-md rounded-none cursor-pointer"
    @click="openSearch"
    data-tauri-drag-region
  >
    <span class="text-sm">🔍</span>
    <span class="font-mono text-sm">{{ timeStr }}</span>
    <span class="ml-auto text-xs text-white/70">CPU {{ cpu }}</span>
    <span class="text-xs text-white/70">内存 {{ mem }}</span>
  </div>
</template>
