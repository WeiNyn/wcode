//! The settings panel's **model** — a closed row taxonomy, a pure row builder,
//! and the key grammar as a pure function. `mod settings;` is PRIVATE (W032
//! first-layer A14): the CLI never touches the panel and `App` is the only
//! caller, so nothing here is crate-public API.
//!
//! Locked design: `docs/work/W032-tui-settings.md` §4.1 (host), §4.2 (grammar),
//! §4.3 (taxonomy), §4.4 (effort vocabulary), §4.5 (pending), §4.6 (the
//! read-only `config` section), §7.1 (the test names). Drawn chrome —
//! cursor glyph, affordance column, hint line, narrow-width drops — is
//! `docs/tui-design.md` §4 "settings panel" and is NOT specified here.
//!
//! # Real so far (steps 1–2) versus still sketch
//!
//! Real: the row ids, the taxonomy ([`SettingKind`] / [`Setting`] /
//! [`Section`]), [`rows`] with every `session` row and the `config` section's
//! line-per-dump split, [`SettingsState`], [`SettingsSnapshot`], [`line_of`],
//! [`KeyOutcome`], [`on_key`]'s movement + `Tab`/`BackTab` + `←`/`→`/`Enter`
//! grammar, and [`SettingAction`]'s effects — the four client-local ones plus the
//! two `Pick` rows' existing pickers.
//!
//! Still a sketch, kept verbatim and COMMENTED at the foot of this file: the
//! effort cycle (step 4: `SetEffort`, `EFFORT_LEVELS`' wrap and the off-list
//! rule, `effort_position` / `effort_at` / `effort_next`), and the tests it
//! needs.
//! They cannot be *declared* before they are constructed: `dead_code` fires on
//! an unconstructed enum variant inside a private module, and this crate may not
//! answer that with `#[allow]` (W032 §6). The step that constructs them brings
//! them back in one edit, because the sketch text is kept verbatim.
//!
//! # Who calls this
//!
//! Only `crate::app` (the reducer) and `crate::ui` (the renderer):
//!
//! - `App::settings_sections()` builds a [`SettingsSnapshot`] from its own
//!   fields and calls [`rows`]; both the key handler and the renderer consume
//!   the result. `rows` is pure, so the two can never disagree.
//! - `App::on_settings_key` calls [`on_key`] and turns the returned
//!   [`KeyOutcome`] into App state — `Close` clears the panel, `Act` runs in
//!   `App::apply_setting`. This module touches no `App`, no `Request`, no
//!   terminal: it is data plus pure functions.
//!
//! # Deviations from §4.3's literal sketch, each one line
//!
//! 1. `Setting` gains `value: String` and loses `value` from `Cycle`/`Nudge`/
//!    `Pick` — one place formats the right column (`"68 cols"`, `"128000"`),
//!    and a read-only row needs somewhere for its text to live.
//! 2. `Setting.label` is `Cow<'static, str>`, not `&'static str` — the injected
//!    `config` lines are owned strings; a `&'static str` cannot hold them.
//! 3. `Toggle`/`Cycle` carry a `what` discriminant (`Toggle::Plan` | `Sidebar`,
//!    `Cycle::Effort` | `View`), mirroring `Pick { kind: PickerKind }` — two rows
//!    share a kind but not an effect, and the brief forbids matching on ids.
//! 4. [`SettingsSnapshot`] carries `pending: &[&'static str]` and NO `running`.
//!    The pending mark must OUTLIVE the panel AND the run (§4.5 clears it on the
//!    next `AgentStart`, not on `AgentEnd` or `Esc`), so it is App data — and
//!    `App.settings` is an `Option` that drops its contents on close. It rides
//!    in the snapshot so [`rows`] can stamp it without a second argument.
//! 5. [`SettingsState`] carries `hint` and `page` beside `cursor` — §4.2's
//!    `Enter`-on-`ReadOnly` hint line needs a cell, and `PgUp`/`PgDn` need a
//!    page height the renderer has already computed (`ui::draw` publishes it,
//!    exactly as `app.set_terminal_width(full.width)` does at `ui.rs:81`).
//! 6. [`rows`] takes the snapshot ONLY: once `pending` is snapshot data, the
//!    row build reads nothing from the panel's own state, and an unused
//!    parameter is a lie about the contract.

use std::borrow::Cow;

use crate::app::{Key, MEASURE_LIMIT, MEASURE_MIN, MEASURE_STEP, PickerKind, ViewMode};

/// The stable row ids. ONE source: [`Setting::id`], the pending set, and the
/// tests' drift guard all read these, so a rename is one edit.
///
/// # Contracts
/// - A `session` row's id is unique and constant across builds.
/// - Every row of the read-only `config` section shares [`CONFIG`]: ids exist to
///   be *addressed* (pending, lookup), and a config row is never addressed — no
///   per-line id can be `&'static str` anyway (the text is injected at runtime).
pub mod ids {
    pub const MODEL: &str = "model";
    pub const EFFORT: &str = "effort";
    pub const PLAN: &str = "plan";
    pub const THEME: &str = "theme";
    pub const WIDTH: &str = "width";
    pub const VIEW: &str = "view";
    pub const SIDEBAR: &str = "sidebar";
    pub const TOOL_OUTPUT: &str = "tool output";
    pub const CONTEXT: &str = "context";
    pub const CONFIG: &str = "config";
}

/// The `config` section's heading, local and over `--socket` (§4.6: a socket
/// client shows ITS OWN config, never the server's; §4.8: the panel never claims
/// to know the server's config).
pub const CONFIG_HEADING_LOCAL: &str = "config";
pub const CONFIG_HEADING_CLIENT: &str = "config (this client)";

/// The reasoning-effort vocabulary the `effort` row cycles.
///
/// TWIN: `editors/vscode/src/tab.ts`'s `EFFORT_LEVELS` — the same seven, and
/// each file's doc comment names the other. Change both or neither.
///
/// CAVEAT, stated twice on purpose (once here, once in `tab.ts`): the KERNEL
/// does not validate effort — `Request::SetEffort { effort: Option<String> }`
/// is free text (`protocol.rs:262` `SetEffort { effort: Option<String> }`) and
/// the CHAT endpoint forwards any string verbatim (`streamfn.rs:100`ff: "Chat
/// forwards any string verbatim under `reasoning_effort`"). Only the Responses
/// endpoint checks these seven client-side. So this const is the PANEL's
/// vocabulary, not a constraint on the wire, and an off-list value is preserved
/// verbatim (§4.4) — never silently rewritten.
///
/// `EFFORT_LEVELS[0]` (`"none"`) is the free-style spelling of the cycle's
/// `(none)` stop, which is `None` on the wire (`/effort`'s
/// `"-" | "none" | "off" => None`, `app.rs:4255`). It is therefore NOT a
/// separate stop: the wrap is `(none) → minimal → … → max → (none)` (§7.1).
pub const EFFORT_LEVELS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// The `view` row's cycle: `ViewMode`'s two words, in `ViewMode::parse`'s
/// spelling, so the drawn token and the parsed action are the same string.
const VIEW_MODES: &[&str] = &["focus", "all"];

/// Cursor rows moved per mouse-wheel notch. Reuses `crate::app::WHEEL_LINES`
/// (`app.rs` `pub(crate) const WHEEL_LINES: usize = 3;` — the transcript's wheel
/// constant), widened by first-layer A13 precisely so this is a reuse and not a
/// second copy of `3`: one number, so one notch means the same thing in the
/// transcript and in the panel.
const WHEEL_ROWS: usize = crate::app::WHEEL_LINES;

/// The page height before the renderer publishes one. `0`, not a magic number
/// (first-layer A6): `PgUp`/`PgDn` are a no-op until a frame has laid the panel
/// out — the `terminal_height == 0` precedent (`app.rs:2600`-`:2605` "`0` before
/// the first frame … nothing visible, nothing to scroll"). A `DEFAULT_PAGE = 10`
/// would be a second, untested clamp.
const DEFAULT_PAGE: usize = 0;

