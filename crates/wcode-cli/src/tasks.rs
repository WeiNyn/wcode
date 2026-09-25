//! The shared task list: the orchestrator's plan (C2). Root-owned; only the
//! root's `task` tool (crate::tools::task) mutates it, so only the root plans —
//! a worker keeps its report-back (see `crate::agents`).
//!
//! Shape mirrors [`crate::agents::Phonebook`]: an `Arc<Inner>` behind a
//! `#[derive(Clone)]` handle, so every clone — the root's `task` tool, the TUI
//! forwarder, the `Orchestrator` — shares one list. Live updates ride a
//! `tokio::sync::watch` channel, exactly as `wcode_protocol::Registry`'s roster
//! does: `send_replace` keeps the value even with no subscriber yet, so a
//! `subscribe()` that starts later still seeds.
//!
//! The canonical `TaskList` lives CLI-side, like `Phonebook` — NOT in
//! `wcode-protocol`, which is transport. The TUI cannot depend on this crate, so
//! the injected **view** type (`wcode_tui::TaskItem`) is defined TUI-side; the
//! CLI builds it (main.rs).
//!
//! **The DAG (P0).** A task was a flat list entry; it is now a node in an
//! acyclic graph the root owns: `deps` are the blocked-by edges, `attempts` /
//! `feedback` / `artifact` are the run memory, and `gate` / `run` say how it
//! runs and whether its `reject` re-opens its deps rather than itself (§3).
//! `ready`/`blocked` are *derived* (never stored). The static graph is acyclic
//! — a "go back" is the runtime control action `reopen`, which resets a node and
//! its whole downstream cone, bounded by the attempt cap (§6).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::watch;
use wcode_harness::protocol::SessionId;

/// A task's lifecycle state.
///
/// Four values. `Failed` is **terminal**: the rework cap was hit, so `reopen`
/// set it instead of looping (§6). P0 never resets it — a dependent seeing a
/// `Failed` (not `Done`) dep stays blocked, deliberately (§3: there is no
/// `Stale` state; stall detection + escalation + a reset land in P2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// Planned, not started.
    Todo,
    /// A worker is on it.
    Doing,
    /// Finished.
    Done,
    /// Terminal: the rework cap was hit, so `reopen` set `Failed` instead of
    /// looping. The scheduler never dispatches it again; a dependent seeing a
    /// `Failed` (not `Done`) dep stays blocked.
    Failed,
}

impl TaskState {
    /// The lowercase label shown by the TUI / `task list` (`todo` / `doing` /
    /// `done` / `failed`).
    pub fn label(self) -> &'static str {
        match self {
            TaskState::Todo => "todo",
            TaskState::Doing => "doing",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
        }
    }
}

/// The outcome of a [`TaskList::reopen`] — so a caller can journal `reopened`
/// vs. `failed` per node (§7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReopenOutcome {
    /// The node (and its cone) was reset to `Todo` for another attempt.
    Reopened,
    /// The attempt cap was hit: the node is `Failed` (terminal), the cone
    /// untouched.
    ReachedCap,
}

/// How a node runs (§3): a model/team node, or a physical (script) node.
///
/// The derives are required, not cosmetic: `run: RunSpec` is a `Task` field, so
/// `Task`'s own `#[derive(Clone, Debug, PartialEq, Eq)]` forces the same here.
///
/// P5-deferred: inline worker overrides (model / role / tools / read_only /
/// effort / provider) reuse the `WorkerSpec` fields (`agents.rs`). They are
/// deliberately NOT modelled yet — `Session` carries only `member` for now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunSpec {
    /// A model/team node: a spawned worker session produces the outcome.
    /// `member` is the `Phonebook`/`[team]` name (`agents.rs`); `None` = the
    /// root itself / a role-less node (validated at dispatch, P1).
    Session { member: Option<String> },
    /// A physical node: a definable action; the exit code IS the verdict (§5, P3).
    // P3 (physical gates) constructs this; unused in P0.
    #[allow(dead_code)]
    Script { command: String },
}

