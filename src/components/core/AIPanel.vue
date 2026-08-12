<script setup lang="ts">
/**
 * 3.1 AI 对话面板(设计文档 §1.4 + §2.4 动作条)。
 * - 顶部栏:标题"AI 助手" + 设置按钮(打开 Settings,Phase2 内嵌在 search 窗口)+ 清空对话;
 * - 消息列表:用户右 / 助手左;助手消息纯文本 + `<pre>` 代码块(不引 markdown 库);
 *   流式期间光标 `▋`;工具动作 chip(如"🔧 正在打开 记事本");
 * - 输入框:Enter 发送 / Shift+Enter 换行;streaming 时回车改为停止;
 * - 错误条:error 事件红色提示,可关闭;
 * - 样式:深色卡片风(bg-gray-950/95 + 边框),不透明窗口,与 Phase2 面板视觉一致。
 * 挂载:App.vue 的 ai_panel 分支(集成 agent 接线),本文件只写组件。
 */
import { ref, watch, nextTick, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useAiChat } from "../../composables/useAiChat";

const { messages, streaming, error, send, stop, clear } = useAiChat();

const input = ref("");
const inputEl = ref<HTMLTextAreaElement | null>(null);
const listEl = ref<HTMLDivElement | null>(null);

/** 打开设置:Settings 内嵌在 search 窗口的 SearchBar(Phase2 模式),走 open_search 命令呼出 */
function openSettings() {
  invoke("open_search").catch((e) => console.error("open_search failed", e));
}

function dismissError() {
  error.value = null;
}

function submit() {
  const t = input.value;
  if (!t.trim()) return;
  input.value = "";
  void send(t);
}

function onClickSend() {
  if (streaming.value) stop();
  else submit();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    if (streaming.value) stop();
    else submit();
  }
}