/// Everything the panel refuses to claim about provenance (§4.8): a fixed
/// `ReadOnly` row's dim detail. The `tool output` row's whole point — there is
/// no global flag, `toggle_all_tools` is a bulk write over the blocks
/// (`app.rs:2803` `fn toggle_all_tools(&mut self)`).
const TOOL_OUTPUT_NOTE: &str = "Ctrl-T expands / collapses all";

/// The `ReadOnly` rows' one-line hint when `Enter` is pressed (§4.2: "`ReadOnly`
/// sets a one-line hint") — and the placeholder a read-only row shows when it has
/// no note of its own.
const READONLY_HINT: &str = "read-only — `--dump-config`";

/// The `config` section's one row when no dump was injected (step 7 wires the
/// real text): a section is never empty, so `Tab` always has a target and the
/// heading is never the panel's last line.
const CONFIG_PLACEHOLDER: &str = "(no config injected)";

/// What `Enter` / `←` / `→` mean for a `Toggle` row. One variant per row that
/// uses `Toggle`, because the brief forbids matching on row ids (§4.3: "the only
/// match is over `SettingKind` — never over row ids") and the two toggles push
/// different effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// `plan`: flips `Status.plan` optimistically and asks `SetPlanMode`.
    Plan,
    /// `sidebar`: flips the client-local view bit. Instant, never pending.
    Sidebar,
}

/// What `←` / `→` / `Enter` cycle for a `Cycle` row. Same rationale as
/// [`Toggle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cycle {
    /// `effort`: the [`EFFORT_LEVELS`] cycle with the off-list rule.
    Effort,
    /// `view`: `focus` ⇄ `all` (`ViewMode`, `app.rs:2388`).
    View,
}

/// How a row is drawn and which keys act on it — the closed taxonomy of §4.3.
///
/// # Contracts
/// - Five variants, no more: a new kind is a new key behaviour, and `on_key`'s
///   match is exhaustive over this enum, so the compiler asks for it.
/// - A variant carries the row's *shape*; the row's *text* is [`Setting::value`]
///   and [`Setting::label`], pre-formatted by [`rows`] — the renderer formats
///   nothing.
/// - `Nudge`'s effect is `SetWidth` and `width` is the ONLY `Nudge` row. A
///   second one must add a target discriminant (the `Toggle`/`Cycle` pattern);
///   `the_only_nudge_row_is_width` is the guard.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    /// `Enter` flips; `←`/`→` are no-ops (§4.2).
    Toggle { on: bool, what: Toggle },
    /// `←`/`→`/`Enter` advance (back/forward/forward); wraps; preserves an
    /// off-list value until the first press (§4.4).
    ///
    /// `current` is the WIRE value the cycle turns from — `None` is the effort
    /// cycle's `(none)` stop, which `Setting::value` draws as the word
    /// `(none)`. The kind carries it so the key handler never parses the drawn
    /// column; a change to `value`'s formatting cannot change what a press does.
    Cycle {
        current: Option<String>,
        options: &'static [&'static str],
        what: Cycle,
    },
    /// `←`/`→` step by `step`, clamped to `min..=max`; `Enter` is a no-op.
    ///
    /// `current` is the machine number (`Setting::value` draws it with its unit:
    /// `"68 cols"`). Carried here for the same reason as [`Self::Cycle`]'s
    /// `current`: input never parses display text.
    Nudge {
        current: usize,
        min: usize,
        max: usize,
        step: usize,
    },
    /// `Enter` opens the existing picker; `←`/`→` are no-ops.
    Pick { kind: PickerKind },
    /// Nothing acts. `Enter` (or any key) sets [`SettingsState::hint`] — a
    /// deliberate, visible "there is nothing to edit here" (§4.2, §4.8).
    ReadOnly { note: Option<&'static str> },
}

/// One row of the panel — one DRAWN line, exactly like [`crate::app::InfoRow`],
/// which keeps the row/cursor arithmetic line-agnostic (`line_of`).
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    /// Stable identity (see [`ids`]). Never rendered — it is what the pending
    /// set is matched against (`rows`' marking pass).
    pub id: &'static str,
    /// The left column, already worded (`"tool output"`, `"provider: "`-less).
    /// `Cow` so a fixed row borrows its literal and a `config` row owns an
    /// injected line.
    pub label: Cow<'static, str>,
    pub kind: SettingKind,
    /// The right column, pre-formatted by [`rows`]: `"high"`, `"68 cols"`,
    /// `"shown"`, `"128000"`, a config line's value tail. Empty = no value.
    pub value: String,
    /// A change the harness ACCEPTED but has not applied yet (§4.5): only the
    /// three Request-backed rows (`model`, `effort`, `plan`) can be `true`.
    /// The `· pending` MARK's wording and style are `tui-design.md` §4's.
    pub pending: bool,
}

/// A group of rows under a heading line. Sections are the `Tab`/`BackTab`
/// targets (§4.2) and the panel has exactly two: `session` and `config`.
///
/// # Contracts
/// - `heading` is ALWAYS present — `tui-design.md` §4 amendment 5: "The section
///   label sits at column 0 and the **first** section is labelled too". So the
///   panel opens on `session`, not on a bare row list (first-layer B4).
/// - A blank separator line is drawn BETWEEN sections (`tui-design.md` §4: "One
///   blank `Prose` row sets the second section off"), so it is counted by
///   [`line_of`] and never stored.
/// - A section is never empty: an empty injected config yields ONE placeholder
///   row, so `Tab` always has somewhere to land.
/// - `rows` never produces a third section today; `Tab` wraps, so a third would
///   still cycle correctly.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub heading: &'static str,
    pub rows: Vec<Setting>,
}

/// The panel's own mutable state — the ONE thing `App` holds beside `overlay`.
/// Not a model of the settings: the values live in [`SettingsSnapshot`] (built
/// from `Status` and the client's view bits, both App-owned).
#[derive(Clone, Debug, Default)]
pub struct SettingsState {
    /// Index into the FLATTENED row list, headings excluded (the order [`rows`]
    /// emits, section by section). Clamped by [`on_key`] on every key, so a
    /// stale cursor cannot point past the end.
    cursor: usize,
    /// The one-line hint the bottom line shows instead of the key legend — set
    /// by an act on a `ReadOnly` row, cleared by any cursor move or by close.
    hint: Option<&'static str>,
    /// The body height the renderer last drew (`PgUp`/`PgDn`'s step). Published
    /// by `ui::draw` (`set_settings_page`), like `terminal_height`.
    page: usize,
}

impl SettingsState {
    /// A fresh panel at the top of the `session` section with no hint
    /// (`/settings` and `F2` both open with this). Pending marks are NOT here —
    /// see the module doc's deviation 4: they belong to `App`, so closing and
    /// reopening the panel cannot lose one.
    pub fn new() -> SettingsState {
        SettingsState {
            cursor: 0,
            hint: None,
            page: DEFAULT_PAGE,
        }
    }

    /// The cursor's flat row index (the renderer + `line_of` need it; nothing
    /// outside this module writes it — [`on_key`] owns the movement).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The hint line to draw instead of the key legend, if one was set.
    pub fn hint(&self) -> Option<&'static str> {
        self.hint
    }

    /// Publish the drawn body height (rows), so `PgUp`/`PgDn` move one screen.
    /// Called by the renderer each frame; `0` is ignored (not yet laid out).
    pub fn set_page(&mut self, page: usize) {
        if page > 0 {
            self.page = page;
        }
    }
}

