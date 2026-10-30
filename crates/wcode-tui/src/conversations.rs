//! **SKETCH — review-only, not real code. NOT a module of this crate yet.**
//!
//! Do NOT add `mod conversations;` to `lib.rs` until this is implemented. Inert
//! and possibly non-compiling by design (`.wcode/skills/interface-sketch/SKILL.md`
//! §0 Form A).
//!
//! # What this module is
//!
//! The TUI's **two-level container** (W040, locked design:
//! `docs/work/W040-multi-root-conversations.md` §13.2 items 5, 6, 8). Today the
//! app is
//! one flat list:
//!
//! ```text
//!   App.surfaces: Vec<Surface>   // crates/wcode-tui/src/app.rs:2926
//!   App.focus: usize             // :2928
//! ```
//!
//! with `surfaces[0]` the root and `surfaces[1..]` its team. W040 adds one level:
//!
//! ```text
//!   conversation 0 ── root ── member ── member
//!   conversation 1 ── root ── member
//!   conversation 2 ── root
//! ```
//!
//! # The N = 1 DELEGATION CONTRACT (the whole point — read this before touching `App`)
//!
//! **With exactly one conversation, every accessor in this module returns exactly
//! what the pre-W040 field access returned.** Concretely:
//!
//! 1. `Conversations::active_index() == 0` and `active()` is `items[0]`, always.
//! 2. `active().surface_ids()` == `[root] ++ members` in **creation order** ==
//!    exactly today's `Vec<Surface>` contents in exactly today's order. So
//!    `App::canonical_order()` (`app.rs:3269`), narrowed to the active
//!    conversation, yields the SAME `Vec<usize>` it yields today.
//! 3. `Conversations` is NEVER empty: `close()` on the last conversation is
//!    `Refused`, so `active()` cannot panic (the `App::surfaces` non-empty
//!    invariant, `app.rs:2925` "Always non-empty (see [`App::new`])", is
//!    preserved at one level up).
//! 4. `Conversation.label` is set but NOT RENDERED while `len() == 1` — the tab
//!    band has height 0 (`ui::draw`), so a single-conversation frame is
//!    byte-identical to today's. That is the test
//!    `n1_frames_are_byte_identical_to_before`, and it is a precondition of the
//!    whole item.
//!
//! The **surfaces themselves stay in `App.surfaces`** (a flat `Vec<Surface>`) and
//! `App.focus` stays an index into it. That is not laziness: `self.focus` appears
//! 129 times and `self.surfaces` 63 times in `app.rs` alone, plus the pixel tests;
//! a nested `Vec<Vec<Surface>>` would rewrite all of them for no behavioural gain.
//! `Conversation` therefore holds **ids**, and `App` holds the **bodies**. The
//! invariant both halves must uphold:
//!
//! > every `Surface.id` appears in exactly one `Conversation`, and
//! > `Conversations::items` is in creation order.
//!
//! # Who calls this
//!
//! - `App` (`app.rs`) — owns one `Conversations`, plus the derived accessors
//!   `conversations()` / `active_conversation()` / `active_surface_indices()`.
//! - `lib.rs`'s event loop — services `ConversationEvent`s from the CLI and emits
//!   `ConversationRequest`s to it.
//! - `ui.rs`'s `draw_tab_bar` — reads `iter()` + `active_index()` for the bar.
//!
//! # THE SHAPE DECISION THAT SAVES ~40 EDITS — and the ONE data model
//!
//! **AUTHORITATIVE: this paragraph, not the `app.rs` sketch block, decides the
//! shape.** There is **no `conversation` field on `SurfaceInfo` and none on
//! `SurfaceSpec`** (first-layer finding 2: the two files had drifted into two
//! mutually exclusive models for one seam). The container is **handed over fully
//! built**, not derived from the surfaces:
//!
//! - **Startup (N ≥ 1):** `Options` gains `conversations: Vec<Conversation>`
//!   (built by the CLI, which alone knows which root a surface belongs to) and
//!   `App::set_surfaces` is UNCHANGED. Adopted by
//!   [`Conversations::from_parts`].
//! - **A runtime spawn:** the member feed carries the tag **beside** the spec, as
//!   `(ConversationId, SurfaceSpec)` — a pair the CLI already knows, because
//!   `set_spawn_sink` (`crates/wcode-cli/src/agents.rs:247`) is installed PER
//!   ORCHESTRATOR and an orchestrator is PER CONVERSATION. Consumed by
//!   [`Conversations::add_surface_in`] / `App::add_surface_in`.
//!
//! Why it matters concretely: `SurfaceInfo {` has **40 literals** across
//! `app.rs`/`ui.rs`/`lib.rs`/`examples/dump.rs` (25 + 8 + 5 + 4, counted in this
//! session), nearly all of them in tests. A `conversation` field would edit all
//! 40 for no behavioural gain; `Options { conversations }` edits **4**
//! (`main.rs` ×2, `lib.rs` test ×1, `examples/demo.rs` ×1).
//!
//! # Integration points (re-verified this session)
//!
//! - `crates/wcode-tui/src/app.rs:2923` `pub struct App {` / `:2926`
//!   `    surfaces: Vec<Surface>,` / `:2928` `    focus: usize,`.
//! - `crates/wcode-tui/src/app.rs:3256` `pub fn focused_id(&self) -> &SessionId {`
//!   — the id the event loop's action dispatch resolves a `Backend` by.
//! - `crates/wcode-tui/src/lib.rs:284` `pub struct Options {` — gains the
//!   startup conversation list (the injection seam this crate already uses for
//!   `models`/`sessions`/`teams`/`skills`/`tasks`).
//! - `crates/wcode-tui/src/lib.rs:378` `    new_surfaces: Option<mpsc::UnboundedReceiver<SurfaceSpec>>,`
//!   — the INBOUND member feed this module's `ConversationEvent` sits beside; its
//!   element type gains the conversation tag.

