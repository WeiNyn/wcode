//! The `task` tool: the root's plan (C2).
//!
//! **Root-only.** It is registered in `Orchestrator::tools`, and only the root
//! is given that tool set — a worker's tools have no `task`, so only the root
//! plans; a worker keeps its report-back. The module is registered in
//! `crate::tools` (tools/mod.rs) but kept out of `default_tools`.
//!
//! Ops: `create | assign | complete | list`. `assignee` is resolved through the
//! `Phonebook` first, then falls back to `agent:<name>` — the same resolution the
//! `message` tool uses (message.rs `W0BMV`).

use serde::Deserialize;
use wcode_harness::protocol::SessionId;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::agents::{Phonebook, short_name};
use crate::tasks::{Task as TaskRecord, TaskList};

/// Args for the `task` tool. Every field is optional at the type level so one
/// schema serves all ops; the op-specific requirement is enforced in `execute`.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct TaskArgs {
    /// The operation: `create | assign | complete | list`.
    op: String,
    /// The task id (`assign` / `complete`).
    #[serde(default)]
    id: Option<u32>,
    /// The task title (`create`).
    #[serde(default)]
    title: Option<String>,
    /// The worker to assign to (`assign`): a phonebook name, or `agent:<id>`.
    #[serde(default)]
    assignee: Option<String>,
}

/// The root's task tool — the plan the orchestrator shares with its workers.
///
/// `parallel_safe` stays `false` (the default): it mutates shared state, so it is
/// a barrier and never runs concurrently with another call in a batch.
pub struct Task {
    list: TaskList,
    phonebook: Phonebook,
}

impl Task {
    pub fn new(list: TaskList, phonebook: Phonebook) -> Self {
        Self { list, phonebook }
    }

    /// Resolve an assignee name to an address: the phonebook first, then the
    /// same rule as `message::address` — a `:`-prefixed name is taken verbatim
    /// (`agent:w1`), otherwise a bare name becomes `agent:<name>`.
    fn resolve(&self, name: &str) -> SessionId {
        self.phonebook
            .get(name)
            .unwrap_or_else(|| crate::tools::message::address(name))
    }
}

/// An `is_error` output carrying `message` (the model learns what was wrong).
fn err(message: impl Into<String>) -> ToolOutput {
    ToolOutput {
        output: message.into(),
        is_error: true,
        ..ToolOutput::default()
    }
}

