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
 * H3 安全加固(2026-08-13):危险工具(open_item)执行前,后端 emit `ai-tool-confirm`
 * 并 await ≤60s;本模块订阅该事件、用 Set 按 id 管理待确认项(实际同一时刻最多
 * 一个),AIPanel 在输入区上方渲染确认条,点击后 invoke ai_confirm_tool 回传。
 * 确认状态为模块级单例:面板卸载重挂不丢待确认项,后端 60s 超时兜底。
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

// ==================== H3 危险工具确认(模块级单例) ====================

/** ai-tool-confirm payload(后端契约:confirm.rs ConfirmPayload;字段勿单方面改) */
export interface ToolConfirmEvent {
  /** 确认 id(回传 ai_confirm_tool 的 confirmId 参数) */
  id: string;
  /** 模型侧 tool_call_id(仅信息,当前未展示) */
  tool_call_id: string;
  /** 工具名(如 open_item) */
  tool: string;
  /** 参数摘要(如目标路径) */
  summary: string;
}

/** 确认条数据(由 ai-tool-confirm 事件产生;与 Set 同步增删) */
export interface ConfirmReq {
  id: string;
  tool: string;
  summary: string;
}

/** 待确认 id 集合(Set 管理,去重/幂等:事件重复到达只保留一条) */
const pendingConfirmIds = new Set<string>();
/** 确认条列表(响应式,驱动 AIPanel 确认条 UI;Set 为去重源,列表为渲染源) */
const confirmReqs = ref<ConfirmReq[]>([]);
let confirmSetup = false;

/** 订阅 ai-tool-confirm(幂等,模块级单例;面板重挂不丢待确认项,监听常驻不注销) */
function setupConfirmListener() {
  if (confirmSetup) return;
  confirmSetup = true;
  void listen<ToolConfirmEvent>("ai-tool-confirm", (e) => {
    const p = e.payload;
    if (pendingConfirmIds.has(p.id)) return; // 重复事件去重
    pendingConfirmIds.add(p.id);
    confirmReqs.value.push({ id: p.id, tool: p.tool, summary: p.summary });
  });
}

export function useAiChat() {
  setupConfirmListener();
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
        // 工具动作记录(§2.4):tool 名 + args 摘要内联展示;
        // 2026-08-14 emoji 清剿:内容纯文本,图标由 AIView 的 tool-chip 用 AuroraIcon 渲染
        messages.value.push({
          role: "tool",
          content: `正在${toolLabel(ev.tool ?? "")}${ev.args ? ` ${ev.args}` : ""}`,
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

  /** 回传危险工具确认决策(H3):先移确认条再 invoke;
   * 后端返回"不存在或已超时"(60s 已过)时静默,后端已按拒绝处理 */
  async function confirmTool(id: string, approve: boolean) {
    if (!pendingConfirmIds.has(id)) return;
    const i = confirmReqs.value.findIndex((r) => r.id === id);
    if (i >= 0) confirmReqs.value.splice(i, 1);
    pendingConfirmIds.delete(id);
    try {
      const res = await invoke<string>("ai_confirm_tool", { confirmId: id, approve });
      if (res !== "已确认" && res !== "已拒绝") {
        // "不存在或已超时":后端已按超时拒绝,确认条已移除,无需再提示
        console.warn(`[ai] 确认回传 ${id} 未生效: ${res}`);
      }
    } catch (e) {
      console.error("ai_confirm_tool failed", e);
    }
  }

  return { messages, streaming, error, send, stop, clear, confirmReqs, confirmTool };
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

/** 工具名 → 中文动词(确认条"AI 请求打开:xx"复用同一映射;导出供 AIPanel 使用) */
export function toolLabel(tool: string): string {
  return TOOL_LABELS[tool] ?? tool;
}
