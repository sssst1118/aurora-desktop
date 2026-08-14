<script setup lang="ts">
/**
 * Phase6 AI 助手视图(AIPanel.vue 内容迁移,设计文档 §4.1 + 预览稿 renderAi)。
 * - 去掉了窗口级根/标题栏/拖拽把手(由 MainPanel 壳统一);原 ⚙ 设置按钮改为
 *   emit("open-settings") 由壳切到设置视图(设置就在 header 切换按钮旁,平级视图);
 *   清空对话按钮保留(工具行内)。
 * - 流式对话/工具动作 chip/危险工具确认条/错误条/代码块/复制按钮全保留;
 * - 输入聚焦时机从"窗口显示(tauri://show)"改为 KeepAlive 激活(onActivated);
 * - Enter 发送 / Shift+Enter 换行 / 流式期间回车停止(能力不变)。
 * 样式:消息气泡/工具 chip/确认条沿用原 AIPanel 语义色(color-mix 透明变体);
 * 列表结构参照预览稿 .ai-body/.msg-user/.msg-ai/.tool-chip 观感。
 */
import { nextTick, onActivated, onUnmounted, ref, watch } from "vue";
import { writeText } from "tauri-plugin-clipboard-api";
import { useAiChat, toolLabel } from "../../../composables/useAiChat";
import AuroraIcon from "../../icons/AuroraIcon.vue";

defineOptions({ name: "AIView" });

const emit = defineEmits<{ (e: "open-settings"): void }>();

const { messages, streaming, error, send, stop, clear, confirmReqs, confirmTool } = useAiChat();

const input = ref("");
const inputEl = ref<HTMLTextAreaElement | null>(null);
const listEl = ref<HTMLDivElement | null>(null);
// 刚复制完的消息下标(按钮短暂显示 ✓)
const copiedIdx = ref<number | null>(null);
let copiedTimer: number | undefined;

// 清空对话确认:两层点击(第一击进入确认态,3s 未确认自动复原;对照剪贴板清空模式)
const clearConfirming = ref(false);
let clearConfirmTimer: number | undefined;