/// The panel's live values, computed by `App::settings_snapshot()` and handed to
/// [`rows`]. Borrowed fields, no behaviour: this is the seam that keeps the row
/// builder pure and testable without an `App` or a terminal.
///
/// # Contracts
/// - Field order and names ARE the drawn order (the drawn table, §4).
/// - `None` in `theme` = the active palette has no catalog name (palette B, or a
///   `[theme]` inline palette) — see `theme::active_name()`.
/// - `None` in `context` = unknown, rendered as the honest absence the brief's
///   §11.8 accepts (`limits::model_limit` knows one endpoint family only).
/// - No `running`: the pending set already answers "is this change still in
///   flight?", which is what the row draws; `App::running()` is read once, at
///   the mutation site (`apply_setting`), to decide whether to mark one. See the
///   module doc's deviations 4 and 6.
#[derive(Clone, Copy, Debug)]
pub struct SettingsSnapshot<'a> {
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub plan: bool,
    pub theme: Option<&'a str>,
    pub width: usize,
    pub view: ViewMode,
    pub sidebar: bool,
    pub context: Option<u64>,
    /// The finished `--dump-config` text, injected by the CLI
    /// (`Options.config`). Split into `ReadOnly` rows by [`rows`].
    pub config: &'a str,
    /// Row ids whose change the harness ACCEPTED and has not applied yet — App's
    /// `deferred_settings` (§4.5). Empty when nothing is in flight; never longer
    /// than three, because only `model`/`effort`/`plan` go through the harness.
    pub pending: &'a [&'static str],
    /// [`CONFIG_HEADING_LOCAL`] or [`CONFIG_HEADING_CLIENT`] — App picks from
    /// `App::remote`.
    pub config_heading: &'static str,
}

/// Build the whole panel from a snapshot.
///
/// # Contracts
/// - PURE: no I/O, no globals, no `App`. Called from the key handler and from
///   the renderer, so the two cannot disagree about what is on screen.
/// - Row ORDER is fixed and is the drawn order (§4.3's table): `session` =
///   model, effort, plan, theme, width, view, sidebar, tool output, context;
///   then the `config` section, one row per non-blank line of `snapshot.config`.
///   Both sections are always present, so `Tab` always has two targets.
/// - IDS are [`ids`] consts, stable across builds and unique among the
///   `session` rows; all `config` rows share [`ids::CONFIG`].
/// - `Tab` targets the FIRST ROW OF THE NEXT SECTION, so `sections[1].rows[0]`
///   must be the first `config` line — never the heading (the heading is
///   `Section::heading`, outside the row list).
/// - `pending` is `snapshot.pending.contains(&id)`; only the three Request rows
///   can ever be `true` (§4.8), and a `config`/`ReadOnly` row is never pending —
///   even if a caller passes its id, because a `ReadOnly` row hard-codes `false`.
/// - An off-list effort is rendered VERBATIM (`"medium-high"`), never clamped —
///   the clamp is a deliberate keypress (§4.4).
/// - The `config` text is split on `\n` with trailing empties dropped; interior
///   empty lines are kept as blank rows (the `InfoRow::Prose("")` idiom). An
///   empty/whitespace-only `config` yields ONE `ReadOnly` placeholder row.
/// - THE TWO ABSENT-VALUE WORDS (first-layer blocker 2 — normative, so the last
///   line of defence is not a test comment): `theme: None` renders
///   **`palette b`** and `theme: Some(n)` renders `n` verbatim; `context: None`
///   renders **`unknown`**. They are different words because they are different
///   facts: `palette b` is the name of the palette actually applied (A9 — the
///   row names a SOURCE, so it may not borrow the catalogue's `default`), while
///   `unknown` is the honest absence of a context window (`limits::model_limit`
///   knows one endpoint family, `limits.rs:63`).
/// - Never matches on a row id.
pub fn rows(snapshot: &SettingsSnapshot<'_>) -> Vec<Section> {
    let session = vec![
        Setting {
            id: ids::MODEL,
            label: Cow::Borrowed("model"),
            kind: SettingKind::Pick {
                kind: PickerKind::Model,
            },
            value: snapshot.model.to_string(),
            pending: false,
        },
        Setting {
            id: ids::EFFORT,
            label: Cow::Borrowed("effort"),
            kind: SettingKind::Cycle {
                // `None` = the `(none)` stop; the drawn word below says so.
                current: snapshot.effort.map(str::to_string),
                options: EFFORT_LEVELS,
                what: Cycle::Effort,
            },
            // An off-list value is shown as it is; `(none)` is the free-style
            // spelling of the cycle's first stop (§4.4).
            value: snapshot.effort.unwrap_or("(none)").to_string(),
            pending: false,
        },
        Setting {
            id: ids::PLAN,
            label: Cow::Borrowed("plan"),
            kind: SettingKind::Toggle {
                on: snapshot.plan,
                what: Toggle::Plan,
            },
            value: if snapshot.plan { "on" } else { "off" }.to_string(),
            pending: false,
        },
        Setting {
            id: ids::THEME,
            label: Cow::Borrowed("theme"),
            kind: SettingKind::Pick {
                kind: PickerKind::Theme,
            },
            // A9: `palette b` names the SOURCE — never the catalogue's
            // `default`, which resolves to the same colours but is a name the
            // client never applied.
            value: snapshot.theme.unwrap_or("palette b").to_string(),
            pending: false,
        },
        Setting {
            id: ids::WIDTH,
            label: Cow::Borrowed("width"),
            kind: SettingKind::Nudge {
                current: snapshot.width,
                min: MEASURE_MIN,
                max: MEASURE_LIMIT,
                step: MEASURE_STEP,
            },
            value: format!("{} cols", snapshot.width),
            pending: false,
        },
        Setting {
            id: ids::VIEW,
            label: Cow::Borrowed("view"),
            kind: SettingKind::Cycle {
                current: Some(view_word(snapshot.view).to_string()),
                options: VIEW_MODES,
                what: Cycle::View,
            },
            value: view_word(snapshot.view).to_string(),
            pending: false,
        },
        Setting {
            id: ids::SIDEBAR,
            label: Cow::Borrowed("sidebar"),
            kind: SettingKind::Toggle {
                on: snapshot.sidebar,
                what: Toggle::Sidebar,
            },
            value: if snapshot.sidebar { "shown" } else { "hidden" }.to_string(),
            pending: false,
        },
        Setting {
            id: ids::TOOL_OUTPUT,
            label: Cow::Borrowed("tool output"),
            // There is no global flag: the note says so rather than inventing a
            // value (§4.8).
            kind: SettingKind::ReadOnly {
                note: Some(TOOL_OUTPUT_NOTE),
            },
            value: String::new(),
            pending: false,
        },
        Setting {
            id: ids::CONTEXT,
            label: Cow::Borrowed("context"),
            kind: SettingKind::ReadOnly { note: None },
            value: match snapshot.context {
                Some(tokens) => tokens.to_string(),
                None => "unknown".to_string(),
            },
            pending: false,
        },
    ];
    let mut sections = vec![
        Section {
            heading: "session",
            rows: session,
        },
        Section {
            heading: snapshot.config_heading,
            rows: config_rows(snapshot.config),
        },
    ];
    // §4.5's marks, in ONE pass after the build — the pass that reads
    // `Setting::id`. A `ReadOnly` row hard-codes `false`, so a config or a note
    // row can never claim a deferral even if a caller passes its id.
    for section in &mut sections {
        for row in &mut section.rows {
            row.pending = !matches!(row.kind, SettingKind::ReadOnly { .. })
                && snapshot.pending.contains(&row.id);
        }
    }
    sections
}

/// The `view` row's drawn word — `ViewMode::parse`'s spelling, so the token the
/// row prints is the token [`on_key`] parses back into a [`ViewMode`].
fn view_word(view: ViewMode) -> &'static str {
    match view {
        ViewMode::All => "all",
        ViewMode::Focus => "focus",
    }
}

/// The `config` section: one `ReadOnly` row per line of the injected dump, the
/// label the dump's own `key:` (colon kept) and the value the rest of the line,
/// so `--dump-config`'s text appears verbatim (§4.6, first-layer A10).
///
/// # Contracts
/// - Trailing empty lines are dropped; INTERIOR empty lines are kept as blank
///   rows (the `InfoRow::Prose("")` idiom).
/// - An empty or whitespace-only dump yields ONE [`CONFIG_PLACEHOLDER`] row, so
///   the section is never empty and `Tab` always has a target.
fn config_rows(config: &str) -> Vec<Setting> {
    let mut lines: Vec<&str> = config.split('\n').collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let line_rows = |lines: Vec<&str>| -> Vec<Setting> {
        lines
            .into_iter()
            .map(|line| {
                let (label, value) = match line.split_once(": ") {
                    Some((key, rest)) => (Cow::Owned(format!("{key}:")), rest.to_string()),
                    None => (Cow::Owned(line.to_string()), String::new()),
                };
                Setting {
                    id: ids::CONFIG,
                    label,
                    kind: SettingKind::ReadOnly { note: None },
                    value,
                    pending: false,
                }
            })
            .collect()
    };
    if lines.is_empty() {
        return vec![Setting {
            id: ids::CONFIG,
            label: Cow::Borrowed(CONFIG_PLACEHOLDER),
            kind: SettingKind::ReadOnly { note: None },
            value: String::new(),
            pending: false,
        }];
    }
    line_rows(lines)
}

