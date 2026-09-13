use std::sync::Arc;

use crate::message::AgentMessage;
use crate::protocol::{Request, SessionId};
use crate::tool::ToolOutput;

/// Loop-local view of `ContentBlock::ToolCall`.
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[async_trait::async_trait]
pub trait Hooks: Send + Sync {
    /// May mutate the tool call's arguments before it is executed (e.g. to
    /// rewrite a bash command through `rtk`). Mutations are what actually
    /// executes, so `before_tool_call`/`after_tool_call` observe the rewrite.
    async fn transform_tool_input(&self, _call: &mut ToolCall) {}

    /// Some(reason) = blocked: tool is not executed; the reason is fed back
    /// to the LLM as an error ToolResult (`blocked: {reason}`).
    async fn before_tool_call(&self, _call: &ToolCall) -> Option<String> {
        None
    }

    /// Runs after every tool execution; may mutate the output before it is
    /// recorded and sent to the LLM.
    async fn after_tool_call(&self, _call: &ToolCall, _out: &mut ToolOutput) {}

    /// Policy for an inbound peer message (an A2A `Notify`/`Interrupt`/`Wake`):
    /// the mirror of [`Hooks::before_tool_call`]. May rewrite the request in
    /// place (the rewrite is what is delivered); `Some(reason)` drops it — the
    /// reason is returned to the sender when it is an `ask`. A message dropped
    /// here never reaches the context or starts a turn.
    async fn before_inbound(
        &self,
        _from: Option<&SessionId>,
        _request: &mut Request,
    ) -> Option<String> {
        None
    }

    /// Runs at every turn start, after steering is drained, before streaming.
    async fn transform_context(&self, _msgs: &mut Vec<AgentMessage>) {}

    /// Checked after tool execution completes; true ends the run with
    /// StopReason::Stop.
    async fn should_stop_after_turn(&self, _ctx: &[AgentMessage]) -> bool {
        false
    }
}

/// Ordered set of hook implementations run at every hook point.
///
/// Adding a hook is just pushing another [`Hooks`] impl: `transform_*` and
/// `after_tool_call` run for every hook in order, `before_tool_call` and
/// `should_stop_after_turn` short-circuit on the first hit. An empty set is
/// the default (every hook method is a no-op), so an agent without extra
/// hooks typically passes `HooksSet::default()`.
#[derive(Clone, Default)]
pub struct HooksSet(Vec<Arc<dyn Hooks>>);

impl HooksSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// A set with a single hook — the common case for one integration
    /// (e.g. `HooksSet::one(Arc::new(RtkHooks::new(rtk)))`).
    pub fn one(hook: Arc<dyn Hooks>) -> Self {
        Self(vec![hook])
    }

    pub fn push(&mut self, hook: Arc<dyn Hooks>) {
        self.0.push(hook);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub async fn transform_tool_input(&self, call: &mut ToolCall) {
        for hook in &self.0 {
            hook.transform_tool_input(call).await;
        }
    }

    /// First `Some(reason)` wins: a blocking hook prevents execution entirely
    /// (and therefore any later `after_tool_call`).
    pub async fn before_tool_call(&self, call: &ToolCall) -> Option<String> {
        for hook in &self.0 {
            if let Some(reason) = hook.before_tool_call(call).await {
                return Some(reason);
            }
        }
        None
    }

    /// First `Some(reason)` wins: a dropping hook prevents delivery (the request
    /// is not serviced). `request` may have been rewritten in place by earlier
    /// hooks — the rewrite is what would be delivered.
    pub async fn before_inbound(
        &self,
        from: Option<&SessionId>,
        request: &mut Request,
    ) -> Option<String> {
        for hook in &self.0 {
            if let Some(reason) = hook.before_inbound(from, request).await {
                return Some(reason);
            }
        }
        None
    }

    pub async fn after_tool_call(&self, call: &ToolCall, out: &mut ToolOutput) {
        for hook in &self.0 {
            hook.after_tool_call(call, out).await;
        }
    }

    pub async fn transform_context(&self, msgs: &mut Vec<AgentMessage>) {
        for hook in &self.0 {
            hook.transform_context(msgs).await;
        }
    }

    /// `true` ends the run; evaluated after the tool loop completes.
    pub async fn should_stop_after_turn(&self, ctx: &[AgentMessage]) -> bool {
        for hook in &self.0 {
            if hook.should_stop_after_turn(ctx).await {
                return true;
            }
        }
        false
    }
}

