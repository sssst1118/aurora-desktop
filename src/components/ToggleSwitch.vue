<script setup lang="ts">
/** 通用开关组件:v-model 布尔;滑块色走主题令牌(浅色深灰/深色白,浅色轨道上可见);
 *  自带 role="switch"/aria-checked 无障碍语义,label/description 为可选项 */
defineProps<{
  modelValue: boolean;
  label?: string;
  description?: string;
  disabled?: boolean;
}>();

const emit = defineEmits<{ (e: "update:modelValue", value: boolean): void }>();
</script>

<template>
  <div class="flex items-center justify-between">
    <div>
      <div v-if="label" class="text-sm flex items-center gap-1.5">
        <span class="w-[3px] h-3 rounded-full bg-[var(--aurora-accent)]"></span>
        {{ label }}
      </div>
      <div v-if="description" class="text-[11px] text-[var(--aurora-text-dim)]">
        {{ description }}
      </div>
    </div>
    <button
      type="button"
      role="switch"
      :aria-checked="modelValue"
      :aria-label="label"
      :disabled="disabled"
      class="w-10 h-5 rounded-full relative transition-colors shrink-0"
      :class="[
        modelValue && !disabled ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]',
        disabled ? 'opacity-40' : '',
      ]"
      @click="emit('update:modelValue', !modelValue)"
    >
      <span
        class="absolute top-0.5 w-4 h-4 rounded-full bg-[var(--aurora-switch-thumb)] shadow-[0_1px_3px_rgba(0,0,0,0.35)] transition-all"
        :class="modelValue ? 'left-[22px]' : 'left-0.5'"
      />
    </button>
  </div>
</template>