use wcode_harness::protocol::SessionId;
use wcode_protocol::{ConversationId, RootSpec};

use crate::SurfaceSpec;

// ==========================================================================
// One conversation
// ==========================================================================

/// One conversation as the TUI knows it: **ids only** (the `Surface` bodies live
/// in `App.surfaces`, see the module doc).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    /// Host-minted identity (`wcode_protocol::ConversationId`). Stable; the tab
    /// bar's click target, the `/tab <n>`-free address, and W041's URL segment.
    pub id: ConversationId,
    /// The tab's label: the host's seed (cwd basename) until the first
    /// `AgentEvent::SessionTitle` lands, then that title
    /// (`crates/wcode-tui/src/app.rs:2322`
    /// `AgentEvent::SessionTitle { title } => {`).
    ///
    /// NOT read from the `Action` reply: `SessionTitle` is a *streamed* event both
    /// a local and a socket client already receive (the `Todo` rationale at
    /// `app.rs:2324`), so a second channel for the same fact would be a second
    /// source of truth that can disagree ("the tab label's single source").
    pub label: String,
    /// The ROOT surface's routing id — the id every `AppEvent::Agent` for this
    /// conversation's root is tagged with.
    pub root: SessionId,
    /// The non-root surfaces' routing ids, in creation order. Never contains
    /// `root`.
    pub members: Vec<SessionId>,
    /// The tab's state. Written by exactly one function,
    /// [`Conversations::set_state_from`] (the sticky `Failed` rule), plus
    /// [`Conversations::mark_died`] (which only ever sets `Failed`). Read-only
    /// everywhere else — and, being the ROOT's state only, NOT an answer to "is
    /// anything running in this conversation" (see [`ConversationState::Idle`]).
    pub state: ConversationState,
    /// `true` when this conversation's backends are `Backend::Remote` (a
    /// `--socket` client). One remote conversation per process today — a socket
    /// client is served ONE root
    /// (`crates/wcode-cli/src/main.rs:1155` `let new_surfaces = {`), so this is
    /// never rendered while it is true (the tab band needs `len() > 1`). It exists
    /// so the tab bar has a place to say `remote` the day W041 gives a client two.
    pub is_remote: bool,
}

