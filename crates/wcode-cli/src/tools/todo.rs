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
//! NOT persisted in v1 — `--resume`/`/reload` start empty (a follow-on).

use std::sync::Mutex;

use serde::Deserialize;
use wcode_harness::event::{AgentEvent, TodoItem, TodoStatus};
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
}

impl Todo {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
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
        match args.todos {
            Some(list) => {
                if let Err(message) = validate(&list) {
                    return err(message);
                }
                *self.items.lock().unwrap_or_else(|e| e.into_inner()) = list.clone();
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
}
