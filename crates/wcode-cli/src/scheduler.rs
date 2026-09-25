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

use crate::tasks::{Task, TaskList};

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
}

impl Scheduler {
    /// Build a scheduler over the root's shared plan and registry.
    pub fn new(registry: Registry, root: SessionId, tasks: TaskList) -> Self {
        Self {
            registry,
            root,
            tasks,
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
        loop {
            let _ = updates.borrow_and_update();
            self.dispatch(&self.frontier());
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
        let owned = tasks.create("implement", vec![]).unwrap();
        tasks.assign(owned.id, worker_id.clone()).unwrap();
        let orphan = tasks.create("no one", vec![]).unwrap();

        let expected = dispatch_content(&owned, &[]);
        let mut rx = worker.subscribe();
        let mut task_updates = tasks.subscribe();
        tokio::spawn(Scheduler::new(registry, root.clone(), tasks.clone()).run());

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
        let node = tasks.create("t", vec![]).unwrap();
        tasks.assign(node.id, worker_id.clone()).unwrap();

        let mut rx = worker.subscribe();
        tokio::spawn(Scheduler::new(registry, root.clone(), tasks.clone()).run());

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            tasks.snapshot()[0].state,
            TaskState::Todo,
            "an undeliverable node stays Todo"
        );
        assert!(rx.try_recv().is_err(), "no delivery to an unowned node");
    }
}
