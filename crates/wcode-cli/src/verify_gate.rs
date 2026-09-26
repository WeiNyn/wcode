//! C4 — **verify gate as a `Hooks` policy** (`docs/swarm-comparison-plan.md` §5,
//! candidate C4; decision #9 of `docs/task-dag-plan.md`).
//!
//! The rule is **R′**, ratified in review: refuse `task{op:"complete", id:X}`
//! when `X` is a work node that **no gate consumes**. It forces the verify step
//! to become *structure* — a gate node depends on the work node, so the work
//! node cannot close until a gate sits above it — instead of a reviewer the
//! root model may silently skip (jcode's "gates become structure").
//!
//! ## Why this lives in the CLI, not the harness
//!
//! [`VerifyGateHooks`] must read the root's [`TaskList`] to decide. `TaskList`
//! is a **CLI** type (`crates/wcode-cli/src/tasks.rs`, anchor `uDRme`); the
//! harness (`wcode-harness`) must not depend on it. So the policy is a CLI
//! `Hooks` impl, exactly like [`crate::agents::ReportBack`] and
//! [`crate::workspace::WorkspaceHooks`] — the same "policy in code, next to the
//! state it needs" seam the harness leaves open.
//!
//! ## The seam it rides
//!
//! `Hooks::before_tool_call` (`crates/wcode-harness/src/hooks.rs:y2lzh`):
//! `Some(reason)` **blocks** the tool — it is not executed, and the loop feeds
//! the reason back to the model as `blocked: {reason}`. `before_tool_call` sees
//! only `ToolCall { id, name, arguments }` (`hooks.rs:2MrGI`) — **no
//! `ToolContext`** — so the hook holds its own `TaskList` clone (cheap: `Arc`,
//! `tasks.rs:uDRme`).
//!
//! ## The rule — R′
//!
//! Refuse `task{ op:"complete", id:X }` when `X` is a **work node**
//! (`gate == false`, `run == Session{..}`, `tasks.rs:pTWTr`/`DXr7W`) that has
//! **no** node `gate == true` depending on it (`∃ t: t.gate && t.deps.contains(&X)`,
//! `deps` at `tasks.rs:kJT06`). This forces the verify step to become
//! **structure** — a gate node must consume the work node's artifact before the
//! work node can close.
//!
//! Candidates weighed:
//! - **R′ (adopted).** Structural; one predicate over the snapshot; rides
//!   the existing `task` tool and `TaskList`; no kernel/config/new-tool change.
//! - **a2 — require the gate node to be `Done` before its dep may
//!   complete.** **Dead.** A gate depends on the work node
//!   (`verify.deps == [implement]`), so the gate cannot be `Done` until its dep
//!   is `Done` — the condition never fires (a cycle in the acceptance order).
//! - **a3 — enforce only the physical `Script` subset.** Already shipped (P3:
//!   non-zero exit → auto-`reject`, run via `bash` under `BashRiskHooks`,
//!   `tasks.rs:ESHY4`; `docs/task-dag-plan.md` §5). Enforcing it again is a
//!   no-op → **C4's `Script` half is already enforced; R′ is the new half.**
//!
//! R′ is the hook's contribution; the *auto-run* of a ready gate is the
//! scheduler's existing dispatch (`scheduler.rs` frontier), not this hook —
//! `before_tool_call` returns only `Option<String>` (`hooks.rs:y2lzh`), it has
//! no channel to dispatch anything.
//!
//! ## Tension with decision #4 ("acceptance is the model's")
//!
//! R′ constrains the **shape** of the plan, not the verdict. A judgment gate is
//! still completed by the model (`complete` on the gate = approve; `reject` =
//! re-open deps, `tasks.rs:5X2Rj`), so the model keeps the accept/reject call.
//! What R′ takes away is the model's freedom to complete a work node *with no
//! verify consuming it* — which is exactly "the reviewer the model may skip".
//! Consistent with #9 ("gate *policy* stays in `Hooks`"); adjacent to #4.
//!
//! ## RESOLVED (review rulings, folded in)
//!
//! 1. **Every `Session` work node needs a gate.** `Task` (`tasks.rs:uNP2G`) has
//!    **no `kind` field** — `title` is free text and `run` is only
//!    `Session`/`Script` — so R′ cannot tell an explore node from an implement
//!    node. Ratified option (a): accept it. **UX cost (documented):** a trivial
//!    single-node plan must carry a self-assigned verify gate; there is no
//!    "this one is exempt" escape. Adding a `kind`/`terminal` field is a schema
//!    change out of C4's scope.
//! 2. **Gate-flag hole — TIGHTENED.** The exemption is a gate **with ≥1 dep**,
//!    NOT `node.gate == true`: a dep-less `gate:true` node is a free bypass (its
//!    own `reject` errors anyway, `tasks.rs:27A4K`), so it is **not** exempt.
//!    Even so, R′ is **shape, not enforcement** — it constrains the plan's
//!    graph, it does not prove the gate actually ran.
//! 3. **Tool-authored gates are always judgment.** The `task` tool's `create`
//!    hard-sets `run: Session{member:None}` (`tasks.rs:9lLd2`) and exposes **no**
//!    `run`/`script`/`member` arg (`crates/wcode-cli/src/tools/task.rs:w6rkk`),
//!    so a gate the model authors is always a **judgment** gate (dispatched
//!    back to the root). A physical `Script` gate is reachable only via
//!    `[workflow]` config or the `TaskList::configure` seam (`tasks.rs:23nfo`).
//!    Gating physically *from the model* is a separate `task`-tool extension.

