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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use tokio::sync::watch;
use wcode_harness::protocol::SessionId;

/// A task's lifecycle state.
///
/// Four values. `Failed` is **terminal**: the rework cap was hit, so `reopen`
/// set it instead of looping (§6). P0 never resets it — a dependent seeing a
/// `Failed` (not `Done`) dep stays blocked, deliberately (§3: there is no
/// `Stale` state; stall detection + escalation + a reset land in P2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunSpec {
    /// A model/team node: a spawned worker session produces the outcome.
    /// `member` is the `Phonebook`/`[team]` name (`agents.rs`); `None` = the
    /// root itself / a role-less node (validated at dispatch, P1).
    Session {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member: Option<String>,
    },
    /// A physical node: a definable action; the exit code IS the verdict (§5, P3).
    Script { command: String },
}

/// One planned task — a node in the root's DAG.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Monotonic id (`1, 2, 3, …`), assigned by [`TaskList::create`].
    pub id: u32,
    /// The one-line description the orchestrator planned.
    pub title: String,
    /// The worker this task is assigned to, if any (`SessionId`, e.g. `agent:w1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<SessionId>,
    /// Current state (a new task starts [`TaskState::Todo`]).
    pub state: TaskState,
    /// Blocked-by edge set: run only after every id here is `Done`. Empty = no
    /// deps (ready at once). The static graph is acyclic (see [`TaskList::depends`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<u32>,
    /// Rework rounds; `reopen` bumps it, the cap compares it (§6).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempts: u32,
    /// The latest reject reason — the next run's extra input (§4 hydration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// This node's output. Hydrates dependents; dropped on `reopen` (§4/§6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// A gate (judgment or physical): its `reject` re-opens `deps`, not itself
    /// (§5).
    #[serde(default, skip_serializing_if = "is_false")]
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

/// Local `skip_serializing_if` helpers (the private `session_groups::is_false`
/// is not visible here).
fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// One journaled line per PUBLIC mutation (§7). Tagged like `SessionEntry`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum PlanOp {
    Create {
        id: u32,
        title: String,
        deps: Vec<u32>,
        gate: bool,
    },
    Depends {
        id: u32,
        on: u32,
    },
    Assign {
        id: u32,
        owner: SessionId,
    },
    Configure {
        id: u32,
        run: RunSpec,
        gate: bool,
    },
    Start {
        id: u32,
    },
    Complete {
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<String>,
    },
    Reopen {
        id: u32,
        reason: String,
    },
    Reject {
        gate: u32,
        reason: String,
    },
    Reset {
        id: u32,
    },
    Fail {
        id: u32,
        reason: String,
    },
    Restart {
        id: u32,
    },
    Snapshot {
        tasks: Vec<Task>,
    },
    /// A future op this build does not understand; ignored on apply.
    #[serde(other)]
    Unknown,
}

/// Rewrite the journal as a single `Snapshot` once it exceeds this many lines (§7).
const COMPACT_AFTER: u64 = 512;

/// The append-only journal sink: append one op line at a time.
struct Journal {
    path: PathBuf,
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
    /// `Some` while live (`<groupdir>/plan.ndjson`); `None` in memory.
    journal: Option<Mutex<Journal>>,
    /// True while replaying, so `record` is a no-op (Ruling 2).
    replaying: AtomicBool,
    /// Ops since the last `Snapshot` (compaction trigger).
    lines: AtomicU64,
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
            journal: None,
            replaying: AtomicBool::new(false),
            lines: AtomicU64::new(0),
        }
    }
}

