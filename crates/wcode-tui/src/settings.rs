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
//! # Real so far (step 1) versus still sketch
//!
//! Real: the two [`CONFIG_HEADING_LOCAL`]/[`CONFIG_HEADING_CLIENT`] constants,
//! [`Section`] (headings only), [`SettingsState`], [`SettingsSnapshot`] (its
//! heading field), [`rows`], [`line_of`], [`KeyOutcome`] and [`on_key`] — the
//! host's model, the drawn-line geometry and the movement grammar, which is
//! exactly step 1's scope.
//!
//! Still a sketch, kept verbatim and COMMENTED at the foot of this file: the
//! row taxonomy (`ids`, `EFFORT_LEVELS`, `Toggle`, `Cycle`, `SettingKind`,
//! `Setting`, `SettingAction`, `effort_*`), the ten value fields of
//! [`SettingsSnapshot`], `Section::rows`, `KeyOutcome::Act` and the pure tests
//! of §7.1. They cannot be *declared* before a row exists to construct them:
//! `dead_code` fires on a declared-but-unconstructed type (and on an
//! unconstructed variant, and on a field nobody reads) inside a private module,
//! and this crate may not answer that with `#[allow]` (W032 §6). The step that
//! builds `rows` brings them back in one edit, because the sketch text is kept
//! verbatim.
//!
//! # Who calls this
//!
//! Only `crate::app` (the reducer) and `crate::ui` (the renderer):
//!
//! - `App::settings_sections()` builds a [`SettingsSnapshot`] from its own
//!   fields and calls [`rows`]; both the key handler and the renderer consume
//!   the result. `rows` is pure, so the two can never disagree.
//! - `App::on_settings_key` calls [`on_key`] and turns the returned
//!   [`KeyOutcome`] into App state — `Close` clears the panel, `Act` will run
//!   in `App::apply_setting` (the effect vocabulary is step 2+). This module
//!   touches no `App`, no `Request`, no terminal: it is data plus pure
//!   functions.
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

use crate::app::Key;

/// The `config` section's heading, local and over `--socket` (§4.6: a socket
/// client shows ITS OWN config, never the server's; §4.8: the panel never claims
/// to know the server's config).
pub const CONFIG_HEADING_LOCAL: &str = "config";
pub const CONFIG_HEADING_CLIENT: &str = "config (this client)";

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
    // STEP 2: `pub rows: Vec<Setting>,` — the row list, whose sketch is at the
    // foot of this file. Absent until the taxonomy lands (see the module doc).
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
/// **Step 1 holds the heading only.** Step 2 grows this to the ten value fields
/// of §4.3's drawn table, whose text is kept verbatim in the sketch at the foot
/// of this file; a declared-but-unread field is a `dead_code` warning and this
/// crate may not `#[allow]` it.
///
/// # Contracts
/// - Field order and names ARE the drawn order (the drawn table, §4).
/// - No `running`: the pending set already answers "is this change still in
///   flight?", which is what the row draws; `App::running()` is read once, at
///   the mutation site (`apply_setting`), to decide whether to mark one. See the
///   module doc's deviations 4 and 6.
#[derive(Clone, Copy, Debug)]
pub struct SettingsSnapshot {
    /// [`CONFIG_HEADING_LOCAL`] or [`CONFIG_HEADING_CLIENT`] — App picks from
    /// `App::remote`.
    pub config_heading: &'static str,
}

/// Build the whole panel from a snapshot.
///
/// **Step 1 emits the two section headings and no rows**; the row build is step
/// 2, and every contract below is what that build must satisfy.
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
pub fn rows(snapshot: &SettingsSnapshot) -> Vec<Section> {
    vec![
        Section {
            heading: "session",
        },
        Section {
            heading: snapshot.config_heading,
        },
    ]
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
    // STEP 2: walk the sections — heading, its rows, then the blank separator —
    // and stop on the cursor's row. With no rows there is no line of the
    // cursor's own, so the clamp below parks it on the last drawn line.
    (cursor.min(total.saturating_sub(1)), total)
}