/// The tab's state. **Deliberately three**, not `TeamState`'s four: a conversation
/// that finished its last turn is `Idle` — a `✓ done` tab would be permanent
/// noise on a bar that is always visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationState {
    /// Nothing running (born here). `TeamState::Idle` and `TeamState::Done` both map
    /// here.
    ///
    /// **`Idle` does NOT mean "nothing in this conversation is running."** A
    /// running MEMBER leaves the tab `Idle` by design (see
    /// [`Conversation::state_of`]). Any caller that needs "is anything live here"
    /// — above all the `/tab close` predicate — must ask the surfaces, not this
    /// value. First-layer finding 10.
    Idle,
    /// The conversation's ROOT is running.
    Running,
    /// Sticky until the conversation next RUNS: either the last run failed
    /// (`TeamState::Failed`) or **the root's actor task died**
    /// (§13.2 item 4 — `ConversationEvent::Died`).
    ///
    /// "Until it next runs" is the only reset that exists: the reducer has no
    /// `AgentStart` input to hand here (first-layer finding 10). The clearing
    /// transition is a [`crate::TeamState::Running`] observation in
    /// [`Conversations::set_state_from`] — see its truth table. Nothing else
    /// clears it, including `AgentEnd` (which reports `Done` → `Idle`) and
    /// [`Conversations::mark_died`], which only ever SETS it.
    Failed,
}

impl Conversation {
    /// Every surface id, root first, then `members` in creation order — the order
    /// `App::canonical_order` (D34) requires, and at N = 1 exactly today's list.
    pub fn surface_ids(&self) -> Vec<SessionId> {
        todo!()
    }

    /// Whether `id` is this conversation's root or one of its members.
    pub fn contains(&self, id: &SessionId) -> bool {
        todo!()
    }

    /// A surface's state as the tab shows it, from the flat store.
    ///
    /// # Contract
    /// A member's state NEVER promotes the tab: a running worker in an idle
    /// conversation shows `Idle`. The tab names the CONVERSATION, and the
    /// conversation's liveness is its root's — the team region
    /// (`app.rs:3865` `pub fn working_team_rows(&self)`) already renders members.
    ///
    /// The mapping is total and lossy on purpose:
    /// `TeamState::Idle | TeamState::Done → Idle`, `Running → Running`,
    /// `Failed → Failed`. `Done` folds into `Idle` because a permanent `✓` on an
    /// always-visible bar is noise (see [`ConversationState`]).
    ///
    /// This is the PURE mapping only — it cannot express stickiness. The sticky
    /// rule lives in [`Conversations::set_state_from`], which is the only writer
    /// of [`Conversation::state`]. **And `Idle` here is not an answer to "is
    /// anything running in this conversation"**: the close predicate must walk
    /// every surface instead (first-layer finding 10).
    pub fn state_of(root_state: crate::TeamState) -> ConversationState {
        todo!()
    }
}

// ==========================================================================
// The container
// ==========================================================================

/// The tab container: conversations in creation order + the active index.
///
/// See the module doc for the **N = 1 delegation contract**.
///
/// **No `Default`** (first-layer nit 1): `#[derive(Default)]` would build
/// `items: []`, the exact state this file calls a constructor bug — `active()`
/// would index an empty `Vec`, and [`Conversations::len`]`() == 0` would make the
/// tab band's `len() > 1` test disagree with every other accessor. The only
/// constructors are [`Conversations::single`] and [`Conversations::from_parts`].
#[derive(Clone, Debug)]
pub struct Conversations {
    items: Vec<Conversation>,
    /// Index into `items`. Clamped by every mutator; `items` is never empty, so
    /// this is always valid (contract 3).
    active: usize,
}

/// What a requested close did. Returned to `App`, which owns the flat surface
/// store and must remove the bodies this outcome names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The conversation was removed. `surfaces` = every id that must be dropped
    /// from `App.surfaces` (root + members), `active` = the index the container
    /// moved focus to.
    Closed {
        id: ConversationId,
        surfaces: Vec<SessionId>,
        active: usize,
    },
    /// Nothing was removed, and here is why (a `transient`-ready string).
    Refused { reason: &'static str },
}

