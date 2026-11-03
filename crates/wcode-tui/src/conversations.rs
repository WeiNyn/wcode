//! The TUI's **two-level container** (W040, locked design:
//! `docs/work/W040-multi-root-conversations.md` §13.2 items 5, 6, 8).
//!
//! # What this module is, and what it is NOT yet (5a vs 5b)
//!
//! **5a (this commit): the container is a DERIVED VIEW. The flat
//! `App.surfaces`/`App.focus` remain AUTHORITATIVE for rendering.** The
//! container is *written* by three real paths (`set_surfaces` reconciles it,
//! `SessionTitle` renames a tab, the agent-event arm records a tab's state) and
//! it is not yet *read* for rendering. That is deliberate and temporary: it lets
//! the container land with the TUI pixel-identical, and **5b deletes the flat
//! fields and makes the container the store**. A reader must not treat the two
//! stores as peers — while both exist, `App.surfaces` wins and this view must
//! never be able to disagree with it.
//!
//! The invariant that keeps them agreeing in 5a is
//! [`Conversations::reconcile_first`]: a roster rebuild RECONCILES (labels and states
//! of surviving conversations survive) instead of replacing, so a `/rename`
//! followed by any refresh cannot silently lose the tab's name.
//!
//! # Deferred to 5b/6 (named here so nothing looks forgotten)
//!
//! - **5b**: `iter`/`len`/`get` for the renderers, `conversation_of`/`index_of`,
//!   `activate`/`activate_id`/`cycle`, `push`, `close`/`CloseOutcome`,
//!   `mark_died`, and the narrowing accessors that make the flat fields
//!   redundant.
//! - **6**: `ConversationFeeds::none`, the `Debug` impl on [`ConversationSpec`]
//!   (it cannot derive one while it carries `SurfaceSpec`/`Backend`), and the
//!   `Request`/`Event` wiring that consumes it.
//!
//! # The N = 1 DELEGATION CONTRACT
//!
//! **With exactly one conversation, every accessor here returns exactly what the
//! pre-W040 field access returned.**
//!
//! 1. `active_index() == 0` and `active()` is `items[0]`, always.
//! 2. `active().surface_ids()` == `[root] ++ members` in **creation order** ==
//!    exactly today's `Vec<Surface>` contents in exactly today's order, so
//!    `App::canonical_order` narrowed to the active conversation yields the SAME
//!    `Vec<usize>` it yields today.
//! 3. `Conversations` is NEVER empty: the only constructors group at least one
//!    surface, so `active()` cannot panic (the `App::surfaces` non-empty
//!    invariant, one level up).
//! 4. `Conversation.label` is set but NOT RENDERED while `len() == 1` — the tab
//!    band has height 0 (`ui::draw`), so a single-conversation frame is
//!    byte-identical to today's.
//!
//! The **surfaces themselves stay in `App.surfaces`** (a flat `Vec<Surface>`) in
//! 5a. `Conversation` therefore holds **ids**, and `App` holds the **bodies**:
//! every `Surface.id` appears in exactly one `Conversation`, and
//! `Conversations.items` is in creation order.
//!
//! # The ONE data model (first-layer finding 2)
//!
//! There is **no `conversation` field on `SurfaceInfo` and none on
//! `SurfaceSpec`**: `SurfaceSpec::info()` drops everything but the display
//! fields, so a runtime member's tag rides BESIDE the spec, as
//! `(ConversationId, SurfaceSpec)` — the pair the CLI already knows, because its
//! spawn sink is per-orchestrator and an orchestrator is per conversation.
//! Consumed by [`Conversations::add_surface_in`].

use wcode_harness::protocol::SessionId;
use wcode_protocol::ConversationId;

use crate::SurfaceSpec;

// ==========================================================================
// One conversation
// ==========================================================================