/// The cursor's address space: one index per ROW, section by section (the
/// headings and the blank separator are never addressable).
///
/// STEP 2: `sections.iter().map(|s| s.rows.len()).sum()` once [`Section`]
/// carries its row list. Until then the address space is empty, which is what
/// makes every movement key below a clamp rather than a move.
fn row_count(_sections: &[Section]) -> usize {
    0
}

/// What a key did — the whole of §4.2's grammar, so `App` never re-derives it.
///
/// Three of the four variants of §4.2 are here: `None` = swallow (no repaint
/// needed beyond the frame the caller already marks), `Moved` = repaint (cursor
/// moved, or a hint was set), `Close` = clear `App.settings`. §4.2's
/// `Act` — run the effect — arrives with the rows that can produce one (step 2;
/// its sketch is at the foot of this file).
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
/// - `Esc` → [`KeyOutcome::Close`]. No other key closes: no `q`, no `Enter`.
/// - Any cursor move clears `state.hint`; `Esc` leaves `pending` untouched.
/// - Everything else → [`KeyOutcome::None`] (swallowed — the panel owns the
///   keyboard, exactly as a modal does).
///
/// The per-kind grammar — `←`/`→` acting per `SettingKind`, `Enter`, `Tab`/
/// `BackTab` jumping sections, the `ReadOnly` hint — lands with the rows (step
/// 2; the full contract is in the `on_key` sketch at the foot of this file,
/// which step 2 replaces this function with). Every one of those keys needs a
/// row to land on, and [`rows`] emits none yet, so they fall to `None` here.
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