/// The drawn-line geometry: `(line index of the cursor's row, total drawn
/// lines)`.
///
/// # Contracts
/// - One line per `Section::heading` (always present, amendment 5), one per row,
///   and **one blank separator between sections** (`sections.len() - 1`), so
///   `total = headings + separators + rows`. Omitting the separator desyncs the
///   cursor's line from the marked row by exactly one at the section boundary —
///   the one place `Tab` lands (first-layer B4).
/// - The panel has NO stored scroll offset (§4 "The panel, drawn": "the scroll
///   offset is derived from the cursor in the renderer"), so the renderer derives
///   `start` from these two numbers with the picker's own keep-the-row-visible
///   formula (`ui.rs` `picker.selected.saturating_sub(visible - 1).min(len - visible)`).
/// - `cursor` past the end is clamped to the last line, never a panic.
/// - # Panics: never; an empty `sections` yields `(0, 0)`.
pub(crate) fn line_of(sections: &[Section], cursor: usize) -> (usize, usize) {
    let headings = sections.len();
    let total = headings + headings.saturating_sub(1) + row_count(sections);
    let mut line = 0;
    let mut seen = 0;
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            line += 1; // the blank separator between sections
        }
        line += 1; // the heading itself
        if cursor < seen + section.rows.len() {
            return (line + (cursor - seen), total);
        }
        seen += section.rows.len();
        line += section.rows.len();
    }
    (total.saturating_sub(1), total)
}

/// The cursor's address space: one index per ROW, section by section (the
/// headings and the blank separator are never addressable).
fn row_count(sections: &[Section]) -> usize {
    sections.iter().map(|s| s.rows.len()).sum()
}

/// The row a flat cursor index addresses, across both sections.
fn row_at(sections: &[Section], cursor: usize) -> Option<&Setting> {
    sections.iter().flat_map(|s| &s.rows).nth(cursor)
}

/// What a key did — the whole of §4.2's grammar, so `App` never re-derives it.
///
/// The four variants are the three answers the caller needs plus the redraw:
/// `None` = swallow (no repaint needed beyond the frame the caller already
/// marks), `Moved` = repaint (cursor moved, or a hint was set), `Close` = clear
/// `App.settings`, `Act` = run the effect.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyOutcome {
    /// Swallowed: this panel does not own the key, or the row kind ignores it
    /// (`←`/`→` on a `Toggle`/`Pick`/`ReadOnly`). The caller marks the frame
    /// dirty anyway, as `on_overlay_key` does.
    None,
    /// The panel's own state moved (cursor / hint). Repaint; no effect.
    Moved,
    /// `Esc` — the ONLY close key, unchanged (§4.2). The caller clears
    /// `App.settings` and nothing else: `hint` dies with the state, `pending`
    /// survives (the deferral outlives the panel).
    Close,
    /// Act on the row under the cursor. The caller executes it in
    /// `App::apply_setting`, a flat match over [`SettingAction`].
    Act(SettingAction),
}

/// The effect vocabulary `App` executes — one variant per existing setter, so
/// `App::apply_setting` is a flat match and this module names no `App` type and
/// no `Request`.
///
/// # Contracts
/// - Every variant maps to ONE setter that already exists (§4.3's table); the
///   panel adds no behaviour, only a face.
/// - `SetPlan` is the DEFERRABLE one: the caller writes `Status` optimistically
///   and, when `App::running()`, marks the row pending (§4.5). The others settle
///   instantly and never mark pending (§4.8).
/// - `SetWidth`'s value is already clamped to the row's `min..=max`; the caller's
///   `set_measure` clamps again (harmless, `app.rs:2705`).
/// - `SetEffort` (step 4) is its own step's arm; its sketch is at the foot of
///   this file.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingAction {
    /// `width`: `App::set_measure(cols)` (`app.rs:2704`).
    SetWidth(usize),
    /// `view`: `App::set_view_mode(view)` (`app.rs:5460`, no-op when unchanged).
    SetView(ViewMode),
    /// `sidebar`: `App::toggle_sidebar()` (`app.rs:5442`).
    ToggleSidebar,
    /// `model`: open the EXISTING picker (`app.rs:5114`) — its accept arm
    /// (`:5217`-`:5221`) writes `Status.model` and asks `SetModel`, and the panel
    /// is still open underneath (the host decision's payoff, §4.1).
    ///
    /// ONE VARIANT PER PICKER (first-layer A3), not `OpenPicker(PickerKind)`: the
    /// other four `PickerKind`s (`Change`/`Resume`/`Team`/`Surface`) are not
    /// settings rows, so a payload would force `apply_setting` to grow a
    /// wildcard arm for impossible states — the silent-match class this item
    /// exists to avoid. Two variants keep the caller's match flat AND total.
    OpenModelPicker,
    /// `theme`: open the existing theme picker (`app.rs:5107`), whose accept arm
    /// (`:5241`-`:5245`) applies the preset.
    OpenThemePicker,
    /// `plan`: the `/plan` arm's body — `plan_pending`, `Status.plan`, and
    /// `Request::SetPlanMode` (`app.rs:4289`–`:4304`).
    SetPlan(bool),
}

/// The key grammar of §4.2, as one pure function.
///
/// # Contracts
/// - Takes the BUILT ROWS, not the snapshot: `App` builds them from `&self`
///   first, then takes `&mut` on `settings`, so no borrow conflict exists and
///   `PgUp`/`PgDn` clamp against exactly what was last drawn.
/// - FIRST the cursor is clamped to `0..total_rows` (a stale cursor after a
///   shorter row list self-heals; never a panic).
/// - `↑`/`↓` move one row, clamped; `PgUp`/`PgDn` move `state.page` rows,
///   clamped; `ScrollUp`/`ScrollDown` move [`WHEEL_ROWS`], clamped.
/// - `←`/`→` act per kind: `Nudge` steps ± `step` clamped to `min..=max`;
///   `Cycle` retreats/advances and wraps; `Toggle`/`Pick`/`ReadOnly` → `None`.
/// - `Enter` acts per kind: `Toggle` flips, `Cycle` advances, `Pick` opens the
///   picker, `Nudge` → `None`, `ReadOnly` sets the hint and returns `Moved`.
///   THE HINT RESOLVES IN THIS ORDER (first-layer nit 4 — stated at the
///   implementation site, not only in a test comment): `ReadOnly { note: Some(n) }`
///   sets `n` verbatim (so `tool output` says `Ctrl-T expands / collapses all`),
///   and `note: None` falls back to [`READONLY_HINT`] (`--dump-config`, right for
///   a config row and wrong for `tool output` — hence the order).
/// - On a `ReadOnly` row `↑`/`↓` still MOVE and clear the hint (first-layer B7);
///   `←`/`→` are no-ops and leave it alone. Without that split the whole `config`
///   section is a trap: `Tab` can enter it, arrows could not leave.
/// - `Tab`/`BackTab` put the cursor on the first row of the next/previous
///   SECTION, wrapping (`session` ⇄ `config`). New grammar with no precedent
///   (§4.2): inside the panel `BackTab` no longer switches surfaces —
///   `app.rs:4031` `Key::BackTab => self.focus_prev(),` is shadowed.
/// - `Esc` → [`KeyOutcome::Close`]. No other key closes: no `q`, no `Enter`.
/// - Any cursor move clears `state.hint`; `Esc` leaves `pending` untouched.
/// - Everything else → [`KeyOutcome::None`] (swallowed — the panel owns the
///   keyboard, exactly as a modal does).
/// - Deliberate divergence from `Overlay::Info`: `Enter` is NOT swallowed here,
///   because `accept_picker`'s `self.overlay.take()` removes the picker, not the
///   panel (`app.rs:5203`).
pub fn on_key(state: &mut SettingsState, key: Key, sections: &[Section]) -> KeyOutcome {
    let total = row_count(sections);
    state.cursor = state.cursor.min(total.saturating_sub(1));
    match key {
        Key::Esc => KeyOutcome::Close,
        Key::Up => step(state, -1, total),
        Key::Down => step(state, 1, total),
        Key::PageUp => step(state, -(state.page as isize), total),
        Key::PageDown => step(state, state.page as isize, total),
        Key::ScrollUp => step(state, -(WHEEL_ROWS as isize), total),
        Key::ScrollDown => step(state, WHEEL_ROWS as isize, total),
        Key::Tab => jump_section(state, sections, 1),
        Key::BackTab => jump_section(state, sections, -1),
        Key::Left | Key::Right | Key::Enter => act(state, sections, key),
        _ => KeyOutcome::None,
    }
}

