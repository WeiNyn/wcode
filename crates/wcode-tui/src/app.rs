//! The TUI's application state — a small, flat struct plus pure reducers.
//!
//! Nothing here touches the terminal or crossterm: [`App::handle`] is a pure
//! function of `(state, event)`, so the whole layer is testable headless. The
//! app never performs IO; it defers side effects as [`Action`]s the event loop
//! drains. The loop feeds it [`AppEvent`]s and draws when [`App::dirty`] is set.

use wcode_harness::event::AgentEvent;
use wcode_harness::message::{AgentMessage, ContentBlock};

/// A key the app understands — decoupled from crossterm so this module stays
/// terminal-free (`event.rs` translates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Enter,
    Esc,
    /// A control chord, e.g. `Ctrl('c')` for Ctrl-C.
    Ctrl(char),
}

/// Everything the app can react to, from any source.
#[derive(Clone, Debug)]
pub enum AppEvent {
    Key(Key),
    Paste(String),
    /// A fact from the session (streamed, or a correlated reply).
    Agent(AgentEvent),
    Tick,
}

/// A side effect the event loop must perform — the app's only outward channel.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Submit(String),
    Cancel,
}

/// A committed transcript block. Thinking and prose share [`Block::Assistant`]
/// (an assistant message interleaves them); tool calls get their own line.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    User(String),
    Assistant(Vec<ContentBlock>),
    Tool(Tool),
    Notice(String),
    Error(String),
}

/// One tool invocation, from `ToolExecutionStart` to `ToolExecutionEnd`.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub output: String,
    pub done: bool,
    pub is_error: bool,
}

/// Status-line fields.
#[derive(Clone, Debug)]
pub struct Status {
    pub model: String,
    pub effort: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            model: "wcode".to_string(),
            effort: None,
        }
    }
}

impl Status {
    pub fn new(model: impl Into<String>) -> Self {
        Status {
            model: model.into(),
            effort: None,
        }
    }
}

/// The whole UI state. Flat by design — grow submodules only when it hurts.
#[derive(Default)]
pub struct App {
    transcript: Vec<Block>,
    /// The assistant message currently streaming (rendered below the transcript).
    live: Option<AgentMessage>,
    input: String,
    /// Cursor position as a *character* index into `input`.
    cursor: usize,
    status: Status,
    running: bool,
    /// A `Cancel` was sent; the next `AgentEnd` is rendered as an abort.
    cancelled: bool,
    dirty: bool,
    should_quit: bool,
    actions: Vec<Action>,
}

impl App {
    pub fn new() -> Self {
        App {
            dirty: true,
            ..App::default()
        }
    }

