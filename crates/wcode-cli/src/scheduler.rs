//! The task-DAG scheduler (C3, P1) — the root's plan, executed.
//!
//! A root-side, long-lived `tokio` task: it subscribes to the shared
//! [`TaskList`]'s `watch` and, on every change, dispatches the **ready
//! frontier** — `Todo` nodes that have an owner and whose deps are all `Done`.
//! Dispatch is a hydrated [`Request::Wake`] delivered through the root's
//! [`Registry`]; on success the node flips to `Doing`.
//!
//! The model proposes, the scheduler disposes (§2): this module owns only the
//! *mechanical* facts — readiness and dispatch. It never judges; rework
//! (`reject`/`reopen` + cone invalidation) and the cap are P2. Because the
//! static graph is a DAG, [`TaskList::ready_ids`] terminates, so the loop is a
//! pure function of list state.
//!
//! No cancel token: the task lives as long as the process, matching the TUI's
//! detached task-feed spawn in `main.rs`. Its own `TaskList`/`Registry` clones
//! keep it alive; the loop ends only if the `watch` closes (it cannot while this
//! task holds a `TaskList` — see `run`), so in practice it ends at process exit.
//!
//! P1 is deliberately **silent**: it holds no UI channel, and an `eprintln!`
//! under the TUI's alternate screen would corrupt the display. A node that
//! cannot be delivered is left `Todo` (visibly un-started in `task list`); stall
//! notice + escalation is P2, through a proper seam.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wcode_harness::hooks::{HooksSet, ToolCall};
use wcode_harness::protocol::{Request, SessionId};
use wcode_protocol::Registry;

use crate::tasks::{Dep, RunSpec, Task, TaskList, TaskState};
use crate::tools::bash::{PREVIEW_TAIL_CHARS, run_command, tail_chars};

/// The ONE rule every bound follows (W028, amended): **no limit unless set.**
///
/// * `None` (the key is absent) or `Some(0)` → `None` = NO timer;
/// * `Some(n)` → `n` seconds.
///
/// There is deliberately no site default: an unset node/edge bound is
/// UNBOUNDED (D033's amendment — a built-in 300 s / 30 s default silently capped
/// every node of a template that never asked for one). The bound that matters is
/// the WHOLE-RUN cap, set at launch (`--timeout`, else `[workflow]
/// timeout_secs`), which this same function maps in `dispatch`.
///
/// The `None` return means UNBOUNDED, which [`run_command`] accepts directly. It
/// must never be spelled `Some(Duration::ZERO)`: that fires on the first poll and
/// would kill the command instantly instead of never bounding it.
pub(crate) fn workflow_bound_opt(secs: Option<u64>) -> Option<Duration> {
    secs.filter(|&s| s > 0).map(Duration::from_secs)
}

/// A node's deadline, if it is a `Session` node: its
/// `RunSpec::Session.timeout_secs` through [`workflow_bound_opt`] — so an unset
/// (or `0`) bound is NO deadline.
///
/// `None` for a `Script` node — deliberately structural (W028 A6) — so the
/// timer arm can never double-book a node whose bound already lives in its
/// `run_command` timeout.
fn session_deadline(tasks: &[Task], id: u32) -> Option<Duration> {
    let task = tasks.iter().find(|t| t.id == id)?;
    let RunSpec::Session { timeout_secs, .. } = &task.run else {
        return None;
    };
    workflow_bound_opt(*timeout_secs)
}

/// When node `id`'s deadline FIRES, dispatching at `at`, or `None` when it has no
/// bound — or when `at + bound` cannot be represented.
///
/// The `checked_add` is not paranoia (W028 N2): an absurd `timeout_secs` such as
/// `u64::MAX` overflows `Instant`, and an `at + d` panic would silently kill the
/// scheduler task. An unrepresentable deadline is treated exactly like "no
/// deadline".
fn node_deadline(
    tasks: &[Task],
    id: u32,
    at: tokio::time::Instant,
) -> Option<tokio::time::Instant> {
    session_deadline(tasks, id).and_then(|d| at.checked_add(d))
}

/// Await the earliest deadline, or forever when there is none, and hand back the
/// node whose deadline fired.
///
/// `pending()` for `None` — NOT a `FAR` sentinel `Instant` (W028 C1): there is no
/// safe "infinitely far" `Instant + Duration` to compute, and `sleep_until` on
/// one would need overflow handling for no gain.
async fn sleep_until_opt(when: Option<(u32, tokio::time::Instant)>) -> Option<u32> {
    match when {
        Some((id, at)) => {
            tokio::time::sleep_until(at).await;
            Some(id)
        }
        None => std::future::pending().await,
    }
}

/// The root's DAG executor: watches the plan and wakes the ready frontier.
///
/// Cheap to build — [`TaskList`] and [`Registry`] are `Arc`-backed handles — so
/// the composition root spawns it with three clones of what the
/// `Orchestrator` already holds (see `Orchestrator::spawn_scheduler`).
pub struct Scheduler {
    /// The address book the root dispatches through. Its ownership edges
    /// (`set_owner`, set at worker spawn) define the permitted set, so a node's
    /// `owner` must be one of the root's registered workers.
    registry: Registry,
    /// The root's own address — the `from` of every `Wake`, so the worker sees
    /// the task as a message from its orchestrator (and its own `ReportBack`
    /// edge back to the root holds, §4).
    root: SessionId,
    /// The shared plan. `subscribe()` seeds the current list and updates on
    /// every mutation (§4 step 1 — the `watch` already exists).
    tasks: TaskList,
    /// The scheduler's own address — the `from` of an escalation `Wake`. The
    /// root has no owner, so `deliver(root, root, …)` would be `NotPermitted`
    /// ([`Registry::permitted`] wants an ownership edge); the orchestrator gives
    /// this address an owner edge to the root (`Orchestrator::spawn_scheduler`),
    /// so `self_addr → root` is permitted.
    self_addr: SessionId,
    /// The directory a `Script` node runs in — the root's cwd, so a physical
    /// gate sees the same tree the workers do.
    working_dir: PathBuf,
    /// The root's hooks — `BashRiskHooks` (and any other policy) applies to a
    /// `Script` command exactly as it does to the `bash` tool.
    hooks: HooksSet,
    /// Cancels an in-flight `Script` run. A fresh, never-cancelled token is fine
    /// for P3 (the script timeout bounds a runaway).
    cancel: CancellationToken,
    /// The edge judge (W025 §4.3, B2j). `dispatch` spawns `judge.decide(…)` for a
    /// `judge` edge; the verdict arrives on `judge_rx`, so ALL `TaskList` mutation
    /// stays on the loop thread (the design's invariant).
    judge: Arc<dyn crate::judge::Judge>,
    /// Verdict channel sender — cloned into each spawned `decide` task. Payload
    /// `(node id, verdict)`; `None` = unparseable.
    judge_tx: tokio::sync::mpsc::UnboundedSender<(u32, crate::judge::Verdict)>,
    /// The receiver half, built in `new` and TAKEN once by `run` (A6).
    /// `Mutex<Option<…>>`, not a bare field: `run(self)` owns `self`, so a bare
    /// receiver would be unreachable there while `new` must still create the pair
    /// for `judge_tx`. Sound because a `Scheduler` runs exactly once.
    judge_rx: std::sync::Mutex<
        Option<tokio::sync::mpsc::UnboundedReceiver<(u32, crate::judge::Verdict)>>,
    >,
    /// Nodes with an in-flight `judge.decide` — EXCLUDED from `frontier` so a
    /// pending (async) verdict is not re-dispatched.
    judging: std::collections::HashSet<u32>,
    /// Nodes blocked-and-escalated ONCE (a refused `script`, or a judge `None`
    /// verdict). A `HashMap<id, reset_epoch>` (A14/R1), NOT a `HashSet` — it
    /// records the `Task.reset_epoch` seen at mark time, so a `TaskList::reset`
    /// (which bumps that epoch) is observable. EXCLUDED from `frontier` until then.
    unjudgeable: std::collections::HashMap<u32, u32>,
    /// When a `Session` node was dispatched (`tasks.start` succeeded), for
    /// `run`'s timer arm. IN-MEMORY ONLY, deliberately (W028 A6): `reconcile`
    /// flips `Doing → Todo` on load, so a crash RESTARTS the deadline — correct,
    /// because a crash is not the node's fault. A journaled deadline would
    /// survive the crash and time out a node that never ran.
    ///
    /// Pruned against the snapshot every `run` iteration: a complete / reject /
    /// reset / reopen leaves a stale entry that would fire a bogus deadline.
    started: std::collections::HashMap<u32, tokio::time::Instant>,
}


