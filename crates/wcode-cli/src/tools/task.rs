//! The `task` tool: the root's plan (C2).
//!
//! **Root-only.** It is registered in `Orchestrator::tools`, and only the root
//! is given that tool set — a worker's tools have no `task`, so only the root
//! plans; a worker keeps its report-back. The module is registered in
//! `crate::tools` (tools/mod.rs) but kept out of `default_tools`.
//!
//! Ops: `create | assign | depends | complete | reject | list`. `assignee` is
//! resolved through the `Phonebook` first, then falls back to `agent:<name>` —
//! the same resolution the `message` tool uses (message.rs).

use serde::Deserialize;
use wcode_harness::protocol::SessionId;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::agents::{Phonebook, short_name};
use crate::tasks::{ReopenOutcome, Task as TaskRecord, TaskList};

/// Args for the `task` tool. Every field is optional at the type level so one
/// schema serves all ops; the op-specific requirement is enforced in `execute`.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct TaskArgs {
    /// The operation: `create | assign | depends | complete | reject | list`.
    op: String,
    /// The task id (`assign` / `depends` / `complete` / `reject`).
    #[serde(default)]
    id: Option<u32>,
    /// The task title (`create`).
    #[serde(default)]
    title: Option<String>,
    /// The worker to assign to (`assign`): a phonebook name, or `agent:<id>`.
    #[serde(default)]
    assignee: Option<String>,
    /// The ids this new task runs after (`create`). Absent = no deps.
    #[serde(default)]
    deps: Option<Vec<u32>>,
    /// The predecessor to depend on (`depends`): the new edge is `id → on`.
    #[serde(default)]
    on: Option<u32>,
    /// The rejection reason (`reject`), fed to the reworked node's next run.
    #[serde(default)]
    reason: Option<String>,
    /// The artifact to store (`complete`): the node's output, hydrating dependents.
    #[serde(default)]
    artifact: Option<String>,
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

