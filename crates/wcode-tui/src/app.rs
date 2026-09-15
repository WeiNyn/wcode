//! The TUI's application state — a small, flat struct plus pure reducers.
//!
//! Nothing here touches the terminal or crossterm: [`App::handle`] is a pure
//! function of `(state, event)`, so the whole layer is testable headless. The
//! app never performs IO; it defers side effects as [`Action`]s the event loop
//! drains. The loop feeds it [`AppEvent`]s and draws when [`App::dirty`] is set.

use std::ops::Range;
use std::path::{Path, PathBuf};

use wcode_harness::event::AgentEvent;
use wcode_harness::message::{AgentMessage, ContentBlock};
use wcode_harness::protocol::Request;

/// Lines per mouse-wheel notch — a nudge, not a page (PgUp/PgDn page).
const WHEEL_LINES: usize = 3;

/// A key the app understands — decoupled from crossterm so this module stays
/// terminal-free (`event.rs` translates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// One mouse-wheel notch up (translated from a `ScrollUp` mouse event).
    ScrollUp,
    /// One mouse-wheel notch down.
    ScrollDown,
    Enter,
    /// Shift-Enter: insert a newline instead of submitting.
    Newline,
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
    /// The terminal was resized; force a redraw and re-measure the scroll.
    Resize,
}

/// A side effect the event loop must perform — the app's only outward channel.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Submit(String),
    Cancel,
    /// Send a request and route its reply back as an [`AppEvent::Agent`].
    Ask(Request),
    /// Copy this text to the terminal clipboard (OSC-52).
    Copy(String),
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
    /// A changed file's diff, re-shown from `/changes` (not the live render).
    Diff { path: String, diff: String },
}

/// One tool invocation, from `ToolExecutionStart` to `ToolExecutionEnd`.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub output: String,
    pub done: bool,
    pub is_error: bool,
    /// A UI-only unified diff, when the tool changed a file (`ToolOutput::diff`).
    pub diff: Option<String>,
    /// The file this tool changed (UI-only), from `ToolExecutionEnd`: labels the
    /// `⚙` line and feeds the run's changeset.
    pub path: Option<String>,
}

/// A file the run changed, recorded from the UI-only `ToolExecutionEnd` fields.
/// The changeset is per-run and view-owned: reset when the next prompt starts a
/// run, kept afterwards so it stays reviewable, and never persisted (durable
/// history is git's job).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub diff: String,
}

impl Change {
    fn new(path: String, diff: String) -> Self {
        let (added, removed) = diff_counts(&diff);
        Change {
            path,
            added,
            removed,
            diff,
        }
    }
}

/// Count `+`/`-` body lines in a unified diff — the `@@` header is neither.
pub(crate) fn diff_counts(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        match line.as_bytes().first() {
            Some(b'+') => added += 1,
            Some(b'-') => removed += 1,
            _ => {}
        }
    }
    (added, removed)
}

/// A transient modal drawn over the three bands. Shared infrastructure: any
/// list the user picks from (models now, sessions later) is an [`Overlay`],
/// opened from a `/`-command, capturing keys until dismissed, and drawn *over*
/// the base layout — it never reflows it (design §1).
#[derive(Clone, Debug)]
pub enum Overlay {
    Pick(Picker),
}

/// A filterable list the user navigates and selects from. `items` is the full
/// injected list; [`Picker::rows`] is the visible subset `selected` indexes.
#[derive(Clone, Debug)]
pub struct Picker {
    pub kind: PickerKind,
    pub title: String,
    pub items: Vec<String>,
    /// Per-item value yielded on selection, aligned with `items`. Empty means
    /// the item itself is the value (a plain list, e.g. the model picker).
    pub values: Vec<String>,
    pub query: String,
    /// Index into [`Picker::rows`], not into `items`.
    pub selected: usize,
}

/// What selecting a [`Picker`] row does — the [`Action`] the reducer emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    /// A file the run changed (`/changes`); selecting re-shows its diff.
    Change,
    /// A resumable session (`/resume`); selecting hands its path back to re-exec.
    Resume,
}

impl Picker {
    fn new(kind: PickerKind, title: impl Into<String>, items: Vec<String>) -> Self {
        Picker {
            kind,
            title: title.into(),
            items,
            values: Vec::new(),
            query: String::new(),
            selected: 0,
        }
    }

    /// The rows to show: each item matching `query` (case-insensitive) and, when
    /// filtering, the byte range of the match within it (for highlighting). An
    /// empty query shows every item.
    pub fn rows(&self) -> Vec<(&str, Option<std::ops::Range<usize>>)> {
        let query = self.query.to_lowercase();
        self.visible_indices()
            .into_iter()
            .map(|i| {
                let item = &self.items[i];
                let range = if query.is_empty() {
                    None
                } else {
                    item.to_lowercase()
                        .find(&query)
                        .map(|start| start..start + query.len())
                };
                (item.as_str(), range)
            })
            .collect()
    }

    /// A picker whose rows carry a distinct value (e.g. a changed path while
    /// the label shows stats). `values` is aligned with `items`.
    fn with_values(
        kind: PickerKind,
        title: impl Into<String>,
        items: Vec<String>,
        values: Vec<String>,
    ) -> Self {
        Picker {
            kind,
            title: title.into(),
            items,
            values,
            query: String::new(),
            selected: 0,
        }
    }

    /// Indices of the items matching the current query, in display order.
    fn visible_indices(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| query.is_empty() || item.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect()
    }

    /// The value of the highlighted row (e.g. a model id, a changed path),
    /// falling back to the label for a plain list.
    pub fn selected_value(&self) -> Option<&str> {
        let index = *self.visible_indices().get(self.selected)?;
        Some(self.values.get(index).map_or(&self.items[index], String::as_str))
    }

    fn move_selection(&mut self, delta: isize) {
        let last = self.rows().len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }
}

/// A resumable session offered by `/resume`: a display `label` (`id · age ·
/// first line`) and the file to hand back for a `--resume` re-exec. Injected by
/// the composition root — the TUI owns no session dir and cannot list one.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionItem {
    pub label: String,
    pub path: PathBuf,
}