impl Scheduler {
    /// Build a scheduler over the root's shared plan and registry.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Registry,
        root: SessionId,
        tasks: TaskList,
        self_addr: SessionId,
        working_dir: PathBuf,
        hooks: HooksSet,
        cancel: CancellationToken,
        judge: Arc<dyn crate::judge::Judge>,
    ) -> Self {
        let (judge_tx, judge_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            registry,
            root,
            tasks,
            self_addr,
            working_dir,
            hooks,
            cancel,
            judge,
            judge_tx,
            judge_rx: std::sync::Mutex::new(Some(judge_rx)),
            judging: std::collections::HashSet::new(),
            unjudgeable: std::collections::HashMap::new(),
            started: std::collections::HashMap::new(),
        }
    }

    /// Run until the process exits (or, defensively, the `TaskList` watch closes).
    ///
    /// ```text
    /// let mut updates = tasks.subscribe();
    /// loop {
    ///     let _ = updates.borrow_and_update();   // see the seed note below
    ///     self.dispatch(&self.frontier());        // wake + start each ready, owned node
    ///     if updates.changed().await.is_err() { break; }
    /// }
    /// ```
    ///
    /// `borrow_and_update` is a **defensive no-op** here, not load-bearing: on
    /// this `tokio` a fresh `subscribe()` already starts the receiver at the
    /// current version, so the seeded value is *already* marked seen and the
    /// first `changed()` waits for a genuinely newer one. The loop still
    /// advances because a dispatched node's `start()` publishes a fresh
    /// snapshot, waking the loop; that node is now `Doing`, so the `Todo`-keyed
    /// frontier drops it and it is not re-dispatched.
    ///
    /// Termination is process exit: this task holds a `TaskList` clone, so the
    /// `watch::Sender` never drops while it runs and `changed()` cannot error —
    /// the `break` is a defensive arm (runtime drop is the real end).
    pub async fn run(mut self) {
        let mut updates = self.tasks.subscribe();
        // A6: the verdict receiver, taken once (one `run`, so the `take` cannot
        // fail).
        let mut judge_rx = self
            .judge_rx
            .lock()
            .unwrap()
            .take()
            .expect("Scheduler::run is called once");
        // The Failed set last escalated (empty = nothing outstanding).
        let mut last_escalated: Vec<u32> = Vec::new();
        loop {
            let _ = updates.borrow_and_update();
            // A14/R1: drop `unjudgeable` entries whose `reset_epoch` moved, BEFORE
            // computing the frontier, so a `task reset` re-arms its node.
            self.prune_unjudgeable();
            let ready = self.frontier();
            self.dispatch(&ready).await;
            // Escalate ONCE per Failed set: a real stall (something terminal and
            // nothing ready). An empty plan, an owner-less Todo plan, and a Doing
            // node mid-turn all leave `failed` empty → no escalation. (A Doing
            // node that never reports is a crash — reconciled in P4, not here.)
            let failed: Vec<u32> = self.failed_ids();
            if failed != last_escalated {
                if failed.is_empty() {
                    // Cleared by a reset/accept — drop the guard so a FUTURE
                    // identical Failed set re-escalates.
                    last_escalated.clear();
                } else if self.frontier().is_empty() {
                    self.escalate(escalation_content(
                        &self.failed_tasks(),
                        self.tasks.max_attempts(),
                    ));
                    last_escalated = failed;
                }
            }
            // A6: the deadline map must be PRUNED against the live snapshot every
            // iteration — a node that completed, was rejected, reset or reopened
            // leaves a stale entry that would otherwise fire a bogus deadline.
            let snap = self.tasks.snapshot();
            self.started
                .retain(|id, _| snap.iter().any(|t| t.id == *id && t.state == TaskState::Doing));
            // The earliest SESSION deadline. A `Script` node yields `None` by
            // construction (`session_deadline`), so it can never be booked here
            // twice — its bound is its own `run_command` timeout (A6).
            // N2: `at + d` can PANIC on an absurd `timeout_secs` (`u64::MAX` s
            // overflows `Instant`), which would silently kill this task. A
            // deadline that cannot be represented is treated as "no deadline"
            // (`checked_add` → `None`), matching the disabled case.
            let next = self
                .started
                .iter()
                .filter_map(|(id, at)| node_deadline(&snap, *id, *at).map(|when| (*id, when)))
                .min_by_key(|(_, when)| *when);
            tokio::select! {
                changed = updates.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                // The ONLY place an async judge result mutates the plan.
                Some((id, verdict)) = judge_rx.recv() => self.on_verdict(id, verdict).await,
                // The node deadline (W028). `sleep_until_opt(None)` is `pending()`,
                // so this arm is simply never ready when every node is unbounded.
                fired = sleep_until_opt(next) => {
                    if let Some(id) = fired {
                        self.started.remove(&id);
                        // The bool only exists for the A8 gate; the loop has
                        // nothing to do with it either way.
                        let _ = self.fire_node_deadline(id);
                    }
                }
            }
        }
    }

    /// The ids to dispatch now: [`TaskList::ready_ids`] (`Todo` ∧ every dep
    /// `Done`), kept where the node has an `owner` OR runs a `Script` (a `Script`
    /// node is worker-less). A `Session` node with no owner is never dispatched.
    ///
    /// Ownership is enforced again downstream: [`Registry::deliver`] → `resolve`
    /// checks the permitted set, so a node assigned to a `[peers]` alias or an
    /// unregistered id yields `NotPermitted`/`Unknown` and never dispatches —
    /// acceptable for P1, since the scheduler drives the root's own `[team]`,
    /// each member `register`ed and `set_owner`ed at spawn.
    // A `Script` node is worker-less, so the owner filter must not drop it.
    fn frontier(&self) -> Vec<u32> {
        let snapshot = self.tasks.snapshot();
        self.tasks
            .ready_ids()
            .into_iter()
            // A node awaiting an async verdict (`judging`) or blocked-and-
            // escalated (`unjudgeable`) must NOT be re-dispatched.
            .filter(|id| !self.judging.contains(id) && !self.unjudgeable.contains_key(id))
            .filter(|id| {
                snapshot.iter().any(|t| {
                    t.id == *id
                        && (t.owner.is_some() || matches!(t.run, RunSpec::Script { .. }))
                })
            })
            .collect()
    }

    /// The timer arm's effect for node `id` (W028): `fail` it if it is STILL
    /// `Doing`, re-checked on a FRESH snapshot.
    ///
    /// A8 — the re-check is load-bearing: `TaskList::fail` has NO state guard, so
    /// a `complete` landing in the same instant the deadline fires would be
    /// flipped `Done → Failed` and a `Fail` journalled into the plan, poisoning
    /// the resume for good. Returns whether the node was failed.
    ///
    /// A node timeout is `fail`, TERMINAL: the EXISTING escalation arm picks it
    /// up at the next loop top (`failed_ids()` is now non-empty). The worker's
    /// in-flight turn is deliberately NOT cancelled — D033 reuses the escalation
    /// path unchanged.
    fn fire_node_deadline(&mut self, id: u32) -> bool {
        let fresh = self.tasks.snapshot();
        let Some(task) = fresh.iter().find(|t| t.id == id) else {
            return false;
        };
        if task.state != TaskState::Doing {
            return false;
        }
        let secs = session_deadline(&fresh, id).map_or(0, |d| d.as_secs());
        let _ = self.tasks.fail(id, format!("node timeout: exceeded {secs}s"));
        true
    }

    /// A14/R1: drop `unjudgeable` entries whose `reset_epoch` has moved — a
    /// `task reset` bumps that epoch, so the entry no longer matches and the node
    /// returns to the frontier for a FRESH judge run. Called at the TOP of every
    /// `run` iteration, before [`Scheduler::frontier`].
    fn prune_unjudgeable(&mut self) {
        let snap = self.tasks.snapshot();
        self.unjudgeable
            .retain(|id, epoch| snap.iter().any(|t| t.id == *id && t.reset_epoch == *epoch));
    }

    /// Apply one async verdict on the loop thread (the ONLY place a judge result
    /// mutates the plan). `Some(true)` → clear `judging`, dispatch the node
    /// normally; `Some(false)` → `tasks.skip(id)`, clear `judging`; `None` →
    /// clear `judging`, record the node `unjudgeable` (keyed by its CURRENT
    /// `reset_epoch`), and escalate ONCE.
    async fn on_verdict(&mut self, id: u32, verdict: crate::judge::Verdict) {
        self.judging.remove(&id);
        match verdict {
            Some(true) => self.dispatch_one(id).await,
            Some(false) => {
                let _ = self.tasks.skip(id);
            }
            None => {
                let epoch = self
                    .tasks
                    .snapshot()
                    .iter()
                    .find(|t| t.id == id)
                    .map_or(0, |t| t.reset_epoch);
                if self.unjudgeable.insert(id, epoch).is_none() {
                    let title = self
                        .tasks
                        .snapshot()
                        .iter()
                        .find(|t| t.id == id)
                        .map(|t| t.title.clone())
                        .unwrap_or_default();
                    self.escalate(unjudgeable_content(id, &title, ""));
                }
            }
        }
    }

    /// Escalate to the root: deliver a `Wake` from `self_addr`. A `Wake` starts
    /// a ROOT turn with `content` as its inbound message (§6) — the root then
    /// decides: `task` op `reset` to retry, or accept (§5). Best-effort: a
    /// delivery error (`Unknown`/`Closed`) is ignored.
    fn escalate(&self, content: String) {
        let _ = self
            .registry
            .deliver(&self.self_addr, &self.root, Request::Wake { content });
    }

    /// The ids that hit the cap, ascending — `state == Failed`.
    fn failed_ids(&self) -> Vec<u32> {
        self.failed_tasks().iter().map(|t| t.id).collect()
    }

    /// The `Failed` nodes, in creation order (for the escalation text).
    fn failed_tasks(&self) -> Vec<Task> {
        self.tasks
            .snapshot()
            .into_iter()
            .filter(|t| t.state == TaskState::Failed)
            .collect()
    }
    /// Evaluate each ready node's INCOMING edges (AND-ed), then dispatch the
    /// admitted ones. See [`EdgeGate`].
    async fn dispatch(&mut self, frontier: &[u32]) {
        for &id in frontier {
            match self.edges_admit(id).await {
                EdgeGate::Admit => self.dispatch_one(id).await,
                // A genuine script verdict (exit ≠ 0): the target is Skipped.
                EdgeGate::Skip => {
                    let _ = self.tasks.skip(id);
                }
                // A judge edge spawned; the verdict arrives on `judge_rx`.
                EdgeGate::Pending => {}
                // Block-and-escalate ONCE (A13/A14): the node stays Todo and is
                // excluded from the frontier until a reset bumps its epoch.
                EdgeGate::Unjudgeable(tail) => {
                    let epoch = self
                        .tasks
                        .snapshot()
                        .iter()
                        .find(|t| t.id == id)
                        .map_or(0, |t| t.reset_epoch);
                    if self.unjudgeable.insert(id, epoch).is_none() {
                        let title = self
                            .tasks
                            .snapshot()
                            .iter()
                            .find(|t| t.id == id)
                            .map(|t| t.title.clone())
                            .unwrap_or_default();
                        self.escalate(unjudgeable_content(id, &title, &tail));
                    }
                }
            }
        }
    }

    /// Evaluate a ready node's incoming edges, AND-ed (sign-off rev-2 #5: every
    /// incoming edge must be taken). A `script` edge runs synchronously here (the
    /// SAME `before_tool_call` gate the `bash` tool runs under); a `judge` edge
    /// spawns `judge.decide` and parks the node in `judging`.
    async fn edges_admit(&mut self, id: u32) -> EdgeGate {
        // A belt-and-suspenders with `frontier`'s filter (A14): an already
        // unjudgeable node must never spawn a second judge.
        if self.unjudgeable.contains_key(&id) {
            return EdgeGate::Unjudgeable("already blocked; reset it".into());
        }
        let deps = match self.tasks.snapshot().iter().find(|t| t.id == id) {
            Some(t) => t.deps.clone(),
            None => return EdgeGate::Admit,
        };
        for dep in &deps {
            let Dep::Edged {
                id: src,
                condition: Some(cond),
            } = dep
            else {
                continue; // an unconditional edge (`Dep::Id` / `condition: None`) is taken
            };
            match cond.mode() {
                Ok(crate::config::EdgeMode::Script(cmd)) => {
                    // (a) policy first — the SAME gate the `bash` tool runs under.
                    let call = ToolCall {
                        id: format!("edge-{id}"),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "command": cmd }),
                    };
                    if let Some(reason) = self.hooks.before_tool_call(&call).await {
                        // A13: a policy refusal stays VISIBLE — never `Skip`.
                        return EdgeGate::Unjudgeable(reason);
                    }
                    let label = format!("edge-{id}");
                    // The edge's OWN bound (`None`/`0` = no bound, which
                    // `run_command` takes as `None` = unbounded).
                    let bound = workflow_bound_opt(cond.timeout_secs);
                    let (code, output) = run_command(
                        cmd,
                        &self.working_dir,
                        bound,
                        &self.cancel,
                        None,
                        &label,
                    )
                    .await;
                    match code {
                        // Exit 0: this edge is taken; continue the AND-fold.
                        Some(0) => {}
                        // A genuine non-zero exit: the ONLY `Skip` case.
                        Some(_) => return EdgeGate::Skip,
                        // Killed / never ran: not a verdict → block-and-escalate.
                        None => {
                            return EdgeGate::Unjudgeable(tail_chars(&output, PREVIEW_TAIL_CHARS));
                        }
                    }
                }
                Ok(crate::config::EdgeMode::Judge { criterion, .. }) => {
                    // The upstream artifact is the SOURCE node's output.
                    let upstream = self
                        .tasks
                        .snapshot()
                        .iter()
                        .find(|t| t.id == *src)
                        .and_then(|t| t.artifact.clone());
                    self.judging.insert(id);
                    let judge = self.judge.clone();
                    let tx = self.judge_tx.clone();
                    let criterion = criterion.to_string();
                    // The edge's bound, COPIED into the `'static` task (`Option<u64>`
                    // is `Copy`; the `&cond` is not). `None` = disabled = unbounded.
                    let bound = workflow_bound_opt(cond.timeout_secs);
                    // The borrow of `criterion` is internal to the spawned block,
                    // so the task is `'static` (no fight with `Judge::decide<'a>`).
                    tokio::spawn(async move {
                        let decided = judge.decide(&criterion, upstream.as_deref());
                        // A stalled judge is bounded by the EDGE, not the run's
                        // `--timeout` (W028). An elapsed deadline maps onto the
                        // EXISTING `Verdict::None` path — "no verdict" — so it
                        // blocks-and-escalates and is never a silent `Skip`.
                        let verdict: crate::judge::Verdict = match bound {
                            // An elapsed deadline IS "no verdict" — the existing
                            // block-and-escalate path.
                            Some(bound) => tokio::time::timeout(bound, decided)
                                .await
                                .unwrap_or_default(),
                            None => decided.await,
                        };
                        let _ = tx.send((id, verdict));
                    });
                    return EdgeGate::Pending;
                }
                // Unreachable after load validation; block-and-escalate.
                Err(_) => return EdgeGate::Unjudgeable("invalid edge condition".into()),
            }
        }
        EdgeGate::Admit
    }


    /// Wake one node. A `Session` node gets a hydrated [`Request::Wake`]
    /// (delivered only if it has an owner) and — on success — flips `Doing`. A
    /// `Script` node (worker-less) runs its command detached: `before_tool_call`
    /// first (a refusal `fail`s it *without running*), then `run_command`, whose
    /// exit code is the verdict (§5).
    /// `&mut self` for ONE reason (W028 A6): the `Session` arm records the
    /// dispatch instant in `self.started`, so `run`'s timer arm can bound the
    /// node. Both callers are already `&mut self`.
    async fn dispatch_one(&mut self, id: u32) {
        let snapshot = self.tasks.snapshot();
        let Some(task) = snapshot.iter().find(|t| t.id == id) else {
            return;
        };
        match &task.run {
            RunSpec::Session { .. } => {
                let Some(owner) = task.owner.as_ref() else {
                    return;
                };
                let deps: Vec<Task> = task
                    .deps
                    .iter()
                    .filter_map(|d| snapshot.iter().find(|t| t.id == d.id()).cloned())
                    .collect();
                let content = dispatch_content(task, &deps);
                if self
                    .registry
                    .deliver(&self.root, owner, Request::Wake { content })
                    .is_ok()
                {
                    let _ = self.tasks.start(id);
                    // A6: enter the deadline map ONLY here (a `Session` node) and
                    // only once the node is really `Doing`. A `Script` node's
                    // bound is its `run_command` timeout, so it is never
                    // double-booked by the timer arm.
                    self.started.insert(id, tokio::time::Instant::now());
                }
            }
            RunSpec::Script { command, .. } => {
                // (a) policy first — the SAME gate the `bash` tool runs under.
                let call = ToolCall {
                    id: format!("script-{id}"),
                    name: "bash".into(),
                    arguments: serde_json::json!({ "command": command }),
                };
                if let Some(reason) = self.hooks.before_tool_call(&call).await {
                    let _ = self.tasks.fail(id, reason); // the command NEVER runs
                    return;
                }
                // (b) leave the frontier SYNCHRONOUSLY, then run detached: a
                // `Script` can outlast the loop's turn, so blocking here would
                // stall every other ready node. The task holds only clones.
                let _ = self.tasks.start(id);
                // The node's OWN bound (`None`/`0` = unbounded — no default).
                // A `Script` node's deadline is this `run_command` timeout, so it
                // is deliberately NOT entered into `self.started` (A6).
                let bound = workflow_bound_opt(task.run.timeout_secs());
                let (tasks, dir, cancel, gate, command) = (
                    self.tasks.clone(),
                    self.working_dir.clone(),
                    self.cancel.clone(),
                    task.gate,
                    command.clone(),
                );
                tokio::spawn(async move {
                    let (code, output) = run_command(
                        &command,
                        &dir,
                        bound,
                        &cancel,
                        None,
                        &format!("script-{id}"),
                    )
                    .await;
                    // A4 — the CANCEL-SAFE EARLY RETURN, the load-bearing one for
                    // the resume. `run_command` returns `(None, "cancelled")` on a
                    // cancel and `(None, "timed out …")` on a timeout; the
                    // `match (code, gate)` below fails the node on ANY `None`. A
                    // cancel-induced `Failed` is PERMANENT (`reconcile` only repairs
                    // `Doing`), so it would WEDGE the resume. Check the TOKEN, not
                    // the code (both cases return `None`): a cancelled node is left
                    // `Doing`, which the next `load_at` reconciles to `Todo` and
                    // re-runs; a genuine TIMEOUT still fails it.
                    if cancel.is_cancelled() {
                        return;
                    }
                    let tail = tail_chars(&output, PREVIEW_TAIL_CHARS);
                    match (code, gate) {
                        (Some(0), false) => {
                            let _ = tasks.complete(id, Some(output));
                        }
                        (Some(0), true) => {
                            let _ = tasks.complete(id, None);
                        }
                        (_, false) => {
                            let _ = tasks.fail(id, tail);
                        }
                        (_, true) => {
                            // A misconfigured gate must not get stuck `Doing`.
                            if let Err(e) = tasks.reject(id, tail) {
                                let _ = tasks.fail(id, e);
                            }
                        }
                    }
                });
            }
        }
    }
}

