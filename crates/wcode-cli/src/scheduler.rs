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

use wcode_harness::protocol::{Request, SessionId};
use wcode_protocol::Registry;

use crate::tasks::{Task, TaskList, TaskState};

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
}

impl Scheduler {
    /// Build a scheduler over the root's shared plan and registry.
    pub fn new(
        registry: Registry,
        root: SessionId,
        tasks: TaskList,
        self_addr: SessionId,
    ) -> Self {
        Self {
            registry,
            root,
            tasks,
            self_addr,
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
    pub async fn run(self) {
        let mut updates = self.tasks.subscribe();
        // The Failed set last escalated (empty = nothing outstanding).
        let mut last_escalated: Vec<u32> = Vec::new();
        loop {
            let _ = updates.borrow_and_update();
            self.dispatch(&self.frontier());
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
            if updates.changed().await.is_err() {
                break;
            }
        }
    }

    /// The ids to dispatch now: [`TaskList::ready_ids`] (`Todo` ∧ every dep
    /// `Done`), kept only where the node has an `owner`. An owner-less node is
    /// never dispatched — it is the root's/human's to drive. Creation order.
    ///
    /// Ownership is enforced again downstream: [`Registry::deliver`] → `resolve`
    /// checks the permitted set, so a node assigned to a `[peers]` alias or an
    /// unregistered id yields `NotPermitted`/`Unknown` and never dispatches —
    /// acceptable for P1, since the scheduler drives the root's own `[team]`,
    /// each member `register`ed and `set_owner`ed at spawn.
    fn frontier(&self) -> Vec<u32> {
        let snapshot = self.tasks.snapshot();
        self.tasks
            .ready_ids()
            .into_iter()
            .filter(|id| snapshot.iter().any(|t| t.id == *id && t.owner.is_some()))
            .collect()
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
    /// Wake each frontier node: hydrate a [`Request::Wake`], [`Registry::deliver`]
    /// it from the root, and — only on `Ok` — flip the node `Doing` via
    /// [`TaskList::start`].
    ///
    /// A `deliver` error (`Unknown` / `NotPermitted` / `Closed`) does **not**
    /// `start` the node: it stays `Todo`, so the next `TaskList` mutation retries
    /// it (no tight spin — a failed delivery publishes nothing). P1 prints
    /// nothing on failure: it holds no UI channel, and `eprintln!` under the TUI
    /// alt-screen corrupts the display; stall notice + escalation are P2.
    ///
    /// `run` is ignored here (P1): dispatch sends an unconditional
    /// `Request::Wake`. P3 branches on `RunSpec::Script` (§5 — the exit code is
    /// the verdict); `Script` nodes are unreachable at runtime until P5.
    ///
    /// The snapshot is recomputed here rather than trusting the caller's
    /// frontier; [`TaskList::start`] ignores unknown ids, so drift between the
    /// two reads is harmless in this single-threaded loop.
    fn dispatch(&self, frontier: &[u32]) {
        let snapshot = self.tasks.snapshot();
        for &id in frontier {
            let Some(task) = snapshot.iter().find(|t| t.id == id) else {
                continue;
            };
            let Some(owner) = task.owner.as_ref() else {
                continue;
            };
            let deps: Vec<Task> = task
                .deps
                .iter()
                .filter_map(|d| snapshot.iter().find(|t| t.id == *d).cloned())
                .collect();
            let content = dispatch_content(task, &deps);
            if self
                .registry
                .deliver(&self.root, owner, Request::Wake { content })
                .is_ok()
            {
                let _ = self.tasks.start(id);
            }
        }
    }
}

/// The escalation `Wake` content (P2 §6). Exact shape:
///
/// ```text
/// scheduler: <N> node(s) hit the rework cap — the plan is stalled.
/// #<id> <title> — failed after <max_attempts> reworks; reset it (task op: reset id:<id>) or accept it.
/// ```
///
/// The number of reworks is `max_attempts` (the cap), NOT `task.attempts`
/// (which is cap+1 once the cap is hit).
fn escalation_content(failed: &[Task], max_attempts: u32) -> String {
    let mut out = format!(
        "scheduler: {} node(s) hit the rework cap — the plan is stalled.",
        failed.len()
    );
    for t in failed {
        out.push_str(&format!(
            "\n#{} {} — failed after {} reworks; reset it (task op: reset id:{}) or accept it.",
            t.id, t.title, max_attempts, t.id
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

    use wcode_harness::actor::{SessionActor, SessionHandle};
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::AgentEvent;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    use super::*;
    use crate::tasks::{RunSpec, TaskState};

    /// A task record carrying just the fields the content builder reads.
    fn node(id: u32, title: &str) -> Task {
        Task {
            id,
            title: title.into(),
            owner: None,
            state: TaskState::Todo,
            deps: Vec::new(),
            attempts: 0,
            feedback: None,
            artifact: None,
            gate: false,
            run: RunSpec::Session { member: None },
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
            "scheduler: 1 node(s) hit the rework cap — the plan is stalled.\n\
             #2 implement — failed after 3 reworks; reset it (task op: reset id:2) or accept it."
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
        let gate = tasks.create("verify", vec![work.id], true).unwrap();
        tasks.assign(work.id, worker_id.clone()).unwrap();
        tasks.assign(gate.id, worker_id.clone()).unwrap();

        let mut rx = worker.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
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
        let gate = tasks.create("verify", vec![work.id], true).unwrap();
        tasks.assign(work.id, worker_id.clone()).unwrap();
        tasks.assign(gate.id, worker_id.clone()).unwrap();

        let mut rx = worker.subscribe();
        tokio::spawn(
            Scheduler::new(
                registry,
                root.clone(),
                tasks.clone(),
                SessionId::agent("scheduler"),
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
        let gate = tasks.create("verify", vec![work.id], true).unwrap();
        tasks.assign(work.id, worker_id.clone()).unwrap();
        tasks.assign(gate.id, worker_id.clone()).unwrap();

        let mut root_rx = root_handle.subscribe();
        tokio::spawn(Scheduler::new(registry, root.clone(), tasks.clone(), self_addr).run());

        for _ in 0..4 {
            tasks.reject(gate.id, "nope").unwrap();
        }
        assert_eq!(tasks.snapshot()[0].state, TaskState::Failed);

        let content = next_message(&mut root_rx).await;
        assert!(content.contains("hit the rework cap"), "{content}");
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
            )
            .run(),
        );
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(!has_pending(&mut root_rx), "an empty plan does not escalate");

        let orphaned = TaskList::new();
        orphaned.create("orphan", vec![], false).unwrap();
        tokio::spawn(Scheduler::new(registry, root.clone(), orphaned.clone(), self_addr).run());
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(
            !has_pending(&mut root_rx),
            "an owner-less Todo plan does not escalate"
        );
    }
}