impl Inner {
    /// Append one op line (Ruling 1). No-op while replaying or un-journaled.
    fn record(&self, op: &PlanOp) -> Result<(), String> {
        if self.replaying.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(journal) = &self.journal else {
            return Ok(());
        };
        let line = serde_json::to_string(op).map_err(|e| format!("plan encode: {e}"))?;
        let journal = journal.lock().unwrap();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&journal.path)
            .map_err(|e| format!("plan journal {}: {e}", journal.path.display()))?;
        file.write_all(format!("{line}\n").as_bytes())
            .map_err(|e| format!("plan write: {e}"))?;
        file.flush().map_err(|e| format!("plan flush: {e}"))?;
        self.lines.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl Inner {
    /// Rewrite the journal as a single `Snapshot` once it exceeds `COMPACT_AFTER`.
    /// Atomic (temp + rename) under the journal lock; a crash mid-rewrite leaves
    /// the old file intact. No-op while replaying.
    fn maybe_compact(&self, tasks: &[Task]) {
        if self.replaying.load(Ordering::Relaxed) {
            return;
        }
        let Some(sink) = &self.journal else {
            return;
        };
        let guard = sink.lock().unwrap();
        if self.lines.load(Ordering::Relaxed) <= COMPACT_AFTER {
            return;
        }
        let Ok(line) = serde_json::to_string(&PlanOp::Snapshot { tasks: tasks.to_vec() }) else {
            return;
        };
        let tmp = crate::tools::temp_path(&guard.path);
        let written = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .and_then(|mut f| {
                f.write_all(format!("{line}\n").as_bytes())?;
                f.flush()
            });
        if written.is_ok() && std::fs::rename(&tmp, &guard.path).is_ok() {
            self.lines.store(1, Ordering::Relaxed);
        } else {
            let _ = std::fs::remove_file(&tmp);
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
    /// An empty list that journals every mutation to `path` (the parent dir is
    /// created; the file appears on the first mutation).
    pub fn with_journal(path: PathBuf) -> Self {
        Self::with_sink(Some(path), false)
    }

    /// A list with an optional sink and the `replaying` flag (Ruling 2). The flag
    /// — not `journal: None` — suppresses writes during replay; it is resettable.
    fn with_sink(journal: Option<PathBuf>, replaying: bool) -> Self {
        let journal = journal.map(|path| {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            Mutex::new(Journal { path })
        });
        Self {
            inner: Arc::new(Inner {
                tasks: Mutex::new(Vec::new()),
                next_id: AtomicU32::new(1),
                updates: watch::channel(Vec::new()).0,
                max_attempts: 3,
                journal,
                replaying: AtomicBool::new(replaying),
                lines: AtomicU64::new(0),
            }),
        }
    }

    /// Load `path`: replay each NDJSON `PlanOp` into a list built with the sink
    /// set but `replaying` ON (so `record` is suppressed — Ruling 2), then clear
    /// the flag and RECONCILE. A missing file ⇒ an empty journaled list. A parse
    /// error is tolerated only on the last non-empty line (a torn tail — mirrors
    /// `Session::open`).
    pub fn load(path: &Path) -> Result<Self, String> {
        let list = Self::with_sink(Some(path.to_path_buf()), true);
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                list.inner.replaying.store(false, Ordering::Relaxed);
                return Ok(list);
            }
            Err(e) => return Err(format!("plan {}: {e}", path.display())),
        };
        let lines: Vec<&str> = raw.lines().collect();
        let last_non_empty = lines.iter().rposition(|l| !l.trim().is_empty());
        let mut applied = 0u64;
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PlanOp>(line) {
                Ok(op) => {
                    list.apply(op);
                    applied += 1;
                }
                Err(e) => {
                    if Some(i) == last_non_empty {
                        break; // torn final write; rest is blank
                    }
                    return Err(format!("plan {} line {}: {e}", path.display(), i + 1));
                }
            }
        }
        list.inner.lines.store(applied, Ordering::Relaxed);
        list.inner.replaying.store(false, Ordering::Relaxed);
        list.reconcile();
        Ok(list)
    }

