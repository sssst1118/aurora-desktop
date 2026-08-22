<script setup lang="ts">
// 2.4 静态壁纸:壁纸文件路径输入 + 设置(2026-08-19 用户定调:不再展示缩略图,
// 「添加地址就可以了」——最终用途就是选一个路径设为壁纸;缩略图网格与
// wallpaper_thumbnail IPC 消费端一并退役,设置页开屏不再有大图批量解码)。
// 挂载点:Settings.vue 壁纸区块,毛玻璃风格与 Island/Settings 一致。
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

const pathInput = ref("");
const applying = ref(false);
const error = ref("");
const notice = ref("");

async function apply() {
  if (applying.value) return;
  const p = pathInput.value.trim();
  error.value = "";
  notice.value = "";
  if (p === "") {
    error.value = "请先输入壁纸文件路径";
    return;
  }
  applying.value = true;
  try {
    await invoke("wallpaper_set_static", { filePath: p });
    // path 归一化展示(后端未回传,直接显示用户输入即可)
    notice.value = "壁纸已更换";
  } catch (e) {
    error.value = String(e);
  } finally {
    applying.value = false;
  }
}
</script>

<template>
  <div class="space-y-3">
    <div>
      <div class="text-xs text-[var(--aurora-text-dim)] mb-1">
        壁纸文件路径(支持 jpg/png/bmp 等常见图片格式)
      </div>
      <div class="flex gap-2">
        <input
          v-model="pathInput"
          class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-3 py-1.5 outline-none focus:bg-[var(--aurora-field)] placeholder:text-[var(--aurora-text-dim)]"
          placeholder="如 D:\Wallpapers\aurora.jpg"
          spellcheck="false"
          @keydown.enter="apply"
        />
        <button
          class="text-xs px-3 py-1.5 rounded-lg bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)] text-white shrink-0 disabled:opacity-60"
          :disabled="applying"
          @click="apply"
        >
          {{ applying ? "设置中…" : "设置壁纸" }}
        </button>
      </div>
    </div>

    <div v-if="error" class="wp-msg wp-msg-danger text-xs rounded-lg px-3 py-1.5">
      {{ error }}
    </div>
    <div v-else-if="notice" class="wp-msg wp-msg-success text-xs rounded-lg px-3 py-1.5">
      {{ notice }}
    </div>
  </div>
</template>

<style scoped>
/* 语义色令牌透明底变体:令牌值由 global.css 提供(hex/rgba 均兼容);
 * Tailwind 的 /opacity 修饰对 var() 任意值不生成样式,用 color-mix 兜底 */
.wp-msg-danger {
  background: color-mix(in srgb, var(--aurora-danger) 10%, transparent);
  color: var(--aurora-danger);
}
.wp-msg-success {
  background: color-mix(in srgb, var(--aurora-success) 10%, transparent);
  color: var(--aurora-success);
}
</style>
