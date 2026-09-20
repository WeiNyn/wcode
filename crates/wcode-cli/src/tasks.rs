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

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::watch;
use wcode_harness::protocol::SessionId;

/// A task's lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// Planned, not started.
    Todo,
    /// A worker is on it.
    Doing,
    /// Finished.
    Done,
}

impl TaskState {
    /// The lowercase label shown by the TUI / `task list` (`todo` / `doing` /
    /// `done`).
    pub fn label(self) -> &'static str {
        match self {
            TaskState::Todo => "todo",
            TaskState::Doing => "doing",
            TaskState::Done => "done",
        }
    }
}

/// One planned task.
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
}

impl Default for Inner {
    /// Hand-written (not derived): `watch::Sender` has no `Default`, and the id
    /// source must start at 1 (not 0).
    fn default() -> Self {
        Self {
            tasks: Mutex::new(Vec::new()),
            next_id: AtomicU32::new(1),
            updates: watch::channel(Vec::new()).0,
        }
    }
}

impl TaskList {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a `Todo` task with the next id, publish, and return a copy of it.
    pub fn create(&self, title: impl Into<String>) -> Task {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let task = Task {
            id,
            title: title.into(),
            owner: None,
            state: TaskState::Todo,
        };
        self.inner.tasks.lock().unwrap().push(task.clone());
        self.publish();
        task
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

    /// Flip `id` to [`TaskState::Done`]. `Err` names an unknown id.
    pub fn complete(&self, id: u32) -> Result<(), String> {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task #{id}"))?;
            task.state = TaskState::Done;
        }
        self.publish();
        Ok(())
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
        let a = list.create("first");
        let b = list.create("second");
        assert_eq!(a.id, 1, "ids start at 1");
        assert_eq!(b.id, 2);
        assert_eq!(a.state, TaskState::Todo);
        assert_eq!(a.owner, None);
        assert_eq!(list.snapshot().len(), 2);
    }

    /// `assign` records the owner and moves the task to `Doing`; an unknown id
    /// is an error (and leaves the list untouched).
    #[test]
    fn assign_sets_the_owner() {
        let list = TaskList::new();
        let t = list.create("first");
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
        let t = list.create("first");
        list.complete(t.id).unwrap();
        assert_eq!(list.snapshot()[0].state, TaskState::Done);

        let err = list.complete(99).unwrap_err();
        assert!(err.contains("99"), "names the unknown id: {err}");

        assert_eq!(TaskState::Todo.label(), "todo");
        assert_eq!(TaskState::Doing.label(), "doing");
        assert_eq!(TaskState::Done.label(), "done");
    }

    /// A `subscribe()` starts seeded and sees the new list after a mutation.
    #[tokio::test]
    async fn subscribe_sees_an_update() {
        let list = TaskList::new();
        list.create("first");

        let mut rx = list.subscribe();
        // Seeded with the current list, no publish needed.
        assert_eq!(rx.borrow().len(), 1);
        assert_eq!(rx.borrow()[0].title, "first");

        // A post-subscribe create publishes a fresh snapshot.
        list.create("second");
        rx.changed().await.expect("open");
        let seen = rx.borrow_and_update();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].title, "second");
    }
}