/// Move the cursor `by` rows, clamped to the address space, clearing the hint —
/// the one place §4.2's "any cursor move clears `state.hint`" is written.
fn step(state: &mut SettingsState, by: isize, total: usize) -> KeyOutcome {
    let target = state
        .cursor
        .saturating_add_signed(by)
        .min(total.saturating_sub(1));
    if target != state.cursor {
        state.cursor = target;
        state.hint = None;
    }
    KeyOutcome::Moved
}

/// `Tab`/`BackTab`: the FIRST row of the next/previous section, wrapping. A
/// section is never empty (an un-injected config yields a placeholder), so the
/// walk is a guard rather than a need — with every section empty it is a no-op.
fn jump_section(state: &mut SettingsState, sections: &[Section], dir: isize) -> KeyOutcome {
    let count = sections.len();
    if count == 0 {
        return KeyOutcome::None;
    }
    // The section the cursor sits in, walking the flat row indices.
    let mut pos = 0;
    let mut seen = 0;
    for (i, section) in sections.iter().enumerate() {
        pos = i;
        if state.cursor < seen + section.rows.len() {
            break;
        }
        seen += section.rows.len();
    }
    let mut i = pos;
    let mut target = None;
    for _ in 0..count {
        i = if dir >= 0 {
            (i + 1) % count
        } else {
            (i + count - 1) % count
        };
        if !sections[i].rows.is_empty() {
            target = Some(sections[..i].iter().map(|s| s.rows.len()).sum::<usize>());
            break;
        }
    }
    let Some(target) = target else {
        return KeyOutcome::None;
    };
    if target != state.cursor {
        state.cursor = target;
        state.hint = None;
    }
    KeyOutcome::Moved
}

/// `←` / `→` / `Enter` on the row under the cursor — §4.2's per-kind table, and
/// the ONLY place that reads [`SettingKind`] to decide an effect.
fn act(state: &mut SettingsState, sections: &[Section], key: Key) -> KeyOutcome {
    let Some(row) = row_at(sections, state.cursor) else {
        return KeyOutcome::None;
    };
    let forward = key != Key::Left;
    match &row.kind {
        // `Enter` flips; `←`/`→` are no-ops on a toggle (§4.2).
        SettingKind::Toggle { on, what } => {
            if key != Key::Enter {
                return KeyOutcome::None;
            }
            KeyOutcome::Act(match what {
                Toggle::Plan => SettingAction::SetPlan(!on),
                Toggle::Sidebar => SettingAction::ToggleSidebar,
            })
        }
        SettingKind::Cycle { current, options, what } => match what {
            Cycle::View => match cycle_value(current.as_deref(), options, forward) {
                Some(next) => match ViewMode::parse(next) {
                    Ok(view) => KeyOutcome::Act(SettingAction::SetView(view)),
                    Err(_) => KeyOutcome::None,
                },
                None => KeyOutcome::None,
            },
            // STEP 4: the effort cycle is the off-list rule plus the wire
            // mapping (`effort_next`), so the row renders but does not turn yet.
            Cycle::Effort => KeyOutcome::None,
        },
        // `←`/`→` step the measure, clamped; `Enter` is a no-op (§4.2). The step
        // starts from the KIND's number, never from `value`'s drawn text.
        SettingKind::Nudge {
            current,
            min,
            max,
            step,
        } => {
            if key == Key::Enter {
                return KeyOutcome::None;
            }
            let next = if forward {
                current.saturating_add(*step).min(*max)
            } else {
                current.saturating_sub(*step).max(*min)
            };
            KeyOutcome::Act(SettingAction::SetWidth(next))
        }
        // `Enter` opens the existing picker (`Overlay::Pick`), whose accept arm
        // applies the value while the panel stays open underneath. `←`/`→` are
        // no-ops (§4.2): a picker is a choice, not a cycle.
        SettingKind::Pick { kind } => match (key, kind) {
            (Key::Enter, PickerKind::Model) => KeyOutcome::Act(SettingAction::OpenModelPicker),
            (Key::Enter, PickerKind::Theme) => KeyOutcome::Act(SettingAction::OpenThemePicker),
            // `rows` builds `Pick` only for the two settings rows; the other four
            // kinds are a guard, never a case.
            _ => KeyOutcome::None,
        },
        // `Enter` sets the one-line hint; `←`/`→` are no-ops and leave it alone
        // (first-layer B7 — `↑`/`↓` never reach here, so they still move).
        SettingKind::ReadOnly { note } => match key {
            Key::Enter => {
                state.hint = Some(note.unwrap_or(READONLY_HINT));
                KeyOutcome::Moved
            }
            _ => KeyOutcome::None,
        },
    }
}

/// The token after (forward) or before (back) `current` in `options`, wrapping.
/// A value that is OFF the list lands on the FIRST entry either way — the
/// visible, deliberate move §4.4 asks for, never a silent clamp.
fn cycle_value(
    current: Option<&str>,
    options: &[&'static str],
    forward: bool,
) -> Option<&'static str> {
    let len = options.len();
    if len == 0 {
        return None;
    }
    let position = current.and_then(|value| options.iter().position(|option| *option == value));
    let next = match position {
        Some(i) if forward => (i + 1) % len,
        Some(i) => (i + len - 1) % len,
        None => 0,
    };
    options.get(next).copied()
}

#[cfg(test)]
mod tests {
    //! The pure half of §7.1 — no terminal, no `App`. The integration tests
    //! (`esc_closes_the_settings_panel_and_nothing_else_does`, …) live in
    //! `app.rs`'s `mod tests`, where `App::handle` is reachable.

    use super::*;

    /// A dumped-config sample: the shape `config_dump` produces (`main.rs:504`
    /// `fn config_dump(cfg: &Config) -> String {`), including the trailing `\n`
    /// and one interior blank row so the splitting rule is exercised.
    const DUMP: &str = "provider: openai-compatible\n\
                        endpoint: chat (source: auto)\n\
                        base_url: http://localhost:11434/v1 (source: flag)\n\
                        model: qwen2.5-coder:7b (source: config)\n\
                        api_key: (none) (source: default)\n\
                        \n\
                        compaction budget: 60000\n";

