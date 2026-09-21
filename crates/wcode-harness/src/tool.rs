use std::sync::Arc;

use crate::event::AgentEvent;

pub struct ToolContext {
    pub call_id: String,
    pub name: String,
    pub working_dir: std::path::PathBuf,
    pub cancel: tokio_util::sync::CancellationToken,
    pub events: tokio::sync::mpsc::UnboundedSender<AgentEvent>, // tool may send ToolExecutionUpdate
    /// The current run's session transcript path, when it has one — `None` for
    /// an in-memory session. `session_search` (`scope:"current"`) reads it to
    /// recover turns a compaction replaced with a summary.
    pub session_path: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct ToolOutput {
    /// What the model reads — this becomes the `ToolResult` content.
    pub output: String,
    pub is_error: bool,
    /// An optional unified diff, for *presentation only*: it rides the
    /// `ToolExecutionEnd` event to the UI but never enters the model's context.
    /// `None` for tools that do not touch files.
    pub diff: Option<String>,
    /// The file the tool changed, as the caller named it. Presentation only —
    /// it rides `ToolExecutionEnd` so the UI can index the run's changes and
    /// label each `⚙` line, but it never enters the model's context.
    /// `None` for tools that do not touch files (or an uncommitted dry run).
    pub path: Option<String>,
}

#[async_trait::async_trait]
pub trait TypedTool: Send + Sync + 'static {
    type Args: serde::de::DeserializeOwned + schemars::JsonSchema;
    fn name(&self) -> &str;
    fn description(&self) -> &str;

    /// Whether this tool may run concurrently with other calls in the same
    /// batch. Default `false` — a tool opts in, so anything that mutates state
    /// is a barrier unless it explicitly claims otherwise.
    fn parallel_safe(&self) -> bool {
        false
    }

    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput;
}

#[async_trait::async_trait]
trait ErasedToolCore: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    fn parallel_safe(&self) -> bool;
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> ToolOutput;
}

#[async_trait::async_trait]
impl<T: TypedTool> ErasedToolCore for T {
    fn name(&self) -> &str {
        TypedTool::name(self)
    }

    fn description(&self) -> &str {
        TypedTool::description(self)
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T::Args))
            .unwrap_or_else(|_| serde_json::json!({ "type": "object" }))
    }

    fn parallel_safe(&self) -> bool {
        TypedTool::parallel_safe(self)
    }

    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> ToolOutput {
        if ctx.cancel.is_cancelled() {
            return ToolOutput {
                output: "cancelled".to_string(),
                is_error: true,
                diff: None,
                path: None,
            };
        }
        let parsed: T::Args = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(e) => {
                return ToolOutput {
                    output: format!(
                        "invalid arguments for tool `{}`: {e}",
                        TypedTool::name(self)
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        };
        TypedTool::execute(self, parsed, &ctx).await
    }
}

#[derive(Clone)]
pub struct Tool(Arc<dyn ErasedToolCore>); // cloneable handle

pub fn erased<T: TypedTool>(t: T) -> Tool {
    Tool(Arc::new(t))
}

impl Tool {
    pub fn parallel_safe(&self) -> bool {
        self.0.parallel_safe()
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    pub fn definition(&self) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: self.name().to_string(),
            description: self.0.description().to_string(),
            parameters: self.0.parameters(),
        }
    }

    pub async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> ToolOutput {
        self.0.execute(args, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tokio::sync::mpsc;

    #[derive(Deserialize, schemars::JsonSchema)]
    struct EchoArgs {
        text: String,
    }

    #[derive(Clone)]
    struct Echo;

    #[async_trait::async_trait]
    impl TypedTool for Echo {
        type Args = EchoArgs;
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "repeats text back"
        }
        async fn execute(&self, args: Self::Args, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput {
                output: format!("echo:{}", args.text),
                is_error: false,
                diff: None,
                path: None,
            }
        }
    }

    fn test_ctx(
        cancel: tokio_util::sync::CancellationToken,
    ) -> (ToolContext, mpsc::UnboundedReceiver<AgentEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            ToolContext {
                call_id: "c1".to_string(),
                name: "echo".to_string(),
                working_dir: std::env::temp_dir(),
                cancel,
                events: tx,
                session_path: None,
            },
            rx,
        )
    }

    #[tokio::test]
    async fn executes_with_typed_args() {
        let tool = erased(Echo);
        let (ctx, _rx) = test_ctx(tokio_util::sync::CancellationToken::new());
        let out = tool
            .execute(serde_json::json!({ "text": "hello" }), ctx)
            .await;
        assert!(!out.is_error);
        assert_eq!(out.output, "echo:hello");
    }

    #[tokio::test]
    async fn garbage_args_yield_error_output_without_panic() {
        let tool = erased(Echo);
        let (ctx, _rx) = test_ctx(tokio_util::sync::CancellationToken::new());
        let out = tool.execute(serde_json::json!({ "wrong": 42 }), ctx).await;
        assert!(out.is_error);
        assert!(
            out.output.starts_with("invalid arguments for tool `echo`"),
            "unexpected output: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn cancelled_before_run_returns_cancelled_without_running() {
        let tool = erased(Echo);
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let (ctx, _rx) = test_ctx(cancel);
        // Note: even valid args must not reach the tool body.
        let out = tool
            .execute(serde_json::json!({ "text": "hello" }), ctx)
            .await;
        assert!(out.is_error);
        assert_eq!(out.output, "cancelled");
    }

    #[test]
    fn definition_parameters_schema_contains_property_names() {
        let tool = erased(Echo);
        let def = tool.definition();
        assert_eq!(def.name, "echo");
        assert_eq!(def.description, "repeats text back");
        assert_eq!(def.parameters["type"], "object");
        assert!(
            def.parameters["properties"].get("text").is_some(),
            "schema must expose `text` property, got: {}",
            def.parameters
        );
    }
}