impl Conversations {
    /// **The N = 1 conversation, from a surface list** — the fallback AND the
    /// startup seed. `surfaces[0]` is the root, `surfaces[1..]` are its members.
    ///
    /// # Why this takes the whole slice (first-layer finding 4)
    /// The earlier `new(root, label)` took no members, so the N = 1 fallback
    /// produced `surface_ids() == [root]` for a TEAM run — and then
    /// `App::canonical_order` (`app.rs:3269`) narrowed to a single index, which
    /// breaks `backtab_moves_focus_backwards_and_wraps` (`app.rs:9995`-region:
    /// `set_surfaces(&mut app, &["root", "a", "b"])` then
    /// `assert_eq!(app.focus(), 2, "Shift-Tab wraps to the last surface")`). The
    /// fallback must therefore group **every** surface, and it is the same
    /// function `App::new` seeds with.
    ///
    /// # Contract
    /// - `surfaces.is_empty()` ⇒ the default-root conversation
    ///   (`SessionId::agent("root")`, label `"wcode"`) — exactly what
    ///   `default_root` gives `App::new` (`app.rs:2737` `fn default_root() -> Surface {`), so the container
    ///   is valid before `set_surfaces` runs.
    /// - Id `ConversationId::first()`, state `Idle`, `is_remote = false` (the
    ///   caller overrides it: `Options.remote`).
    /// - `surface_ids()` equals the input order, always. That equality IS the
    ///   N = 1 delegation contract's clause 2.
    pub fn single(surfaces: &[crate::SurfaceInfo]) -> Self {
        todo!()
    }

    /// Adopt the CLI's startup list (`Options.conversations`) after
    /// `App::set_surfaces` has installed the bodies.
    ///
    /// # Contract
    /// - **Validated, not trusted**: every `SurfaceInfo.id` must appear in exactly
    ///   one conversation's `root`/`members`, and every listed id must exist in
    ///   `surfaces`. A mismatch is a CLI bug, and the recovery is to FALL BACK to
    ///   [`Conversations::single`] over the SAME slice (which groups every
    ///   surface into one conversation — finding 4) plus a `debug_assert!` — never
    ///   a panic, never a silently dropped surface (`App::focused` indexes by
    ///   `focus`, and an orphaned surface would be unreachable from every narrowed
    ///   accessor).
    /// - **Empty input** (a test, `examples/dump.rs`, a socket client that built
    ///   none) ⇒ `Conversations::single(surfaces)`. That is what keeps the 40
    ///   existing `SurfaceInfo {` literals and every pixel test compiling and
    ///   passing untouched — including the team case, because `single` takes the
    ///   members too.
    /// - Preserves the input order: `items[0]` is the startup conversation
    ///   (`ConversationId::first()`), so `active == 0` at startup.
    pub fn from_parts(conversations: Vec<Conversation>, surfaces: &[crate::SurfaceInfo]) -> Self {
        todo!()
    }

    /// The active conversation. Panics never (contract 3) — a container with no
    /// items is a constructor bug, and every mutator refuses that state.
    pub fn active(&self) -> &Conversation {
        todo!()
    }

    /// The active index (the tab bar's `▌` position; `App`'s reconcile input).
    pub fn active_index(&self) -> usize {
        todo!()
    }

    /// Every conversation, in creation order — the tab bar's draw order and
    /// `/tab`'s numbering (1-based, exactly like the sidebar's badges).
    pub fn iter(&self) -> impl Iterator<Item = &Conversation> {
        todo!()
    }

    /// How many conversations. `1` means the tab band has height 0 (contract 4).
    pub fn len(&self) -> usize {
        todo!()
    }

    /// `len() == 0`. Present for the `clippy::len_without_is_empty` gate; it is
    /// never `true`.
    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// The conversation owning `surface`, if any — `App`'s `SessionTitle` and
    /// `add_surface` both need it.
    pub fn conversation_of(&self, surface: &SessionId) -> Option<&Conversation> {
        todo!()
    }

