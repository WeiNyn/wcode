use std::sync::Arc;

use crate::message::{AgentMessage, StopReason};
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

    /// Runs once when a `run` finishes, with the final context and why it
    /// stopped. The A2A auto-report rides this (a worker forwards its result to
    /// its orchestrator); a no-op by default.
    async fn after_run(&self, _ctx: &[AgentMessage], _stop: StopReason) {}
}

/// The workspace-mutating tools blocked in plan mode (D3). Read-only tools
/// (`read`/`grep`/`find`/`ast_search`/`session_search`) and the plan-recording
/// `todo` (and the team tools `task`/`peers`/`message`) are deliberately absent.
pub const MUTATING_TOOLS: &[&str] = &["edit", "edits", "write", "replace", "ast_edit"];

/// A cheap, cloneable on/off switch shared between the [`PlanModeHooks`] (read
/// from the run task) and the toggle that flips it (the actor, via
/// `Agent::set_plan_mode`). `Default` is off.
#[derive(Clone, Default)]
pub struct PlanModeHandle(Arc<std::sync::atomic::AtomicBool>);

impl PlanModeHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set(&self, on: bool) {
        self.0.store(on, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Plan-mode enforcement (D3/D4): while the mode is on, block workspace-mutating
/// tools and mutating `bash` commands. A no-op when the mode is off.
pub struct PlanModeHooks {
    plan: PlanModeHandle,
}

impl PlanModeHooks {
    pub fn new(plan: PlanModeHandle) -> Self {
        Self { plan }
    }
}

#[async_trait::async_trait]
impl Hooks for PlanModeHooks {
    /// In plan mode: block a workspace-mutating tool (by name), else gate `bash`
    /// by the read heuristic. A no-op when plan mode is off.
    async fn before_tool_call(&self, call: &ToolCall) -> Option<String> {
        if !self.plan.get() {
            return None;
        }
        if MUTATING_TOOLS.contains(&call.name.as_str()) {
            return Some(format!(
                "plan mode: `{}` is disabled — you are planning, not executing",
                call.name
            ));
        }
        if call.name == "bash" {
            return plan_bash_reason(call);
        }
        None
    }
}

/// Shell verbs whose presence as a segment's leading word blocks `bash` in plan
/// mode.
const MUTATING_VERBS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "mkdir", "touch", "chmod", "chown", "chgrp", "ln", "dd", "truncate",
    "tee", "patch",
];

/// `git` subcommands that mutate the index, history, or the remote.
const MUTATING_GIT: &[&str] = &[
    "add", "commit", "checkout", "reset", "clean", "apply", "merge", "rebase", "push", "stash",
    "restore", "rm", "mv",
];

/// D4 — the bash read-gate. Raw extraction mirrors `rtk.rs:112-118`
/// (`call.arguments.get("command").and_then(|c| c.as_str())`); a STRING SCAN for
/// obvious mutators — a guardrail, NOT a sandbox (it both over- and under-blocks;
/// see the FP/FN table in `docs/plan-mode.md` §2.4).
fn plan_bash_reason(call: &ToolCall) -> Option<String> {
    let cmd = call.arguments.get("command").and_then(|c| c.as_str())?;
    looks_mutating(cmd).then(|| {
        "plan mode: `bash` mutation refused — you are planning, not executing".to_string()
    })
}

/// Whether a shell command looks like it mutates the workspace.
fn looks_mutating(cmd: &str) -> bool {
    // Any output redirection (`>`/`>>`) mutates a file (or a device). `| tee`
    // is caught by `tee` in the segment scan below.
    if cmd.contains('>') {
        return true;
    }
    cmd.split([';', '&', '|', '\n']).any(segment_mutates)
}

/// Whether one shell segment (a command between separators) mutates.
fn segment_mutates(seg: &str) -> bool {
    let mut words = seg
        .split_whitespace()
        .map(|w| w.trim_matches(|c| c == '"' || c == '\''));
    let Some(lead) = words.next() else {
        return false;
    };
    let rest: Vec<&str> = words.collect();
    if MUTATING_VERBS.contains(&lead) {
        return true;
    }
    match lead {
        "sed" => rest.iter().any(|w| w.starts_with("-i")),
        "git" => matches!(rest.first().copied(), Some(g) if MUTATING_GIT.contains(&g)),
        _ => mutating_build_command(lead, &rest),
    }
}

/// Package-manager / build subcommands that mutate the source tree or lockfiles
/// (as opposed to read-only builds like `cargo build`/`make`/`tsc`).
fn mutating_build_command(lead: &str, rest: &[&str]) -> bool {
    match lead {
        "cargo" => matches!(
            rest.first().copied(),
            Some("fix" | "fmt" | "install" | "add" | "remove")
        ),
        "npm" | "pnpm" | "yarn" => matches!(
            rest.first().copied(),
            Some("install" | "ci" | "add" | "remove" | "update")
        ),
        "go" => {
            matches!(rest.first().copied(), Some("get"))
                || (matches!(rest.first().copied(), Some("mod"))
                    && matches!(rest.get(1).copied(), Some("tidy")))
        }
        "pip" => matches!(rest.first().copied(), Some("install")),
        _ => false,
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

    /// Runs every hook's `after_run` in order, once per run.
    pub async fn after_run(&self, ctx: &[AgentMessage], stop: StopReason) {
        for hook in &self.0 {
            hook.after_run(ctx, stop).await;
        }
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

    #[derive(Default)]
    struct RunLog {
        calls: std::sync::Mutex<Vec<usize>>,
    }

    #[async_trait::async_trait]
    impl Hooks for RunLog {
        async fn after_run(&self, ctx: &[AgentMessage], _stop: StopReason) {
            self.calls.lock().unwrap().push(ctx.len());
        }
    }

    #[tokio::test]
    async fn after_run_runs_every_hook() {
        let a = Arc::new(RunLog::default());
        let b = Arc::new(RunLog::default());
        let set = HooksSet::from_iter(vec![a.clone() as Arc<dyn Hooks>, b.clone()]);

        let ctx = vec![AgentMessage::user_text("hi")];
        set.after_run(&ctx, StopReason::Stop).await;

        assert_eq!(*a.calls.lock().unwrap(), vec![1]);
        assert_eq!(*b.calls.lock().unwrap(), vec![1]);
    }

    fn tool_call(name: &str) -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: serde_json::json!({}),
        }
    }

    fn bash_call(cmd: &str) -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": cmd }),
        }
    }

    #[tokio::test]
    async fn plan_mode_blocks_every_mutating_tool() {
        let plan = PlanModeHandle::new();
        plan.set(true);
        let hooks = PlanModeHooks::new(plan);
        for name in MUTATING_TOOLS {
            let reason = hooks.before_tool_call(&tool_call(name)).await;
            assert!(reason.is_some(), "{name} must be blocked in plan mode");
        }
    }

    #[tokio::test]
    async fn plan_mode_allows_read_tools_and_todo() {
        let plan = PlanModeHandle::new();
        plan.set(true);
        let hooks = PlanModeHooks::new(plan);
        for name in [
            "read",
            "grep",
            "find",
            "ast_search",
            "session_search",
            "todo",
            "task",
        ] {
            let reason = hooks.before_tool_call(&tool_call(name)).await;
            assert!(reason.is_none(), "{name} must be allowed in plan mode");
        }
    }

    #[tokio::test]
    async fn plan_mode_off_blocks_nothing() {
        let hooks = PlanModeHooks::new(PlanModeHandle::new());
        assert!(hooks.before_tool_call(&tool_call("edit")).await.is_none());
        assert!(hooks.before_tool_call(&bash_call("rm -rf x")).await.is_none());
    }

    #[tokio::test]
    async fn bash_gate_blocks_mutations_and_allows_reads() {
        let plan = PlanModeHandle::new();
        plan.set(true);
        let hooks = PlanModeHooks::new(plan);
        for cmd in [
            "rm -rf build",
            "mv a b",
            "cp a b",
            "mkdir d",
            "sed -i s/x/y/ f",
            "tee f",
            "echo hi > f",
            "echo hi >> f",
            "cat f | tee g",
            "dd if=/dev/zero of=f",
            "git add .",
            "git commit -m x",
            "git checkout -- .",
            "git push",
            "git stash",
            "cargo fmt",
            "cargo fix",
            "cargo add serde",
            "npm install",
            "pnpm ci",
            "yarn add x",
            "go mod tidy",
            "go get x",
            "pip install foo",
            "chmod +x f",
            "patch < p.diff",
        ] {
            assert!(
                hooks.before_tool_call(&bash_call(cmd)).await.is_some(),
                "`{cmd}` should be blocked"
            );
        }
        for cmd in [
            "ls -la",
            "cat x",
            "git status",
            "git log --oneline",
            "rg foo",
            "cargo build",
            "cargo test",
            "cargo check",
            "cargo run",
            "make",
            "tsc --noEmit",
            // A non-mutating sed (no `-i`) and a `>`-free command.
            "sed 's/x/y/' f",
        ] {
            assert!(
                hooks.before_tool_call(&bash_call(cmd)).await.is_none(),
                "`{cmd}` should be allowed"
            );
        }
    }
}
