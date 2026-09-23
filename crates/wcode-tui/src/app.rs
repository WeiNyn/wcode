//! The TUI's application state — a small, flat struct plus pure reducers.
//!
//! Nothing here touches the terminal or crossterm: [`App::handle`] is a pure
//! function of `(state, event)`, so the whole layer is testable headless. The
//! app never performs IO; it defers side effects as [`Action`]s the event loop
//! drains. The loop feeds it [`AppEvent`]s and draws when [`App::dirty`] is set.

use std::ops::Range;
use std::path::{Path, PathBuf};

use wcode_harness::event::{AgentEvent, TodoItem, TodoStatus};
use wcode_harness::message::{AgentMessage, ContentBlock};
use wcode_harness::stats::session_stats;
use wcode_harness::protocol::{Request, SessionId};

// The per-block render cache stores `ui`'s own output type (`Line`) so a hit hands
// the renderer ready lines; `Line` is the one render-backend type app.rs names.
use ratatui::text::Line;

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
    /// A `/btw` side answer: display-only, never committed to ctx/session.
    /// The renderer styles it distinctly (a `btw:` gutter, dim/italic).
    Btw(String),
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
    /// preview. Toggled per block in browse mode (`Enter`/`Space`), or all at
    /// once with `Ctrl-T`; forced on when the tool errored, so a failure is never
    /// hidden.
    pub expanded: bool,
    /// A UI-only unified diff, when the tool changed a file (`ToolOutput::diff`).
    pub diff: Option<String>,
    /// The file this tool changed (UI-only), from `ToolExecutionEnd`: labels the
    /// `⚙` line and feeds the run's changeset.
    pub path: Option<String>,
    /// Wall-clock ms the tool's `execute` took (from `ToolExecutionEnd`).
    /// UI-only: never copied (`copy_text`), never persisted. `None` while the
    /// tool is still running, or for a tool reseeded from a resumed session
    /// (the transcript has no timing).
    pub duration_ms: Option<u64>,
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

/// The text `y` copies for a block — `None` when there is nothing to copy.
///
/// - `User` / `Notice` / `Error`: the block's text.
/// - `Assistant`: the text blocks only — **thinking is internal reasoning and is
///   never copied** — with code fences preserved.
/// - `Tool`: the **full `output`**, never the collapsed preview on screen (the
///   headline case: yanking the whole `bash` result).
/// - `Diff`: the diff body.
fn copy_text(block: &Block) -> Option<String> {
    let text = match block {
        Block::User(text)
        | Block::Notice(text)
        | Block::Error(text)
        | Block::Btw(text) => text.clone(),
        Block::Assistant(content) => content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                // Thinking is reasoning; a tool call has its own line, not text.
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Tool(tool) => tool.output.clone(),
        Block::Diff { diff, .. } => diff.clone(),
    };
    (!text.trim().is_empty()).then_some(text)
}