use wcode_harness::hooks::{Hooks, ToolCall};

use crate::tasks::{RunSpec, Task, TaskList};

/// The verbatim block reason. The loop prefixes `blocked: ` (`hooks.rs:y2lzh`),
/// so the model sees `blocked: verify gate required: …`. It must NOT itself
/// start with `refused: `/`blocked: ` (mirrors `bash_risk_reason`,
/// `hooks.rs:J0bZ3`). `{id}` is the work node the model tried to complete.
pub const GATE_REASON: &str = "verify gate required: task #{id} is a work node \
with no gate depending on it — create a gate (`task create` with \
`deps: [{id}]` and `gate: true`, then `assign` it) before completing #{id}";

/// Enforces R′: a structural verify gate must consume a work node before the
/// root may `complete` it. A no-op for every non-`task` tool and every op but
/// `complete`.
///
/// Holds a [`TaskList`] clone (`Arc`-backed, `tasks.rs:uDRme`) because
/// `before_tool_call` receives no `ToolContext`. Construct it with the **root
/// orchestrator's** plan — `Orchestrator::tasks()` (`agents.rs:n9aLi`).
/// Building it for a worker or for a run with no orchestrator is meaningless
/// (a worker has no `task` tool), so the wiring guards on `orchestrator.is_some()`.
pub struct VerifyGateHooks {
    tasks: TaskList,
}

impl VerifyGateHooks {
    /// Build over the root's shared plan (a cheap clone).
    pub fn new(tasks: TaskList) -> Self {
        Self { tasks }
    }
}

#[async_trait::async_trait]
impl Hooks for VerifyGateHooks {
    /// `Some(reason)` iff — and only iff — this is a `task` **complete** whose
    /// target is an un-gated work node. The decision table (implemented in
    /// [`complete_target`] + [`ungated_work_node`]):
    ///
    /// ```text
    /// call.name != "task"                          -> None  (allow)
    /// arguments["op"] != "complete"                -> None  (allow: create/assign/
    ///                                                        depends/reject/reset/list)
    /// arguments["id"] absent / not a u32           -> None  (let the tool error)
    /// snapshot.find(id) == None                    -> None  (unknown id / empty plan)
    /// node.gate && !node.deps.is_empty()           -> None  (a REAL gate's complete = approve)
    /// node.run  != Session{..}                      -> None  (Script node: exit code is the verdict)
    /// ∃ t: t.gate && t.deps.contains(&id)          -> None  (a gate already depends on it)
    /// otherwise                                    -> Some(GATE_REASON with {id})
    /// ```
    ///
    /// Read the list via `self.tasks.snapshot()` (`tasks.rs:SCzgm`). Non-`task`
    /// calls are the hot path — return `None` first (mirrors
    /// `BashRiskHooks::before_tool_call`, `hooks.rs:i824s`).
    async fn before_tool_call(&self, call: &ToolCall) -> Option<String> {
        if call.name != "task" {
            return None; // every non-task tool is untouched (the hot path)
        }
        let id = complete_target(call)?;
        ungated_work_node(&self.tasks.snapshot(), id)
    }
}