    /// Apply one event. Pure state transition; sets [`App::dirty`] on a change.
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Paste(text) => self.insert_str(&text),
            AppEvent::Agent(event) => self.on_agent(event),
            AppEvent::Tick => {}
        }
    }

    fn on_key(&mut self, key: Key) {
        match key {
            Key::Char(c) => self.insert_char(c),
            Key::Backspace => self.backspace(),
            Key::Delete => {}
            Key::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                self.dirty = true;
            }
            Key::Right => {
                if self.cursor < self.input.chars().count() {
                    self.cursor += 1;
                }
                self.dirty = true;
            }
            Key::Home => {
                self.cursor = 0;
                self.dirty = true;
            }
            Key::End => {
                self.cursor = self.input.chars().count();
                self.dirty = true;
            }
            Key::Enter => self.submit(),
            Key::Esc | Key::Ctrl('c') => self.interrupt(),
            _ => {}
        }
    }

    /// Esc / Ctrl-C: cancel a run, else quit.
    fn interrupt(&mut self) {
        if self.running {
            if !self.cancelled {
                self.cancelled = true;
                self.actions.push(Action::Cancel);
            }
        } else {
            self.should_quit = true;
        }
        self.dirty = true;
    }

    fn submit(&mut self) {
        let text = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.dirty = true;
        if text.trim().is_empty() {
            return;
        }
        if self.running {
            self.transcript
                .push(Block::Notice("a turn is already running — Esc to cancel".into()));
            return;
        }
        self.transcript.push(Block::User(text.clone()));
        self.running = true;
        self.cancelled = false;
        self.actions.push(Action::Submit(text));
    }

    fn on_agent(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::MessageStart { message } => {
                if matches!(message, AgentMessage::Assistant { .. }) {
                    self.live = Some(message);
                }
                self.dirty = true;
            }
            AgentEvent::MessageUpdate { message } => {
                if self.live.is_some() {
                    self.live = Some(message);
                    self.dirty = true;
                }
            }
            AgentEvent::MessageEnd { message } => {
                self.live = None;
                self.commit(message);
                self.dirty = true;
            }
            AgentEvent::ToolExecutionStart { name, .. } => {
                self.flush_live();
                self.transcript.push(Block::Tool(Tool {
                    name,
                    output: String::new(),
                    done: false,
                    is_error: false,
                }));
                self.dirty = true;
            }
            AgentEvent::ToolExecutionUpdate { partial, .. } => {
                if let Some(Block::Tool(tool)) = self.transcript.last_mut() {
                    tool.output.push_str(&partial);
                    self.dirty = true;
                }
            }
            AgentEvent::ToolExecutionEnd {
                output, is_error, ..
            } => {
                if let Some(Block::Tool(tool)) = self.transcript.last_mut() {
                    if !output.is_empty() {
                        tool.output = output;
                    }
                    tool.done = true;
                    tool.is_error = is_error;
                    self.dirty = true;
                }
            }
            AgentEvent::AgentEnd => {
                self.flush_live();
                self.running = false;
                if self.cancelled {
                    self.cancelled = false;
                    self.transcript.push(Block::Notice("⏹ aborted".into()));
                }
                self.dirty = true;
            }
            AgentEvent::Error { message } => {
                self.flush_live();
                self.transcript.push(Block::Error(message));
                self.dirty = true;
            }
            AgentEvent::Compaction { summarized, kept } => {
                self.transcript.push(Block::Notice(format!(
                    "⋯ compacted {summarized} messages, kept {kept}"
                )));
                self.dirty = true;
            }
            AgentEvent::Retrying {
                attempt,
                max,
                reason,
            } => {
                self.transcript.push(Block::Notice(format!(
                    "⋯ retrying ({attempt}/{max}): {reason}"
                )));
                self.dirty = true;
            }
            // Turn boundaries, start, and non-streamed replies need no state.
            _ => {}
        }
    }

    /// Commit an assistant message, dropping tool-call blocks (the tool lines
    /// carry those) and empty messages.
    fn commit(&mut self, message: AgentMessage) {
        if let AgentMessage::Assistant { content, .. } = message {
            let visible: Vec<ContentBlock> = content
                .into_iter()
                .filter(|b| !matches!(b, ContentBlock::ToolCall { .. }))
                .collect();
            if !visible.is_empty() {
                self.transcript.push(Block::Assistant(visible));
            }
        }
    }

    /// Commit a still-streaming message (a tool started, the run ended early).
    fn flush_live(&mut self) {
        if let Some(message) = self.live.take() {
            self.commit(message);
        }
    }

    fn insert_char(&mut self, c: char) {
        let at = self.byte_index(self.cursor);
        self.input.insert(at, c);
        self.cursor += 1;
        self.dirty = true;
    }

    fn insert_str(&mut self, text: &str) {
        let at = self.byte_index(self.cursor);
        self.input.insert_str(at, text);
        self.cursor += text.chars().count();
        self.dirty = true;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_index(self.cursor);
        let start = self.byte_index(self.cursor - 1);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
        self.dirty = true;
    }

    /// Byte offset of the `n`th character, or the string end.
    fn byte_index(&self, n: usize) -> usize {
        self.input
            .char_indices()
            .nth(n)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    pub fn transcript(&self) -> &[Block] {
        &self.transcript
    }

    pub fn live(&self) -> Option<&AgentMessage> {
        self.live.as_ref()
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn set_status(&mut self, status: Status) {
        self.status = status;
        self.dirty = true;
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Drain the side effects accumulated since the last call.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wcode_harness::message::StopReason;

    fn typed(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
    }

    fn assistant(text: &str) -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        }
    }

    fn submit(app: &mut App, text: &str) {
        typed(app, text);
        app.handle(AppEvent::Key(Key::Enter));
    }

    #[test]
    fn typing_builds_the_input_buffer() {
        let mut app = App::new();
        typed(&mut app, "hello");
        assert_eq!(app.input(), "hello");
        assert_eq!(app.cursor(), 5);
    }

    #[test]
    fn backspace_removes_the_char_before_the_cursor() {
        let mut app = App::new();
        typed(&mut app, "abc");
        app.handle(AppEvent::Key(Key::Left));
        app.handle(AppEvent::Key(Key::Backspace));
        assert_eq!(app.input(), "ac");
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn paste_inserts_at_the_cursor() {
        let mut app = App::new();
        typed(&mut app, "ac");
        app.handle(AppEvent::Key(Key::Left));
        app.handle(AppEvent::Paste("b".into()));
        assert_eq!(app.input(), "abc");
        assert_eq!(app.cursor(), 2);
    }

    #[test]
    fn submit_commits_a_user_block_and_emits_an_action() {
        let mut app = App::new();
        submit(&mut app, "hi there");
        assert_eq!(app.input(), "");
        assert!(app.running());
        assert_eq!(app.transcript()[0], Block::User("hi there".into()));
        assert_eq!(app.take_actions(), vec![Action::Submit("hi there".into())]);
    }

    #[test]
    fn empty_submit_does_nothing() {
        let mut app = App::new();
        submit(&mut app, "   ");
        assert!(app.transcript().is_empty());
        assert!(app.take_actions().is_empty());
    }

    #[test]
    fn a_second_submit_while_running_is_refused() {
        let mut app = App::new();
        submit(&mut app, "first");
        let _ = app.take_actions();
        submit(&mut app, "second");
        assert!(matches!(app.transcript().last(), Some(Block::Notice(_))));
        assert!(app.take_actions().is_empty());
    }

    #[test]
    fn streaming_text_lands_in_the_transcript() {
        let mut app = App::new();
        submit(&mut app, "hi");
        let _ = app.take_actions();

        app.handle(AppEvent::Agent(AgentEvent::MessageStart {
            message: assistant(""),
        }));
        assert!(app.live().is_some());
        app.handle(AppEvent::Agent(AgentEvent::MessageUpdate {
            message: assistant("hel"),
        }));
        app.handle(AppEvent::Agent(AgentEvent::MessageEnd {
            message: assistant("hello"),
        }));
        app.handle(AppEvent::Agent(AgentEvent::AgentEnd));

        assert!(app.live().is_none());
        assert!(!app.running());
        assert_eq!(
            app.transcript().last(),
            Some(&Block::Assistant(vec![ContentBlock::Text {
                text: "hello".into()
            }]))
        );
    }

    #[test]
    fn tool_lifecycle_becomes_one_tool_block() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(AgentEvent::ToolExecutionStart {
            call_id: "t1".into(),
            name: "bash".into(),
        }));
        app.handle(AppEvent::Agent(AgentEvent::ToolExecutionUpdate {
            call_id: "t1".into(),
            name: "bash".into(),
            partial: "building\n".into(),
        }));
        app.handle(AppEvent::Agent(AgentEvent::ToolExecutionEnd {
            call_id: "t1".into(),
            name: "bash".into(),
            output: "building\nok".into(),
            is_error: false,
        }));
        match app.transcript().last() {
            Some(Block::Tool(tool)) => {
                assert_eq!(tool.name, "bash");
                assert!(tool.done);
                assert!(!tool.is_error);
                assert_eq!(tool.output, "building\nok");
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[test]
    fn cancel_is_sent_once_and_rendered_as_an_abort() {
        let mut app = App::new();
        submit(&mut app, "go");
        let _ = app.take_actions();

        app.handle(AppEvent::Key(Key::Esc));
        app.handle(AppEvent::Key(Key::Esc));
        assert_eq!(app.take_actions(), vec![Action::Cancel]);
        assert!(!app.should_quit());

        app.handle(AppEvent::Agent(AgentEvent::AgentEnd));
        assert!(!app.running());
        assert_eq!(app.transcript().last(), Some(&Block::Notice("⏹ aborted".into())));
    }

    #[test]
    fn esc_quits_when_idle() {
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::Esc));
        assert!(app.should_quit());
    }

    #[test]
    fn dirty_is_set_on_change_and_cleared_explicitly() {
        let mut app = App::new();
        app.clear_dirty();
        assert!(!app.dirty());
        typed(&mut app, "x");
        assert!(app.dirty());
        app.clear_dirty();
        assert!(!app.dirty());
    }
}