/// One planned task — a node in the root's DAG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// Monotonic id (`1, 2, 3, …`), assigned by [`TaskList::create`].
    pub id: u32,
    /// The one-line description the orchestrator planned.
    pub title: String,
    /// The worker this task is assigned to, if any (`SessionId`, e.g. `agent:w1`).
    pub owner: Option<SessionId>,
    /// Current state (a new task starts [`TaskState::Todo`]).
    pub state: TaskState,
    /// Blocked-by edge set: run only after every id here is `Done`. Empty = no
    /// deps (ready at once). The static graph is acyclic (see [`TaskList::depends`]).
    pub deps: Vec<u32>,
    /// Rework rounds; `reopen` bumps it, the cap compares it (§6).
    pub attempts: u32,
    /// The latest reject reason — the next run's extra input (§4 hydration).
    pub feedback: Option<String>,
    /// This node's output. Hydrates dependents; dropped on `reopen` (§4/§6).
    pub artifact: Option<String>,
    /// A gate (judgment or physical): its `reject` re-opens `deps`, not itself
    /// (§5).
    pub gate: bool,
    /// How the node runs (Session vs. Script, §3).
    pub run: RunSpec,
}

/// The shared, root-owned task list. Cloneable; every clone sees the same tasks.
///
/// Cheap to clone (`Arc`). Mutation is serialized by the `Mutex`; each successful
/// mutation republishes a snapshot on the `watch` channel, so a subscriber (the
/// TUI's forwarder) sees live updates without polling.
#[derive(Clone, Default)]
pub struct TaskList {
    inner: Arc<Inner>,
}

struct Inner {
    tasks: Mutex<Vec<Task>>,
    /// The next id to hand out (`fetch_add`); starts at 1.
    next_id: AtomicU32,
    /// The published snapshot. A `watch` (not a `broadcast`) so a late subscriber
    /// is *seeded* with the current list — it needs the whole state, not deltas.
    updates: watch::Sender<Vec<Task>>,
    /// The rework cap: `reopen` refuses at `attempts > max_attempts` → `Failed`.
    /// Fixed at 3 for now (§11.6); `[workflow] max_attempts` (P5) will plumb it.
    max_attempts: u32,
}

impl Default for Inner {
    /// Hand-written (not derived): `watch::Sender` has no `Default`, and the id
    /// source must start at 1 (not 0). `max_attempts` is set explicitly — a
    /// derived `Default` would cap every node at 0.
    fn default() -> Self {
        Self {
            tasks: Mutex::new(Vec::new()),
            next_id: AtomicU32::new(1),
            updates: watch::channel(Vec::new()).0,
            max_attempts: 3,
        }
    }
}

/// The transitive downstream cone of `id`: every node that reaches `id` through
/// `deps` (directly or transitively), excluding `id` itself; sorted ascending.
/// A free function so `depends` can run it while holding the `tasks` lock.
fn cone(tasks: &[Task], id: u32) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let mut frontier = vec![id];
    while let Some(cur) = frontier.pop() {
        for t in tasks {
            if t.id != id && t.deps.contains(&cur) && !out.contains(&t.id) {
                out.push(t.id);
                frontier.push(t.id);
            }
        }
    }
    out.sort_unstable();
    out
}

impl TaskList {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// The rework cap: `reopen` refuses beyond it → `Failed` (§6).
    pub fn max_attempts(&self) -> u32 {
        self.inner.max_attempts
    }

