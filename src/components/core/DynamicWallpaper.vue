<script setup lang="ts">
// 4.1 动态壁纸窗口(wallpaper label)渲染组件。
// 挂载点 = App.vue label === "wallpaper" 分支(由集成 agent 接线)。
//
// - video 素材:<video autoplay muted loop playsinline>(壁纸永不出声);
// - html 素材:<iframe> 直接渲染用户提供的页面;
// - 素材 URL 两段式(设计 §1.5):默认图片目录内 → convertFileSrc 走 asset 协议,
//   目录外 → set 命令后端返回 data URL(state.url);
// - 电池降载(设计 §1.4):收到 wallpaper-power{on_battery:true} → 暂停视频 + 遮罩提示,
//   遮罩可点"恢复播放"手动临时恢复;on_battery:false → 自动恢复播放。
// - 样式用现有硬编码风格(4.4 主题令牌合入后由集成 agent 统一迁移)。
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useConfigStore } from "../../stores/config";
import { useDynamicWallpaper } from "../../composables/useDynamicWallpaper";

const store = useConfigStore();
const { state, start, stop } = useDynamicWallpaper();

const videoRef = ref<HTMLVideoElement | null>(null);
/** 电池模式下用户手动"恢复播放"后,本周期内不再弹遮罩(下次 on_battery 事件重新武装) */
const userResumed = ref(false);

// 素材 src:目录外素材为 data URL(state.url),Pictures 内走 asset 协议
const mediaSrc = computed(() => {
  if (!state.value.path) return "";
  if (state.value.url) return state.value.url;
  return convertFileSrc(state.value.path);
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

// 电池降载遮罩:仅视频素材展示(html 素材动画由素材自身 JS 处理,此处只隐藏画面停合成)
const showBatteryOverlay = computed(
  () => state.value.on_battery && !userResumed.value && state.value.kind === "video",
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
  await store.load(); // 读 wallpaper_scale_mode 等配置
  await start(); // 拉初始状态 + 订阅 wallpaper-power
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
    <!-- video 素材(视频壁纸永不出声:muted 硬编码) -->
    <video
      v-if="state.kind === 'video' && mediaSrc"
      ref="videoRef"
      :src="mediaSrc"
      autoplay
      muted
      loop
      playsinline
      class="absolute inset-0 w-full h-full"
      :class="scaleClass"
    />
    <!-- html 素材:iframe 直接渲染用户提供的页面;电池模式隐藏画面(停合成省电) -->
    <iframe
      v-else-if="state.kind === 'html' && mediaSrc"
      :src="mediaSrc"
      class="absolute inset-0 w-full h-full border-0"
      :class="{ invisible: state.on_battery && !userResumed }"
    />
    <!-- 无素材/禁用态:纯黑底(注入 WorkerW 前不可见,注入后即黑底壁纸) -->
    <div v-else class="w-full h-full bg-black" />

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