/// One conversation as the TUI knows it: **ids only** (the `Surface` bodies live
/// in `App.surfaces` in 5a, see the module doc).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    /// Host-minted identity (`wcode_protocol::ConversationId`). Stable; the tab
    /// bar's click target and W041's URL segment.
    pub id: ConversationId,
    /// The tab's label: the host's seed (a cwd basename, or a `Resume` dir's
    /// stem) until the first `AgentEvent::SessionTitle` lands, then that title.
    ///
    /// NOT read from an `Action` reply: `SessionTitle` is a *streamed* event both
    /// a local and a socket client already receive, so a second channel for the
    /// same fact would be a second source of truth that can disagree ("the tab
    /// label's single source").
    pub label: String,
    /// The ROOT surface's routing id — the id every `AppEvent::Agent` for this
    /// conversation's root is tagged with.
    pub root: SessionId,
    /// The non-root surfaces' routing ids, in creation order. Never contains
    /// `root`.
    pub members: Vec<SessionId>,
    /// The tab's state. Written by [`Conversations::reconcile_first`] (which
    /// preserves it) and [`Conversations::set_state_from`] (the sticky `Failed`
    /// rule) only.
    pub state: ConversationState,
    /// `true` when this conversation's backends are `Backend::Remote` (a
    /// `--socket` client). One remote conversation per process today, so it is
    /// never rendered while it is true (the tab band needs `len() > 1`); it exists
    /// so the tab bar has a place to say `remote` the day a client has two.
    pub is_remote: bool,
    /// What the build would have printed; rendered as `Notice` blocks on the
    /// root surface when the tab is added. Printed by the boot to stderr too
    /// (two media, one fact) — do not "fix" the apparent duplication.
    pub notices: Vec<String>,
}

/// The tab's state. **Deliberately three**, not `TeamState`'s four: a conversation
/// that finished its last turn is `Idle` — a `✓ done` tab would be permanent
/// noise on a bar that is always visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ConversationState {
    /// Nothing running (born here). `TeamState::Idle` and `TeamState::Done` both
    /// map here.
    ///
    /// **`Idle` does NOT mean "nothing in this conversation is running."** A
    /// running MEMBER leaves the tab `Idle` by design (see [`Conversation::root`]).
    /// Any caller that needs "is anything live here" must ask the surfaces, not
    /// this value (first-layer finding 10).
    #[default]
    Idle,
    /// The conversation's ROOT is running.
    Running,
    /// Sticky until the conversation next RUNS: either the last run failed
    /// (`TeamState::Failed`) or the root's actor task died (§13.2 item 4).
    Failed,
}

impl Conversation {
    /// Every surface id, root first, then `members` in creation order — the order
    /// `App::canonical_order` (D34) requires, and at N = 1 exactly today's list.
    pub fn surface_ids(&self) -> Vec<SessionId> {
        std::iter::once(self.root.clone())
            .chain(self.members.iter().cloned())
            .collect()
    }

    /// Whether `id` is this conversation's root or one of its members.
    pub fn contains(&self, id: &SessionId) -> bool {
        &self.root == id || self.members.contains(id)
    }
}

// ==========================================================================
// The container
// ==========================================================================

/// The tab container: conversations in creation order + the active index.
///
/// See the module doc for the **N = 1 delegation contract** and for the 5a
/// inversion (this view is derived; the flat store is authoritative).
///
/// **No `Default`** (first-layer nit 1): `#[derive(Default)]` would build
/// `items: []`, the exact state this file calls a constructor bug — `active()`
/// would index an empty `Vec`, and `len() == 0` would make the tab band's
/// `len() > 1` test disagree with every other accessor. The only constructors are
/// [`Conversations::single`] and [`Conversations::from_parts`].
#[derive(Clone, Debug)]
pub struct Conversations {
    items: Vec<Conversation>,
    /// Index into `items`. Clamped by every mutator; `items` is never empty.
    active: usize,
}