/// Render the `todo` checklist as one line per item, for a transcript notice.
/// Local to the TUI — `wcode-tui` does not depend on `wcode-cli`, so it cannot
/// reach the tool's `render`: `[ ]` pending, `[>]` in progress, `[x]` completed.
fn render_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "todos: (none)".to_string();
    }
    let lines: Vec<String> = todos
        .iter()
        .map(|item| {
            let mark = match item.status {
                TodoStatus::Pending => "[ ]",
                TodoStatus::InProgress => "[>]",
                TodoStatus::Completed => "[x]",
            };
            format!("{mark} {}", item.content)
        })
        .collect();
    format!("todos:\n{}", lines.join("\n"))
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
        name: "reload",
        aliases: &[],
        args: Some("[--no-session]"),
        summary: "rebuild and re-exec into this session",
    },
    Command {
        name: "btw",
        aliases: &[],
        args: Some("<question>"),
        summary: "ask a tool-free side question",
    },
    Command {
        name: "plan",
        aliases: &[],
        args: Some("[on|off]"),
        summary: "toggle plan mode (explore, don't mutate)",
    },
    Command {
        name: "verify",
        aliases: &[],
        args: None,
        summary: "check the plan's progress (are we finished?)",
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
        name: "tasks",
        aliases: &[],
        args: None,
        summary: "list the tasks",
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

/// The `/team` listing: one `{glyph} {label} · {state}` line per member surface,
/// with its live action appended when there is one. The model is not shown (§2).
fn team_text(rows: &[(&str, TeamState, bool, Option<&str>)]) -> String {
    if rows.is_empty() {
        return "(no team)".to_string();
    }
    rows.iter()
        .map(|(label, state, _, action)| {
            let head = format!("{} {label} · {}", state.glyph(), state.label());
            match action {
                Some(action) => format!("{head} · {action}"),
                None => head,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `/tasks` listing: one `#<id> [<state>] <title> (<owner>)` line per task,
/// with `(unassigned)` when no owner. The plan is reflected here, not edited.
fn tasks_text(tasks: &[TaskItem]) -> String {
    if tasks.is_empty() {
        return "(no tasks yet)".to_string();
    }
    tasks
        .iter()
        .map(|t| match &t.owner {
            Some(owner) => format!("#{} [{}] {} ({owner})", t.id, t.state, t.title),
            None => format!("#{} [{}] {} (unassigned)", t.id, t.state, t.title),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The argument keys a tool call may name in its label, most specific first.
const ACTION_KEYS: [&str; 9] = [
    "path",
    "file_path",
    "file",
    "pattern",
    "command",
    "cmd",
    "query",
    "url",
    "name",
];

/// The team strip's action label for a tool call: `"{name} {target}"`, where the
/// target is the first present string argument among [`ACTION_KEYS`] on the
/// committed assistant block whose `ToolCall` id matches `call_id` (§3). No
/// matching call, or no recognizable argument, falls back to just `"{name}"`.
fn action_label(transcript: &[Block], call_id: &str, name: &str) -> String {
    // The arguments ride the committed assistant block; scan from the end, as a
    // call id is unique to the newest turn that carries it.
    for block in transcript.iter().rev() {
        let Block::Assistant(content) = block else {
            continue;
        };
        let call = content
            .iter()
            .find(|b| matches!(b, ContentBlock::ToolCall { id, .. } if id == call_id));
        let Some(ContentBlock::ToolCall { arguments, .. }) = call else {
            continue;
        };
        let target = ACTION_KEYS
            .iter()
            .find_map(|key| arguments.get(*key).and_then(|v| v.as_str()));
        return match target {
            Some(target) => clip_label(&format!("{name} {target}")),
            None => name.to_string(),
        };
    }
    name.to_string()
}

/// Truncate a team-strip action to `~18` columns, ending with `…` when cut.
fn clip_label(text: &str) -> String {
    const MAX: usize = 18;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX - 1).collect();
    out.push('…');
    out
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
    ("Ctrl-T", "expand / collapse all tool output"),
    ("Ctrl-N / Shift-Tab", "focus the next / previous surface"),
    ("Alt-1..9", "focus the Nth surface"),
    ("Ctrl-G", "browse the transcript"),
    ("Ctrl-B", "toggle the sidebar"),
    ("j / k · g / G", "browse: next / previous · first / last"),
    ("Esc / q / ? (browse)", "leave / help · F1, Ctrl-C global"),
    ("Enter / Space (browse)", "expand / collapse the selected block"),
    ("y (browse)", "copy the selected block"),
    ("Home / End (browse)", "jump to the top / bottom block in view"),
    ("{ / } (browse)", "previous / next block"),
    ("/ (browse)", "search the transcript (n / N repeat)"),
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

/// One planned task for the `/tasks` listing — a *view* type injected by the
/// composition root (the TUI owns no `TaskList` and cannot see one). Mirrors
/// [`SessionItem`]: a plain, cloneable value the CLI builds from a snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskItem {
    pub id: u32,
    pub title: String,
    /// The assigned worker's short name (`w1`) or address, if any.
    pub owner: Option<String>,
    /// The display state (`todo`/`doing`/`done`).
    pub state: String,
}

/// Status-line fields.
#[derive(Clone, Debug)]
pub struct Status {
    pub model: String,
    pub effort: Option<String>,
    /// The session's id, shown in the status line (local sessions only).
    pub session: Option<String>,
    /// The model's context window, for the `used / limit` readout.
    /// The model's context window, for the `used / limit` readout.
    pub context_limit: Option<u64>,
    /// Plan mode is on (the status line shows a `plan` chip). Optimistic on the
    /// client; the harness hook is the source of truth.
    pub plan: bool,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            model: "wcode".to_string(),
            effort: None,
            session: None,
            context_limit: None,
            plan: false,
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
            plan: false,
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
/// One conversation's state: its identity (the id events route by), the
/// `label` the team strip shows, the transcript, run state, changeset,
/// scroll pin, and prompt history. `App` holds a `Vec<Surface>`; index 0 is the
/// root, the rest are team members.
/// One committed block's cached render, index-aligned with `Surface::transcript`.
///
/// A frame hits iff `rev == block_revs[i] && width == draw_width`; a miss re-renders
/// the block once (`ui::block_lines`) and overwrites. Stores PRE-bar lines — the
/// browse selection bar is a second pass (`ui::paint_bar`).
struct CacheEntry {
    /// The block revision this entry rendered; compared to `block_revs[i]`.
    rev: u64,
    /// The width this entry rendered at; a resize invalidates every entry here.
    width: usize,
    /// The block's rendered lines (`lines.len()` is its height).
    lines: Vec<Line<'static>>,
}

impl CacheEntry {
    /// A guaranteed miss: no real draw is `usize::MAX` columns wide.
    fn never() -> Self {
        Self {
            rev: 0,
            width: usize::MAX,
            lines: Vec::new(),
        }
    }
}
pub struct Surface {
    /// The id this surface's events route by.
    id: SessionId,
    /// A short display name (the team strip + status line).
    label: String,
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
    /// A run finished (drives the team strip's `done`).
    finished: bool,
    /// The last run failed (set on a run-failure `Error` while running, kept
    /// across `AgentEnd`, cleared on the next `AgentStart`).
    failed: bool,
    /// The current run's live action (`"{tool} {target}"`) for the team strip
    /// and `/team`; cleared at run start/end so it reflects the current run
    /// only (§3).
    last_action: Option<String>,
    /// Monotonic stamp of the last action, for ordering members by recency
    /// (active first, then most recent action first) in the team strip.
    last_action_at: Option<u64>,
    /// A `Cancel` was sent; the next `AgentEnd` is rendered as an abort.
    cancelled: bool,
    /// Provider-reported input tokens of the last turn: how full the context was.
    context_used: Option<u64>,
    /// The pre-toggle value of `status.plan` while a `/plan` `SetPlanMode` is in
    /// flight; restored if the reply is an `AgentEvent::Error` (amendment 8).
    plan_pending: Option<bool>,
    /// The latest `AgentEvent::Todo` list, cached for `/verify` (a client-side
    /// render of the event — never `Session::todo()`, whose own-session write the
    /// agent's in-memory entries never see).
    last_todos: Option<Vec<TodoItem>>,
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
    /// The selected committed block while browsing. Indexes `transcript` only —
    /// never the live block, which is transient. Clamped/reset when the
    /// transcript changes.
    selected: Option<usize>,
    /// Each committed block's line range from the last drawn frame — the
    /// renderer's feedback for drawing the selection bar.
    ranges: Vec<Range<usize>>,
    /// Per-block render cache, index-aligned with `transcript`.
    cache: Vec<CacheEntry>,
    /// Per-block revision, bumped at each `Block::Tool` mutation; `0` = append-only.
    block_revs: Vec<u64>,
    /// Time since this surface's `AgentStart`, set by the loop on each tick
    /// while the run is in flight; cleared on `AgentEnd`. `None` when idle, so
    /// the status line shows the bare state glyph.
    run_elapsed: Option<std::time::Duration>,
    /// Cache misses so far (a `block_lines` render). Test-only; the renderer never
    /// reads it.
    #[cfg(test)]
    cache_misses: usize,
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
            is_root: info.is_root,
            status,
            transcript: Vec::new(),
            live: None,
            changes: Vec::new(),
            running: false,
            finished: false,
            failed: false,
            last_action: None,
            last_action_at: None,
            cancelled: false,
            context_used: None,
            plan_pending: None,
            last_todos: None,
            scroll: 0,
            max_scroll: 0,
            viewport: 0,
            last_total: 0,
            last_width: 0,
            history: Vec::new(),
            history_index: None,
            draft: String::new(),
            selected: None,
            ranges: Vec::new(),
            cache: Vec::new(),
            block_revs: Vec::new(),
            run_elapsed: None,
            #[cfg(test)]
            cache_misses: 0,
        }
    }

    /// Ensure block `i` is rendered for `width`, then append the inter-block blank
    /// separator (when `i > 0`, `out` is non-empty, and the block renders ≥1 line)
    /// and the block's cached lines to `out`. Returns the range of the block's OWN
    /// lines — the separator is excluded, so `ranges` stays exact.
    ///
    /// Owns the transcript/cache split borrow itself, so the caller never holds
    /// `transcript().iter()` across the `&mut` cache.
    pub(crate) fn append_block_lines(
        &mut self,
        i: usize,
        width: usize,
        out: &mut Vec<Line<'static>>,
    ) -> Range<usize> {
        // Defensive parity: a missed `push_block`/`insert_block` (or a test that
        // edits `transcript` directly) must re-render, never index out of bounds.
        if i >= self.cache.len() {
            self.cache.resize_with(i + 1, CacheEntry::never);
        }
        if i >= self.block_revs.len() {
            self.block_revs.resize(i + 1, 0);
        }
        let rev = self.block_revs[i];
        if self.cache[i].rev != rev || self.cache[i].width != width {
            let rendered = crate::ui::block_lines(&self.transcript[i], width);
            self.cache[i] = CacheEntry {
                rev,
                width,
                lines: rendered,
            };
            #[cfg(test)]
            {
                self.cache_misses += 1;
            }
        }
        // Clone the cached lines — the entry stays populated for the next frame (a
        // hit must still append; taking/emptying it would make the block vanish).
        let lines = &self.cache[i].lines;
        if i > 0 && !out.is_empty() && !lines.is_empty() {
            out.push(Line::default());
        }
        let start = out.len();
        let len = lines.len();
        out.extend(lines.iter().cloned());
        start..start + len
    }

    /// Bump block `i`'s revision (saturating). Call at every `Block::Tool`
    /// mutation so the next frame re-renders that block.
    fn bump_rev(&mut self, i: usize) {
        if let Some(rev) = self.block_revs.get_mut(i) {
            *rev = rev.saturating_add(1);
        }
    }

    /// Push a committed block, keeping `cache`/`block_revs` index-aligned with
    /// `transcript`. Every transcript push goes through here.
    fn push_block(&mut self, block: Block) {
        self.transcript.push(block);
        self.block_revs.push(0);
        self.cache.push(CacheEntry::never());
    }

    /// Insert a committed block mid-transcript, shifting all three parallel vecs
    /// together. A mid-vec insert SHIFTS every later index, so a lazy
    /// length-reconcile would be unsound.
    fn insert_block(&mut self, at: usize, block: Block) {
        self.transcript.insert(at, block);
        self.block_revs.insert(at, 0);
        self.cache.insert(at, CacheEntry::never());
    }

    /// Cache misses so far (each is a `ui::block_lines` render). Test-only; the
    /// renderer never reads it.
    #[cfg(test)]
    pub(crate) fn cache_misses(&self) -> usize {
        self.cache_misses
    }

    /// The team strip/`/team` state, derived from the run flags.
    fn state(&self) -> TeamState {
        if self.failed {
            TeamState::Failed
        } else if self.running {
            TeamState::Running
        } else if self.finished {
            TeamState::Done
        } else {
            TeamState::Idle
        }
    }

    /// Apply one agent event. Returns `true` when the surface changed, so `App`
    /// can mark itself dirty (the surface owns no `dirty` flag). `action_seq`
    /// is the app-wide monotonic clock for stamping action recency.
    fn apply(&mut self, event: AgentEvent, action_seq: &mut u64) -> bool {
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
            AgentEvent::ToolExecutionStart { call_id, name } => {
                self.flush_live();
                // The committed assistant block carries this call's arguments
                // (§3), so resolve a short "name target" label for the team strip.
                self.last_action = Some(action_label(&self.transcript, &call_id, &name));
                self.last_action_at = Some(*action_seq);
                *action_seq += 1;
                self.push_block(Block::Tool(Tool {
                    name,
                    output: String::new(),
                    done: false,
                    is_error: false,
                    expanded: false,
                    diff: None,
                    path: None,
                    duration_ms: None,
                }));
                true
            }
            AgentEvent::ToolExecutionUpdate { partial, .. } => {
                let idx = self.transcript.len().wrapping_sub(1);
                let changed = if let Some(Block::Tool(tool)) = self.transcript.last_mut() {
                    tool.output.push_str(&partial);
                    true
                } else {
                    false
                };
                if changed {
                    self.bump_rev(idx);
                }
                changed
            }
            AgentEvent::ToolExecutionEnd {
                output,
                is_error,
                diff,
                path,
                duration_ms,
                ..
            } => {
                let idx = self.transcript.len().wrapping_sub(1);
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
                    tool.duration_ms = duration_ms;
                    true
                } else {
                    false
                };
                if matched {
                    self.bump_rev(idx);
                }
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
                self.failed = false; // a new run clears the previous failure
                self.plan_pending = None; // a run supersedes any settled toggle
                self.last_action = None;
                true
            }
            AgentEvent::AgentEnd => {
                self.flush_live();
                self.running = false;
                self.finished = true;
                self.last_action = None;
                if self.cancelled {
                    self.cancelled = false;
                    self.push_block(Block::Notice("⏹ aborted".into()));
                }
                // Surface the run's changes once it settles; they stay for `/changes`.
                if !self.changes.is_empty() {
                    self.push_block(Block::Notice(self.changes_summary()));
                }
                true
            }
            AgentEvent::Error { message } => {
                self.flush_live();
                // A run failure (the loop emits `Error` before `AgentEnd`) marks the
                // member failed; an idle reply error (SetModel/Compact/…) does not.
                // `failed` is never cleared here — only `AgentStart` clears it.
                if self.running {
                    self.failed = true;
                }
                // A pending optimistic `/plan` toggle whose reply errored: revert.
                if let Some(prev) = self.plan_pending.take() {
                    self.status.plan = prev;
                }
                self.push_block(Block::Error(message));
                true
            }
            AgentEvent::Todo { todos } => {
                // Minimal v1: a notice. A dedicated panel is a follow-on. This arm
                // runs for a local run AND a socket client — the point of routing
                // `todo` through the event seam.
                self.last_todos = Some(todos.clone());
                self.push_block(Block::Notice(render_todos(&todos)));
                true
            }
            AgentEvent::CompactionSkipped { reason } => {
                // Non-fatal: a best-effort auto-compaction miss. Surface it as a
                // Notice but do NOT set `self.failed` — that flag is set only on a
                // run-failure `Error` while running (app.rs:tT4I8), so routing
                // this here would falsely mark the run failed.
                self.push_block(Block::Notice(format!("⋯ compaction skipped: {reason}")));
                true
            }
            AgentEvent::Compaction { summarized, kept } => {
                self.push_block(Block::Notice(format!(
                    "⋯ compacted {summarized} messages, kept {kept}"
                )));
                true
            }
            AgentEvent::Retrying {
                attempt,
                max,
                reason,
            } => {
                self.push_block(Block::Notice(format!(
                    "⋯ retrying ({attempt}/{max}): {reason}"
                )));
                true
            }
            AgentEvent::TurnEnd { message } => self.record_usage(&message),
            AgentEvent::SideAnswer { text, .. } => {
                self.push_block(Block::Btw(text)); // display-only; no ctx/session
                true
            }
            AgentEvent::History { messages } => {
                self.render_usage(&messages);
                true
            }
            // A successful non-streamed reply settles any optimistic `/plan`
            // toggle, so a later unrelated error cannot revert the chip.
            AgentEvent::Ack => {
                self.plan_pending = None;
                false
            }
            // Other start/reply events need no per-surface state.
            _ => false,
        }
    }

    /// Commit an assistant message. Tool calls ride the block (the team strip reads
    /// their arguments, §3; the renderer ignores them); empty messages are not
    /// committed.
    fn commit(&mut self, message: AgentMessage) {
        if let AgentMessage::Assistant { content, .. } = message
            && !content.is_empty()
        {
            self.push_block(Block::Assistant(content));
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
        self.push_block(Block::Notice(text.into()));
    }

    /// Render a `GetHistory` reply as a one-line usage summary.
    fn render_usage(&mut self, messages: &[AgentMessage]) {
        let stats = session_stats(messages);
        // Keep the live context-fullness readout the status line consumes.
        if let Some(used) = stats.last_input_tokens {
            self.context_used = Some(used);
        }
        // `summary()` carries the shared body; the TUI appends its own context
        // chrome (the REPL prints context on a separate line instead).
        let mut line = stats.summary();
        if let (Some(used), Some(limit)) = (stats.last_input_tokens, self.status.context_limit) {
            line.push_str(&format!(", context {used}/{limit}"));
        }
        self.push_notice(line);
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
                        self.push_block(Block::User(text));
                    }
                }
                AgentMessage::Assistant { content, .. } => {
                    // The tool calls ride the block so a member's team strip action
                    // can name the call's target (§3); the renderer ignores them.
                    let visible = content.to_vec();
                    if !visible.is_empty() {
                        self.push_block(Block::Assistant(visible));
                    }
                    self.record_usage(message);
                }
                AgentMessage::ToolResult {
                    name,
                    output,
                    is_error,
                    ..
                } => {
                    self.push_block(Block::Tool(Tool {
                        name: name.clone(),
                        output: output.clone(),
                        done: true,
                        is_error: *is_error,
                        expanded: *is_error,
                        diff: None,
                        path: None,
                        duration_ms: None,
                    }));
                }
            }
        }
        // Mark where the replayed prefix ends, mirroring the REPL's divider.
        // Goes through `insert_block` so `cache`/`block_revs` shift WITH
        // `transcript` (a mid-vec insert shifts every later index).
        self.insert_block(
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

/// The app's top-level input mode. `Input` is the composer (the default);
/// `Browse` moves a selection over the committed transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Input,
    Browse,
}

/// The open transcript search prompt (`/` in browse). Browse-owned rather
/// than an `Overlay` so `n`/`N` can repeat the last jump after the prompt
/// closes; like a modal it still owns every key while up.
#[derive(Clone, Debug, Default)]
struct BrowseSearch {
    /// The raw term, as typed. Filters the block list live.
    query: String,
}

/// The visible-viewport edge a browse `Home` / `End` anchors to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewEdge {
    Top,
    Bottom,
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
    /// The orchestrator's plan — injected by the composition root and refreshed
    /// live by the `new_tasks` feed to `run`.
    tasks: Vec<TaskItem>,
    /// Set when the user picks a session to resume; the run returns it so the CLI
    /// can re-exec with `--resume <path>`.
    pending_resume: Option<PathBuf>,
    /// Set by `/reload`; the run returns it so the CLI can rebuild + re-exec
    /// into the same session (`no_session` = start fresh with `--no-session`).
    pending_reload: Option<bool>,
    /// True when this TUI is a `--socket` client: there is no local binary or
    /// session to rebuild/re-exec, so `/reload` is refused.
    remote: bool,
    /// The open modal, if any. While one is shown it captures every key.
    overlay: Option<Overlay>,
    /// The inline command-completion popup, if one is showing. Recomputed after
    /// every buffer edit; non-modal (it never owns the keyboard — see [`Completion`]).
    completion: Option<Completion>,
    dirty: bool,
    should_quit: bool,
    actions: Vec<Action>,
    /// Monotonic clock stamping the last action of each member, so the team
    /// strip can order members active-first then by action recency.
    action_seq: u64,
    /// The active input mode (composer vs transcript browse).
    mode: Mode,
    /// The open browse search prompt, if any — see [`BrowseSearch`].
    search: Option<BrowseSearch>,
    /// The term the last search jumped on; `n`/`N` repeat it in browse.
    last_search: Option<String>,
    /// The process cwd (`repo`), shown last in the status line (drops first).
    /// Injected at startup, like `models`/`sessions`.
    cwd: Option<String>,
    /// `branch`, with a trailing `*` when the worktree is dirty. Display-only;
    /// never refreshed after startup.
    git: Option<String>,
    /// The docked left sidebar is open. OFF by default, so the base layout
    /// (transcript · rule · [team strip] · input · status) is byte-identical
    /// while it is closed. Toggled only by `Ctrl-B` (see `on_key` / `KEYS`).
    /// It is view state like `mode`, not surface state: `toggle_sidebar`
    /// flips this bit and marks dirty, never touching a surface. The renderer
    /// keys its horizontal split on `App::sidebar()` (see `ui::draw`).
    sidebar: bool,
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
            tasks: Vec::new(),
            pending_resume: None,
            pending_reload: None,
            remote: false,
            overlay: None,
            completion: None,
            dirty: true,
            should_quit: false,
            actions: Vec::new(),
            action_seq: 0,
            mode: Mode::Input,
            search: None,
            last_search: None,
            cwd: None,
            git: None,
            // OFF by default: the base three-band layout is byte-identical
            // until the user hits Ctrl-B.
            sidebar: false,
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

    /// Expand or collapse every tool block on the focused surface — the
    /// all-tools complement to browse mode's per-block toggle. All expanded
    /// collapses; anything else expands all.
    fn toggle_all_tools(&mut self) {
        let indices: Vec<usize> = self
            .focused()
            .transcript
            .iter()
            .enumerate()
            .filter_map(|(i, block)| matches!(block, Block::Tool(_)).then_some(i))
            .collect();
        if indices.is_empty() {
            return;
        }
        // All expanded collapses; anything else expands all.
        let expand = !indices
            .iter()
            .all(|&i| matches!(&self.focused().transcript[i], Block::Tool(tool) if tool.expanded));
        for i in indices {
            if let Some(Block::Tool(tool)) = self.focused_mut().transcript.get_mut(i) {
                tool.expanded = expand;
            }
            self.focused_mut().bump_rev(i);
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

    /// The RUNNING teammates as `(label, state, action)` — the rows the team
    /// region above the input box shows. Ordered OLDEST→NEWEST by
    /// `last_action_at`, so the LATEST event is the BOTTOM row, capped to the 3
    /// MOST RECENT. The root is excluded (it is the orchestrator, not a
    /// teammate); idle/done/failed members are dropped entirely.
    ///
    /// [`App::member_rows`] cannot serve this: it drops `last_action_at` at its
    /// sort and keeps every state (active-first). The glyph comes from
    /// `state.glyph()`. `last_action` is `Some("{tool} {target}")` while a run
    /// has a live tool (§3), else `None`.
    ///
    /// RESISTANCE: [`App::run_elapsed`] is the FOCUSED surface only, so a
    /// non-focused running teammate's row cannot show a per-member elapsed — the
    /// same limitation `draw_sidebar` documents.
    pub fn working_team_rows(&self) -> Vec<(&str, TeamState, Option<&str>)> {
        let mut rows: Vec<_> = self
            .surfaces
            .iter()
            .filter(|s| !s.is_root && s.state() == TeamState::Running)
            .map(|s| {
                (
                    s.label.as_str(),
                    s.state(),
                    s.last_action.as_deref(),
                    s.last_action_at,
                )
            })
            .collect();
        // oldest → newest (the BOTTOM row is the most recent); stable on ties.
        rows.sort_by_key(|(_, _, _, at)| at.unwrap_or(0));
        let keep = rows.len().saturating_sub(3); // drop all but the 3 MOST RECENT
        rows.into_iter()
            .skip(keep)
            .map(|(label, state, action, _)| (label, state, action))
            .collect()
    }
    /// The non-root surfaces as `(label, state, focused, action)` — the team
    /// strip and `/team`. Ordered active-first, then by most recent action
    /// (stable: surface order breaks ties). The action is the current run's
    /// live tool label, if any (§3).
    pub fn member_rows(&self) -> Vec<(&str, TeamState, bool, Option<&str>)> {
        let mut rows: Vec<_> = self
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_root)
            .map(|(i, s)| {
                (
                    s.label.as_str(),
                    s.state(),
                    i == self.focus,
                    s.last_action.as_deref(),
                    s.last_action_at,
                )
            })
            .collect();
        rows.sort_by_key(|(_, state, _, _, at)| (*state != TeamState::Running, std::cmp::Reverse(at.unwrap_or(0))));
        rows.into_iter()
            .map(|(label, state, focused, action, _)| (label, state, focused, action))
            .collect()
    }

    /// The active input mode (composer vs transcript browse).
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The focused surface's selected committed-block index, if any.
    pub fn selected(&self) -> Option<usize> {
        self.focused().selected
    }

    /// The selected block's line range from the last measured frame.
    pub(crate) fn selected_range(&self) -> Option<Range<usize>> {
        let surface = self.focused();
        surface.selected.and_then(|i| surface.ranges.get(i).cloned())
    }

    /// The transcript row count the renderer measured on the previous frame.
    pub(crate) fn total_lines(&self) -> usize {
        self.focused().last_total
    }

    /// Record each committed block's line range from the frame just built — the
    /// renderer's feedback channel for the selection bar. Clamps a stale
    /// selection to the last block (or clears it when the transcript is empty).
    pub fn set_block_ranges(&mut self, ranges: Vec<Range<usize>>) {
        let len = ranges.len();
        let surface = self.focused_mut();
        surface.ranges = ranges;
        if let Some(i) = surface.selected {
            if len == 0 {
                surface.selected = None;
            } else if i >= len {
                surface.selected = Some(len - 1);
            }
        }
    }

    /// Enter transcript browse (`Ctrl-G`): select the last committed block. The
    /// composer draft and cursor are untouched; the completion popup is closed
    /// so browse owns the arrows.
    fn enter_browse(&mut self) {
        let last = self.focused().transcript.len().checked_sub(1);
        self.mode = Mode::Browse;
        self.completion = None;
        self.focused_mut().selected = last;
        self.reveal_selected();
        self.dirty = true;
    }

    /// Leave browse (`Esc` / `q` / `Ctrl-G`).
    fn exit_browse(&mut self) {
        self.mode = Mode::Input;
        self.focused_mut().selected = None;
        // An open search prompt never survives browse (defensive — the prompt
        // keys own Esc, so this only fires when it is already closed).
        self.search = None;
        self.dirty = true;
    }

    /// Handle a key while browsing. Browse owns the text and navigation keys;
    /// `F1`/`?` (help) and `Ctrl-C` (cancel / quit) stay **global**, and
    /// `Esc`/`q`/`Ctrl-G` leave. Every other key is deliberately ignored — the
    /// composer must not be edited behind the mode.
    fn on_browse_key(&mut self, key: Key) {
        // An open search prompt owns the keys: typing filters live, `Enter`
        // jumps and closes, `Esc` closes without jumping. The global keys
        // (`F1`/`?`, `Ctrl-C`) stay global here exactly as in plain browse.
        if self.search.is_some() {
            self.on_search_key(key);
            return;
        }
        match key {
            Key::Esc | Key::Ctrl('g') | Key::Char('q') => self.exit_browse(),
            // Global: the keymap overlay and the panic button. Neither leaves
            // browse — closing help returns to the mode you opened it from.
            Key::F(1) | Key::Char('?') => self.open_help(),
            Key::Ctrl('c') => self.interrupt(),
            // Act on the selected block.
            Key::Enter | Key::Char(' ') => self.toggle_selected(),
            Key::Char('y') => self.copy_selected(),
            Key::Char('j') | Key::Down => self.select_by(1),
            Key::Char('k') | Key::Up => self.select_by(-1),
            Key::Char('g') => self.select_first(),
            Key::Char('G') => self.select_last(),
            Key::PageDown => self.select_by(self.page() as isize),
            Key::PageUp => self.select_by(-(self.page() as isize)),
            // Viewport-anchored: the topmost / bottommost block in view.
            Key::Home => self.select_viewport_edge(ViewEdge::Top),
            Key::End => self.select_viewport_edge(ViewEdge::Bottom),
            // Block-wise movement, clamped at the ends. Braces arrive as `Alt`
            // chords on some layouts (macOS Option-8/9, German AltGr), so
            // both spellings step the same way.
            Key::Char('{') | Key::Alt('{') => self.select_by(-1),
            Key::Char('}') | Key::Alt('}') => self.select_by(1),
            // Search: `/` opens the prompt; `n` / `N` repeat the last jump.
            Key::Char('/') => self.open_search(),
            Key::Char('n') => self.search_repeat(1),
            Key::Char('N') => self.search_repeat(-1),
            // The wheel scrolls the view without moving the selection.
            Key::ScrollUp => self.scroll_up(WHEEL_LINES),
            Key::ScrollDown => self.scroll_down(WHEEL_LINES),
            _ => {}
        }
    }

    /// Keys while the search prompt is open. `Char` appends to the query and
    /// `Backspace` pops — exactly how the picker edits — while `Enter` jumps
    /// and closes and `Esc` closes without jumping. `F1`/`?` (help) and
    /// `Ctrl-C` (cancel / quit) stay **global**, like plain browse.
    fn on_search_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.close_search(),
            Key::F(1) | Key::Char('?') => self.open_help(),
            Key::Ctrl('c') => self.interrupt(),
            Key::Enter => self.accept_search(),
            Key::Char(c) => {
                if let Some(search) = self.search.as_mut() {
                    search.query.push(c);
                }
                self.dirty = true;
            }
            Key::Backspace => {
                if let Some(search) = self.search.as_mut() {
                    search.query.pop();
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// `/` in browse: open the search prompt with an empty query.
    fn open_search(&mut self) {
        self.search = Some(BrowseSearch::default());
        self.dirty = true;
    }

    /// `Esc` in the search prompt: close it without jumping.
    fn close_search(&mut self) {
        self.search = None;
        self.dirty = true;
    }

    /// `Enter` in the search prompt: jump to the first committed block at/after
    /// the selection whose copy-text contains the term (case-insensitive,
    /// wrapping), remember the term for `n`/`N`, and close. An empty term or no
    /// match is a no-op — nothing moves and the prompt stays up.
    fn accept_search(&mut self) {
        let term = self.search.as_ref().map(|s| s.query.clone());
        let Some(term) = term else { return };
        // At/after the selection: a matching selection stays put.
        let base = self.focused().selected.unwrap_or(0);
        if let Some(idx) = self.search_target(&term, 1, base, false) {
            self.last_search = Some(term);
            self.focused_mut().selected = Some(idx);
            self.reveal_selected();
            self.close_search();
        }
    }

    /// `n` / `N` in browse: repeat the remembered search from the current
    /// selection, wrapping. Nothing remembered, or no further match, is a
    /// no-op.
    fn search_repeat(&mut self, dir: isize) {
        let Some(term) = self.last_search.clone() else { return };
        // Strictly next/previous: `n`/`N` must move off a matching selection.
        // No selection means the search starts fresh at the nearest end.
        let len = self.focused().transcript.len();
        let (base, strict) = match self.focused().selected {
            Some(i) => (i, true),
            None => (if dir > 0 { 0 } else { len }, false),
        };
        if let Some(idx) = self.search_target(&term, dir, base, strict) {
            self.focused_mut().selected = Some(idx);
            self.reveal_selected();
            self.dirty = true;
        }
    }

    /// The first index in `matches` at/after the selection for `dir` = 1, or
    /// at/before it for -1, wrapping around the ends. `None` when the term
    /// matches nothing. With no selection the search starts at the nearest end.
    fn search_target(&self, term: &str, dir: isize, base: usize, strict: bool) -> Option<usize> {
        let matches = self.matching_blocks(term);
        if matches.is_empty() {
            return None;
        }
        if dir > 0 {
            // First match at/after `base` (or strictly after), else wrap to
            // the first match overall.
            matches
                .iter()
                .copied()
                .find(|&i| if strict { i > base } else { i >= base })
                .or_else(|| matches.first().copied())
        } else {
            // Last match at/before `base` (or strictly before), else wrap to
            // the last match overall.
            matches
                .iter()
                .rev()
                .copied()
                .find(|&i| if strict { i < base } else { i <= base })
                .or_else(|| matches.last().copied())
        }
    }

    /// Indices of the committed blocks whose copy-text contains `term`,
    /// case-insensitively. An empty term matches nothing (every block would
    /// otherwise contain it).
    fn matching_blocks(&self, term: &str) -> Vec<usize> {
        if term.is_empty() {
            return Vec::new();
        }
        let term = term.to_lowercase();
        let mut hits = Vec::new();
        for (i, block) in self.focused().transcript.iter().enumerate() {
            if copy_text(block).is_some_and(|text| text.to_lowercase().contains(&term)) {
                hits.push(i);
            }
        }
        hits
    }

    /// The open search query, if the prompt is up (read by the renderer).
    pub(crate) fn search_query(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.query.as_str())
    }

    /// How many committed blocks match the open query — the live filter count.
    pub(crate) fn search_hits(&self) -> usize {
        self.search.as_ref().map_or(0, |s| self.matching_blocks(&s.query).len())
    }

    /// `Home` / `End` in browse: select the topmost / bottommost committed
    /// block whose line range intersects the visible viewport, then bring it
    /// into view. A block that renders no lines is skipped; before the renderer
    /// has measured a frame, or when nothing intersects the view, it falls back
    /// to the transcript ends (`g` / `G`).
    fn select_viewport_edge(&mut self, edge: ViewEdge) {
        // No measured view yet — there is nothing to anchor to, so behave
        // like `g`/`G` (which is also exactly right when everything fits).
        if self.focused().viewport == 0 || self.focused().last_total == 0 {
            match edge {
                ViewEdge::Top => self.select_first(),
                ViewEdge::Bottom => self.select_last(),
            }
            return;
        }
        let surface = self.focused();
        let top = surface
            .last_total
            .saturating_sub(surface.viewport)
            .saturating_sub(surface.scroll);
        let bottom = top.saturating_add(surface.viewport).min(surface.last_total);
        let visible = surface.ranges.iter().enumerate().filter(|(_, range)| {
            let (b_start, b_end) = (range.start, range.end);
            // Renders lines, and intersects the visible window.
            b_end > b_start && b_end > top && b_start < bottom
        });
        if let Some(idx) = match edge {
            ViewEdge::Top => visible.map(|(i, _)| i).next(),
            ViewEdge::Bottom => visible.map(|(i, _)| i).next_back(),
        } {
            self.focused_mut().selected = Some(idx);
            self.reveal_selected();
            self.dirty = true;
        } else {
            // Nothing intersects the view (e.g. a sparse transcript): fall
            // back to the transcript ends.
            match edge {
                ViewEdge::Top => self.select_first(),
                ViewEdge::Bottom => self.select_last(),
            }
        }
    }

    /// Move the selection by `delta` blocks, clamped to the transcript; then
    /// bring it into view.
    fn select_by(&mut self, delta: isize) {
        let len = self.focused().transcript.len();
        if len == 0 {
            self.focused_mut().selected = None;
            return;
        }
        let next = match self.focused().selected {
            None => 0, // nothing selected yet: start at the first block
            Some(i) => (i as isize + delta).clamp(0, len as isize - 1) as usize,
        };
        self.focused_mut().selected = Some(next);
        self.reveal_selected();
        self.dirty = true;
    }

    /// Select the first committed block.
    fn select_first(&mut self) {
        if !self.focused().transcript.is_empty() {
            self.focused_mut().selected = Some(0);
            self.reveal_selected();
            self.dirty = true;
        }
    }

    /// Select the last committed block.
    fn select_last(&mut self) {
        let len = self.focused().transcript.len();
        if len > 0 {
            self.focused_mut().selected = Some(len - 1);
            self.reveal_selected();
            self.dirty = true;
        }
    }

    /// `Enter` / `Space` in browse: toggle the selected block's detail. Only a
    /// tool block has expansion state today — a no-op elsewhere, and silent (no
    /// notice noise).
    fn toggle_selected(&mut self) {
        let Some(idx) = self.focused().selected else {
            return;
        };
        let toggled = if let Some(Block::Tool(tool)) = self.focused_mut().transcript.get_mut(idx) {
            tool.expanded = !tool.expanded;
            true
        } else {
            false
        };
        if toggled {
            self.focused_mut().bump_rev(idx);
            self.dirty = true;
        }
    }

    /// `y` in browse: copy the selected block, mirroring `copy_last` (push the
    /// `Action::Copy` and a `copied N chars to the clipboard` notice). Nothing to
    /// copy becomes a notice, with no action pushed.
    fn copy_selected(&mut self) {
        let selected = self.focused().selected;
        let text = selected
            .and_then(|idx| self.focused().transcript.get(idx))
            .and_then(copy_text);
        match text {
            Some(text) => {
                let chars = text.chars().count();
                self.actions.push(Action::Copy(text));
                self.notice(format!("copied {chars} chars to the clipboard"));
            }
            None => self.notice("nothing to copy"),
        }
    }

    /// Adjust `scroll` so the selected block is visible, biased to its top (a
    /// block taller than the viewport shows its first line). A no-op before the
    /// renderer has measured a frame, or when the block is already fully in view.
    pub(crate) fn reveal_selected(&mut self) {
        let surface = self.focused_mut();
        let Some(idx) = surface.selected else {
            return;
        };
        let Some(range) = surface.ranges.get(idx).cloned() else {
            return;
        };
        let (height, total) = (surface.viewport, surface.last_total);
        if height == 0 || total == 0 {
            return;
        }
        let (b_start, b_end) = (range.start, range.end.min(total));
        let top = total.saturating_sub(height).saturating_sub(surface.scroll);
        let bottom = top.saturating_add(height).min(total);
        let target = if b_end.saturating_sub(b_start) > height || b_start < top {
            // A tall block, or its top is off-screen above: show its top.
            total.saturating_sub(height).saturating_sub(b_start)
        } else if b_end > bottom {
            // Its bottom is below the window: scroll down to show it.
            total.saturating_sub(b_end)
        } else {
            return;
        };
        surface.scroll = target.min(surface.max_scroll);
    }

    /// Apply one event. Pure state transition; sets [`App::dirty`] on a change.
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Paste(text) => {
                // A modal owns the input: a paste must not edit the buffer.
                // With the browse search prompt up (not an `Overlay`, so its
                // guard would miss it) the paste is appended to the *query*
                // instead — the composer draft and cursor stay untouched.
                if self.overlay.is_none() {
                    if let Some(search) = self.search.as_mut() {
                        search.query.push_str(&text);
                        self.dirty = true;
                    } else {
                        self.on_paste(text);
                    }
                }
            }
            AppEvent::Agent(id, event) => {
                // Route to the surface that owns this session; an unknown id is
                // ignored (a stray/duplicate event must not panic).
                if let Some(idx) = self.surface_index(&id)
                    && self.surfaces[idx].apply(event, &mut self.action_seq)
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
        // Browse owns the keyboard: `Esc`/`q`/`Ctrl-G` leave it, and no key
        // reaches the composer — so `Esc` here never cancels or quits.
        if self.mode == Mode::Browse {
            self.on_browse_key(key);
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
            Key::Ctrl('g') => self.enter_browse(),
            // Ctrl-B docks/undocks the left sidebar. Reached only in INPUT
            // mode: `on_key`'s head already returned for an open overlay, an
            // open completion, and browse — so Ctrl-B is deliberately inert
            // while any of those owns the keyboard.
            Key::Ctrl('b') => self.toggle_sidebar(),
            // Tool detail: Ctrl-T toggles every tool at once; the per-block
            // toggle lives in browse mode, where the target is drawn.
            Key::Ctrl('t') => self.toggle_all_tools(),
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
        self.focused_mut().push_block(Block::User(text.clone()));
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
            "btw" => match arg {
                Some(q) => self
                    .actions
                    .push(Action::Ask(Request::SideAsk { text: q.to_string() })),
                None => self.notice("usage: /btw <question>"),
            },
            "plan" => {
                let prev = self.focused().status.plan;
                let on = match arg {
                    None => !prev, // bare `/plan` toggles
                    Some("on") => true,
                    Some("off") => false,
                    Some(_) => {
                        self.notice("usage: /plan [on|off]");
                        return;
                    }
                };
                // Optimistic; reverted if the reply is an `Error` (amendment 8).
                let surface = self.focused_mut();
                surface.plan_pending = Some(prev);
                surface.status.plan = on;
                self.actions.push(Action::Ask(Request::SetPlanMode { on }));
            }
            "verify" => self.notice(self.verify_text()),
            "usage" => self.actions.push(Action::Ask(Request::GetHistory)),
            "changes" => self.open_changes_picker(),
            "resume" => self.open_session_picker(arg),
            "reload" => match arg {
                None => self.request_reload(false),
                Some("--no-session") => self.request_reload(true),
                Some(_) => self.notice("usage: /reload [--no-session]"),
            },
            "copy" => self.copy_last(),
            "team" => self.notice(team_text(&self.member_rows())),
            "tasks" => self.notice(tasks_text(&self.tasks)),
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

    /// `/verify`: render the cached `AgentEvent::Todo` checklist (a client view of
    /// the last emitted list — never `Session::todo()`), the done count, the
    /// remaining items, and the last assistant reply + this run's changed files.
    fn verify_text(&self) -> String {
        let surface = self.focused();
        let mut out = match &surface.last_todos {
            None => "(no plan/todos recorded yet)".to_string(),
            Some(list) if list.is_empty() => "(no todos — the plan list is empty)".to_string(),
            Some(list) => {
                let done = list
                    .iter()
                    .filter(|t| t.status == TodoStatus::Completed)
                    .count();
                let remaining: Vec<&str> = list
                    .iter()
                    .filter(|t| t.status != TodoStatus::Completed)
                    .map(|t| t.content.as_str())
                    .collect();
                let mut s = render_todos(list);
                s.push_str(&format!("\n{done}/{} done", list.len()));
                if remaining.is_empty() {
                    s.push_str("\nfinished — all items completed");
                } else {
                    s.push_str(&format!(
                        "\nnot finished — {} remaining: {}",
                        remaining.len(),
                        remaining.join(", ")
                    ));
                }
                s
            }
        };
        if let Some(text) = surface.last_assistant_text() {
            out.push_str("\n— last reply —\n");
            out.push_str(&text);
        }
        if !surface.changes.is_empty() {
            out.push_str("\n— changed files —\n");
            out.push_str(&surface.changes_summary());
        }
        out
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
        self.focused_mut().push_block(Block::Diff {
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
            .map(|s| format!("{} · {}", s.label, s.status.model))
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

    /// Replace the plan when the composition root (or the `new_tasks` feed)
    /// pushes one. Mirrors [`Self::set_sessions`]; marks dirty so the next frame
    /// redraws.
    pub fn set_tasks(&mut self, tasks: Vec<TaskItem>) {
        self.tasks = tasks;
        self.dirty = true;
    }

    /// The session the user chose to resume, if any — read by the event loop
    /// after it exits so the composition root can re-exec with `--resume`.
    pub fn pending_resume(&self) -> Option<&Path> {
        self.pending_resume.as_deref()
    }

    /// Set by `/reload` when the run should rebuild + re-exec into this session
    /// instead of quitting (`Some(no_session)`).
    pub fn pending_reload(&self) -> Option<bool> {
        self.pending_reload
    }

    /// Mark this TUI as a `--socket` client, so `/reload` is refused.
    pub fn set_remote(&mut self, remote: bool) {
        self.remote = remote;
    }

    /// `/reload [--no-session]`: ask the composition root to rebuild + re-exec
    /// into this session (the REPL's semantics). Over a socket there is no local
    /// binary or session to re-exec, so refuse as the REPL does.
    fn request_reload(&mut self, no_session: bool) {
        if self.remote {
            self.notice("/reload (rebuild + re-exec) is unavailable over a socket");
            return;
        }
        self.pending_reload = Some(no_session);
        self.should_quit = true;
    }

    /// The run's changed files so far (view-owned changeset), for the status
    /// chip — a thin accessor over `Surface::changes` (`changes_by_path` does the
    /// summing). Empty when nothing changed this run.
    pub fn changes(&self) -> &[Change] {
        &self.focused().changes
    }

    /// The latest `Todo` list, for the status chip (`☑ done/total`) — an
    /// accessor over `Surface::last_todos` (set in the `Todo` arm).
    pub fn last_todos(&self) -> Option<&[TodoItem]> {
        self.focused().last_todos.as_deref()
    }

    /// Time since this surface's run started — INJECTED by the loop (see
    /// `Surface::run_elapsed`); NOT computed here (purity).
    pub fn run_elapsed(&self) -> Option<std::time::Duration> {
        self.focused().run_elapsed
    }

    /// Set the elapsed time for surface `id` — an injection setter (like
    /// `set_models`), called by the event loop from its own clock, NOT the
    /// reducer. Marks the app dirty so the status line repaints.
    pub fn set_run_elapsed(&mut self, id: &SessionId, elapsed: std::time::Duration) {
        if let Some(i) = self.surface_index(id) {
            self.surfaces[i].run_elapsed = Some(elapsed);
            self.dirty = true;
        }
    }
    /// The process cwd (`repo`), for the status line.
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// The branch, with a `*` dirty marker, for the status line.
    pub fn git(&self) -> Option<&str> {
        self.git.as_deref()
    }

    /// Toggle the docked left sidebar (`Ctrl-B`). A pure reducer step: flip
    /// the view bit and mark the app dirty so the next frame repaints the
    /// split. Touches no surface and no transcript.
    fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
        self.dirty = true;
    }

    /// Whether the docked left sidebar is open — the renderer's split key
    /// (`ui::draw` reads it). A read-only view accessor; it never mutates.
    pub fn sidebar(&self) -> bool {
        self.sidebar
    }
    /// Inject the startup cwd (see [`Options::cwd`]); marks the app dirty.
    pub fn set_cwd(&mut self, cwd: Option<String>) {
        self.cwd = cwd;
        self.dirty = true;
    }

    /// Inject the startup branch/dirty marker (see `Options::git`); marks dirty.
    pub fn set_git(&mut self, git: Option<String>) {
        self.git = git;
        self.dirty = true;
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
    fn tool_execution_end_carries_duration_onto_the_block() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionStart {
            call_id: "t1".into(), name: "read".into(),
        }));
        app.handle(AppEvent::Agent(root(), AgentEvent::ToolExecutionEnd {
            call_id: "t1".into(), name: "read".into(),
            output: "128 lines".into(), is_error: false,
            diff: None, path: None, duration_ms: Some(12),
        }));
        match app.transcript().last() {
            Some(Block::Tool(tool)) => assert_eq!(tool.duration_ms, Some(12)),
            other => panic!("expected a tool block, got {other:?}"),
        }
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
            duration_ms: None,
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
    fn a_todo_event_renders_a_notice() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::Todo {
                todos: vec![
                    TodoItem {
                        content: "step one".into(),
                        status: TodoStatus::Pending,
                    },
                    TodoItem {
                        content: "step two".into(),
                        status: TodoStatus::Completed,
                    },
                ],
            },
        ));
        match app.transcript().last() {
            Some(Block::Notice(text)) => {
                assert!(text.contains("[ ] step one"), "{text}");
                assert!(text.contains("[x] step two"), "{text}");
            }
            other => panic!("expected a todo notice, got {other:?}"),
        }
    }

    #[test]
    fn verify_renders_the_checklist_and_flags_unfinished() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::Todo {
                todos: vec![
                    TodoItem {
                        content: "step one".into(),
                        status: TodoStatus::Completed,
                    },
                    TodoItem {
                        content: "step two".into(),
                        status: TodoStatus::Pending,
                    },
                ],
            },
        ));
        submit(&mut app, "/verify");
        let Some(Block::Notice(text)) = app.transcript().last() else {
            panic!("expected a verify notice");
        };
        assert!(text.contains("[x] step one"), "{text}");
        assert!(text.contains("[ ] step two"), "{text}");
        assert!(text.contains("1/2 done"), "{text}");
        assert!(text.contains("not finished"), "{text}");
    }

    #[test]
    fn verify_when_all_items_are_done_reports_finished() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::Todo {
                todos: vec![TodoItem {
                    content: "only step".into(),
                    status: TodoStatus::Completed,
                }],
            },
        ));
        submit(&mut app, "/verify");
        let Some(Block::Notice(text)) = app.transcript().last() else {
            panic!("expected a verify notice");
        };
        assert!(text.contains("1/1 done"), "{text}");
        assert!(text.contains("finished"), "{text}");
        assert!(!text.contains("not finished"), "{text}");
    }

    #[test]
    fn verify_without_todos_reports_none() {
        let mut app = App::new();
        submit(&mut app, "/verify");
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("no plan/todos recorded")
        ));
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
        // The tool call rides the assistant block — the renderer ignores it, the
        // team strip reads its target (§3).
        assert_eq!(
            app.transcript()[2],
            Block::Assistant(vec![
                ContentBlock::Text {
                    text: "earlier answer".into()
                },
                ContentBlock::ToolCall {
                    id: "t1".into(),
                    name: "read".into(),
                    arguments: Default::default(),
                },
            ])
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

        submit(&mut app, "/btw why the sky?");
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SideAsk {
                text: "why the sky?".into()
            })]
        );

        submit(&mut app, "/btw");
        assert!(app.take_actions().is_empty(), "a bare /btw nudges only");

        submit(&mut app, "/plan on");
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SetPlanMode { on: true })]
        );
        assert!(app.status().plan, "optimistic on");

        submit(&mut app, "/plan");
        assert_eq!(
            app.take_actions(),
            vec![Action::Ask(Request::SetPlanMode { on: false })]
        );
        assert!(!app.status().plan);

        submit(&mut app, "/nonsense");
        assert!(app.take_actions().is_empty());
    }

    #[test]
    fn a_plan_error_reverts_the_optimistic_chip() {
        let mut app = App::new();
        submit(&mut app, "/plan on");
        let _ = app.take_actions();
        assert!(app.status().plan, "optimistically on");
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::Error {
                message: "boom".into(),
            },
        ));
        assert!(!app.status().plan, "the chip reverts on error");
    }

    #[test]
    fn ack_settles_a_pending_plan_toggle_so_a_later_error_does_not_revert() {
        let mut app = App::new();
        submit(&mut app, "/plan on");
        let _ = app.take_actions();
        assert!(app.status().plan, "optimistically on");

        // The success reply (`Ack`) settles the toggle.
        app.handle(AppEvent::Agent(root(), AgentEvent::Ack));

        // An unrelated later error must NOT revert the settled chip.
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::Error {
                message: "boom".into(),
            },
        ));
        assert!(app.status().plan, "a settled toggle is not reverted");
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
                content: vec![
                    ContentBlock::Text { text: "x".into() },
                    ContentBlock::ToolCall {
                        id: "t1".into(),
                        name: "read".into(),
                        arguments: "{}".parse().unwrap(),
                    },
                ],
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
            Some(Block::Notice(text)) if text.contains("1 turn") && text.contains("tools: read×1")
        ));
    }

    #[test]
    fn a_history_reply_without_usage_reports_none() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(root(), AgentEvent::History {
            messages: vec![AgentMessage::user_text("q")],
        }));
        assert_eq!(app.context_used(), None);
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(text)) if text == "(no usage reported)"
        ));
    }

    #[test]
    fn a_side_answer_pushes_a_btw_block() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::SideAnswer {
                text: "42".into(),
                usage: None,
            },
        ));
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Btw(t)) if t == "42"
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
    fn surface_picker_reflects_a_model_change() {
        // Regression: `Surface` carried its own `model`, so `/model` (which
        // updates only the status line) left the `/surface` picker showing a
        // stale model. The picker must read the single live source of truth.
        let (mut app, _root, _member) = two_surfaces();
        app.set_models(vec!["zeta".into()]);
        submit(&mut app, "/model");
        let _ = app.take_actions();
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.status().model, "zeta", "the picker updates the status line");

        submit(&mut app, "/surface");
        let Some(Overlay::Pick(picker)) = app.overlay() else {
            panic!("the surface picker is open");
        };
        assert_eq!(
            picker.items[0], "root · zeta",
            "the picker must read the live status model"
        );
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
            duration_ms: None,
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
            duration_ms: None,
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
        assert_eq!(app.member_rows()[0].1, TeamState::Idle);
        app.handle(AppEvent::Agent(id.clone(), AgentEvent::AgentStart));
        assert_eq!(app.member_rows()[0].1, TeamState::Running);
        app.handle(AppEvent::Agent(id, AgentEvent::AgentEnd));
        assert_eq!(app.member_rows()[0].1, TeamState::Done);
        assert!(app.dirty(), "an event requests a redraw");
    }

    #[test]
    fn a_run_failure_marks_the_member_failed_until_the_next_run() {
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

        // The loop emits `Error` before `AgentEnd`; `Failed` must survive the end.
        app.handle(AppEvent::Agent(id.clone(), AgentEvent::AgentStart));
        assert_eq!(app.member_rows()[0].1, TeamState::Running);
        app.handle(AppEvent::Agent(
            id.clone(),
            AgentEvent::Error {
                message: "boom".into(),
            },
        ));
        assert_eq!(
            app.member_rows()[0].1,
            TeamState::Failed,
            "a run error fails the member"
        );
        app.handle(AppEvent::Agent(id.clone(), AgentEvent::AgentEnd));
        assert_eq!(
            app.member_rows()[0].1,
            TeamState::Failed,
            "AgentEnd must not clear the failure"
        );

        // The next run clears it.
        app.handle(AppEvent::Agent(id, AgentEvent::AgentStart));
        assert_eq!(
            app.member_rows()[0].1,
            TeamState::Running,
            "a fresh run clears the failure"
        );
    }

    #[test]
    fn an_idle_error_does_not_fail_a_member() {
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

        // No `AgentStart`: a command-reply error (SetModel/Compact/…) must not
        // mark the member failed.
        app.handle(AppEvent::Agent(
            id,
            AgentEvent::Error {
                message: "no such model".into(),
            },
        ));
        assert_eq!(app.member_rows()[0].1, TeamState::Idle);
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
        // One `{glyph} {label} · {state}` line each — no model (§2).
        assert!(text.contains("○ explorer · idle"), "{text}");
        assert!(text.contains("✓ reviewer · done"), "{text}");
        assert!(
            !text.contains("m1") && !text.contains("m2"),
            "model shown: {text}"
        );
    }

    /// `/tasks` reflects the injected plan: `(no tasks yet)`, then one line each.
    #[test]
    fn the_tasks_command_lists_the_plan() {
        let mut app = App::new();
        submit(&mut app, "/tasks");
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t == "(no tasks yet)"
        ));

        let mut app = App::new();
        app.set_tasks(vec![
            TaskItem {
                id: 1,
                title: "explore".into(),
                owner: None,
                state: "todo".into(),
            },
            TaskItem {
                id: 2,
                title: "fix it".into(),
                owner: Some("w1".into()),
                state: "doing".into(),
            },
        ]);
        submit(&mut app, "/tasks");
        let Some(Block::Notice(text)) = app.transcript().last() else {
            panic!("expected a tasks notice");
        };
        assert!(text.contains("#1 [todo] explore (unassigned)"), "{text}");
        assert!(text.contains("#2 [doing] fix it (w1)"), "{text}");
    }

    /// `tasks_text` renders `(no tasks yet)` when empty, one line per task else.
    #[test]
    fn tasks_text_renders_each_task() {
        assert_eq!(tasks_text(&[]), "(no tasks yet)");
        let items = vec![TaskItem {
            id: 3,
            title: "verify".into(),
            owner: Some("reviewer".into()),
            state: "done".into(),
        }];
        assert_eq!(tasks_text(&items), "#3 [done] verify (reviewer)");
    }

    /// `set_tasks` replaces the plan and requests a redraw.
    #[test]
    fn set_tasks_replaces_the_plan_and_marks_dirty() {
        let mut app = App::new();
        assert!(app.tasks.is_empty());
        app.dirty = false;
        app.set_tasks(vec![TaskItem {
            id: 7,
            title: "t".into(),
            owner: None,
            state: "todo".into(),
        }]);
        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.tasks[0].id, 7);
        assert!(app.dirty(), "a new plan requests a redraw");
    }

    #[test]
    fn a_tool_start_sets_the_member_action_and_agent_end_clears_it() {
        let (mut app, _root, id) = two_surfaces();
        app.handle(AppEvent::Agent(id.clone(), AgentEvent::AgentStart));

        // The committed assistant block carries the call's arguments (§3).
        app.handle(AppEvent::Agent(
            id.clone(),
            AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{\"path\":\"a.rs\"}".parse().unwrap(),
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: None,
                    model: None,
                },
            },
        ));
        app.handle(AppEvent::Agent(
            id.clone(),
            AgentEvent::ToolExecutionStart {
                call_id: "c1".into(),
                name: "read".into(),
            },
        ));
        assert_eq!(app.member_rows()[0].3, Some("read a.rs"));

        app.handle(AppEvent::Agent(id, AgentEvent::AgentEnd));
        assert_eq!(app.member_rows()[0].3, None, "AgentEnd clears the action");
    }

    #[test]
    fn the_status_glyphs_match_the_run_state() {
        assert_eq!(TeamState::Idle.glyph(), "○");
        assert_eq!(TeamState::Running.glyph(), "●");
        assert_eq!(TeamState::Done.glyph(), "✓");
        assert_eq!(TeamState::Failed.glyph(), "✗");
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
        assert_eq!(row[0].1, TeamState::Running, "the added surface runs");

        app.handle(AppEvent::Agent(id, AgentEvent::AgentEnd));
        assert_eq!(app.member_rows()[0].1, TeamState::Done);
    }

    /// Push one `ToolExecutionStart` into `id`, stamping the action (and its
    /// recency) the team strip orders by.
    fn member_tool_start(app: &mut App, id: &SessionId, name: &str) {
        app.handle(AppEvent::Agent(
            id.clone(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t".into(),
                name: name.into(),
            },
        ));
    }

    #[test]
    fn working_team_rows_keep_only_running_oldest_first_capped_at_three() {
        let mut app = App::new();
        set_surfaces(&mut app, &["root", "a", "b", "c", "d"]);
        let (a, b, c, d) = (
            SessionId::agent("a"),
            SessionId::agent("b"),
            SessionId::agent("c"),
            SessionId::agent("d"),
        );

        // Only running members appear; idle ones are dropped entirely.
        app.handle(AppEvent::Agent(a.clone(), AgentEvent::AgentStart));
        app.handle(AppEvent::Agent(b.clone(), AgentEvent::AgentStart));
        let labels = |app: &App| -> Vec<String> {
            app.working_team_rows()
                .iter()
                .map(|r| r.0.to_string())
                .collect()
        };
        assert_eq!(labels(&app), vec!["a", "b"]);

        // Recency stamps order oldest→newest (the LAST row is the most recent).
        member_tool_start(&mut app, &b, "read"); // b's action is newer than a's
        assert_eq!(labels(&app), vec!["a", "b"]);

        // Cap: with four running members, only the 3 MOST RECENT survive.
        app.handle(AppEvent::Agent(c.clone(), AgentEvent::AgentStart));
        app.handle(AppEvent::Agent(d.clone(), AgentEvent::AgentStart));
        member_tool_start(&mut app, &c, "edit");
        member_tool_start(&mut app, &d, "bash");
        assert_eq!(labels(&app).len(), 3);
        assert_eq!(*labels(&app).last().unwrap(), "d", "newest is the bottom row");

        // The live action rides the row (as member_rows does).
        assert_eq!(app.working_team_rows().last().unwrap().2, Some("bash"));
    }
    #[test]
    fn member_rows_order_active_first_then_by_action_recency() {
        let mut app = App::new();
        set_surfaces(&mut app, &["root", "zebra", "alpha", "mike"]);
        let zebra = SessionId::agent("zebra");
        let alpha = SessionId::agent("alpha");
        let mike = SessionId::agent("mike");
        let labels = |app: &App| -> Vec<String> {
            app.member_rows()
                .iter()
                .map(|row| row.0.to_string())
                .collect()
        };

        // All idle with no actions: surface order (stable sort) is preserved.
        assert_eq!(labels(&app), vec!["zebra", "alpha", "mike"]);
        // alpha's action is the more recent: idle members sort by recency.
        member_tool_start(&mut app, &zebra, "read");
        member_tool_start(&mut app, &alpha, "bash");
        assert_eq!(labels(&app), vec!["alpha", "zebra", "mike"]);
        // A running member jumps the queue even before any live action.
        app.handle(AppEvent::Agent(mike.clone(), AgentEvent::AgentStart));
        assert_eq!(labels(&app), vec!["mike", "alpha", "zebra"]);
        // Among running members, the newest action comes first.
        app.handle(AppEvent::Agent(zebra.clone(), AgentEvent::AgentStart));
        member_tool_start(&mut app, &zebra, "edit");
        let rows = app.member_rows();
        assert_eq!(rows[0].0, "zebra", "newer action wins among running members");
        assert_eq!(rows[0].3, Some("edit"), "the live action rides the row");
        assert_eq!(rows[1].0, "mike", "a runner with no action trails");
        assert_eq!(rows[1].3, None);
        assert_eq!(rows[2].0, "alpha", "idle members come last, by recency");
        assert_eq!(rows[2].3, Some("bash"));
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

    #[test]
    fn reload_requests_a_reexec_and_quits() {
        let mut app = App::new();
        submit(&mut app, "/reload");
        assert_eq!(app.pending_reload(), Some(false));
        assert!(app.should_quit(), "a reload quits the TUI to re-exec");
        assert_eq!(app.pending_resume(), None);
    }

    #[test]
    fn reload_no_session_flag_is_carried() {
        let mut app = App::new();
        submit(&mut app, "/reload --no-session");
        assert_eq!(app.pending_reload(), Some(true));
        assert!(app.should_quit());
    }

    #[test]
    fn reload_rejects_a_bad_argument_without_quitting() {
        let mut app = App::new();
        submit(&mut app, "/reload wat");
        assert_eq!(app.pending_reload(), None);
        assert!(!app.should_quit());
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("usage: /reload")
        ));
    }

    #[test]
    fn reload_is_refused_over_a_socket() {
        let mut app = App::new();
        app.set_remote(true);
        submit(&mut app, "/reload");
        assert_eq!(app.pending_reload(), None, "a socket client cannot re-exec");
        assert!(!app.should_quit());
        assert!(matches!(
            app.transcript().last(),
            Some(Block::Notice(t)) if t.contains("unavailable over a socket")
        ));
    }

    #[test]
    fn the_command_table_lists_reload() {
        assert!(COMMANDS.iter().any(|c| c.name == "reload"));
        assert!(help_text().contains("/reload [--no-session]"));
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
                 /resume /reload [--no-session] /btw <question> /plan [on|off] /verify \
                 /usage /copy /surface /team /tasks /help"
            ),
            "the command listing changed: {text}"
        );
        for name in [
            "exit", "model", "effort", "compact", "changes", "resume", "reload", "btw", "plan",
            "verify", "usage", "copy", "surface", "team", "tasks", "help",
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
                duration_ms: None,
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

    // --- transcript browse mode ---------------------------------------------

    #[test]
    fn ctrl_g_enters_browse_on_the_last_block_and_clamps_movement() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("a"), assistant("b")]);
        // transcript: [Notice(divider), User, Assistant]
        assert_eq!(app.mode(), Mode::Input);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.mode(), Mode::Browse);
        assert_eq!(app.selected(), Some(2), "the last committed block");

        app.handle(AppEvent::Key(Key::Char('j'))); // clamp at the end
        assert_eq!(app.selected(), Some(2));
        app.handle(AppEvent::Key(Key::Char('k')));
        assert_eq!(app.selected(), Some(1));
        app.handle(AppEvent::Key(Key::Char('g')));
        assert_eq!(app.selected(), Some(0));
        app.handle(AppEvent::Key(Key::Char('k'))); // clamp at the start
        assert_eq!(app.selected(), Some(0));
        app.handle(AppEvent::Key(Key::Char('G')));
        assert_eq!(app.selected(), Some(2));
    }

    #[test]
    fn esc_in_browse_exits_without_cancelling_or_quitting() {
        let mut app = App::new();
        submit(&mut app, "go"); // a run is in flight
        let _ = app.take_actions();
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.mode(), Mode::Browse);

        app.handle(AppEvent::Key(Key::Esc));
        assert_eq!(app.mode(), Mode::Input, "Esc leaves browse");
        assert!(!app.should_quit(), "Esc in browse must not quit");
        assert!(app.take_actions().is_empty(), "Esc in browse must not cancel");
        assert!(app.running(), "the run is untouched");

        // In input mode Esc still cancels a run.
        app.handle(AppEvent::Key(Key::Esc));
        assert_eq!(app.take_actions(), vec![Action::Cancel]);
    }

    #[test]
    fn q_and_ctrl_g_also_leave_browse() {
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Char('q')));
        assert_eq!(app.mode(), Mode::Input);

        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.mode(), Mode::Input);
    }

    #[test]
    fn browsing_preserves_the_composer_draft_and_cursor() {
        let mut app = App::new();
        typed(&mut app, "draft");
        app.handle(AppEvent::Key(Key::Left));
        let (draft, cursor) = (app.input(), app.cursor());
        assert_eq!(cursor, 4);

        app.handle(AppEvent::Key(Key::Ctrl('g')));
        // Keys in browse never reach the composer.
        app.handle(AppEvent::Key(Key::Char('j')));
        app.handle(AppEvent::Key(Key::Char('x')));
        assert_eq!(app.input(), draft, "the draft is untouched while browsing");
        app.handle(AppEvent::Key(Key::Esc));
        assert_eq!(app.input(), draft);
        assert_eq!(app.cursor(), cursor);
    }

    #[test]
    fn a_front_inserted_seed_divider_does_not_strand_the_selection() {
        let mut app = App::new();
        // Browse an empty transcript: nothing to select yet.
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), None);
        // Seed replays a session; on an empty transcript the divider lands first.
        app.seed_history(&root(), &[AgentMessage::user_text("q"), assistant("a")]);
        assert!(matches!(app.transcript()[0], Block::Notice(_)), "divider first");
        assert_eq!(app.transcript().len(), 3);
        // The selection is still absent — never a stale index — and moving picks a
        // real block.
        assert_eq!(app.selected(), None);
        app.handle(AppEvent::Key(Key::Char('j')));
        assert_eq!(app.selected(), Some(0));
    }

    #[test]
    fn seeding_an_existing_transcript_keeps_the_selection_on_the_same_block() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::MessageEnd {
                message: assistant("live one"),
            },
        ));
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(0));
        // A seed on a non-empty transcript drops its divider at the end, so index
        // 0 still names the same block.
        app.seed_history(&root(), &[AgentMessage::user_text("old question")]);
        assert!(matches!(app.transcript()[0], Block::Assistant(_)));
        assert_eq!(app.selected(), Some(0));
    }

    #[test]
    fn a_stale_selection_clamps_when_the_transcript_shrinks_out_of_reach() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("a"), assistant("b")]);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(2));
        // The renderer reports a shorter block list: the selection clamps, never
        // pointing past the end.
        app.set_block_ranges(std::iter::once(0..1).collect());
        assert_eq!(app.selected(), Some(0));
    }

    #[test]
    fn f1_opens_help_from_browse_and_returns_to_browse() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("q")]);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.mode(), Mode::Browse);

        app.handle(AppEvent::Key(Key::F(1)));
        assert!(
            matches!(app.overlay(), Some(Overlay::Help)),
            "F1 opens help from browse"
        );
        assert_eq!(app.mode(), Mode::Browse, "browse survives under the overlay");
        app.handle(AppEvent::Key(Key::Esc));
        assert!(app.overlay().is_none(), "Esc closes help");
        assert_eq!(app.mode(), Mode::Browse, "closing help returns to browse");
        assert_eq!(app.selected(), Some(1), "the selection is preserved");

        // `?` is the same overlay.
        app.handle(AppEvent::Key(Key::Char('?')));
        assert!(matches!(app.overlay(), Some(Overlay::Help)));
        app.handle(AppEvent::Key(Key::F(1)));
        assert_eq!(app.mode(), Mode::Browse);
    }

    #[test]
    fn ctrl_c_cancels_a_run_while_browsing_and_stays_in_browse() {
        let mut app = App::new();
        submit(&mut app, "go"); // a run is in flight
        let _ = app.take_actions();
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.mode(), Mode::Browse);

        app.handle(AppEvent::Key(Key::Ctrl('c')));
        assert_eq!(app.mode(), Mode::Browse, "Ctrl-C does not leave browse");
        assert_eq!(app.take_actions(), vec![Action::Cancel]);
        assert!(!app.should_quit());
    }

    #[test]
    fn ctrl_c_quits_when_idle_and_browsing() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("q")]);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert!(!app.should_quit());

        app.handle(AppEvent::Key(Key::Ctrl('c')));
        assert!(app.should_quit(), "Ctrl-C quits when idle, in browse too");
        assert_eq!(app.mode(), Mode::Browse);
    }

    #[test]
    fn browsing_an_empty_transcript_selects_nothing() {
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.mode(), Mode::Browse);
        assert_eq!(app.selected(), None, "nothing to select");
        app.handle(AppEvent::Key(Key::Char('j')));
        assert_eq!(app.selected(), None, "j stays None with no blocks");
        app.handle(AppEvent::Key(Key::Char('k')));
        assert_eq!(app.selected(), None);
        app.handle(AppEvent::Key(Key::Char('G')));
        assert_eq!(app.selected(), None, "G stays None with no blocks");
    }

    #[test]
    fn enter_toggles_the_selected_tool_in_browse() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_done_tool(&mut app, &output); // collapsed by default
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // browse selects the tool
        assert_eq!(app.selected(), Some(0));
        assert!(!tool_expanded(&app, 0));

        app.handle(AppEvent::Key(Key::Enter));
        assert!(tool_expanded(&app, 0), "Enter expands the selected tool");
        app.handle(AppEvent::Key(Key::Enter));
        assert!(!tool_expanded(&app, 0), "Enter collapses it again");
        // Space is the same.
        app.handle(AppEvent::Key(Key::Char(' ')));
        assert!(tool_expanded(&app, 0), "Space matches Enter");
    }

    #[test]
    fn enter_on_a_non_tool_block_is_a_silent_no_op() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello")]);
        // transcript: [Notice(divider), User]
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(1), "the User block");

        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.transcript().len(), 2, "no block was added");
        assert!(app.take_actions().is_empty(), "Enter pushes no action");

        // The divider (a Notice) too.
        app.handle(AppEvent::Key(Key::Char('g')));
        assert_eq!(app.selected(), Some(0));
        app.handle(AppEvent::Key(Key::Char(' ')));
        assert!(app.take_actions().is_empty());
        assert_eq!(app.transcript().len(), 2);
    }

    #[test]
    fn y_on_a_collapsed_tool_copies_the_whole_output() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_done_tool(&mut app, &output);
        assert!(!tool_expanded(&app, 0), "the tool is collapsed");
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Char('y')));
        // The whole output, never the collapsed preview — copying the preview
        // would be a silent data loss.
        assert_eq!(app.take_actions(), vec![Action::Copy(output.clone())]);
    }

    #[test]
    fn y_copies_each_block_kind_by_its_text() {
        // User
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello world")]);
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // the User block
        app.handle(AppEvent::Key(Key::Char('y')));
        assert_eq!(app.take_actions(), vec![Action::Copy("hello world".into())]);

        // Assistant: text only, thinking excluded.
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::Assistant {
            content: vec![
                ContentBlock::Thinking {
                    text: "secret reasoning".into(),
                },
                ContentBlock::Text {
                    text: "the visible answer".into(),
                },
            ],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        }]);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Char('y')));
        assert_eq!(app.take_actions(), vec![Action::Copy("the visible answer".into())]);

        // Notice
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::Compaction {
                summarized: 3,
                kept: 1,
            },
        ));
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Char('y')));
        assert_eq!(
            app.take_actions(),
            vec![Action::Copy("⋯ compacted 3 messages, kept 1".into())]
        );

        // CompactionSkipped rides the same Notice seam as Compaction (a
        // non-fatal, best-effort miss). The "does not mark failed" contract is
        // covered by `compaction_skipped_notices_and_does_not_mark_failed`.
        // Error
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::Error {
                message: "boom".into(),
            },
        ));
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Char('y')));
        assert_eq!(app.take_actions(), vec![Action::Copy("boom".into())]);
    }

    #[test]
    fn compaction_skipped_notices_and_does_not_mark_failed() {
        // `failed` lives on the private `Surface`, so observe it through the same
        // member-state seam `a_run_failure_marks_the_member_failed_until_the_next_run`
        // uses.
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
        app.handle(AppEvent::Agent(id.clone(), AgentEvent::AgentStart));
        assert_eq!(app.member_rows()[0].1, TeamState::Running);

        app.handle(AppEvent::Agent(
            id,
            AgentEvent::CompactionSkipped {
                reason: "summarizer 500".into(),
            },
        ));

        // Visible as a Notice: focus the member and copy its last block.
        app.set_focus(1);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Char('y')));
        assert_eq!(
            app.take_actions(),
            vec![Action::Copy("⋯ compaction skipped: summarizer 500".into())]
        );

        // …and CRUCIALLY the run is NOT failed: contrast the `Error` arm, which
        // sets `failed` while running (app.rs:1139).
        assert_eq!(
            app.member_rows()[0].1,
            TeamState::Running,
            "a best-effort compaction miss must not fail the run"
        );
    }

    #[test]
    fn y_on_a_diff_block_copies_the_diff_body() {
        let mut app = App::new();
        // A tool that changed a file records the (path, diff) changeset.
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::ToolExecutionStart {
                call_id: "t".into(),
                name: "edit".into(),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            AgentEvent::ToolExecutionEnd {
                call_id: "t".into(),
                name: "edit".into(),
                output: "ok".into(),
                is_error: false,
                diff: Some("@@ -1 +1 @@\n-old\n+new".into()),
                path: Some("f.rs".into()),
                duration_ms: None,
            },
        ));
        // `/changes` → select → re-shows the diff as a `Block::Diff`.
        typed(&mut app, "/changes");
        app.handle(AppEvent::Key(Key::Enter)); // dispatch the command
        app.handle(AppEvent::Key(Key::Enter)); // select the only row
        let _ = app.take_actions();

        app.handle(AppEvent::Key(Key::Ctrl('g'))); // selects the last block (the diff)
        app.handle(AppEvent::Key(Key::Char('y')));
        assert_eq!(
            app.take_actions(),
            vec![Action::Copy("@@ -1 +1 @@\n-old\n+new".into())]
        );
    }

    #[test]
    fn y_with_nothing_to_copy_pushes_no_action() {
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // empty transcript: no selection
        app.handle(AppEvent::Key(Key::Char('y')));
        assert!(app.take_actions().is_empty(), "no action for an empty selection");
        assert!(
            matches!(app.transcript().last(), Some(Block::Notice(t)) if t == "nothing to copy")
        );
    }

    #[test]
    fn home_and_end_anchor_to_the_visible_viewport() {
        let mut app = App::new();
        // Five blocks; block 1 renders no lines (a tool-call-only slot). The
        // total is 14 lines and the viewport 8; each press is tested against a
        // fresh scroll position (reveal re-anchors, so we reset between keys).
        for text in ["a", "b", "c", "d", "e"] {
            app.focused_mut().push_block(Block::User(text.into()));
        }
        {
            let s = app.focused_mut();
            s.ranges = vec![0..2, 2..2, 2..6, 6..10, 10..14];
            s.viewport = 8;
            s.last_total = 14;
            s.max_scroll = 6;
            s.scroll = 0;
        }

        app.handle(AppEvent::Key(Key::Ctrl('g'))); // browse on the last block
        assert_eq!(app.selected(), Some(4));

        // Tail view [6,14): blocks 3 and 4.
        app.handle(AppEvent::Key(Key::Home));
        assert_eq!(app.selected(), Some(3), "topmost in [6,14)");
        app.focused_mut().scroll = 0;
        app.handle(AppEvent::Key(Key::End));
        assert_eq!(app.selected(), Some(4), "bottommost in [6,14)");

        // Three lines up: view [3,11) shows blocks 2–4 (block 1 is 2..2, a
        // zero-line slot that must never be a target).
        app.focused_mut().scroll = 3;
        app.handle(AppEvent::Key(Key::Home));
        assert_eq!(app.selected(), Some(2), "topmost in [3,11)");
        app.focused_mut().scroll = 3;
        app.handle(AppEvent::Key(Key::End));
        assert_eq!(app.selected(), Some(4), "bottommost in [3,11)");

        // Six lines up: view [0,8) shows blocks 0, 2 and 3.
        app.focused_mut().scroll = 6;
        app.handle(AppEvent::Key(Key::Home));
        assert_eq!(app.selected(), Some(0), "topmost in [0,8)");
        app.focused_mut().scroll = 6;
        app.handle(AppEvent::Key(Key::End));
        assert_eq!(app.selected(), Some(3), "bottommost in [0,8), skipping 2..2");
    }

    #[test]
    fn home_and_end_fall_back_to_the_transcript_ends() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("a"), assistant("b")]);
        // transcript: [Notice, User, Assistant] — no frame has been measured.
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(2));
        app.handle(AppEvent::Key(Key::Home));
        assert_eq!(app.selected(), Some(0), "fallback: the first committed block");
        app.handle(AppEvent::Key(Key::End));
        assert_eq!(app.selected(), Some(2), "fallback: the last committed block");

        // With an empty transcript neither key does anything.
        let mut empty = App::new();
        empty.handle(AppEvent::Key(Key::Ctrl('g')));
        empty.handle(AppEvent::Key(Key::Home));
        empty.handle(AppEvent::Key(Key::End));
        assert_eq!(empty.selected(), None, "no blocks, no selection");
    }

    #[test]
    fn brace_keys_step_blocks_and_clamp_at_the_ends() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("a"), assistant("b")]);
        // transcript: [Notice, User, Assistant]
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(2));

        app.handle(AppEvent::Key(Key::Char('}')));
        assert_eq!(app.selected(), Some(2), "}} clamps at the last block");
        app.handle(AppEvent::Key(Key::Char('{')));
        assert_eq!(app.selected(), Some(1));
        app.handle(AppEvent::Key(Key::Char('{')));
        assert_eq!(app.selected(), Some(0));
        app.handle(AppEvent::Key(Key::Char('{')));
        assert_eq!(app.selected(), Some(0), "{{ clamps at the first block");
        app.handle(AppEvent::Key(Key::Char('}')));
        assert_eq!(app.selected(), Some(1), "}} steps forward again");
    }

    #[test]
    fn alt_braces_step_blocks_and_clamp_at_the_ends() {
        // macOS Option-8/9 and German AltGr deliver braces as Alt chords —
        // browse accepts them exactly like the plain `Char` spelling.
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("a"), assistant("b")]);
        // transcript: [Notice, User, Assistant]
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(2));

        app.handle(AppEvent::Key(Key::Alt('}')));
        assert_eq!(app.selected(), Some(2), "Alt-}} clamps at the last block");
        app.handle(AppEvent::Key(Key::Alt('{')));
        assert_eq!(app.selected(), Some(1));
        app.handle(AppEvent::Key(Key::Alt('{')));
        assert_eq!(app.selected(), Some(0));
        app.handle(AppEvent::Key(Key::Alt('{')));
        assert_eq!(app.selected(), Some(0), "Alt-{{ clamps at the first block");
        app.handle(AppEvent::Key(Key::Alt('}')));
        assert_eq!(app.selected(), Some(1), "Alt-}} steps forward again");
    }

    #[test]
    fn non_brace_alt_chords_are_still_ignored_in_browse() {
        // Alt-1..9 focuses surfaces in input mode; in browse (where browse
        // owns the keys) an Alt chord that is not a brace does nothing.
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("a"), assistant("b")]);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(2));

        app.handle(AppEvent::Key(Key::Alt('x')));
        assert_eq!(app.selected(), Some(2), "Alt-x is a no-op");
        app.handle(AppEvent::Key(Key::Alt('y')));
        assert_eq!(app.selected(), Some(2), "Alt-y must not copy");
        assert!(app.take_actions().is_empty(), "no action for Alt-y");
        app.handle(AppEvent::Key(Key::Alt('1')));
        assert_eq!(app.selected(), Some(2), "Alt-1 must not focus a surface");
        app.handle(AppEvent::Key(Key::Alt('j')));
        assert_eq!(app.selected(), Some(2), "Alt-j must not move the selection");
    }

    #[test]
    fn append_block_lines_extends_the_parallel_vecs_on_a_parity_miss() {
        let mut app = App::new();
        // Deliberately bypass `push_block` so `transcript` runs ahead of the
        // parallel vecs — the guard must re-render, not panic.
        app.focused_mut().transcript.push(Block::Notice("x".into()));
        let mut out = Vec::new();
        let range = app.focused_mut().append_block_lines(0, 40, &mut out);
        assert_eq!(range, 0..1, "the block renders one line");
        assert_eq!(out.len(), 1);
    }

    /// A transcript with four blocks; "alpha" occurs in blocks 0 and 2.
    fn searchable_transcript(app: &mut App) {
        let surface = app.focused_mut();
        surface.transcript = vec![
            Block::User("alpha".into()),
            Block::User("beta".into()),
            Block::User("ALPHA too".into()),
            Block::User("gamma".into()),
        ];
        // Replaced wholesale — keep the cache/revs parallel vecs aligned.
        surface.cache.clear();
        surface.block_revs.clear();
    }

    #[test]
    fn slash_opens_the_search_prompt_and_typing_filters_live() {
        let mut app = App::new();
        searchable_transcript(&mut app);
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // selection: the last block
        assert_eq!(app.selected(), Some(3));

        app.handle(AppEvent::Key(Key::Char('/')));
        assert_eq!(app.search_query(), Some(""), "an empty query opens");
        assert_eq!(app.search_hits(), 0, "an empty term matches nothing");

        typed(&mut app, "ALPHA");
        assert_eq!(app.search_query(), Some("ALPHA"));
        assert_eq!(app.search_hits(), 2, "case-insensitive live filter");

        app.handle(AppEvent::Key(Key::Backspace));
        app.handle(AppEvent::Key(Key::Backspace));
        app.handle(AppEvent::Key(Key::Backspace));
        app.handle(AppEvent::Key(Key::Backspace));
        app.handle(AppEvent::Key(Key::Backspace));
        assert_eq!(app.search_query(), Some(""), "backspace empties the query");
    }

    #[test]
    fn enter_in_search_jumps_at_or_after_the_selection_wrapping_and_closes() {
        let mut app = App::new();
        searchable_transcript(&mut app);
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // selection: block 3
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "beta");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.search_query(), None, "Enter closes the prompt");
        assert_eq!(app.selected(), Some(1), "the only match at/after 3 wraps to 1");

        // From block 1, "alpha" (matching blocks 0 and 2) jumps to 2: the
        // first match at/after the selection.
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "alpha");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.selected(), Some(2), "first match at/after block 1");

        // From a matching selection the same search is a no-op (at/after).
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "alpha");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.selected(), Some(2), "stays on the matching block");
    }

    #[test]
    fn n_and_n_repeat_the_search_strictly_and_wrap() {
        let mut app = App::new();
        searchable_transcript(&mut app);
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // selection: block 3
        // Jump once to block 0 (the only match at/after 3, wrapping).
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "alpha");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.selected(), Some(0));

        app.handle(AppEvent::Key(Key::Char('n')));
        assert_eq!(app.selected(), Some(2), "strictly next from block 0");
        app.handle(AppEvent::Key(Key::Char('n')));
        assert_eq!(app.selected(), Some(0), "n wraps past the end");
        app.handle(AppEvent::Key(Key::Char('N')));
        assert_eq!(app.selected(), Some(2), "N wraps before the start");
        app.handle(AppEvent::Key(Key::Char('N')));
        assert_eq!(app.selected(), Some(0), "strictly previous from block 2");
    }

    #[test]
    fn empty_and_matchless_searches_are_no_ops() {
        let mut app = App::new();
        searchable_transcript(&mut app);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(3));

        // No remembered search: n / N do nothing.
        app.handle(AppEvent::Key(Key::Char('n')));
        app.handle(AppEvent::Key(Key::Char('N')));
        assert_eq!(app.selected(), Some(3), "nothing to repeat");

        // Enter with an empty term keeps the prompt open and moves nothing.
        app.handle(AppEvent::Key(Key::Char('/')));
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.search_query(), Some(""), "empty Enter is a no-op");
        assert_eq!(app.selected(), Some(3));

        // A matchless term is a no-op too, and Esc closes without jumping.
        typed(&mut app, "zzz");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.search_query(), Some("zzz"), "no match: prompt stays up");
        assert_eq!(app.selected(), Some(3), "no match: nothing moves");
        app.handle(AppEvent::Key(Key::Esc));
        assert_eq!(app.search_query(), None, "Esc closes without jumping");
        assert_eq!(app.selected(), Some(3));
        // The fruitless term was not remembered.
        app.handle(AppEvent::Key(Key::Char('n')));
        assert_eq!(app.selected(), Some(3), "a skip still has nothing to repeat");
    }

    #[test]
    fn search_edits_do_not_reach_the_composer_draft() {
        let mut app = App::new();
        searchable_transcript(&mut app);
        typed(&mut app, "draft text");
        assert_eq!(app.input(), "draft text");
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        // Typing into the search prompt must not touch the composer.
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "alpha");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.input(), "draft text", "the draft is untouched");
        assert_eq!(
            app.cursor(),
            10,
            "the composer cursor is untouched (10 chars typed)"
        );
        app.handle(AppEvent::Key(Key::Esc)); // leave browse
        assert_eq!(app.input(), "draft text", "still intact after leaving browse");
    }

    #[test]
    fn search_prompt_keeps_help_and_ctrl_c_global() {
        let mut app = App::new();
        searchable_transcript(&mut app);
        submit(&mut app, "go"); // a run in flight
        let _ = app.take_actions();
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "alpha");

        app.handle(AppEvent::Key(Key::F(1)));
        assert!(
            matches!(app.overlay(), Some(Overlay::Help)),
            "F1 opens help from the search prompt"
        );
        assert_eq!(app.search_query(), Some("alpha"), "the prompt survives help");
        app.handle(AppEvent::Key(Key::Esc)); // close help
        app.handle(AppEvent::Key(Key::Char('?')));
        assert!(matches!(app.overlay(), Some(Overlay::Help)), "? opens help too");
        app.handle(AppEvent::Key(Key::Esc));

        app.handle(AppEvent::Key(Key::Ctrl('c')));
        assert_eq!(app.take_actions(), vec![Action::Cancel], "Ctrl-C cancels");
        assert_eq!(app.search_query(), Some("alpha"), "still searching after cancel");
        app.handle(AppEvent::Key(Key::Ctrl('c')));
        assert!(
            app.take_actions().is_empty(),
            "a running turn is only cancelled once"
        );
        assert!(!app.should_quit(), "Ctrl-C never quits a running turn");
    }

    #[test]
    fn paste_with_the_search_prompt_up_appends_to_the_query() {
        let mut app = App::new();
        searchable_transcript(&mut app);
        typed(&mut app, "draft");
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // browse on the last block
        assert_eq!(app.selected(), Some(3));
        app.handle(AppEvent::Key(Key::Char('/')));

        // A single-line paste goes into the query, not the composer.
        app.handle(AppEvent::Paste("needle".into()));
        assert_eq!(app.search_query(), Some("needle"), "paste edits the query");
        assert_eq!(app.input(), "draft", "the composer draft is untouched");
        assert_eq!(app.cursor(), 5, "the composer cursor is untouched");
        assert_eq!(app.selected(), Some(3), "the selection is untouched");

        // A multi-line paste appends verbatim.
        app.handle(AppEvent::Paste("alpha\nbeta".into()));
        assert_eq!(
            app.search_query(),
            Some("needlealpha\nbeta"),
            "multiline paste appends verbatim"
        );
        assert_eq!(app.input(), "draft");
        assert_eq!(app.cursor(), 5);

        // A pasted term is a live filter: reopen with one and jump.
        app.handle(AppEvent::Key(Key::Esc)); // close without jumping
        assert_eq!(app.selected(), Some(3));
        app.handle(AppEvent::Key(Key::Char('/')));
        app.handle(AppEvent::Paste("alpha".into()));
        assert_eq!(app.search_hits(), 2, "the pasted term filters live");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.selected(), Some(0), "first match at/after 3 wraps to 0");
        assert_eq!(app.input(), "draft", "still untouched after the jump");
    }
}