/// Status-line fields.
#[derive(Clone, Debug)]
pub struct Status {
    pub model: String,
    pub effort: Option<String>,
    /// The session's id, shown in the status line (local sessions only).
    pub session: Option<String>,
    /// The model's context window, for the `used / limit` readout.
    pub context_limit: Option<u64>,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            model: "wcode".to_string(),
            effort: None,
            session: None,
            context_limit: None,
        }
    }
}

impl Status {
    pub fn new(model: impl Into<String>) -> Self {
        Status {
            model: model.into(),
            effort: None,
            session: None,
            context_limit: None,
        }
    }
}

/// One piece of the input buffer: a typed character, or a whole pasted blob kept
/// atomic (one Backspace removes it; Left/Right step over it in one move).
#[derive(Clone, Debug, PartialEq)]
enum Atom {
    Char(char),
    Paste(PasteBlock),
}

impl Atom {
    /// The text this atom contributes to the *submitted* prompt.
    fn text(&self) -> String {
        match self {
            Atom::Char(c) => c.to_string(),
            Atom::Paste(p) => p.text.clone(),
        }
    }

    /// What this atom shows in the input box (a paste collapses to a chip).
    fn display(&self) -> String {
        match self {
            Atom::Char(c) => c.to_string(),
            Atom::Paste(p) => p.chip(),
        }
    }
}

/// A pasted blob kept as one placeholder. `lines`/`chars` are measured once at
/// paste time; `id` distinguishes blocks (a future command can address one).
#[derive(Clone, Debug, PartialEq)]
struct PasteBlock {
    id: u64,
    text: String,
    lines: usize,
    chars: usize,
}

impl PasteBlock {
    /// The one-line chip shown in the input, e.g. `❰ pasted 3 lines · 128 chars ❱`.
    fn chip(&self) -> String {
        // `chars` counts characters (the threshold and the `chars` suffix); the
        // KB branch is sized in UTF-8 *bytes*, matching the unit's label.
        let size = if self.chars >= 1024 {
            format!("{:.1} KB", self.text.len() as f64 / 1024.0)
        } else {
            format!("{} chars", self.chars)
        };
        let line = if self.lines == 1 { "line" } else { "lines" };
        format!("❰ pasted {} {line} · {size} ❱", self.lines)
    }
}

/// A read-only view of the input for the renderer: the display string (paste
/// chips expanded to their chip text), the cursor's display column, and the char
/// ranges of the chips so they can be styled rather than shown raw.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InputView {
    pub(crate) display: String,
    pub(crate) cursor_col: usize,
    pub(crate) chips: Vec<Range<usize>>,
}

