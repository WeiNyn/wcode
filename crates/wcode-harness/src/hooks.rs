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
// ---- bash command-risk gate (Tier-2; docs/gap-analysis-jcode.md §3) ---------
//
// Doctrine fit: code, not config — ALWAYS ON, no knob, no `HooksConfig` field.
//
// WHY BESIDE `PlanModeHooks`: both are kernel `Hooks` policies that gate `bash`.
// Unlike plan mode this one carries NO handle and NO flag (`PlanModeHandle` has
// no analogue here) — it is unconditionally active, so a unit struct suffices.
//
// SEAM: `Hooks::before_tool_call` -> `Some(reason)` blocks. The loop converts it
// to `Planned::fail(id, name, format!("blocked: {reason}"))`, so the model
// receives `blocked: catastrophic shell command (…) …` as a tool error and can
// adapt. This is the exact plan-mode block path.
//
// ARG EXTRACTION mirrors `plan_bash_reason` (hooks.rs:cVnkO): skip unless
// `call.name == "bash"`; then `call.arguments.get("command").and_then(|c| c.as_str())`.
//
// NOT REUSED: `looks_mutating` / `segment_mutates`. They are PRIVATE and test
// *any* mutation (`rm -rf build`, `cargo fmt`, any `>` redirect) — the wrong
// predicate for a catastrophic-only guard. This gate is SELF-CONTAINED: it does
// NOT share a tokenizer with plan mode. Rationale: the two policies have OPPOSITE
// FP budgets (plan mode over-blocks by design; this one must under-block to stay
// a guardrail), so a shared scanner would have to satisfy both. Duplicating ~30
// lines of splitting is cheaper — and keeps the FP tables independent.
//
// SCOPE: catastrophic ONLY — irreversible system loss (jcode's crate exists
// because a user lost a home dir; issue #604). NOT a sandbox. Default = ALLOW
// (`None`); block only on a clear match. Over-blocking is the lesser evil here,
// but keep the subset tight.

/// Enforced read-only (D1): while installed, refuse the workspace-mutating tools
/// and mutating `bash`/`bg`. A guardrail, NOT a sandbox (the same FP/FN budget as
/// plan mode's bash gate — docs/plan-mode.md §2.4). Stateless: a unit struct.
pub struct ReadOnlyHooks;

impl ReadOnlyHooks {
    /// Kept for symmetry with `BashRiskHooks::new` (hooks.rs:zmKr5), so the CLI
    /// wiring reads `Arc::new(ReadOnlyHooks::new())` like every other built-in.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ReadOnlyHooks {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Hooks for ReadOnlyHooks {
    /// `Some(reason)` — the loop shows `blocked: {reason}`, so the reason starts
    /// `read-only worker: ` — for: any `MUTATING_TOOLS` name (hooks.rs:osi4C);
    /// `bg` (background `bash` can mutate); `bash` whose `command` satisfies the
    /// private `looks_mutating` (hooks.rs:SsKd8). Everything else passes.
    async fn before_tool_call(&self, call: &ToolCall) -> Option<String> {
        if MUTATING_TOOLS.contains(&call.name.as_str()) {
            return Some(format!("read-only worker: `{}` is disabled", call.name));
        }
        if call.name == "bg" {
            return Some("read-only worker: `bg` can run a mutating command".into());
        }
        if call.name == "bash" {
            // Extraction mirrors plan_bash_reason (hooks.rs:cVnkO).
            let cmd = call.arguments.get("command").and_then(|c| c.as_str())?;
            if looks_mutating(cmd) {
                return Some("read-only worker: `bash` mutation refused".into());
            }
        }
        None
    }
}
/// Always-on catastrophic-shell guard. Stateless — a unit struct is the entire
/// type.
pub struct BashRiskHooks;

impl BashRiskHooks {
    /// Kept for symmetry with `PlanModeHooks::new`, so the wiring in the CLI
    /// reads `Arc::new(BashRiskHooks::new())` like every other built-in hook.
    pub fn new() -> Self {
        Self
    }
}

