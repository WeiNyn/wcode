use crate::message::AgentMessage;
use crate::tool::ToolOutput;

/// Loop-local view of `ContentBlock::ToolCall`.
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[async_trait::async_trait]
pub trait Hooks: Send + Sync {
    /// Some(reason) = blocked: tool is not executed; the reason is fed back
    /// to the LLM as an error ToolResult (`blocked: {reason}`).
    async fn before_tool_call(&self, _call: &ToolCall) -> Option<String> {
        None
    }

    /// Runs after every tool execution; may mutate the output before it is
    /// recorded and sent to the LLM.
    async fn after_tool_call(&self, _call: &ToolCall, _out: &mut ToolOutput) {}

    /// Runs at every turn start, after steering is drained, before streaming.
    async fn transform_context(&self, _msgs: &mut Vec<AgentMessage>) {}

    /// Checked after tool execution completes; true ends the run with
    /// StopReason::Stop.
    async fn should_stop_after_turn(&self, _ctx: &[AgentMessage]) -> bool {
        false
    }
}

pub struct DefaultHooks;

impl Hooks for DefaultHooks {}
