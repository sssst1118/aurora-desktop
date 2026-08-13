//! H3 安全加固:危险工具执行前的用户确认通道(3.1 命令层专用)。
//!
//! 流程:run_tool_loop 路由到危险工具 → 构造 DangerRequest → [`request_confirm`]
//!   注册 oneshot → emit `ai-tool-confirm` → 等待前端 `ai_confirm_tool` 回传(≤60s)
//!   → approve 执行;拒绝 / 超时 / 通道异常一律按拒绝跳过该工具。
//!
//! 存储:Tauri managed state([`ToolConfirmState`] = Mutex<HashMap<confirm_id, Sender<bool>>>)。
//!   lib.rs 的 builder 接线由主对话统一完成(见完成清单);本模块在命令层首次使用时
//!   经 `app.manage()` 运行时幂等注册兜底——builder 已注册时该调用自动退化为 no-op,
//!   两者共用同一实例,不冲突。

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// 确认等待上限:60s 无响应视为拒绝(前端窗口关闭/事件丢失时不会卡死对话)
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

/// `ai-tool-confirm` 事件 payload(前端按 id 用 Set 管理待确认项;字段契约勿单方面改)
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConfirmPayload {
    /// 确认 id(前端回传 ai_confirm_tool 的 confirm_id 参数)
    pub id: String,
    /// 模型侧 tool_call_id(仅信息展示)
    pub tool_call_id: String,
    /// 工具名(如 open_item)
    pub tool: String,
    /// 参数摘要(如目标路径)
    pub summary: String,
}

/// 确认决策结果(等待语义收敛为两种:批准 / 拒绝)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approved,
    Rejected,
}

/// 待确认注册表(tauri managed state;confirm_id → 决策回传通道)。
/// 单实例应用 + 单确认流,容量无压力;条目的 Sender 被 resolve 移除或超时清理。
#[derive(Default)]
pub struct ToolConfirmState {
    map: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

/// 确认 id 生成:进程内自增,唯一即可(并发只可能来自同一进程内的事件循环)
static CONFIRM_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn new_confirm_id() -> String {
    format!("tc-{}", CONFIRM_SEQ.fetch_add(1, Ordering::Relaxed))
}

impl ToolConfirmState {
    /// 注册待确认项,返回接收端(await 决策用)。
    /// id 重复(理论不发生)时旧条目被覆盖:旧发送端 drop → 旧等待方收 Err = 拒绝,不产生悬挂。
    pub fn register(&self, id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.map.lock().unwrap().insert(id.to_string(), tx);
        rx
    }

    /// 前端回传决策:查表发信。发信成功 → true;条目不存在(已超时清理/从未注册)→ false。
    pub fn resolve(&self, id: &str, approve: bool) -> bool {
        match self.map.lock().unwrap().remove(id) {
            Some(tx) => tx.send(approve).is_ok(),
            None => false,
        }
    }

    /// 兜底清理:决策后幂等移除(正常决策时已被 resolve 移除;超时分支清残留)
    pub fn remove(&self, id: &str) {
        self.map.lock().unwrap().remove(id);
    }
}

/// 等待前端决策(独立成函数便于短超时单测):
/// 超时 / 通道关闭 / 显式拒绝 一律 Rejected;只有明确收到 true 才算 Approved。
pub async fn wait_decision(rx: oneshot::Receiver<bool>, timeout: Duration) -> Decision {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(true)) => Decision::Approved,
        _ => Decision::Rejected,
    }
}

/// 取确认注册表(未注册时运行时幂等注册;builder 接线后自动复用已有实例)。
/// 不能碰 lib.rs 的过渡方案:builder `.manage()` 由主对话统一接线,此处兜底保证
/// 单测/接线前也能工作;运行时 manage 对已注册类型返回 false 且不改动原实例。
pub fn confirm_state(app: &tauri::AppHandle) -> tauri::State<'_, ToolConfirmState> {
    use tauri::Manager;
    let _ = app.manage(ToolConfirmState::default());
    app.state::<ToolConfirmState>()
}