    /// Apply one replayed op. `Create`/`Snapshot` bypass the public methods so a
    /// replay keeps ids verbatim (Blocker A); everything else goes through them
    /// (writes are suppressed by `replaying`).
    fn apply(&self, op: PlanOp) {
        match op {
            PlanOp::Create {
                id,
                title,
                deps,
                gate,
            } => {
                {
                    let mut tasks = self.inner.tasks.lock().unwrap();
                    if !tasks.iter().any(|t| t.id == id) {
                        tasks.push(Task {
                            id,
                            title,
                            owner: None,
                            state: TaskState::Todo,
                            deps,
                            attempts: 0,
                            feedback: None,
                            artifact: None,
                            gate,
                            run: RunSpec::Session { member: None },
                        });
                    }
                }
                self.inner.next_id.fetch_max(id + 1, Ordering::Relaxed);
                self.publish();
            }
            PlanOp::Snapshot { tasks } => {
                let next = tasks.iter().map(|t| t.id).max().map_or(1, |m| m + 1);
                *self.inner.tasks.lock().unwrap() = tasks;
                self.inner.next_id.store(next, Ordering::Relaxed);
                self.publish();
            }
            PlanOp::Depends { id, on } => {
                let _ = self.depends(id, on);
            }
            PlanOp::Assign { id, owner } => {
                let _ = self.assign(id, owner);
            }
            PlanOp::Configure { id, run, gate } => {
                let _ = self.configure(id, run, gate);
            }
            PlanOp::Start { id } => {
                let _ = self.start(id);
            }
            PlanOp::Complete { id, artifact } => {
                let _ = self.complete(id, artifact);
            }
            PlanOp::Reopen { id, reason } => {
                let _ = self.reopen(id, reason);
            }
            PlanOp::Reject { gate, reason } => {
                let _ = self.reject(gate, reason);
            }
            PlanOp::Reset { id } => {
                let _ = self.reset(id);
            }
            PlanOp::Fail { id, reason } => {
                let _ = self.fail(id, reason);
            }
            PlanOp::Restart { id } => {
                {
                    let mut tasks = self.inner.tasks.lock().unwrap();
                    if let Some(t) = tasks.iter_mut().find(|t| t.id == id) {
                        t.state = TaskState::Todo;
                    }
                }
                self.publish();
            }
            PlanOp::Unknown => {}
        }
    }