// ==== SKETCH (review-only, not real code) — W032 steps 2–6: the row taxonomy,
// the effect vocabulary and the snapshot's value fields. Kept VERBATIM from the
// reviewed sketch: the step that builds `rows` uncomments these and deletes this
// block (see the module doc's "Real so far").
//
// Step 2 registers: `ids`, `Setting`, `SettingKind`, `Section::rows`,
// `SettingsSnapshot`'s ten value fields and `KeyOutcome::Act`.
// Step 3 registers: `SettingAction::OpenModelPicker` / `OpenThemePicker`.
// Step 4 registers: `EFFORT_LEVELS`, `Toggle`, `Cycle`, `SettingAction` and the
// three `effort_*` helpers.
// Step 6 registers: `SettingsSnapshot::pending`.
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the row ids (step 2) ====
// /// The stable row ids. ONE source: [`Setting::id`], the pending set, and the
// /// tests' drift guard all read these, so a rename is one edit.
// ///
// /// # Contracts
// /// - A `session` row's id is unique and constant across builds.
// /// - Every row of the read-only `config` section shares [`CONFIG`]: ids exist to
// ///   be *addressed* (pending, lookup), and a config row is never addressed — no
// ///   per-line id can be `&'static str` anyway (the text is injected at runtime).
// pub mod ids {
//     pub const MODEL: &str = "model";
//     pub const EFFORT: &str = "effort";
//     pub const PLAN: &str = "plan";
//     pub const THEME: &str = "theme";
//     pub const WIDTH: &str = "width";
//     pub const VIEW: &str = "view";
//     pub const SIDEBAR: &str = "sidebar";
//     pub const TOOL_OUTPUT: &str = "tool output";
//     pub const CONTEXT: &str = "context";
//     pub const CONFIG: &str = "config";
// }
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the effort vocabulary (step 4) ====
// /// The reasoning-effort vocabulary the `effort` row cycles.
// ///
// /// TWIN: `editors/vscode/src/tab.ts`'s `EFFORT_LEVELS` — the same seven, and
// /// each file's doc comment names the other. Change both or neither.
// ///
// /// CAVEAT, stated twice on purpose (once here, once in `tab.ts`): the KERNEL
// /// does not validate effort — `Request::SetEffort { effort: Option<String> }`
// /// is free text (`protocol.rs:262` `SetEffort { effort: Option<String> }`) and
// /// the CHAT endpoint forwards any string verbatim (`streamfn.rs:100`ff: "Chat
// /// forwards any string verbatim under `reasoning_effort`"). Only the Responses
// /// endpoint checks these seven client-side. So this const is the PANEL's
// /// vocabulary, not a constraint on the wire, and an off-list value is preserved
// /// verbatim (§4.4) — never silently rewritten.
// ///
// /// `EFFORT_LEVELS[0]` (`"none"`) is the free-style spelling of the cycle's
// /// `(none)` stop, which is `None` on the wire (`/effort`'s
// /// `"-" | "none" | "off" => None`, `app.rs:4255`). It is therefore NOT a
// /// separate stop: the wrap is `(none) → minimal → … → max → (none)` (§7.1).
// pub const EFFORT_LEVELS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the read-only notes (step 2) ====
// /// Everything the panel refuses to claim about provenance (§4.8): a fixed
// /// `ReadOnly` row's dim detail. The `tool output` row's whole point — there is
// /// no global flag, `toggle_all_tools` is a bulk write over the blocks
// /// (`app.rs:2803` `fn toggle_all_tools(&mut self)`).
// const TOOL_OUTPUT_NOTE: &str = "Ctrl-T expands / collapses all";
//
// /// The `ReadOnly` rows' one-line hint when `Enter` is pressed (§4.2: "`ReadOnly`
// /// sets a one-line hint") — and the placeholder a read-only row shows when it has
// /// no note of its own.
// const READONLY_HINT: &str = "read-only — `--dump-config`";
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — `Toggle` / `Cycle` (steps 2, 4) ====
// /// What `Enter` / `←` / `→` mean for a `Toggle` row. One variant per row that
// /// uses `Toggle`, because the brief forbids matching on row ids (§4.3: "the only
// /// match is over `SettingKind` — never over row ids") and the two toggles push
// /// different effects.
// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
// pub enum Toggle {
//     /// `plan`: flips `Status.plan` optimistically and asks `SetPlanMode`.
//     Plan,
//     /// `sidebar`: flips the client-local view bit. Instant, never pending.
//     Sidebar,
// }
//
// /// What `←` / `→` / `Enter` cycle for a `Cycle` row. Same rationale as
// /// [`Toggle`].
// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
// pub enum Cycle {
//     /// `effort`: the [`EFFORT_LEVELS`] cycle with the off-list rule.
//     Effort,
//     /// `view`: `focus` ⇄ `all` (`ViewMode`, `app.rs:2388`).
//     View,
// }
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the row taxonomy (step 2) ====
// /// How a row is drawn and which keys act on it — the closed taxonomy of §4.3.
// ///
// /// # Contracts
// /// - Five variants, no more: a new kind is a new key behaviour, and `on_key`'s
// ///   match is exhaustive over this enum, so the compiler asks for it.
// /// - A variant carries the row's *shape*; the row's *text* is [`Setting::value`]
// ///   and [`Setting::label`], pre-formatted by [`rows`] — the renderer formats
// ///   nothing.
// /// - `Nudge`'s effect is `SetWidth` and `width` is the ONLY `Nudge` row. A
// ///   second one must add a target discriminant (the `Toggle`/`Cycle` pattern);
// ///   `the_only_nudge_row_is_width` is the guard.
// #[derive(Clone, Debug, PartialEq)]
// pub enum SettingKind {
//     /// `Enter` flips; `←`/`→` are no-ops (§4.2).
//     Toggle { on: bool, what: Toggle },
//     /// `←`/`→`/`Enter` advance (back/forward/forward); wraps; preserves an
//     /// off-list value until the first press (§4.4).
//     Cycle {
//         options: &'static [&'static str],
//         what: Cycle,
//     },
//     /// `←`/`→` step by `step`, clamped to `min..=max`; `Enter` is a no-op.
//     Nudge {
//         min: usize,
//         max: usize,
//         step: usize,
//     },
//     /// `Enter` opens the existing picker; `←`/`→` are no-ops.
//     Pick { kind: PickerKind },
//     /// Nothing acts. `Enter` (or any key) sets [`SettingsState::hint`] — a
//     /// deliberate, visible "there is nothing to edit here" (§4.2, §4.8).
//     ReadOnly { note: Option<&'static str> },
// }
//
// /// One row of the panel — one DRAWN line, exactly like [`crate::app::InfoRow`],
// /// which keeps the row/cursor arithmetic line-agnostic (`line_of`).
// #[derive(Clone, Debug, PartialEq)]
// pub struct Setting {
//     /// Stable identity (see [`ids`]). Never rendered.
//     pub id: &'static str,
//     /// The left column, already worded (`"tool output"`, `"provider: "`-less).
//     /// `Cow` so a fixed row borrows its literal and a `config` row owns an
//     /// injected line.
//     pub label: Cow<'static, str>,
//     pub kind: SettingKind,
//     /// The right column, pre-formatted by [`rows`]: `"high"`, `"68 cols"`,
//     /// `"shown"`, `"128000"`, a config line's value tail. Empty = no value.
//     pub value: String,
//     /// A change the harness ACCEPTED but has not applied yet (§4.5): only the
//     /// three Request-backed rows (`model`, `effort`, `plan`) can be `true`.
//     /// The `· pending` MARK's wording and style are `tui-design.md` §4's.
//     pub pending: bool,
// }
//
// Section's row list — the field the real struct above leaves out:
// /// The panel's own mutable state — the ONE thing `App` holds beside `overlay`.
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the snapshot value fields (steps 2, 6, 7) ====
// Inserted in the real `SettingsSnapshot` above `config_heading`, in this order:
//     pub model: &'a str,
//     pub effort: Option<&'a str>,
//     pub plan: bool,
//     pub theme: Option<&'a str>,
//     pub width: usize,
//     pub view: ViewMode,
//     pub sidebar: bool,
//     pub context: Option<u64>,
//     /// The finished `--dump-config` text, injected by the CLI
//     /// (`Options.config`). Split into `ReadOnly` rows by [`rows`].
//     pub config: &'a str,
//     /// Row ids whose change the harness ACCEPTED and has not applied yet — App's
//     /// `deferred_settings` (§4.5). Empty when nothing is in flight; never longer
//     /// than three, because only `model`/`effort`/`plan` go through the harness.
//     pub pending: &'a [&'static str],
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — `KeyOutcome::Act` (step 2) ====
// The fourth variant of the real `KeyOutcome` above, after `Close`:
//     /// Act on the row under the cursor. The caller executes it in
//     /// `App::apply_setting`, a flat match over [`SettingAction`].
//     Act(SettingAction),
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the effect vocabulary (steps 2–4) ====
// /// The effect vocabulary `App` executes — one variant per existing setter, so
// /// `App::apply_setting` is a flat match and this module names no `App` type and
// /// no `Request`.
// ///
// /// # Contracts
// /// - Every variant maps to ONE setter that already exists (§4.3's table); the
// ///   panel adds no behaviour, only a face.
// /// - `SetEffort`/`SetPlan` are the DEFERRABLE ones: the caller writes `Status`
// ///   optimistically and, when `App::running()`, marks the row pending (§4.5).
// ///   The other four settle instantly and never mark pending (§4.8).
// /// - `SetWidth`'s value is already clamped to the row's `min..=max`; the caller's
// ///   `set_measure` clamps again (harmless, `app.rs:2705`).
// #[derive(Clone, Debug, PartialEq)]
// pub enum SettingAction {
//     /// `width`: `App::set_measure(cols)` (`app.rs:2704`).
//     SetWidth(usize),
//     /// `view`: `App::set_view_mode(view)` (`app.rs:5460`, no-op when unchanged).
//     SetView(ViewMode),
//     /// `sidebar`: `App::toggle_sidebar()` (`app.rs:5442`).
//     ToggleSidebar,
//     /// `model` / `theme`: open the EXISTING picker (`app.rs:5114`, `:5107`); the
//     /// accept arm that already exists applies it (`:5217`–`:5221`, `:5241`–`:5245`),
//     /// and the panel is still open underneath — the host decision's payoff (§4.1).
//     ///
//     /// ONE VARIANT PER PICKER (first-layer A3), not `OpenPicker(PickerKind)`: the
//     /// four other `PickerKind`s (`Change`/`Resume`/`Team`/`Surface`) are not
//     /// settings rows, so a payload would force `apply_setting` to grow a
//     /// wildcard arm for impossible states — the silent-match class this item
//     /// exists to avoid. Two variants keep the caller's match flat AND total.
//     OpenModelPicker,
//     OpenThemePicker,
//     /// `effort`: `Status.effort` optimistically + `Request::SetEffort` (the
//     /// `/effort` arm, `app.rs:4252`–`:4261`, which today writes no `Status`).
//     SetEffort(Option<String>),
//     /// `plan`: the `/plan` arm's body — `plan_pending`, `Status.plan`, and
//     /// `Request::SetPlanMode` (`app.rs:4289`–`:4304`).
//     SetPlan(bool),
// }
// ==== /SKETCH ====