impl Conversations {
    /// **The N = 1 conversation, from a surface list** — the startup seed and the
    /// `App::new` placeholder. `surfaces[0]` is the root, `surfaces[1..]` are its
    /// members.
    ///
    /// # Why this takes the whole slice (first-layer finding 4)
    /// A `single(root, label)` that took no members produced
    /// `surface_ids() == [root]` for a TEAM run, and then `App::canonical_order`
    /// narrowed to a single index — which breaks
    /// `backtab_moves_focus_backwards_and_wraps` (`set_surfaces(&mut app,
    /// &["root","a","b"])` then `focus() == 2`). The fallback must therefore group
    /// **every** surface, and it is the same function `App::new` seeds with.
    ///
    /// # Contract
    /// - `surfaces.is_empty()` ⇒ the default-root conversation
    ///   (`SessionId::agent("root")`, label `"wcode"`) — exactly what
    ///   `App::new`'s `default_root()` gives, so the container is valid before
    ///   `set_surfaces` runs.
    /// - Id `ConversationId::first()`, state `Idle`, `is_remote = false` (the
    ///   caller overrides it: `Options.remote`).
    /// - `surface_ids()` equals the input order, always: that equality IS the
    ///   N = 1 contract's clause 2.
    pub fn single(surfaces: &[crate::SurfaceInfo]) -> Self {
        let (root, members, label) = match surfaces.split_first() {
            Some((first, rest)) => (
                first.id.clone(),
                rest.iter().map(|s| s.id.clone()).collect(),
                first.label.clone(),
            ),
            None => (
                SessionId::agent("root"),
                Vec::new(),
                "wcode".to_string(),
            ),
        };
        Self {
            items: vec![Conversation {
                id: ConversationId::first(),
                label,
                root,
                members,
                state: ConversationState::Idle,
                is_remote: false,
            notices: Vec::new(),
            }],
            active: 0,
        }
    }

    /// Adopt the CLI's startup list (`Options.conversations`) once
    /// `App::set_surfaces` has installed the bodies.
    ///
    /// # Contract
    /// - **Validated, not trusted**: every listed id must exist in `surfaces`, and
    ///   every `surfaces` id must appear in exactly one conversation. A mismatch is
    ///   a CLI bug: fall back to [`Conversations::single`] over the SAME slice
    ///   (which groups every surface into one conversation — finding 4) plus a
    ///   `debug_assert!`. Never a panic, never a silently dropped surface.
    /// - **An empty list** (a test, `examples/dump.rs`, a socket client) ⇒
    ///   `single(surfaces)`.
    /// - Preserves the input order: `items[0]` is the startup conversation, so
    ///   `active == 0` at startup.
    pub fn from_parts(conversations: Vec<Conversation>, surfaces: &[crate::SurfaceInfo]) -> Self {
        let covered: Vec<SessionId> = conversations
            .iter()
            .flat_map(|c| c.surface_ids())
            .collect();
        let complete = !conversations.is_empty()
            && covered.len() == surfaces.len()
            && surfaces.iter().all(|s| covered.contains(&s.id))
            && conversations
                .iter()
                .all(|c| surfaces.iter().any(|s| s.id == c.root));
        if !complete {
            debug_assert!(
                conversations.is_empty(),
                "the conversation list does not cover the surfaces exactly once"
            );
            return Self::single(surfaces);
        }
        Self {
            items: conversations,
            active: 0,
        }
    }

    /// **Reconcile the derived view with a rebuilt surface list** — the
    /// `App::set_surfaces` hook, and the rule that keeps 5a's two stores from
    /// disagreeing.
    ///
    /// # Contract
    /// - **It reconciles, it never replaces.** The startup conversation's `label`
    ///   and `state` SURVIVE as long as its root id is still present, so a
    ///   `/rename` (or a `Failed`) followed by any roster refresh cannot be
    ///   silently lost — the "two tabs disagree" class the sketch review removed.
    /// - The id set is re-derived from `surfaces`: `surfaces[0]` is the root,
    ///   `surfaces[1..]` the members, in order (finding 4: members are never
    ///   dropped).
    /// - An empty list keeps the current conversation and re-points it at the
    ///   default root only when it had none.
    /// - **It reconciles `items[0]` and TRUNCATES the rest** — the name says so
    ///   (`reconcile_first`), because until 6b's channel there is exactly one
    ///   conversation and a second item with no body behind it would draw a tab
    ///   over nothing. The multi-conversation reconcile arrives with the commit
    ///   that can create one.
    pub fn reconcile_first(&mut self, surfaces: &[crate::SurfaceInfo]) {
        // One conversation is the whole assumption: anything beyond `items[0]`
        // has no body behind it here.
        self.items.truncate(1);
        self.active = 0;
        let Some((first, rest)) = surfaces.split_first() else {
            return;
        };
        let root = first.id.clone();
        let members: Vec<SessionId> = rest.iter().map(|s| s.id.clone()).collect();
        match self.items.first_mut() {
            // The same root: keep the name and the state (this is the reconcile).
            Some(existing) if existing.root == root => {
                existing.members = members;
            }
            Some(existing) => {
                existing.root = root;
                existing.members = members;
                existing.label = first.label.clone();
                existing.state = ConversationState::Idle;
            }
            None => {
                self.items.push(Conversation {
                    id: ConversationId::first(),
                    label: first.label.clone(),
                    root,
                    members,
                    state: ConversationState::Idle,
                    is_remote: false,
            notices: Vec::new(),
                });
                self.active = 0;
            }
        }
    }

