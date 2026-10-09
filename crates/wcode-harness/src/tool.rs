use std::sync::Arc;

use crate::event::AgentEvent;

pub struct ToolContext {
    pub call_id: String,
    pub name: String,
    pub working_dir: std::path::PathBuf,
/// The run's cancel token. A tool that needs clean-up must watch this and
    /// finish BEFORE returning (e.g. `bash` kills its process group); the loop
    /// only backstops a tool that ignores it, racing the call and recording a
    /// synthetic error `ToolResult` ("aborted: cancelled before the tool
    /// returned") if the token fires first.
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

    /// Whether this tool may mutate the workspace. Default `false` — a mutator
    /// opts in, so plan mode's `MUTATING_TOOLS` denylist and this flag stay in
    /// sync (a test in `wcode-cli` asserts it).
    fn mutating(&self) -> bool {
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
    fn mutating(&self) -> bool;
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
            .map(|schema| slim_schema(&schema))
            .unwrap_or_else(|_| serde_json::json!({ "type": "object" }))
    }

    fn parallel_safe(&self) -> bool {
        TypedTool::parallel_safe(self)
    }

    fn mutating(&self) -> bool {
        TypedTool::mutating(self)
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

/// Strip the JSON-Schema scaffolding only a *validator* reads: the `$schema`
/// meta-schema URL, schemars' `title`, the `format` hints, and a `default` of
/// `null`. Tool definitions ride **every** request, so this boilerplate is paid
/// on each turn and says nothing an LLM caller acts on (the prose `description`s
/// carry the meaning).
///
/// The distinction that matters: `title`/`format` are *keywords* in a **schema**
/// position, but they are arbitrary **property names** inside a name map
/// (`properties`, `patternProperties`, `dependentSchemas`, `$defs`,
/// `definitions` — the set schemars itself treats as name maps). Only the former
/// is boilerplate — `webfetch` really does take a `format` argument and `task`
/// really does take a `title`, so filtering by key name alone would silently
/// delete them from the wire schema (a bug caught in review, W006). Name-map
/// keys are therefore never filtered; only their values are slimmed as schemas.
///
/// Purely mechanical — property names, `required`, `$ref`s, `enum`s and non-null
/// `default`s all survive, and a nullable type (`["T", "null"]`) is left intact
/// so nullability is still expressed.
///
/// Recursive, so nested `$defs` (e.g. `edits`'s `EditOp`, `todo`'s `TodoItem`)
/// are slimmed too.
fn slim_schema(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                if is_name_map(k) {
                    // The keys here are names the tool author chose, not schema
                    // keywords: never filter them. Their values are schemas.
                    let inner = match val {
                        serde_json::Value::Object(names) => serde_json::Value::Object(
                            names
                                .iter()
                                .map(|(name, schema)| (name.clone(), slim_schema(schema)))
                                .collect(),
                        ),
                        other => slim_schema(other),
                    };
                    out.insert(k.clone(), inner);
                    continue;
                }
                let boilerplate = k == "$schema"
                    || k == "title"
                    || k == "format"
                    || (k == "default" && val.is_null());
                if !boilerplate {
                    out.insert(k.clone(), slim_schema(val));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(slim_schema).collect())
        }
        other => other.clone(),
    }
}

/// Whether `key` introduces a map of user-chosen *names* → schemas, rather than
/// being a schema keyword itself. Its keys must be preserved verbatim.
fn is_name_map(key: &str) -> bool {
    matches!(
        key,
        "properties" | "patternProperties" | "dependentSchemas" | "$defs" | "definitions"
    )
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

    /// Whether this tool may mutate the workspace (see [`TypedTool::mutating`]).
    pub fn mutating(&self) -> bool {
        self.0.mutating()
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

    #[test]
    fn slim_schema_drops_boilerplate_but_keeps_semantics() {
        let raw = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "XArgs",
            "type": "object",
            "properties": {
                "n": { "type": ["integer", "null"], "format": "uint64", "minimum": 0, "default": null },
                "flag": { "type": "boolean", "default": false },
                "nested": { "$ref": "#/$defs/Inner" },
                // Properties whose NAMES collide with schema keywords. These are
                // real arguments (`webfetch.format`, `task.title`) and must
                // survive; the inner `format` keyword below must not.
                "format": { "type": "string", "format": "uint64" },
                "title": { "type": "string", "title": "Title" },
            },
            "$defs": { "Inner": { "title": "Inner", "type": "string", "description": "kept" } },
            // Another name map (schemars treats it as one): its keys are names too.
            "dependentSchemas": { "title": { "type": "object" } },
            "required": ["n"],
        });
        let slim = slim_schema(&raw);

        assert!(slim.get("$schema").is_none(), "$schema is validator-only");
        assert!(slim.get("title").is_none(), "title is noise");
        assert_eq!(
            slim["properties"]["n"]["type"],
            serde_json::json!(["integer", "null"]),
            "nullability must survive (Phase 1 does not collapse it)"
        );
        assert!(slim["properties"]["n"].get("format").is_none());
        assert!(
            slim["properties"]["n"].get("default").is_none(),
            "a null default is dropped"
        );
        assert_eq!(
            slim["properties"]["flag"]["default"],
            serde_json::json!(false),
            "a non-null default is meaningful and must survive"
        );
        assert_eq!(slim["properties"]["nested"]["$ref"], "#/$defs/Inner");
        // Regression (W006 review): a property *named* `format`/`title` is an
        // argument, not boilerplate — filtering by key name alone deleted them
        // from the wire schema.
        assert_eq!(
            slim["properties"]["format"]["type"], "string",
            "a property named `format` must survive"
        );
        assert!(
            slim["properties"]["format"].get("format").is_none(),
            "...while the `format` KEYWORD inside it is still dropped"
        );
        assert_eq!(
            slim["properties"]["title"]["type"], "string",
            "a property named `title` must survive"
        );
        assert!(
            slim["properties"]["title"].get("title").is_none(),
            "...while the `title` KEYWORD inside it is still dropped"
        );
        assert_eq!(
            slim["dependentSchemas"]["title"]["type"], "object",
            "every name map in `is_name_map` keeps its keys — including dependentSchemas"
        );
        assert_eq!(slim["required"], serde_json::json!(["n"]));
        assert_eq!(
            slim["$defs"]["Inner"]["description"], "kept",
            "recursion must preserve prose inside $defs"
        );
        assert!(
            slim["$defs"]["Inner"].get("title").is_none(),
            "recursion must reach $defs"
        );
    }

    #[test]
    fn real_tool_definitions_are_slim() {
        let parameters = erased(Echo).definition().parameters;
        let text = parameters.to_string();
        for gone in ["$schema", "\"title\"", "\"format\""] {
            assert!(!text.contains(gone), "`{gone}` leaked: {text}");
        }
        // ...and the schema still does its job.
        assert_eq!(parameters["type"], "object");
        assert!(parameters["properties"].get("text").is_some());
    }
}