/// The result of evaluating a ready node's incoming edges (AND-ed). A judge edge
/// is ASYNC, so the gate cannot report a plain bool: it needs a `Pending` arm
/// (an unjudged node must not dispatch) and an `Unjudgeable` arm (a refusal or an
/// unparseable verdict must block-and-escalate, never silently prune).
enum EdgeGate {
    /// Every incoming edge is taken (or there are none): dispatch normally.
    Admit,
    /// A `script` condition exited NON-ZERO: the target is `Skipped` for real.
    Skip,
    /// A `judge` condition spawned; the verdict arrives on `judge_rx`.
    Pending,
    /// A `script` was REFUSED by `before_tool_call`, or a judge replied `None`.
    /// Block-and-escalate (A13): visible, never a silent prune. Carries the
    /// refusal reason / reply tail for the escalation `Wake`.
    Unjudgeable(String),
}

/// The escalation `Wake` for a blocked-and-escalated edge: names the node and
/// title, a tail of the raw reply/refusal, and points at
/// `task op: reset id:<id>` (the escape hatch). Pure (like [`escalation_content`])
/// so its text is testable.
fn unjudgeable_content(id: u32, title: &str, tail: &str) -> String {
    let mut out = format!(
        "scheduler: node #{id} {title} is blocked — an edge condition could not be decided.\n\
         reset it (task op: reset id:{id}) to re-evaluate the edge."
    );
    let tail = tail.trim();
    if !tail.is_empty() {
        out.push_str(&format!("\nlast output: {tail}"));
    }
    out
}