// ==== SKETCH (review-only, not real code) — the effort cycle (step 4) ====
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

#[cfg(test)]
mod tests {
    //! Step 1's pure half — no terminal, no `App`. The row-level tests of §7.1
    //! (`nudge_clamps_at_the_measure_bounds`, `cycle_wraps_…`, `tab_jumps_…`, …)
    //! arrive with the taxonomy in step 2; they are kept, verbatim and still
    //! commented, at the foot of this file. The integration tests
    //! (`esc_closes_the_settings_panel_and_nothing_else_does`, …) live in
    //! `app.rs`'s `mod tests`, where `App::handle` is reachable.

    use super::*;

    /// A local-client snapshot: two headings, no rows yet.
    fn snap() -> SettingsSnapshot {
        SettingsSnapshot {
            config_heading: CONFIG_HEADING_LOCAL,
        }
    }

    #[test]
    fn rows_emits_the_two_section_headings() {
        let sections = rows(&snap());
        assert_eq!(sections.len(), 2, "session + config, always both");
        assert_eq!(sections[0].heading, "session");
        assert_eq!(sections[1].heading, "config");
        // A `--socket` client labels the section with whose config it shows.
        let client = rows(&SettingsSnapshot {
            config_heading: CONFIG_HEADING_CLIENT,
        });
        assert_eq!(client[0].heading, "session", "the first section is labelled too");
        assert_eq!(client[1].heading, "config (this client)");
    }