/// Render a reopened-dep list as `#2, #3`; empty ⇒ `(none)`.
fn fmt_ids(reopened: &[(u32, ReopenOutcome)]) -> String {
    if reopened.is_empty() {
        return "(none)".to_string();
    }
    reopened
        .iter()
        .map(|(id, _)| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render a snapshot, one line per task: `#<id> [<state>] <title> (<owner>)`,
/// with `(unassigned)` when no owner is set; `(no tasks)` when empty. A node
/// with deps appends `← #a,#b` (its blocked-by set); one with rework appends
/// `×n` (its attempts). Both suffixes are omitted when empty/idle, so a plain
/// node renders exactly as before.
fn render(tasks: &[TaskRecord]) -> String {
    if tasks.is_empty() {
        return "(no tasks)".to_string();
    }
    tasks
        .iter()
        .map(|t| {
            let mut line = match &t.owner {
                Some(owner) => format!(
                    "#{} [{}] {} ({})",
                    t.id,
                    t.state.label(),
                    t.title,
                    short_name(owner)
                ),
                None => format!("#{} [{}] {} (unassigned)", t.id, t.state.label(), t.title),
            };
            if !t.deps.is_empty() {
                let ids = t
                    .deps
                    .iter()
                    .map(|d| format!("#{d}"))
                    .collect::<Vec<_>>()
                    .join(",");
                line.push_str(&format!(" ← {ids}"));
            }
            if t.attempts > 0 {
                line.push_str(&format!(" ×{}", t.attempts));
            }
            line
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
        "Plan and track tasks for the team. `op` is `create` (needs `title`; \
         optional `deps` = ids to run after), `assign` (needs `id` and \
         `assignee`), `depends` (needs `id` and `on`), `complete` (needs `id`; \
         optional `artifact`), `reject` (needs `id`; optional `reason`) for a \
         gate, or `list`. `assignee` is a worker name (or `agent:<id>`)."
    }

    async fn execute(&self, args: TaskArgs, _ctx: &ToolContext) -> ToolOutput {
        match args.op.as_str() {
            "create" => {
                let Some(title) = args.title.as_deref() else {
                    return err("create needs `title`");
                };
                match self.list.create(title, args.deps.clone().unwrap_or_default()) {
                    Ok(task) => ToolOutput {
                        output: format!("created #{} {}", task.id, task.title),
                        ..ToolOutput::default()
                    },
                    Err(e) => err(e), // e.g. "no task #7" for a bad dep
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
            "depends" => {
                let (Some(id), Some(on)) = (args.id, args.on) else {
                    return err("depends needs `id` and `on`");
                };
                match self.list.depends(id, on) {
                    Ok(()) => ToolOutput {
                        output: format!("#{id} now depends on #{on}"),
                        ..ToolOutput::default()
                    },
                    Err(e) => err(e), // unknown / self / duplicate / cycle
                }
            }
            "complete" => {
                let Some(id) = args.id else {
                    return err("complete needs `id`");
                };
                match self.list.complete(id, args.artifact.clone()) {
                    Ok(()) => ToolOutput {
                        output: format!("completed #{id}"),
                        ..ToolOutput::default()
                    },
                    Err(e) => err(e),
                }
            }
            "reject" => {
                let Some(id) = args.id else {
                    return err("reject needs `id`");
                };
                let reason = args.reason.as_deref().unwrap_or("");
                match self.list.reject(id, reason) {
                    Ok(reopened) => ToolOutput {
                        output: format!("rejected #{id}; reopened {}", fmt_ids(&reopened)),
                        ..ToolOutput::default()
                    },
                    Err(e) => err(e), // unknown id, or not a gate
                }
            }
            "list" => ToolOutput {
                output: render(&self.list.snapshot()),
                ..ToolOutput::default()
            },
            other => err(format!(
                "unknown op `{other}` (expected create | assign | depends | complete | reject | list)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wcode_harness::tool::erased;

    use crate::tasks::{RunSpec, TaskState};

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

    /// `create` records `deps`, `depends` adds an edge (and rejects a cycle),
    /// and `list` shows the `← #…` suffix; `reject` on a non-gate is an error.
    #[tokio::test]
    async fn dag_ops() {
        let (tool, list) = tool();
        // 1: a root work node.
        tool.execute(serde_json::json!({ "op": "create", "title": "work" }), ctx())
            .await;
        // 2: depends on 1.
        let out = tool
            .execute(
                serde_json::json!({ "op": "create", "title": "verify", "deps": [1] }),
                ctx(),
            )
            .await;
        assert!(!out.is_error, "{out:?}");
        // 3: no deps yet.
        tool.execute(serde_json::json!({ "op": "create", "title": "extra" }), ctx())
            .await;

        let out = tool.execute(serde_json::json!({ "op": "list" }), ctx()).await;
        assert!(
            out.output.contains("#2 [todo] verify (unassigned) ← #1"),
            "the dep suffix renders: {}",
            out.output
        );
        assert!(!out.output.contains('×'), "no attempts suffix at 0: {}", out.output);

        // `depends` adds a fresh edge 3 → 1.
        let out = tool
            .execute(serde_json::json!({ "op": "depends", "id": 3, "on": 1 }), ctx())
            .await;
        assert!(!out.is_error, "{out:?}");
        assert_eq!(list.snapshot()[2].deps, vec![1]);

        // A cycle (1 → 3 while 3 → 1) is rejected.
        let out = tool
            .execute(serde_json::json!({ "op": "depends", "id": 1, "on": 3 }), ctx())
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("cycle"), "{}", out.output);

        // `reject` on a non-gate errors.
        let out = tool
            .execute(serde_json::json!({ "op": "reject", "id": 1, "reason": "x" }), ctx())
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("not a gate"), "{}", out.output);
    }

    /// The tool's `reject` success arm (and `fmt_ids`'s non-empty join) plus
    /// `complete` carrying a real artifact — seams the unit tests left open.
    #[tokio::test]
    async fn reject_and_complete_carry_payloads() {
        let (tool, list) = tool();
        // work (1) and a gate (2) that depends on it.
        tool.execute(serde_json::json!({ "op": "create", "title": "work" }), ctx())
            .await;
        tool.execute(
            serde_json::json!({ "op": "create", "title": "verify", "deps": [1] }),
            ctx(),
        )
        .await;
        list.configure(2, RunSpec::Session { member: None }, true)
            .unwrap();
        list.complete(1, Some("A₁".into())).unwrap();

        // `reject` on the gate re-opens its dep; the output names it.
        let out = tool
            .execute(
                serde_json::json!({ "op": "reject", "id": 2, "reason": "redo" }),
                ctx(),
            )
            .await;
        assert!(!out.is_error, "{out:?}");
        assert!(
            out.output.contains("#1"),
            "names the reopened dep: {}",
            out.output
        );

        // `complete` stores the artifact on the node.
        let out = tool
            .execute(
                serde_json::json!({ "op": "complete", "id": 1, "artifact": "the report" }),
                ctx(),
            )
            .await;
        assert!(!out.is_error, "{out:?}");
        assert_eq!(list.snapshot()[0].artifact.as_deref(), Some("the report"));
    }

    /// `render` appends `← #deps` and `×attempts` only when set: a dep-less,
    /// 0-attempt node renders exactly as before.
    #[test]
    fn render_appends_suffixes_only_when_set() {
        let plain = TaskRecord {
            id: 1,
            title: "do it".into(),
            owner: None,
            state: TaskState::Todo,
            deps: vec![],
            attempts: 0,
            feedback: None,
            artifact: None,
            gate: false,
            run: RunSpec::Session { member: None },
        };
        assert_eq!(render(&[plain]), "#1 [todo] do it (unassigned)");

        let busy = TaskRecord {
            id: 3,
            title: "verify".into(),
            owner: Some(SessionId::agent("reviewer")),
            state: TaskState::Todo,
            deps: vec![2],
            attempts: 1,
            feedback: Some("x".into()),
            artifact: None,
            gate: true,
            run: RunSpec::Session {
                member: Some("reviewer".into()),
            },
        };
        assert_eq!(render(&[busy]), "#3 [todo] verify (reviewer) ← #2 ×1");
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