    /// Crash reconcile (§7): every `Doing` → `Todo` WITHOUT bumping `attempts`
    /// (a crash is not the node's fault), journaling one `Restart{id}` per flip.
    /// MUST run before the scheduler subscribes.
    fn reconcile(&self) {
        let doing: Vec<u32> = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let mut ids = Vec::new();
            for t in tasks.iter_mut() {
                if t.state == TaskState::Doing {
                    t.state = TaskState::Todo;
                    ids.push(t.id);
                }
            }
            ids
        };
        for id in doing {
            let _ = self.inner.record(&PlanOp::Restart { id });
        }
        self.publish();
    }
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
    /// attempts=0, feedback/artifact=None, run=Session{member:None}, and `gate`
    /// set from the arg (a `Script` node is still set later via
    /// [`TaskList::configure`]).
    pub fn create(
        &self,
        title: impl Into<String>,
        deps: Vec<u32>,
        gate: bool,
    ) -> Result<Task, String> {
        let title = title.into();
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
                title: title.clone(),
                owner: None,
                state: TaskState::Todo,
                deps: deps.clone(),
                attempts: 0,
                feedback: None,
                artifact: None,
                gate,
                run: RunSpec::Session { member: None },
            };
            self.inner.record(&PlanOp::Create {
                id,
                title,
                deps,
                gate,
            })?;
            tasks.push(task.clone());
            task
        };
        self.publish();
        Ok(task)
    }

    /// Assign `id` to a worker, recording the **owner only** — `state` is left
    /// untouched. The `Doing` transition belongs to [`TaskList::start`], which
    /// the scheduler calls on dispatch (§4): an assigned node must stay `Todo`
    /// so it remains in the scheduler's ready frontier. `Err` names an unknown
    /// id.
    pub fn assign(&self, id: u32, owner: SessionId) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            self.inner.record(&PlanOp::Assign {
                id,
                owner: owner.clone(),
            })?;
            task.owner = Some(owner);
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
            self.inner.record(&PlanOp::Depends { id, on })?;
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
    // `create` now carries `gate` itself (P2), so this is no longer the dynamic
    // gate path — it stays the **test / P5 config** seam for the `run` axis
    // (`RunSpec::Script`, P3) and an explicit post-hoc reconfigure.
    pub fn configure(&self, id: u32, run: RunSpec, gate: bool) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            self.inner
                .record(&PlanOp::Configure {
                    id,
                    run: run.clone(),
                    gate,
                })?;
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
    // Called by the scheduler's `dispatch` on a successful deliver (P1).
    pub fn start(&self, id: u32) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            self.inner.record(&PlanOp::Start { id })?;
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
            self.inner.record(&PlanOp::Complete {
                id,
                artifact: artifact.clone(),
            })?;
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
    fn reopen_inner(&self, id: u32, reason: &str) -> Result<ReopenOutcome, String> {
        // The downstream cone is read before taking the lock (`descendants`
        // acquires it); the root owns the list single-threaded.
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
                tasks[idx].feedback = Some(reason.to_string());
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

    /// Journal one `Reopen`, then delegate to [`TaskList::reopen_inner`]. A cap
    /// `Reopen` is journaled too — replay recomputes the `Failed` state.
    // The public rework seam: applied on replay (and by tests / a future op).
    pub fn reopen(&self, id: u32, reason: impl Into<String>) -> Result<ReopenOutcome, String> {
        let reason = reason.into();
        self.inner.record(&PlanOp::Reopen {
            id,
            reason: reason.clone(),
        })?;
        self.reopen_inner(id, &reason)
    }

    /// Manually clear a terminal `Failed` node so it can run again — the
    /// human/model escape hatch for a cap hit that should NOT end the plan (§6).
    ///
    /// Resets `id` ALONE: `state = Todo`, `attempts = 0`, `feedback = None`,
    /// `artifact = None`. The downstream cone is left untouched — a capped node
    /// never invalidated it (`reopen`'s cap branch); `reject`'s gate fix is what
    /// makes the gate re-runnable. `reset` on a non-`Failed` node is allowed (it
    /// just clears the counters). Err `"no task #<id>"`. Publishes on Ok.
    pub fn reset(&self, id: u32) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            self.inner.record(&PlanOp::Reset { id })?;
            task.state = TaskState::Todo;
            task.attempts = 0;
            task.feedback = None;
            task.artifact = None;
        }
        self.publish();
        Ok(())
    }

    /// Fail `id` terminally with `reason` — a deterministic verdict (a `Script`'s
    /// non-zero exit, §5), NOT a rework. Sets `state = Failed` and
    /// `feedback = Some(reason)` (so `task list` / the escalation show why);
    /// `attempts` is untouched and the cone is untouched (a failed node never
    /// invalidates dependents — they stay blocked, §3). Err `"no task #<id>"`.
    /// Publishes on Ok.
    pub fn fail(&self, id: u32, reason: impl Into<String>) -> Result<(), String> {
        let reason = reason.into();
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            self.inner.record(&PlanOp::Fail {
                id,
                reason: reason.clone(),
            })?;
            task.state = TaskState::Failed;
            task.feedback = Some(reason);
        }
        self.publish();
        Ok(())
    }
    /// A gate's rejection (§5): `gate_id` must be a gate; re-open each of its
    /// `deps` (via [`TaskList::reopen`]) and return `(dep_id, outcome)` per dep —
    /// so the caller can journal `reopen` vs. `fail` (§7). A dep that hit the cap
    /// becomes `Failed` with [`ReopenOutcome::ReachedCap`]. The gate itself is
    /// returned to `Todo` (§6): on the success path it is already in the cone
    /// (`reopen` resets it); on the cap path — where `reopen` leaves the cone
    /// alone — this is what re-arms it, so a `reset` of the dep can resume. Err
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
        if deps.is_empty() {
            return Err(format!("task #{gate_id} is a gate with no deps to re-open"));
        }
        let reason = reason.into();
        // ONE line per reject (Ruling 1); replay re-runs the cone via `reopen_inner`.
        self.inner.record(&PlanOp::Reject {
            gate: gate_id,
            reason: reason.clone(),
        })?;
        let mut reopened = Vec::with_capacity(deps.len());
        for dep in deps {
            let outcome = self.reopen_inner(dep, &reason)?;
            reopened.push((dep, outcome));
        }
        // §6: a gate does not "finish" when it rejects — return it to `Todo`.
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            if let Some(gate) = tasks.iter_mut().find(|t| t.id == gate_id) {
                gate.state = TaskState::Todo;
                gate.artifact = None;
            }
        }
        self.publish();
        Ok(reopened)
    }

    /// The ready frontier, in creation order: every `Todo` node all of whose
    /// deps are `Done` (derived, never stored, §3). Read-only — no publish.
    // Called by the scheduler's `frontier` (P1).
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
        let snapshot = self.snapshot();
        self.inner.updates.send_replace(snapshot.clone());
        self.inner.maybe_compact(&snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `create` hands out strictly increasing ids, starting at 1.
    #[test]
    fn create_returns_increasing_ids() {
        let list = TaskList::new();
        let a = list.create("first", vec![], false).unwrap();
        let b = list.create("second", vec![], false).unwrap();
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
        let err = list.create("t", vec![7], false).unwrap_err();
        assert!(err.contains('7'), "names the bad dep: {err}");
        assert!(list.snapshot().is_empty(), "nothing is created");
    }

    /// `assign` records the owner and leaves the state `Todo` (the `Doing`
    /// transition is `start`'s); an unknown id is an error (and leaves the list
    /// untouched).
    #[test]
    fn assign_sets_the_owner() {
        let list = TaskList::new();
        let t = list.create("first", vec![], false).unwrap();
        list.assign(t.id, SessionId::agent("w1")).unwrap();

        let got = &list.snapshot()[0];
        assert_eq!(got.owner, Some(SessionId::agent("w1")));
        assert_eq!(got.state, TaskState::Todo, "assign only records the owner");

        let err = list.assign(99, SessionId::agent("w2")).unwrap_err();
        assert!(err.contains("99"), "names the unknown id: {err}");
        assert_eq!(list.snapshot().len(), 1, "the list is unchanged");
    }

    /// `complete` flips a task to `Done`; an unknown id is an error.
    #[test]
    fn complete_flips_state() {
        let list = TaskList::new();
        let t = list.create("first", vec![], false).unwrap();
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
        let a = list.create("a", vec![], false).unwrap();
        let b = list.create("b", vec![a.id], false).unwrap();

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
        list.create("first", vec![], false).unwrap();

        let mut rx = list.subscribe();
        // Seeded with the current list, no publish needed.
        assert_eq!(rx.borrow().len(), 1);
        assert_eq!(rx.borrow()[0].title, "first");

        // A post-subscribe create publishes a fresh snapshot.
        list.create("second", vec![], false).unwrap();
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
        let a = list.create("a", vec![], false).unwrap();
        let b = list.create("b", vec![a.id], false).unwrap();

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
        let a = list.create("a", vec![], false).unwrap();
        let b = list.create("b", vec![a.id], false).unwrap();

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
        let t1 = list.create("one", vec![], false).unwrap();
        let t2 = list.create("two", vec![t1.id], false).unwrap();
        let t3 = list.create("three", vec![t2.id], false).unwrap();
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

    /// `reopen` past the cap sets `Failed` (terminal) and — the regression guard
    /// — leaves the downstream cone untouched: a capped node does not invalidate
    /// its dependents. (P2 reworks this branch.)
    #[test]
    fn reopen_hits_the_cap_and_fails_terminal() {
        let list = TaskList::new();
        assert_eq!(list.max_attempts(), 3);
        let t = list.create("t", vec![], false).unwrap();
        let child = list.create("child", vec![t.id], false).unwrap();

        // Three successful reopens, each invalidating the cone (child → Todo).
        for _ in 0..3 {
            assert_eq!(list.reopen(t.id, "again").unwrap(), ReopenOutcome::Reopened);
            assert_eq!(
                list.snapshot()[1].state,
                TaskState::Todo,
                "a successful reopen resets the cone"
            );
        }

        // Re-establish the cone as Done, each with an artifact.
        list.complete(t.id, Some("a1".into())).unwrap();
        list.complete(child.id, Some("a2".into())).unwrap();

        // The 4th reopen exceeds the cap: node 1 → Failed, cone NOT touched.
        assert_eq!(list.reopen(t.id, "again").unwrap(), ReopenOutcome::ReachedCap);
        let snap = list.snapshot();
        assert_eq!(snap[0].state, TaskState::Failed);
        assert_eq!(
            snap[1].state,
            TaskState::Done,
            "the cone is not reset when the cap is hit"
        );
        assert_eq!(
            snap[1].artifact.as_deref(),
            Some("a2"),
            "the descendant's artifact survives a cap hit"
        );
        assert_eq!(TaskState::Failed.label(), "failed");
    }

    /// A gate's `reject` re-opens its dep and the gate (in the cone); a
    /// non-gate's `reject` is an error.
    #[test]
    fn reject_reopens_the_gated_dep_and_the_cone() {
        let list = TaskList::new();
        let work = list.create("work", vec![], false).unwrap();
        let gate = list.create("verify", vec![work.id], false).unwrap();
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
        let work2 = list.create("w2", vec![], false).unwrap();
        let err = list.reject(work2.id, "nope").unwrap_err();
        assert!(err.contains("not a gate"), "{err}");
    }

    /// `reset` clears a terminal `Failed` node back to a fresh `Todo`.
    #[test]
    fn reset_clears_a_failed_node() {
        let list = TaskList::new();
        let t = list.create("t", vec![], false).unwrap();
        for _ in 0..4 {
            list.reopen(t.id, "again").unwrap();
        }
        assert_eq!(list.snapshot()[0].state, TaskState::Failed, "capped");

        list.reset(t.id).unwrap();
        let got = &list.snapshot()[0];
        assert_eq!(got.state, TaskState::Todo);
        assert_eq!(got.attempts, 0);
        assert_eq!(got.feedback, None);
        assert_eq!(got.artifact, None);

        // A bad id errors, naming it.
        let err = list.reset(99).unwrap_err();
        assert!(err.contains("99"), "{err}");
    }

    /// `reject` returns the GATE to `Todo` even when a dep hits the cap — the P2
    /// blocker fix (without it a capped dep's gate stays `Doing` forever).
    #[test]
    fn reject_unsticks_the_gate_on_a_cap() {
        let list = TaskList::new();
        let work = list.create("work", vec![], false).unwrap();
        let gate = list.create("verify", vec![work.id], true).unwrap();
        list.complete(work.id, Some("A".into())).unwrap();

        // Three reworks, then the gate is on it (Doing) when the cap hits.
        for _ in 0..3 {
            let r = list.reject(gate.id, "nope").unwrap();
            assert_eq!(r, vec![(work.id, ReopenOutcome::Reopened)]);
        }
        list.start(gate.id).unwrap();
        assert_eq!(list.snapshot()[1].state, TaskState::Doing);

        let r = list.reject(gate.id, "nope").unwrap();
        assert_eq!(r, vec![(work.id, ReopenOutcome::ReachedCap)]);
        let snap = list.snapshot();
        assert_eq!(snap[0].state, TaskState::Failed, "the dep hit the cap");
        assert_eq!(
            snap[1].state,
            TaskState::Todo,
            "reject returns the gate to Todo even on a cap"
        );
    }

    /// `fail` sets a terminal `Failed` with the reason, leaving `attempts` at 0
    /// (a deterministic `Script` verdict, NOT a rework — so not via `reopen`).
    #[test]
    fn fail_sets_terminal_state_without_a_rework() {
        let list = TaskList::new();
        let t = list.create("build", vec![], false).unwrap();
        list.fail(t.id, "exit 1").unwrap();

        let got = &list.snapshot()[0];
        assert_eq!(got.state, TaskState::Failed);
        assert_eq!(got.attempts, 0, "a fail is not a rework");
        assert_eq!(got.feedback.as_deref(), Some("exit 1"));

        let err = list.fail(99, "x").unwrap_err();
        assert!(err.contains("99"), "{err}");
    }

/// `PlanOp`/`Task` survive a serde round-trip (the journal's encoding).
#[test]
fn plan_op_and_task_serde_round_trip() {
    let list = TaskList::new();
    let t = list.create("work", vec![], true).unwrap();
    let s = serde_json::to_string(&t).unwrap();
    assert_eq!(serde_json::from_str::<Task>(&s).unwrap(), t);

    let op = PlanOp::Create {
        id: 1,
        title: "t".into(),
        deps: vec![],
        gate: false,
    };
    let s = serde_json::to_string(&op).unwrap();
    assert_eq!(serde_json::from_str::<PlanOp>(&s).unwrap(), op);
}

/// `reject` refuses a gate with no deps to re-open — it would loop forever.
    #[test]
    fn reject_refuses_a_dep_less_gate() {
        let list = TaskList::new();
        let gate = list.create("check", vec![], true).unwrap();
        let err = list.reject(gate.id, "nope").unwrap_err();
        assert!(err.contains("no deps to re-open"), "{err}");
    }

    /// A bare task record for building a `Snapshot` by hand.
    fn task(id: u32, title: &str) -> Task {
        Task {
            id,
            title: title.into(),
            owner: None,
            state: TaskState::Todo,
            deps: vec![],
            attempts: 0,
            feedback: None,
            artifact: None,
            gate: false,
            run: RunSpec::Session { member: None },
        }
    }

    /// Round-trip: a journaled list reloads to an identical snapshot.
    #[test]
    fn journal_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let l = TaskList::with_journal(path.clone());
        let a = l.create("a", vec![], false).unwrap();
        let b = l.create("b", vec![a.id], false).unwrap();
        l.assign(b.id, SessionId::agent("w1")).unwrap();
        l.complete(a.id, Some("A".into())).unwrap();
        l.configure(b.id, RunSpec::Session { member: Some("rev".into()) }, true)
            .unwrap();
        l.reject(b.id, "redo").unwrap();
        l.reset(a.id).unwrap();
        l.fail(a.id, "boom").unwrap();

        let r = TaskList::load(&path).unwrap();
        assert_eq!(r.snapshot(), l.snapshot());
    }

    /// Blocker A: a `Create` after a `Snapshot` gets a FRESH, non-colliding id.
    #[test]
    fn a_create_after_a_snapshot_gets_a_fresh_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let snap = PlanOp::Snapshot {
            tasks: vec![task(1, "one"), task(2, "two")],
        };
        std::fs::write(&path, format!("{}\n", serde_json::to_string(&snap).unwrap())).unwrap();

        let l = TaskList::load(&path).unwrap();
        assert_eq!(l.snapshot().len(), 2);
        let c = l.create("three", vec![], false).unwrap();
        assert_eq!(c.id, 3, "a Create after a Snapshot must not collide with its ids");
    }

    /// A torn trailing line is dropped (mirrors `Session::open`).
    #[test]
    fn a_torn_tail_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let one = PlanOp::Create {
            id: 1,
            title: "a".into(),
            deps: vec![],
            gate: false,
        };
        let two = PlanOp::Create {
            id: 2,
            title: "b".into(),
            deps: vec![],
            gate: false,
        };
        let body = format!(
            "{}\n{}\n{{\"op\":\"create\",\"id\":3",
            serde_json::to_string(&one).unwrap(),
            serde_json::to_string(&two).unwrap()
        );
        std::fs::write(&path, body).unwrap();

        let l = TaskList::load(&path).unwrap();
        assert_eq!(l.snapshot().len(), 2, "the torn third line is dropped");
    }

    /// Reconcile: a journal ending with a `Doing` node reloads as `Todo`,
    /// `attempts` unchanged, with a `Restart` line appended.
    #[test]
    fn reload_reconciles_a_doing_node() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let l = TaskList::with_journal(path.clone());
        let a = l.create("a", vec![], false).unwrap();
        l.assign(a.id, SessionId::agent("w1")).unwrap();
        l.start(a.id).unwrap();
        assert_eq!(l.snapshot()[0].state, TaskState::Doing);

        let r = TaskList::load(&path).unwrap();
        let got = &r.snapshot()[0];
        assert_eq!(got.state, TaskState::Todo, "a Doing node was reconciled");
        assert_eq!(got.attempts, 0, "a crash is not the node's fault");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("restart"), "a Restart line was journaled: {raw}");
    }

    /// A `reject` writes exactly ONE line (no nested `reopen` lines).
    #[test]
    fn reject_journals_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let l = TaskList::with_journal(path.clone());
        let work = l.create("work", vec![], false).unwrap();
        let gate = l.create("verify", vec![work.id], true).unwrap();
        l.complete(work.id, Some("A".into())).unwrap();
        l.reject(gate.id, "redo").unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            raw.lines().filter(|l| l.contains("\"op\":\"reject\"")).count(),
            1
        );
        assert_eq!(
            raw.lines().filter(|l| l.contains("\"op\":\"reopen\"")).count(),
            0
        );
    }

    /// A `Snapshot` line replaces the set; later ops apply on top.
    #[test]
    fn snapshot_replaces_the_set_then_later_ops_apply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let snap = PlanOp::Snapshot {
            tasks: vec![task(1, "one"), task(2, "two")],
        };
        let reopen = PlanOp::Reopen {
            id: 1,
            reason: "x".into(),
        };
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&snap).unwrap(),
                serde_json::to_string(&reopen).unwrap()
            ),
        )
        .unwrap();

        let l = TaskList::load(&path).unwrap();
        assert_eq!(l.snapshot().len(), 2);
        assert_eq!(l.snapshot()[0].attempts, 1, "the reopen applied on top");
    }

    /// The journal compacts to a single `Snapshot` past `COMPACT_AFTER`.
    #[test]
    fn the_journal_compacts_to_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.ndjson");
        let l = TaskList::with_journal(path.clone());
        let n = COMPACT_AFTER + 10;
        for i in 0..n {
            l.create(format!("t{i}"), vec![], false).unwrap();
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.lines().next().unwrap().contains("\"op\":\"snapshot\""),
            "compacted to a snapshot"
        );
        let r = TaskList::load(&path).unwrap();
        assert_eq!(r.snapshot().len(), n as usize);
    }
}
