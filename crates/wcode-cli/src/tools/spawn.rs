//! The `spawn` tool: create a worker session and hand it a task (§10.1).

use std::sync::Arc;

use serde::Deserialize;
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::agents::{Phonebook, SessionFactory, WorkerSpec, short_name};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SpawnArgs {
    /// The task the worker should carry out.
    task: String,
    /// Optional address for the worker (default: `w1`, `w2`, …).
    #[serde(default)]
    name: Option<String>,
    /// Optional model id override for the worker (default: the orchestrator's).
    #[serde(default)]
    model: Option<String>,
    /// Optional role text appended to the worker's system prompt as a `# Role`.
    #[serde(default)]
    role: Option<String>,
    /// Optional tool allow-list for the worker (`message` is always kept; an
    /// unknown name is rejected). Omit for the full default set.
    #[serde(default)]
    tools: Option<Vec<String>>,
}

impl SpawnArgs {
    /// Map the tool args onto a worker spec: `role` → `system`; `name`, `model`,
    /// and `tools` pass through unchanged.
    fn to_worker_spec(&self) -> WorkerSpec {
        WorkerSpec {
            name: self.name.clone(),
            model: self.model.clone(),
            system: self.role.clone(),
            tools: self.tools.clone(),
        }
    }
}

/// Spawns a worker and starts it on a task.
///
/// Only the orchestrator holds one: a worker's tool set has no `spawn`, so only
/// the root spawns (bounded fan-out by construction, §10.1).
pub struct Spawn {
    factory: Arc<SessionFactory>,
    me: SessionId,
    phonebook: Phonebook,
}

impl Spawn {
    pub fn new(factory: Arc<SessionFactory>, me: SessionId, phonebook: Phonebook) -> Self {
        Self {
            factory,
            me,
            phonebook,
        }
    }
}

#[async_trait::async_trait]
impl TypedTool for Spawn {
    type Args = SpawnArgs;

    fn name(&self) -> &str {
        "spawn"
    }

    fn description(&self) -> &str {
        "Spawn a worker agent for a task. The worker runs in its own session, \
         works on the task, and reports its result back to you. Returns the \
         worker's address (use it with the `message` tool). Optional: `model` \
         overrides the worker's model, `role` appends a role section to its \
         system prompt, and `tools` restricts it to the named tools (`message` \
         is always kept; an unknown tool name is rejected)."
    }

    async fn execute(&self, args: SpawnArgs, _ctx: &ToolContext) -> ToolOutput {
        let spec = args.to_worker_spec();
        // Fail loudly on a bad allow-list entry rather than silently dropping it
        // (D14) — the model must learn the name was wrong.
        if let Err(e) = self.factory.validate_tools(&spec) {
            return ToolOutput {
                output: e,
                is_error: true,
                ..ToolOutput::default()
            };
        }
        let worker = match self.factory.spawn(&self.me, spec) {
            Ok(worker) => worker,
            Err(e) => {
                return ToolOutput {
                    output: e,
                    is_error: true,
                    ..ToolOutput::default()
                };
            }
        };
        // The worker joins the phonebook, so the model can address it by name.
        self.phonebook
            .insert(short_name(&worker.id), worker.id.clone());
        // Hand the task over at once — as a `Wake`, so the worker runs even if
        // idle.
        match self.factory.registry().deliver(
            &self.me,
            &worker.id,
            Request::Wake {
                content: args.task,
            },
        ) {
            Ok(()) => ToolOutput {
                output: format!("spawned {}", worker.id),
                ..ToolOutput::default()
            },
            Err(e) => ToolOutput {
                output: format!("spawned {} but could not deliver its task: {e}", worker.id),
                is_error: true,
                ..ToolOutput::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
    use wcode_harness::tool::erased;
    use wcode_protocol::Registry;

    use crate::agents::WorkerTemplate;
    use crate::config::ToolsConfig;

    fn factory() -> Arc<SessionFactory> {
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        SessionFactory::new(
            Registry::new(),
            WorkerTemplate {
                system: "sys".into(),
                llm: LlmOpts::default(),
                stream_fn,
                hooks: HooksSet::default(),
                tools: ToolsConfig::default(),
                compaction: CompactionPolicy::default(),
                working_dir: std::env::temp_dir(),
            },
        )
    }

    fn ctx() -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "s1".into(),
            name: "spawn".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_name_fails_loudly() {
        let tool = erased(Spawn::new(
            factory(),
            SessionId::agent("orch"),
            Phonebook::default(),
        ));
        let out = tool
            .execute(
                serde_json::json!({ "task": "hi", "tools": ["read", "bogus"] }),
                ctx(),
            )
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("bogus"), "names the bad entry: {}", out.output);
        assert!(out.output.contains("read"), "lists a valid tool: {}", out.output);
    }

    #[test]
    fn spawn_args_map_onto_a_worker_spec() {
        let args = SpawnArgs {
            task: "do it".into(),
            name: Some("w9".into()),
            model: Some("m".into()),
            role: Some("reviewer".into()),
            tools: Some(vec!["read".into()]),
        };
        let spec = args.to_worker_spec();
        assert_eq!(spec.name.as_deref(), Some("w9"));
        assert_eq!(spec.model.as_deref(), Some("m"));
        // `role` lands in `system`; the rest pass through.
        assert_eq!(spec.system.as_deref(), Some("reviewer"));
        assert_eq!(spec.tools, Some(vec!["read".to_string()]));
    }
}