    /// The active conversation. Panics never: `items` is never empty.
    pub fn active(&self) -> &Conversation {
        &self.items[self.active]
    }

    /// The active index (the tab bar's `▌` position, and `App`'s reconcile input).
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// The conversation owning `surface`, if any — the `SessionTitle` path needs
    /// it to decide whether the titled surface is a tab's root.
    pub fn conversation_of(&self, surface: &SessionId) -> Option<&Conversation> {
        self.items.iter().find(|c| c.contains(surface))
    }

    /// Append a conversation the host just announced. `Err` on a duplicate id or a
    /// root already owned by another conversation (never a panic). Does NOT
    /// activate: the caller decides (a `/tab new` activates; a background reply
    /// must not steal the view).
    pub fn push(&mut self, c: Conversation) -> Result<(), String> {
        if self.items.iter().any(|i| i.id == c.id) {
            return Err(format!("conversation {} is already live", c.id));
        }
        if self
            .items
            .iter()
            .any(|i| i.root == c.root || i.members.contains(&c.root))
        {
            return Err(format!("surface {} is already owned", c.root));
        }
        self.items.push(c);
        Ok(())
    }

    /// Remove a conversation and return every surface id it owned, for the caller
    /// that owns the bodies. `None` (and no change) for the LAST conversation: a
    /// host cannot close its only tab, and `active()` must stay valid.
    pub fn remove(&mut self, id: ConversationId) -> Option<Vec<SessionId>> {
        if self.items.len() <= 1 {
            return None;
        }
        let idx = self.items.iter().position(|c| c.id == id)?;
        let gone = self.items.remove(idx);
        // Keep the same conversation active: clamp into range.
        self.active = self.active.min(self.items.len() - 1);
        Some(gone.surface_ids())
    }

    /// Record the host's verdict that a conversation's root runtime is gone
    /// (§13.2 item 4). Sticky `Failed` — the tab must not lie that it is idle.
    /// `false` when the id is unknown (a `Died` racing a close is not an error).
    pub fn mark_died(&mut self, id: ConversationId) -> bool {
        match self.items.iter_mut().find(|c| c.id == id) {
            Some(item) => {
                item.state = ConversationState::Failed;
                true
            }
            None => false,
        }
    }

    /// Index of `id`, if any.
    pub fn index_of(&self, id: ConversationId) -> Option<usize> {
        self.items.iter().position(|c| c.id == id)
    }

    /// Activate by index. `false` (and no change) when out of range — the
    /// `App::set_focus` tolerance.
    pub fn activate(&mut self, idx: usize) -> bool {
        if idx >= self.items.len() {
            return false;
        }
        self.active = idx;
        true
    }

    /// Every conversation, in creation order — the tab band's draw order and
    /// `/tab`'s numbering (1-based, exactly like the sidebar's badges).
    pub fn iter(&self) -> impl Iterator<Item = &Conversation> {
        self.items.iter()
    }

