//! The `todo` tool: a session-local checklist (a plan the model keeps for itself).
//!
//! Unlike `task` — root-only, team-scoped, and fed to the TUI through a CLI-side
//! injected-view channel a socket client cannot see (main.rs `new_tasks = None`
//! over a socket) — `todo` is:
//! - **every session's** (registered in `default_tools`, workers included), and
//! - **event-sourced**: a write emits `AgentEvent::Todo { todos }` on the run's
//!   sink (`ToolContext.events`, wired `events: sink.clone()` in `loop_.rs`), so a
//!   LOCAL and a REMOTE (socket) TUI learn the state through the existing event
//!   seam.
//!
//! State is an in-memory `Mutex<Vec<TodoItem>>` inside the tool instance: one
//! `Agent` builds its own tools, so the checklist is per-session for free. It is
//! **persisted** as a `SessionEntry::Todo` (D7): every write appends the list to
//! the session, and on first access the tool seeds from
//! `ToolContext.session_path`, so a bare read after `--resume`/`/reload` reflects
//! the restored checklist instead of answering `(no todos)` against a visible
//! one.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use wcode_harness::event::{AgentEvent, TodoItem, TodoStatus};
use wcode_harness::session::{Session, SessionEntry};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

/// Args for the `todo` tool. A `todos` array is a full-list WRITE; omitting it
/// is a READ (the familiar TodoWrite shape).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TodoArgs {
    /// The new checklist. Present = REPLACE the whole list and emit the update;
    /// absent = return the current list unchanged. An empty array is a legitimate
    /// CLEAR.
    #[serde(default)]
    pub todos: Option<Vec<TodoItem>>,
}

/// The session-local checklist tool. `parallel_safe` stays `false` (the trait
/// default): it mutates shared state, so it is a barrier in a batch (like
/// `task`).
pub struct Todo {
    items: Mutex<Vec<TodoItem>>,
    /// Whether the persisted checklist has been loaded once (D7); guards the
    /// first-access seed so an empty/cleared list is not re-read every call.
    seeded: AtomicBool,
}

impl Todo {
    /// Lazily load the persisted checklist the first time the tool runs in a
    /// session that has one (D7). Runs once per tool instance — guarded by
    /// `seeded`, so a legitimately empty/cleared list is not re-read every call.
    fn seed_from_session(&self, ctx: &ToolContext) {
        if self.seeded.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(path) = &ctx.session_path else {
            return;
        };
        if let Ok(session) = Session::open(path)
            && let Some(restored) = session.todo()
        {
            *self.items.lock().unwrap_or_else(|e| e.into_inner()) = restored;
        }
    }

    /// Persist a write so the checklist survives `--resume`/`/reload` (D7). The
    /// tool opens its OWN session handle (the agent's in-memory `Session` never
    /// sees this entry); both writers open O_APPEND and write one full JSON line
    /// per call, and a turn is serialized around its tool calls, so appends
    /// cannot interleave.
    fn persist(&self, todos: &[TodoItem], ctx: &ToolContext) {
        let Some(path) = &ctx.session_path else {
            return;
        };
        if let Ok(mut session) = Session::open(path) {
            let _ = session.append(SessionEntry::Todo {
                id: uuid::Uuid::new_v4().to_string(),
                todos: todos.to_vec(),
            });
        }
    }
    pub fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            seeded: AtomicBool::new(false),
        }
    }

    /// Number of items currently held (test convenience; not on the wire).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl Default for Todo {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the list, one line per item: `[ ]` pending, `[>]` in_progress,