/// The escalation `Wake` content (P2 §6). Exact shape:
///
/// ```text
/// scheduler: <N> node(s) failed — the plan is stalled.
/// #<id> <title> — <failed after <max_attempts> reworks | failed>; reset it (task op: reset id:<id>) or accept it.
/// ```
///
/// The number of reworks is `max_attempts` (the cap), NOT `task.attempts`
/// (which is cap+1 once the cap is hit).
fn escalation_content(failed: &[Task], max_attempts: u32) -> String {
    let mut out = format!(
        "scheduler: {} node(s) failed — the plan is stalled.",
        failed.len()
    );
    for t in failed {
        // A capped node was reworked (attempts = cap+1); a `fail()`-ed one was not
        // — say which, so the escalation never overstates the cause.
        let why = if t.attempts > max_attempts {
            format!("failed after {max_attempts} reworks")
        } else {
            "failed".to_string()
        };
        out.push_str(&format!(
            "\n#{} {} — {}; reset it (task op: reset id:{}) or accept it.",
            t.id, t.title, why, t.id
        ));
    }
    out
}

/// The hydrated [`Request::Wake`] content for `task` (§4): the node's
/// `#<id> <title>`, then each dep's `artifact`, then the `feedback` when the
/// node is being reworked.
///
/// Exact shape — one block per blank-line-separated section; a block is omitted
/// when empty:
///
/// ```text
/// #2 implement
///
/// #1 produced:
/// <artifact of dep #1>
///
/// Revise per this feedback:
/// <feedback>
/// ```
///
/// - the `#<dep_id> produced:` block appears once per dep **that has an
///   `artifact`** (readiness requires every dep `Done`, and `complete` carries
///   the artifact, so on the live frontier each dep contributes one);
/// - the `Revise per this feedback:` block appears only when `task.attempts > 0`
///   **and** `task.feedback.is_some()` (§6: the reject reason is the rework
///   input; on a first run `attempts == 0`, so it is absent).
///
/// Pure — no state, no I/O — so its format is unit-testable without a scheduler.
pub fn dispatch_content(task: &Task, deps: &[Task]) -> String {
    let mut sections = vec![format!("#{} {}", task.id, task.title)];
    for dep in deps {
        if let Some(artifact) = &dep.artifact {
            sections.push(format!("#{} produced:\n{}", dep.id, artifact));
        }
    }
    if task.attempts > 0
        && let Some(feedback) = &task.feedback
    {
        sections.push(format!("Revise per this feedback:\n{feedback}"));
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::broadcast;

    /// The ONE bound rule (D033, amended): an UNSET bound is no bound. `None` and
    /// `Some(0)` are both `None` = no timer — never a zero-length kill — and
    /// `Some(n)` is `n` seconds. There is no site default to fall back to.
    #[test]
    fn an_unset_or_zero_bound_is_unbounded() {
        assert_eq!(workflow_bound_opt(None), None, "unset = unbounded");
        assert_eq!(
            workflow_bound_opt(Some(0)),
            None,
            "0 = unbounded, NOT an instant kill"
        );
        assert_eq!(
            workflow_bound_opt(Some(7)),
            Some(Duration::from_secs(7))
        );
    }

    /// The same rule at the NODE level: `session_deadline` yields no deadline for
    /// an unset (or `0`) `Session` bound, and the bound itself for `Some(n)`.
    #[test]
    fn an_unset_node_bound_yields_no_deadline() {
        let tasks = TaskList::new();
        let id = tasks.create("node", vec![], false).unwrap().id;
        let set = |secs: Option<u64>| {
            tasks
                .configure(
                    id,
                    RunSpec::Session {
                        member: None,
                        timeout_secs: secs,
                    },
                    false,
                )
                .unwrap();
            session_deadline(&tasks.snapshot(), id)
        };
        assert_eq!(set(None), None, "an unset node bound is unbounded");
        assert_eq!(set(Some(0)), None, "`0` is unbounded too");
        assert_eq!(set(Some(5)), Some(Duration::from_secs(5)));
    }

    /// THE BEHAVIOUR GATE for the amendment: a `Session` node with NO bound, whose
    /// worker never reports, must NOT be failed by the timer arm — even long after
    /// the retired 300 s default would have fired. Without this, a future edit
    /// could silently reintroduce a default and only a live run would notice.
    #[tokio::test(start_paused = true)]
    async fn an_unbounded_session_node_is_never_timed_out() {
        let (registry, root, _worker, worker_id) = wiring();
        let tasks = TaskList::new();
        let id = tasks.create("patient", vec![], false).unwrap().id;
        tasks.assign(id, worker_id.clone()).unwrap();
        // No `configure`: the node keeps the default `RunSpec::Session` — no bound.
        assert_eq!(tasks.snapshot()[0].run.timeout_secs(), None);

        tokio::spawn(
            Scheduler::new(
                registry,
                root,
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        // It dispatches and parks Doing (the worker never reports) …
        for _ in 0..200 {
            if tasks.snapshot()[0].state == TaskState::Doing {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(tasks.snapshot()[0].state, TaskState::Doing, "dispatched");
        // … and stays Doing past the old 300 s default (plus a generous margin).
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        assert_eq!(
            tasks.snapshot()[0].state,
            TaskState::Doing,
            "an unset node bound must never time out — no default exists"
        );
        assert!(tasks.snapshot()[0].feedback.is_none(), "no failure reason");
    }

    /// A4, the resume's load-bearing rule: cancelling the run token leaves a
    /// `Script` node **`Doing`**, never `Failed`. `reconcile` only repairs
    /// `Doing → Todo`, so a cancel-induced `Fail` would wedge every later resume.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_script_node_stays_doing() {
        let tasks = TaskList::new();
        let id = tasks.create("runaway", vec![], false).unwrap().id;
        tasks
            .configure(
                id,
                RunSpec::Script {
                    command: "sleep 30".into(),
                    timeout_secs: None, // no bound at all: only the cancel stops it
                },
                false,
            )
            .unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        tokio::spawn(
            Scheduler::new(
                Registry::new(),
                SessionId::agent("orchestrator"),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                cancel.clone(),
                Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        // It starts (and the detached `run_command` is in flight) …
        for _ in 0..200 {
            if tasks.snapshot()[0].state == TaskState::Doing {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(tasks.snapshot()[0].state, TaskState::Doing, "dispatched");
        // … then the token is cancelled: the node must NOT be failed.
        cancel.cancel();
        for _ in 0..200 {
            if tasks.snapshot()[0].state != TaskState::Doing {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            tasks.snapshot()[0].state,
            TaskState::Doing,
            "a cancelled Script node stays `Doing` so the resume re-runs it"
        );
        assert!(tasks.snapshot()[0].feedback.is_none(), "no failure reason");
    }

    /// N2: an over-representable `timeout_secs` yields "no deadline" rather than
    /// an `Instant` overflow panic that would silently kill the scheduler task.
    #[test]
    fn an_over_representable_deadline_is_no_deadline() {
        let tasks = TaskList::new();
        let id = tasks.create("absurd", vec![], false).unwrap().id;
        tasks
            .configure(
                id,
                RunSpec::Session {
                    member: None,
                    timeout_secs: Some(u64::MAX),
                },
                false,
            )
            .unwrap();
        let snap = tasks.snapshot();
        // The configured bound is read back verbatim …
        assert_eq!(session_deadline(&snap, id), Some(Duration::from_secs(u64::MAX)));
        // … but the absolute instant cannot be represented, so the arm is skipped
        // instead of panicking (`u64::MAX` seconds overflows `Instant`).
        assert_eq!(node_deadline(&snap, id, tokio::time::Instant::now()), None);

        // A sane bound does compute, and "disabled" (`Some(0)`) is no deadline.
        tasks
            .configure(
                id,
                RunSpec::Session {
                    member: None,
                    timeout_secs: Some(1),
                },
                false,
            )
            .unwrap();
        let snap = tasks.snapshot();
        assert!(node_deadline(&snap, id, tokio::time::Instant::now()).is_some());
        tasks
            .configure(
                id,
                RunSpec::Session {
                    member: None,
                    timeout_secs: Some(0),
                },
                false,
            )
            .unwrap();
        let snap = tasks.snapshot();
        assert_eq!(node_deadline(&snap, id, tokio::time::Instant::now()), None);
    }

    /// A `judge` that never decides — for the edge-deadline gate. No model.
    struct HangingJudge;
    impl crate::judge::Judge for HangingJudge {
        fn decide<'a>(
            &'a self,
            _criterion: &'a str,
            _upstream: Option<&'a str>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::judge::Verdict> + Send + 'a>>
        {
            Box::pin(std::future::pending())
        }
    }

    /// Poll the plan until `id` reaches `want`, advancing the VIRTUAL clock (the
    /// tests run `start_paused`). A wall-clock `timeout` is unusable there: it
    /// would elapse the moment every task parks, before the timer arm runs.
    async fn wait_for_paused(tasks: &TaskList, id: u32, want: TaskState) {
        for _ in 0..2_000 {
            let hit = tasks
                .snapshot()
                .iter()
                .any(|t| t.id == id && t.state == want);
            if hit {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!(
            "node {id} never reached {want:?}: {:?}",
            tasks.snapshot()
        );
    }

    /// Read the next `MessageReceived` under a PAUSED clock: `next_message`'s
    /// inner wall-clock `timeout` cannot be used there (the runtime would advance
    /// the virtual clock past it while the real work — a killed child, an actor
    /// turn — is still settling). Polls instead, letting the clock tick in steps.
    async fn next_message_paused(rx: &mut broadcast::Receiver<AgentEvent>) -> String {
        for _ in 0..2_000 {
            loop {
                match rx.try_recv() {
                    Ok(AgentEvent::MessageReceived { content, .. }) => return content,
                    Ok(_) => continue,
                    Err(broadcast::error::TryRecvError::Empty) => break,
                    Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                    Err(broadcast::error::TryRecvError::Closed) => panic!("channel closed"),
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no escalation Wake arrived");
    }

    /// A `script` edge whose command outlives the edge's own `timeout_secs` is
    /// `Unjudgeable` → the target stays `Todo` and the run blocks-and-escalates.
    /// NEVER `Skip` (only a genuine non-zero exit skips).
    #[tokio::test(start_paused = true)]
    async fn a_timed_out_script_edge_blocks_and_escalates() {
        let (registry, root, _worker, worker_id) = wiring();
        let (root_handle, self_addr) = escalate_edge(&registry, &root);
        let tasks = TaskList::new();
        let a = tasks.create("a", vec![], false).unwrap().id;
        tasks.assign(a, worker_id.clone()).unwrap();
        let mut cond = script_cond("sleep 30");
        cond.timeout_secs = Some(1); // 1 s bound on a 30 s command
        let b = tasks
            .create("b", vec![Dep::Edged { id: a, condition: Some(cond) }], false)
            .unwrap()
            .id;
        tasks.assign(b, worker_id.clone()).unwrap();

        let mut root_rx = root_handle.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                self_addr,
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        tasks.complete(a, None).unwrap();
        let content = next_message_paused(&mut root_rx).await;
        assert!(
            content.contains(&format!("id:{b}")),
            "the escalation names the blocked target: {content}"
        );
        assert_eq!(
            tasks.snapshot().into_iter().find(|t| t.id == b).unwrap().state,
            TaskState::Todo,
            "a timed-out edge blocks-and-escalates; it is never Skipped"
        );
    }

    /// A `judge` edge whose `decide` never returns is bounded by the EDGE (not
    /// the run's `--timeout`): elapsed maps onto the existing `Verdict::None`
    /// path → block-and-escalate, never `Skip`.
    #[tokio::test(start_paused = true)]
    async fn a_timed_out_judge_blocks_and_escalates() {
        let (registry, root, _worker, worker_id) = wiring();
        let (root_handle, self_addr) = escalate_edge(&registry, &root);
        let tasks = TaskList::new();
        let a = tasks.create("a", vec![], false).unwrap().id;
        tasks.assign(a, worker_id.clone()).unwrap();
        let mut cond = judge_cond("forever?");
        cond.timeout_secs = Some(1);
        let b = tasks
            .create("b", vec![Dep::Edged { id: a, condition: Some(cond) }], false)
            .unwrap()
            .id;
        tasks.assign(b, worker_id.clone()).unwrap();

        let mut root_rx = root_handle.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                self_addr,
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(HangingJudge),
            )
            .run(),
        );

        tasks.complete(a, None).unwrap();
        let content = next_message_paused(&mut root_rx).await;
        assert!(
            content.contains(&format!("id:{b}")),
            "the escalation names the blocked target: {content}"
        );
        assert_eq!(
            tasks.snapshot().into_iter().find(|t| t.id == b).unwrap().state,
            TaskState::Todo,
            "a timed-out judge blocks-and-escalates; it is never Skipped"
        );
    }

    /// A `Script` node's own `timeout_secs` bounds its `run_command`, and the
    /// resulting `fail` names the limit — the same shape the `bash` tool's own
    /// timeout produces.
    #[tokio::test(start_paused = true)]
    async fn a_script_node_timeout_names_the_limit() {
        let tasks = TaskList::new();
        let id = tasks.create("slow", vec![], false).unwrap().id;
        tasks
            .configure(
                id,
                RunSpec::Script {
                    command: "sleep 30".into(),
                    timeout_secs: Some(1),
                },
                false,
            )
            .unwrap();

        spawn_scheduler(&tasks, HooksSet::default());
        wait_for_paused(&tasks, id, TaskState::Failed).await;
        let t = &tasks.snapshot()[0];
        assert_eq!(t.state, TaskState::Failed);
        assert!(
            t.feedback.as_deref().unwrap_or_default().contains("timed out after 1s"),
            "the reason names the limit: {:?}",
            t.feedback
        );
    }

    /// A `Session` node past its own deadline is `fail`ed by the timer arm, the
    /// reason names the limit, and its dependent is never dispatched.
    #[tokio::test(start_paused = true)]
    async fn a_session_node_past_its_deadline_fails() {
        let (registry, root, _worker, worker_id) = wiring();
        let tasks = TaskList::new();
        let a = tasks.create("slow", vec![], false).unwrap().id;
        tasks.assign(a, worker_id.clone()).unwrap();
        tasks
            .configure(
                a,
                RunSpec::Session {
                    member: None,
                    timeout_secs: Some(1),
                },
                false,
            )
            .unwrap();
        let b = tasks.create("after", vec![Dep::Id(a)], false).unwrap().id;
        tasks.assign(b, worker_id.clone()).unwrap();

        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        wait_for_paused(&tasks, a, TaskState::Failed).await;
        let node = |id: u32| tasks.snapshot().into_iter().find(|t| t.id == id).unwrap();
        assert!(
            node(a)
                .feedback
                .as_deref()
                .unwrap_or_default()
                .contains("node timeout: exceeded 1s"),
            "the reason names the limit: {:?}",
            node(a).feedback
        );
        assert_eq!(
            node(b).state,
            TaskState::Todo,
            "a dependent of a failed node stays blocked"
        );
    }

    /// A8, as its own gate: a node that COMPLETED is never failed by its stale
    /// deadline — `TaskList::fail` has no state guard, so the re-check is the
    /// only thing standing between a same-instant `complete` and a `Fail` line
    /// journalled into the plan (which would poison the resume).
    #[tokio::test]
    async fn a_completed_node_is_never_failed_by_its_stale_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let (registry, root, _worker, worker_id) = wiring();
        let tasks = TaskList::with_journal_at(path.clone(), 3);
        let mut sched = Scheduler::new(
            registry,
            root,
            tasks.clone(),
            SessionId::agent("scheduler"),
            std::env::temp_dir(),
            HooksSet::default(),
            tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
        );

        // Dispatched (so it has a `started` entry) …
        let done = tasks.create("done", vec![], false).unwrap().id;
        tasks
            .configure(
                done,
                RunSpec::Session {
                    member: None,
                    timeout_secs: Some(1),
                },
                false,
            )
            .unwrap();
        tasks.assign(done, worker_id.clone()).unwrap();
        tasks.start(done).unwrap();
        sched
            .started
            .insert(done, tokio::time::Instant::now());
        // … then COMPLETED before the deadline is serviced.
        tasks.complete(done, Some("ok".into())).unwrap();

        assert!(
            !sched.fire_node_deadline(done),
            "a completed node must not be failed by its stale deadline"
        );
        assert_eq!(
            tasks.snapshot().into_iter().find(|t| t.id == done).unwrap().state,
            TaskState::Done
        );
        let journal = std::fs::read_to_string(&path).unwrap();
        assert!(
            !journal.contains(r#" "fail" "#) && !journal.contains(r#""op":"fail""#),
            "no `Fail` op was journalled: {journal}"
        );

        // … and the arm DOES fire for a node that is still `Doing`.
        let doing = tasks.create("doing", vec![], false).unwrap().id;
        tasks
            .configure(
                doing,
                RunSpec::Session {
                    member: None,
                    timeout_secs: Some(2),
                },
                false,
            )
            .unwrap();
        tasks.start(doing).unwrap();
        assert!(sched.fire_node_deadline(doing), "a `Doing` node IS failed");
        let node = tasks.snapshot().into_iter().find(|t| t.id == doing).unwrap();
        assert_eq!(node.state, TaskState::Failed);
        assert!(
            node.feedback
                .as_deref()
                .unwrap_or_default()
                .contains("node timeout: exceeded 2s"),
            "the reason names the limit: {:?}",
            node.feedback
        );
    }

    use wcode_harness::actor::{SessionActor, SessionHandle};
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::AgentEvent;
    use wcode_harness::hooks::{Hooks, HooksSet};
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    use super::*;
    use crate::tasks::{Dep, RunSpec, TaskState};

    /// A task record carrying just the fields the content builder reads.
    fn node(id: u32, title: &str) -> Task {
        Task {
            id,
            title: title.into(),
            owner: None,
            state: TaskState::Todo,
            deps: Vec::new(),
            attempts: 0,
            reset_epoch: 0,
            feedback: None,
            artifact: None,
            gate: false,
            run: RunSpec::Session {
                member: None,
                timeout_secs: None,
            },
        }
    }

    /// A live worker actor over a never-streaming model (mirrors the
    /// `session()` seam in `agents.rs`): enough to observe an inbound `Wake`
    /// as a `MessageReceived` event.
    fn worker_session() -> SessionHandle {
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        SessionActor::spawn(Agent::new(AgentConfig {
            system: "sys".into(),
            tools: Vec::new(),
            llm: LlmOpts::default(),
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
            plan_mode: wcode_harness::hooks::PlanModeHandle::new(),
        }))
    }

    /// `dispatch_content` hydrates exactly the doc's shape: title, each dep's
    /// artifact (in dep order), and the feedback only when `attempts > 0`.
    #[test]
    fn content_hydrates_title_artifacts_and_feedback() {
        let mut task = node(2, "implement");
        task.attempts = 1;
        task.feedback = Some("fix X".into());
        let mut dep = node(1, "explore");
        dep.artifact = Some("A\u{2081}".into());

        assert_eq!(
            dispatch_content(&task, &[dep]),
            "#2 implement\n\n#1 produced:\nA\u{2081}\n\nRevise per this feedback:\nfix X"
        );

        // Every optional block is omitted when its source is empty.
        assert_eq!(dispatch_content(&node(3, "solo"), &[]), "#3 solo");
    }

    /// A ready, owned node is woken — the worker's handle sees the hydrated
    /// `Wake` as `MessageReceived` — and the node reaches `Doing`; an owner-less
    /// node is left alone. Real `Registry` + real `SessionActor` (fake
    /// `StreamFn`) + real `TaskList`.
    #[tokio::test]
    async fn dispatch_wakes_owned_ready_nodes_only() {
        let registry = Registry::new();
        let root = SessionId::agent("orchestrator");
        let worker = worker_session();
        let worker_id = SessionId::agent("w1");
        registry.register(worker_id.clone(), worker.clone());
        registry.set_owner(worker_id.clone(), root.clone()); // permit root → worker

        let tasks = TaskList::new();
        let owned = tasks.create("implement", vec![], false).unwrap();
        tasks.assign(owned.id, worker_id.clone()).unwrap();
        let orphan = tasks.create("no one", vec![], false).unwrap();

        let expected = dispatch_content(&owned, &[]);
        let mut rx = worker.subscribe();
        let mut task_updates = tasks.subscribe();
    tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        // The worker sees the hydrated `Wake`, attributed to the root.
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("an event")
            .expect("open");
        assert!(
            matches!(&event, AgentEvent::MessageReceived { from, content }
                if from == &root && content == &expected),
            "{event:?}"
        );

        // And the node reached `Doing` (via `start`'s publish).
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = tasks
                    .snapshot()
                    .iter()
                    .find(|t| t.id == owned.id)
                    .unwrap()
                    .state;
                if state == TaskState::Doing {
                    break;
                }
                task_updates.changed().await.expect("watch open");
            }
        })
        .await
        .expect("the node reaches Doing");

        // The owner-less node was never touched.
        assert_eq!(
            tasks
                .snapshot()
                .iter()
                .find(|t| t.id == orphan.id)
                .unwrap()
                .state,
            TaskState::Todo,
            "an owner-less node is never dispatched"
        );
    }

    /// `deliver` failure leaves the node `Todo` (never `Doing`) and does not
    /// spin. A node assigned to an address with no ownership edge → `NotPermitted`.
    #[tokio::test]
    async fn a_undeliverable_node_stays_todo() {
        let registry = Registry::new();
        let root = SessionId::agent("orchestrator");
        let worker = worker_session();
        let worker_id = SessionId::agent("w1");
        registry.register(worker_id.clone(), worker.clone());
        // NOTE: no `set_owner` — the root may not address this worker.

        let tasks = TaskList::new();
        let node = tasks.create("t", vec![], false).unwrap();
        tasks.assign(node.id, worker_id.clone()).unwrap();

        let mut rx = worker.subscribe();
    tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            tasks.snapshot()[0].state,
            TaskState::Todo,
            "an undeliverable node stays Todo"
        );
        assert!(rx.try_recv().is_err(), "no delivery to an unowned node");
    }

    /// Read the next `MessageReceived` (skipping the turn-lifecycle events an
    /// empty-stream model still emits).
    async fn next_message(rx: &mut broadcast::Receiver<AgentEvent>) -> String {
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .expect("a message")
                .expect("open");
            if let AgentEvent::MessageReceived { content, .. } = ev {
                return content;
            }
        }
    }

    /// Whether a `MessageReceived` is already queued (non-blocking).
    fn has_pending(rx: &mut broadcast::Receiver<AgentEvent>) -> bool {
        loop {
            match rx.try_recv() {
                Ok(AgentEvent::MessageReceived { .. }) => return true,
                Ok(_) => continue,
                Err(broadcast::error::TryRecvError::Empty) => return false,
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => return false,
            }
        }
    }

    /// A registry + root id + a live worker actor owned by the root.
    fn wiring() -> (Registry, SessionId, SessionHandle, SessionId) {
        let registry = Registry::new();
        let root = SessionId::agent("orchestrator");
        let worker = worker_session();
        let worker_id = SessionId::agent("w1");
        registry.register(worker_id.clone(), worker.clone());
        registry.set_owner(worker_id.clone(), root.clone());
        (registry, root, worker, worker_id)
    }

    /// Register a root mailbox and a `self_addr` owned by the root, so an
    /// escalation `Wake` is permitted and observable on the returned handle.
    fn escalate_edge(registry: &Registry, root: &SessionId) -> (SessionHandle, SessionId) {
        let root_handle = worker_session();
        registry.register(root.clone(), root_handle.clone());
        let self_addr = SessionId::agent("scheduler");
        registry.set_owner(self_addr.clone(), root.clone());
        (root_handle, self_addr)
    }

    /// `escalation_content` names the cap (the rework COUNT, not `attempts`) and
    /// the node to reset.
    #[test]
    fn escalation_content_names_the_cap() {
        let mut t = node(2, "implement");
        t.state = TaskState::Failed;
        t.attempts = 4; // cap+1 at a cap hit — must NOT be rendered
        assert_eq!(
            escalation_content(&[t], 3),
            "scheduler: 1 node(s) failed — the plan is stalled.\n\
             #2 implement — failed after 3 reworks; reset it (task op: reset id:2) or accept it."
        );

        // A `fail()`-ed node (no rework) renders a plain `failed`.
        let mut f = node(3, "build");
        f.state = TaskState::Failed;
        f.attempts = 0;
        assert_eq!(
            escalation_content(&[f], 3),
            "scheduler: 1 node(s) failed — the plan is stalled.\n\
             #3 build — failed; reset it (task op: reset id:3) or accept it."
        );
    }

    /// Full cycle: work + gate; the gate's reject re-opens the work node, which
    /// re-dispatches WITH the feedback block, and the gate re-runs only after the
    /// work node is `Done` again.
    #[tokio::test]
    async fn reject_reopens_and_redispatches_the_work_node() {
        let (registry, root, worker, worker_id) = wiring();
        let tasks = TaskList::new();
        let work = tasks.create("implement", vec![], false).unwrap();
        let gate = tasks.create("verify", vec![Dep::Id(work.id)], true).unwrap();
        tasks.assign(work.id, worker_id.clone()).unwrap();
        tasks.assign(gate.id, worker_id.clone()).unwrap();

        let mut rx = worker.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        assert_eq!(next_message(&mut rx).await, format!("#{} implement", work.id));

        tasks.complete(work.id, Some("A1".into())).unwrap();
        assert_eq!(
            next_message(&mut rx).await,
            format!("#{} verify\n\n#{} produced:\nA1", gate.id, work.id)
        );

        tasks.reject(gate.id, "redo").unwrap();
        assert_eq!(
            next_message(&mut rx).await,
            format!("#{} implement\n\nRevise per this feedback:\nredo", work.id)
        );

        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(
            !has_pending(&mut rx),
            "the gate is not re-dispatched while its dep is Todo"
        );
    }

    /// Cap → `reset` → re-dispatch → the plan completes, with a REAL gate. After
    /// the cap, `reject` must have returned the gate to `Todo`; otherwise it
    /// stays `Doing` and never re-runs (the P2 blocker).
    #[tokio::test]
    async fn cap_then_reset_resumes_and_the_plan_completes() {
        let (registry, root, worker, worker_id) = wiring();
        let tasks = TaskList::new();
        let work = tasks.create("implement", vec![], false).unwrap();
        let gate = tasks.create("verify", vec![Dep::Id(work.id)], true).unwrap();
        tasks.assign(work.id, worker_id.clone()).unwrap();
        tasks.assign(gate.id, worker_id.clone()).unwrap();

        let mut rx = worker.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        let _ = next_message(&mut rx).await; // initial work dispatch
        for _ in 0..3 {
            tasks.reject(gate.id, "nope").unwrap();
            let _ = next_message(&mut rx).await; // work re-dispatched
        }
        tasks.start(gate.id).unwrap(); // the gate is on it again
        tasks.reject(gate.id, "nope").unwrap(); // 4th → cap

        assert_eq!(tasks.snapshot()[0].state, TaskState::Failed, "the dep capped");
        assert_eq!(
            tasks.snapshot()[1].state,
            TaskState::Todo,
            "reject re-armed the gate (the P2 blocker fix)"
        );

        // reset → re-dispatch → Done → the gate re-runs → complete.
        tasks.reset(work.id).unwrap();
        let _ = next_message(&mut rx).await; // work re-dispatched
        tasks.complete(work.id, Some("A2".into())).unwrap();
        assert_eq!(
            next_message(&mut rx).await,
            format!("#{} verify\n\n#{} produced:\nA2", gate.id, work.id),
            "the gate re-runs after the dep is Done again"
        );
        tasks.complete(gate.id, Some("ok".into())).unwrap();

        let snap = tasks.snapshot();
        assert_eq!(snap[0].state, TaskState::Done);
        assert_eq!(snap[1].state, TaskState::Done, "the plan completes");
    }

    /// A cap escalates to the root exactly ONCE per Failed set.
    #[tokio::test]
    async fn cap_escalates_exactly_once() {
        let (registry, root, _worker, worker_id) = wiring();
        let (root_handle, self_addr) = escalate_edge(&registry, &root);
        let tasks = TaskList::new();
        let work = tasks.create("implement", vec![], false).unwrap();
        let gate = tasks.create("verify", vec![Dep::Id(work.id)], true).unwrap();
        tasks.assign(work.id, worker_id.clone()).unwrap();
        tasks.assign(gate.id, worker_id.clone()).unwrap();

        let mut root_rx = root_handle.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                self_addr,
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        for _ in 0..4 {
            tasks.reject(gate.id, "nope").unwrap();
        }
        assert_eq!(tasks.snapshot()[0].state, TaskState::Failed);

        let content = next_message(&mut root_rx).await;
        assert!(content.contains("node(s) failed"), "{content}");
        assert!(
            content.contains(&format!("failed after {} reworks", tasks.max_attempts())),
            "{content}"
        );
        assert!(content.contains(&format!("reset id:{}", work.id)), "{content}");

        // A further mutation with the SAME Failed set does not re-escalate.
        tasks.create("noise", vec![], false).unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !has_pending(&mut root_rx),
            "no second escalation for the same Failed set"
        );
    }

    /// Spawn a scheduler over `tasks` (an empty registry — `Script` nodes need no
    /// peers) with `hooks`.
    fn spawn_scheduler(tasks: &TaskList, hooks: HooksSet) {
        tokio::spawn(
            Scheduler::new(
                Registry::new(),
                SessionId::agent("orchestrator"),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                hooks,
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );
    }

    /// Wait (via the plan's watch) until `id` is `want`.
    async fn wait_for(tasks: &TaskList, id: u32, want: TaskState) {
        let mut updates = tasks.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if tasks
                    .snapshot()
                    .iter()
                    .any(|t| t.id == id && t.state == want)
                {
                    return;
                }
                updates.changed().await.expect("watch open");
            }
        })
        .await
        .expect("node reaches the expected state");
    }

    /// A test hook refusing any `bash` call whose command contains `BLOCK` — lets
    /// a `Script` be blocked safely (no dangerous command is ever executed).
    struct BlockMarker;
    #[async_trait::async_trait]
    impl Hooks for BlockMarker {
        async fn before_tool_call(&self, call: &ToolCall) -> Option<String> {
            let cmd = call.arguments.get("command").and_then(|c| c.as_str())?;
            cmd.contains("BLOCK")
                .then(|| "blocked by test policy".to_string())
        }
    }

    /// A `Script` work node runs detached and lands `Done` (its output as the
    /// artifact) on exit 0.
    #[tokio::test]
    async fn a_script_work_node_completes_on_zero() {
        let tasks = TaskList::new();
        let id = tasks.create("build", vec![], false).unwrap().id;
        tasks
            .configure(id, RunSpec::Script {
                command: "echo ok".into(),
                timeout_secs: None,
            }, false)
            .unwrap();
        spawn_scheduler(&tasks, HooksSet::default());
        wait_for(&tasks, id, TaskState::Done).await;
        let t = &tasks.snapshot()[0];
        assert_eq!(t.state, TaskState::Done);
        assert!(
            t.artifact.as_deref().unwrap_or_default().contains("ok"),
            "{:?}",
            t.artifact
        );
    }

    /// A non-zero `Script` work node lands `Failed` with `attempts == 0` — a
    /// direct `fail`, NOT a rework (this fails a `reopen`-based impl).
    #[tokio::test]
    async fn a_script_work_node_fails_on_nonzero() {
        let tasks = TaskList::new();
        let id = tasks.create("build", vec![], false).unwrap().id;
        tasks
            .configure(
                id,
                RunSpec::Script {
                    command: "false".into(),
                    timeout_secs: None,
                },
                false,
            )
            .unwrap();
        spawn_scheduler(&tasks, HooksSet::default());
        wait_for(&tasks, id, TaskState::Failed).await;
        assert_eq!(tasks.snapshot()[0].attempts, 0, "a fail is not a rework");
    }

    /// A `Script` GATE with a failing command `reject`s (re-opens the gated dep +
    /// cone); a passing command completes the gate.
    #[tokio::test]
    async fn a_script_gate_rejects_then_passes() {
        let tasks = TaskList::new();
        let work = tasks.create("work", vec![], false).unwrap().id;
        let gate = tasks.create("verify", vec![Dep::Id(work)], true).unwrap().id;
        tasks
            .configure(
                gate,
                RunSpec::Script {
                    command: "false".into(),
                    timeout_secs: None,
                },
                true,
            )
            .unwrap();
        spawn_scheduler(&tasks, HooksSet::default());

        // The work node is a Session node with no owner — complete it by hand.
        tasks.complete(work, Some("A".into())).unwrap();
        wait_for(&tasks, work, TaskState::Todo).await; // rejected → re-opened
        assert_eq!(
            tasks
                .snapshot()
                .iter()
                .find(|t| t.id == gate)
                .unwrap()
                .state,
            TaskState::Todo
        );

        // A passing gate completes.
        tasks
            .configure(
                gate,
                RunSpec::Script {
                    command: "true".into(),
                    timeout_secs: None,
                },
                true,
            )
            .unwrap();
        tasks.complete(work, Some("A2".into())).unwrap();
        wait_for(&tasks, gate, TaskState::Done).await;
    }

    /// A blocked `Script` command `fail`s the node with the policy reason and the
    /// command NEVER runs.
    #[tokio::test]
    async fn a_blocked_script_fails_without_running() {
        let tasks = TaskList::new();
        let id = tasks.create("danger", vec![], false).unwrap().id;
        let marker = std::env::temp_dir().join(format!("wcode-p3-blocked-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        // `BLOCK` trips the hook; the `touch` after the `;` must never run.
        tasks
            .configure(
                id,
                RunSpec::Script {
                    command: format!("echo BLOCK; touch {}", marker.display()),
                    timeout_secs: None,
                },
                false,
            )
            .unwrap();
        spawn_scheduler(&tasks, HooksSet::one(Arc::new(BlockMarker)));
        wait_for(&tasks, id, TaskState::Failed).await;
        let t = &tasks.snapshot()[0];
        assert_eq!(t.attempts, 0);
        assert!(
            t.feedback
                .as_deref()
                .unwrap_or_default()
                .contains("blocked by test policy"),
            "{:?}",
            t.feedback
        );
        assert!(!marker.exists(), "a blocked command must never run");
    }

    /// `BashRiskHooks` refuses a catastrophic `Script` command (checked through
    /// the hook directly — the command is never executed).
    #[tokio::test]
    async fn bash_risk_hooks_refuse_a_catastrophic_script() {
        use wcode_harness::hooks::BashRiskHooks;
        let hooks = HooksSet::one(Arc::new(BashRiskHooks::new()));
        let call = ToolCall {
            id: "script-1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": "rm -rf /" }),
        };
        assert!(hooks.before_tool_call(&call).await.is_some());
    }

    /// A `script` edge condition with the given shell command.
    fn script_cond(cmd: &str) -> crate::config::EdgeCondition {
        crate::config::EdgeCondition {
            script: Some(cmd.into()),
            ..Default::default()
        }
    }

    /// A `judge` edge condition naming `judger` with the given criterion.
    fn judge_cond(criterion: &str) -> crate::config::EdgeCondition {
        crate::config::EdgeCondition {
            judge: Some("judger".into()),
            criterion: Some(criterion.into()),
            ..Default::default()
        }
    }

    /// A `script` edge whose command exits non-zero makes its target `Skipped`
    /// and it is NEVER dispatched (no `Doing`, no `Wake`). Uses `false` (a
    /// `before_tool_call`-safe command), so `HooksSet::default()`.
    #[tokio::test]
    async fn a_false_script_condition_skips_the_target() {
        let (registry, root, worker, worker_id) = wiring();
        let tasks = TaskList::new();
        let a = tasks.create("a", vec![], false).unwrap().id;
        tasks.assign(a, worker_id.clone()).unwrap();
        let b = tasks
            .create(
                "b",
                vec![Dep::Edged {
                    id: a,
                    condition: Some(script_cond("false")),
                }],
                false,
            )
            .unwrap()
            .id;
        tasks.assign(b, worker_id.clone()).unwrap();

        let mut rx = worker.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        // `a` is dispatched (a `Wake`); complete it so `b` becomes ready.
        assert_eq!(next_message(&mut rx).await, format!("#{a} a"));
        tasks.complete(a, None).unwrap();

        // `b`'s edge is a false script → Skipped, and it never ran.
        wait_for(&tasks, b, TaskState::Skipped).await;
        let bnode = tasks.snapshot().into_iter().find(|t| t.id == b).unwrap();
        assert_eq!(bnode.state, TaskState::Skipped);
        assert_eq!(bnode.attempts, 0, "a false edge never runs the node");
        // Never dispatched: no second `Wake` reached the worker.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!has_pending(&mut rx), "the target was never dispatched");
    }

    /// `FakeJudge::new(vec![Some(true)])` → the guarded node is dispatched
    /// (reaches the worker's `Wake`); `vec![Some(false)]` → the target is
    /// `Skipped`, never dispatched. No live model.
    #[tokio::test]
    async fn a_judge_take_dispatches_and_skip_marks_skipped() {
        // TAKE: the guarded node is dispatched.
        {
            let (registry, root, worker, worker_id) = wiring();
            let tasks = TaskList::new();
            let a = tasks.create("a", vec![], false).unwrap().id;
            tasks.assign(a, worker_id.clone()).unwrap();
            let b = tasks
                .create(
                    "b",
                    vec![Dep::Edged {
                        id: a,
                        condition: Some(judge_cond("take?")),
                    }],
                    false,
                )
                .unwrap()
                .id;
            tasks.assign(b, worker_id.clone()).unwrap();

            let mut rx = worker.subscribe();
            tokio::spawn(
                Scheduler::new(
                    registry,
                    root.clone(),
                    tasks.clone(),
                    SessionId::agent("scheduler"),
                    std::env::temp_dir(),
                    HooksSet::default(),
                    tokio_util::sync::CancellationToken::new(),
                    Arc::new(crate::judge::FakeJudge::new(vec![Some(true)])),
                )
                .run(),
            );

            assert_eq!(next_message(&mut rx).await, format!("#{a} a"));
            tasks.complete(a, None).unwrap();
            assert_eq!(
                next_message(&mut rx).await,
                format!("#{b} b"),
                "a TAKE verdict dispatches b"
            );
        }
        // SKIP: the guarded node is `Skipped`, never dispatched.
        {
            let (registry, root, worker, worker_id) = wiring();
            let tasks = TaskList::new();
            let a = tasks.create("a", vec![], false).unwrap().id;
            tasks.assign(a, worker_id.clone()).unwrap();
            let b = tasks
                .create(
                    "b",
                    vec![Dep::Edged {
                        id: a,
                        condition: Some(judge_cond("skip?")),
                    }],
                    false,
                )
                .unwrap()
                .id;
            tasks.assign(b, worker_id.clone()).unwrap();

            let mut rx = worker.subscribe();
            tokio::spawn(
                Scheduler::new(
                    registry,
                    root.clone(),
                    tasks.clone(),
                    SessionId::agent("scheduler"),
                    std::env::temp_dir(),
                    HooksSet::default(),
                    tokio_util::sync::CancellationToken::new(),
                    Arc::new(crate::judge::FakeJudge::new(vec![Some(false)])),
                )
                .run(),
            );

            assert_eq!(next_message(&mut rx).await, format!("#{a} a"));
            tasks.complete(a, None).unwrap();
            wait_for(&tasks, b, TaskState::Skipped).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(!has_pending(&mut rx), "a SKIP verdict never dispatches b");
        }
    }

    /// `FakeJudge::new(vec![None])` → the node stays `Todo`, lands in
    /// `unjudgeable`, and the root receives EXACTLY ONE escalation `Wake` — and no
    /// tight re-judge loop.
    #[tokio::test]
    async fn an_unparseable_verdict_escalates_once() {
        let (registry, root, _worker, worker_id) = wiring();
        let (root_handle, self_addr) = escalate_edge(&registry, &root);
        let tasks = TaskList::new();
        let a = tasks.create("a", vec![], false).unwrap().id;
        tasks.assign(a, worker_id.clone()).unwrap();
        let b = tasks
            .create(
                "b",
                vec![Dep::Edged {
                    id: a,
                    condition: Some(judge_cond("huh?")),
                }],
                false,
            )
            .unwrap()
            .id;
        tasks.assign(b, worker_id.clone()).unwrap();

        let mut root_rx = root_handle.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                self_addr,
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(crate::judge::FakeJudge::new(vec![None])),
            )
            .run(),
        );

        tasks.complete(a, None).unwrap();
        let content = next_message(&mut root_rx).await;
        assert!(
            content.contains(&format!("reset id:{b}")),
            "the escalation names b: {content}"
        );
        // Exactly ONE escalation — a further mutation does not re-escalate.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!has_pending(&mut root_rx), "the node escalates exactly once");
        assert_eq!(
            tasks.snapshot().into_iter().find(|t| t.id == b).unwrap().state,
            TaskState::Todo,
            "an unjudgeable node stays Todo"
        );
    }

    /// A13: a REFUSED `script` edge (a hook returning `Some(reason)`) escalates
    /// like an unparseable verdict and is NOT `Skipped` — the sibling of
    /// `a_blocked_script_fails_without_running`, but asserting `Todo` +
    /// `unjudgeable` + one escalation rather than `Failed`.
    #[tokio::test]
    async fn a_refused_script_edge_escalates_and_is_not_skipped() {
        let (registry, root, _worker, worker_id) = wiring();
        let (root_handle, self_addr) = escalate_edge(&registry, &root);
        let tasks = TaskList::new();
        let a = tasks.create("a", vec![], false).unwrap().id;
        tasks.assign(a, worker_id.clone()).unwrap();
        let b = tasks
            .create(
                "b",
                vec![Dep::Edged {
                    id: a,
                    condition: Some(script_cond("echo BLOCK")),
                }],
                false,
            )
            .unwrap()
            .id;
        tasks.assign(b, worker_id.clone()).unwrap();

        let mut root_rx = root_handle.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                self_addr,
                std::env::temp_dir(),
                HooksSet::one(Arc::new(BlockMarker)),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );

        tasks.complete(a, None).unwrap();
        let content = next_message(&mut root_rx).await;
        assert!(
            content.contains(&format!("reset id:{b}")),
            "the escalation names b: {content}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!has_pending(&mut root_rx), "exactly one escalation");
        let bnode = tasks.snapshot().into_iter().find(|t| t.id == b).unwrap();
        assert_eq!(
            bnode.state,
            TaskState::Todo,
            "a refusal blocks and escalates — it is never Skipped"
        );
    }

    /// A13/A14: `unjudgeable` is CLEARED by a reset — `tasks.reset(id)` bumps
    /// `reset_epoch`; the next `prune_unjudgeable()` drops the entry, the node is
    /// back in `frontier`, and the `FakeJudge` is asked again (a second verdict is
    /// consumed). FAILS if the epoch is not bumped.
    #[tokio::test]
    async fn unjudgeable_is_cleared_by_a_reset() {
        let (registry, root, worker, worker_id) = wiring();
        let tasks = TaskList::new();
        let a = tasks.create("a", vec![], false).unwrap().id;
        tasks.assign(a, worker_id.clone()).unwrap();
        let b = tasks
            .create(
                "b",
                vec![Dep::Edged {
                    id: a,
                    condition: Some(judge_cond("again?")),
                }],
                false,
            )
            .unwrap()
            .id;
        tasks.assign(b, worker_id.clone()).unwrap();

        let judge = Arc::new(crate::judge::FakeJudge::new(vec![None, Some(true)]));
        let judge_dyn: Arc<dyn crate::judge::Judge> = judge.clone();
        let mut rx = worker.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
                judge_dyn,
            )
            .run(),
        );

        assert_eq!(next_message(&mut rx).await, format!("#{a} a"));
        tasks.complete(a, None).unwrap();
        // First verdict: None → b is unjudgeable, stays Todo, never dispatched.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            tasks.snapshot().into_iter().find(|t| t.id == b).unwrap().state,
            TaskState::Todo
        );
        assert_eq!(judge.seen().len(), 1, "one judge call so far");
        assert!(!has_pending(&mut rx), "b was not dispatched");

        // A reset bumps reset_epoch → the entry is pruned → a SECOND verdict is asked.
        tasks.reset(b).unwrap();
        assert_eq!(
            next_message(&mut rx).await,
            format!("#{b} b"),
            "b dispatches once the reset re-arms the judge"
        );
        assert_eq!(
            judge.seen().len(),
            2,
            "the reset re-armed the judge (reset_epoch bumped)"
        );
    }

    /// `frontier` includes a `Script` node with no owner, but not an owner-less
    /// `Session` node.
    #[test]
    fn frontier_includes_a_script_node_but_not_an_ownerless_session() {
        let tasks = TaskList::new();
        let script = tasks.create("build", vec![], false).unwrap().id;
        tasks
            .configure(
                script,
                RunSpec::Script {
                    command: "true".into(),
                    timeout_secs: None,
                },
                false,
            )
            .unwrap();
        let _orphan = tasks.create("orphan", vec![], false).unwrap();
        let scheduler = Scheduler::new(
            Registry::new(),
            SessionId::agent("orchestrator"),
            tasks.clone(),
            SessionId::agent("scheduler"),
            std::env::temp_dir(),
            HooksSet::default(),
            tokio_util::sync::CancellationToken::new(),
        Arc::new(crate::judge::FakeJudge::new(vec![])),
        );
        assert_eq!(scheduler.frontier(), vec![script]);
    }
    /// No false stall: an empty plan and an owner-less Todo plan emit nothing.
    #[tokio::test]
    async fn empty_and_ownerless_plans_do_not_stall() {
        let (registry, root, _worker, _worker_id) = wiring();
        let (root_handle, self_addr) = escalate_edge(&registry, &root);
        let mut root_rx = root_handle.subscribe();

        let empty = TaskList::new();
        tokio::spawn(
            Scheduler::new(
                registry.clone(),
                root.clone(),
                empty.clone(),
                self_addr.clone(),
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(!has_pending(&mut root_rx), "an empty plan does not escalate");

        let orphaned = TaskList::new();
        orphaned.create("orphan", vec![], false).unwrap();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                orphaned.clone(),
                self_addr,
                std::env::temp_dir(),
                HooksSet::default(),
                tokio_util::sync::CancellationToken::new(),
            Arc::new(crate::judge::FakeJudge::new(vec![])),
            )
            .run(),
        );
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(
            !has_pending(&mut root_rx),
            "an owner-less Todo plan does not escalate"
        );
    }
}