/// The whole UI state. Flat by design — grow submodules only when it hurts.
#[derive(Default)]
pub struct App {
    transcript: Vec<Block>,
    /// The assistant message currently streaming (rendered below the transcript).
    live: Option<AgentMessage>,
    input: Vec<Atom>,
    /// Cursor as a *gap index* between atoms (`0..=input.len()`).
    cursor: usize,
    /// Monotonic id source for [`PasteBlock`]s.
    paste_id: u64,
    /// Submitted prompts, oldest first, for Up/Down recall.
    history: Vec<String>,
    /// Index into `history` while browsing; `None` edits the draft.
    history_index: Option<usize>,
    /// The in-progress line, saved while browsing history.
    draft: String,
    status: Status,
    /// Model ids for the `/model` picker — injected by the composition root,
    /// since the TUI holds no `LlmOpts` and cannot list models itself.
    models: Vec<String>,
    /// Files changed during the current run, in call order; reset when the next
    /// prompt starts a run, kept afterwards so the run stays reviewable.
    changes: Vec<Change>,
    /// Resumable sessions for the `/resume` picker — injected by the composition
    /// root, which owns the session dir the TUI cannot see.
    sessions: Vec<SessionItem>,
    /// Set when the user picks a session to resume; the run returns it so the CLI
    /// can re-exec with `--resume <path>`.
    pending_resume: Option<PathBuf>,
    /// The open modal, if any. While one is shown it captures every key.
    overlay: Option<Overlay>,
    /// Provider-reported input tokens of the last turn: how full the context was.
    context_used: Option<u64>,
    /// Lines scrolled up from the bottom; `0` follows the tail.
    scroll: usize,
    /// Clamp for [`App::scroll`], set by the renderer from the line count.
    max_scroll: usize,
    /// Transcript height and total line count from the last draw, so the view
    /// can stay pinned while new lines stream in.
    viewport: usize,
    last_total: usize,
    last_width: usize,
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
            AppEvent::Paste(text) => {
                // A modal owns the input: a paste must not edit the buffer.
                if self.overlay.is_none() {
                    self.on_paste(text);
                }
            }
            AppEvent::Agent(event) => self.on_agent(event),
            AppEvent::Tick => {}
            AppEvent::Resize => {
                // Force a redraw; the width change resets the scroll pin.
                self.dirty = true;
            }
        }
    }

    fn on_key(&mut self, key: Key) {
        // A modal owns the keyboard: no key reaches the input while it is up.
        if self.overlay.is_some() {
            self.on_overlay_key(key);
            return;
        }
        match key {
            Key::Char(c) => {
                self.history_index = None;
                self.insert_char(c);
            }
            Key::Backspace => {
                self.history_index = None;
                self.backspace();
            }
            Key::Delete => {
                self.history_index = None;
                self.delete();
            }
            Key::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                self.dirty = true;
            }
            Key::Right => {
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
                self.dirty = true;
            }
            Key::Up => self.history_up(),
            Key::Down => self.history_down(),
            Key::Home => {
                self.cursor = 0;
                self.dirty = true;
            }
            Key::End => {
                self.cursor = self.input.len();
                self.dirty = true;
            }
            Key::Enter => self.submit(),
            Key::Newline => {
                self.history_index = None;
                self.insert_char('\n');
            }
            Key::Esc | Key::Ctrl('c') => self.interrupt(),
            Key::Ctrl('y') => self.copy_last(),
            Key::PageUp => self.scroll_up(self.page()),
            Key::PageDown => self.scroll_down(self.page()),
            Key::ScrollUp => self.scroll_up(WHEEL_LINES),
            Key::ScrollDown => self.scroll_down(WHEEL_LINES),
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
        // Expand pasted blocks back to their full text before submitting.
        let atoms = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.scroll = 0;
        self.dirty = true;
        let text: String = atoms.iter().map(Atom::text).collect();
        let text = text.trim().to_string();
        self.history_index = None;
        self.draft.clear();
        if text.is_empty() {
            return;
        }
        if text.starts_with('/') {
            self.command(&text);
            return;
        }
        if self.running {
            self.transcript
                .push(Block::Notice("a turn is already running — Esc to cancel".into()));
            return;
        }
        self.history.push(text.clone());
        self.transcript.push(Block::User(text.clone()));
        self.running = true;
        self.cancelled = false;
        // A new run starts a fresh changeset; the previous one is superseded.
        self.changes.clear();
        self.actions.push(Action::Submit(text));
    }

    /// Parse and act on a `/`-command typed at the prompt.
    fn command(&mut self, line: &str) {
        let mut parts = line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).filter(|s| !s.is_empty());
        self.notice(line);
        match name {
            "/exit" | "/quit" => self.should_quit = true,
            "/model" => match arg {
                Some(m) => self.actions.push(Action::Ask(Request::SetModel {
                    model: m.to_string(),
                })),
                None => self.open_model_picker(),
            },
            "/effort" => match arg {
                Some(level) => {
                    let effort = match level {
                        "-" | "none" | "off" => None,
                        _ => Some(level.to_string()),
                    };
                    self.actions.push(Action::Ask(Request::SetEffort { effort }));
                }
                None => self.notice("usage: /effort <level> ('-' clears)"),
            },
            "/compact" => self.actions.push(Action::Ask(Request::Compact {
                instructions: arg.map(str::to_string),
            })),
            "/usage" => self.actions.push(Action::Ask(Request::GetHistory)),
            "/changes" => self.open_changes_picker(),
            "/resume" => self.open_session_picker(arg),
            "/copy" => self.copy_last(),
            "/help" => self.notice(
                "commands: /exit /model <id> /effort [level] /compact [text] /changes /resume /usage /copy /help",
            ),
            other => self.notice(format!("unknown command: {other}")),
        }
        self.dirty = true;
    }
    /// Copy the last assistant reply to the terminal clipboard.
    fn copy_last(&mut self) {
        match self.last_assistant_text() {
            Some(text) => {
                let chars = text.chars().count();
                self.actions.push(Action::Copy(text));
                self.notice(format!("copied {chars} chars to the clipboard"));
            }
            None => self.notice("nothing to copy yet"),
        }
    }

    /// The text of the most recent assistant reply, if any.
    fn last_assistant_text(&self) -> Option<String> {
        self.transcript.iter().rev().find_map(|block| match block {
            Block::Assistant(content) => {
                let text: String = content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                (!text.trim().is_empty()).then_some(text)
            }
            _ => None,
        })
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.transcript.push(Block::Notice(text.into()));
        self.dirty = true;
    }

    /// Render a `GetHistory` reply as a one-line usage summary.
    fn render_usage(&mut self, messages: &[AgentMessage]) {
        let mut turns = 0u64;
        let (mut input, mut output, mut cache_read, mut cache_write) = (0u64, 0u64, 0u64, 0u64);
        let mut last_input = None;
        for message in messages {
            if let AgentMessage::Assistant {
                usage: Some(u), ..
            } = message
            {
                turns += 1;
                input += u.input_tokens;
                output += u.output_tokens;
                cache_read += u.cache_read_tokens.unwrap_or(0);
                cache_write += u.cache_write_tokens.unwrap_or(0);
                last_input = Some(u.input_tokens);
            }
        }
        if let Some(used) = last_input {
            self.context_used = Some(used);
        }
        if turns == 0 {
            self.notice("usage: no usage reported");
        } else {
            let mut parts = vec![
                format!("{turns} turn{}", if turns == 1 { "" } else { "s" }),
                format!("{input} in"),
                format!("{output} out"),
            ];
            if cache_read > 0 {
                parts.push(format!("{cache_read} cache read"));
            }
            if cache_write > 0 {
                parts.push(format!("{cache_write} cache write"));
            }
            if let (Some(used), Some(limit)) = (last_input, self.status.context_limit) {
                parts.push(format!("context {used}/{limit}"));
            }
            self.notice(format!("usage: {}", parts.join(", ")));
        }
        self.dirty = true;
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
                    diff: None,
                    path: None,
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
                output,
                is_error,
                diff,
                path,
                ..
            } => {
                if let Some(Block::Tool(tool)) = self.transcript.last_mut() {
                    if !output.is_empty() {
                        tool.output = output;
                    }
                    tool.done = true;
                    tool.is_error = is_error;
                    tool.path = path.clone();
                    tool.diff = diff.clone();
                    self.dirty = true;
                }
                // A mutating tool's UI-only (path, diff) pair feeds the run's
                // changeset — recorded even if no block matched the call.
                if let (Some(path), Some(diff)) = (path, diff) {
                    self.changes.push(Change::new(path, diff));
                }
            }
            AgentEvent::AgentEnd => {
                self.flush_live();
                self.running = false;
                if self.cancelled {
                    self.cancelled = false;
                    self.transcript.push(Block::Notice("⏹ aborted".into()));
                }
                // Surface the run's changes once it settles; they stay for `/changes`.
                if !self.changes.is_empty() {
                    self.transcript.push(Block::Notice(self.changes_summary()));
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
            AgentEvent::TurnEnd { message } => self.record_usage(&message),
            AgentEvent::History { messages } => self.render_usage(&messages),
            // Start and non-streamed replies need no state.
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

    /// Record the context usage the provider reported for a finished turn.
    fn record_usage(&mut self, message: &AgentMessage) {
        if let AgentMessage::Assistant { usage: Some(u), .. } = message {
            self.context_used = Some(u.input_tokens);
            self.dirty = true;
        }
    }

    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, Atom::Char(c));
        self.cursor += 1;
        self.dirty = true;
    }

    /// Insert `text` literally, one `Char` atom per character (small pastes).
    fn insert_str(&mut self, text: &str) {
        for c in text.chars() {
            self.input.insert(self.cursor, Atom::Char(c));
            self.cursor += 1;
        }
        self.dirty = true;
    }

    /// Handle a paste: a large blob becomes one atomic chip, a small one is
    /// inserted inline. `lines()` counts logical lines, so a trailing newline
    /// does not add an empty line.
    fn on_paste(&mut self, text: String) {
        let lines = text.lines().count();
        let chars = text.chars().count();
        if chars > 100 || lines > 3 {
            self.paste_id += 1;
            let id = self.paste_id;
            self.input
                .insert(self.cursor, Atom::Paste(PasteBlock { id, text, lines, chars }));
            self.cursor += 1;
            self.dirty = true;
        } else {
            self.insert_str(&text);
        }
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.input.remove(self.cursor - 1);
        self.cursor -= 1;
        self.dirty = true;
    }

    /// Forward-delete: remove the atom at the cursor (a no-op at the end).
    fn delete(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
            self.dirty = true;
        }
    }

    /// The buffer expanded to plain text — what a submit sends.
    fn expanded(&self) -> String {
        self.input.iter().map(Atom::text).collect()
    }

    /// Wrap a plain string as all-`Char` atoms (history recall and the draft).
    fn atoms(text: &str) -> Vec<Atom> {
        text.chars().map(Atom::Char).collect()
    }

    /// Recall the previous prompt (saving the draft on the way up).
    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.history_index {
            None => {
                self.draft = self.expanded();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_index = Some(next);
        let entry = self.history[next].clone();
        self.input = Self::atoms(&entry);
        self.cursor = self.input.len();
        self.dirty = true;
    }

    /// Recall the next prompt, or restore the draft at the bottom.
    fn history_down(&mut self) {
        let Some(i) = self.history_index else {
            return;
        };
        if i + 1 < self.history.len() {
            self.history_index = Some(i + 1);
            let entry = self.history[i + 1].clone();
            self.input = Self::atoms(&entry);
        } else {
            self.history_index = None;
            let draft = std::mem::take(&mut self.draft);
            self.input = Self::atoms(&draft);
        }
        self.cursor = self.input.len();
        self.dirty = true;
    }

    /// Seed the prompt history (oldest first), e.g. loaded from disk.
    pub fn load_history(&mut self, lines: Vec<String>) {
        self.history = lines;
    }

    /// The prompt history, oldest first, for persistence.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn transcript(&self) -> &[Block] {
        &self.transcript
    }

    pub fn live(&self) -> Option<&AgentMessage> {
        self.live.as_ref()
    }

    /// The buffer expanded to plain text (paste blocks inlined) — what is sent.
    pub fn input(&self) -> String {
        self.expanded()
    }

    /// A render-ready view of the buffer: display text, cursor column, chip ranges.
    pub(crate) fn input_view(&self) -> InputView {
        let mut display = String::new();
        let mut chips = Vec::new();
        let mut cursor_col = 0;
        let mut len = 0;
        for (i, atom) in self.input.iter().enumerate() {
            let s = atom.display();
            let n = s.chars().count();
            if i < self.cursor {
                cursor_col += n;
            }
            if matches!(atom, Atom::Paste(_)) {
                chips.push(len..len + n);
            }
            display.push_str(&s);
            len += n;
        }
        InputView { display, cursor_col, chips }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Provider-reported input tokens of the most recent turn, if any.
    pub fn context_used(&self) -> Option<u64> {
        self.context_used
    }

    /// A full page of transcript lines, for PgUp/PgDn.
    fn page(&self) -> usize {
        self.viewport.saturating_sub(1).max(1)
    }

    /// Scroll up by `lines`, clamped to the top of the transcript.
    fn scroll_up(&mut self, lines: usize) {
        self.scroll = (self.scroll + lines).min(self.max_scroll);
        self.dirty = true;
    }

    /// Scroll down by `lines`; `0` is the tail.
    fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
        self.dirty = true;
    }

    /// Lines scrolled up from the tail (`0` = following).
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Reconcile scroll state with the transcript the renderer just measured:
    /// keep the view pinned while content grows, then clamp to the top.
    pub fn sync_scroll(&mut self, total: usize, height: usize, width: usize) {
        // Pin the view while content grows — but not across a resize, which
        // re-wraps everything and moves every line.
        if self.scroll > 0 && width == self.last_width {
            self.scroll = self.scroll.saturating_add(total.saturating_sub(self.last_total));
        }
        let max = total.saturating_sub(height);
        self.scroll = self.scroll.min(max);
        self.max_scroll = max;
        self.viewport = height;
        self.last_total = total;
        self.last_width = width;
    }

    /// Open the `/changes` picker over the files this run changed.
    fn open_changes_picker(&mut self) {
        if self.changes.is_empty() {
            self.notice("no changes this run");
            return;
        }
        let mut items = Vec::new();
        let mut values = Vec::new();
        for (path, added, removed) in self.changes_by_path() {
            items.push(format!("{path} · +{added} −{removed}"));
            values.push(path);
        }
        self.overlay = Some(Overlay::Pick(Picker::with_values(
            PickerKind::Change,
            "changes",
            items,
            values,
        )));
        self.dirty = true;
    }

    /// Re-show a changed file's diff from this run in the transcript.
    fn show_change(&mut self, path: &str) {
        let diff: String = self
            .changes
            .iter()
            .filter(|c| c.path == path)
            .map(|c| c.diff.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if diff.is_empty() {
            return;
        }
        self.transcript.push(Block::Diff {
            path: path.to_string(),
            diff,
        });
        self.dirty = true;
    }

    /// Distinct changed paths in first-seen order, with their summed stats.
    fn changes_by_path(&self) -> Vec<(String, usize, usize)> {
        let mut out: Vec<(String, usize, usize)> = Vec::new();
        for change in &self.changes {
            match out.iter_mut().find(|(p, _, _)| *p == change.path) {
                Some((_, added, removed)) => {
                    *added += change.added;
                    *removed += change.removed;
                }
                None => out.push((change.path.clone(), change.added, change.removed)),
            }
        }
        out
    }

    /// A one-line summary of the run's changes, e.g. `⋯ 2 files changed · +9 −3`.
    fn changes_summary(&self) -> String {
        let by_path = self.changes_by_path();
        let added: usize = by_path.iter().map(|(_, a, _)| a).sum();
        let removed: usize = by_path.iter().map(|(_, _, r)| r).sum();
        let files = by_path.len();
        format!(
            "⋯ {files} file{} changed · +{added} −{removed} · /changes",
            if files == 1 { "" } else { "s" }
        )
    }

    /// Open the `/resume` picker over the injected session list. An argument
    /// pre-fills the filter (a quick way to jump to a known id).
    fn open_session_picker(&mut self, filter: Option<&str>) {
        if self.sessions.is_empty() {
            self.notice("no sessions to resume");
            return;
        }
        let items = self.sessions.iter().map(|s| s.label.clone()).collect();
        let values = self
            .sessions
            .iter()
            .map(|s| s.path.display().to_string())
            .collect();
        let mut picker = Picker::with_values(PickerKind::Resume, "resume", items, values);
        if let Some(filter) = filter {
            picker.query = filter.to_string();
        }
        self.overlay = Some(Overlay::Pick(picker));
        self.dirty = true;
    }

    /// Open the model picker over the injected model list.
    fn open_model_picker(&mut self) {
        if self.models.is_empty() {
            self.notice("no models available to pick");
            return;
        }
        self.overlay = Some(Overlay::Pick(Picker::new(
            PickerKind::Model,
            "model",
            self.models.clone(),
        )));
        self.dirty = true;
    }

    /// Keys while a modal is open: ↑/↓ move, printable chars filter, Backspace
    /// deletes, Enter selects, Esc dismisses. Everything else is swallowed.
    fn on_overlay_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.overlay = None,
            Key::Up => self.map_picker(|p| p.move_selection(-1)),
            Key::Down => self.map_picker(|p| p.move_selection(1)),
            Key::Backspace => self.map_picker(|p| {
                p.query.pop();
                p.selected = 0;
            }),
            Key::Char(c) => self.map_picker(|p| {
                p.query.push(c);
                p.selected = 0;
            }),
            Key::Enter => self.accept_picker(),
            _ => {}
        }
        self.dirty = true;
    }

    /// Run `f` against the open picker, if any.
    fn map_picker(&mut self, f: impl FnOnce(&mut Picker)) {
        if let Some(Overlay::Pick(picker)) = self.overlay.as_mut() {
            f(picker);
        }
    }

    /// Apply the highlighted row and dismiss the modal.
    fn accept_picker(&mut self) {
        let Some(Overlay::Pick(picker)) = self.overlay.take() else {
            return;
        };
        let Some(selected) = picker.selected_value().map(str::to_string) else {
            self.notice("no match to select");
            return;
        };
        match picker.kind {
            PickerKind::Model => {
                self.status.model.clone_from(&selected);
                self.notice(format!("model: {selected}"));
                self.actions
                    .push(Action::Ask(Request::SetModel { model: selected }));
            }
            PickerKind::Change => self.show_change(&selected),
            PickerKind::Resume => {
                // Hand the choice back and quit; the composition root re-execs
                // with `--resume <path>` — a client cannot rebuild the agent.
                self.pending_resume = Some(PathBuf::from(selected));
                self.should_quit = true;
            }
        }
    }

    /// The open modal, if any (read by the renderer).
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    /// Seed the session list the `/resume` picker offers.
    pub fn set_sessions(&mut self, sessions: Vec<SessionItem>) {
        self.sessions = sessions;
    }

    /// The session the user chose to resume, if any — read by the event loop
    /// after it exits so the composition root can re-exec with `--resume`.
    pub fn pending_resume(&self) -> Option<&Path> {
        self.pending_resume.as_deref()
    }

    /// Seed the model list the picker offers (e.g. from `list_models`).
    pub fn set_models(&mut self, models: Vec<String>) {
        self.models = models;
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

    /// Seed the transcript with the conversation so far — what a resumed
    /// session (or a reconnecting socket client) already has. The event loop
    /// calls this once at startup, before any live event, so the earlier turns
    /// are readable (and scrollable) instead of missing.
    pub fn seed_history(&mut self, messages: &[AgentMessage]) {
        if messages.is_empty() {
            return;
        }
        let start = self.transcript.len();
        for message in messages {
            match message {
                AgentMessage::User { .. } => {
                    let text = message.as_text();
                    if !text.trim().is_empty() {
                        self.transcript.push(Block::User(text));
                    }
                }
                AgentMessage::Assistant { content, .. } => {
                    // As when committing live: tool calls get their own line.
                    let visible: Vec<ContentBlock> = content
                        .iter()
                        .filter(|b| !matches!(b, ContentBlock::ToolCall { .. }))
                        .cloned()
                        .collect();
                    if !visible.is_empty() {
                        self.transcript.push(Block::Assistant(visible));
                    }
                    self.record_usage(message);
                }
                AgentMessage::ToolResult {
                    name,
                    output,
                    is_error,
                    ..
                } => {
                    self.transcript.push(Block::Tool(Tool {
                        name: name.clone(),
                        output: output.clone(),
                        done: true,
                        is_error: *is_error,
                        diff: None,
                        path: None,
                    }));
                }
            }
        }
        // Mark where the replayed prefix ends, mirroring the REPL's divider.
        self.transcript.insert(
            start,
            Block::Notice(format!("⋯ {} earlier message(s)", messages.len())),
        );
        self.dirty = true;
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
    fn delete_removes_the_char_at_the_cursor() {
        let mut app = App::new();
        typed(&mut app, "abc");
        app.handle(AppEvent::Key(Key::Left));
        app.handle(AppEvent::Key(Key::Delete));
        assert_eq!(app.input(), "ab");
        assert_eq!(app.cursor(), 2);
        // At the end of the buffer, Delete is a no-op.
        app.handle(AppEvent::Key(Key::Delete));
        assert_eq!(app.input(), "ab");
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
    fn a_small_paste_inserts_inline() {
        let mut app = App::new();
        app.handle(AppEvent::Paste("hi there".into()));
        assert_eq!(app.input(), "hi there");
        assert_eq!(app.cursor(), 8);
        assert!(app.input_view().chips.is_empty());
    }

    #[test]
    fn a_large_paste_becomes_one_chip() {
        let mut app = App::new();
        let blob = "one\ntwo\nthree\nfour"; // 4 lines ⇒ a chip
        app.handle(AppEvent::Paste(blob.into()));
        let view = app.input_view();
        assert_eq!(view.chips.len(), 1);
        assert!(view.display.contains("pasted 4 lines"), "chip: {}", view.display);
        assert!(!view.display.contains("two"), "raw text leaked: {}", view.display);
        // The buffer still expands to the full pasted text.
        assert_eq!(app.input(), blob);
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn submitting_a_chip_sends_the_full_text() {
        let mut app = App::new();
        let blob = "one\ntwo\nthree\nfour";
        app.handle(AppEvent::Paste(blob.into()));
        // The paste really became a chip (not raw text), so the submit can only
        // carry the expanded full text.
        assert_eq!(app.input_view().chips.len(), 1);
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.transcript()[0], Block::User(blob.into()));
        assert_eq!(app.history().first().map(String::as_str), Some(blob));
        assert_eq!(app.take_actions(), vec![Action::Submit(blob.into())]);
        assert_eq!(app.input(), "");
        assert_eq!(app.cursor(), 0);
    }

    #[test]
    fn backspace_removes_a_whole_chip_and_arrows_step_over_it() {
        let mut app = App::new();
        app.handle(AppEvent::Paste("a\nb\nc\nd".into())); // 4 lines ⇒ a chip
        assert_eq!(app.cursor(), 1);
        app.handle(AppEvent::Key(Key::Left));
        assert_eq!(app.cursor(), 0);
        app.handle(AppEvent::Key(Key::Right));
        assert_eq!(app.cursor(), 1);
        app.handle(AppEvent::Key(Key::Right)); // already at the end
        assert_eq!(app.cursor(), 1);
        app.handle(AppEvent::Key(Key::Backspace));
        assert_eq!(app.cursor(), 0);
        assert_eq!(app.input(), "");
    }

    #[test]
    fn two_pastes_make_two_chips() {
        let mut app = App::new();
        app.handle(AppEvent::Paste("a\nb\nc\nd".into()));
        app.handle(AppEvent::Paste("w\nx\ny\nz".into()));
        assert_eq!(app.cursor(), 2);
        let ids: Vec<u64> = app
            .input
            .iter()
            .filter_map(|a| match a {
                Atom::Paste(p) => Some(p.id),
                Atom::Char(_) => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn paste_is_ignored_while_an_overlay_is_open() {
        let mut app = App::new();
        app.set_models(vec!["m".into()]);
        submit(&mut app, "/model");
        let _ = app.take_actions();
        app.handle(AppEvent::Paste("a\nb\nc\nd".into()));
        assert_eq!(app.input(), "", "the paste must not reach the gated input");
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
            diff: None,
            path: None,
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
    fn a_turn_end_records_context_usage() {
        let mut app = App::new();
        let message = AgentMessage::Assistant {
            content: vec![ContentBlock::Text { text: "hi".into() }],
            stop_reason: StopReason::Stop,
            usage: Some(wcode_harness::message::Usage {
                input_tokens: 14_200,
                output_tokens: 30,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            model: None,
        };
        app.handle(AppEvent::Agent(AgentEvent::TurnEnd { message }));
        assert_eq!(app.context_used(), Some(14_200));
    }

    #[test]
    fn scrolling_clamps_and_stays_pinned_while_content_grows() {
        let mut app = App::new();
        app.sync_scroll(100, 10, 80); // 100 lines in a 10-high viewport: max 90
        app.handle(AppEvent::Key(Key::PageUp));
        assert_eq!(app.scroll(), 9); // one page (height - 1)

        // Scrolled up: growing the transcript keeps the same lines in view.
        app.sync_scroll(110, 10, 80);
        assert_eq!(app.scroll(), 19);

        // PageDown past the bottom lands at 0.
        for _ in 0..20 {
            app.handle(AppEvent::Key(Key::PageDown));
        }
        assert_eq!(app.scroll(), 0);
    }

    #[test]
    fn seeding_replays_the_conversation_so_far() {
        let mut app = App::new();
        app.seed_history(&[
            AgentMessage::user_text("earlier question"),
            AgentMessage::Assistant {
                content: vec![
                    ContentBlock::Text {
                        text: "earlier answer".into(),
                    },
                    ContentBlock::ToolCall {
                        id: "t1".into(),
                        name: "read".into(),
                        arguments: Default::default(),
                    },
                ],
                stop_reason: StopReason::ToolUse,
                usage: Some(wcode_harness::message::Usage {
                    input_tokens: 700,
                    output_tokens: 5,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                }),
                model: None,
            },
            AgentMessage::ToolResult {
                tool_call_id: "t1".into(),
                name: "read".into(),
                output: "128 lines".into(),
                is_error: false,
            },
        ]);

        // A divider marks the replayed prefix, as the REPL's does.
        assert!(matches!(&app.transcript()[0], Block::Notice(t) if t.contains("3 earlier message")));
        assert_eq!(app.transcript()[1], Block::User("earlier question".into()));
        // Dropped tool call, kept prose — same shape as a live commit.
        assert_eq!(
            app.transcript()[2],
            Block::Assistant(vec![ContentBlock::Text {
                text: "earlier answer".into()
            }])
        );
        // The tool result keeps the one-line summary form.
        assert!(
            matches!(&app.transcript()[3], Block::Tool(t)
                if t.done && !t.is_error && t.name == "read" && t.output == "128 lines")
        );
        // The resumed context fill reads like a live turn's.
        assert_eq!(app.context_used(), Some(700));
        assert!(app.dirty());
    }

    #[test]
    fn seeding_an_empty_history_changes_nothing() {
        let mut app = App::new();
        app.clear_dirty();
        app.seed_history(&[]);
        assert!(app.transcript().is_empty());
        assert!(!app.dirty());
    }

    #[test]
    fn the_wheel_scrolls_a_few_lines_and_clamps_at_both_ends() {
        let mut app = App::new();
        app.sync_scroll(100, 10, 80); // max 90
        app.clear_dirty();

        app.handle(AppEvent::Key(Key::ScrollUp));
        assert_eq!(app.scroll(), WHEEL_LINES);
        assert!(app.dirty());

        app.handle(AppEvent::Key(Key::ScrollDown));
        assert_eq!(app.scroll(), 0);

        // The top and the tail are hard stops, not wraps.
        for _ in 0..50 {
            app.handle(AppEvent::Key(Key::ScrollUp));
        }
        assert_eq!(app.scroll(), 90);
        for _ in 0..50 {
            app.handle(AppEvent::Key(Key::ScrollDown));
        }
        assert_eq!(app.scroll(), 0);
    }

    #[test]
    fn a_resize_does_not_pin_the_view() {
        let mut app = App::new();
        app.sync_scroll(100, 10, 80);
        app.handle(AppEvent::Key(Key::PageUp));
        assert_eq!(app.scroll(), 9);

        // A width change re-wraps everything: the offset is kept, not extended.
        app.sync_scroll(120, 10, 60);
        assert_eq!(app.scroll(), 9);
    }

    #[test]
    fn submitting_returns_to_the_tail() {
        let mut app = App::new();
        app.sync_scroll(100, 10, 80);
        app.handle(AppEvent::Key(Key::PageUp));
        assert_eq!(app.scroll(), 9);
        submit(&mut app, "go");
        assert_eq!(app.scroll(), 0);
    }

    #[test]
    fn slash_commands_emit_actions() {
        let mut app = App::new();
        submit(&mut app, "/model gpt-x");
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SetModel {
                model: "gpt-x".into()
            })]
        );

        submit(&mut app, "/effort -");
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SetEffort { effort: None })]
        );

        submit(&mut app, "/usage");
        assert_eq!(app.take_actions(), vec![Action::Ask(Request::GetHistory)]);

        submit(&mut app, "/nonsense");
        assert!(app.take_actions().is_empty());
    }

    #[test]
    fn exit_command_quits_without_an_action() {
        let mut app = App::new();
        submit(&mut app, "/exit");
        assert!(app.should_quit());
        assert!(app.take_actions().is_empty());
    }

    #[test]
    fn a_history_reply_updates_usage() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(AgentEvent::History {
            messages: vec![AgentMessage::Assistant {
                content: vec![ContentBlock::Text { text: "x".into() }],
                stop_reason: StopReason::Stop,
                usage: Some(wcode_harness::message::Usage {
                    input_tokens: 500,
                    output_tokens: 20,
                    cache_read_tokens: Some(3),
                    cache_write_tokens: None,
                }),
                model: None,
            }],
        }));
        assert_eq!(app.context_used(), Some(500));
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(text)) if text.contains("1 turn")
        ));
    }

    #[test]
    fn history_recalls_and_restores_the_draft() {
        let mut app = App::new();
        submit(&mut app, "first");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(AgentEvent::AgentEnd)); // the run finished
        submit(&mut app, "second");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(AgentEvent::AgentEnd));

        typed(&mut app, "drafty");
        app.handle(AppEvent::Key(Key::Up));
        assert_eq!(app.input(), "second");
        app.handle(AppEvent::Key(Key::Up));
        assert_eq!(app.input(), "first");
        app.handle(AppEvent::Key(Key::Up)); // oldest: stays put
        assert_eq!(app.input(), "first");
        app.handle(AppEvent::Key(Key::Down));
        assert_eq!(app.input(), "second");
        app.handle(AppEvent::Key(Key::Down));
        assert_eq!(app.input(), "drafty");
    }

    #[test]
    fn shift_enter_inserts_a_newline() {
        let mut app = App::new();
        typed(&mut app, "a");
        app.handle(AppEvent::Key(Key::Newline));
        typed(&mut app, "b");
        assert_eq!(app.input(), "a\nb");

        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.transcript()[0], Block::User("a\nb".into()));
    }

    #[test]
    fn copy_emits_the_last_reply() {
        let mut app = App::new();
        submit(&mut app, "hi");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(AgentEvent::MessageStart {
            message: assistant(""),
        }));
        app.handle(AppEvent::Agent(AgentEvent::MessageEnd {
            message: assistant("the answer"),
        }));
        app.handle(AppEvent::Agent(AgentEvent::AgentEnd));
        let _ = app.take_actions();

        submit(&mut app, "/copy");
        assert_eq!(app.take_actions(), vec![Action::Copy("the answer".into())]);
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("copied")
        ));
    }

    #[test]
    fn copy_with_nothing_says_so() {
        let mut app = App::new();
        submit(&mut app, "/copy");
        assert!(app.take_actions().is_empty());
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("nothing to copy")
        ));
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

    #[test]
    fn model_without_an_argument_opens_the_picker() {
        let mut app = App::new();
        app.set_models(vec!["gpt-4o".into(), "gpt-4o-mini".into()]);
        submit(&mut app, "/model");
        assert!(matches!(app.overlay(), Some(Overlay::Pick(p)) if p.title == "model"));
        // Opening a picker emits nothing; the selection does.
        assert!(app.take_actions().is_empty());
    }

    #[test]
    fn the_picker_navigates_and_selects() {
        let mut app = App::new();
        app.set_models(vec!["alpha".into(), "beta".into(), "gamma".into()]);
        submit(&mut app, "/model");
        let _ = app.take_actions();

        app.handle(AppEvent::Key(Key::Down));
        app.handle(AppEvent::Key(Key::Enter));
        assert!(app.overlay().is_none(), "selecting closes the modal");
        assert_eq!(app.status().model, "beta");
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SetModel {
                model: "beta".into()
            })]
        );
    }

    #[test]
    fn the_picker_filters_then_selects_the_only_match() {
        let mut app = App::new();
        app.set_models(vec!["alpha".into(), "beta".into()]);
        submit(&mut app, "/model");
        let _ = app.take_actions();

        for c in "bet".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SetModel {
                model: "beta".into()
            })]
        );
    }

    #[test]
    fn esc_dismisses_the_picker_without_selecting() {
        let mut app = App::new();
        app.set_models(vec!["alpha".into()]);
        submit(&mut app, "/model");
        let _ = app.take_actions();

        app.handle(AppEvent::Key(Key::Esc));
        assert!(app.overlay().is_none());
        assert!(app.take_actions().is_empty());
        assert!(!app.should_quit(), "Esc closes the modal, it does not quit");
    }

    #[test]
    fn keys_do_not_reach_the_input_while_a_modal_is_open() {
        let mut app = App::new();
        app.set_models(vec!["alpha".into()]);
        submit(&mut app, "/model");
        for c in "xyz".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        assert_eq!(app.input(), "", "the input stayed untouched");
    }

    #[test]
    fn model_without_an_argument_and_no_list_says_so() {
        let mut app = App::new();
        submit(&mut app, "/model");
        assert!(app.overlay().is_none());
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("no models")
        ));
    }

    #[test]
    fn tool_end_captures_the_ui_only_diff() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(AgentEvent::ToolExecutionStart {
            call_id: "t1".into(),
            name: "edit".into(),
        }));
        app.handle(AppEvent::Agent(AgentEvent::ToolExecutionEnd {
            call_id: "t1".into(),
            name: "edit".into(),
            output: "edited f (lines 1)".into(),
            is_error: false,
            diff: Some("@@ -1 +1 @@\n-old\n+new".into()),
            path: Some("f.rs".into()),
        }));
        match app.transcript().last() {
            Some(Block::Tool(tool)) => {
                assert_eq!(tool.diff.as_deref(), Some("@@ -1 +1 @@\n-old\n+new"));
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    /// Drive a tool lifecycle whose end carries the UI-only (path, diff) pair.
    fn tool_end(app: &mut App, name: &str, path: Option<&str>, diff: Option<&str>) {
        app.handle(AppEvent::Agent(AgentEvent::ToolExecutionStart {
            call_id: "t1".into(),
            name: name.into(),
        }));
        app.handle(AppEvent::Agent(AgentEvent::ToolExecutionEnd {
            call_id: "t1".into(),
            name: name.into(),
            output: "ok".into(),
            is_error: false,
            diff: diff.map(str::to_string),
            path: path.map(str::to_string),
        }));
    }

    #[test]
    fn a_mutating_tool_records_a_change_and_a_new_run_clears_it() {
        let mut app = App::new();
        submit(&mut app, "edit things");
        let _ = app.take_actions();
        tool_end(&mut app, "edit", Some("src/a.rs"), Some("@@ -1 +1 @@\n-old\n+new"));

        assert_eq!(app.changes.len(), 1);
        assert_eq!(app.changes[0].path, "src/a.rs");
        assert_eq!((app.changes[0].added, app.changes[0].removed), (1, 1));
        app.handle(AppEvent::Agent(AgentEvent::AgentEnd));

        // A new prompt starts a fresh changeset; the settled one is superseded.
        submit(&mut app, "again");
        assert!(app.changes.is_empty());
    }

    #[test]
    fn a_tool_without_a_path_or_diff_is_not_a_change() {
        let mut app = App::new();
        submit(&mut app, "read");
        let _ = app.take_actions();
        tool_end(&mut app, "read", None, None);
        assert!(app.changes.is_empty());
    }

    #[test]
    fn the_changeset_summary_appears_when_the_run_settles() {
        let mut app = App::new();
        submit(&mut app, "edit");
        let _ = app.take_actions();
        tool_end(&mut app, "edit", Some("a.rs"), Some("@@ -1 +1 @@\n-old\n+new"));
        tool_end(&mut app, "write", Some("b.rs"), Some("@@ -0,0 +1 @@\n+new"));
        app.handle(AppEvent::Agent(AgentEvent::AgentEnd));

        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("2 files changed") && t.contains("+2 −1")
        ));
    }

    #[test]
    fn the_tool_block_carries_the_changed_path() {
        let mut app = App::new();
        tool_end(&mut app, "edit", Some("src/a.rs"), Some("@@ -1 +1 @@\n-old\n+new"));
        match app.transcript().last() {
            Some(Block::Tool(tool)) => assert_eq!(tool.path.as_deref(), Some("src/a.rs")),
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[test]
    fn changes_command_opens_a_picker_and_selection_shows_the_diff() {
        let mut app = App::new();
        submit(&mut app, "edit");
        let _ = app.take_actions();
        tool_end(&mut app, "edit", Some("src/a.rs"), Some("@@ -1 +1 @@\n-old\n+new"));
        app.handle(AppEvent::Agent(AgentEvent::AgentEnd));

        submit(&mut app, "/changes");
        assert!(matches!(app.overlay(), Some(Overlay::Pick(p)) if p.title == "changes"));
        app.handle(AppEvent::Key(Key::Enter));

        assert!(app.overlay().is_none(), "selecting closes the modal");
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Diff { path, .. }) if path == "src/a.rs"
        ));
    }

    #[test]
    fn changes_with_nothing_says_so() {
        let mut app = App::new();
        submit(&mut app, "/changes");
        assert!(app.overlay().is_none());
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("no changes")
        ));
    }

    fn sessions() -> Vec<SessionItem> {
        vec![
            SessionItem {
                label: "a1b2 · 5m · hello".into(),
                path: PathBuf::from("/s/a1b2.jsonl"),
            },
            SessionItem {
                label: "c3d4 · 1h · world".into(),
                path: PathBuf::from("/s/c3d4.jsonl"),
            },
        ]
    }

    #[test]
    fn resume_opens_the_picker_and_selection_hands_back_the_path() {
        let mut app = App::new();
        app.set_sessions(sessions());
        submit(&mut app, "/resume");
        assert!(matches!(app.overlay(), Some(Overlay::Pick(p)) if p.title == "resume"));

        app.handle(AppEvent::Key(Key::Down));
        app.handle(AppEvent::Key(Key::Enter));
        assert!(app.overlay().is_none(), "selecting closes the modal");
        assert_eq!(app.pending_resume(), Some(Path::new("/s/c3d4.jsonl")));
        assert!(app.should_quit(), "a resume selection quits the TUI");
    }

    #[test]
    fn the_resume_filter_prefills_the_query() {
        let mut app = App::new();
        app.set_sessions(sessions());
        submit(&mut app, "/resume c3d4");
        // The filter narrows to one row, so Enter selects it directly.
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.pending_resume(), Some(Path::new("/s/c3d4.jsonl")));
    }

    #[test]
    fn resume_with_no_sessions_says_so() {
        let mut app = App::new();
        submit(&mut app, "/resume");
        assert!(app.overlay().is_none());
        assert!(!app.should_quit());
        assert_eq!(app.pending_resume(), None);
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("no sessions")
        ));
    }
}