    /// How many conversations. `1` means the tab band has height 0 (contract 4).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `len() == 0`. Present for the `clippy::len_without_is_empty` gate; it is
    /// never `true`.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Register a runtime-spawned surface into `conversation`. `Err` when the
    /// conversation is unknown (the CLI emitted a member for a tab the user just
    /// closed) or the id is already owned.
    ///
    /// # Why the tag is a PARAMETER and not a field (first-layer finding 2)
    /// `SurfaceSpec::info()` copies four fields into a [`crate::SurfaceInfo`] and
    /// **drops the conversation tag** — it is not a `SurfaceSpec` field, by the
    /// module header's decision. The losing line is `app.add_surface(spec.info())`
    /// in `lib.rs`, where the inbound `(ConversationId, SurfaceSpec)` pair is
    /// consumed. So the tag must ride BESIDE the spec and be handed in
    /// explicitly; a caller that has only a `SurfaceInfo` has no way to say which
    /// tab it belongs to.
    pub fn add_surface_in(
        &mut self,
        conversation: ConversationId,
        info: crate::SurfaceInfo,
    ) -> Result<(), String> {
        // A surface owned by ANOTHER conversation is never ours to take.
        if self.items.iter().any(|c| {
            c.id != conversation && (c.root == info.id || c.members.contains(&info.id))
        }) {
            return Err(format!("surface {} is owned by another conversation", info.id));
        }
        let Some(item) = self.items.iter_mut().find(|c| c.id == conversation) else {
            return Err(format!("conversation {conversation} is not live"));
        };
        // **The root is not a member** (and is already tracked as the root): a
        // `ConversationSpec` lists `root + members`, so the root reaches this
        // method on every `Added` reply. Accepting it idempotently is what makes
        // "every listed surface has a body" true; refusing it (the first cut)
        // silently dropped the root's body while its backend was wired.
        if item.root == info.id {
            return Ok(());
        }
        if item.members.contains(&info.id) {
            return Err(format!("surface {} is already tracked", info.id));
        }
        item.members.push(info.id);
        Ok(())
    }

    /// Record the state of the conversation owning `surface` from that surface's
    /// `TeamState` — the agent-event arm's one-line hook. `false` when the surface
    /// belongs to no conversation.
    ///
    /// # The truth table (first-layer finding 10 — `Failed` stickiness)
    ///
    /// For the conversation's ROOT surface:
    ///
    /// | input `TeamState` | previous tab state | next tab state |
    /// |---|---|---|
    /// | `Running` | anything (incl. `Failed`) | **`Running`** — the reset |
    /// | `Failed` | anything | `Failed` |
    /// | `Idle` / `Done` | `Failed` | **stays `Failed`** |
    /// | `Idle` / `Done` | `Running` / `Idle` | `Idle` |
    ///
    /// A **member** surface is a no-op for the state (it still returns `true`: the
    /// surface was found — `set_label`'s convention), because the tab never
    /// promotes a member.
    ///
    /// # Why not "until the root's next `AgentStart`"
    /// There is no `AgentStart` input on this side of the seam: `Surface::apply`
    /// consumes the event and all `App` can read afterwards is `Surface::state()`.
    /// `TeamState::Running` is the exact observable proxy: `state()`'s ladder is
    /// `failed → running → finished → idle`, and `failed` is itself cleared only by
    /// `AgentStart`, so the first `Running` after a failure IS the next run's start.
    pub fn set_state_from(&mut self, surface: &SessionId, state: crate::TeamState) -> bool {
        let Some(item) = self.items.iter_mut().find(|c| c.contains(surface)) else {
            return false;
        };
        if &item.root != surface {
            return true; // a member never promotes the tab
        }
        item.state = match (state, item.state) {
            (crate::TeamState::Running, _) => ConversationState::Running,
            (crate::TeamState::Failed, _) => ConversationState::Failed,
            (_, ConversationState::Failed) => ConversationState::Failed, // sticky
            (crate::TeamState::Idle | crate::TeamState::Done, _) => ConversationState::Idle,
        };
        true
    }

    /// Replace a conversation's label — the `AgentEvent::SessionTitle` hook.
    /// `false` when the surface belongs to no conversation.
    ///
    /// Only the ROOT's title renames the tab: a member's `/rename` names the
    /// member, and the sidebar already shows that.
    pub fn set_label(&mut self, surface: &SessionId, label: String) -> bool {
        let Some(item) = self.items.iter_mut().find(|c| c.contains(surface)) else {
            return false;
        };
        if &item.root == surface {
            item.label = label;
        }
        true
    }
}