    /// The conversation with this id, if any (`ConversationEvent::Died`, `/tab`).
    pub fn get(&self, id: ConversationId) -> Option<&Conversation> {
        todo!()
    }

    /// Index of `id`, if any.
    pub fn index_of(&self, id: ConversationId) -> Option<usize> {
        todo!()
    }

    /// Activate by index. `false` (and no change) when out of range — the
    /// `App::set_focus` tolerance (`app.rs:3292`).
    pub fn activate(&mut self, idx: usize) -> bool {
        todo!()
    }

    /// Activate by id. `false` when unknown.
    pub fn activate_id(&mut self, id: ConversationId) -> bool {
        todo!()
    }

    /// Advance the active index by `step`, wrapping — `Ctrl-N` / `Shift-Tab` at
    /// N > 1 ([`crate::app::App::cycle_or_focus`], `app.rs:1110`-region).
    ///
    /// # Contract
    /// A no-op with `false` when `len() <= 1`: at N = 1 the SAME keys keep their
    /// today meaning (cycle surfaces), and this must not steal them. See
    /// the keys recommendation in the W040 report.
    pub fn cycle(&mut self, step: isize) -> bool {
        todo!()
    }

    /// Append a conversation delivered by the host. `Err(reason)` on a duplicate
    /// id or a root already owned by another conversation (never a panic).
    /// Does NOT activate it: `/tab new` activates explicitly, so a background
    /// reply cannot steal the user's view (the `add_surface` rule,
    /// `app.rs:3231` "a new teammate must not steal the user's perspective").
    pub fn push(&mut self, c: Conversation) -> Result<(), String> {
        todo!()
    }

    /// Register a runtime-spawned surface into `conversation`. `Err` when the
    /// conversation is unknown (the CLI emitted a member for a closed tab).
    ///
    /// # Why the tag is a PARAMETER and not a field (first-layer finding 2)
    /// `SurfaceSpec::info()` (`crates/wcode-tui/src/lib.rs:268`-region) copies
    /// four fields into a [`crate::SurfaceInfo`] and **drops the conversation
    /// tag** — it is not a `SurfaceSpec` field, by the module header's decision.
    /// The losing line is `app.add_surface(spec.info());` at
    /// `crates/wcode-tui/src/lib.rs:581`, where the inbound `(ConversationId,
    /// SurfaceSpec)` pair is consumed. So the tag must ride BESIDE the spec and
    /// be handed in here explicitly; a caller that has only a `SurfaceInfo` has
    /// no way to say which tab it belongs to, which is exactly why this method
    /// does not exist in any other shape.
    pub fn add_surface_in(
        &mut self,
        conversation: ConversationId,
        info: crate::SurfaceInfo,
    ) -> Result<(), String> {
        todo!()
    }

    /// Remove a conversation (§13.2 items 3–4: close = kill its `bg` scope, drop
    /// its handles, forget it).
    ///
    /// # Contract
    /// - **The last conversation is REFUSED** (`CloseOutcome::Refused`), so
    ///   `items` is never empty.
    /// - Closing the ACTIVE conversation moves `active` to the neighbour at the
    ///   old index, clamped into range (`min(old, len - 1)`) — the "close a tab"
    ///   rule every tabbed UI uses.
    /// - Closing a non-active conversation moves `active` down by one when the
    ///   removed one was before it, so the same conversation stays active.
    /// - Returns the surface ids the caller must remove; it does NOT touch
    ///   `App.surfaces` (this module cannot; the bodies live there).
    pub fn close(&mut self, id: ConversationId) -> CloseOutcome {
        todo!()
    }

    /// Record the host's verdict that a conversation's root runtime is gone
    /// (§13.2 item 4). Sticky `Failed` — the tab must not lie that it is idle.
    /// `false` when the id is unknown (a `Died` racing a close is not an error).
    pub fn mark_died(&mut self, id: ConversationId) -> bool {
        todo!()
    }