/// Render a snapshot, one line per task: `#<id> [<state>] <title> (<owner>)`,
/// with `(unassigned)` when no owner is set; `(no tasks)` when empty.
fn render(tasks: &[TaskRecord]) -> String {
    if tasks.is_empty() {
        return "(no tasks)".to_string();
    }
    tasks
        .iter()
        .map(|t| match &t.owner {
            Some(owner) => format!(
                "#{} [{}] {} ({})",
                t.id,
                t.state.label(),
                t.title,
                short_name(owner)
            ),
            None => format!("#{} [{}] {} (unassigned)", t.id, t.state.label(), t.title),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait::async_trait]
impl TypedTool for Task {
    type Args = TaskArgs;

    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Plan and track tasks for the team. `op` is `create` (needs `title`), \
         `assign` (needs `id` and `assignee`), `complete` (needs `id`), or \
         `list`. `assignee` is a worker name (or `agent:<id>`)."
    }

    async fn execute(&self, args: TaskArgs, _ctx: &ToolContext) -> ToolOutput {
        match args.op.as_str() {
            "create" => {
                let Some(title) = args.title.as_deref() else {
                    return err("create needs `title`");
                };
                let task = self.list.create(title);
                ToolOutput {
                    output: format!("created #{} {}", task.id, task.title),
                    ..ToolOutput::default()
                }
            }
            "assign" => {
                let (Some(id), Some(assignee)) = (args.id, args.assignee.as_deref()) else {
                    return err("assign needs `id` and `assignee`");
                };
                let owner = self.resolve(assignee);
                match self.list.assign(id, owner.clone()) {
                    Ok(()) => ToolOutput {
                        output: format!("assigned #{id} to {owner}"),
                        ..ToolOutput::default()
                    },
                    Err(e) => err(e),
                }
            }
            "complete" => {
                let Some(id) = args.id else {
                    return err("complete needs `id`");
                };
                match self.list.complete(id) {
                    Ok(()) => ToolOutput {
                        output: format!("completed #{id}"),
                        ..ToolOutput::default()
                    },
                    Err(e) => err(e),
                }
            }
            "list" => ToolOutput {
                output: render(&self.list.snapshot()),
                ..ToolOutput::default()
            },
            other => err(format!(
                "unknown op `{other}` (expected create | assign | complete | list)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wcode_harness::tool::erased;

    use crate::tasks::TaskState;

    /// A tool context, enough to call `execute` (mirrors `spawn.rs::ctx`).
    fn ctx() -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "t1".into(),
            name: "task".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
            session_path: None,
        }
    }

    /// A tool over a fresh list and an empty phonebook, plus the list to assert on.
    fn tool() -> (wcode_harness::tool::Tool, TaskList) {
        let list = TaskList::new();
        let tool = erased(Task::new(list.clone(), Phonebook::default()));
        (tool, list)
    }

    /// `create` then `list` shows the task.
    #[tokio::test]
    async fn create_then_list() {
        let (tool, list) = tool();
        let out = tool
            .execute(serde_json::json!({ "op": "create", "title": "do it" }), ctx())
            .await;
        assert!(!out.is_error, "{out:?}");
        assert!(out.output.contains("#1"), "{}", out.output);
        assert_eq!(list.snapshot().len(), 1);

        let out = tool.execute(serde_json::json!({ "op": "list" }), ctx()).await;
        assert!(!out.is_error, "{out:?}");
        assert!(
            out.output.contains("[todo] do it (unassigned)"),
            "{}",
            out.output
        );
    }

    /// `assign` records an owner (and `doing`), `complete` flips the state; an
    /// unknown id is an error, not a panic.
    #[tokio::test]
    async fn assign_and_complete() {
        let (tool, list) = tool();
        tool.execute(serde_json::json!({ "op": "create", "title": "t" }), ctx())
            .await;

        let out = tool
            .execute(
                serde_json::json!({ "op": "assign", "id": 1, "assignee": "w1" }),
                ctx(),
            )
            .await;
        assert!(!out.is_error, "{out:?}");

        let out = tool
            .execute(serde_json::json!({ "op": "complete", "id": 1 }), ctx())
            .await;
        assert!(!out.is_error, "{out:?}");

        let t = &list.snapshot()[0];
        assert_eq!(t.owner, Some(SessionId::agent("w1")));
        assert_eq!(t.state, TaskState::Done);

        // An unknown id is an error, not a panic.
        let out = tool
            .execute(
                serde_json::json!({ "op": "assign", "id": 9, "assignee": "w1" }),
                ctx(),
            )
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("9"), "{}", out.output);
    }

    /// A prefixed `assignee` is taken verbatim (mirrors `message::address`) —
    /// the bare-name path must not double-prefix `agent:w1`.
    #[tokio::test]
    async fn assign_accepts_a_prefixed_assignee() {
        let (tool, list) = tool();
        tool.execute(serde_json::json!({ "op": "create", "title": "t" }), ctx())
            .await;
        let out = tool
            .execute(
                serde_json::json!({ "op": "assign", "id": 1, "assignee": "agent:w1" }),
                ctx(),
            )
            .await;
        assert!(!out.is_error, "{out:?}");
        assert_eq!(list.snapshot()[0].owner, Some(SessionId::agent("w1")));
        assert!(
            out.output.contains("agent:w1") && !out.output.contains("agent:agent:w1"),
            "the echo shows the address verbatim: {}",
            out.output
        );
    }

    /// An unknown `op` (and a missing required field) is an `is_error`, not a panic.
    #[tokio::test]
    async fn unknown_op_is_an_error() {
        let (tool, _list) = tool();

        let out = tool
            .execute(serde_json::json!({ "op": "frobnicate" }), ctx())
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("frobnicate"), "{}", out.output);

        // A missing required field (`create` without `title`) is an error too.
        let out = tool.execute(serde_json::json!({ "op": "create" }), ctx()).await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("title"), "{}", out.output);
    }
}
