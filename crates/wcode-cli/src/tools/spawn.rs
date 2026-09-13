//! The `spawn` tool: create a worker session and hand it a task (§10.1).

use std::sync::Arc;

use serde::Deserialize;
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::agents::{SessionFactory, WorkerSpec};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SpawnArgs {
    /// The task the worker should carry out.
    task: String,
    /// Optional address for the worker (default: `w1`, `w2`, …).
    #[serde(default)]
    name: Option<String>,
}

/// Spawns a worker and starts it on a task.
///
/// Only the orchestrator holds one: a worker's tool set has no `spawn`, so only
/// the root spawns (bounded fan-out by construction, §10.1).
pub struct Spawn {
    factory: Arc<SessionFactory>,
    me: SessionId,
}

impl Spawn {
    pub fn new(factory: Arc<SessionFactory>, me: SessionId) -> Self {
        Self { factory, me }
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
         worker's address (use it with the `message` tool)."
    }

    async fn execute(&self, args: SpawnArgs, _ctx: &ToolContext) -> ToolOutput {
        let worker = self.factory.spawn(&self.me, WorkerSpec { name: args.name });
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