    /// Set the state of the conversation owning `surface` from that surface's
    /// `TeamState` — the `AppEvent::Agent` arm's one-line hook.
    /// `false` when the surface belongs to no conversation.
    ///
    /// # The truth table (first-layer finding 10 — `Failed` stickiness)
    ///
    /// `Failed` is sticky, and THIS is the only function that can clear it — so
    /// the rule has to be here, not in a doc comment on the enum. For the
    /// conversation's ROOT surface:
    ///
    /// | input `TeamState` | previous tab state | next tab state |
    /// |---|---|---|
    /// | `Running`  | anything (incl. `Failed`) | **`Running`** — the reset |
    /// | `Failed`   | anything | `Failed` |
    /// | `Idle` / `Done` | `Failed` | **stays `Failed`** |
    /// | `Idle` / `Done` | `Running` / `Idle` | `Idle` |
    ///
    /// A **member** surface is a no-op for the state (it still returns `true`:
    /// the surface was found — `set_label`'s convention), because the tab never
    /// promotes a member.
    ///
    /// # Why not "until the root's next `AgentStart`"
    /// There is no `AgentStart` input on this side of the seam: `Surface::apply`
    /// consumes the event (`app.rs:2164` `fn apply(&mut self, event: AgentEvent)
    /// -> bool {`) and all the `App` arm can read afterwards is `Surface::state()`.
    /// `TeamState::Running` is the observable proxy, and it is exact: `state()`'s
    /// ladder is `failed → running → finished → idle` (`app.rs:2150` `fn state(&self) -> TeamState {`-region), and
    /// `failed` is itself cleared only by `AgentStart` — so the first `Running`
    /// after a failure IS the next run's start, which is precisely the reset the
    /// reviewer asked for.
    pub fn set_state_from(&mut self, surface: &SessionId, state: crate::TeamState) -> bool {
        todo!()
    }

    /// Replace a conversation's label — the `AgentEvent::SessionTitle` hook.
    /// `false` when the surface belongs to no conversation.
    ///
    /// Only the ROOT's title renames the tab: a member's `/rename` names the
    /// member, and the sidebar already shows that.
    pub fn set_label(&mut self, surface: &SessionId, label: String) -> bool {
        todo!()
    }
}

// ==========================================================================
// The reverse channel (§13.2 item 8)
// ==========================================================================

/// **The TUI → CLI channel**: what the app asks the host to do. The exact reverse
/// of `new_surfaces`, which is the CLI → TUI channel
/// (`crates/wcode-tui/src/lib.rs:378`).
///
/// # Why the reverse is needed at all
///
/// Every mutator the TUI has ever had is *local*: it can only answer the CLI at
/// EXIT (`pending_resume`/`pending_team`/`pending_reload`/`pending_new`,
/// `app.rs:2951`-region) and let the CLI re-exec — which is exactly why "new
/// conversation" means "new terminal" today. A tab cannot re-exec: the other
/// conversations would die with the process. So the TUI needs a live
/// request/response pair, and the *build* half can only be done by the CLI (the
/// TUI holds no `llm`, no session dir, no `Background`, no `Agent`.
/// `crates/wcode-tui/src/lib.rs:284` `pub struct Options` — the injection list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationRequest {
    /// Build one root. Answered by a [`ConversationEvent::Added`], or by
    /// `ConversationEvent::Failed` carrying the factory's error string.
    Start(RootSpec),
    /// Tear one down: `Background::shutdown_scope` for its `bg` tasks, then
    /// `Registry::remove` + drop the handle
    /// (`crates/wcode-protocol/src/registry.rs:202` `pub fn remove(&self, id: &SessionId) {`).
    ///
    /// Sent AFTER `App` has already removed the tab optimistically? **No** — see
    /// the two halves of a teardown: the TUI removes it on the reply's echo
    /// (`Event::Stopped`),
    /// so a failed teardown cannot leave a tab whose backend is gone.
    Stop(ConversationId),
}