// ==========================================================================
// The reverse channel (§13.2 item 8) — the vocabulary; the wiring is commit 6
// ==========================================================================

/// **The TUI → CLI channel**: what the app asks the host to do. The exact reverse
/// of `new_surfaces`, which is the CLI → TUI channel.
///
/// # Why the reverse is needed at all
///
/// Every mutator the TUI has ever had is *local*: it can only answer the CLI at
/// EXIT (`pending_team`/`pending_reload`) and let
/// the CLI re-exec — which is exactly why "new conversation" means "new terminal"
/// today. A tab cannot re-exec: the other conversations would die with the
/// process. So the TUI needs a live request/response pair, and the *build* half
/// can only be done by the CLI (the TUI holds no `llm`, no session dir, no
/// `Background`, no `Agent` — `Options` is the injection list).
///
/// Commit 6 emits these; 5a only fixes the vocabulary.
pub enum ConversationRequest {
    /// Build one root. Answered by a [`ConversationEvent::Added`], or by
    /// `ConversationEvent::Failed` carrying the factory's error string.
    Start(wcode_protocol::RootSpec),
    /// Tear one down: `Background::shutdown_scope` for its `bg` tasks, then
    /// `Registry::remove` + drop the handle.
    ///
    /// `App` removes the tab on the reply's echo (`Event::Stopped`), not
    /// optimistically, so a failed teardown cannot leave a tab whose backend is
    /// gone.
    Stop(ConversationId),
}

/// **The CLI → TUI channel's payload** — the `new_surfaces` sibling, for
/// conversations rather than members. Commit 6 consumes it.
pub enum ConversationEvent {
    /// One conversation is live: its id, seed label, and the surfaces to attach
    /// (root first).
    Added(ConversationSpec),
    /// The host could not build it (`/tab new` on a full disk, a bad `Resume`).
    /// Rendered as a `transient` — the `/reload`-refusal idiom.
    Failed { error: String },
    /// The tab was torn down; `App` may now forget it.
    Stopped { id: ConversationId },
    /// The root's actor task ENDED unexpectedly (§13.2 item 4). Distinct from
    /// `Stopped` so the tab renders `Failed` instead of disappearing.
    Died { id: ConversationId },
}

/// One conversation's startup payload — [`ConversationEvent::Added`]'s body.
///
/// **Not** a second `Conversation`: it is what the CLI can build (surfaces with
/// live `Backend`s), and `Conversations` derives the view-model from it.
///
/// No derive: it carries [`SurfaceSpec`], which is neither `Debug` nor `Clone`
/// (`Backend` is not `Debug`). Commit 6 adds a hand-written `Debug` when
/// something actually prints it.
pub struct ConversationSpec {
    pub id: ConversationId,
    /// The seed label — the cwd basename, or a `Resume` dir's stem.
    pub label: String,
    /// Root first (`is_root: true`), then members in creation order. The
    /// conversation id is THIS struct's `id`: `SurfaceSpec` carries no tag, so the
    /// payload's shape is what associates every listed surface with `id`.
    pub surfaces: Vec<SurfaceSpec>,
    /// What the build would have printed, rendered as `Notice` blocks on the
    /// root surface when the tab is added.
    pub notices: Vec<String>,
}

/// The two directions the CLI hands `wcode_tui::run`, bundled so `run` gains ONE
/// parameter instead of two.
///
/// A `--socket` client has no local build capability, so it passes
/// `ConversationFeeds::none()` and `/tab new` refuses with the same wording.
/// `None` ⇒ the reducer's gate refuses before any `Action` is emitted.
///
/// Commit 6 adds `none()` and the loop that services it.
#[derive(Default)]
pub struct ConversationFeeds {
    /// Replies + lifecycle from the CLI.
    pub incoming: Option<tokio::sync::mpsc::UnboundedReceiver<ConversationEvent>>,
    /// Requests out to the CLI.
    pub outbound: Option<tokio::sync::mpsc::UnboundedSender<ConversationRequest>>,
}