/// The u32 `id` of a `task{op:"complete"}` call, or `None` when this is not that
/// call. Extraction mirrors the tool's own arg shape
/// (`crates/wcode-cli/src/tools/task.rs:w6rkk`): `op` is a `String`, `id` an
/// optional `u32`. A malformed call yields `None` (allow; the tool reports it).
fn complete_target(call: &ToolCall) -> Option<u32> {
    if call.arguments.get("op")?.as_str()? != "complete" {
        return None;
    }
    // Not `id`'s presence alone: a present-but-non-numeric `id` still yields
    // `None` here and is left to the tool to reject.
    call.arguments.get("id")?.as_u64().map(|n| n as u32)
}

/// The R′ predicate over a snapshot: `Some(reason)` iff `id` is a work node
/// (`gate == false`, `run == Session{..}`) with no gate depending on it;
/// `None` to allow (unknown id, a real gate, a `Script` node, or an
/// already-gated node). See [`VerifyGateHooks::before_tool_call`]'s table.
fn ungated_work_node(tasks: &[Task], id: u32) -> Option<String> {
    let node = tasks.iter().find(|t| t.id == id)?;
    // A REAL gate (gate + ≥1 dep) consumes work; its own `complete` IS the
    // approval. A dep-less `gate:true` node is a degenerate bypass (its `reject`
    // errors, `tasks.rs:27A4K`), so it is deliberately NOT exempt — Q2 tightened.
    if node.gate && !node.deps.is_empty() {
        return None;
    }
    // A physical (`Script`) node's verdict is its exit code (§5, P3), not a
    // model judgment — never gated here.
    if !matches!(node.run, RunSpec::Session { .. }) {
        return None;
    }
    // Already gated: some gate consumes this node (any such `t` has `id` among
    // its deps, so it is a real gate) → allow.
    if tasks.iter().any(|t| t.gate && t.deps.contains(&id)) {
        return None;
    }
    Some(GATE_REASON.replace("{id}", &id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw `task` call, shaped like `tools/task.rs` builds them.
    fn task_call(op: &str, id: Option<u32>) -> ToolCall {
        let mut args = serde_json::json!({ "op": op });
        if let Some(id) = id {
            args["id"] = serde_json::json!(id);
        }
        ToolCall {
            id: "c1".into(),
            name: "task".into(),
            arguments: args,
        }
    }

    /// A call to some other tool (empty args) — the hot path.
    fn named_call(name: &str) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: serde_json::json!({}),
        }
    }

    /// A plan with node #1 as a plain `Session` work node.
    fn work_only() -> TaskList {
        let list = TaskList::new();
        list.create("implement", vec![], false).unwrap();
        list
    }

    /// #1 work, #2 a judgment gate (`gate:true`) depending on #1.
    fn work_plus_gate() -> TaskList {
        let list = work_only();
        list.create("verify", vec![1], true).unwrap();
        list
    }

    #[tokio::test]
    async fn r_prime_blocks_completing_an_ungated_work_node() {
        let hooks = VerifyGateHooks::new(work_only());
        let reason = hooks
            .before_tool_call(&task_call("complete", Some(1)))
            .await
            .expect("an un-gated work node must be refused");
        assert!(
            reason.starts_with("verify gate required: "),
            "the reason must be the verbatim block text (no `blocked:`/`refused:` prefix): {reason}"
        );
        // The `{id}` substitution must actually fire — `starts_with` alone
        // does not pin it.
        assert!(reason.contains("#1"), "the reason must name the node: {reason}");
        assert!(
            !reason.contains("{id}"),
            "a `{{id}}` placeholder was left unsubstituted: {reason}"
        );
    }

    #[tokio::test]
    async fn allows_after_a_gate_depends_on_the_work_node() {
        let hooks = VerifyGateHooks::new(work_plus_gate());
        assert!(
            hooks
                .before_tool_call(&task_call("complete", Some(1)))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn blocks_a_work_node_whose_gate_depends_elsewhere() {
        // (i) A gate EXISTS but does not consume #1: the predicate is per-node,
        // not "some gate exists in the plan".
        let list = TaskList::new();
        list.create("a", vec![], false).unwrap(); // #1 — ungated
        list.create("b", vec![], false).unwrap(); // #2
        list.create("verify-b", vec![2], true).unwrap(); // #3 — gate on #2 only
        let hooks = VerifyGateHooks::new(list);
        assert!(
            hooks
                .before_tool_call(&task_call("complete", Some(1)))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn dep_less_gate_flag_is_not_exempt() {
        // (ii) Q2 TIGHTENED: `gate:true` with NO deps is a degenerate bypass,
        // not a real gate — so even its own `complete` is refused.
        let list = TaskList::new();
        list.create("degenerate", vec![], true).unwrap();
        let hooks = VerifyGateHooks::new(list);
        assert!(
            hooks
                .before_tool_call(&task_call("complete", Some(1)))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn ignores_non_task_tools() {
        let hooks = VerifyGateHooks::new(work_only());
        for name in ["bash", "edit", "read", "write", "grep", "find"] {
            assert!(
                hooks.before_tool_call(&named_call(name)).await.is_none(),
                "{name} must pass untouched"
            );
        }
    }

    #[tokio::test]
    async fn ignores_task_ops_other_than_complete() {
        let hooks = VerifyGateHooks::new(work_only());
        for op in [
            "create", "assign", "depends", "reject", "reset", "list", "configure",
        ] {
            assert!(
                hooks.before_tool_call(&task_call(op, Some(1))).await.is_none(),
                "{op} must not be gated"
            );
        }
    }

    #[tokio::test]
    async fn noop_on_unknown_id_and_empty_plan() {
        let empty = VerifyGateHooks::new(TaskList::new());
        assert!(
            empty
                .before_tool_call(&task_call("complete", Some(9)))
                .await
                .is_none()
        );
        let one = VerifyGateHooks::new(work_only());
        assert!(
            one.before_tool_call(&task_call("complete", Some(9)))
                .await
                .is_none(),
            "an unknown id is left for the tool to report"
        );
        // An absent / non-numeric `id` is malformed: allow (the tool errors).
        assert!(
            one.before_tool_call(&task_call("complete", None)).await.is_none(),
            "a missing id must not be gated"
        );
    }

    #[tokio::test]
    async fn exempts_script_work_nodes() {
        let list = TaskList::new();
        list.create("test", vec![], false).unwrap();
        list.configure(
            1,
            RunSpec::Script {
                command: "cargo test".into(),
            },
            false,
        )
        .unwrap();
        let hooks = VerifyGateHooks::new(list);
        assert!(
            hooks
                .before_tool_call(&task_call("complete", Some(1)))
                .await
                .is_none(),
            "a Script node's verdict is its exit code, not a gate"
        );
    }

    #[tokio::test]
    async fn exempts_the_gate_itself() {
        let hooks = VerifyGateHooks::new(work_plus_gate());
        // #2 is a real gate (`gate:true`, deps [1]) — its `complete` is the approval.
        assert!(
            hooks
                .before_tool_call(&task_call("complete", Some(2)))
                .await
                .is_none()
        );
    }
}