function onClickClear() {
  if (!clearConfirming.value) {
    clearConfirming.value = true;
    if (clearConfirmTimer) window.clearTimeout(clearConfirmTimer);
    clearConfirmTimer = window.setTimeout(() => {
      clearConfirming.value = false;
    }, 3000);
    return;
  }
  if (clearConfirmTimer) window.clearTimeout(clearConfirmTimer);
  clearConfirmTimer = undefined;
  clearConfirming.value = false;
  clear();
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

/** 停止流式输出:先停再推一条 tool 角色"已停止"chip(复用工具动作展示) */
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

// 激活(窗口呼出且 AI 为当前视图 / 切回本视图)时聚焦输入框
onActivated(() => {
  void nextTick().then(() => inputEl.value?.focus());
});

onUnmounted(() => {
  if (copiedTimer) window.clearTimeout(copiedTimer);
  if (clearConfirmTimer) window.clearTimeout(clearConfirmTimer);
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
  <div class="h-full w-full flex flex-col min-h-0 select-none">
    <!-- 工具行:设置入口(原 AIPanel 顶栏 ⚙,改为切壳的设置视图)+ 清空对话 -->
    <div
      class="flex items-center justify-between px-4 py-1.5 border-b border-[var(--aurora-border)] shrink-0"
    >
      <span class="text-[10.5px] text-[var(--aurora-text-dim)]">
        {{ streaming ? "正在生成…" : "可调用工具:打开应用 / 搜文件 / 设壁纸 / 查系统状态" }}
      </span>
      <div class="flex items-center gap-1.5">
        <button
          class="flex items-center gap-1 text-xs text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] transition-colors"
          title="设置(AI 服务商/密钥/模型)"
          @click="emit('open-settings')"
        >
          <AuroraIcon name="settings" :size="11" />
          设置
        </button>
        <button
          class="flex items-center gap-1 text-xs transition-colors"
          :class="
            clearConfirming
              ? 'text-[var(--aurora-danger)]'
              : 'text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)]'
          "
          :title="clearConfirming ? '再次点击确认清空对话' : '清空对话'"
          @click="onClickClear"
        >
          <AuroraIcon name="trash" :size="11" />
          {{ clearConfirming ? "再点一次确认清空" : "清空对话" }}
        </button>
      </div>
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
        <AuroraIcon name="close" :size="11" />
      </button>
    </div>

    <!-- 消息列表 -->
    <div ref="listEl" class="ai-body flex-1 min-h-0">
      <div
        v-if="messages.length === 0"
        class="h-full flex items-center justify-center text-xs text-[var(--aurora-text-dim)] leading-relaxed text-center px-6"
      >
        输入消息开始与 AI 对话<br />可调用工具:打开应用 / 搜文件 / 设壁纸 / 查系统状态…
      </div>

      <template v-for="(m, i) in messages" :key="i">
        <!-- 工具动作 chip(含"已停止"反馈,同样式;emoji 已清剿,图标走 AuroraIcon) -->
        <div v-if="m.role === 'tool'" class="flex justify-center">
          <div
            class="tool-chip max-w-[90%] truncate"
            :title="m.content"
          >
            <AuroraIcon name="ai" :size="11" class="shrink-0" />
            <span class="truncate">{{ m.content }}</span>
          </div>
        </div>

        <!-- 用户消息(右,气泡 + 复制按钮) -->
        <div v-else-if="m.role === 'user'" class="flex justify-end items-start group gap-1.5">
          <button
            v-if="m.content"
            class="hidden group-hover:flex items-center justify-center w-6 h-6 shrink-0 rounded-md transition-colors"
            :class="
              copiedIdx === i
                ? 'text-[var(--aurora-success)]'
                : 'text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] hover:bg-[var(--aurora-field)]'
            "
            :title="copiedIdx === i ? '已复制' : '复制内容'"
            @click="copyMessage(i, m.content)"
          >
            <AuroraIcon :name="copiedIdx === i ? 'check' : 'copy'" :size="11" />
          </button>
          <div class="msg-user">
            {{ m.content }}
          </div>
        </div>

        <!-- 助手消息(左,纯文本 + 代码块 + 复制按钮) -->
        <div v-else-if="m.role === 'assistant'" class="flex justify-start items-start group gap-1.5">
          <div class="msg-ai">
            <template v-if="m.content">
              <template v-for="(b, j) in splitBlocks(m.content)" :key="j">
                <pre
                  v-if="b.code"
                  class="code-block mt-1 mb-1 p-2 rounded border border-[var(--aurora-border)] text-xs overflow-x-auto font-mono"
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
            class="hidden group-hover:flex items-center justify-center w-6 h-6 shrink-0 rounded-md transition-colors"
            :class="
              copiedIdx === i
                ? 'text-[var(--aurora-success)]'
                : 'text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] hover:bg-[var(--aurora-field)]'
            "
            :title="copiedIdx === i ? '已复制' : '复制内容'"
            @click="copyMessage(i, m.content)"
          >
            <AuroraIcon :name="copiedIdx === i ? 'check' : 'copy'" :size="11" />
          </button>
        </div>
      </template>

      <!-- 等待确认期间轻提示(确认条弹出后到用户决策前;列表尾部跟随滚动) -->
      <div v-if="confirmReqs.length > 0" class="flex justify-center pt-1">
        <div class="ai-confirm-hint">等待确认操作…</div>
      </div>
    </div>

    <!-- 危险工具确认条:输入区上方;后端 await 最多 60s,期间不执行;
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
        class="w-full bg-[var(--aurora-field)] rounded-lg px-3 py-2 text-sm outline-none resize-none placeholder:text-[var(--aurora-text-dim)] leading-relaxed"
        placeholder="输入消息…"
        @keydown="onKeydown"
      />
      <div class="flex items-center justify-between mt-1.5">
        <span class="text-[10px] text-[var(--aurora-text-dim)]">
          {{ streaming ? "回车停止 · 已接收内容保留" : "Enter 发送 · Shift+Enter 换行" }}
        </span>
        <button
          class="text-xs px-3 py-1 rounded transition-colors"
          :class="streaming ? 'ai-btn-stop' : 'bg-[var(--aurora-accent)] text-white'"
          @click="onClickSend"
        >
          {{ streaming ? "停止" : "发送" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 消息列表容器与气泡:参照预览稿 .ai-body/.msg-user/.msg-ai/.tool-chip;
 * 语义色透明变体沿用原 AIPanel 的 color-mix 方案(Tailwind 3.4 对 var()+opacity 不生效) */
.ai-body {
  padding: 14px 16px 10px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.msg-user,
.msg-ai {
  max-width: 86%;
  padding: 9px 13px;
  border-radius: 12px;
  font-size: 13px;
  line-height: 1.55;
}

.msg-user {
  align-self: flex-end;
  background: color-mix(in srgb, var(--aurora-accent) 20%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-accent) 25%, transparent);
  color: var(--aurora-text);
  white-space: pre-wrap;
  word-break: break-word;
}

.msg-ai {
  align-self: flex-start;
  background: var(--aurora-field);
  border: 1px solid var(--aurora-border);
  color: var(--aurora-text);
  min-width: 0;
}

/* 代码块(2026-08-14:原 bg-black/50 硬编码在浅色皮肤下成"浅面板嵌黑块",
   语义色绿字误用于代码正文 → 改半透明黑叠加皮肤自适应,文字用正文色) */
.code-block {
  background: color-mix(in srgb, #000 35%, transparent);
  color: var(--aurora-text);
}

.tool-chip {
  align-self: center;
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 11px;
  border-radius: 99px;
  font-size: 11px;
  color: var(--aurora-warn);
  border: 1px solid color-mix(in srgb, var(--aurora-warn) 30%, transparent);
  background: color-mix(in srgb, var(--aurora-warn) 10%, transparent);
}

.ai-error {
  background: color-mix(in srgb, var(--aurora-danger) 15%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-danger) 30%, transparent);
  color: var(--aurora-danger);
}

.ai-btn-stop {
  background: color-mix(in srgb, var(--aurora-danger) 60%, transparent);
  color: #fff;
}

.ai-btn-stop:hover {
  background: color-mix(in srgb, var(--aurora-danger) 80%, transparent);
}

/* 危险工具确认条:warn 色系(与工具 chip 同源,表"待决策") */
.ai-confirm {
  background: color-mix(in srgb, var(--aurora-warn) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--aurora-warn) 35%, transparent);
}

.ai-confirm .font-medium {
  color: var(--aurora-warn);
}

/* 执行按钮:与发送按钮同强调色;取消按钮:幽灵样式,悬停回默认文本色 */
.ai-confirm-ok {
  background: var(--aurora-accent);
  color: #fff;
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
  padding: 2.5px 11px;
  border-radius: 99px;
  font-size: 11px;
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

@media (prefers-reduced-motion: reduce) {
  /* 确认条脉冲:归零动画 */
  .ai-confirm-hint {
    animation: none;
  }
}
</style>