impl FromIterator<Arc<dyn Hooks>> for HooksSet {
    fn from_iter<T: IntoIterator<Item = Arc<dyn Hooks>>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingHooks {
        transforms: std::sync::atomic::AtomicUsize,
        outputs: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Hooks for RecordingHooks {
        async fn transform_tool_input(&self, call: &mut ToolCall) {
            self.transforms
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(obj) = call.arguments.as_object_mut() {
                obj.insert("n".into(), serde_json::json!(1));
            }
        }
        async fn after_tool_call(&self, _call: &ToolCall, out: &mut ToolOutput) {
            self.outputs
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            out.output.push('!');
        }
    }

    struct BlockingHooks;
    #[async_trait::async_trait]
    impl Hooks for BlockingHooks {
        async fn before_tool_call(&self, _call: &ToolCall) -> Option<String> {
            Some("blocked".into())
        }
    }

    #[tokio::test]
    async fn empty_set_is_noop() {
        let set = HooksSet::default();
        let mut call = ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": "ls" }),
        };
        set.transform_tool_input(&mut call).await;
        assert!(set.before_tool_call(&call).await.is_none());
        set.after_tool_call(&call, &mut ToolOutput::default()).await;
        assert!(!set.should_stop_after_turn(&[]).await);
    }

    #[tokio::test]
    async fn all_hooks_run_and_mutations_compose() {
        let a = Arc::new(RecordingHooks::default());
        let b = Arc::new(RecordingHooks::default());
        let set = HooksSet::from_iter(vec![a.clone() as Arc<dyn Hooks>, b.clone()]);

        let mut call = ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": "ls" }),
        };
        set.transform_tool_input(&mut call).await;
        assert_eq!(call.arguments["n"], 1, "composition kept the mutation");

        let mut out = ToolOutput {
            output: "x".into(),
            ..ToolOutput::default()
        };
        set.after_tool_call(&call, &mut out).await;
        assert_eq!(out.output, "x!!", "both hooks patched the output");
        assert_eq!(a.transforms.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(b.transforms.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn first_blocking_hook_wins() {
        let set = HooksSet::from_iter(vec![
            Arc::new(BlockingHooks) as Arc<dyn Hooks>,
            Arc::new(RecordingHooks::default()),
        ]);
        let call = ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": "ls" }),
        };
        assert_eq!(
            set.before_tool_call(&call).await.as_deref(),
            Some("blocked")
        );
    }

    struct GatingHooks;

    #[async_trait::async_trait]
    impl Hooks for GatingHooks {
        async fn before_inbound(
            &self,
            _from: Option<&SessionId>,
            request: &mut Request,
        ) -> Option<String> {
            let Request::Notify { content } = request else {
                return None;
            };
            if content == "drop me" {
                return Some("not allowed".into());
            }
            content.push_str(" (seen)");
            None
        }
    }

    #[tokio::test]
    async fn before_inbound_gates_and_rewrites() {
        let set = HooksSet::one(Arc::new(GatingHooks));

        let mut kept = Request::Notify {
            content: "hello".into(),
        };
        assert!(set.before_inbound(None, &mut kept).await.is_none());
        assert_eq!(
            kept,
            Request::Notify {
                content: "hello (seen)".into()
            }
        );

        let mut dropped = Request::Notify {
            content: "drop me".into(),
        };
        assert_eq!(
            set.before_inbound(None, &mut dropped).await.as_deref(),
            Some("not allowed")
        );

        // A local command is never gated by the inbound policy.
        let mut plain = Request::Cancel;
        assert!(set.before_inbound(None, &mut plain).await.is_none());
    }
}
