<script setup lang="ts">
// 截图遮罩窗口(capture-{i} label):全屏透明 + 变暗遮罩 + 十字光标 + 拖选矩形 + 实时尺寸标注。
// 交互(设计 docs/截图功能-设计.md §②):pointerdown 起 → pointermove 更新 → pointerup 松开即截图;
// Esc/右键取消。坐标换算:clientX/Y(逻辑像素)× scaleFactor + 窗口物理位置 → 虚拟桌面物理坐标,
// 后端按该坐标裁剪 BitBlt 整幅截图(零 monitor 匹配问题)。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event"; // 全局 emit:岛窗口监听的是全局事件(窗口级 win.emit 岛收不到,实测 2026-08-18)
import { getCurrentWindow } from "@tauri-apps/api/window";

const win = getCurrentWindow();
const MIN_SEL = 5; // 逻辑像素,小于此视为误触(设计 §②)

const dragging = ref(false);
const rect = ref({ x0: 0, y0: 0, x1: 0, y1: 0 });
const cursor = ref({ x: 0, y: 0 });

/** 选区归一化(起点在左上) */
const sel = computed(() => {
  const { x0, y0, x1, y1 } = rect.value;
  return { x: Math.min(x0, x1), y: Math.min(y0, y1), w: Math.abs(x1 - x0), h: Math.abs(y1 - y0) };
});
const selStyle = computed(() => ({
  left: `${sel.value.x}px`,
  top: `${sel.value.y}px`,
  width: `${sel.value.w}px`,
  height: `${sel.value.h}px`,
}));
/** 尺寸标注:选区太靠上时放选区内侧,避免溢出窗口顶 */
const labelStyle = computed(() => ({
  left: `${sel.value.x}px`,
  top: sel.value.y >= 26 ? `${sel.value.y - 22}px` : `${sel.value.y + 4}px`,
}));
/** 选区外 4 块变暗遮罩(选区内不遮,亮区分明) */
const maskTop = computed(() => ({ top: 0, left: 0, right: 0, height: `${sel.value.y}px` }));
const maskBottom = computed(() => ({
  top: `${sel.value.y + sel.value.h}px`,
  left: 0,
  right: 0,
  bottom: 0,
}));
const maskLeft = computed(() => ({
  top: `${sel.value.y}px`,
  left: 0,
  width: `${sel.value.x}px`,
  height: `${sel.value.h}px`,
}));
const maskRight = computed(() => ({
  top: `${sel.value.y}px`,
  left: `${sel.value.x + sel.value.w}px`,
  right: 0,
  height: `${sel.value.h}px`,
}));

/** 取消(Esc/右键/误触):直接隐藏,窗口常驻复用(下次热键 show),ARMED 态去重靠后端 */
function cancel() {
  dragging.value = false;
  void win.hide();
}

/** 松开即截图:换算虚拟桌面物理坐标 → 隐藏遮罩 → invoke → 广播结果给岛窗口 */
async function finish() {
  dragging.value = false;
  const { w: lw, h: lh } = sel.value;
  if (lw < MIN_SEL || lh < MIN_SEL) {
    cancel(); // 误触
    return;
  }
  try {
    const scale = await win.scaleFactor();
    const pos = await win.outerPosition(); // PhysicalPosition(虚拟桌面物理坐标)
    const x = pos.x + Math.round(sel.value.x * scale);
    const y = pos.y + Math.round(sel.value.y * scale);
    const w = Math.round(lw * scale);
    const h = Math.round(lh * scale);
    // 先隐藏遮罩再截,防遮罩入图;等 100ms 让窗口真正不可见
    await win.hide();
    await new Promise((r) => setTimeout(r, 100));
    const res = await invoke<{ path: string; w: number; h: number; copy_ok: boolean }>(
      "screenshot_capture",
      { x, y, w, h },
    );
    await emit("screenshot-done", res);
  } catch (e) {
    await emit("screenshot-done", { error: String(e) });
  }
}

function onPointerDown(e: PointerEvent) {
  rect.value = { x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY };
  dragging.value = true;
}
function onPointerMove(e: PointerEvent) {
  cursor.value = { x: e.clientX, y: e.clientY };
  if (dragging.value) {
    rect.value.x1 = e.clientX;
    rect.value.y1 = e.clientY;
  }
}
function onPointerUp() {
  if (dragging.value) void finish();
}
function onKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    cancel();
  }
}
function onContextMenu(e: Event) {
  e.preventDefault();
  cancel();
}

onMounted(() => {
  window.addEventListener("pointerdown", onPointerDown);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("contextmenu", onContextMenu);
});
onBeforeUnmount(() => {
  window.removeEventListener("pointerdown", onPointerDown);
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("contextmenu", onContextMenu);
});
</script>

<template>
  <div class="fixed inset-0 select-none" style="cursor: crosshair">
    <!-- ARMED 常态:全屏变暗(按下后切换为选区外 4 块镂空) -->
    <div v-if="!dragging" class="absolute inset-0 bg-black/60" />
    <template v-else>
      <div class="absolute bg-black/60" :style="maskTop" />
      <div class="absolute bg-black/60" :style="maskBottom" />
      <div class="absolute bg-black/60" :style="maskLeft" />
      <div class="absolute bg-black/60" :style="maskRight" />
    </template>
    <!-- 跟随鼠标十字参考线 -->
    <div v-if="dragging" class="absolute bg-white/40 w-px" :style="{ left: `${cursor.x}px`, top: 0, bottom: 0 }" />
    <div v-if="dragging" class="absolute bg-white/40 h-px" :style="{ top: `${cursor.y}px`, left: 0, right: 0 }" />
    <!-- 选区矩形 + 实时尺寸标注 -->
    <div v-if="dragging" class="absolute border-2 border-[#5b9cff] rounded-[2px]" :style="selStyle">
      <span
        class="absolute px-1.5 py-0.5 rounded text-[11px] leading-none text-white bg-[#5b9cff] font-mono"
        :style="labelStyle"
      >
        {{ sel.w }} × {{ sel.h }}
      </span>
    </div>
  </div>
</template>