    #[test]
    fn line_geometry_counts_each_heading_and_the_one_separator() {
        let sections = rows(&snap());
        // Two headings + the blank separator between them, and no rows yet.
        assert_eq!(line_of(&sections, 0), (0, 3));
        // A cursor past the end is clamped to the last line, never a panic.
        assert_eq!(line_of(&sections, 99), (2, 3));
        // An empty panel has no lines at all.
        assert_eq!(line_of(&[], 0), (0, 0));
    }

    #[test]
    fn esc_closes_and_movement_clamps_on_an_empty_row_list() {
        let sections = rows(&snap());
        let mut state = SettingsState::new();
        // `Esc` is the only close key: nothing else in the grammar closes.
        for key in [Key::Enter, Key::Char('q'), Key::Char('x'), Key::Tab] {
            assert_eq!(on_key(&mut state, key, &sections), KeyOutcome::None, "{key:?}");
        }
        // Movement keys are ANSWERED (not swallowed) and clamp on zero rows.
        for key in [Key::Up, Key::Down, Key::PageUp, Key::ScrollDown] {
            assert_eq!(on_key(&mut state, key, &sections), KeyOutcome::Moved, "{key:?}");
            assert_eq!(state.cursor(), 0, "an empty row list has one position");
        }
        state.set_page(10); // a published page must not un-clamp the empty list
        assert_eq!(on_key(&mut state, Key::PageDown, &sections), KeyOutcome::Moved);
        assert_eq!(state.cursor(), 0);
        assert_eq!(on_key(&mut state, Key::Esc, &sections), KeyOutcome::Close);
    }
}