/// **The CLI → TUI channel's payload** — the `new_surfaces` sibling, for
/// conversations rather than members.
#[derive(Clone, Debug)]
pub enum ConversationEvent {
    /// One conversation is live: its id, seed label, and the surfaces to attach
    /// (root first). Exactly the `Vec<SurfaceSpec>` shape `run` already takes at
    /// startup (`crates/wcode-tui/src/lib.rs:381` `surfaces: Vec<SurfaceSpec>,`),
    /// so `App::set_surfaces`-equivalent grouping is reused, not forked.
    Added(ConversationSpec),
    /// The host could not build it (`/tab new` on a full disk, a bad `Resume`).
    /// Rendered as a `transient` — the `/reload`-refusal idiom — and the pending
    /// guard is released.
    Failed { error: String },
    /// The tab was torn down; `App` may now forget it. Sent for both `/tab close`
    /// and the supervisor's `JoinHandle` exit.
    Stopped { id: ConversationId },
    /// The root's actor task ENDED unexpectedly (§13.2 item 4). Distinct from
    /// `Stopped` so the tab renders `Failed` instead of disappearing.
    Died { id: ConversationId },
}

/// One conversation's startup payload — [`ConversationEvent::Added`]'s body.
///
/// **Not** a second `Conversation`: it is what the CLI can build (surfaces with
/// live `Backend`s), and `Conversations::push` derives the view-model from it.
#[derive(Debug)]
pub struct ConversationSpec {
    pub id: ConversationId,
    /// The seed label — the cwd basename, or a `Resume` dir's stem.
    pub label: String,
    /// Root first (`is_root: true`), then members in creation order. The
    /// conversation id is THIS struct's `id`: `SurfaceSpec` carries no tag (see
    /// the module header — `SurfaceSpec::info()` drops it), so the payload's
    /// shape is what associates every listed surface with `id`.
    pub surfaces: Vec<SurfaceSpec>,
}

/// The two directions the CLI hands `wcode_tui::run`, bundled so `run` gains ONE
/// parameter instead of two (`run` already takes `surfaces`, `options`,
/// `new_surfaces`, `new_tasks` — `lib.rs:376`-region).
///
/// # Why `Option` on both
///
/// A `--socket` client has no local build capability (`/reload`, `/new`, `/team`
/// are all refused there — `app.rs:6472`, `:6484`, `:6059`), so it passes
/// `ConversationFeeds::none()` and `/tab new` refuses with the same wording.
/// `None` ⇒ the reducer's gate refuses before any `Action` is emitted.
///
/// **The gate is `App.remote`, not a method here** (first-layer nit 6):
/// `remote: bool` (`app.rs:2963`), set once by `set_remote` (`app.rs:6465`), is
/// already exactly that flag and all three existing refusals read it. This type
/// deliberately has no `is_local()` and there is no
/// `set_conversation_feeds_local` — a second copy of one boolean is a second
/// thing to get wrong, and the empty-channel case is already covered by
/// [`ConversationRequest`]'s "never emitted over a socket" contract.
#[derive(Default)]
pub struct ConversationFeeds {
    /// Replies + lifecycle from the CLI.
    pub incoming: Option<tokio::sync::mpsc::UnboundedReceiver<ConversationEvent>>,
    /// Requests out to the CLI.
    pub outbound: Option<tokio::sync::mpsc::UnboundedSender<ConversationRequest>>,
}

impl ConversationFeeds {
    /// No host behind this TUI (a `--socket` client): both ends `None`.
    pub fn none() -> Self {
        todo!()
    }
}

/// The CLI's `Resume` picker entry for `/tab new --resume`? **Not sketched** —
/// §13.2 item 7 keeps `/tab` to `new | close | <n>`; a resume tab reuses
/// `/resume`'s picker only if the reviewer asks for it.

#[cfg(test)]
mod tests {
    //! Behaviour-named stubs, one per invariant named in the W040 task. Each
    //! `todo!()` becomes a real assertion; none may be deleted.

    use super::*;

    fn conv(id: u64, root: &str, members: &[&str]) -> Conversation {
        todo!()
    }

