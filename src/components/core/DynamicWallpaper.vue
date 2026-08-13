<script setup lang="ts">
// 4.1 动态壁纸窗口(wallpaper / wallpaper_<i> label)渲染组件。
// 挂载点 = App.vue label 前缀分支(由集成 agent 接线)。
//
// - video 素材:<video autoplay muted loop playsinline>(壁纸永不出声);
// - 素材 URL 两段式(设计 §1.5):默认图片目录内 → convertFileSrc 走 asset 协议,
//   目录外 → set 命令后端返回 data URL(state.url);
// - 电池降载(设计 §1.4):收到 wallpaper-power{on_battery:true} → 暂停视频 + 遮罩提示,
//   遮罩可点"恢复播放"手动临时恢复;on_battery:false → 自动恢复播放。
// - Phase5 多屏(设计 §2.2):
//   - 拼接模式:每屏窗口加载同一素材,素材坐标系 = 虚拟桌面 rect(各屏窗口按自身
//     在虚拟桌面中的坐标切片显示,img/video 统一用 translate 定位实现);
//   - 独立模式:每屏窗口从 get_state().monitors 取自己 index 的素材单独渲染;
//   - 多屏关:走 4.1 单值状态,行为与 Phase4 完全一致。
// - 样式用现有硬编码风格(4.4 主题令牌合入后由集成 agent 统一迁移)。
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useConfigStore } from "../../stores/config";
import { useDynamicWallpaper } from "../../composables/useDynamicWallpaper";

/** 与 Rust 侧 MonitorInfo 对应(wallpaper_multi_monitors 返回) */
interface MonitorInfo {
  index: number;
  x: number;
  y: number;
  width: number;
  height: number;
  primary: boolean;
}

const store = useConfigStore();
const { state, start, stop } = useDynamicWallpaper();

const videoRef = ref<HTMLVideoElement | null>(null);
/** 电池模式下用户手动"恢复播放"后,本周期内不再弹遮罩(下次 on_battery 事件重新武装) */
const userResumed = ref(false);

/** 本窗口对应的显示器 index:主屏 label "wallpaper" → 0;副屏 "wallpaper_<i>" → i */
const myIndex = computed(() => {
  const label = getCurrentWindow().label;
  if (label === "wallpaper") return 0;
  const m = label.match(/^wallpaper_(\d+)$/);
  return m ? Number(m[1]) : 0;
});

/** 多屏是否启用(配置驱动;get_state 的 monitors 非空时以状态为准) */
const multiEnabled = computed(() => {
  if (state.value.monitors.length > 0) return true;
  return store.cfg?.wallpaper_multi_monitor === true;
});

/** 拼接模式(多屏启用时才有意义;默认 true = 一张素材铺满虚拟桌面) */
const spanMode = computed(() => store.cfg?.wallpaper_span_mode !== false);

/** 本屏素材:多屏 → monitors 按 index 取;多屏关 → 单值状态 */
const myMaterial = computed(() => {
  if (multiEnabled.value && state.value.monitors.length > 0) {
    const mine = state.value.monitors.find((m) => m.index === myIndex.value);
    if (mine) {
      return { kind: mine.kind, path: mine.path, url: mine.url };
    }
  }
  return { kind: state.value.kind, path: state.value.path, url: state.value.url };
});

/** 素材 src:目录外素材为 data URL(url),Pictures 内走 asset 协议 */
const mediaSrc = computed(() => {
  if (!myMaterial.value.path) return "";
  if (myMaterial.value.url) return myMaterial.value.url;
  return convertFileSrc(myMaterial.value.path);
});

// ---- Phase5 拼接切片:素材坐标系 = 虚拟桌面 rect,本屏窗口显示自己的矩形区域 ----
const monitors = ref<MonitorInfo[]>([]);
const span = ref({ x: 0, y: 0, width: 1920, height: 1080 });
const offset = ref({ x: 0, y: 0 }); // 素材元素相对窗口的平移(本屏虚拟坐标 - span 原点)

/** 计算虚拟桌面并集 rect(与 Rust 侧 span_viewport 同算法) */
function spanViewport(mons: MonitorInfo[]) {
  if (mons.length === 0) return { x: 0, y: 0, width: 1920, height: 1080 };
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const m of mons) {
    minX = Math.min(minX, m.x);
    minY = Math.min(minY, m.y);
    maxX = Math.max(maxX, m.x + m.width);
    maxY = Math.max(maxY, m.y + m.height);
  }
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