    /// Append a `Todo` node with the next id and deps `deps` (every id must
    /// already exist), publish, and return a copy of it.
    ///
    /// Err `"no task #<d>"` for the first dep id that names no task. Self-dep
    /// and cycles are impossible here: the id is fresh (strictly greater than
    /// every existing one), so existence is the only check. Initialises
    /// attempts=0, feedback/artifact=None, gate=false, run=Session{member:None};
    /// a gate or `Script` node is set afterwards via [`TaskList::configure`].
    pub fn create(&self, title: impl Into<String>, deps: Vec<u32>) -> Result<Task, String> {
        let task = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            for &d in &deps {
                if !tasks.iter().any(|t| t.id == d) {
                    return Err(format!("no task #{d}"));
                }
            }
            let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
            let task = Task {
                id,
                title: title.into(),
                owner: None,
                state: TaskState::Todo,
                deps,
                attempts: 0,
                feedback: None,
                artifact: None,
                gate: false,
                run: RunSpec::Session { member: None },
            };
            tasks.push(task.clone());
            task
        };
        self.publish();
        Ok(task)
    }

    /// Assign `id` to a worker and mark it [`TaskState::Doing`] (the three-state
    /// machine: an assigned task is being worked). `Err` names an unknown id.
    pub fn assign(&self, id: u32, owner: SessionId) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            task.owner = Some(owner);
            task.state = TaskState::Doing;
        }
        self.publish();
        Ok(())
    }

    /// Add the edge `id` depends-on `on` (`id` runs after `on`).
    ///
    /// Err `"no task #<id>"` / `"no task #<on>"` (unknown); `"task #<id> cannot
    /// depend on itself"`; `"task #<id> already depends on #<on>"` (duplicate);
    /// or `"dependency cycle: #<id> → … → #<on>"` when `on` already
    /// (transitively) depends on `id` — the static graph stays a DAG (§2/§11.2).
    /// Publishes on Ok; an `Err` leaves the list untouched.
    pub fn depends(&self, id: u32, on: u32) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            if !tasks.iter().any(|t| t.id == id) {
                return Err(format!("no task #{id}"));
            }
            if !tasks.iter().any(|t| t.id == on) {
                return Err(format!("no task #{on}"));
            }
            if id == on {
                return Err(format!("task #{id} cannot depend on itself"));
            }
            if tasks
                .iter()
                .find(|t| t.id == id)
                .is_some_and(|t| t.deps.contains(&on))
            {
                return Err(format!("task #{id} already depends on #{on}"));
            }
            // A cycle iff `on` is already downstream of `id` (on depends on id):
            // adding `id → on` would close a loop.
            if cone(&tasks, id).contains(&on) {
                return Err(format!("dependency cycle: #{id} → … → #{on}"));
            }
            tasks
                .iter_mut()
                .find(|t| t.id == id)
                .expect("id checked to exist above")
                .deps
                .push(on);
        }
        self.publish();
        Ok(())
    }

    /// Configure how `id` runs and whether it is a gate (§3/§5). The seam P5's
    /// config uses to build gate / `Script` nodes; `create` alone always makes a
    /// dep-less, gate-less `Session` node. `Err` names an unknown id. Publishes
    /// on Ok.
    // The P5 config seam (build gate / `Script` nodes); only tests call it in P0.
    #[allow(dead_code)]
    pub fn configure(&self, id: u32, run: RunSpec, gate: bool) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            task.run = run;
            task.gate = gate;
        }
        self.publish();
        Ok(())
    }

    /// Flip `id` to [`TaskState::Doing`] (the scheduler calls this right after
    /// dispatch, §4).
    ///
    /// Err `"no task #<id>"`. Contrast [`TaskList::assign`], which also records
    /// the owner; `start` is the owner-less "work has begun" move.
    // The P1 scheduler's owner-less dispatch move; unused in P0.
    #[allow(dead_code)]
    pub fn start(&self, id: u32) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            task.state = TaskState::Doing;
        }
        self.publish();
        Ok(())
    }

    /// Flip `id` to [`TaskState::Done`], storing `artifact` — a model/external
    /// acceptance (§4).
    ///
    /// Guard: refuses while blocked (some dep not `Done`) so an out-of-order
    /// complete cannot corrupt the DAG. Err `"no task #<id>"` or
    /// `"#<id> is blocked by #<dep>"` (first non-Done dep). Publishes on Ok.
    pub fn complete(&self, id: u32, artifact: Option<String>) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let idx = tasks
                .iter()
                .position(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            let deps = tasks[idx].deps.clone();
            for d in &deps {
                let done = tasks
                    .iter()
                    .find(|t| t.id == *d)
                    .is_some_and(|t| t.state == TaskState::Done);
                if !done {
                    return Err(format!("#{id} is blocked by #{d}"));
                }
            }
            tasks[idx].state = TaskState::Done;
            tasks[idx].artifact = artifact;
        }
        self.publish();
        Ok(())
    }

    /// Rework: reset `id` and its whole downstream cone back to `Todo`.
    ///
    /// Sets `id.attempts += 1`, `id.feedback = Some(reason)`, `id.state = Todo`,
    /// `id.artifact = None`; then resets every [`TaskList::descendants`] of `id`
    /// to `Todo` with `artifact = None` (transitive invalidation, §6 — NOT just
    /// the predecessor; `blocked`/`ready` then recompute for free, §3).
    ///
    /// Cap (§6/§11.6): if `attempts` exceeds [`TaskList::max_attempts`], do NOT
    /// reopen — set `id = Failed` (terminal), leave the cone untouched, and
    /// return [`ReopenOutcome::ReachedCap`] (the scheduler escalates, P2). Err
    /// `"no task #<id>"`. Publishes on Ok.
    pub fn reopen(&self, id: u32, reason: impl Into<String>) -> Result<ReopenOutcome, String> {
        // The downstream cone is read before taking the lock (`descendants`
        // acquires it); the root owns the list single-threaded, so nothing
        // mutates the graph between the read and the reset below.
        let cone_ids = self.descendants(id);
        let outcome;
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let idx = tasks
                .iter()
                .position(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            tasks[idx].attempts += 1;
            if tasks[idx].attempts > self.max_attempts() {
                tasks[idx].state = TaskState::Failed;
                outcome = ReopenOutcome::ReachedCap;
            } else {
                tasks[idx].feedback = Some(reason.into());
                tasks[idx].state = TaskState::Todo;
                tasks[idx].artifact = None;
                for cid in cone_ids {
                    if let Some(t) = tasks.iter_mut().find(|t| t.id == cid) {
                        t.state = TaskState::Todo;
                        t.artifact = None;
                    }
                }
                outcome = ReopenOutcome::Reopened;
            }
        }
        self.publish();
        Ok(outcome)
    }

    /// A gate's rejection (§5): `gate_id` must be a gate; re-open each of its
    /// `deps` (via [`TaskList::reopen`]) and return `(dep_id, outcome)` per dep
    /// — so the caller can journal `reopen` vs. `fail` (§7). A dep that hit the
    /// cap becomes `Failed` with [`ReopenOutcome::ReachedCap`]. The gate itself
    /// is in the cone (it depends on a reopened dep), so it resets too (§6). Err
    /// `"no task #<gate_id>"` or `"task #<gate_id> is not a gate"`. Publishes.
    pub fn reject(
        &self,
        gate_id: u32,
        reason: impl Into<String>,
    ) -> Result<Vec<(u32, ReopenOutcome)>, String> {
        let deps = {
            let tasks = self.inner.tasks.lock().unwrap();
            let gate = tasks
                .iter()
                .find(|t| t.id == gate_id)
                .ok_or_else(|| format!("no task #{gate_id}"))?;
            if !gate.gate {
                return Err(format!("task #{gate_id} is not a gate"));
            }
            gate.deps.clone()
        };
        let reason = reason.into();
        let mut reopened = Vec::with_capacity(deps.len());
        for dep in deps {
            let outcome = self.reopen(dep, reason.clone())?;
            reopened.push((dep, outcome));
        }
        Ok(reopened)
    }

    /// The ready frontier, in creation order: every `Todo` node all of whose
    /// deps are `Done` (derived, never stored, §3). Read-only — no publish.
    // The P1 scheduler's frontier query; unused in P0.
    #[allow(dead_code)]
    pub fn ready_ids(&self) -> Vec<u32> {
        let tasks = self.inner.tasks.lock().unwrap();
        tasks
            .iter()
            .filter(|t| {
                t.state == TaskState::Todo
                    && t.deps.iter().all(|d| {
                        tasks
                            .iter()
                            .find(|x| x.id == *d)
                            .is_some_and(|x| x.state == TaskState::Done)
                    })
            })
            .map(|t| t.id)
            .collect()
    }

    /// The transitive downstream cone of `id` (every node reaching `id` through
    /// `deps`), excluding `id`. Used by `reopen`. No publish.
    pub fn descendants(&self, id: u32) -> Vec<u32> {
        cone(&self.inner.tasks.lock().unwrap(), id)
    }

    /// A point-in-time copy, in creation (id) order — what the TUI renders.
    pub fn snapshot(&self) -> Vec<Task> {
        self.inner.tasks.lock().unwrap().clone()
    }

    /// A receiver **seeded** with the current tasks and updated on every mutation
    /// (a `watch`, like `Registry::subscribe`).
    pub fn subscribe(&self) -> watch::Receiver<Vec<Task>> {
        self.inner.updates.subscribe()
    }

    /// Recompute the snapshot and publish it to every subscriber. Uses
    /// `send_replace` so the value survives with no receiver yet (mirrors
    /// `Registry`'s roster channel).
    fn publish(&self) {
        self.inner.updates.send_replace(self.snapshot());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `create` hands out strictly increasing ids, starting at 1.
    #[test]
    fn create_returns_increasing_ids() {
        let list = TaskList::new();
        let a = list.create("first", vec![]).unwrap();
        let b = list.create("second", vec![]).unwrap();
        assert_eq!(a.id, 1, "ids start at 1");
        assert_eq!(b.id, 2);
        assert_eq!(a.state, TaskState::Todo);
        assert_eq!(a.owner, None);
        assert_eq!(list.snapshot().len(), 2);
    }

    /// `create` with an unknown dep id is an error and creates nothing.
    #[test]
    fn create_rejects_an_unknown_dep() {
        let list = TaskList::new();
        let err = list.create("t", vec![7]).unwrap_err();
        assert!(err.contains('7'), "names the bad dep: {err}");
        assert!(list.snapshot().is_empty(), "nothing is created");
    }

    /// `assign` records the owner and moves the task to `Doing`; an unknown id
    /// is an error (and leaves the list untouched).
    #[test]
    fn assign_sets_the_owner() {
        let list = TaskList::new();
        let t = list.create("first", vec![]).unwrap();
        list.assign(t.id, SessionId::agent("w1")).unwrap();

        let got = &list.snapshot()[0];
        assert_eq!(got.owner, Some(SessionId::agent("w1")));
        assert_eq!(got.state, TaskState::Doing, "an assigned task is doing");

        let err = list.assign(99, SessionId::agent("w2")).unwrap_err();
        assert!(err.contains("99"), "names the unknown id: {err}");
        assert_eq!(list.snapshot().len(), 1, "the list is unchanged");
    }

    /// `complete` flips a task to `Done`; an unknown id is an error.
    #[test]
    fn complete_flips_state() {
        let list = TaskList::new();
        let t = list.create("first", vec![]).unwrap();
        list.complete(t.id, None).unwrap();
        assert_eq!(list.snapshot()[0].state, TaskState::Done);

        let err = list.complete(99, None).unwrap_err();
        assert!(err.contains("99"), "names the unknown id: {err}");

        assert_eq!(TaskState::Todo.label(), "todo");
        assert_eq!(TaskState::Doing.label(), "doing");
        assert_eq!(TaskState::Done.label(), "done");
        assert_eq!(TaskState::Failed.label(), "failed");
    }

    /// `complete` stores the artifact and refuses while a dep is not `Done`.
    #[test]
    fn complete_stores_the_artifact_and_respects_blocking() {
        let list = TaskList::new();
        let a = list.create("a", vec![]).unwrap();
        let b = list.create("b", vec![a.id]).unwrap();

        // b is blocked by a.
        let err = list.complete(b.id, None).unwrap_err();
        assert_eq!(err, format!("#{} is blocked by #{}", b.id, a.id));

        list.complete(a.id, Some("A₁".into())).unwrap();
        list.complete(b.id, Some("A₂".into())).unwrap();
        let snap = list.snapshot();
        assert_eq!(snap[0].artifact.as_deref(), Some("A₁"));
        assert_eq!(snap[1].artifact.as_deref(), Some("A₂"));
    }

    /// A `subscribe()` starts seeded and sees the new list after a mutation.
    #[tokio::test]
    async fn subscribe_sees_an_update() {
        let list = TaskList::new();
        list.create("first", vec![]).unwrap();

        let mut rx = list.subscribe();
        // Seeded with the current list, no publish needed.
        assert_eq!(rx.borrow().len(), 1);
        assert_eq!(rx.borrow()[0].title, "first");

        // A post-subscribe create publishes a fresh snapshot.
        list.create("second", vec![]).unwrap();
        rx.changed().await.expect("open");
        let seen = rx.borrow_and_update();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].title, "second");
    }

    /// `depends` adds an edge; unknown ids, self-edges and duplicates are
    /// rejected; and a cycle is rejected — the graph stays a DAG (§2/§11.2).
    #[test]
    fn depends_rejects_a_cycle() {
        let list = TaskList::new();
        let a = list.create("a", vec![]).unwrap();
        let b = list.create("b", vec![a.id]).unwrap();

        // b already depends on a: closing a→b would make the cycle a→b→a.
        let err = list.depends(a.id, b.id).unwrap_err();
        assert!(err.contains("cycle"), "names the cycle: {err}");
        assert!(
            err.contains(&a.id.to_string()) && err.contains(&b.id.to_string()),
            "the error names both ids: {err}"
        );
        assert_eq!(
            list.snapshot()[0].deps,
            Vec::<u32>::new(),
            "a's deps are unchanged"
        );

        // Self, unknown, and duplicate edges are errors too.
        assert!(list.depends(a.id, a.id).unwrap_err().contains("itself"));
        assert!(list.depends(a.id, 99).unwrap_err().contains("99"));
        assert!(list.depends(b.id, a.id).unwrap_err().contains("already"));
    }

    /// `ready_ids` is a `Todo` node all of whose deps are `Done`, in creation
    /// order (derived, never stored).
    #[test]
    fn ready_ids_tracks_the_frontier() {
        let list = TaskList::new();
        let a = list.create("a", vec![]).unwrap();
        let b = list.create("b", vec![a.id]).unwrap();

        assert_eq!(list.ready_ids(), vec![a.id], "only the root is ready");
        list.complete(a.id, None).unwrap();
        assert_eq!(list.ready_ids(), vec![b.id], "b unblocks when a is done");
        list.start(b.id).unwrap();
        assert!(list.ready_ids().is_empty(), "a doing node is not ready");
    }

    /// `reopen` resets the transitive cone. Chain 1→2→3, all `Done`; reopen 2
    /// ⇒ 2 and 3 drop to `Todo`; 1 (upstream, not a descendant) is untouched;
    /// 2's attempts bump to 1 and its feedback/artifact are set/cleared.
    #[test]
    fn reopen_resets_the_transitive_cone() {
        let list = TaskList::new();
        let t1 = list.create("one", vec![]).unwrap();
        let t2 = list.create("two", vec![t1.id]).unwrap();
        let t3 = list.create("three", vec![t2.id]).unwrap();
        list.complete(t1.id, Some("A₁".into())).unwrap();
        list.complete(t2.id, Some("A₂".into())).unwrap();
        list.complete(t3.id, Some("A₃".into())).unwrap();

        let outcome = list.reopen(t2.id, "fix").unwrap();
        assert_eq!(outcome, ReopenOutcome::Reopened);

        let snap = list.snapshot();
        // 1 is upstream, NOT a descendant: it stays Done.
        assert_eq!(snap[0].state, TaskState::Done, "upstream is untouched");
        assert_eq!(snap[0].artifact.as_deref(), Some("A₁"));
        // 2 is reset, with the rework memory set.
        assert_eq!(snap[1].state, TaskState::Todo);
        assert_eq!(snap[1].attempts, 1);
        assert_eq!(snap[1].feedback.as_deref(), Some("fix"));
        assert_eq!(snap[1].artifact, None);
        // 3 is in the cone: reset, artifact dropped.
        assert_eq!(snap[2].state, TaskState::Todo);
        assert_eq!(snap[2].artifact, None);

        assert_eq!(list.descendants(t2.id), vec![t3.id]);
    }

    /// `reopen` past the cap sets `Failed` (terminal) and leaves the cone alone.
    #[test]
    fn reopen_hits_the_cap_and_fails_terminal() {
        let list = TaskList::new();
        assert_eq!(list.max_attempts(), 3);
        let t = list.create("t", vec![]).unwrap();
        for _ in 0..3 {
            assert_eq!(list.reopen(t.id, "again").unwrap(), ReopenOutcome::Reopened);
        }
        // The 4th reopen exceeds the cap.
        assert_eq!(
            list.reopen(t.id, "again").unwrap(),
            ReopenOutcome::ReachedCap
        );
        assert_eq!(list.snapshot()[0].state, TaskState::Failed);
        assert_eq!(TaskState::Failed.label(), "failed");
    }

    /// A gate's `reject` re-opens its dep and the gate (in the cone); a
    /// non-gate's `reject` is an error.
    #[test]
    fn reject_reopens_the_gated_dep_and_the_cone() {
        let list = TaskList::new();
        let work = list.create("work", vec![]).unwrap();
        let gate = list.create("verify", vec![work.id]).unwrap();
        list.configure(
            gate.id,
            RunSpec::Session {
                member: Some("reviewer".into()),
            },
            true,
        )
        .unwrap();

        list.complete(work.id, Some("A₁".into())).unwrap();
        list.start(gate.id).unwrap();
        assert_eq!(list.snapshot()[1].state, TaskState::Doing);

        let reopened = list.reject(gate.id, "missing edge case X").unwrap();
        assert_eq!(reopened, vec![(work.id, ReopenOutcome::Reopened)]);

        let snap = list.snapshot();
        assert_eq!(snap[0].state, TaskState::Todo, "the dep is reopened");
        assert_eq!(snap[0].attempts, 1);
        assert_eq!(snap[0].feedback.as_deref(), Some("missing edge case X"));
        assert_eq!(snap[0].artifact, None);
        assert_eq!(snap[1].state, TaskState::Todo, "the gate is in the cone");

        // A non-gate `reject` is a clear error.
        let work2 = list.create("w2", vec![]).unwrap();
        let err = list.reject(work2.id, "nope").unwrap_err();
        assert!(err.contains("not a gate"), "{err}");
    }
}