    #[test]
    fn n1_delegates_every_accessor_to_the_single_conversation() {
        // `Conversations::single(&[])` (the `App::new` seed):
        //  - `len() == 1`, `active_index() == 0`, `is_empty() == false`;
        //  - `active().surface_ids() == vec![agent:root]`, label "wcode".
        // `Conversations::single(&[root])` — one surface, no members: same.
        //  - `cycle(1) == false` and the index is unchanged (contract 3 + the
        //    "N = 1 steals no key" rule);
        //  - `close(first())` is `Refused` (the last tab cannot be closed).
        todo!()
    }

    #[test]
    fn single_over_a_team_run_keeps_every_surface_in_the_conversation() {
        // FINDING 4's gate: `Conversations::single(&["root","a","b"])` gives
        // `active().surface_ids() == [root, a, b]` and `active().members == [a, b]`
        // — so `App::canonical_order` narrowed to the active conversation still
        // yields all three indices, and `backtab_moves_focus_backwards_and_wraps`
        // (`app.rs:9995`) passes UNMODIFIED. A `single` that returned
        // `[root]` would make `assert_eq!(app.focus(), 2)` fail there.
        todo!()
    }

    #[test]
    fn from_parts_groups_by_conversation_and_falls_back_to_single() {
        // Two conversations handed in (`c0: root, member` / `c1: root`):
        // `len() == 2`, `items[0].members == [member]`, `items[1].members == []`;
        // every `SurfaceInfo.id` covered exactly once.
        // An EMPTY list falls back to `single(&infos)` — which groups EVERY
        // surface, so a team run at N = 1 is not narrowed (finding 4).
        // A list that MISSES a surface also falls back to `single` and
        // `debug_assert!`s — no surface is ever dropped.
        todo!()
    }

    #[test]
    fn closing_the_active_conversation_moves_active_to_the_clamped_neighbour() {
        // 3 conversations, active = 1, close 1 → active = 1 (`min(1, 1)`), which
        // is the conversation that was index 2. Close the last → active stays.
        todo!()
    }

    #[test]
    fn closing_a_conversation_before_the_active_one_shifts_active_down() {
        // active = 2, close 0 → active = 1, and `active().id` is the same as
        // before the close.
        todo!()
    }

    #[test]
    fn a_duplicate_id_or_root_is_refused_not_overwritten() {
        // `push` of an existing id, and `push` of a conversation whose root id is
        // already owned → `Err`, `len()` unchanged. The `spawn_worker` D14 rule.
        todo!()
    }

    #[test]
    fn a_member_running_does_not_promote_the_tab_state() {
        // root Idle + member Running → `ConversationState::Idle`;
        // root Running → `Running`; `mark_died` → sticky `Failed`.
        // CONSEQUENCE, asserted in the same test so nobody "fixes" it: this is
        // why `App::request_close_conversation` must ask the SURFACES, not the
        // tab state (finding 10).
        todo!()
    }

    #[test]
    fn failed_is_sticky_until_the_next_run_starts() {
        // FINDING 10(a)'s gate — `set_state_from`, the only writer:
        //   root Failed        → tab Failed
        //   root Done          → tab STAYS Failed      (the old bug cleared here)
        //   root Idle          → tab STAYS Failed
        //   root Running       → tab Running           (the ONE reset)
        //   root Idle          → tab Idle              (from Running, no failure)
        // and a MEMBER's Failed/Running never touches the tab.
        todo!()
    }

    #[test]
    fn only_the_roots_title_renames_the_tab() {
        // `set_label(root, "api")` → `active().label == "api"`;
        // `set_label(member, "w1")` → unchanged, and it returns `true` (the
        // surface was found) so the caller is not tempted to add a fallback.
        todo!()
    }

    #[test]
    fn a_stopped_or_died_event_for_an_unknown_id_is_not_an_error() {
        // `mark_died(ConversationId::new(99)) == false` with no panic — a `Died`
        // racing a `/tab close` is the normal case.
        todo!()
    }
}