impl Default for BashRiskHooks {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Hooks for BashRiskHooks {
    /// `Some(reason)` iff this is a `bash` call whose command is catastrophic;
    /// `None` (allow) otherwise — the hot path.
    async fn before_tool_call(&self, call: &ToolCall) -> Option<String> {
        if call.name != "bash" {
            return None; // every non-bash tool is untouched
        }
        // Mirrors plan_bash_reason's extraction (hooks.rs:cVnkO). A missing /
        // non-string `command` (malformed call, or args already rewritten by an
        // earlier hook) => allow.
        let cmd = call.arguments.get("command").and_then(|c| c.as_str())?;
        bash_risk_reason(cmd)
    }
}

/// `/dev/` device-name prefixes. A write *target* — `dd of=<dev>`, a `>`/`>>`
/// redirect into a device, or an `mkfs*`/`wipefs`/`shred` operand — whose
/// basename starts with one of these is catastrophic. Deliberately blunt:
/// `/dev/sdafoo` would also match. Acceptable: an FP merely yields a reason;
/// the guardrail never silently allows a real device.
const DEVICE_PREFIXES: &[&str] = &["sd", "hd", "nvme", "vd", "disk", "mmcblk"];

/// Targets that make a RECURSIVE delete/perm change catastrophic. Compared
/// quote-trimmed, with a trailing `/*` glob folding onto its root (`~/*` -> `~`,
/// `/*` -> `/`).
const ROOT_TARGETS: &[&str] = &["/", "~", "$HOME", "${HOME}"];

/// `rm` letters that mark a delete recursive (`-r`, `-rf`, `-fr`, `-R`).
const RM_RECURSIVE: &[char] = &['r', 'R'];

/// Canonical fork bomb, whitespace-free. [`forkbomb`] strips ALL ASCII
/// whitespace and tests `contains` — covers `:(){ :|:& };:`, `:(){:|:&};:`, and a
/// bomb hidden after a `;`/`&&` separator.
const FORK_BOMB: &str = ":(){:|:&};:";

/// Map a `bash` command to a refusal reason, or `None` to allow. Tries each
/// predicate in turn; the first hit supplies a verbatim `label`.
///
/// The reason must NOT start with `refused: ` — the loop already prefixes
/// `blocked: ` (loop_.rs:STqt9), and the `({label})` + "run it outside wcode"
/// shape mirrors plan mode's tone.
fn bash_risk_reason(cmd: &str) -> Option<String> {
    let label = device_write(cmd)
        .or_else(|| mkfs_or_wipe(cmd))
        .or_else(|| recursive_root_delete(cmd))
        .or_else(|| forkbomb(cmd))
        .or_else(|| recursive_perm_root(cmd))?;
    Some(format!(
        "catastrophic shell command (`{label}`) — this would destroy the system; \
         run it outside wcode if you really mean it"
    ))
}

/// (1) Device write: `dd … of=/dev/<dev>` or a `>`/`>>` redirect into
/// `/dev/<dev>`. Returns the offending segment as the label, or `None`.
///
/// Only `of=` counts for `dd` — `dd if=/dev/sda of=/tmp/img` (read a device into
/// a file) stays allowed.
fn device_write(cmd: &str) -> Option<String> {
    for seg in scan_segments(cmd) {
        let lead = seg.first().map(String::as_str).unwrap_or("");
        let dd_write = lead == "dd"
            && seg.iter().skip(1).any(|w| {
                w.strip_prefix("of=")
                    .is_some_and(|dev| is_device_path(dev, DEVICE_PREFIXES))
            });
        let redirect = seg.iter().enumerate().any(|(i, w)| {
            let target = if let Some(t) = redirect_target(w) {
                Some(t)
            } else if matches!(w.as_str(), ">" | ">>") {
                seg.get(i + 1).map(String::as_str)
            } else {
                None
            };
            target.is_some_and(|t| is_device_path(t, DEVICE_PREFIXES))
        });
        if dd_write || redirect {
            return Some(seg.join(" "));
        }
    }
    None
}

/// The redirect target baked into a single token, if any: `>/dev/sda`,
/// `>>/dev/sda`, `2>/dev/sda`, … A bare `>`/`>>` (target in the *next* token)
/// yields `None` — the caller handles that case.
fn redirect_target(w: &str) -> Option<&str> {
    let rest = w.trim_start_matches(|c: char| c.is_ascii_digit());
    let after = rest.strip_prefix(">>").or_else(|| rest.strip_prefix('>'))?;
    (!after.is_empty()).then_some(after)
}

/// (2) `mkfs` / `mkfs.<fs>` / `wipefs` / `shred` whose operand is a device path.
/// `shred -n1 /dev/sda` blocks; `shred build/x` is allowed.
fn mkfs_or_wipe(cmd: &str) -> Option<String> {
    for seg in scan_segments(cmd) {
        let Some(lead) = seg.first() else { continue };
        let formats = lead == "mkfs" || lead.starts_with("mkfs.");
        let wipes = lead == "wipefs" || lead == "shred";
        if (formats || wipes) && seg.iter().skip(1).any(|w| is_device_path(w, DEVICE_PREFIXES)) {
            return Some(seg.join(" "));
        }
    }
    None
}

/// (3) `rm` with a recursive flag AND a root/home target (`rm -rf /`, `rm -fr ~`,
/// `rm -R /*`, `rm -rf $HOME/*`).
fn recursive_root_delete(cmd: &str) -> Option<String> {
    for seg in scan_segments(cmd) {
        let Some(lead) = seg.first() else { continue };
        if lead == "rm"
            && seg.iter().skip(1).any(|w| is_recursive_flag(w))
            && seg.iter().skip(1).any(|w| is_root_target(w))
        {
            return Some(seg.join(" "));
        }
    }
    None
}

/// (4) Fork bomb (`:(){ :|:& };:` and spacing variants). See [`FORK_BOMB`].
fn forkbomb(cmd: &str) -> Option<String> {
    let squeezed: String = cmd.chars().filter(|c| !c.is_whitespace()).collect();
    if squeezed.contains(FORK_BOMB) {
        Some(FORK_BOMB.to_string())
    } else {
        None
    }
}

/// (5) `chmod`/`chown` with `-R`/`--recursive` on a root/home target
/// (`chmod -R 777 /`, `chown -R me ~`).
fn recursive_perm_root(cmd: &str) -> Option<String> {
    for seg in scan_segments(cmd) {
        let Some(lead) = seg.first() else { continue };
        if (lead == "chmod" || lead == "chown")
            && seg.iter().skip(1).any(|w| is_recursive_flag(w))
            && seg.iter().skip(1).any(|w| is_root_target(w))
        {
            return Some(seg.join(" "));
        }
    }
    None
}

/// True iff `s` (quote-trimmed) is `/dev/` immediately followed by one of
/// `prefixes`.
fn is_device_path(s: &str, prefixes: &[&str]) -> bool {
    let t = s.trim_matches(|c| c == '"' || c == '\'');
    t.strip_prefix("/dev/")
        .is_some_and(|rest| prefixes.iter().any(|p| rest.starts_with(p)))
}

/// Whether a token is a recursive flag: `--recursive`, or a short cluster
/// (`-r`, `-rf`, `-fr`, `-Rf`) carrying an `r`/`R`.
fn is_recursive_flag(w: &str) -> bool {
    if w == "--recursive" {
        return true;
    }
    match w.strip_prefix('-') {
        Some(rest) if !rest.is_empty() && !rest.starts_with('-') => {
            rest.chars().any(|c| RM_RECURSIVE.contains(&c))
        }
        _ => false,
    }
}

/// True iff `s` (quote-trimmed) names a root/home dir — `/`, `~`, `$HOME`,
/// `${HOME}` — with an optional trailing `/` or `/*` glob. Both fold onto the
/// root (see [`root_glob`]), so `rm -rf ~/`, `rm -rf ~/*`, and `rm -rf //` are
/// all as catastrophic as `rm -rf ~` / `/`.
fn is_root_target(s: &str) -> bool {
    let raw = s.trim_matches(|c| c == '"' || c == '\'');
    // A trailing `/` (or several) does not change the target — `~/` is `~`,
    // `//` is `/` — so collapse it before comparing; keep a bare `/` intact.
    let trimmed = raw.trim_end_matches('/');
    let t = if trimmed.is_empty() && raw.starts_with('/') {
        "/"
    } else {
        trimmed
    };
    ROOT_TARGETS
        .iter()
        .any(|root| t == *root || t == root_glob(root).as_str())
}

/// The `/*`-glob spelling of a root target: `~` -> `~/*`, but `/` -> `/*` (it
/// already ends in the separator).
fn root_glob(root: &str) -> String {
    if root.ends_with('/') {
        format!("{root}*")
    } else {
        format!("{root}/*")
    }
}

/// Split a command into segments on `;`/`&`/`|`/newline, then each segment into
/// whitespace tokens with surrounding quotes trimmed. Self-contained —
/// deliberately NOT `segment_mutates`.
fn scan_segments(cmd: &str) -> Vec<Vec<String>> {
    cmd.split([';', '&', '|', '\n'])
        .map(|seg| {
            seg.split_whitespace()
                .map(|w| w.trim_matches(|c| c == '"' || c == '\'').to_string())
                .collect::<Vec<String>>()
        })
        .filter(|seg| !seg.is_empty())
        .collect()
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
    async fn read_only_blocks_every_mutating_tool_and_bg() {
        let hooks = ReadOnlyHooks::new();
        for name in MUTATING_TOOLS.iter().copied().chain(std::iter::once("bg")) {
            let reason = hooks.before_tool_call(&tool_call(name)).await;
            assert!(
                reason.is_some_and(|r| r.starts_with("read-only worker: ")),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn read_only_blocks_mutating_bash() {
        let hooks = ReadOnlyHooks::new();
        for cmd in [
            "rm -rf build",
            "mv a b",
            "echo hi > f",
            "git commit -m x",
            "cargo fmt",
        ] {
            assert!(
                hooks.before_tool_call(&bash_call(cmd)).await.is_some(),
                "{cmd}"
            );
        }
    }

    #[tokio::test]
    async fn read_only_allows_read_tools_todo_message_and_benign_bash() {
        let hooks = ReadOnlyHooks::new();
        for name in [
            "read",
            "grep",
            "find",
            "ast_search",
            "session_search",
            "webfetch",
            "todo",
            "message",
            "peers",
            "task",
        ] {
            assert!(
                hooks.before_tool_call(&tool_call(name)).await.is_none(),
                "{name}"
            );
        }
        for cmd in ["ls -la", "git status", "cat x", "cargo build"] {
            assert!(
                hooks.before_tool_call(&bash_call(cmd)).await.is_none(),
                "{cmd}"
            );
        }
    }
    // The gate is plan-mode-INDEPENDENT: no handle/flag is set up; a bare
    // `BashRiskHooks::new()` drives it (`bash_call` (ucUhO) / `tool_call`
    // (JHwEE) are the helpers above).

    #[tokio::test]
    async fn bash_risk_blocks_catastrophic() {
        let hooks = BashRiskHooks::new();
        for cmd in [
            // (1) device writes — dd only for its `of=`, plus `>`/`>>` redirects
            "dd if=/dev/zero of=/dev/sda",
            "echo x > /dev/sda",
            "echo x >>/dev/nvme0n1",
            // (2) mkfs / wipefs / shred on a device
            "mkfs.ext4 /dev/sdb",
            "wipefs -a /dev/sdb",
            "shred -n1 /dev/sda",
            // (3) recursive root/home delete, incl. the `/*` fold
            "rm -rf /",
            "rm -fr ~",
            "rm -R /*",
            "rm -rf ~/*",
            "rm -rf $HOME/*",
            "rm -rf ${HOME}/*",
            // (4) fork bomb (spacing variants)
            ":(){ :|:& };:",
            ":(){:|:&};:",
            // (5) recursive perm/owner of root, incl. the fold
            "chmod -R 777 /",
            "chmod -R 777 /*",
            "chown --recursive me ~",
            // (3)/(5) a trailing slash folds onto its root too
            "rm -rf ~/",
            "rm -rf $HOME/",
            "rm -rf ${HOME}/",
            "rm -rf /*/",
            "chmod -R 777 //",
        ] {
            assert!(
                hooks.before_tool_call(&bash_call(cmd)).await.is_some(),
                "`{cmd}` should be blocked"
            );
        }
    }

    #[tokio::test]
    async fn bash_risk_allows_benign() {
        let hooks = BashRiskHooks::new();
        for cmd in [
            "rm -rf build",
            "rm -rf /tmp/x",
            "rm -r build",
            "rm -f /", // non-recursive delete of / is not this gate's target
            "dd if=/dev/zero of=/tmp/f bs=1M count=1",
            "dd if=/dev/sda of=/tmp/img", // read a device into a file: allowed
            "shred build/x",
            "chmod 777 /", // non-recursive perm change: allowed
            "ls -la",
            "cargo build",
        ] {
            assert!(
                hooks.before_tool_call(&bash_call(cmd)).await.is_none(),
                "`{cmd}` should be allowed"
            );
        }
    }

    #[tokio::test]
    async fn bash_risk_ignores_non_bash_tools() {
        let hooks = BashRiskHooks::new();
        assert!(hooks.before_tool_call(&tool_call("read")).await.is_none());
        assert!(hooks.before_tool_call(&tool_call("edit")).await.is_none());
    }

    #[tokio::test]
    async fn bash_risk_is_independent_of_plan_mode() {
        // The risk gate never consults the plan flag: it fires whether or not a
        // `PlanModeHooks` is present / on. Pins the ordering claim in the wiring.
        let hooks = BashRiskHooks::new();
        assert!(hooks.before_tool_call(&bash_call("rm -rf /")).await.is_some());

        // In a set beside an OFF plan hook, the risk gate still blocks.
        let off = HooksSet::from_iter([
            Arc::new(BashRiskHooks::new()) as Arc<dyn Hooks>,
            Arc::new(PlanModeHooks::new(PlanModeHandle::new())),
        ]);
        assert!(off.before_tool_call(&bash_call("rm -rf /")).await.is_some());

        // Beside an ON plan hook (which would ALSO block `rm -rf /`, with its own
        // "plan mode: …" reason), the risk gate is FIRST in the set, so ITS reason
        // wins — pins the ratified ordering.
        let plan = PlanModeHandle::new();
        plan.set(true);
        let on = HooksSet::from_iter([
            Arc::new(BashRiskHooks::new()) as Arc<dyn Hooks>,
            Arc::new(PlanModeHooks::new(plan)),
        ]);
        let reason = on.before_tool_call(&bash_call("rm -rf /")).await;
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.starts_with("catastrophic shell command")),
            "the risk gate's reason must win over plan mode's, got {reason:?}"
        );
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