/// 危险工具确认全流程:注册 → emit ai-tool-confirm → 等 ≤60s → 兜底清理。
/// 返回 true = 用户批准执行;false = 拒绝/超时/异常(一律不执行)。
/// app 按 owned 传入:命令层的确认器闭包须返回 'static future(同 ToolExecutor 约束)。
pub async fn request_confirm(app: tauri::AppHandle, req: DangerRequest) -> bool {
    use tauri::Emitter;
    let state = confirm_state(&app);
    let id = new_confirm_id();
    let rx = state.register(&id);
    let payload = ConfirmPayload {
        id: id.clone(),
        tool_call_id: req.tool_call_id,
        tool: req.tool,
        summary: req.summary,
    };
    let _ = app.emit("ai-tool-confirm", payload);
    let approved = wait_decision(rx, CONFIRM_TIMEOUT).await == Decision::Approved;
    state.remove(&id); // 已决策时条目已被 resolve 移除;这里兜底清超时/异常残留
    approved
}

use crate::ai::tools::DangerRequest;

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    // ---------- payload 序列化契约 ----------

    #[test]
    fn confirm_payload_serializes_contract_fields() {
        let p = ConfirmPayload {
            id: "tc-0".into(),
            tool_call_id: "call_9".into(),
            tool: "open_item".into(),
            summary: r"C:\Windows\System32\cmd.exe".into(),
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"id":"tc-0","tool_call_id":"call_9","tool":"open_item","summary":"C:\\Windows\\System32\\cmd.exe"}"#
        );
    }

    // ---------- 注册表注册/决策语义 ----------

    #[test]
    fn confirm_state_register_resolve_flow() {
        let st = ToolConfirmState::default();
        let id = new_confirm_id();
        let rx = st.register(&id);
        assert!(st.resolve(&id, true), "存在的条目发信成功");
        assert_eq!(block_on(rx), Ok(true));
        assert!(!st.resolve(&id, false), "已决策条目再 resolve → 不存在或已超时");
        assert!(!st.resolve("tc-nope", true), "未知 id → false");
    }

    #[test]
    fn confirm_state_resolve_after_receiver_dropped_is_false() {
        // 等待方超时放弃(receiver drop)后,迟到的前端回传应拿到"不存在或已超时"
        let st = ToolConfirmState::default();
        let id = new_confirm_id();
        let rx = st.register(&id);
        drop(rx);
        assert!(!st.resolve(&id, true), "接收端已消失 → 发信失败,语义为不存在或已超时");
        // remove 幂等:不 panic
        st.remove(&id);
        st.remove("tc-missing");
    }

    #[test]
    fn new_confirm_id_is_unique() {
        assert_ne!(new_confirm_id(), new_confirm_id());
    }

    // ---------- 决策等待语义(短超时直接测试) ----------

    #[test]
    fn wait_decision_approve_and_reject_paths() {
        // 通道里已是 true → Approved
        let (tx, rx) = oneshot::channel();
        tx.send(true).unwrap();
        assert_eq!(block_on(wait_decision(rx, CONFIRM_TIMEOUT)), Decision::Approved);
        // 通道里已是 false → Rejected
        let (tx, rx) = oneshot::channel();
        tx.send(false).unwrap();
        assert_eq!(block_on(wait_decision(rx, CONFIRM_TIMEOUT)), Decision::Rejected);
    }

    #[test]
    fn wait_decision_short_timeout_treated_as_reject() {
        // 发送端悬空不回复 → 短超时后必须按拒绝处理(等价生产 60s 超时语义)
        let (_tx, rx) = oneshot::channel();
        assert_eq!(
            block_on(wait_decision(rx, Duration::from_millis(20))),
            Decision::Rejected
        );
    }

    #[test]
    fn wait_decision_sender_dropped_is_reject() {
        // 发送端消失(通道关闭)→ 立即 Err → 拒绝,不挂起
        let (tx, rx) = oneshot::channel();
        drop(tx);
        assert_eq!(block_on(wait_decision(rx, CONFIRM_TIMEOUT)), Decision::Rejected);
    }
}
