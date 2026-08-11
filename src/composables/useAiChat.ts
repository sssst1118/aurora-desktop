import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * 3.1 AI 对话状态机(设计文档 §1.4 + §2.4 动作条)。
 *
 * 会话无状态后端:本模块维护消息数组,send 时 invoke 全量传(截断最近 40 条);
 * 流式走单事件 `ai-event`(契约 §0.3):chunk 增量追加 / tool 记录工具动作 /
 * done 完整回复落地 / error 置错误条。
 *
 * 契约说明:ai_tools_enabled 开关由后端决定是否带 tools 参数(每请求读配置),
 * 前端只按配置/事件决定是否展示工具动作 chip,不注入任何 tools 数据。
 */
export type UiRole = "user" | "assistant" | "system" | "tool";

/** 前端消息(含 tool 动作占位行,仅用于展示,不发后端) */
export interface UiMessage {
  role: UiRole;
  content: string;
}

/** 发往后端的消息(与 Rust 侧 ChatMessage 对应,仅 user/assistant/system) */
export interface BackendMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

/** ai-event payload(事件契约见 docs/Phase3-设计.md §0.3,字段不允许单方面改) */
export interface AiEvent {
  kind: "chunk" | "tool" | "done" | "error";
  delta?: string;
  tool?: string;
  args?: string;
  full?: string;
  message?: string;
}

/** 发往后端消息数组上限(§1.4 会话管理;后端 truncate_messages 兜底) */
const MAX_MESSAGES = 40;

export function useAiChat() {
  const messages = ref<UiMessage[]>([]);
  const streaming = ref(false);
  const error = ref<string | null>(null);
  let unlisten: UnlistenFn | null = null;

  /** 过滤工具动作行 + 截断最近 40 条,转后端格式 */
  function toBackendMessages(): BackendMessage[] {
    const backend = messages.value
      .filter((m) => m.role !== "tool")
      .map((m) => ({ role: m.role, content: m.content } as BackendMessage));
    return backend.slice(-MAX_MESSAGES);
  }

  /** 最后一条助手消息(流式增量追加目标;工具动作行不打断) */
  function lastAssistant(): UiMessage | null {
    for (let i = messages.value.length - 1; i >= 0; i--) {
      const m = messages.value[i];
      if (m.role === "assistant") return m;
    }
    return null;
  }

  /** 移除空的助手占位消息(如纯工具轮次最终无文本) */
  function cleanupPlaceholder() {
    const last = messages.value[messages.value.length - 1];
    if (last && last.role === "assistant" && last.content === "") {
      messages.value.pop();
    }
  }

  function finish() {
    streaming.value = false;
    cleanupPlaceholder();
    unlisten?.();
    unlisten = null;
  }

  function handleEvent(ev: AiEvent) {
    switch (ev.kind) {
      case "chunk":
        if (ev.delta) {
          const m = lastAssistant();
          if (m) m.content += ev.delta;
        }
        break;
      case "tool":
        // 工具动作记录(§2.4):tool 名 + args 摘要内联展示
        messages.value.push({
          role: "tool",
          content: `🔧 正在${toolLabel(ev.tool ?? "")}${ev.args ? ` ${ev.args}` : ""}`,
        });
        break;
      case "done":
        if (ev.full !== undefined) {
          const m = lastAssistant();
          if (m) m.content = ev.full;
        }
        finish();
        break;
      case "error":
        error.value = ev.message ?? "对话出错,请重试";
        finish();
        break;
    }
  }

  /** 发送:push 用户消息 + 空助手占位 → 先订阅 ai-event 再 invoke 流式命令 */
  async function send(text: string) {
    const t = text.trim();
    if (!t || streaming.value) return;
    messages.value.push({ role: "user", content: t });
    messages.value.push({ role: "assistant", content: "" });
    error.value = null;
    streaming.value = true;

    // 先订阅后 invoke,避免漏掉后端首帧事件
    unlisten = await listen<AiEvent>("ai-event", (e) => {
      handleEvent(e.payload);
    });
    try {
      await invoke("ai_chat_stream", { messages: toBackendMessages() });
    } catch (e) {
      // 返回 Err 仅代表任务未能启动(参数非法/未配置密钥等)
      streaming.value = false;
      error.value = typeof e === "string" ? e : String(e);
      cleanupPlaceholder();
      unlisten?.();
      unlisten = null;
    }
  }

  /** 停止:取消订阅,已收文本保留(显示"已停止"由 UI 负责) */
  function stop() {
    if (!streaming.value) return;
    streaming.value = false;
    cleanupPlaceholder();
    unlisten?.();
    unlisten = null;
  }

  /** 清空对话(本地即可,后端无会话) */
  function clear() {
    stop();
    messages.value = [];
    error.value = null;
  }

  return { messages, streaming, error, send, stop, clear };
}

/** 工具名 → 动作条中文动词(§2.4 展示层自定义,不进事件契约) */
const TOOL_LABELS: Record<string, string> = {
  open_item: "打开",
  search_apps: "搜索应用",
  search_files: "搜索文件",
  set_wallpaper: "设置壁纸",
  get_system_status: "查询系统状态",
  get_clipboard_history: "查看剪贴板",
};

function toolLabel(tool: string): string {
  return TOOL_LABELS[tool] ?? tool;
}