impl ConversationFeeds {
    /// No host behind this TUI (a `--socket` client): both ends `None`, and
    /// `/tab new|close` refuse before any `Action` is emitted (`App.remote`).
    pub fn none() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SurfaceInfo;

    fn info(id: &str, is_root: bool) -> SurfaceInfo {
        SurfaceInfo {
            id: SessionId::agent(id),
            label: id.to_string(),
            model: "m1".into(),
            is_root,
            resume_path: None,
        }
    }

    fn ids(c: &Conversations) -> Vec<String> {
        c.active()
            .surface_ids()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn n1_delegates_every_accessor_to_the_single_conversation() {
        // The `App::new` placeholder: valid, non-empty, and the default root.
        let empty = Conversations::single(&[]);
        assert_eq!(empty.len(), 1);
        assert!(!empty.is_empty());
        assert_eq!(empty.active_index(), 0);
        assert_eq!(ids(&empty), vec!["agent:root".to_string()]);
        assert_eq!(empty.active().label, "wcode");
        assert_eq!(empty.active().id, ConversationId::first());

        // One surface, no members.
        let one = Conversations::single(&[info("root", true)]);
        assert_eq!(ids(&one), vec!["agent:root".to_string()]);
        assert_eq!(one.active().label, "root");
    }

    #[test]
    fn single_over_a_team_run_keeps_every_surface_in_the_conversation() {
        // FINDING 4's gate: a `single` that returned `[root]` would make
        // `backtab_moves_focus_backwards_and_wraps` (`focus() == 2`) fail.
        let c = Conversations::single(&[
            info("root", true),
            info("a", false),
            info("b", false),
        ]);
        assert_eq!(
            ids(&c),
            vec!["agent:root".to_string(), "agent:a".into(), "agent:b".into()],
            "N = 1 keeps root + members, in input order"
        );
        assert_eq!(
            c.active().members,
            vec![SessionId::agent("a"), SessionId::agent("b")]
        );
    }

    #[test]
    fn from_parts_groups_by_conversation_and_falls_back_to_single() {
        let surfaces = [
            info("r0", true),
            info("m0", false),
            info("r1", true),
        ];
        let two = vec![
            Conversation {
                id: ConversationId::first(),
                label: "first".into(),
                root: SessionId::agent("r0"),
                members: vec![SessionId::agent("m0")],
                state: ConversationState::Idle,
                is_remote: false,
            notices: Vec::new(),
            },
            Conversation {
                id: ConversationId::new(1),
                label: "second".into(),
                root: SessionId::agent("r1"),
                members: Vec::new(),
                state: ConversationState::Idle,
                is_remote: false,
            notices: Vec::new(),
            },
        ];
        let c = Conversations::from_parts(two, &surfaces);
        assert_eq!(c.len(), 2);
        assert_eq!(c.active().label, "first");
        assert_eq!(c.active().members, vec![SessionId::agent("m0")]);

        // An empty list falls back to `single`, which groups EVERY surface.
        let fallback = Conversations::from_parts(Vec::new(), &surfaces);
        assert_eq!(fallback.len(), 1);
        assert_eq!(
            ids(&fallback),
            vec!["agent:r0".to_string(), "agent:m0".into(), "agent:r1".into()]
        );

    }

    /// A list that MISSES a surface is a CLI bug: the recovery is the `single`
    /// fallback (never a dropped surface) and a `debug_assert!` that names it — so
    /// in a debug build it is a panic, which is what this pins. The fallback value
    /// itself is the same code path as the empty list above.
    #[test]
    #[should_panic(expected = "cover the surfaces exactly once")]
    fn from_parts_flags_a_list_that_misses_a_surface() {
        let surfaces = [info("r0", true), info("m0", false)];
        let _ = Conversations::from_parts(
            vec![Conversation {
                id: ConversationId::first(),
                label: "only".into(),
                root: SessionId::agent("r0"),
                members: Vec::new(),
                state: ConversationState::Idle,
                is_remote: false,
            notices: Vec::new(),
            }],
            &surfaces,
        );
    }

    /// **5a's anti-drift rule (condition 1).** The container is a derived view
    /// while `App.surfaces` renders, so a roster rebuild must RECONCILE: a
    /// `/rename` and a `Failed` survive `set_surfaces`, or the two stores would
    /// silently disagree — the class the sketch review removed.
    #[test]
    fn reconcile_first_keeps_the_label_and_state_of_the_surviving_conversation() {
        let surfaces = [info("root", true), info("a", false)];
        let mut c = Conversations::single(&[]);
        c.reconcile_first(&surfaces);

        let root = SessionId::agent("root");
        assert!(c.set_label(&root, "api".into()), "the root renames the tab");
        assert!(c.set_state_from(&root, crate::TeamState::Failed));
        assert_eq!(c.active().label, "api");
        assert_eq!(c.active().state, ConversationState::Failed);

        // The same ids arrive again — the roster refresh — and nothing is lost.
        c.reconcile_first(&surfaces);
        assert_eq!(c.active().label, "api", "the label survived the rebuild");
        assert_eq!(
            c.active().state,
            ConversationState::Failed,
            "the sticky state survived the rebuild"
        );
        assert_eq!(ids(&c), vec!["agent:root".to_string(), "agent:a".into()]);

        // A different root IS a new conversation: name and state start over.
        c.reconcile_first(&[info("other", true)]);
        assert_eq!(c.active().label, "other");
        assert_eq!(c.active().state, ConversationState::Idle);
        assert_eq!(ids(&c), vec!["agent:other".to_string()]);
    }

    #[test]
    fn a_member_running_does_not_promote_the_tab_state() {
        let mut c = Conversations::single(&[info("root", true), info("a", false)]);
        let member = SessionId::agent("a");
        assert!(c.set_state_from(&member, crate::TeamState::Running));
        assert_eq!(
            c.active().state,
            ConversationState::Idle,
            "the tab never promotes a member (finding 10: the close predicate must ask the surfaces)"
        );
        // And a member's title does not rename the tab, but is `true` (found).
        assert!(c.set_label(&member, "w1".into()));
        assert_eq!(c.active().label, "root");
    }

    #[test]
    fn failed_is_sticky_until_the_next_run_starts() {
        let mut c = Conversations::single(&[info("root", true)]);
        let root = SessionId::agent("root");
        // root Failed -> Failed
        c.set_state_from(&root, crate::TeamState::Failed);
        assert_eq!(c.active().state, ConversationState::Failed);
        // root Done / Idle -> STAYS Failed (the old bug cleared here)
        c.set_state_from(&root, crate::TeamState::Done);
        assert_eq!(c.active().state, ConversationState::Failed);
        c.set_state_from(&root, crate::TeamState::Idle);
        assert_eq!(c.active().state, ConversationState::Failed);
        // root Running -> Running (the ONE reset)
        c.set_state_from(&root, crate::TeamState::Running);
        assert_eq!(c.active().state, ConversationState::Running);
        // root Done, from Running -> Idle
        c.set_state_from(&root, crate::TeamState::Done);
        assert_eq!(c.active().state, ConversationState::Idle);
    }

    #[test]
    fn add_surface_in_tags_the_conversation_and_refuses_strangers() {
        let mut c = Conversations::single(&[info("root", true)]);
        let tag = c.active().id;
        assert!(c.add_surface_in(tag, info("w1", false)).is_ok());
        assert_eq!(ids(&c), vec!["agent:root".to_string(), "agent:w1".into()]);
        // The same surface twice is refused, not duplicated.
        assert!(c.add_surface_in(tag, info("w1", false)).is_err());
        // A member for a conversation that is not live is refused.
        assert!(c.add_surface_in(ConversationId::new(99), info("w2", false)).is_err());
        assert_eq!(ids(&c).len(), 2, "no body was added for the stranger");
    }

    #[test]
    fn a_stopped_or_died_event_for_an_unknown_id_is_not_an_error() {
        // A `Died` racing a `/tab close` is the normal case; `mark_died` lands in
        // 5b/6, but the lookups it rests on must already be tolerant.
        let c = Conversations::single(&[info("root", true)]);
        assert!(
            c.conversation_of(&SessionId::agent("nobody")).is_none(),
            "an unknown surface belongs to no conversation"
        );
        assert_eq!(c.active_index(), 0, "and nothing moved");
    }
}