function scrollToBottom() {
  void nextTick(() => {
    const el = listEl.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

// 消息内容变化(含流式增量)时自动滚底;仅流式期间跟随滚动
watch(
  messages,
  () => {
    if (streaming.value) scrollToBottom();
  },
  { deep: true },
);
watch(streaming, (v) => {
  if (!v) scrollToBottom();
});

let unlistenShow: UnlistenFn | undefined;

onMounted(async () => {
  void nextTick().then(() => inputEl.value?.focus());
  // 窗口真正显示时(tauri://show)聚焦输入框;不绑焦点事件——
  // 焦点在窗口激活抖动(拖拽/缩放/点击回窗)时也会触发,会打断面板内正在进行的操作
  unlistenShow = await listen("tauri://show", () => {
    void nextTick().then(() => inputEl.value?.focus());
  });
});

onUnmounted(() => {
  unlistenShow?.();
});

interface Block {
  code: boolean;
  text: string;
}

/** 把助手文本按 ``` 围栏拆成 文本/代码 块(轻量,不引 markdown 库) */
function splitBlocks(text: string): Block[] {
  const parts: Block[] = [];
  const re = /```([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m.index > last) parts.push({ code: false, text: text.slice(last, m.index) });
    // 围栏内首行若为语言标记(如 javascript)仅作展示提示,内容去掉该行
    const lines = m[1].split("\n");
    const content = lines.length > 1 ? lines.slice(1).join("\n") : m[1];
    parts.push({ code: true, text: content });
    last = m.index + m[0].length;
  }
  if (last < text.length) parts.push({ code: false, text: text.slice(last) });
  return parts;
}
</script>

<template>
  <div
    class="h-full w-full flex flex-col bg-[var(--aurora-panel-solid)] border border-[var(--aurora-border)] rounded-xl overflow-hidden text-[var(--aurora-text)] select-none"
  >
    <!-- 顶部栏 -->
    <div class="flex items-center gap-2 px-4 py-3 border-b border-[var(--aurora-border)] shrink-0">
      <span class="text-sm">✨ AI 助手</span>
      <div class="flex-1" />
      <button
        class="text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] text-sm shrink-0"
        title="设置(AI 服务商/密钥/模型)"
        @click="openSettings"
      >
        ⚙
      </button>
      <button
        class="text-xs text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] shrink-0"
        title="清空对话"
        @click="clear"
      >
        清空
      </button>
    </div>

    <!-- 错误条(可关闭) -->
    <div
      v-if="error"
      class="mx-3 mt-2 px-3 py-2 rounded-lg bg-red-500/15 border border-red-500/30 text-red-300 text-xs flex items-center gap-2 shrink-0"
    >
      <span class="flex-1 break-all">{{ error }}</span>
      <button class="text-red-300/70 hover:text-red-300 shrink-0" title="关闭" @click="dismissError">
        ✕
      </button>
    </div>

    <!-- 消息列表 -->
    <div ref="listEl" class="flex-1 overflow-y-auto px-4 py-3 space-y-2.5">
      <div
        v-if="messages.length === 0"
        class="h-full flex items-center justify-center text-xs text-[var(--aurora-text-dim)] leading-relaxed text-center px-6"
      >
        输入消息开始与 AI 对话<br />可调用工具:打开应用 / 搜文件 / 设壁纸 / 查系统状态…
      </div>

      <template v-for="(m, i) in messages" :key="i">
        <!-- 工具动作 chip(§2.4) -->
        <div v-if="m.role === 'tool'" class="flex justify-center">
          <div
            class="max-w-[90%] px-2.5 py-1 rounded-full bg-amber-500/10 border border-amber-500/30 text-amber-200/90 text-[11px] truncate"
            :title="m.content"
          >
            {{ m.content }}
          </div>
        </div>

        <!-- 用户消息(右) -->
        <div v-else-if="m.role === 'user'" class="flex justify-end">
          <div
            class="max-w-[85%] px-3 py-2 rounded-lg bg-blue-500/20 border border-blue-500/25 text-sm whitespace-pre-wrap break-words"
          >
            {{ m.content }}
          </div>
        </div>

        <!-- 助手消息(左,纯文本 + 代码块) -->
        <div v-else-if="m.role === 'assistant'" class="flex justify-start">
          <div
            class="max-w-[85%] px-3 py-2 rounded-lg bg-[var(--aurora-field)] border border-[var(--aurora-border)] text-sm leading-relaxed min-w-0"
          >
            <template v-if="m.content">
              <template v-for="(b, j) in splitBlocks(m.content)" :key="j">
                <pre
                  v-if="b.code"
                  class="mt-1 mb-1 p-2 rounded bg-black/50 border border-[var(--aurora-border)] text-xs overflow-x-auto font-mono text-emerald-200/90"
                  >{{ b.text }}</pre
                >
                <p v-else class="whitespace-pre-wrap break-words">{{ b.text }}</p>
              </template>
            </template>
            <span
              v-if="streaming && m.role === 'assistant' && i === messages.length - 1"
              class="inline-block w-[6px] h-[14px] bg-[var(--aurora-accent)] align-text-bottom ml-0.5 animate-pulse"
            ></span>
          </div>
        </div>
      </template>
    </div>

    <!-- 输入区 -->
    <div class="shrink-0 border-t border-[var(--aurora-border)] p-3">
      <textarea
        ref="inputEl"
        v-model="input"
        rows="2"
        class="w-full bg-[var(--aurora-field)] rounded-lg px-3 py-2 text-sm outline-none focus:bg-[var(--aurora-field)] resize-none placeholder:text-[var(--aurora-text-dim)] leading-relaxed"
        placeholder="输入消息…"
        @keydown="onKeydown"
      />
      <div class="flex items-center justify-between mt-1.5">
        <span class="text-[10px] text-[var(--aurora-text-dim)]">
          {{ streaming ? "回车停止 · 已接收内容保留" : "Enter 发送 · Shift+Enter 换行" }}
        </span>
        <button
          class="text-xs px-3 py-1 rounded transition-colors"
          :class="streaming ? 'bg-red-500/60 hover:bg-red-500/80' : 'bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)]'"
          @click="onClickSend"
        >
          {{ streaming ? "停止" : "发送" }}
        </button>
      </div>
    </div>
  </div>
</template>