/// `[x]` completed. `(no todos)` when empty.
pub fn render(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "(no todos)".to_string();
    }
    todos
        .iter()
        .map(|item| {
            let mark = match item.status {
                TodoStatus::Pending => "[ ]",
                TodoStatus::InProgress => "[>]",
                TodoStatus::Completed => "[x]",
            };
            format!("{mark} {}", item.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Validate a write: `content` must be non-empty after trim. `Err(message)` names
/// the offending index; the model sees it as the tool's `is_error` output. An
/// empty list is valid (a clear).
fn validate(todos: &[TodoItem]) -> Result<(), String> {
    for (i, item) in todos.iter().enumerate() {
        if item.content.trim().is_empty() {
            return Err(format!("todo item {i} has empty `content`"));
        }
    }
    Ok(())
}

fn err(message: impl Into<String>) -> ToolOutput {
    ToolOutput {
        output: message.into(),
        is_error: true,
        ..ToolOutput::default()
    }
}

#[async_trait::async_trait]
impl TypedTool for Todo {
    type Args = TodoArgs;

    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "Maintain a session-local todo list. Call with `todos` to replace the whole \
         list (each item has `content` and a `status` of pending | in_progress | \
         completed); call with no arguments to read the current list back. Use it \
         to plan and track multi-step work within this session."
    }

    async fn execute(&self, args: TodoArgs, ctx: &ToolContext) -> ToolOutput {
        self.seed_from_session(ctx);
        match args.todos {
            Some(list) => {
                if let Err(message) = validate(&list) {
                    return err(message);
                }
                *self.items.lock().unwrap_or_else(|e| e.into_inner()) = list.clone();
                self.persist(&list, ctx);
                // The event IS the read path (the UI reduces it; the tool holds no
                // shared handle).
                let _ = ctx.events.send(AgentEvent::Todo { todos: list.clone() });
                ToolOutput {
                    output: render(&list),
                    ..ToolOutput::default()
                }
            }
            None => {
                let items = self.items.lock().unwrap_or_else(|e| e.into_inner());
                ToolOutput {
                    output: render(&items),
                    ..ToolOutput::default()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(content: &str, status: TodoStatus) -> TodoItem {
        TodoItem {
            content: content.into(),
            status,
        }
    }

    /// A tool context plus the receiver end of its `events` sink, so a test can
    /// assert the emitted `AgentEvent::Todo` (mirrors `task.rs::ctx`).
    fn ctx() -> (
        ToolContext,
        tokio::sync::mpsc::UnboundedReceiver<wcode_harness::event::AgentEvent>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ToolContext {
            call_id: "t1".into(),
            name: "todo".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events: tx,
            session_path: None,
        };
        (ctx, rx)
    }

    #[tokio::test]
    async fn a_write_replaces_the_list_and_renders_it() {
        let tool = Todo::new();
        let (ctx, _rx) = ctx();
        let out = tool
            .execute(
                TodoArgs {
                    todos: Some(vec![
                        item("a", TodoStatus::Pending),
                        item("b", TodoStatus::InProgress),
                    ]),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{out:?}");
        assert_eq!(out.output, "[ ] a\n[>] b");

        // A second write REPLACES the whole list (not append).
        let out = tool
            .execute(
                TodoArgs {
                    todos: Some(vec![item("c", TodoStatus::Completed)]),
                },
                &ctx,
            )
            .await;
        assert_eq!(out.output, "[x] c");
        assert_eq!(tool.len(), 1, "the earlier items are gone");
    }

    #[tokio::test]
    async fn a_bare_call_reads_without_changing_the_list() {
        let tool = Todo::new();
        let (ctx, mut rx) = ctx();
        tool.execute(
            TodoArgs {
                todos: Some(vec![item("a", TodoStatus::Pending)]),
            },
            &ctx,
        )
        .await;
        let _ = rx.try_recv(); // drain the write event

        let out = tool.execute(TodoArgs { todos: None }, &ctx).await;
        assert!(!out.is_error, "{out:?}");
        assert_eq!(out.output, "[ ] a", "the read returns the same rendering");
        assert!(rx.try_recv().is_err(), "a read emits no event");
    }

    #[tokio::test]
    async fn a_write_emits_a_todo_event_on_the_sink() {
        let tool = Todo::new();
        let (ctx, mut rx) = ctx();
        let list = vec![item("a", TodoStatus::Pending), item("b", TodoStatus::Completed)];
        tool.execute(
            TodoArgs {
                todos: Some(list.clone()),
            },
            &ctx,
        )
        .await;
        match rx.try_recv() {
            Ok(AgentEvent::Todo { todos }) => assert_eq!(todos, list),
            other => panic!("expected a Todo event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_empty_list_clears_the_checklist() {
        let tool = Todo::new();
        let (ctx, mut rx) = ctx();
        tool.execute(
            TodoArgs {
                todos: Some(vec![item("a", TodoStatus::Pending)]),
            },
            &ctx,
        )
        .await;
        let _ = rx.try_recv();

        let out = tool.execute(TodoArgs { todos: Some(Vec::new()) }, &ctx).await;
        assert!(!out.is_error, "an empty list is a legitimate clear");
        assert_eq!(out.output, "(no todos)");
        assert_eq!(tool.len(), 0);
        match rx.try_recv() {
            Ok(AgentEvent::Todo { todos }) => assert!(todos.is_empty()),
            other => panic!("expected a clearing Todo event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn blank_content_is_rejected() {
        let tool = Todo::new();
        let (ctx, mut rx) = ctx();
        let out = tool
            .execute(
                TodoArgs {
                    todos: Some(vec![item("a", TodoStatus::Pending), item("  ", TodoStatus::InProgress)]),
                },
                &ctx,
            )
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("item 1"), "names the index: {}", out.output);
        assert_eq!(tool.len(), 0, "a rejected write changes nothing");
        assert!(rx.try_recv().is_err(), "a rejected write emits no event");
    }

    #[test]
    fn render_shows_each_status() {
        assert_eq!(render(&[]), "(no todos)");
        assert_eq!(
            render(&[
                item("a", TodoStatus::Pending),
                item("b", TodoStatus::InProgress),
                item("c", TodoStatus::Completed),
            ]),
            "[ ] a\n[>] b\n[x] c"
        );
    }

    /// A context whose `session_path` is `path`, so the seed/persist paths engage.
    fn ctx_at(path: &std::path::Path) -> ToolContext {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "t1".into(),
            name: "todo".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events: tx,
            session_path: Some(path.to_path_buf()),
        }
    }

    /// A fresh session file carrying one `Todo` entry with `todos`.
    fn session_with_todo(dir: &std::path::Path, todos: Vec<TodoItem>) -> std::path::PathBuf {
        let mut s = wcode_harness::session::Session::create(dir).unwrap();
        s.append(wcode_harness::session::SessionEntry::Todo {
            id: "t0".into(),
            todos,
        })
        .unwrap();
        s.path().unwrap().to_path_buf()
    }

    #[tokio::test]
    async fn a_write_persists_a_session_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = session_with_todo(dir.path(), Vec::new());
        let tool = Todo::new();
        let ctx = ctx_at(&path);
        let list = vec![
            item("a", TodoStatus::Pending),
            item("b", TodoStatus::Completed),
        ];
        tool.execute(
            TodoArgs {
                todos: Some(list.clone()),
            },
            &ctx,
        )
        .await;

        // The write appended a `SessionEntry::Todo`; a reopen reads it back.
        let reopened = wcode_harness::session::Session::open(&path).unwrap();
        assert_eq!(reopened.todo().unwrap(), list);
    }

    #[tokio::test]
    async fn a_bare_read_seeds_from_the_persisted_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = session_with_todo(dir.path(), vec![item("restored", TodoStatus::InProgress)]);
        let tool = Todo::new();
        let ctx = ctx_at(&path);

        // A brand-new tool (empty in memory) reflects the persisted list.
        let out = tool.execute(TodoArgs { todos: None }, &ctx).await;
        assert!(!out.is_error, "{out:?}");
        assert_eq!(out.output, "[>] restored", "the bare read reflects the seed");
    }

    #[tokio::test]
    async fn the_seed_flag_keeps_a_cleared_list_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let path = session_with_todo(dir.path(), vec![item("restored", TodoStatus::Pending)]);
        let tool = Todo::new();
        let ctx = ctx_at(&path);

        // First access seeds the persisted list.
        let out = tool.execute(TodoArgs { todos: None }, &ctx).await;
        assert_eq!(out.output, "[ ] restored");

        // Clear it — the persisted file still holds the old list.
        tool.execute(TodoArgs { todos: Some(Vec::new()) }, &ctx).await;
        assert_eq!(tool.len(), 0);

        // A later bare read must NOT re-seed the empty list from the file: the
        // list stays cleared.
        let out = tool.execute(TodoArgs { todos: None }, &ctx).await;
        assert_eq!(out.output, "(no todos)", "a cleared list stays cleared");
        assert_eq!(tool.len(), 0);
    }
}