/** 拼接模式下加载坐标并计算本窗口的切片偏移 */
async function refreshSlice() {
  try {
    monitors.value = await invoke<MonitorInfo[]>("wallpaper_multi_monitors");
  } catch (e) {
    console.error("wallpaper_multi_monitors failed", e);
    return;
  }
  span.value = spanViewport(monitors.value);
  const mine = monitors.value.find((m) => m.index === myIndex.value);
  offset.value = mine
    ? { x: mine.x - span.value.x, y: mine.y - span.value.y }
    : { x: 0, y: 0 };
}

/** 拼接模式:素材元素铺满虚拟桌面,平移后只露出本屏矩形(素材坐标系 = 虚拟桌面 rect);
 *  非拼接(独立/单屏):素材元素铺满本窗口(与 Phase4 行为一致) */
const sliceStyle = computed(() => {
  if (!spanMode.value) return { width: "100%", height: "100%" };
  return {
    width: `${span.value.width}px`,
    height: `${span.value.height}px`,
    transform: `translate(-${offset.value.x}px, -${offset.value.y}px)`,
  };
});

// 填充方式(配置 wallpaper_scale_mode:"cover" | "contain" | "stretch")
const scaleClass = computed(() => {
  switch (store.cfg?.wallpaper_scale_mode ?? "cover") {
    case "contain":
      return "object-contain";
    case "stretch":
      return "object-fill";
    default:
      return "object-cover";
  }
});

// 电池降载遮罩:仅视频素材展示
const showBatteryOverlay = computed(
  () => state.value.on_battery && !userResumed.value && myMaterial.value.kind === "video",
);

// 电池状态翻转:true → 暂停视频(WebView 停止解码渲染,GPU 占用归零);false → 自动恢复
watch(
  () => state.value.on_battery,
  (onBattery) => {
    if (onBattery) {
      userResumed.value = false;
      videoRef.value?.pause();
    } else {
      videoRef.value?.play().catch(() => {}); // 恢复播放(autoplay 策略失败静默)
    }
  },
);

/** 电池模式下手动恢复播放(遮罩按钮;播放到下次状态翻转或再次暂停) */
function resume() {
  userResumed.value = true;
  videoRef.value?.play().catch(() => {});
}

onMounted(async () => {
  await store.load(); // 读 wallpaper_scale_mode / 多屏开关等配置
  await start(); // 拉初始状态 + 订阅 wallpaper-power
  // 拼接模式(多屏开):拉显示器坐标计算切片偏移(热插拔后由后端重建窗口并重新挂载)
  if (multiEnabled.value && spanMode.value) {
    await refreshSlice();
  }
  await nextTick();
  // 进入窗口时已在电池模式:立即暂停(autoplay 属性只负责非电池场景首播)
  if (state.value.on_battery) {
    videoRef.value?.pause();
  }
});

onUnmounted(() => {
  stop();
});
</script>

<template>
  <div class="w-full h-full bg-black overflow-hidden relative select-none">
    <!-- 素材元素:拼接模式铺满虚拟桌面 + translate 切片;独立/单屏铺满本窗口 -->
    <div :style="sliceStyle" class="absolute left-0 top-0">
      <!-- video 素材(视频壁纸永不出声:muted 硬编码) -->
      <video
        v-if="myMaterial.kind === 'video' && mediaSrc"
        ref="videoRef"
        :src="mediaSrc"
        autoplay
        muted
        loop
        playsinline
        class="w-full h-full"
        :class="scaleClass"
      />
      <!-- 无素材/禁用态:纯黑底(注入 WorkerW 前不可见,注入后即黑底壁纸) -->
      <div v-else class="w-full h-full bg-black" />
    </div>

    <!-- 电池降载遮罩(半透明提示,可点"恢复播放") -->
    <div
      v-if="showBatteryOverlay"
      class="absolute inset-0 bg-black/50 flex flex-col items-center justify-center gap-3"
    >
      <div class="text-white text-lg">电池模式,动态壁纸已暂停</div>
      <div class="text-white/50 text-xs">为节省电量,插上电源后自动恢复</div>
      <button
        class="text-xs px-3 py-1.5 rounded-lg bg-blue-500/80 hover:bg-blue-500 text-white"
        @click="resume"
      >
        恢复播放
      </button>
    </div>
  </div>
</template>