// ==== SKETCH (review-only, not real code) — the pure §7.1 tests (steps 2–4) ====
// Helpers + one test per row-level rule; they need the rows and the effect
// vocabulary above, so they stay commented until those register.
//     //! The pure half of §7.1 — no terminal, no `App`. The integration tests
//     //! (`esc_closes_the_settings_panel_and_nothing_else_does`, …) live in
//     //! `app.rs`'s `mod tests`, where `App::handle` is reachable.
//
//     use super::*;
//
//     /// A dumped-config sample: the shape `config_dump` produces (`main.rs:504`
//     /// `fn config_dump(cfg: &Config) -> String {`), including the trailing `\n`
//     /// and one interior blank row so the splitting rule is exercised.
//     const DUMP: &str = "provider: openai-compatible\n\
//                         endpoint: chat (source: auto)\n\
//                         base_url: http://localhost:11434/v1 (source: flag)\n\
//                         model: qwen2.5-coder:7b (source: config)\n\
//                         api_key: (none) (source: default)\n\
//                         \n\
//                         compaction budget: 60000\n";
//
//     /// A snapshot with every field at a known value, so a test states only the
//     /// field it is about: no pending marks, no off-list effort.
//     fn snap() -> SettingsSnapshot<'static> {
//         SettingsSnapshot {
//             model: "qwen2.5-coder:7b",
//             effort: None,
//             plan: false,
//             theme: Some("nord"),
//             width: 68,
//             view: ViewMode::Focus,
//             sidebar: true,
//             context: Some(128_000),
//             config: DUMP,
//             pending: &[],
//             config_heading: CONFIG_HEADING_LOCAL,
//         }
//     }
//
//     /// One press of `key` against `snap()`'s rows — the grammar tests' entry.
//     fn press(state: &mut SettingsState, snapshot: &SettingsSnapshot<'_>, key: Key) -> KeyOutcome {
//         on_key(state, key, &rows(snapshot))
//     }
//
//     /// The row `id` appears in, across both sections.
//     fn row<'a>(sections: &'a [Section], id: &str) -> &'a Setting {
//         sections
//             .iter()
//             .flat_map(|s| &s.rows)
//             .find(|r| r.id == id)
//             .expect("row id present")
//     }
//
//     // ---- §7.1, pure --------------------------------------------------------
//
//     #[test]
//     fn the_row_list_names_every_session_knob_once() {
//         todo!()
//         // model/effort/plan/theme/width/view/sidebar each appear exactly once
//         // (by `ids` const, not by label); `tool output` and `context` are
//         // `ReadOnly`. The drift guard, and the cheap half of v1's candidate C-f.
//     }
//
//     #[test]
//     fn nudge_clamps_at_the_measure_bounds() {
//         todo!()
//         // at 40, `←` stays 40; at 200, `→` stays 200; 68 + → = 76 (± MEASURE_STEP).
//     }
//
//     #[test]
//     fn cycle_wraps_and_preserves_an_off_list_value() {
//         todo!()
//         // `None` → minimal → … → max → `None`; `Some("medium-high")` renders
//         // verbatim and ONE arrow press lands on `(none)` — either direction.
//     }
//
//     #[test]
//     fn read_only_rows_yield_a_hint_not_an_action() {
//         todo!()
//         // FIRST-LAYER B7 — the split is the point, and the first cut got it wrong:
//         //  - `Enter` on `tool output` / `context` / a config row → `Moved`, never
//         //    `Act`, with `state.hint()` set (§4.2 "`ReadOnly` sets a one-line hint");
//         //  - `←`/`→` on those rows → `None`, hint unchanged ("no-ops", §4.2);
//         //  - `↑`/`↓` on those rows → MOVE the cursor and CLEAR the hint, exactly as
//         //    on an editable row — otherwise every row of the `config` section is a
//         //    trap: `Tab` can enter that section but `↑`/`↓` could never leave it.
//         // Hint selection: `ReadOnly { note: Some(n) }` shows `n` (so `tool output`
//         // says `Ctrl-T expands / collapses all`), and `note: None` falls back to
//         // `READONLY_HINT` — `--dump-config`'s text is the right hint for a config
//         // row and the WRONG one for `tool output`, which it never prints.
//     }
//
//     #[test]
//     fn tab_jumps_between_the_two_sections() {
//         todo!()
//         // From ANY session row, `Tab` lands on the first `config` row;
//         // `BackTab` returns to the first `session` row (`ids::MODEL`).
//     }
//
//     // ---- the shape's own guards (not in §7.1; each pins a trap) ------------
//
//     #[test]
//     fn the_only_nudge_row_is_width() {
//         todo!()
//         // `Nudge` -> `SetWidth` is hardcoded in `on_key`; a second nudge row
//         // would silently drive the measure. Count the `Nudge` rows == 1.
//     }
//
//     #[test]
//     fn every_config_row_shares_one_id_and_no_session_id_repeats() {
//         todo!()
//         // Editable ids are unique; every config row carries `ids::CONFIG`;
//         // no config row is ever `pending`.
//     }
//
//     #[test]
//     fn an_empty_config_renders_a_placeholder_not_a_blank_section() {
//         todo!()
//         // `""` and a whitespace-only `config` both yield exactly one `ReadOnly`
//         // row, so `Tab` always has a target and the heading is never the last
//         // line. Also: the second section always EXISTS (two `Section`s, not one).
//     }
//
//     #[test]
//     fn the_config_section_has_one_row_per_dumped_line() {
//         todo!()
//         // Interior blank lines are kept, the trailing `\n` is dropped, and the
//         // text appears verbatim (no re-wrapping, no key/value re-splitting of
//         // the row COUNT).
//     }
//
//     #[test]
//     fn wheel_and_page_move_the_cursor_three_and_a_page() {
//         todo!()
//         // `ScrollDown` x N advances THREE rows; `PageDown` advances
//         // `state.page`; both clamp at the last row.
//     }
//
//     #[test]
//     fn enter_acts_per_kind_and_arrows_are_no_ops_on_toggle_and_pick() {
//         todo!()
//         // `Enter` on the plan row -> `Act(SetPlan(true))`; on the model row ->
//         // `Act(OpenPicker(PickerKind::Model))`; `←` on a `Toggle` -> `None`.
//     }
//
//     #[test]
//     fn only_the_three_request_rows_can_be_pending() {
//         todo!()
//         // With `snap().pending = &[ids::MODEL, ids::EFFORT, ids::PLAN]`, exactly
//         // those three rows are `pending` and every other row is `false` —
//         // including all `config` rows and `tool output`/`context` (§4.8).
//         //
//         // The CLOSE-survival half of §4.5 (`Esc` clears no mark) is App-level —
//         // `App::deferred_settings` is not inside the `Option` that close drops —
//         // and is pinned by `a_mid_run_setting_shows_pending_until_the_reply` in
//         // `app.rs`'s tests, where `App::handle` is reachable.
//     }
//
//     #[test]
//     fn a_stale_cursor_clamps_instead_of_panicking() {
//         todo!()
//         // `cursor` far past the end after a rebuild that shortened the list.
//     }
//
//     #[test]
//     fn the_theme_row_names_the_builtin_palette_when_there_is_no_catalog_name() {
//         todo!()
//         // `theme: None` (palette B / an inline `[theme]` palette) renders
//         // `palette b` — NOT `unknown` (that word belongs to the `context` row) and
//         // NOT `default` (A9: `default` is a catalogue entry that resolves to the
//         // SAME colours, `theme.rs:624`-`:625`, so borrowing it would claim a
//         // source the client never applied). `Some("nord")` renders `nord`.
//     }
//
//     #[test]
//     fn line_geometry_counts_one_line_per_heading() {
//         todo!()
//         // `line_of` = (heading offset + cursor index, headings + rows); the
//         // renderer's derived `start` never lands on the heading as "row 0".
//     }
// }
// ==== /SKETCH ====
