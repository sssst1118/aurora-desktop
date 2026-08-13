<script setup lang="ts">
/**
 * 3.1 AI 对话面板(设计文档 §1.4 + §2.4 动作条)。
 * - 顶部栏:标题"AI 助手" + 设置按钮(打开 Settings,Phase2 内嵌在 search 窗口)+ 清空对话;
 * - 消息列表:用户右 / 助手左;助手消息纯文本 + `<pre>` 代码块(不引 markdown 库);
 *   流式期间光标 `▋`;工具动作 chip(如"🔧 正在打开 记事本");
 * - 输入框:Enter 发送 / Shift+Enter 换行;streaming 时回车改为停止;
 * - 错误条:error 事件红色提示,可关闭;
 * - H3 安全加固:危险工具(open_item)执行前,输入区上方弹确认条
 *   「AI 请求打开:<路径>」+ 执行/取消,点击 invoke ai_confirm_tool 回传后端;
 *   等待确认期间消息区显示轻提示"等待确认操作…";
 * - 样式:深色卡片风(bg-gray-950/95 + 边框),不透明窗口,与 Phase2 面板视觉一致。
 * 挂载:App.vue 的 ai_panel 分支(集成 agent 接线),本文件只写组件。
 */
import { ref, watch, nextTick, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { writeText } from "tauri-plugin-clipboard-api";
import { useAiChat, toolLabel } from "../../composables/useAiChat";

const { messages, streaming, error, send, stop, clear, confirmReqs, confirmTool } = useAiChat();

const input = ref("");
const inputEl = ref<HTMLTextAreaElement | null>(null);
const listEl = ref<HTMLDivElement | null>(null);
// 刚复制完的消息下标(按钮短暂显示 ✓)
const copiedIdx = ref<number | null>(null);
let copiedTimer: number | undefined;

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

/** 停止流式输出:先停再推一条 tool 角色"已停止"chip(复用 §2.4 工具动作展示) */
function stopWithFeedback() {
  if (!streaming.value) return;
  stop();
  messages.value.push({ role: "tool", content: "已停止" });
}

/** 复制消息文本到系统剪贴板(插件 writeText,权限已授权);按钮短暂显示 ✓ */
async function copyMessage(i: number, content: string) {
  try {
    await writeText(content);
    copiedIdx.value = i;
    if (copiedTimer) window.clearTimeout(copiedTimer);
    copiedTimer = window.setTimeout(() => {
      copiedIdx.value = null;
    }, 1200);
  } catch (e) {
    console.error("clipboard writeText failed", e);
  }
}

function onClickSend() {
  if (streaming.value) stopWithFeedback();
  else submit();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    if (streaming.value) stopWithFeedback();
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
  if (copiedTimer) window.clearTimeout(copiedTimer);
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

    <!-- 错误条(可关闭;语义色令牌 ai-error 见下方 style) -->
    <div
      v-if="error"
      class="mx-3 mt-2 px-3 py-2 rounded-lg ai-error text-xs flex items-center gap-2 shrink-0"
    >
      <span class="flex-1 break-all">{{ error }}</span>
      <button
        class="text-[var(--aurora-danger)] opacity-70 hover:opacity-100 shrink-0"
        title="关闭"
        @click="dismissError"
      >
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
        <!-- 工具动作 chip(§2.4;含"已停止"反馈,同样式) -->
        <div v-if="m.role === 'tool'" class="flex justify-center">
          <div
            class="max-w-[90%] px-2.5 py-1 rounded-full ai-tool-chip text-[11px] truncate"
            :title="m.content"
          >
            {{ m.content }}
          </div>
        </div>

        <!-- 用户消息(右,气泡 + 复制按钮) -->
        <div v-else-if="m.role === 'user'" class="flex justify-end items-start group gap-1.5">
          <button
            v-if="m.content"
            class="hidden group-hover:flex items-center justify-center w-6 h-6 shrink-0 rounded-md text-[11px] transition-colors"
            :class="
              copiedIdx === i
                ? 'text-[var(--aurora-success)]'
                : 'text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] hover:bg-[var(--aurora-field)]'
            "
            :title="copiedIdx === i ? '已复制' : '复制内容'"
            @click="copyMessage(i, m.content)"
          >
            {{ copiedIdx === i ? "✓" : "📋" }}
          </button>
          <div
            class="ai-user-bubble max-w-[85%] px-3 py-2 rounded-lg text-sm whitespace-pre-wrap break-words"
          >
            {{ m.content }}
          </div>
        </div>

        <!-- 助手消息(左,纯文本 + 代码块 + 复制按钮) -->
        <div v-else-if="m.role === 'assistant'" class="flex justify-start items-start group gap-1.5">
          <div
            class="max-w-[85%] px-3 py-2 rounded-lg bg-[var(--aurora-field)] border border-[var(--aurora-border)] text-sm leading-relaxed min-w-0"
          >
            <template v-if="m.content">
              <template v-for="(b, j) in splitBlocks(m.content)" :key="j">
                <pre
                  v-if="b.code"
                  class="mt-1 mb-1 p-2 rounded bg-black/50 border border-[var(--aurora-border)] text-xs overflow-x-auto font-mono text-[var(--aurora-success)]"
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
          <button
            v-if="m.content"
            class="hidden group-hover:flex items-center justify-center w-6 h-6 shrink-0 rounded-md text-[11px] transition-colors"
            :class="
              copiedIdx === i
                ? 'text-[var(--aurora-success)]'
                : 'text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] hover:bg-[var(--aurora-field)]'
            "
            :title="copiedIdx === i ? '已复制' : '复制内容'"
            @click="copyMessage(i, m.content)"
          >
            {{ copiedIdx === i ? "✓" : "📋" }}
          </button>
        </div>
      </template>

      <!-- H3 等待确认期间轻提示(确认条弹出后到用户决策前;列表尾部跟随滚动) -->
      <div v-if="confirmReqs.length > 0" class="flex justify-center pt-1">
        <div class="px-2.5 py-1 rounded-full ai-confirm-hint text-[11px]">
          等待确认操作…
        </div>
      </div>
    </div>

    <!-- H3 危险工具确认条:输入区上方;后端 await 最多 60s,期间不执行;
         执行/取消回传 ai_confirm_tool,确认条随即移除 -->
    <div
      v-for="req in confirmReqs"
      :key="req.id"
      class="mx-3 mb-2 px-3 py-2 rounded-lg ai-confirm flex items-center gap-2 shrink-0"
    >
      <span class="text-xs flex-1 truncate" :title="req.summary">
        AI 请求{{ toolLabel(req.tool) }}:<span class="font-medium">{{ req.summary }}</span>
      </span>
      <button
        class="ai-confirm-ok text-xs px-2.5 py-1 rounded-md shrink-0"
        title="批准执行该操作"
        @click="confirmTool(req.id, true)"
      >
        执行
      </button>
      <button
        class="ai-confirm-cancel text-xs px-2.5 py-1 rounded-md shrink-0"
        title="拒绝执行该操作"
        @click="confirmTool(req.id, false)"
      >
        取消
      </button>
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
          :class="streaming ? 'ai-btn-stop' : 'bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)]'"
          @click="onClickSend"
        >
          {{ streaming ? "停止" : "发送" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 语义色令牌的透明底/边框变体:令牌值由 global.css 提供(hex/rgba 均兼容)。
 * Tailwind 的 /opacity 修饰对 var() 任意值不生成样式(实测 bg-[var(--x)]/15 被丢弃),
 * 统一用 color-mix 合成透明色,不依赖令牌的书写格式。 */
.ai-error {
  background: color-mix(in srgb, var(--aurora-danger) 15%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-danger) 30%, transparent);
  color: var(--aurora-danger);
}
.ai-tool-chip {
  background: color-mix(in srgb, var(--aurora-warn) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-warn) 30%, transparent);
  color: var(--aurora-warn);
}
/* 用户气泡:跟随所选强调色,与发送按钮/选中态同源 */
.ai-user-bubble {
  background: color-mix(in srgb, var(--aurora-accent) 20%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-accent) 25%, transparent);
}
.ai-btn-stop {
  background: color-mix(in srgb, var(--aurora-danger) 60%, transparent);
}
.ai-btn-stop:hover {
  background: color-mix(in srgb, var(--aurora-danger) 80%, transparent);
}
/* H3 危险工具确认条:warn 色系(与工具 chip 同源,表"待决策");
 * 透明底/边框一律 color-mix(Tailwind 3.4 对 var() 任意值 + /opacity 不生成样式) */
.ai-confirm {
  background: color-mix(in srgb, var(--aurora-warn) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-warn) 35%, transparent);
}
.ai-confirm .font-medium {
  color: var(--aurora-warn);
}
/* 执行按钮:与发送按钮同强调色(文字色继承面板文本,与发送按钮一致);
 * 取消按钮:幽灵样式,悬停回默认文本色 */
.ai-confirm-ok {
  background: var(--aurora-accent);
}
.ai-confirm-ok:hover {
  filter: brightness(1.12);
}
.ai-confirm-cancel {
  background: color-mix(in srgb, var(--aurora-text-dim) 12%, transparent);
  color: var(--aurora-text-dim);
  border: 1px solid color-mix(in srgb, var(--aurora-text-dim) 25%, transparent);
}
.ai-confirm-cancel:hover {
  color: var(--aurora-text);
}
/* 等待确认轻提示:与 chip 同 warn 系 */
.ai-confirm-hint {
  background: color-mix(in srgb, var(--aurora-warn) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-warn) 30%, transparent);
  color: var(--aurora-warn);
  animation: aurora-pulse-soft 1.6s ease-in-out infinite;
}
@keyframes aurora-pulse-soft {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.55;
  }
}
</style>