    /// A snapshot with every field at a known value, so a test states only the
    /// field it is about: no pending marks, no off-list effort.
    fn snap() -> SettingsSnapshot<'static> {
        SettingsSnapshot {
            model: "qwen2.5-coder:7b",
            effort: None,
            plan: false,
            theme: Some("nord"),
            width: 68,
            view: ViewMode::Focus,
            sidebar: true,
            context: Some(128_000),
            config: DUMP,
            pending: &[],
            config_heading: CONFIG_HEADING_LOCAL,
        }
    }

    /// One press of `key` against `snap()`'s rows — the grammar tests' entry.
    fn press(state: &mut SettingsState, snapshot: &SettingsSnapshot<'_>, key: Key) -> KeyOutcome {
        on_key(state, key, &rows(snapshot))
    }

    /// The row `id` appears in, across both sections.
    fn row<'a>(sections: &'a [Section], id: &str) -> &'a Setting {
        sections
            .iter()
            .flat_map(|s| &s.rows)
            .find(|r| r.id == id)
            .expect("row id present")
    }

    /// Put the cursor on `id` — `Tab`-free (the tests that use it are about the
    /// row, not about how the cursor got there).
    fn focus(state: &mut SettingsState, snapshot: &SettingsSnapshot<'_>, id: &str) {
        let sections = rows(snapshot);
        let index = sections
            .iter()
            .flat_map(|s| &s.rows)
            .position(|r| r.id == id)
            .expect("row id present");
        while state.cursor() < index {
            press(state, snapshot, Key::Down);
        }
        while state.cursor() > index {
            press(state, snapshot, Key::Up);
        }
        assert_eq!(state.cursor(), index, "the cursor reached {id}");
    }

    // ---- the drawn rows ----------------------------------------------------

    #[test]
    fn rows_emits_the_two_section_headings() {
        let sections = rows(&snap());
        assert_eq!(sections.len(), 2, "session + config, always both");
        assert_eq!(sections[0].heading, "session");
        assert_eq!(sections[1].heading, "config");
        // A `--socket` client labels the section with whose config it shows.
        let client = rows(&SettingsSnapshot {
            config_heading: CONFIG_HEADING_CLIENT,
            ..snap()
        });
        assert_eq!(
            client[0].heading, "session",
            "the first section is labelled too"
        );
        assert_eq!(client[1].heading, "config (this client)");
    }

    #[test]
    fn the_row_list_names_every_session_knob_once() {
        let sections = rows(&snap());
        // model/effort/plan/theme/width/view/sidebar each appear exactly once —
        // by `ids` const, so a rename is one edit, and the drift guard is the
        // cheap half of v1's candidate C-f.
        for id in [
            ids::MODEL,
            ids::EFFORT,
            ids::PLAN,
            ids::THEME,
            ids::WIDTH,
            ids::VIEW,
            ids::SIDEBAR,
            ids::TOOL_OUTPUT,
            ids::CONTEXT,
        ] {
            let count = sections
                .iter()
                .flat_map(|s| &s.rows)
                .filter(|r| r.id == id)
                .count();
            assert_eq!(count, 1, "{id} appears {count} times");
        }
        // `tool output` and `context` are the read-only pair (§4.8: no global
        // flag, and no invented number).
        for id in [ids::TOOL_OUTPUT, ids::CONTEXT] {
            assert!(
                matches!(row(&sections, id).kind, SettingKind::ReadOnly { .. }),
                "{id} is read-only"
            );
        }
        // The session section is exactly those nine, in the drawn order.
        assert_eq!(
            sections[0].rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![
                ids::MODEL,
                ids::EFFORT,
                ids::PLAN,
                ids::THEME,
                ids::WIDTH,
                ids::VIEW,
                ids::SIDEBAR,
                ids::TOOL_OUTPUT,
                ids::CONTEXT,
            ]
        );
    }

    #[test]
    fn the_theme_row_names_the_builtin_palette_when_there_is_no_catalog_name() {
        // `None` (palette B / an inline `[theme]` palette) renders `palette b` —
        // NOT `unknown` (the `context` row's word) and NOT `default` (A9:
        // `default` is a catalogue entry resolving to the SAME colours, so
        // borrowing it would claim a source the client never applied).
        let plain = rows(&SettingsSnapshot {
            theme: None,
            ..snap()
        });
        assert_eq!(row(&plain, ids::THEME).value, "palette b");
        let named = rows(&snap());
        assert_eq!(row(&named, ids::THEME).value, "nord");
        // The two absent-value words are different facts, so they never
        // coincide: `unknown` belongs to the context row alone.
        let unknown = rows(&SettingsSnapshot {
            context: None,
            ..snap()
        });
        assert_eq!(row(&unknown, ids::CONTEXT).value, "unknown");
        assert_eq!(row(&unknown, ids::THEME).value, "nord");
    }

    #[test]
    fn the_row_values_are_pre_formatted_for_the_renderer() {
        let sections = rows(&snap());
        assert_eq!(row(&sections, ids::MODEL).value, "qwen2.5-coder:7b");
        assert_eq!(row(&sections, ids::EFFORT).value, "(none)");
        assert_eq!(row(&sections, ids::PLAN).value, "off");
        assert_eq!(row(&sections, ids::WIDTH).value, "68 cols");
        assert_eq!(row(&sections, ids::VIEW).value, "focus");
        assert_eq!(row(&sections, ids::SIDEBAR).value, "shown");
        assert_eq!(row(&sections, ids::CONTEXT).value, "128000");
        assert_eq!(
            row(&sections, ids::TOOL_OUTPUT).value,
            "",
            "a note row carries no value"
        );
        // An off-list effort is shown verbatim, never clamped (§4.4).
        let off = rows(&SettingsSnapshot {
            effort: Some("medium-high"),
            ..snap()
        });
        assert_eq!(row(&off, ids::EFFORT).value, "medium-high");
    }

    #[test]
    fn line_geometry_counts_one_line_per_heading() {
        let sections = rows(&snap());
        let rows_total = row_count(&sections);
        let total = sections.len() + (sections.len() - 1) + rows_total;
        // Two headings, one blank separator, then every row.
        assert_eq!(total, 2 + 1 + 9 + 7, "9 session rows + 7 dump lines");
        // The cursor's first row sits BELOW the `session` heading, never on it.
        assert_eq!(line_of(&sections, 0), (1, total));
        // Crossing into `config` skips the blank separator and both headings:
        // 1 + 9 session rows + 1 blank + 1 heading = 12.
        assert_eq!(line_of(&sections, 9), (12, total));
        // A cursor past the end clamps to the last drawn line.
        assert_eq!(line_of(&sections, 99), (total - 1, total));
        // An empty panel has no lines at all.
        assert_eq!(line_of(&[], 0), (0, 0));
    }

    #[test]
    fn the_config_section_has_one_row_per_dumped_line() {
        let sections = rows(&snap());
        let config = &sections[1].rows;
        // Six dumped lines plus the interior blank row; the trailing `\n` is
        // dropped, and the text is not re-wrapped or re-split beyond `": "`.
        assert_eq!(config.len(), 7);
        assert_eq!(config[0].label, "provider:");
        assert_eq!(config[0].value, "openai-compatible");
        assert_eq!(config[2].label, "base_url:");
        assert_eq!(
            config[2].value, "http://localhost:11434/v1 (source: flag)",
            "the value is verbatim, colon included"
        );
        assert_eq!(config[5].label, "", "the interior blank row is kept");
        assert_eq!(config[6].label, "compaction budget:");
        assert_eq!(config[6].value, "60000");
    }

    #[test]
    fn an_empty_config_renders_a_placeholder_not_a_blank_section() {
        for empty in ["", "   ", "\n\n"] {
            let sections = rows(&SettingsSnapshot {
                config: empty,
                ..snap()
            });
            assert_eq!(sections.len(), 2, "the section still exists");
            assert_eq!(sections[1].rows.len(), 1, "one placeholder row");
            assert_eq!(sections[1].rows[0].label, CONFIG_PLACEHOLDER);
            assert!(matches!(
                sections[1].rows[0].kind,
                SettingKind::ReadOnly { .. }
            ));
        }
    }

    #[test]
    fn every_config_row_shares_one_id_and_no_session_id_repeats() {
        let sections = rows(&snap());
        assert!(
            sections[1].rows.iter().all(|r| r.id == ids::CONFIG),
            "every config row shares ids::CONFIG"
        );
        let mut seen: Vec<&str> = Vec::new();
        for row in &sections[0].rows {
            assert!(!seen.contains(&row.id), "{} repeats", row.id);
            seen.push(row.id);
        }
        // Nothing is pending in this snapshot, and a config row can never be.
        assert!(
            sections.iter().flat_map(|s| &s.rows).all(|r| !r.pending),
            "an empty pending set marks nothing"
        );
        assert!(
            sections[1].rows.iter().all(|r| !r.pending),
            "a config row hard-codes false"
        );
    }

    #[test]
    fn the_only_nudge_row_is_width() {
        let sections = rows(&snap());
        let nudges: Vec<&Setting> = sections
            .iter()
            .flat_map(|s| &s.rows)
            .filter(|r| matches!(r.kind, SettingKind::Nudge { .. }))
            .collect();
        assert_eq!(nudges.len(), 1, "`Nudge` -> `SetWidth` is hardcoded in `act`");
        assert_eq!(nudges[0].id, ids::WIDTH);
        assert_eq!(
            nudges[0].kind,
            SettingKind::Nudge {
                current: 68,
                min: MEASURE_MIN,
                max: MEASURE_LIMIT,
                step: MEASURE_STEP,
            }
        );
    }

    // ---- the key grammar ---------------------------------------------------

    #[test]
    fn nudge_clamps_at_the_measure_bounds() {
        let mut state = SettingsState::new();
        let narrow = SettingsSnapshot {
            width: MEASURE_MIN,
            ..snap()
        };
        focus(&mut state, &narrow, ids::WIDTH);
        assert_eq!(
            press(&mut state, &narrow, Key::Left),
            KeyOutcome::Act(SettingAction::SetWidth(MEASURE_MIN)),
            "40 - step stays at the floor"
        );
        let wide = SettingsSnapshot {
            width: MEASURE_LIMIT,
            ..snap()
        };
        focus(&mut state, &wide, ids::WIDTH);
        assert_eq!(
            press(&mut state, &wide, Key::Right),
            KeyOutcome::Act(SettingAction::SetWidth(MEASURE_LIMIT)),
            "200 + step stays at the ceiling"
        );
        let mut state = SettingsState::new();
        focus(&mut state, &snap(), ids::WIDTH);
        assert_eq!(
            press(&mut state, &snap(), Key::Right),
            KeyOutcome::Act(SettingAction::SetWidth(68 + MEASURE_STEP)),
            "68 + → steps by MEASURE_STEP"
        );
        assert_eq!(
            press(&mut state, &snap(), Key::Left),
            KeyOutcome::Act(SettingAction::SetWidth(68 - MEASURE_STEP)),
            "and ← steps back — from the SNAPSHOT's width, because `rows` is \
             rebuilt from it on every press"
        );
        // `Enter` is a no-op on a Nudge.
        assert_eq!(press(&mut state, &snap(), Key::Enter), KeyOutcome::None);
    }

    #[test]
    fn the_nudge_steps_from_the_kind_not_from_the_drawn_value() {
        // The `width` row draws `"68 cols"`; the press must not depend on that
        // formatting. Rewriting the drawn column to something unparsable leaves
        // the step identical — the machine value lives in `SettingKind::Nudge`.
        let snapshot = snap();
        let mut sections = rows(&snapshot);
        for row in sections.iter_mut().flat_map(|s| &mut s.rows) {
            if row.id == ids::WIDTH {
                row.value = "not a number".to_string();
            }
        }
        let mut state = SettingsState::new();
        for _ in 0..4 {
            on_key(&mut state, Key::Down, &sections);
        }
        assert_eq!(
            on_key(&mut state, Key::Right, &sections),
            KeyOutcome::Act(SettingAction::SetWidth(68 + MEASURE_STEP)),
            "the step comes from the kind, not the drawn text"
        );
        // The same for the cycle: its `current` is the wire value, so a rewritten
        // `value` cannot change which token comes next.
        let mut sections = rows(&snapshot);
        for row in sections.iter_mut().flat_map(|s| &mut s.rows) {
            if row.id == ids::VIEW {
                row.value = "??".to_string();
            }
        }
        let mut state = SettingsState::new();
        while state.cursor() < 5 {
            on_key(&mut state, Key::Down, &sections);
        }
        assert_eq!(
            on_key(&mut state, Key::Right, &sections),
            KeyOutcome::Act(SettingAction::SetView(ViewMode::All)),
            "the cycle turns from the kind too"
        );
    }

    #[test]
    fn enter_acts_per_kind_and_arrows_are_no_ops_on_a_toggle() {
        let mut state = SettingsState::new();
        focus(&mut state, &snap(), ids::PLAN);
        assert_eq!(
            press(&mut state, &snap(), Key::Enter),
            KeyOutcome::Act(SettingAction::SetPlan(true)),
            "Enter flips plan off -> on"
        );
        assert_eq!(
            press(&mut state, &snap(), Key::Left),
            KeyOutcome::None,
            "← is a no-op on a Toggle"
        );
        let mut state = SettingsState::new();
        focus(&mut state, &snap(), ids::SIDEBAR);
        assert_eq!(
            press(&mut state, &snap(), Key::Enter),
            KeyOutcome::Act(SettingAction::ToggleSidebar)
        );
        // A `Cycle` advances on `Enter` and `→`, retreats on `←`, and wraps.
        let mut state = SettingsState::new();
        focus(&mut state, &snap(), ids::VIEW);
        assert_eq!(
            press(&mut state, &snap(), Key::Enter),
            KeyOutcome::Act(SettingAction::SetView(ViewMode::All))
        );
        assert_eq!(
            press(&mut state, &snap(), Key::Right),
            KeyOutcome::Act(SettingAction::SetView(ViewMode::All))
        );
        let all = SettingsSnapshot {
            view: ViewMode::All,
            ..snap()
        };
        assert_eq!(
            press(&mut state, &all, Key::Right),
            KeyOutcome::Act(SettingAction::SetView(ViewMode::Focus)),
            "the cycle wraps at the end"
        );
        assert_eq!(
            press(&mut state, &all, Key::Left),
            KeyOutcome::Act(SettingAction::SetView(ViewMode::Focus)),
            "and `←` is the other direction, also wrapping"
        );
    }

    #[test]
    fn enter_on_a_pick_row_opens_its_picker_and_arrows_do_not() {
        let mut state = SettingsState::new();
        // The cursor opens on `model`.
        assert_eq!(
            press(&mut state, &snap(), Key::Enter),
            KeyOutcome::Act(SettingAction::OpenModelPicker)
        );
        // `←`/`→` are no-ops on a Pick — a picker is a choice, not a cycle.
        for key in [Key::Left, Key::Right] {
            assert_eq!(press(&mut state, &snap(), key), KeyOutcome::None, "{key:?}");
        }
        // `theme` is the other `Pick` row.
        focus(&mut state, &snap(), ids::THEME);
        assert_eq!(
            press(&mut state, &snap(), Key::Enter),
            KeyOutcome::Act(SettingAction::OpenThemePicker)
        );
        // The effort row does not cycle yet (step 4): `Enter` is swallowed, not a
        // silent no-op action.
        focus(&mut state, &snap(), ids::EFFORT);
        assert_eq!(press(&mut state, &snap(), Key::Enter), KeyOutcome::None);
    }

    #[test]
    fn read_only_rows_yield_a_hint_not_an_action() {
        // FIRST-LAYER B7 — the split is the point, and the first cut got it wrong:
        //  - `Enter` on `tool output` / `context` / a config row → `Moved`, never
        //    `Act`, with `state.hint()` set (§4.2 "`ReadOnly` sets a one-line hint");
        //  - `←`/`→` on those rows → `None`, hint unchanged ("no-ops", §4.2);
        //  - `↑`/`↓` on those rows → MOVE the cursor and CLEAR the hint, exactly as
        //    on an editable row — otherwise every row of the `config` section is a
        //    trap: `Tab` can enter that section but `↑`/`↓` could never leave it.
        let mut state = SettingsState::new();
        focus(&mut state, &snap(), ids::TOOL_OUTPUT);
        assert_eq!(press(&mut state, &snap(), Key::Enter), KeyOutcome::Moved);
        assert_eq!(
            state.hint(),
            Some(TOOL_OUTPUT_NOTE),
            "`tool output` says Ctrl-T, never --dump-config"
        );
        // `←`/`→` leave the hint alone.
        assert_eq!(press(&mut state, &snap(), Key::Left), KeyOutcome::None);
        assert_eq!(press(&mut state, &snap(), Key::Right), KeyOutcome::None);
        assert_eq!(state.hint(), Some(TOOL_OUTPUT_NOTE));
        // `↑`/`↓` move and clear it.
        assert_eq!(press(&mut state, &snap(), Key::Down), KeyOutcome::Moved);
        assert_eq!(state.hint(), None, "a cursor move clears the hint");
        assert_eq!(state.cursor(), 8, "on to `context`");
        // A `ReadOnly { note: None }` falls back to the `--dump-config` hint.
        assert_eq!(press(&mut state, &snap(), Key::Enter), KeyOutcome::Moved);
        assert_eq!(state.hint(), Some(READONLY_HINT));
        // A config row behaves the same, and `Tab` can leave the section too.
        focus(&mut state, &snap(), ids::CONFIG);
        assert_eq!(press(&mut state, &snap(), Key::Enter), KeyOutcome::Moved);
        assert_eq!(state.hint(), Some(READONLY_HINT));
        assert_eq!(press(&mut state, &snap(), Key::Up), KeyOutcome::Moved);
        assert_eq!(state.hint(), None, "arrows leave the config section");
    }

    #[test]
    fn tab_jumps_between_the_two_sections() {
        let sections = rows(&snap());
        let config_start = sections[0].rows.len();
        // From ANY session row, `Tab` lands on the first `config` row.
        for start in [0, 3, 8] {
            let mut state = SettingsState::new();
            while state.cursor() < start {
                press(&mut state, &snap(), Key::Down);
            }
            assert_eq!(press(&mut state, &snap(), Key::Tab), KeyOutcome::Moved);
            assert_eq!(state.cursor(), config_start, "from row {start}");
            // `BackTab` returns to the first `session` row.
            assert_eq!(press(&mut state, &snap(), Key::BackTab), KeyOutcome::Moved);
            assert_eq!(state.cursor(), 0, "back to `model`");
        }
        // `Tab` from the config section wraps to `session` again.
        let mut state = SettingsState::new();
        while state.cursor() < config_start {
            press(&mut state, &snap(), Key::Tab);
        }
        assert_eq!(state.cursor(), config_start, "on the first config row");
        assert_eq!(press(&mut state, &snap(), Key::Tab), KeyOutcome::Moved);
        assert_eq!(state.cursor(), 0, "Tab from the config section wraps");
    }

    #[test]
    fn wheel_and_page_move_the_cursor_three_and_a_page() {
        let mut state = SettingsState::new();
        let sections = rows(&snap());
        let last = row_count(&sections) - 1;
        assert_eq!(press(&mut state, &snap(), Key::ScrollDown), KeyOutcome::Moved);
        assert_eq!(state.cursor(), WHEEL_ROWS, "one notch is three rows");
        assert_eq!(press(&mut state, &snap(), Key::ScrollUp), KeyOutcome::Moved);
        assert_eq!(state.cursor(), 0);
        state.set_page(4);
        assert_eq!(press(&mut state, &snap(), Key::PageDown), KeyOutcome::Moved);
        assert_eq!(state.cursor(), 4, "a page is the published body height");
        for _ in 0..8 {
            press(&mut state, &snap(), Key::PageDown);
        }
        assert_eq!(state.cursor(), last, "and it clamps at the last row");
    }

    #[test]
    fn a_stale_cursor_clamps_instead_of_panicking() {
        let mut state = SettingsState::new();
        for _ in 0..40 {
            press(&mut state, &snap(), Key::Down);
        }
        assert_eq!(state.cursor(), row_count(&rows(&snap())) - 1, "clamped");
        // A rebuild that shortens the list self-heals on the next key, and the
        // renderer's `line_of` is clamped too.
        let short = SettingsSnapshot {
            config: "",
            ..snap()
        };
        assert_eq!(press(&mut state, &short, Key::Down), KeyOutcome::Moved);
        assert_eq!(state.cursor(), row_count(&rows(&short)) - 1);
        let sections = rows(&short);
        assert_eq!(
            line_of(&sections, 99),
            (line_of(&sections, 0).1 - 1, line_of(&sections, 0).1)
        );
    }

    #[test]
    fn esc_closes_and_only_esc_does() {
        let sections = rows(&snap());
        let mut state = SettingsState::new();
        for key in [Key::Enter, Key::Char('q'), Key::Char('x'), Key::Tab] {
            assert_ne!(on_key(&mut state, key, &sections), KeyOutcome::Close, "{key:?}");
        }
        let before = state.cursor();
        assert_eq!(on_key(&mut state, Key::Esc, &sections), KeyOutcome::Close);
        assert_eq!(state.cursor(), before, "Esc closes without moving");
        // Movement keys are ANSWERED (not swallowed) — the `_ => {}` trap guard.
        for key in [Key::Up, Key::Down, Key::PageUp, Key::ScrollDown] {
            assert_eq!(on_key(&mut state, key, &sections), KeyOutcome::Moved, "{key:?}");
        }
        assert_eq!(on_key(&mut state, Key::Char('x'), &sections), KeyOutcome::None);
    }
}

