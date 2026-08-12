<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface SysStatus {
  cpu: number;
  mem_used_mb: number;
  mem_total_mb: number;
  /** 聚合接收速率 bytes/s */
  net_rx_bps: number;
  /** 聚合发送速率 bytes/s */
  net_tx_bps: number;
}

const timeStr = ref("");
const cpu = ref("--");
const mem = ref("");
const net = ref("");

let timeTimer: number | undefined;
let unlistenSys: UnlistenFn | undefined;

function pad(n: number) {
  return n.toString().padStart(2, "0");
}

function tickTime() {
  const d = new Date();
  timeStr.value = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** 字节/秒 → 可读速率(B/s、KB/s、MB/s) */
function formatRate(bps: number): string {
  if (bps >= 1024 * 1024) return `${(bps / 1024 / 1024).toFixed(1)}MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(0)}KB/s`;
  return `${bps}B/s`;
}

/** 应用后端快照(事件推送或 invoke 兜底均走这里;?? 0 兼容旧后端缺字段) */
function applyStatus(s: SysStatus) {
  cpu.value = `${Math.round(s.cpu)}%`;
  mem.value = `${(s.mem_used_mb / 1024).toFixed(1)}G / ${(s.mem_total_mb / 1024).toFixed(0)}G`;
  net.value = `↓${formatRate(s.net_rx_bps ?? 0)} ↑${formatRate(s.net_tx_bps ?? 0)}`;
}

function openSearch() {
  invoke("open_search").catch((e) => console.error("open_search failed", e));
}

onMounted(async () => {
  tickTime();
  // Phase2 2.5:后端 2s 采样线程广播,取代 Phase1 的前端 2s 轮询
  try {
    unlistenSys = await listen<SysStatus>("sys-status", (e) => applyStatus(e.payload));
  } catch (e) {
    console.error("listen sys-status failed", e);
  }
  // 首帧兜底:invoke 一次立即拿最近快照(同时幂等触发后端采样线程启动)
  try {
    applyStatus(await invoke<SysStatus>("sys_get_status"));
  } catch (e) {
    console.error("sys_get_status failed", e);
  }
  timeTimer = window.setInterval(tickTime, 1000);
});

onUnmounted(() => {
  if (timeTimer) window.clearInterval(timeTimer);
  if (unlistenSys) unlistenSys();
});
</script>

<template>
  <div
    class="h-full w-full flex items-center gap-4 px-4 text-[var(--aurora-text)] select-none bg-[var(--aurora-panel)] backdrop-blur-md rounded-none cursor-pointer"
    @click="openSearch"
    data-tauri-drag-region
  >
    <span class="text-sm">🔍</span>
    <span class="font-mono text-sm">{{ timeStr }}</span>
    <span class="ml-auto text-xs text-[var(--aurora-text)]">CPU {{ cpu }}</span>
    <span class="text-xs text-[var(--aurora-text)]">内存 {{ mem }}</span>
    <span class="text-xs text-[var(--aurora-text)] font-mono">{{ net }}</span>
  </div>
</template>
