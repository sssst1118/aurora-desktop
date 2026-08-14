<script setup lang="ts">
import { ICON_PATHS } from "./paths";

withDefaults(defineProps<{ name: string; size?: number }>(), { size: 15 });
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    v-html="ICON_PATHS[name] || ''"
    aria-hidden="true"
  />
</template>