// ==== SKETCH (review-only, not real code) — W032 step 4: the effort cycle.
// Kept VERBATIM from the reviewed sketch: the step that constructs it uncomments
// these and deletes this block (see the module doc's "Real so far").
//
// Step 4 registers `SetEffort`, `effort_position` / `effort_at` / `effort_next`
// below, and replaces `Cycle::Effort => KeyOutcome::None` in `act` with the
// off-list rule.
//
// /// `effort`: `Status.effort` optimistically + `Request::SetEffort` (the
// /// `/effort` arm, `app.rs:4252`–`:4261`, which today writes no `Status`).
// SetEffort(Option<String>),
//
// ---- the effort helpers, verbatim ----
//
// /// The cycle position of `effort`, or `None` when the value is OFF the list.
// /// Position `0` is `(none)` (`None` on the wire, and also the synonyms
// /// `"none"`/`"off"` that `/effort` already treats as "clear", `app.rs:4255`);
// /// positions `1..` are `EFFORT_LEVELS[1..]`. One function, so the off-list rule
// /// (§4.4) is written once for both `rows` and `on_key`.
// fn effort_position(effort: Option<&str>) -> Option<usize> {
//     todo!()
// }
//
// /// The wire value at a cycle position: `None` at 0, `Some(EFFORT_LEVELS[i])`
// /// above it — byte-identical to what `/effort <level>` sends.
// fn effort_at(position: usize) -> Option<String> {
//     todo!()
// }
//
// /// `Enter`/`→` = forward, `←` = back. From OFF the list BOTH directions land on
// /// `(none)` — "any arrow press moves to the first list entry" (§4.4) — a
// /// deliberate, visible action rather than a silent clamp. Otherwise wraps over
// /// `EFFORT_LEVELS.len()`.
// fn effort_next(effort: Option<&str>, forward: bool) -> Option<String> {
//     todo!()
// }
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the tests of steps 3–6 ====
// Helpers + one test per row-level rule that needs the steps above.
//
// #[test]
// fn cycle_wraps_and_preserves_an_off_list_value() {
//     todo!()
//     // `None` → minimal → … → max → `None`; `Some("medium-high")` renders
//     // verbatim and ONE arrow press lands on `(none)` — either direction.
// }
//
// #[test]
// fn only_the_three_request_rows_can_be_pending() {
//     todo!()
//     // With `snap().pending = &[ids::MODEL, ids::EFFORT, ids::PLAN]`, exactly
//     // those three rows are `pending` and every other row is `false` —
//     // including all `config` rows and `tool output`/`context` (§4.8).
//     //
//     // The CLOSE-survival half of §4.5 (`Esc` clears no mark) is App-level —
//     // `App::deferred_settings` is not inside the `Option` that close drops —
//     // and is pinned by `a_mid_run_setting_shows_pending_until_the_reply` in
//     // `app.rs`'s tests, where `App::handle` is reachable.
// }
// ==== /SKETCH ====
