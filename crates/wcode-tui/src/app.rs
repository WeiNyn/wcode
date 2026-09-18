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
use wcode_harness::protocol::{Request, SessionId};

use crate::{SurfaceInfo, TeamState};

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
    /// Tab: accept the highlighted inline command completion.
    Tab,
    Enter,
    /// Shift-Enter: insert a newline instead of submitting.
    Newline,
    Esc,
    /// A control chord, e.g. `Ctrl('c')` for Ctrl-C.
    Ctrl(char),
    /// An Alt chord, e.g. `Alt('3')` for Alt-3. An Alt chord never inserts the
    /// character it carries.
    Alt(char),
    /// A function key, e.g. `F(1)` for F1.
    F(u8),
    /// Shift-Tab: focus the previous surface (`Tab` still accepts completion).
    BackTab,
}

/// Everything the app can react to, from any source.
#[derive(Clone, Debug)]
pub enum AppEvent {
    Key(Key),
    Paste(String),
    /// A fact from a session, tagged with the surface it belongs to (an
    /// unmatched id is ignored).
    Agent(SessionId, AgentEvent),
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
    /// Whether the transcript shows the full output rather than the collapsed
    /// preview. Toggled with `Ctrl-O` (all tools with `Ctrl-T`); forced on when
    /// the tool errored, so a failure is never hidden.
    pub expanded: bool,
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
    /// The `F1` keymap. A pure modal: it dismisses only on `Esc` / `F1`.
    Help,
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
    /// A surface (`/surface`); selecting focuses it.
    Surface,
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
    pub fn rows(&self) -> Vec<(&str, Option<Range<usize>>)> {
        matching_indices(&self.items, &self.query)
            .into_iter()
            .map(|i| {
                (
                    self.items[i].as_str(),
                    match_range(&self.items[i], &self.query),
                )
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
        matching_indices(&self.items, &self.query)
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

/// One `/`-command the TUI understands. [`COMMANDS`] is the single source of
/// truth for dispatch, the generated `/help`, and inline completion. It is
/// deliberately TUI-local: the REPL keeps its own parser, since the two command
/// sets legitimately differ (see `docs/team-and-tui-plan.md`, decision D8).
struct Command {
    /// Canonical name, without the leading slash.
    name: &'static str,
    /// Accepted alternative spellings (also slash-less).
    aliases: &'static [&'static str],
    /// Argument hint for `/help` and the completion popup, e.g. `"<id>"`.
    args: Option<&'static str>,
    /// One-line description shown in the completion popup.
    summary: &'static str,
}

/// The command table — its order is the `/help` listing.
const COMMANDS: &[Command] = &[
    Command {
        name: "exit",
        aliases: &["quit"],
        args: None,
        summary: "quit wcode",
    },
    Command {
        name: "model",
        aliases: &[],
        args: Some("<id>"),
        summary: "switch model (no arg: pick)",
    },
    Command {
        name: "effort",
        aliases: &[],
        args: Some("[level]"),
        summary: "set reasoning effort ('-' clears)",
    },
    Command {
        name: "compact",
        aliases: &[],
        args: Some("[text]"),
        summary: "summarize older context",
    },
    Command {
        name: "changes",
        aliases: &[],
        args: None,
        summary: "files changed this run",
    },
    Command {
        name: "resume",
        aliases: &["sessions"],
        args: None,
        summary: "resume a saved session",
    },
    Command {
        name: "usage",
        aliases: &[],
        args: None,
        summary: "token usage so far",
    },
    Command {
        name: "copy",
        aliases: &[],
        args: None,
        summary: "copy the last reply",
    },
    Command {
        name: "surface",
        aliases: &[],
        args: None,
        summary: "switch surface",
    },
    Command {
        name: "team",
        aliases: &[],
        args: None,
        summary: "list the team",
    },
    Command {
        name: "help",
        aliases: &[],
        args: None,
        summary: "list commands",
    },
];

/// The inline `/`-command completion popup. Non-modal: it never owns the
/// keyboard, so the user keeps typing arguments beneath it. `matches` indexes
/// [`COMMANDS`]; `selected` indexes `matches`.
#[derive(Clone, Debug, Default)]
struct Completion {
    pub matches: Vec<usize>,
    pub selected: usize,
}

/// One renderable completion row.
pub(crate) struct CompletionRow {
    pub(crate) label: String,
    pub(crate) range: Option<Range<usize>>,
    /// The alias that matched when the canonical name has no prefix (a hint).
    pub(crate) alias: Option<&'static str>,
    pub(crate) args: Option<&'static str>,
    pub(crate) summary: &'static str,
}

/// Look up a command by its typed name (canonical or alias). The leading slash
/// is optional.
fn command_named(token: &str) -> Option<&'static Command> {
    let name = token.strip_prefix('/').unwrap_or(token);
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

/// The `/team` listing: one `label · model · state` line per member surface.
fn team_text(rows: &[(&str, &str, TeamState, bool)]) -> String {
    if rows.is_empty() {
        return "(no team)".to_string();
    }
    rows.iter()
        .map(|(label, model, state, _)| format!("{label} · {model} · {}", state.label()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The keymap — the single source of truth for the `F1` help overlay and the
/// keys section of `/help`. Chord → what it does.
pub(crate) const KEYS: &[(&str, &str)] = &[
    ("Enter", "submit the prompt"),
    ("Shift-Enter / Ctrl-J", "insert a newline"),
    ("Up / Down", "recall prompt history"),
    ("Ctrl-A / Ctrl-E", "move to the start / end of the input"),
    ("Ctrl-W", "delete the previous word"),
    ("Ctrl-U", "delete to the start of the line"),
    ("Ctrl-K", "delete to the end of the line"),
    ("PgUp / PgDn", "scroll the transcript a page"),
    ("wheel", "scroll three lines"),
    ("Ctrl-O", "expand / collapse the last tool's output"),
    ("Ctrl-T", "expand / collapse all tool output"),
    ("Ctrl-N / Shift-Tab", "focus the next / previous surface"),
    ("Alt-1..9", "focus the Nth surface"),
    ("Ctrl-B", "toggle the team sidebar"),
    ("F1", "toggle this help"),
    ("Ctrl-Y", "copy the last reply"),
    ("Esc / Ctrl-C", "cancel a run; quit when idle"),
];

/// The generated `/help` text: the command list, then the keymap from [`KEYS`].
fn help_text() -> String {
    let mut out = String::from("commands:");
    for cmd in COMMANDS {
        out.push(' ');
        out.push('/');
        out.push_str(cmd.name);
        if let Some(args) = cmd.args {
            out.push(' ');
            out.push_str(args);
        }
    }
    out.push_str("\nkeys:");
    for (chord, what) in KEYS {
        out.push_str(&format!("\n  {chord} — {what}"));
    }
    out
}

/// `true` when `query` is already a complete command name *or alias* — nothing
/// left to complete, so Enter submits rather than completes.
fn is_exact_command(query: &str) -> bool {
    COMMANDS
        .iter()
        .any(|c| c.name == query || c.aliases.contains(&query))
}

/// Indices of the entries matching `query` (case-insensitive substring), in
/// order. An empty query matches everything. Used by the picker filter — the
/// completion popup uses [`prefix_indices`] instead.
fn matching_indices<S: AsRef<str>>(items: &[S], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| query.is_empty() || item.as_ref().to_lowercase().contains(&query))
        .map(|(i, _)| i)
        .collect()
}

/// Indices of the commands whose *name* — or any of whose aliases — starts with
/// `query` (case-insensitive). This is the completion popup's matcher: a leading
/// prefix, not the picker's mid-string [`matching_indices`]. An empty query
/// matches every name, so a bare `/` still lists them all.
fn prefix_indices(commands: &[Command], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    commands
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            c.name.to_lowercase().starts_with(&query)
                || c.aliases
                    .iter()
                    .any(|a| a.to_lowercase().starts_with(&query))
        })
        .map(|(i, _)| i)
        .collect()
}

/// The leading-prefix highlight range for a completion row, or `None` when the
/// query matched an alias rather than the canonical name (there is then no
/// prefix to light up in the label). `label` is `/` + `name`, hence the shift.
fn prefix_range(label: &str, name: &str, query: &str) -> Option<Range<usize>> {
    if query.is_empty() || !name.to_lowercase().starts_with(&query.to_lowercase()) {
        return None;
    }
    Some(0..(1 + query.len()).min(label.len()))
}

/// The first alias of `cmd` that starts with `query` (case-insensitive) — the
/// popup's dim "why did this match" hint when the canonical name has no prefix.
fn matched_alias(cmd: &Command, query: &str) -> Option<&'static str> {
    let query = query.to_lowercase();
    cmd.aliases
        .iter()
        .copied()
        .find(|a| a.to_lowercase().starts_with(&query))
}

/// The byte range of `query` within `item` for highlighting (case-insensitive),
/// or `None` when the query is empty or absent. Used by the picker; the
/// completion popup uses [`prefix_range`].
fn match_range(item: &str, query: &str) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let query = query.to_lowercase();
    item.to_lowercase()
        .find(&query)
        .map(|start| start..start + query.len())
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

    /// Whether this atom is a whitespace character. A paste block never is, so
    /// it reads as one "word" for `Ctrl-W`.
    fn is_space(&self) -> bool {
        matches!(self, Atom::Char(c) if c.is_whitespace())
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

/// One conversation's state: its identity (the id events route by), the
/// `label`/`model` the sidebar shows, the transcript, run state, changeset,
/// scroll pin, and prompt history. `App` holds a `Vec<Surface>`; index 0 is the
/// root, the rest are team members.
pub struct Surface {
    /// The id this surface's events route by.
    id: SessionId,
    /// A short display name (the sidebar + status line).
    label: String,
    /// The effective model id (the sidebar).
    model: String,
    /// The root surface (full status line); members are the team.
    is_root: bool,
    /// The status line (the root's; members get a reduced one).
    status: Status,
    transcript: Vec<Block>,
    /// The assistant message currently streaming (rendered below the transcript).
    live: Option<AgentMessage>,
    /// Files changed during the current run, in call order; reset when the next
    /// prompt starts a run, kept afterwards so the run stays reviewable.
    changes: Vec<Change>,
    running: bool,
    /// A run finished (drives the sidebar's `done`).
    finished: bool,
    /// A `Cancel` was sent; the next `AgentEnd` is rendered as an abort.
    cancelled: bool,
    /// Provider-reported input tokens of the last turn: how full the context was.
    context_used: Option<u64>,
    /// Lines scrolled up from the bottom; `0` follows the tail.
    scroll: usize,
    /// Clamp for [`Surface::scroll`], set by the renderer from the line count.
    max_scroll: usize,
    /// Transcript height and total line count from the last draw, so the view
    /// can stay pinned while new lines stream in.
    viewport: usize,
    last_total: usize,
    last_width: usize,
    /// Submitted prompts, oldest first, for Up/Down recall (per-surface, D25).
    history: Vec<String>,
    /// Index into `history` while browsing; `None` edits the draft.
    history_index: Option<usize>,
    /// The in-progress line, saved while browsing history.
    draft: String,
}

impl Surface {
    fn new(info: SurfaceInfo) -> Self {
        let status = if info.is_root {
            Status::default()
        } else {
            // A member shows its model and label, nothing else (D28).
            let mut status = Status::new(info.model.clone());
            status.session = Some(info.label.clone());
            status
        };
        Surface {
            id: info.id,
            label: info.label,
            model: info.model,
            is_root: info.is_root,
            status,
            transcript: Vec::new(),
            live: None,
            changes: Vec::new(),
            running: false,
            finished: false,
            cancelled: false,
            context_used: None,
            scroll: 0,
            max_scroll: 0,
            viewport: 0,
            last_total: 0,
            last_width: 0,
            history: Vec::new(),
            history_index: None,
            draft: String::new(),
        }
    }

    /// The sidebar/`/team` state, derived from the run flags.
    fn state(&self) -> TeamState {
        if self.running {
            TeamState::Running
        } else if self.finished {
            TeamState::Done
        } else {
            TeamState::Idle
        }
    }

    /// The most recent tool block, if the transcript has one — what `Ctrl-O`
    /// expands or collapses.
    fn last_tool_mut(&mut self) -> Option<&mut Tool> {
        self.transcript.iter_mut().rev().find_map(|block| match block {
            Block::Tool(tool) => Some(tool),
            _ => None,
        })
    }

    /// Apply one agent event. Returns `true` when the surface changed, so `App`
    /// can mark itself dirty (the surface owns no `dirty` flag).
    fn apply(&mut self, event: AgentEvent) -> bool {
        match event {
            AgentEvent::MessageStart { message } => {
                if matches!(message, AgentMessage::Assistant { .. }) {
                    self.live = Some(message);
                }
                true
            }
            AgentEvent::MessageUpdate { message } => {
                if self.live.is_some() {
                    self.live = Some(message);
                    true
                } else {
                    false
                }
            }
            AgentEvent::MessageEnd { message } => {
                self.live = None;
                self.commit(message);
                true
            }
            AgentEvent::ToolExecutionStart { name, .. } => {
                self.flush_live();
                self.transcript.push(Block::Tool(Tool {
                    name,
                    output: String::new(),
                    done: false,
                    is_error: false,
                    expanded: false,
                    diff: None,
                    path: None,
                }));
                true
            }
            AgentEvent::ToolExecutionUpdate { partial, .. } => {
                if let Some(Block::Tool(tool)) = self.transcript.last_mut() {
                    tool.output.push_str(&partial);
                    true
                } else {
                    false
                }
            }
            AgentEvent::ToolExecutionEnd {
                output,
                is_error,
                diff,
                path,
                ..
            } => {
                let matched = if let Some(Block::Tool(tool)) = self.transcript.last_mut() {
                    if !output.is_empty() {
                        tool.output = output;
                    }
                    tool.done = true;
                    tool.is_error = is_error;
                    // A failure is never hidden: an errored tool renders expanded.
                    if is_error {
                        tool.expanded = true;
                    }
                    tool.path = path.clone();
                    tool.diff = diff.clone();
                    true
                } else {
                    false
                };
                // A mutating tool's UI-only (path, diff) pair feeds the run's
                // changeset — recorded even if no block matched the call.
                if let (Some(path), Some(diff)) = (path, diff) {
                    self.changes.push(Change::new(path, diff));
                }
                matched
            }
            AgentEvent::AgentStart => {
                self.running = true;
                self.finished = false;
                true
            }
            AgentEvent::AgentEnd => {
                self.flush_live();
                self.running = false;
                self.finished = true;
                if self.cancelled {
                    self.cancelled = false;
                    self.transcript.push(Block::Notice("⏹ aborted".into()));
                }
                // Surface the run's changes once it settles; they stay for `/changes`.
                if !self.changes.is_empty() {
                    self.transcript.push(Block::Notice(self.changes_summary()));
                }
                true
            }
            AgentEvent::Error { message } => {
                self.flush_live();
                self.transcript.push(Block::Error(message));
                true
            }
            AgentEvent::Compaction { summarized, kept } => {
                self.transcript.push(Block::Notice(format!(
                    "⋯ compacted {summarized} messages, kept {kept}"
                )));
                true
            }
            AgentEvent::Retrying {
                attempt,
                max,
                reason,
            } => {
                self.transcript.push(Block::Notice(format!(
                    "⋯ retrying ({attempt}/{max}): {reason}"
                )));
                true
            }
            AgentEvent::TurnEnd { message } => self.record_usage(&message),
            AgentEvent::History { messages } => {
                self.render_usage(&messages);
                true
            }
            // Start and non-streamed replies need no state.
            _ => false,
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
    /// Returns `true` when it changed.
    fn record_usage(&mut self, message: &AgentMessage) -> bool {
        if let AgentMessage::Assistant {
            usage: Some(u), ..
        } = message
        {
            self.context_used = Some(u.input_tokens);
            true
        } else {
            false
        }
    }

    fn push_notice(&mut self, text: impl Into<String>) {
        self.transcript.push(Block::Notice(text.into()));
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
            self.push_notice("usage: no usage reported");
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
            self.push_notice(format!("usage: {}", parts.join(", ")));
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

    /// Seed the transcript with the conversation so far — what a resumed session
    /// (or a reconnecting socket client) already has. Returns `true` when it
    /// added anything.
    fn seed(&mut self, messages: &[AgentMessage]) -> bool {
        if messages.is_empty() {
            return false;
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
                        expanded: *is_error,
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
        true
    }

    /// A full page of transcript lines, for PgUp/PgDn.
    fn page(&self) -> usize {
        self.viewport.saturating_sub(1).max(1)
    }

    /// Scroll up by `lines`, clamped to the top of the transcript.
    fn scroll_up(&mut self, lines: usize) {
        self.scroll = (self.scroll + lines).min(self.max_scroll);
    }

    /// Scroll down by `lines`; `0` is the tail.
    fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// Reconcile scroll state with the transcript the renderer just measured:
    /// keep the view pinned while content grows, then clamp to the top.
    fn sync_scroll(&mut self, total: usize, height: usize, width: usize) {
        // Pin the view while content grows — but not across a resize, which
        // re-wraps everything and moves every line.
        if self.scroll > 0 && width == self.last_width {
            self.scroll = self
                .scroll
                .saturating_add(total.saturating_sub(self.last_total));
        }
        let max = total.saturating_sub(height);
        self.scroll = self.scroll.min(max);
        self.max_scroll = max;
        self.viewport = height;
        self.last_total = total;
        self.last_width = width;
    }
}

/// The placeholder root surface `App::new` starts with — replaced by
/// `set_surfaces` at startup, and the fallback that keeps the list non-empty.
fn default_root() -> Surface {
    Surface::new(SurfaceInfo {
        id: SessionId::agent("root"),
        label: "root".to_string(),
        model: String::new(),
        is_root: true,
    })
}

/// The whole UI state. Flat by design — grow submodules only when it hurts.
pub struct App {
    /// One entry per conversation surface. Index 0 is the root; the rest are
    /// team members. Always non-empty (see [`App::new`]).
    surfaces: Vec<Surface>,
    /// Index into `surfaces` of the focused surface.
    focus: usize,
    input: Vec<Atom>,
    /// Cursor as a *gap index* between atoms (`0..=input.len()`).
    cursor: usize,
    /// Monotonic id source for [`PasteBlock`]s.
    paste_id: u64,
    /// Model ids for the `/model` picker — injected by the composition root,
    /// since the TUI holds no `LlmOpts` and cannot list models itself.
    models: Vec<String>,
    /// Resumable sessions for the `/resume` picker — injected by the composition
    /// root, which owns the session dir the TUI cannot see.
    sessions: Vec<SessionItem>,
    /// Set when the user picks a session to resume; the run returns it so the CLI
    /// can re-exec with `--resume <path>`.
    pending_resume: Option<PathBuf>,
    /// The open modal, if any. While one is shown it captures every key.
    overlay: Option<Overlay>,
    /// The inline command-completion popup, if one is showing. Recomputed after
    /// every buffer edit; non-modal (it never owns the keyboard — see [`Completion`]).
    completion: Option<Completion>,
    dirty: bool,
    should_quit: bool,
    actions: Vec<Action>,
    /// Hide the team sidebar even when there is a team and the terminal is wide
    /// enough (`Ctrl-B`).
    hide_sidebar: bool,
}

impl Default for App {
    /// Same as [`App::new`] — a `Default` app still has its (root) surface, so
    /// the non-empty invariant holds.
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// A fresh app with one root surface — so [`App::focused`] is always valid.
    pub fn new() -> Self {
        App {
            surfaces: vec![default_root()],
            focus: 0,
            input: Vec::new(),
            cursor: 0,
            paste_id: 0,
            models: Vec::new(),
            sessions: Vec::new(),
            pending_resume: None,
            overlay: None,
            completion: None,
            dirty: true,
            should_quit: false,
            actions: Vec::new(),
            hide_sidebar: false,
        }
    }

    /// Replace the surface list (index 0 is the root) and focus the root. The
    /// list is never left empty, so [`App::focused`] cannot panic.
    pub fn set_surfaces(&mut self, surfaces: Vec<SurfaceInfo>) {
        self.surfaces = surfaces.into_iter().map(Surface::new).collect();
        if self.surfaces.is_empty() {
            self.surfaces.push(default_root());
        }
        self.focus = 0;
        self.dirty = true;
    }

    /// Add a surface that appeared mid-run (a runtime-spawned worker). A no-op
    /// if its id is already present; unlike [`App::set_surfaces`], focus is left
    /// where it is — a new teammate must not steal the user's perspective.
    pub fn add_surface(&mut self, info: SurfaceInfo) {
        if self.surface_index(&info.id).is_some() {
            return;
        }
        self.surfaces.push(Surface::new(info));
        self.dirty = true;
    }

    /// The focused surface's index (the run loop resolves its backend by it).
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// The focused surface — the one input and commands target.
    pub fn focused(&self) -> &Surface {
        &self.surfaces[self.focus]
    }

    /// Mutable access to the focused surface.
    pub fn focused_mut(&mut self) -> &mut Surface {
        &mut self.surfaces[self.focus]
    }

    /// The focused surface's id.
    pub fn focused_id(&self) -> &SessionId {
        &self.focused().id
    }

    fn surface_index(&self, id: &SessionId) -> Option<usize> {
        self.surfaces.iter().position(|s| &s.id == id)
    }

    /// Focus the next surface, wrapping.
    fn focus_next(&mut self) {
        if !self.surfaces.is_empty() {
            self.focus = (self.focus + 1) % self.surfaces.len();
            self.dirty = true;
        }
    }

    /// Focus surface `idx`, if it exists.
    fn set_focus(&mut self, idx: usize) {
        if idx < self.surfaces.len() {
            self.focus = idx;
            self.dirty = true;
        }
    }

    /// Expand or collapse the focused surface's most recent tool block — the
    /// per-tool complement of `Ctrl-T`. A no-op when the surface has no tool.
    fn toggle_last_tool(&mut self) {
        if let Some(tool) = self.focused_mut().last_tool_mut() {
            tool.expanded = !tool.expanded;
        } else {
            return;
        }
        self.dirty = true;
    }

    /// Expand or collapse every tool block on the focused surface — the
    /// complement of `Ctrl-O`. All expanded collapses; anything else expands all.
    fn toggle_all_tools(&mut self) {
        let tools: Vec<&mut Tool> = self
            .focused_mut()
            .transcript
            .iter_mut()
            .filter_map(|block| match block {
                Block::Tool(tool) => Some(tool),
                _ => None,
            })
            .collect();
        if tools.is_empty() {
            return;
        }
        let expand = !tools.iter().all(|tool| tool.expanded);
        for tool in tools {
            tool.expanded = expand;
        }
        self.dirty = true;
    }

    /// `Alt-1..9`: focus the Nth surface. `Alt-0` and an out-of-range N are a
    /// no-op.
    fn focus_digit(&mut self, c: char) {
        if let Some(n) = c.to_digit(10)
            && n >= 1
        {
            self.set_focus(n as usize - 1);
        }
    }

    /// Focus the previous surface, wrapping — the mirror of `focus_next`, bound
    /// to `Shift-Tab`.
    fn focus_prev(&mut self) {
        if !self.surfaces.is_empty() {
            self.focus = (self.focus + self.surfaces.len() - 1) % self.surfaces.len();
            self.dirty = true;
        }
    }

    /// Show or hide the team sidebar (`Ctrl-B`).
    fn toggle_sidebar(&mut self) {
        self.hide_sidebar = !self.hide_sidebar;
        self.dirty = true;
    }

    /// Whether the team sidebar is hidden (`Ctrl-B`) — read by the renderer.
    pub(crate) fn hide_sidebar(&self) -> bool {
        self.hide_sidebar
    }

    /// Open the keymap overlay (`F1`).
    fn open_help(&mut self) {
        self.overlay = Some(Overlay::Help);
        self.dirty = true;
    }

    /// Ctrl-A: move the cursor to the start of the input.
    fn move_cursor_start(&mut self) {
        self.cursor = 0;
        self.dirty = true;
    }

    /// Ctrl-E: move the cursor to the end of the input.
    fn move_cursor_end(&mut self) {
        self.cursor = self.input.len();
        self.dirty = true;
    }

    /// Ctrl-W: delete the word before the cursor. A paste chip is one word, so
    /// this removes the whole chip rather than reaching inside it.
    fn delete_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut start = self.cursor;
        while start > 0 && self.input[start - 1].is_space() {
            start -= 1;
        }
        while start > 0 && !self.input[start - 1].is_space() {
            start -= 1;
        }
        self.input.drain(start..self.cursor);
        self.cursor = start;
        self.focused_mut().history_index = None;
        self.dirty = true;
    }

    /// Ctrl-U: delete from the cursor back to the start of the current line
    /// (readline's `unix-line-discard`). Stops at a newline, so it is
    /// multi-line aware; a paste chip is one atom and is never split.
    fn kill_to_line_start(&mut self) {
        let start = (0..self.cursor)
            .rev()
            .find(|&i| matches!(self.input[i], Atom::Char('\n')))
            .map_or(0, |i| i + 1);
        if start == self.cursor {
            return;
        }
        self.input.drain(start..self.cursor);
        self.cursor = start;
        self.focused_mut().history_index = None;
        self.dirty = true;
    }

    /// Ctrl-K: delete from the cursor to the end of the current line (the
    /// newline itself is kept).
    fn kill_to_line_end(&mut self) {
        let end = (self.cursor..self.input.len())
            .find(|&i| matches!(self.input[i], Atom::Char('\n')))
            .unwrap_or(self.input.len());
        if end == self.cursor {
            return;
        }
        self.input.drain(self.cursor..end);
        self.focused_mut().history_index = None;
        self.dirty = true;
    }

    /// The non-root surfaces as `(label, model, state, focused)` — the sidebar
    /// and `/team`.
    pub fn member_rows(&self) -> Vec<(&str, &str, TeamState, bool)> {
        self.surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_root)
            .map(|(i, s)| {
                (
                    s.label.as_str(),
                    s.model.as_str(),
                    s.state(),
                    i == self.focus,
                )
            })
            .collect()
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
            AppEvent::Agent(id, event) => {
                // Route to the surface that owns this session; an unknown id is
                // ignored (a stray/duplicate event must not panic).
                if let Some(idx) = self.surface_index(&id)
                    && self.surfaces[idx].apply(event)
                {
                    self.dirty = true;
                }
            }
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
        // While the inline completion is open it owns Up/Down/Tab/Esc (and an
        // Enter with something left to complete); Char/Backspace fall through to
        // the buffer below.
        if self.completion.is_some() && self.on_completion_key(key) {
            return;
        }
        match key {
            Key::Char(c) => {
                self.focused_mut().history_index = None;
                self.insert_char(c);
            }
            Key::Backspace => {
                self.focused_mut().history_index = None;
                self.backspace();
            }
            Key::Delete => {
                self.focused_mut().history_index = None;
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
                self.focused_mut().history_index = None;
                self.insert_char('\n');
            }
            Key::Esc | Key::Ctrl('c') => self.interrupt(),
            Key::Ctrl('y') => self.copy_last(),
            // Ctrl-N / Shift-Tab cycle the focused surface (Ctrl-C cancels,
            // Ctrl-J/Ctrl-Y are taken; Tab/Enter/Esc/Up/Down belong to the composer).
            Key::Ctrl('n') => self.focus_next(),
            Key::BackTab => self.focus_prev(),
            Key::Alt(c) => self.focus_digit(c),
            Key::F(1) => self.open_help(),
            // Tool detail: Ctrl-O the last tool, Ctrl-T all of them.
            Key::Ctrl('o') => self.toggle_last_tool(),
            Key::Ctrl('t') => self.toggle_all_tools(),
            Key::Ctrl('b') => self.toggle_sidebar(),
            // Readline word/line editing on the atom buffer.
            Key::Ctrl('a') => self.move_cursor_start(),
            Key::Ctrl('e') => self.move_cursor_end(),
            Key::Ctrl('w') => self.delete_word(),
            Key::Ctrl('u') => self.kill_to_line_start(),
            Key::Ctrl('k') => self.kill_to_line_end(),
            Key::PageUp => self.scroll_up(self.page()),
            Key::PageDown => self.scroll_down(self.page()),
            Key::ScrollUp => self.scroll_up(WHEEL_LINES),
            Key::ScrollDown => self.scroll_down(WHEEL_LINES),
            _ => {}
        }
        self.recompute_completion();
    }

    /// Esc / Ctrl-C: cancel a run, else quit.
    fn interrupt(&mut self) {
        if self.focused().running {
            if !self.focused().cancelled {
                self.focused_mut().cancelled = true;
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
        self.focused_mut().scroll = 0;
        self.dirty = true;
        let text: String = atoms.iter().map(Atom::text).collect();
        let text = text.trim().to_string();
        self.focused_mut().history_index = None;
        self.focused_mut().draft.clear();
        if text.is_empty() {
            return;
        }
        if text.starts_with('/') {
            self.command(&text);
            return;
        }
        if self.focused().running {
            self.focused_mut()
                .transcript
                .push(Block::Notice("a turn is already running — Esc to cancel".into()));
            return;
        }
        self.focused_mut().history.push(text.clone());
        self.focused_mut().transcript.push(Block::User(text.clone()));
        self.focused_mut().running = true;
        self.focused_mut().finished = false;
        self.focused_mut().cancelled = false;
        // A new run starts a fresh changeset; the previous one is superseded.
        self.focused_mut().changes.clear();
        self.actions.push(Action::Submit(text));
    }

    /// Parse and act on a `/`-command typed at the prompt. Dispatch goes through
    /// [`COMMANDS`], so aliases and the generated `/help` share one source.
    fn command(&mut self, line: &str) {
        let mut parts = line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).filter(|s| !s.is_empty());
        self.notice(line);
        match command_named(name) {
            Some(cmd) => self.run_command(cmd, arg),
            None => self.notice(format!("unknown command: {name}")),
        }
        self.dirty = true;
    }

    /// Run a looked-up command; `arg` is the text after the name, if any.
    fn run_command(&mut self, cmd: &Command, arg: Option<&str>) {
        match cmd.name {
            "exit" => self.should_quit = true,
            "model" => match arg {
                Some(m) => self.actions.push(Action::Ask(Request::SetModel {
                    model: m.to_string(),
                })),
                None => self.open_model_picker(),
            },
            "effort" => match arg {
                Some(level) => {
                    let effort = match level {
                        "-" | "none" | "off" => None,
                        _ => Some(level.to_string()),
                    };
                    self.actions.push(Action::Ask(Request::SetEffort { effort }));
                }
                None => self.notice("usage: /effort <level> ('-' clears)"),
            },
            "compact" => self.actions.push(Action::Ask(Request::Compact {
                instructions: arg.map(str::to_string),
            })),
            "usage" => self.actions.push(Action::Ask(Request::GetHistory)),
            "changes" => self.open_changes_picker(),
            "resume" => self.open_session_picker(arg),
            "copy" => self.copy_last(),
            "team" => self.notice(team_text(&self.member_rows())),
            "surface" => self.open_surface_picker(),
            "help" => self.notice(help_text()),
            // Unreachable: every [`COMMANDS`] name is matched above. A debug
            // assert keeps a table entry from silently shadowing a real arm.
            _ => debug_assert!(
                false,
                "COMMANDS entry without a run_command arm: {}",
                cmd.name
            ),
        }
    }

    /// The command name being typed, when the buffer is still a lone `/`-token:
    /// `Some(text-after-slash)`. `None` once the buffer holds whitespace (the
    /// user moved on to arguments) or does not start with `/`.
    fn command_query(&self) -> Option<String> {
        let text = self.expanded();
        if text.contains(char::is_whitespace) {
            return None;
        }
        Some(text.strip_prefix('/')?.to_string())
    }

    /// Recompute the inline completion from the current buffer. Called after
    /// every edit; a lone `/`-token opens it, anything else closes it. A live
    /// selection is preserved (clamped) across edits.
    fn recompute_completion(&mut self) {
        let Some(query) = self.command_query() else {
            self.completion = None;
            return;
        };
        let matches = prefix_indices(COMMANDS, &query);
        if matches.is_empty() {
            self.completion = None;
            return;
        }
        let selected = self
            .completion
            .as_ref()
            .map_or(0, |c| c.selected.min(matches.len() - 1));
        self.completion = Some(Completion { matches, selected });
    }

    /// Handle a key while the completion popup is open. Returns `true` when the
    /// popup consumed it. `Char`/`Backspace` (and Enter with nothing left to
    /// complete) fall through to the buffer below.
    fn on_completion_key(&mut self, key: Key) -> bool {
        match key {
            Key::Up => {
                self.move_completion(-1);
                true
            }
            // Shift-Tab moves the selection up too, so it is consumed here and
            // never reaches the surface switch while the popup is open.
            Key::BackTab => {
                self.move_completion(-1);
                true
            }
            Key::Down => {
                self.move_completion(1);
                true
            }
            Key::Tab => {
                self.accept_completion();
                true
            }
            Key::Esc => {
                self.completion = None;
                self.dirty = true;
                true
            }
            Key::Enter => {
                // Enter completes only when something is left to complete; a
                // fully typed name/alias submits as usual, and a bare `/` is not
                // a command at all (it falls through to "unknown command: /").
                let pending = self
                    .command_query()
                    .is_some_and(|q| !q.is_empty() && !is_exact_command(&q));
                if pending {
                    self.accept_completion();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Move the highlighted completion row by `delta`, clamped to the matches.
    fn move_completion(&mut self, delta: isize) {
        if let Some(completion) = self.completion.as_mut() {
            let last = completion.matches.len().saturating_sub(1) as isize;
            completion.selected = (completion.selected as isize + delta).clamp(0, last) as usize;
        }
        self.dirty = true;
    }

    /// Insert the highlighted command as `/<name> ` (trailing space) and close
    /// the popup, so the user can type arguments.
    fn accept_completion(&mut self) {
        let Some(&idx) = self.completion.as_ref().and_then(|c| c.matches.get(c.selected)) else {
            self.completion = None;
            return;
        };
        let text = format!("/{} ", COMMANDS[idx].name);
        self.input = Self::atoms(&text);
        self.cursor = self.input.len();
        self.focused_mut().history_index = None;
        self.completion = None;
        self.dirty = true;
    }

    /// The completion rows to render (empty when no popup is showing).
    pub(crate) fn completion_rows(&self) -> Vec<CompletionRow> {
        let Some(completion) = &self.completion else {
            return Vec::new();
        };
        let query = self.command_query().unwrap_or_default();
        completion
            .matches
            .iter()
            .map(|&idx| {
                let cmd = &COMMANDS[idx];
                let label = format!("/{}", cmd.name);
                let range = prefix_range(&label, cmd.name, &query);
                let alias = (range.is_none() && !query.is_empty())
                    .then(|| matched_alias(cmd, &query))
                    .flatten();
                CompletionRow {
                    range,
                    label,
                    alias,
                    args: cmd.args,
                    summary: cmd.summary,
                }
            })
            .collect()
    }

    /// The highlighted completion row index (0 when no popup is showing).
    pub(crate) fn completion_selected(&self) -> usize {
        self.completion.as_ref().map_or(0, |c| c.selected)
    }
    /// Copy the last assistant reply to the terminal clipboard.
    fn copy_last(&mut self) {
        match self.focused().last_assistant_text() {
            Some(text) => {
                let chars = text.chars().count();
                self.actions.push(Action::Copy(text));
                self.notice(format!("copied {chars} chars to the clipboard"));
            }
            None => self.notice("nothing to copy yet"),
        }
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.focused_mut().push_notice(text);
        self.dirty = true;
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
        self.recompute_completion();
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

    /// Recall the previous prompt (saving the draft on the way up). The history
    /// is the focused surface's (per-surface recall, D25).
    fn history_up(&mut self) {
        if self.focused().history.is_empty() {
            return;
        }
        let next = match self.focused().history_index {
            None => {
                let draft = self.expanded();
                self.focused_mut().draft = draft;
                self.focused().history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.focused_mut().history_index = Some(next);
        let entry = self.focused().history[next].clone();
        self.input = Self::atoms(&entry);
        self.cursor = self.input.len();
        self.dirty = true;
    }

    /// Recall the next prompt, or restore the draft at the bottom.
    fn history_down(&mut self) {
        let Some(i) = self.focused().history_index else {
            return;
        };
        if i + 1 < self.focused().history.len() {
            self.focused_mut().history_index = Some(i + 1);
            let entry = self.focused().history[i + 1].clone();
            self.input = Self::atoms(&entry);
        } else {
            self.focused_mut().history_index = None;
            let draft = std::mem::take(&mut self.focused_mut().draft);
            self.input = Self::atoms(&draft);
        }
        self.cursor = self.input.len();
        self.dirty = true;
    }

    /// Seed the **root** surface's prompt history (oldest first), e.g. loaded
    /// from disk. Member histories stay in-memory (D25).
    pub fn load_history(&mut self, lines: Vec<String>) {
        if let Some(root) = self.surfaces.first_mut() {
            root.history = lines;
        }
    }

    /// The root surface's prompt history, oldest first, for persistence.
    pub fn history(&self) -> &[String] {
        self.surfaces.first().map_or(&[], |s| s.history.as_slice())
    }

    pub fn transcript(&self) -> &[Block] {
        &self.focused().transcript
    }

    pub fn live(&self) -> Option<&AgentMessage> {
        self.focused().live.as_ref()
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
        self.focused().context_used
    }

    /// A full page of transcript lines, for PgUp/PgDn.
    fn page(&self) -> usize {
        self.focused().page()
    }

    /// Scroll up by `lines`, clamped to the top of the transcript.
    fn scroll_up(&mut self, lines: usize) {
        self.focused_mut().scroll_up(lines);
        self.dirty = true;
    }

    /// Scroll down by `lines`; `0` is the tail.
    fn scroll_down(&mut self, lines: usize) {
        self.focused_mut().scroll_down(lines);
        self.dirty = true;
    }

    /// Lines scrolled up from the tail (`0` = following).
    pub fn scroll(&self) -> usize {
        self.focused().scroll
    }

    /// Reconcile scroll state with the transcript the renderer just measured:
    /// keep the view pinned while content grows, then clamp to the top.
    pub fn sync_scroll(&mut self, total: usize, height: usize, width: usize) {
        self.focused_mut().sync_scroll(total, height, width);
    }

    /// Open the `/changes` picker over the files this run changed.
    fn open_changes_picker(&mut self) {
        let rows = self.focused().changes_by_path();
        if rows.is_empty() {
            self.notice("no changes this run");
            return;
        }
        let mut items = Vec::new();
        let mut values = Vec::new();
        for (path, added, removed) in rows {
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
            .focused()
            .changes
            .iter()
            .filter(|c| c.path == path)
            .map(|c| c.diff.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if diff.is_empty() {
            return;
        }
        self.focused_mut().transcript.push(Block::Diff {
            path: path.to_string(),
            diff,
        });
        self.dirty = true;
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

    /// Open the `/surface` picker over the surfaces.
    fn open_surface_picker(&mut self) {
        let items = self
            .surfaces
            .iter()
            .map(|s| format!("{} · {}", s.label, s.model))
            .collect();
        let values = (0..self.surfaces.len()).map(|i| i.to_string()).collect();
        self.overlay = Some(Overlay::Pick(Picker::with_values(
            PickerKind::Surface,
            "surface",
            items,
            values,
        )));
        self.dirty = true;
    }

    /// Keys while a modal is open. The help overlay dismisses only on `Esc` /
    /// `F1` — every other key is ignored (a modal that silently swallowed a
    /// keystroke would be worse than one you must dismiss deliberately). The
    /// picker: ↑/↓ move, printable chars filter, Backspace deletes, Enter
    /// selects, Esc dismisses; everything else is swallowed.
    fn on_overlay_key(&mut self, key: Key) {
        if matches!(self.overlay, Some(Overlay::Help)) {
            if matches!(key, Key::Esc | Key::F(1)) {
                self.overlay = None;
                self.dirty = true;
            }
            return;
        }
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
                self.focused_mut().status.model.clone_from(&selected);
                self.notice(format!("model: {selected}"));
                self.actions
                    .push(Action::Ask(Request::SetModel { model: selected }));
            }
            PickerKind::Change => self.show_change(&selected),
            PickerKind::Surface => {
                if let Ok(idx) = selected.parse::<usize>() {
                    self.set_focus(idx);
                }
            }
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
        &self.focused().status
    }

    pub fn set_status(&mut self, status: Status) {
        self.focused_mut().status = status;
        self.dirty = true;
    }

    pub fn running(&self) -> bool {
        self.focused().running
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
    pub fn seed_history(&mut self, id: &SessionId, messages: &[AgentMessage]) {
        if let Some(idx) = self.surface_index(id)
            && self.surfaces[idx].seed(messages)
        {
            self.dirty = true;
        }
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

    /// The default root surface's id (`App::new`'s single surface).
    fn root() -> SessionId {
        SessionId::agent("root")
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
    fn a_fresh_app_has_one_focused_surface() {
        let app = App::new();
        assert_eq!(app.surfaces.len(), 1, "exactly one surface");
        assert_eq!(app.focus, 0, "focused on it");
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

        app.handle(AppEvent::Agent(root(), AgentEvent::MessageStart {
            message: assistant(""),
        }));
        assert!(app.live().is_some());
        app.handle(AppEvent::Agent(root(), AgentEvent::MessageUpdate {
            message: assistant("hel"),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::MessageEnd {
            message: assistant("hello"),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));

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
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionStart {
            call_id: "t1".into(),
            name: "bash".into(),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionUpdate {
            call_id: "t1".into(),
            name: "bash".into(),
            partial: "building\n".into(),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionEnd {
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

        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));
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
        app.handle(AppEvent::Agent(root(), AgentEvent::TurnEnd { message }));
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
        app.seed_history(&root(), &[
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
        app.seed_history(&root(), &[]);
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
        app.handle(AppEvent::Agent(root(), AgentEvent::History {
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
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd)); // the run finished
        submit(&mut app, "second");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));

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
        app.handle(AppEvent::Agent(root(), AgentEvent::MessageStart {
            message: assistant(""),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::MessageEnd {
            message: assistant("the answer"),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));
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
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionStart {
            call_id: "t1".into(),
            name: "edit".into(),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionEnd {
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
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionStart {
            call_id: "t1".into(),
            name: name.into(),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionEnd {
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

        assert_eq!(app.focused().changes.len(), 1);
        assert_eq!(app.focused().changes[0].path, "src/a.rs");
        assert_eq!(
            (
                app.focused().changes[0].added,
                app.focused().changes[0].removed
            ),
            (1, 1)
        );
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));

        // A new prompt starts a fresh changeset; the settled one is superseded.
        submit(&mut app, "again");
        assert!(app.focused().changes.is_empty());
    }

    #[test]
    fn a_tool_without_a_path_or_diff_is_not_a_change() {
        let mut app = App::new();
        submit(&mut app, "read");
        let _ = app.take_actions();
        tool_end(&mut app, "read", None, None);
        assert!(app.focused().changes.is_empty());
    }

    #[test]
    fn the_changeset_summary_appears_when_the_run_settles() {
        let mut app = App::new();
        submit(&mut app, "edit");
        let _ = app.take_actions();
        tool_end(&mut app, "edit", Some("a.rs"), Some("@@ -1 +1 @@\n-old\n+new"));
        tool_end(&mut app, "write", Some("b.rs"), Some("@@ -0,0 +1 @@\n+new"));
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));

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
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));

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
    fn a_member_surface_state_follows_its_run_events() {
        let mut app = App::new();
        let id = SessionId::agent("w1");
        app.set_surfaces(vec![
            SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "m".into(),
                is_root: true,
            },
            SurfaceInfo {
                id: id.clone(),
                label: "w1".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        assert_eq!(app.member_rows()[0].2, TeamState::Idle);
        app.handle(AppEvent::Agent(id.clone(), AgentEvent::AgentStart));
        assert_eq!(app.member_rows()[0].2, TeamState::Running);
        app.handle(AppEvent::Agent(id, AgentEvent::AgentEnd));
        assert_eq!(app.member_rows()[0].2, TeamState::Done);
        assert!(app.dirty(), "an event requests a redraw");
    }

    #[test]
    fn the_team_command_lists_the_member_surfaces() {
        // No members reads as "(no team)".
        let mut app = App::new();
        submit(&mut app, "/team");
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t == "(no team)"
        ));

        // Members list one `label · model · state` line each.
        let mut app = App::new();
        app.set_surfaces(vec![
            SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "m".into(),
                is_root: true,
            },
            SurfaceInfo {
                id: SessionId::agent("explorer"),
                label: "explorer".into(),
                model: "m1".into(),
                is_root: false,
            },
            SurfaceInfo {
                id: SessionId::agent("reviewer"),
                label: "reviewer".into(),
                model: "m2".into(),
                is_root: false,
            },
        ]);
        let reviewer = SessionId::agent("reviewer");
        app.handle(AppEvent::Agent(reviewer.clone(), AgentEvent::AgentStart));
        app.handle(AppEvent::Agent(reviewer, AgentEvent::AgentEnd));
        submit(&mut app, "/team");
        let Some(Block::Notice(text)) = app.transcript().last() else {
            panic!("expected a team notice");
        };
        assert!(text.contains("explorer · m1 · idle"), "{text}");
        assert!(text.contains("reviewer · m2 · done"), "{text}");
    }

    #[test]
    fn add_surface_adds_a_member_row_and_leaves_focus() {
        let mut app = App::new();
        assert!(app.member_rows().is_empty());

        app.add_surface(SurfaceInfo {
            id: SessionId::agent("explorer"),
            label: "explorer".into(),
            model: "m".into(),
            is_root: false,
        });
        assert_eq!(app.member_rows().len(), 1);
        assert_eq!(app.member_rows()[0].0, "explorer");
        assert_eq!(app.focus(), 0, "focus stays on the root");

        // `/team` reads the added surface too.
        submit(&mut app, "/team");
        let Some(Block::Notice(text)) = app.transcript().last() else {
            panic!("expected a team notice");
        };
        assert!(text.contains("explorer"), "{text}");

        // A duplicate id is a no-op.
        app.add_surface(SurfaceInfo {
            id: SessionId::agent("explorer"),
            label: "dup".into(),
            model: "m".into(),
            is_root: false,
        });
        assert_eq!(app.member_rows().len(), 1);
    }

    #[test]
    fn an_event_for_an_added_surface_routes_to_it() {
        let mut app = App::new();
        let id = SessionId::agent("explorer");
        app.add_surface(SurfaceInfo {
            id: id.clone(),
            label: "explorer".into(),
            model: "m".into(),
            is_root: false,
        });
        // Focus is still the root, so routing must be by id, not by focus.
        assert_eq!(app.focus(), 0);

        app.handle(AppEvent::Agent(id.clone(), AgentEvent::AgentStart));
        let row = app.member_rows();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].2, TeamState::Running, "the added surface runs");

        app.handle(AppEvent::Agent(id, AgentEvent::AgentEnd));
        assert_eq!(app.member_rows()[0].2, TeamState::Done);
    }

    /// A root + one member surface.
    fn two_surfaces() -> (App, SessionId, SessionId) {
        let mut app = App::new();
        let root_id = root();
        let member = SessionId::agent("w1");
        app.set_surfaces(vec![
            SurfaceInfo {
                id: root_id.clone(),
                label: "root".into(),
                model: "m".into(),
                is_root: true,
            },
            SurfaceInfo {
                id: member.clone(),
                label: "w1".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        (app, root_id, member)
    }

    #[test]
    fn agent_events_route_by_session_id() {
        let (mut app, _root_id, member) = two_surfaces();
        // An event for the member lands there, not on the focused root.
        app.handle(AppEvent::Agent(
            member.clone(),
            AgentEvent::Error {
                message: "boom".into(),
            },
        ));
        assert!(app.transcript().is_empty(), "the focused root is untouched");
        // An unknown id is ignored, without panicking.
        app.handle(AppEvent::Agent(
            SessionId::agent("ghost"),
            AgentEvent::AgentEnd,
        ));
        // Focusing the member shows its own event.
        app.set_focus(1);
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Error(m)) if m == "boom"
        ));
    }

    #[test]
    fn two_surfaces_keep_independent_transcripts() {
        let (mut app, a, b) = two_surfaces();
        app.handle(AppEvent::Agent(
            a,
            AgentEvent::Error { message: "A".into() },
        ));
        app.handle(AppEvent::Agent(
            b,
            AgentEvent::Error { message: "B".into() },
        ));
        app.set_focus(0);
        assert_eq!(app.transcript().len(), 1);
        assert!(matches!(&app.transcript()[0], Block::Error(m) if m == "A"));
        app.set_focus(1);
        assert_eq!(app.transcript().len(), 1);
        assert!(matches!(&app.transcript()[0], Block::Error(m) if m == "B"));
    }

    #[test]
    fn slash_surface_and_the_cycle_key_switch_focus() {
        let (mut app, _, _) = two_surfaces();
        assert_eq!(app.focus(), 0);
        // Ctrl-N cycles, wrapping.
        app.handle(AppEvent::Key(Key::Ctrl('n')));
        assert_eq!(app.focus(), 1);
        app.handle(AppEvent::Key(Key::Ctrl('n')));
        assert_eq!(app.focus(), 0, "wraps back to the root");

        // `/surface` opens the picker; selecting row 1 focuses surface 1.
        submit(&mut app, "/surface");
        assert!(matches!(app.overlay(), Some(Overlay::Pick(p)) if p.title == "surface"));
        app.handle(AppEvent::Key(Key::Down));
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.focus(), 1);
    }

    #[test]
    fn prompt_history_recall_is_per_surface() {
        let (mut app, root_id, member) = two_surfaces();
        submit(&mut app, "root one");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(root_id.clone(), AgentEvent::AgentEnd));
        submit(&mut app, "root two");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(root_id.clone(), AgentEvent::AgentEnd));
        app.set_focus(1);
        submit(&mut app, "member one");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(member, AgentEvent::AgentEnd));

        // The member recalls only its own prompt.
        app.handle(AppEvent::Key(Key::Up));
        assert_eq!(app.input(), "member one");
        // The root recalls its own (most recent first).
        app.set_focus(0);
        app.handle(AppEvent::Key(Key::Up));
        assert_eq!(app.input(), "root two");
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

    fn completion_labels(app: &App) -> Vec<String> {
        app.completion_rows().into_iter().map(|r| r.label).collect()
    }

    #[test]
    fn help_lists_every_command_and_the_keymap_from_the_tables() {
        let mut app = App::new();
        submit(&mut app, "/help");
        let Some(Block::Notice(text)) = app.transcript().last() else {
            panic!("expected a help notice");
        };
        assert!(
            text.starts_with(
                "commands: /exit /model <id> /effort [level] /compact [text] /changes \
                 /resume /usage /copy /surface /team /help"
            ),
            "the command listing changed: {text}"
        );
        for name in [
            "exit", "model", "effort", "compact", "changes", "resume", "usage", "copy", "surface",
            "team", "help",
        ] {
            assert!(
                text.contains(&format!("/{name}")),
                "{name} missing from help: {text}"
            );
        }
        // The keymap is generated from `KEYS`, so discovery has a path without F1.
        assert!(text.contains("keys:"), "keys section missing: {text}");
        for (chord, _) in KEYS {
            assert!(
                text.contains(chord),
                "{chord} missing from /help: {text}"
            );
        }
    }

    #[test]
    fn completion_filters_by_the_typed_prefix() {
        let mut app = App::new();
        typed(&mut app, "/mo");
        assert_eq!(completion_labels(&app), vec!["/model"]);
    }

    #[test]
    fn completion_matches_a_prefix_case_insensitively() {
        // `prefix_indices` lowercases both sides, so an upper/mixed-case prefix
        // still completes — a canonical name and an alias each.
        let mut app = App::new();
        typed(&mut app, "/MO");
        assert_eq!(completion_labels(&app), vec!["/model"]);

        let mut app = App::new();
        typed(&mut app, "/SES"); // a case-insensitive prefix of the `/sessions` alias
        assert_eq!(completion_labels(&app), vec!["/resume"]);
    }

    #[test]
    fn a_bare_slash_matches_every_command() {
        let mut app = App::new();
        typed(&mut app, "/");
        let labels = completion_labels(&app);
        assert_eq!(labels.len(), COMMANDS.len());
        assert!(labels.contains(&"/exit".to_string()));
        assert!(labels.contains(&"/help".to_string()));
    }

    #[test]
    fn tab_accepts_the_highlighted_command_with_a_trailing_space() {
        let mut app = App::new();
        typed(&mut app, "/mo");
        app.handle(AppEvent::Key(Key::Tab));
        assert_eq!(app.input(), "/model ");
        assert_eq!(app.cursor(), 7);
        assert!(app.completion_rows().is_empty(), "the popup closed");
    }

    #[test]
    fn esc_dismisses_the_completion_without_editing_the_buffer() {
        let mut app = App::new();
        typed(&mut app, "/mo");
        assert!(!app.completion_rows().is_empty());
        app.handle(AppEvent::Key(Key::Esc));
        assert_eq!(app.input(), "/mo", "the buffer is untouched");
        assert!(app.completion_rows().is_empty(), "the popup closed");
        assert!(!app.should_quit(), "Esc dismisses the popup, it does not quit");
    }

    #[test]
    fn up_and_down_move_the_completion_selection() {
        let mut app = App::new();
        typed(&mut app, "/");
        assert_eq!(app.completion_selected(), 0);
        app.handle(AppEvent::Key(Key::Down));
        assert_eq!(app.completion_selected(), 1);
        assert_eq!(app.input(), "/", "moving the selection does not edit the buffer");
        app.handle(AppEvent::Key(Key::Up));
        assert_eq!(app.completion_selected(), 0);
    }

    #[test]
    fn up_recalls_history_once_the_popup_is_closed() {
        let mut app = App::new();
        submit(&mut app, "first");
        let _ = app.take_actions();
        app.handle(AppEvent::Agent(root(), AgentEvent::AgentEnd));

        typed(&mut app, "drafty"); // no slash: no completion popup
        assert!(app.completion_rows().is_empty());
        app.handle(AppEvent::Key(Key::Up));
        assert_eq!(app.input(), "first");
        app.handle(AppEvent::Key(Key::Down));
        assert_eq!(app.input(), "drafty");
    }

    #[test]
    fn a_space_falls_through_and_closes_the_popup() {
        let mut app = App::new();
        typed(&mut app, "/model");
        assert!(!app.completion_rows().is_empty());
        typed(&mut app, " gpt");
        assert!(app.completion_rows().is_empty(), "a space closed the popup");
        assert_eq!(app.input(), "/model gpt");
    }

    #[test]
    fn enter_completes_a_partial_command_but_submits_a_complete_one() {
        // A partial name: Enter accepts the highlighted completion (no submit).
        let mut app = App::new();
        typed(&mut app, "/mo");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.input(), "/model ");
        assert!(app.take_actions().is_empty(), "no submit happened");
        assert!(app.transcript().is_empty(), "no user block was committed");
        assert!(app.completion_rows().is_empty());

        // A complete name: Enter submits it as today (`/usage` asks for history).
        let mut app = App::new();
        typed(&mut app, "/usage");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.take_actions(), vec![Action::Ask(Request::GetHistory)]);
    }

    #[test]
    fn quit_alias_dispatches_through_the_table() {
        let mut app = App::new();
        submit(&mut app, "/quit");
        assert!(app.should_quit());
        assert!(app.take_actions().is_empty());
    }
    #[test]
    fn completion_matches_a_leading_prefix_only() {
        // `/s` is a prefix of the `/sessions` alias (`/resume`) and of the
        // `/surface` name; `/z` is a prefix of nothing.
        let mut app = App::new();
        typed(&mut app, "/s");
        assert_eq!(completion_labels(&app), vec!["/resume", "/surface"]);

        let mut app = App::new();
        typed(&mut app, "/z");
        assert!(completion_labels(&app).is_empty(), "/z must not match anything");

        // `/m` → `/model`; `/e` → `/effort` + `/exit`.
        let mut app = App::new();
        typed(&mut app, "/m");
        assert_eq!(completion_labels(&app), vec!["/model"]);

        let mut app = App::new();
        typed(&mut app, "/e");
        assert_eq!(completion_labels(&app), vec!["/exit", "/effort"]);
    }

    #[test]
    fn completion_suggests_an_alias_by_its_canonical_name() {
        let mut app = App::new();
        typed(&mut app, "/ses"); // a prefix of the `/sessions` alias for `/resume`
        assert_eq!(completion_labels(&app), vec!["/resume"]);
    }

    #[test]
    fn an_alias_match_carries_the_alias_hint() {
        let mut app = App::new();
        typed(&mut app, "/se");
        let rows = app.completion_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "/resume");
        assert_eq!(rows[0].alias, Some("sessions"));
        assert!(rows[0].range.is_none(), "no canonical-name prefix to highlight");
    }

    #[test]
    fn typing_a_full_alias_submits_it_rather_than_rewriting_it() {
        let mut app = App::new();
        app.set_sessions(sessions());
        submit(&mut app, "/sessions");
        // Dispatched to `/resume` (the picker opened) — not rewritten to `/resume `.
        assert!(matches!(app.overlay(), Some(Overlay::Pick(p)) if p.title == "resume"));
    }

    #[test]
    fn a_bare_slash_does_not_accept_on_enter() {
        let mut app = App::new();
        typed(&mut app, "/");
        assert!(!app.completion_rows().is_empty(), "the popup lists every name");
        app.handle(AppEvent::Key(Key::Enter));
        // Enter fell through: the buffer was submitted as a (bad) command.
        assert_eq!(app.input(), "");
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t == "unknown command: /"
        ));
    }

    #[test]
    fn the_picker_still_matches_mid_string() {
        // The completion moved to prefix matching, but the picker keeps the
        // substring filter: "eta" is not a prefix of "beta" yet must match it.
        let mut app = App::new();
        app.set_models(vec!["alpha".into(), "beta".into()]);
        submit(&mut app, "/model");
        let _ = app.take_actions();
        typed(&mut app, "eta");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SetModel {
                model: "beta".into()
            })]
        );
    }

    /// Give `app` a root plus `labels` (index 0 is the root).
    fn set_surfaces(app: &mut App, labels: &[&str]) {
        app.set_surfaces(
            labels
                .iter()
                .enumerate()
                .map(|(i, label)| crate::SurfaceInfo {
                    id: SessionId::agent(label),
                    label: (*label).to_string(),
                    model: "m".to_string(),
                    is_root: i == 0,
                })
                .collect(),
        );
    }

    /// Commit one finished `bash` tool block with `output`.
    fn push_done_tool(app: &mut App, output: &str) {
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t".into(),
                name: "bash".into(),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionEnd {
                call_id: "t".into(),
                name: "bash".into(),
                output: output.into(),
                is_error: false,
                diff: None,
                path: None,
            },
        ));
    }

    fn tool_expanded(app: &App, nth_from_end: usize) -> bool {
        app.transcript()
            .iter()
            .rev()
            .filter_map(|b| match b {
                Block::Tool(t) => Some(t.expanded),
                _ => None,
            })
            .nth(nth_from_end)
            .expect("a tool block")
    }

    #[test]
    fn alt_digit_focuses_the_nth_surface() {
        let mut app = App::new();
        set_surfaces(&mut app, &["root", "a", "b"]);
        app.handle(AppEvent::Key(Key::Alt('3')));
        assert_eq!(app.focus(), 2, "Alt-3 focuses the third surface");
        // Alt-0 and an out-of-range surface are a no-op.
        app.handle(AppEvent::Key(Key::Alt('0')));
        assert_eq!(app.focus(), 2);
        app.handle(AppEvent::Key(Key::Alt('9')));
        assert_eq!(app.focus(), 2);
        app.handle(AppEvent::Key(Key::Alt('1')));
        assert_eq!(app.focus(), 0);
    }

    #[test]
    fn backtab_moves_focus_backwards_and_wraps() {
        let mut app = App::new();
        set_surfaces(&mut app, &["root", "a", "b"]);
        app.handle(AppEvent::Key(Key::BackTab));
        assert_eq!(app.focus(), 2, "Shift-Tab wraps to the last surface");
        app.handle(AppEvent::Key(Key::BackTab));
        assert_eq!(app.focus(), 1);
    }

    #[test]
    fn backtab_is_consumed_by_the_completion_popup_before_the_surface_switch() {
        let mut app = App::new();
        set_surfaces(&mut app, &["root", "a"]);
        typed(&mut app, "/mo"); // opens the completion popup
        assert!(!app.completion_rows().is_empty());
        app.handle(AppEvent::Key(Key::BackTab));
        // The popup owns the key while it is open; the surface must not change.
        assert_eq!(app.focus(), 0, "the popup consumed BackTab");
        assert!(!app.completion_rows().is_empty(), "the popup stays open");
        // Once dismissed, BackTab reaches the surface switch again.
        app.handle(AppEvent::Key(Key::Esc));
        app.handle(AppEvent::Key(Key::BackTab));
        assert_eq!(app.focus(), 1);
    }

    #[test]
    fn f1_opens_the_help_overlay_and_only_esc_or_f1_closes_it() {
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::F(1)));
        assert!(matches!(app.overlay(), Some(Overlay::Help)));
        // Every other key is ignored — it neither closes the modal nor edits.
        app.handle(AppEvent::Key(Key::Char('x')));
        assert!(matches!(app.overlay(), Some(Overlay::Help)));
        assert_eq!(app.input(), "", "the modal gates the input");
        app.handle(AppEvent::Key(Key::Esc));
        assert!(app.overlay().is_none());
        // F1 also closes it.
        app.handle(AppEvent::Key(Key::F(1)));
        app.handle(AppEvent::Key(Key::F(1)));
        assert!(app.overlay().is_none());
    }

    #[test]
    fn ctrl_b_toggles_the_sidebar_flag() {
        let mut app = App::new();
        assert!(!app.hide_sidebar());
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        assert!(app.hide_sidebar());
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        assert!(!app.hide_sidebar());
    }

    #[test]
    fn ctrl_t_flips_every_tool_at_once() {
        let mut app = App::new();
        push_done_tool(&mut app, "one");
        push_done_tool(&mut app, "two");
        assert!(!tool_expanded(&app, 0) && !tool_expanded(&app, 1));
        app.handle(AppEvent::Key(Key::Ctrl('t')));
        assert!(tool_expanded(&app, 0) && tool_expanded(&app, 1), "all expanded");
        app.handle(AppEvent::Key(Key::Ctrl('t')));
        assert!(!tool_expanded(&app, 0) && !tool_expanded(&app, 1), "all collapsed");
    }

    #[test]
    fn ctrl_w_deletes_the_previous_word_and_a_paste_chip_whole() {
        let mut app = App::new();
        typed(&mut app, "hello world");
        app.handle(AppEvent::Key(Key::Ctrl('w')));
        assert_eq!(app.input(), "hello ");
        app.handle(AppEvent::Key(Key::Ctrl('w')));
        assert_eq!(app.input(), "");
    }

    #[test]
    fn ctrl_w_next_to_a_paste_chip_deletes_the_whole_chip() {
        let mut app = App::new();
        app.handle(AppEvent::Paste("a\nb\nc\nd".into())); // one atomic chip
        typed(&mut app, " done");
        app.handle(AppEvent::Key(Key::Ctrl('w')));
        assert_eq!(app.input(), "a\nb\nc\nd ", "the chip survives the word kill");
        app.handle(AppEvent::Key(Key::Ctrl('w')));
        // The chip is one atom: it goes whole, never partially.
        assert_eq!(app.input(), "", "the second kill takes the chip whole");
    }

    #[test]
    fn ctrl_u_kills_to_the_start_of_the_current_line() {
        let mut app = App::new();
        typed(&mut app, "abc def");
        for _ in 0..3 {
            app.handle(AppEvent::Key(Key::Left)); // cursor after "abc "
        }
        app.handle(AppEvent::Key(Key::Ctrl('u')));
        assert_eq!(app.input(), "def");

        // Line-aware: it stops at the newline, keeping the earlier line.
        let mut app = App::new();
        typed(&mut app, "keep");
        app.handle(AppEvent::Key(Key::Newline));
        typed(&mut app, "drop");
        app.handle(AppEvent::Key(Key::Ctrl('u')));
        assert_eq!(app.input(), "keep\n");
    }

    #[test]
    fn ctrl_k_kills_to_the_end_of_the_current_line() {
        let mut app = App::new();
        typed(&mut app, "abc def");
        for _ in 0..3 {
            app.handle(AppEvent::Key(Key::Left)); // cursor after "abc "
        }
        app.handle(AppEvent::Key(Key::Ctrl('k')));
        assert_eq!(app.input(), "abc ");

        // The newline itself is kept.
        let mut app = App::new();
        typed(&mut app, "drop");
        app.handle(AppEvent::Key(Key::Newline));
        typed(&mut app, "keep");
        app.handle(AppEvent::Key(Key::Ctrl('a'))); // to the very start
        app.handle(AppEvent::Key(Key::Right)); // just past 'd'
        app.handle(AppEvent::Key(Key::Ctrl('k')));
        assert_eq!(app.input(), "d\nkeep");
    }

    #[test]
    fn ctrl_a_and_ctrl_e_move_the_cursor() {
        let mut app = App::new();
        typed(&mut app, "abc");
        app.handle(AppEvent::Key(Key::Ctrl('a')));
        assert_eq!(app.cursor(), 0);
        app.handle(AppEvent::Key(Key::Ctrl('e')));
        assert_eq!(app.cursor(), 3);
    }
}
