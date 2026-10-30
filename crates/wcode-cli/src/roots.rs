//! **SKETCH — review-only, not real code. NOT a module of this crate yet.**
//!
//! Do NOT add `mod roots;` to `main.rs` until this is implemented. Until then it
//! is inert: it is not compiled, it may not compile, and that is the point
//! (`.wcode/skills/interface-sketch/SKILL.md` §0 Form A).
//!
//! # What this module is
//!
//! The **multi-root core** (W040 §13.2 items 1–4, 8): the CLI's registry of live
//! *conversations*, the injected factory that builds one, the supervisor that
//! notices one dying, and the reverse channel that lets a client ask for another.
//!
//! A conversation is an **in-process root runtime** (§13.2 item 1):
//! `Backend::Local(SessionHandle)` — no child process, no socket, no pid, no
//! readiness wait. Today exactly one such root exists, constructed by hand at
//! `crates/wcode-cli/src/main.rs:2323`-region (the TUI branch) and
//! `:1896`-region (`serve()`); this module turns that hand-rolled construction
//! into `N` of them behind one type.
//!
//! # Why here, and not in `wcode-protocol` / `wcode-tui`
//!
//! `Root` owns `Arc<Background>`, `Orchestrator`, `SessionGroup` — all
//! `wcode-cli` types. So the *container* cannot live below the composition root;
//! only the **keys** (`ConversationId`, `RootSpec`) can, and they do
//! (`crates/wcode-protocol/src/conversation.rs`). §13.3 states the split:
//! "The container and the root lifecycle live in the CLI + TUI".
//!
//! # Reuse by a future HTTP host: a CANDIDATE, not a claim
//!
//! **The whole point of the finding-6 question is that nothing here is proved
//! reusable, because `wcode-cli` is a BINARY — no other crate can reach this
//! module at all.** Whether the shape survives contact with `wcode-serve` is
//! open (`W040` §13.5.2); every sentence below about a second host is therefore
//! written as *candidate reuse, pending that ruling*, never as fact.
//!
//! What the shape does establish, and what a reviewer can check today:
//! **§§1–5 name no `wcode_tui` type.** The supervisor speaks [`RootEvent`]; the
//! host loop's inbound/outbound vocabulary is [`ConversationRequest`] and
//! `wcode_tui::conversations::ConversationEvent` — and **that pair, plus
//! [`HostChannels`], is the entire TUI edge, all of it in §6 and §7** (§6 is
//! where the two channel types appear; §7 is the only TUI-shaped `impl` block).
//! So the honest boundary is "§§1–5 are TUI-free; §6–§7 are the seam a second
//! host would have to re-translate", and the concrete inventory is in the
//! report's web-reuse verdict. An earlier draft of this header claimed
//! "nothing in §§1–6 names a `wcode_tui` type" and then named two in §6's own
//! `HostChannels` — corrected here.
//!
//! # Integration points (re-verified in this session, this tree)
//!
//! - `crates/wcode-cli/src/main.rs:1883` `async fn serve(` — the single-root
//!   construction that becomes `build_root` + `Root::adopt`.
//! - `crates/wcode-cli/src/main.rs:1907` `let registry = orchestrator` — the
//!   per-root `Registry` (falls back to a bare `Registry::default()` without
//!   `--agents`, `:1840`).
//! - `crates/wcode-cli/src/main.rs:1929` `let roster = match orchestrator {` +
//!   `:1930` `Some(o) => live_roster(o.registry(), o.id().clone()),`.
//! - `crates/wcode-cli/src/main.rs:2323` `let handle = SessionActor::spawn(agent);`
//!   … `:2296` `.unwrap_or_else(|| SessionId::agent("root")),` — the TUI branch's
//!   root surface id.
//! - `crates/wcode-cli/src/main.rs:2346` `let new_surfaces = if args.owner.is_none() {`
//!   — the member feed's install site (`o.set_spawn_sink(tx)`), which becomes
//!   per-root and tagged.
//! - `crates/wcode-cli/src/repl.rs:388` `pub fn build_agent(` — the engine a
//!   `Fresh` root already has.
//! - `crates/wcode-cli/src/repl.rs:920` `// The rebuild inputs (llm/hooks/tools/…/orchestrator) are threaded positionally;`
//!   — the input bundle the factory must acquire a name for.
//! - `crates/wcode-cli/src/tools/background.rs:287`
//!   `static LIVE: OnceLock<Mutex<Vec<Weak<Background>>>> = OnceLock::new();` —
//!   the unscoped list `shutdown_scope` replaces for per-root teardown.
//! - `crates/wcode-cli/src/session_groups.rs:9`-region (the on-disk layout) and
//!   `:164` `pub fn create_group(base: &Path) -> io::Result<SessionGroup> {` — one
//!   group dir per root, which is why a conversation is resumable on its own.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use wcode_harness::actor::SessionHandle;
use wcode_harness::agent::Agent;
use wcode_harness::protocol::SessionId;
use wcode_protocol::{ConversationId, Registry, RootSpec, Roster};

use crate::agents::Orchestrator;
use crate::repl::RunInputs;
use crate::session_groups::SessionGroup;
use crate::tools::background::Background;

// ==========================================================================
// §1 — one live root
// ==========================================================================

/// What building or adopting one root can go wrong. `Display` is the string the
/// client sees, so every variant must be a sentence a human can act on (§13.2
/// item 8's `Failed` reply is `String`, and that reply carries no id — see
/// "the id-less `Failed` reply" below).
#[derive(Debug)]
pub enum RootError {
    /// The client asked for a conversation whose group dir does not exist or is
    /// not a group (`session_groups::is_group_dir_name`, `:281`).
    BadResume(PathBuf),
    /// Session IO (create/open a group, `root.jsonl`, a member file).
    Session(std::io::Error),
    /// The runtime could not be built — a bad `[team]`/`[workflow]`/`[peers]`
    /// entry that `build_runtime` (`main.rs:1462`) currently answers with
    /// `std::process::exit(2)`. Those exits must become this ("the exiting,
    /// printing factory body").
    Config(String),
    /// A conversation with this id (or this `Resume` dir) is already live.
    AlreadyLive(String),
}

impl std::fmt::Display for RootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for RootError {}

/// **One live conversation's runtime.** Everything the host needs to talk to it,
/// tear it down, serve it, and notice it dying — and nothing a client needs.
///
/// # Invariants
///
/// 1. `registry` is **this root's own** `Registry` — never a clone of another
///    root's, never a process-global. Two roots share nothing
///    (`two_roots_do_not_share_a_registry_or_roster`). Without `--agents` it is
///    still a real `Registry::new()` (`main.rs:1907`
///    `.unwrap_or_default()`), so `Root.registry` is not an `Option`.
/// 2. `roster` excludes `address` (`live_roster`'s `exclude` argument,
///    `main.rs:1927`) so the root is never listed as its own member.
/// 3. `bg` is the root's own `Arc<Background>` (`repl.rs:395`
///    `let bg = Background::new();`) **and** every member's `Background` is
///    tagged with this root's `id` (`background.rs` sketch). Dropping this field
///    is not a teardown: `Background` has no `Drop` (`background.rs:284`-region,
///    "Drop never runs on `exec_self` or the ~29 `process::exit` sites"), which is
///    why [`Roots::close`] is explicit.
/// 4. `supervisor` is the actor task's `JoinHandle` (§13.2 item 4). It exists
///    only because `SessionActor::spawn` returns a bare `SessionHandle`
///    (`crates/wcode-harness/src/actor.rs:273` `pub fn spawn(agent: Agent) ->
///    SessionHandle {`, discarding `tokio::spawn`'s handle at `:278`).
///
///    **`SessionActor::spawn_supervised` DOES NOT EXIST YET** — this sketch is
///    the only place that names it, and it is part of this item, not an existing
///    seam. The change is two lines in `crates/wcode-harness/src/actor.rs`: a
///    new `pub fn spawn_supervised(agent: Agent) -> (SessionHandle, JoinHandle<()>)`
///    that keeps the handle `tokio::spawn` returns, with `spawn` reduced to
///    `spawn_supervised(agent).0` so **every existing call site keeps its
///    signature unchanged** — 17 of them, in `main.rs` (`:1899`, `:2022`, `:2036`,
///    `:2319`, `:3539`), `repl.rs` (`:956`, `:1097`, `:1253`, `:1302`),
///    `agents.rs` (`:402`, `:1108`) and `tools/{spawn,member,background}.rs`
///    (their test fixtures), e.g.
///    `crates/wcode-harness`'s own `spawn` doc at `actor.rs:266`-region. The harness is
///    therefore NOT "unchanged" (§13.3 as amended: the WIRE is unchanged — no
///    `Frame` field, no `PROTOCOL_VERSION` bump — and this is one of the two
///    additive, non-wire exceptions, the other being
///    `crates/wcode-protocol/src/conversation.rs`).
/// 5. `members` is in **the order the TUI/`serve` must show them** — the
///    `[team]` order, or a resumed group's `scan_members` order
///    (`main.rs:2348`-region `let member_names: Vec<String> =`). NOT
///    `Registry::locals()`'s alphabetical order
///    (`crates/wcode-protocol/src/registry.rs:166`), which would reorder today's
///    sidebar and break N = 1.
/// 6. **A SURFACE ID IS OPAQUE** (first-layer finding 3). Every id a client
///    routes by — `surface` and each member id [`Root::surface_specs`] emits — is
///    a CLIENT LABEL produced by [`mint_surface_id`]. It is **never** a
///    `Registry` key, **never** passed to `Registry::resolve`, and **never**
///    `short_name`d for display. The A2A addresses are `address` and `members`,
///    and those are what `message`/`member`/`spawn { to }` address. The two
///    spaces happen to coincide for conversation 0 (byte-identical N = 1) and
///    diverge from `c1` on — so any code that treats a surface id as an address
///    is correct exactly until the first `/tab new`, and then silently wrong.
///    Concretely why minting is mandatory: every root's worker is `agent:w1`
///    because the uniqueness check is per-orchestrator
///    (`agents.rs:339` `if !self.registry.contains(&SessionId::agent(&candidate)) {`),
///    and a flat client map (`fn surface_index(&self, id: &SessionId)`) would
///    then deliver one root's events to the other's row ("the one flat
///    surface-id key").
pub struct Root {
    /// The host-minted key. Stable for the root's life; the client groups by it.
    pub id: ConversationId,
    /// The tab label's seed — the cwd basename (`main.rs:2093`-region) or a
    /// resumed group's dir stem. A later `AgentEvent::SessionTitle` overrides it
    /// **in the client**, never here ("the tab label's single source").
    pub label: String,
    /// The root's A2A address — `Orchestrator::id()` (`agents.rs:780`), i.e.
    /// `agent:orchestrator`, or `agent:root` without `--agents`. This is the key
    /// the root's OWN `registry` holds it under (`register_root`, `agents.rs:872`)
    /// and the id `live_roster` excludes. **Per-root unique by construction**
    /// (each root has its own registry) but **NOT unique across roots** — it is
    /// the same string in every conversation ("the one flat surface-id key").
    pub address: SessionId,
    /// The id the CLIENT tags this root's events with (`SurfaceSpec.id`,
    /// `crates/wcode-tui/src/lib.rs:257`). Equal to `address` for conversation 0
    /// (byte-identical N = 1) and minted distinct for 1..N. A client-side label
    /// only — `lib.rs`'s forwarder tags events with it and nothing puts it on a
    /// wire (`spawn_forwarder`, `crates/wcode-tui/src/lib.rs:689`). OPAQUE
    /// (invariant 6): never a `Registry` key, never `short_name`d. The SAME rule
    /// governs every member id `surface_specs` emits, which is why the mint is
    /// applied per-surface and not only here.
    pub surface: SessionId,
    /// The live actor. Cloning is cheap and keeps the session alive
    /// (`crates/wcode-harness/src/actor.rs:137`-region, `SessionHandle` doc).
    pub handle: SessionHandle,
    /// This root's address book + A2A bus, shared with `serve()` and the
    /// orchestrator's tools.
    pub registry: Registry,
    /// The live team, root excluded (`Roster` = `watch::Receiver<Vec<(SessionId,
    /// SessionHandle)>>`, `crates/wcode-protocol/src/server.rs:37`).
    pub roster: Roster,
    /// The root's background-task registry+notifier; also the scope tag its
    /// members' `Background`s inherit (§13.2 item 3).
    pub bg: Arc<Background>,
    /// The A2A wiring; `None` without `--agents` and for a served `--owner`
    /// worker (`main.rs:2346` `if args.owner.is_none() {`).
    pub orchestrator: Option<Orchestrator>,
    /// The team, in display order (invariant 5). These are **A2A addresses** —
    /// what `message`/`member` address and what `Registry::remove` takes — NOT
    /// surface ids. The client-visible id for each is
    /// `mint_surface_id(self.id, &member)` (invariant 6).
    pub members: Vec<SessionId>,
    /// The root's session file, when sessions are on. `None` for `--no-session`
    /// and for a served worker with no local file.
    pub session_path: Option<PathBuf>,
    /// The group dir (`session_groups.rs:47` `pub struct SessionGroup {`), when
    /// this root has one. The unit of "resume this conversation".
    pub group: Option<SessionGroup>,
    /// The effective model this root was built with — per-conversation, because
    /// a resumed `Fresh`/`Resume` restores the session's own
    /// (`main.rs:1322`-region `llm.model = m;`).
    pub model: String,
    /// Supervisor-owned state, mirrored to the client. `Failed` is sticky.
    pub state: RootState,
    /// Set before the handles drop, so the supervisor does not mistake our own
    /// teardown for a crash (§5).
    closing: Arc<AtomicBool>,
    /// The actor task's handle (§13.2 item 4). `None` only in tests.
    supervisor: Option<JoinHandle<()>>,
}

/// A root's liveness, as the host tracks it. The client's
/// `wcode_tui::conversations::ConversationState` is a *projection* of this
/// (`Idle`/`Running` come from the client's own surface events; only `Failed`
/// and `Dead` originate here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootState {
    /// Built and answering.
    Live,
    /// The last run failed — client-derived; the host only mirrors it if a
    /// client asks (`Roots::set_state`).
    Failed,
    /// The actor task resolved: panicked, or ended without a `close`
    /// (§13.2 item 4). Sticky; the handles are still held so the client can read
    /// what the root produced.
    Dead,
}

/// The several things one root is built from, grouped so `Root::adopt` and
/// `build_root` cannot drift apart.
pub struct RootParts {
    pub id: ConversationId,
    pub label: String,
    pub agent: Agent,
    pub bg: Arc<Background>,
    pub orchestrator: Option<Orchestrator>,
    pub group: Option<SessionGroup>,
    pub session_path: Option<PathBuf>,
    pub model: String,
    /// The team, in display order (invariant 5). Empty at startup for a fresh
    /// root — the `[team]` loop inside the build fills it.
    pub members: Vec<SessionId>,
}

impl Root {
    /// **Adopt an already-built agent** — the startup path, where `main` has
    /// already run `load_session` + `build_runtime` + `build_agent_for`
    /// (`main.rs:1286`, `:1462`, `:1737`) before `dispatch` is called.
    ///
    /// This is the seam that makes `dispatch`'s hand-rolled five lines
    /// (`main.rs:2323`-region: spawn, bind, register_root, surface list) into one
    /// call, so the startup root and a `/tab new` root are the same kind of
    /// object — otherwise closing tab 0 would be a special case.
    ///
    /// # Contract
    /// - Spawns the actor via **`SessionActor::spawn_supervised` — a function this
    ///   item ADDS; it does not exist in the tree yet** (see `Root`'s invariant 4:
    ///   two lines in `crates/wcode-harness/src/actor.rs`, `spawn` unchanged for
    ///   its five call sites). Never the plain `spawn`: the `JoinHandle` it throws
    ///   away is the whole of §13.2 item 4.
    /// - `bg.bind(handle.clone())`, then `orchestrator.register_root(handle.clone())`
    ///   — today's order (`main.rs:2324`–`:2327`).
    /// - `surface = mint_surface_id(id, address)`, `address = orchestrator.id()`
    ///   or `SessionId::agent("root")` — today's rule (`main.rs:2332`-region).
    /// - Registers the supervisor. `Err` only if `bg` was already bound to another
    ///   handle (`Background::bind`'s own contract, `background.rs:504`).
    pub fn adopt(parts: RootParts) -> Result<Root, RootError> {
        todo!()
    }

    /// Tear this root down **without** a `Roots` — for the exit paths. Same steps
    /// and the same order as [`Roots::close`] step 2 onwards; kept as one
    /// function body (`Roots::close` = mark closing + `remove` + this).
    pub fn tear_down(self) {
        todo!()
    }
}

// ==========================================================================
// §2 — the registry of live roots
// ==========================================================================

/// **The host's live-root registry.** One `Roots` per process (the TUI's host
/// task owns it; a future HTTP host would own its own — a CANDIDATE reading,
/// not a claim (finding 6 is open; `W040` §13.5.2).
///
/// # What it deliberately is NOT
///
/// - **Not a focus holder.** Which conversation is 'active' is the client's
///   business (`wcode_tui::conversations::Conversations::active_index`). The host
///   never reorders, never prioritises.
/// - **Not a `Registry`.** No name clash by accident: `wcode_protocol::Registry`
///   is the *address book inside one root*; this is the *list of roots*.
/// - **Not a session store.** Files live in `session_groups`; a closed root's
///   group stays on disk (§4).
///
/// No `Default`: the event sink is installed **once, before any root exists**, so
/// a supervisor can never be built without somewhere to report. `Roots::new` is
/// the only constructor.
pub struct Roots {
    /// Creation order — the tab order. Dense: `remove` drains, never swaps, so a
    /// client's index-based views never see a reordering.
    items: Vec<Root>,
    /// The `ConversationId` mint source. Starts at **1**: seq `0`
    /// (`ConversationId::first()`) is RESERVED for the startup conversation, which
    /// `dispatch` builds outside [`Roots::start`] and hands to [`Roots::insert`]
    /// (finding 8 — starting at 0 double-booked `c0` onto the first `/tab new`).
    /// Monotone and never reused, so a closed id can never be confused with a new
    /// root and a failed build never hands its id back.
    next: u64,
    /// Where supervisors report (`RootEvent`).
    events: mpsc::UnboundedSender<RootEvent>,
}

impl Roots {
    /// A registry wired to one event sink, with the startup id already reserved.
    ///
    /// # Contract
    /// `next = 1` and `items` empty. `ConversationId::first()` (seq `0`) is the
    /// STARTUP conversation's id; this constructor does not create it — the
    /// composition root does, before the client exists, and hands it to
    /// [`Roots::insert`]. Starting `next` at 1 is what stops the first `/tab new`
    /// from minting `c0` a second time (finding 8).
    pub fn new(events: mpsc::UnboundedSender<RootEvent>) -> Self {
        todo!()
    }

    /// **The two verbs a HOST is made of**, hoisted out of the transport so that
    /// a second host would not have to re-derive them. The TUI's `run_host` (§6)
    /// uses them today; whether an HTTP host shares them VERBATIM is the open
    /// finding-6 question (`W040` §13.5.2), and this comment used to assert it
    /// ("share them verbatim"). Everything either host does on `Start`/`Stop` is
    /// in here; what differs is only how the request arrived and where the reply
    /// goes.
    ///
    /// # Contract
    /// Exactly three steps, in this order:
    ///
    /// ```text
    /// let id = self.mint();
    /// let root = factory(id, spec)?;      // the id goes IN — see Factories
    /// self.insert(root)?;                 // refuses an occupied id
    /// Ok(id)
    /// ```
    ///
    /// - **The id is minted HERE and handed to the factory** (finding 7), so
    ///   `Root.id` and every surface id derived from it come from one authority.
    /// - **A failure BURNS the id** (finding 8's pinning): `mint()` advances
    ///   `next` before the factory runs, so a failed `Start` leaves `Roots`
    ///   unchanged but does not hand the same id back to the next `Start`. Reuse
    ///   would be tempting (`Failed { error }` carries no id, so no client ever
    ///   saw it) and it is refused anyway: `next`'s documented contract is
    ///   "monotone, never reused", and a reviewer who wants reuse has to change
    ///   that sentence, not silently rely on it.
    /// - **Serialised by construction** while this takes `&mut self`: the borrow is
    ///   held for the (synchronous) build, so two `start`s cannot interleave on one
    ///   `Roots`. That is what makes the id-less `Failed` reply safe —
    ///   `Failed { error }` names no conversation, so only one build may be in
    ///   flight at a time.
    pub fn start(&mut self, factory: &Factories, spec: RootSpec) -> Result<ConversationId, RootError> {
        todo!()
    }

    /// The `Stop` verb — [`Roots::close`]'s alias, so both hosts speak the same
    /// vocabulary as the channel. Returns `false` when the id is unknown.
    pub fn stop(&mut self, id: ConversationId) -> bool {
        todo!()
    }

    /// Mint the next id — **the single id authority** (first-layer finding 8).
    ///
    /// # Contract
    /// - `Roots::new` starts `next` at **1**, reserving `ConversationId::first()`
    ///   (seq `0`) for the startup conversation. That root is built *outside*
    ///   `start` — it exists before the client runs (`Root::adopt(RootParts)` from
    ///   `dispatch`) — so `insert` must accept it without `mint` ever returning
    ///   `0`. Returning `0` from both places was the double-booking: the startup
    ///   root took `c0` without advancing `next`, and the first `/tab new`
    ///   minted `c0` again.
    /// - Monotone and **never reused**, so a closed id can never be confused with
    ///   a new root (`a_failed_build_inserts_nothing_and_burns_the_id`).
    /// - Never returns an id already occupied: `insert` refuses one, and this
    ///   cannot produce one.
    pub fn mint(&mut self) -> ConversationId {
        todo!()
    }

    /// Adopt an already-built root. `Err` on a duplicate id or a `Resume` dir
    /// already owned by a live root (`contains_session_path`).
    ///
    /// # Contract
    /// - **Refuses an occupied id** — the startup root legitimately arrives with
    ///   `ConversationId::first()` (the reserved one), a freshly built one with
    ///   whatever `mint` handed out; anything else is a bug, and
    ///   `debug_assert!(root.id.seq() < self.next)` states it.
    /// - Insertion order IS the client's tab order, so `insert` never sorts.
    pub fn insert(&mut self, root: Root) -> Result<(), RootError> {
        todo!()
    }

    /// The root for `id`.
    pub fn get(&self, id: ConversationId) -> Option<&Root> {
        todo!()
    }

    /// Mutable access — `run_host` only (the state mirror).
    pub fn get_mut(&mut self, id: ConversationId) -> Option<&mut Root> {
        todo!()
    }

    /// Every live root, creation order.
    pub fn iter(&self) -> impl Iterator<Item = &Root> {
        todo!()
    }

    /// How many live roots. `1` at startup, always ≥ 0.
    pub fn len(&self) -> usize {
        todo!()
    }

    /// Never true after `Roots::new` + one `insert`; present for the
    /// `clippy::len_without_is_empty` gate.
    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// The live root whose group dir is `dir` — the `Resume`-twice guard.
    ///
    /// # Why refuse
    /// Two roots over one group dir means two writers to one `root.jsonl`
    /// (`session_groups.rs:12` `root.jsonl  # the root's transcript`), i.e. an
    /// interleaved transcript on resume. §13 does not name this case; refusing is
    /// the only reading consistent with "a conversation is an independent root".
    pub fn contains_session_path(&self, dir: &Path) -> bool {
        todo!()
    }

    /// Take the root out **without** tearing it down — for a caller that needs
    /// the pieces (the socket server, a future `wcode-serve` handler). The caller
    /// then owns the teardown obligation.
    pub fn remove(&mut self, id: ConversationId) -> Option<Root> {
        todo!()
    }

    /// The liveness the host reports for `id`.
    pub fn state_of(&self, id: ConversationId) -> Option<RootState> {
        todo!()
    }

    /// Mirror a client-derived state (the TUI's `Failed`) so a later client
    /// reads it back. Never downgrades `Dead`.
    pub fn set_state(&mut self, id: ConversationId, state: RootState) -> bool {
        todo!()
    }

    // ======================================================================
    // §4 — teardown (§13.2 items 3–4)
    // ======================================================================

    /// **Close one conversation.** `true` if a root was removed.
    ///
    /// **Cancel-then-abandon.** It is `Request::Cancel` best-effort, the
    /// orchestrator token cancelled, the `bg` scope torn down, every registry
    /// entry retired, and the handles released — **idempotent (an unknown id is a
    /// no-op), never awaited, and the group dir left on disk**. It does not
    /// interrupt a turn and does not wait for one; the reducer's refusal while
    /// any surface is `Running` is what keeps that abandonment away from a live
    /// turn (see step 3).
    ///
    /// # Contract — the ORDER is load-bearing, and each step says why
    ///
    /// 1. **Mark closing** (`closing.store(true)`): the supervisor will see its
    ///    `JoinHandle` resolve and MUST stay silent, or a `/tab close` reports
    ///    `Died` after `Stopped` and the client paints a closed tab `Failed`.
    /// 2. **Cancel the orchestrator's token** (`agents.rs:651`
    ///    `pub fn cancel_token(&self)`): stops the DAG scheduler and any in-flight
    ///    `Script` node. A no-op without `--agents`. This is the only "stop" that
    ///    is not merely a signal — the fleet's workers observe it.
    /// 3. **`Request::Cancel` on the root handle** — best effort. The actor
    ///    services requests one at a time (`actor.rs:294`-region), so this lands
    ///    after an in-flight turn. **The CLIENT refuses a close while ANY surface
    ///    of the conversation is `Running`** — a running MEMBER counts, because
    ///    the tab state never promotes a member — and that refusal is the only
    ///    thing standing between this step and an abandoned live turn. Wording and
    ///    predicate: `wcode_tui::conversations::ConversationState` +
    ///    `request_close_conversation` (first-layer finding 10, TUI half).
    /// 4. **`Background::shutdown_scope(id)`** — the ROOT's tasks *and every
    ///    member's*, which is the whole point of §13.2 item 3: a member builds its
    ///    own `Background` (`agents.rs:399` `let bg = Background::new();`), so
    ///    `root.bg.shutdown()` (`background.rs:590`) would miss them. It also
    ///    calls **`release_notify()`** on every in-scope `Background` it upgrades
    ///    out of `LIVE` — the step that lets the actor END, see step 6.
    /// 5. **`Registry::remove` for every id** — `address`, `surface` (they coincide
    ///    for conversation 0), each member's **address**, and the *session* id when
    ///    it differs. NOTE the two id spaces: `remove` takes ADDRESSES, and
    ///    `surface` is only an address for conversation 0 (invariant 6) — so the
    ///    retirement list is `address` + `members`, and `surface` is included only
    ///    because for `c0` it IS `address` (removing an unregistered id is a
    ///    documented no-op, `registry.rs:202`-region).
    ///    This is not belt-and-braces: `Registry` is `Arc<Inner>` and `register`
    ///    spawns a watcher task holding a `Registry` clone
    ///    (`registry.rs:126` `let registry = self.clone();`), so the `Inner` — and
    ///    with it the `peers` map's `SessionHandle`s — is only released once
    ///    `peers` is emptied by `remove`. Dropping our clones is NOT enough.
    /// 6. **Consume the `Root`** (it is moved in): its `SessionHandle`s drop.
    ///    **And the actors END — but only because of step 4's
    ///    `release_notify()`.** The earlier claim here was that the actors end
    ///    when both the host's and the client's handles are gone; that is FALSE
    ///    and was first-layer finding 5. The real graph is a CYCLE:
    ///
    ///    ```text
    ///      actor task ─owns─► Agent ─owns─► tools ─hold─► Arc<Background>
    ///        ▲                                                  │
    ///        └── inbox_tx ◄── SessionHandle ◄── notify: OnceLock ┘
    ///    ```
    ///
    ///    `background.rs:481` `    notify: OnceLock<SessionHandle>,` is set by
    ///    `:435` `let _ = self.notify.set(handle);` (`bg.bind`, called from
    ///    `repl.rs:395` and `agents.rs:404`-region), so `inbox.recv()`
    ///    (`actor.rs:296`-region) never returns `None` and the parked actor task — plus
    ///    its `Agent`, its tool set, its `Arc<Background>` and ONE
    ///    registry-watcher task per registered peer (each holding a `Registry`
    ///    clone) — outlives the close. `release_notify()` breaks the cycle at the
    ///    `notify` end; that is the line that makes this step true.
    ///    Then the client's `remove_conversation` drops its `Backend`s, which
    ///    releases the last handles — the two halves of a teardown.
    ///
    /// # What close does NOT do
    ///
    /// - **It never deletes files.** `<group>/root.jsonl`, `members/`,
    ///   `manifest.json`, `plan.ndjson` (`session_groups.rs:9`-region) all stay,
    ///   so `/resume` finds the conversation again
    ///   (`a_closed_roots_files_remain_on_disk`).
    /// - It does not print, does not `exit`, and does not re-exec — the three
    ///   things the `pending_*` exit path does today.
    /// - It does not wait for the actor to finish dying. `release_notify` makes
    ///   the death *happen*; `closing_a_conversation_ends_its_actor_task` asserts
    ///   the supervisor's `JoinHandle` resolves within N yields, not that close
    ///   blocked on it.
    pub fn close(&mut self, id: ConversationId) -> bool {
        todo!()
    }

    /// Close every root — the process-exit path, called where `main.rs` calls
    /// `Background::shutdown_all()` today (`:1968`-region and 9 other sites).
    pub fn close_all(&mut self) {
        todo!()
    }
}

// ==========================================================================
// §5 — supervision (§13.2 item 4)
// ==========================================================================

/// **What a host must react to.** Emitted by the supervisor task, never by a
/// client-facing type — deliberately, so that a second host COULD consume the
/// same stream without a translation layer. Candidate reuse only: finding 6 is
/// open (`W040` §13.5.2), and the TUI is the only consumer today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootEvent {
    /// The root's actor task resolved. `reason` is a short human string
    /// (`"actor ended"`, `"panicked"`), never a backtrace.
    Died {
        id: ConversationId,
        reason: &'static str,
    },
}

/// Watch one root's actor task; report [`RootEvent::Died`] unless we are closing
/// it ourselves.
///
/// # Contract
///
/// - **Never unwinds the process.** A panicking actor is caught as
///   `JoinError::is_panic()` and reported as `reason: "panicked"`. This is new
///   ground (§13.2 item 4: "R3 catches a panicking *tool*; a panicking *actor* is
///   new ground").
/// - **Silent during a close**: `closing` is checked first, so step 1 of
///   [`Roots::close`] suppresses the report.
/// - **Why a `JoinHandle` and not a liveness probe**: the handle holds clones of
///   BOTH channel senders (`actor.rs:279`-region `SessionHandle { inbox, events,
///   state }`), so neither `subscribe()` closing nor `state_rx().changed()`
///   can observe the actor's death; only `inbox.send()` failing can
///   (`actor.rs:161` `pub fn send(&self, request: Request) -> Result<(), SessionClosed>`),
///   and probing means sending real requests — "the harness `JoinHandle` gap".
/// - The task ends after one report (the root is sticky `Dead`).
fn spawn_supervisor(
    id: ConversationId,
    join: JoinHandle<()>,
    closing: Arc<AtomicBool>,
    events: mpsc::UnboundedSender<RootEvent>,
) -> JoinHandle<()> {
    todo!()
}

// ==========================================================================
// §3 — the factory (§13.2 item 2)
// ==========================================================================

/// **The injected build function's type** — the one thing a client cannot do for
/// itself, so the composition root injects it exactly as it injects
/// `DefineHandler`.
///
/// Declared as `Arc<dyn Fn(..) -> Result<..> + Send + Sync>` and **synchronous**,
/// matching `crates/wcode-protocol/src/server.rs:68` `pub type DefineHandler =`
/// (`Arc<dyn Fn(DefineArgs) -> Result<SessionId, String> + Send + Sync>`) — the
/// precedent §13.2 item 2 names. A `BoxFuture`/`async fn` would add a `'static`
/// future bound and an `.await` seam for no benefit: every step the build needs
/// is synchronous (`session_groups::create_group` is `io::Result`,
/// `repl::build_agent` is a plain `fn`, and `SessionActor::spawn` internally
/// `tokio::spawn`s). One `Arc` because more than one holder is plausible (the
/// TUI's task today; a second host's request handlers under finding 6's open
/// ruling — not asserted).
///
/// # Why the id is an ARGUMENT (first-layer finding 7)
/// The id is minted by the host ([`Roots::mint`]) because it is the host's
/// uniqueness authority — but [`build_root`] cannot produce a `Root` without it:
/// it stamps `Root.id`, and [`mint_surface_id`] derives every surface id from it.
/// If the factory did not receive it, the closure would have to invent one, and
/// two authorities minting ids is how a `c0` gets reused. So the signature is
/// `Fn(ConversationId, RootSpec)`; the closure is pure in it (no state), which
/// keeps it `Send + Sync` and `Fn` rather than `FnMut`.
pub type Factories = Arc<dyn Fn(ConversationId, RootSpec) -> Result<Root, RootError> + Send + Sync>;

/// Everything a `Fresh`/`Resume` build needs that does **not** come from the
/// `RootSpec` — i.e. the process's own resolved configuration, cloned once so the
/// closure is `'static`.
///
/// # ONE struct, not two (first-layer finding 9)
/// `RootInputs` is **deleted**. The type the factory captures IS
/// [`crate::repl::RunInputs`] — the same struct `repl::run` takes — defined once
/// in `repl.rs`, beside the function whose parameter list it names. The
/// `repl.rs:807`-region block is authoritative for its field list; this module
/// only *consumes* it, so the two cannot drift into "the factory's inputs" and
/// "the REPL's inputs" again. It is **not** a wrapper: a wrapper would give one
/// set of values two names and reintroduce exactly the drift the finding named.
///
/// The fields the build reads, and where each comes from today:
///
/// | field | read by | today |
/// |---|---|---|
/// | `llm` | `build_agent` | `AgentSpec.llm` (`repl.rs:376`) |
/// | `hooks` / `tools` / `instructions` / `skills` / `team` / `guidelines` | `build_agent` → `system_prompt` | `AgentSpec` (`repl.rs:377`-`:386`) |
/// | `compaction` | `build_agent` | `AgentSpec.compaction` — **missing from the old `RootInputs`** |
/// | `title_after_turns` | `build_agent` | `AgentSpec.title_after_turns` — **missing** |
/// | `overlay` / `no_project_config` / `owner` / `name` | `load_session`, `build_agent_for` | `main.rs:1750`-region (`--owner`), `:1783`-region (`--title`) — **missing** |
/// | `orchestrator` | `[team]` spawn, `register_root` | `Runtime.orchestrator` |
/// | `cfg` / `args` | `build_runtime`, `build_agent_for` | `&Config` / `&Args` |
/// | `cwd` / `session_dir` | `create_group`, `system_prompt` | `SessionSetup.cwd`, `repl::session_dir()` |
///
/// # `Args` must gain `Clone` — and `pub`ness (finding 9)
/// `pub args: crate::Args` was a `private_interfaces` warning: `Args`
/// (`main.rs:145`-`146` `#[derive(Debug, Default, PartialEq)]` / `struct Args {`)
/// is neither `pub` nor `Clone`. Both are real-code edits, both mechanical, both
/// **in `main.rs`** (see that file's block (F)): `#[derive(Clone)] pub(crate)
/// struct Args`. Because `RunInputs` then holds a `pub(crate)` type, `RunInputs`
/// itself is `pub(crate)` and so is [`factory_for`] — a `pub fn` exposing a
/// `pub(crate)` argument re-raises the same lint.
///
/// # The three `SessionSetup` fields are DERIVED, not carried (finding 9's edge)
/// The finding lists `SessionSetup`'s `session` / `context` / `resuming_group`
/// among the missing inputs. They are needed by the build, and they must NOT go
/// in this struct, for a reason that is typing, not taste:
///
/// - `session: Option<Session>` is a **file handle** and `context:
///   Vec<AgentMessage>` is **conversation state** — both owned, single-use, and
///   neither `Clone` nor shareable. A `RunInputs` that carried them would be
///   captured ONCE by the closure, so **every `/tab new` would re-open the
///   startup conversation's transcript**. That is a data-loss bug, not an
///   inconvenience.
/// - They are exactly what `RootSpec` already encodes: `Fresh` ⇒ create a group
///   and an empty context; `Resume { dir }` ⇒ `open_group` + `load_root`
///   (`session_groups.rs:290`, `:658`). `resuming_group` is likewise
///   `matches!(spec, RootSpec::Resume { .. })`.
/// - The STARTUP root, whose `Session`/context `load_session` (`main.rs:1286`)
///   already produced, does not go through the factory at all: it is
///   [`Root::adopt`]ed from [`RootParts`], which is where those owned values are
///   consumed (`build_agent_for`, `main.rs:1737`).
///
/// So the factory's per-build input is `(ConversationId, RootSpec)` and nothing
/// else — the process-level set plus the spec. If a reviewer wants them in the
/// struct anyway, the answer is a second, per-call parameter (a `RootSeed`),
/// never a field: a field would be captured.
///
/// **NOT DEFINED HERE.** The one definition is
/// `crate::repl::RunInputs`, beside `repl::run` (`repl.rs:807`–`:920` block,
/// authoritative for the field list) — this module imports it. Never re-declare
/// it in this file: two declarations of one input set is the drift finding 9
/// named.

/// Build the closure the host calls. Pure: it captures `inputs` and returns; the
/// building happens on call.
///
/// `pub(crate)`, not `pub`: its parameter is a `pub(crate)` type (see
/// `RunInputs`'s `Args` note — one `private_interfaces` warning is enough).
///
/// # Contract
/// - The returned `Fn` takes the already-minted `ConversationId` and never mints
///   one (finding 7: `Roots::mint` is the only authority).
/// - **One build at a time.** The returned `Fn` is not reentrant-safe w.r.t. the
///   ambient `Background` scope (`background.rs` block, `with_scope`), and the
///   client's `Failed` reply carries no id, so `run_host` must
///   serialise `Start`s. Documented here because the type cannot enforce it.
/// - The closure never `exit`s and never prints (see [`build_root`]).
pub(crate) fn factory_for(inputs: RunInputs) -> Factories {
    todo!()
}

/// **Build one root** — `RootSpec::Fresh` or `RootSpec::Resume`, from nothing, in
/// one synchronous pass.
///
/// # Contract
///
/// 1. **Session first.** `Fresh` → `session_groups::create_group(&inputs.session_dir)`
///    (`:164`) then `create_root_session(&group.dir, &inputs.cwd)` (`:181`);
///    `Resume { dir }` → validate with `is_group_dir_name` (`:281`) then
///    `open_group(&dir)` (`:290`) + `load_root(&group)` (`:658`); `--no-session`
///    → neither, and `session_path` is `None`. Distinct dirs are guaranteed by
///    the fs, not by luck: `fresh_group_name` is `{millis}_{subsec_nanos:08x}`
///    and `create_group` uses `fs::create_dir` (`:167`), which fails on a
///    collision rather than merging
///    (`fresh_mints_a_distinct_group_dir_and_resume_reuses_one`).
/// 2. **The LLM is per-root.** A `Resume` restores the session's model/effort
///    (`main.rs:1322`-region) unless `--model`/`--effort` were passed, so
///    `inputs.llm` is cloned + mutated here, never shared.
/// 3. **The runtime**, via a NON-EXITING refactor of `build_runtime`
///    (`main.rs:1462`): it builds `instructions`/`skills` (already in
///    `RunInputs`), the orchestrator, spawns `[team]` and the DAG scheduler, and
///    instantiates a `[workflow]`. **Today it `exit(2)`s on 8 config errors and
///    `eprintln!`s the `team:`/`workflow:` lines** (`:1555`, `:1559`, `:1569`,
///    `:1579`, `:1586`, `:1599`, `:1663`, `:1669`, `:1685`) — inside a running TUI
///    that kills every other conversation — "the exiting, printing factory
///    body"; the factory needs the
///    `Result` form.
/// 4. **The agent**, via the existing builder (`repl.rs:388` `pub fn build_agent(`)
///    with the `AgentSpec` (`repl.rs:374`) the rebuild set already uses, plus
///    `extra_tools` from `o.tools()` when `--agents`. `build_agent_for`
///    (`main.rs:1737`) is the same thing with the `--owner`/`--title` branches;
///    the factory needs the non-exiting form of those two lines.
/// 5. **Spawn + bind**, in today's order and with today's meaning:
///    `SessionActor::spawn_supervised(agent)` — **added by this item, not
///    present today** (invariant 4; two lines of harness, `spawn` untouched),
///    `bg.bind(handle.clone())` (`main.rs:2324`), then
///    `o.register_root(handle.clone())` (`main.rs:2326`, `agents.rs:872`), then
///    `record_root_meta(&registry, &session_id, llm)` (`main.rs:1915` — note it
///    keys on the **session** id, not `address`).
/// 6. **The roster**: `live_roster(&registry, address)` (`main.rs:1930`).
/// 7. **The member order** (invariant 5): the `[team]` loop or `scan_members`,
///    exactly as `main.rs:2348`-region does, so N = 1 is byte-identical.
/// 8. **The surface ids**: [`mint_surface_id`] applied to the root (`address`)
///    **and to every member** — `mint_surface_id(id, &SessionId::agent(&name))`.
///    Not the root only: every root's worker is `agent:w1` (the uniqueness check
///    is per-orchestrator, `agents.rs:339`), so an unminted member id collides
///    with the other conversation's `w1` the moment N = 2 ("the one flat
///    surface-id key").
///    Conversation 0 is unchanged, so N = 1 stays byte-identical.
/// 9. **The supervisor** (§5).
///
/// # Errors
/// [`RootError`]. Never `exit`, never a stdout write, never a panic on a
/// user-supplied path.
pub(crate) fn build_root(
    inputs: &RunInputs,
    id: ConversationId,
    spec: RootSpec,
) -> Result<Root, RootError> {
    todo!()
}

/// **The surface id for conversation `id`** — the F1 resolution, applied to the
/// ROOT **and** every MEMBER (finding 3).
///
/// # Contract
/// - `id == ConversationId::first()` ⇒ `base` **unchanged**, so conversation 0's
///   surface ids are `agent:orchestrator`/`agent:root`/`agent:w1` exactly as
///   today and `App::focused_id()`'s value never moves (byte-identical N = 1).
/// - Otherwise `<base>@c<seq>`: distinct, loggable, and reversible back to
///   `base`. It is a **client-side label**; the A2A address stays `base`
///   (`Root.address`, `Root.members`), so nothing on a wire changes.
/// - **Call it per surface, not once per root.** The root alone is not enough:
///   every root's worker is `agent:w1` (`agents.rs:339`), so a root-only mint
///   leaves the two `w1`s colliding at N = 2.
/// - Why mint at all: the client routes events with a FLAT `SessionId` map
///   (`AppEvent::Agent(SessionId, …)`, `crates/wcode-tui/src/app.rs:97`
///   `    Agent(SessionId, AgentEvent),`), and two roots both answer to
///   `agent:orchestrator` / `agent:w1`. Two conversations with the same surface id
///   would cross-deliver every event — "the one flat surface-id key".
///
/// # Not a name
/// The result is for routing only: never `Registry::resolve`d, never
/// `short_name`d for a label (invariant 6).
pub fn mint_surface_id(id: ConversationId, base: &SessionId) -> SessionId {
    todo!()
}

// ==========================================================================
// §6 — the reverse channel (the host loop, §13.2 item 8)
// ==========================================================================

/// The three channels one host owns — the two the TUI already has
/// (`new_surfaces`, `new_tasks`), plus the reverse pair from
/// `wcode_tui::conversations::ConversationFeeds`.
///
/// # The exact types the sketched TUI expects (verified against the files on disk)
///
/// | direction | type | where |
/// |---|---|---|
/// | client → host | `mpsc::UnboundedSender<wcode_tui::conversations::ConversationRequest>` | `ConversationFeeds::outbound` |
/// | host → client | `mpsc::UnboundedReceiver<wcode_tui::conversations::ConversationEvent>` | `ConversationFeeds::incoming` |
/// | host → client (members) | `mpsc::UnboundedSender<(ConversationId, SurfaceSpec)>` | the tagged `new_surfaces` |
///
/// `ConversationRequest` = `Start(RootSpec) | Stop(ConversationId)` and
/// `ConversationEvent` = `Added(ConversationSpec) | Failed{error: String} |
/// Stopped{id} | Died{id}` — **both are `wcode_tui` types, so this struct and
/// [`ConversationRequest`] above are the entire TUI edge of §§1–7** (§§1–5 name
/// none). That edge is the open finding-6 question: a second host could not use
/// [`HostChannels`] as written and would need its own translation, while the
/// handlers behind it (`Roots::start`/`Roots::stop`) are the part that might
/// carry over. Candidate reuse, pending `W040` §13.5.2 — not a claim.
///
/// # THREE mismatches with the code on disk (reported, not papered over)
///
/// 1. **The member feed does not accept the tag yet.** `wcode_tui::run`'s
///    parameter is still `new_surfaces: Option<mpsc::UnboundedReceiver<SurfaceSpec>>`
///    (`crates/wcode-tui/src/lib.rs:378`), while
///    `crates/wcode-tui/src/conversations.rs`'s module doc specifies
///    `(ConversationId, SurfaceSpec)`. The CLI must send the pair (an untagged
///    member cannot be placed in a tab), so **the TUI needs that one signature
///    change** — either the element type or a fifth `run` parameter. Until then
///    `install_surface_sink` can only be written against the untagged channel and
///    a runtime-spawned member of a non-active root lands on the wrong tab.
/// 2. **`Options.conversations` does not exist yet.** The CLI is supposed to hand
///    the startup list over (`Options`, `crates/wcode-tui/src/lib.rs:284`); that
///    field is specified in the TUI sketch's module doc and absent from `lib.rs`
///    today. Without it the client derives exactly one conversation, so **N = 1
///    works and N = 2 does not** until the field lands.
/// 3. **`Failed { error }` carries no id.** Benign while exactly one build is in
///    flight (which [`run_host`] enforces), a mis-attribution the moment that
///    becomes two. Recorded, not relied on: the host does not depend on it.
pub struct HostChannels {
    /// `Start`/`Stop` from the client.
    pub requests: mpsc::UnboundedReceiver<wcode_tui::conversations::ConversationRequest>,
    /// `Added`/`Failed`/`Stopped`/`Died` to the client.
    pub events: mpsc::UnboundedSender<wcode_tui::conversations::ConversationEvent>,
    /// Members spawned at runtime, tagged with their conversation.
    pub surfaces: mpsc::UnboundedSender<(ConversationId, wcode_tui::SurfaceSpec)>,
    /// The supervisor end. `Roots::new` gets the matching SENDER, so the
    /// receiver must be handed to the loop that selects on it — this field.
    /// Splitting the pair is deliberate: `Roots` owns the sink (so no root can be
    /// inserted without one) while the loop owns the wake-up.
    pub root_events: mpsc::UnboundedReceiver<RootEvent>,
}

/// **Service the reverse channel until the client goes away.** The whole of
/// §13.2 item 8's "a reverse channel" is this loop.
///
/// # Contract
///
/// - `Start(spec)` → `roots.start(&factory, spec)` (mint + build + insert —
///   the verb any other host would call) →
///   `events.send(Added(ConversationSpec { id, label, surfaces }))`.
///   **If that `send` fails** (the client quit while we were building), the root
///   is closed immediately — otherwise a `/tab new` + instant `Ctrl-C` leaks an
///   actor, a `bg` scope and a session file. That obligation is spelled out at
///   `crates/wcode-tui/src/app.rs:190`-region.
/// - `Stop(id)` → `roots.close(id)` → `events.send(Stopped { id })`.
///   `Stopped` is sent even for an unknown id, so a client that optimistically
///   removed the tab cannot hang (it does not — it waits for the echo).
/// - `RootEvent::Died` → `roots.set_state(id, Dead)` then `events.send(Died { id })`.
/// - The loop ends when `requests` closes; the function returns [`Roots`] so the
///   caller can [`Roots::close_all`].
/// - **Builds are serialised** (one `Start` in flight) because a `Failed` reply
///   carries no id.
/// - A blocking build inside the `select!` is accepted: creating a group and an
///   agent is milliseconds and touches no network. `spawn_blocking` is NOT used —
///   it would move `build_runtime`'s `tokio::spawn`s off the reactor's task
///   context for no gain.
pub async fn run_host(
    roots: Roots,
    factory: Factories,
    channel: HostChannels,
) -> Roots {
    todo!()
}

/// Install a per-root member sink: one `UnboundedSender<SurfaceSpec>` per
/// orchestrator (`agents.rs:247` `pub fn set_spawn_sink(&self, tx:
/// mpsc::UnboundedSender<SurfaceSpec>)`), forwarded into the tagged channel —
/// **with the member's surface id MINTED on the way through** (finding 3).
///
/// # The forwarder's two rewrites, and why it must do them
/// The spec the orchestrator sends (`agents.rs:378`-region `let _ =
/// tx.send(SurfaceSpec { … })`) carries three things this module must fix:
///
/// 1. **`spec.id`** is the member's A2A address (`SessionId::agent(&name)`,
///    `agents.rs:339`-region), i.e. `agent:w1` in EVERY root. It is replaced with
///    `mint_surface_id(root.id, &spec.id)` — the root's [id](Root::id) is captured
///    by the closure, and it is the only place that knows which conversation the
///    member belongs to. Untouched, two roots' `w1` rows would cross-deliver.
/// 2. **The pair's tag** — `(ConversationId, SurfaceSpec)` — because
///    `SurfaceSpec::info()` (`crates/wcode-tui/src/lib.rs:268`-region) drops it at
///    `lib.rs:581` (`app.add_surface(spec.info());`).
///
/// # Why a wrapper task instead of changing `set_spawn_sink`'s element type
/// The sink's payload type is `wcode_tui::SurfaceSpec`, and the client's feed
/// wants `(ConversationId, SurfaceSpec)`. Two ways to bridge:
/// (a) change `set_spawn_sink` + `register_and_announce` to send the pair (and to
///     mint there) — touches `crates/wcode-cli/src/agents.rs` in 3 places;
/// (b) this small forwarder, one per root, capturing the id.
/// (b) is chosen: it keeps `agents.rs` untouched, it composes with a root that
/// has no sink, and it is the only place that knows which conversation a member
/// belongs to — the orchestrator is per-root, so the tag is injected at the
/// source rather than guessed by the client. Route (a) — changing the sink's
/// element type — is recorded as "the untagged member sink", and the
/// cost it would avoid (the forwarder's id rewrite is a **second** site that must
/// agree with [`Root::surface_specs`]'s, so both call [`mint_surface_id`] and
/// neither may inline the rule).
pub fn install_surface_sink(
    root: &Root,
    out: mpsc::UnboundedSender<(ConversationId, wcode_tui::SurfaceSpec)>,
) {
    todo!()
}

// ==========================================================================
// §7 — the TUI adapter (the ONLY TUI-shaped code in this file)
// ==========================================================================

impl Root {
    /// The client's view-model row: id, label, root surface, members, state.
    ///
    /// `is_remote` is always `false` here — a locally built root is `Backend::Local`
    /// by §13.2 item 1; the socket client's single conversation is built by
    /// `main.rs:1155`-region and never comes through this module.
    pub fn conversation(&self) -> wcode_tui::conversations::Conversation {
        todo!()
    }

    /// The client's attachments for this root: the root surface first
    /// (`is_root: true`), then `members` in invariant-5 order. Labels follow
    /// today's rule — the root reads `wcode` (`main.rs:2335`-region), a member
    /// reads `crate::agents::short_name` (`agents.rs:85`).
    ///
    /// # Every `id` is MINTED (invariant 6, finding 3)
    /// `id: mint_surface_id(self.id, &self.address)` for the root, and
    /// `mint_surface_id(self.id, &SessionId::agent(&name))` for each member —
    /// the ADDRESS is `members`' entry, the ID is the label. `short_name` is
    /// applied to the ADDRESS for the label, never to the id (an id may be
    /// `agent:w1@c2`, which would render as `w1@c2` in the sidebar and diverge
    /// from the roster).
    ///
    /// # Errors
    /// `RootError::AlreadyLive` is not reachable here; the `Result` exists so a
    /// member that vanished between build and call is reported instead of
    /// silently dropped — a surface the client never learns about is a backend
    /// it can never address.
    pub fn surface_specs(&self) -> Result<Vec<wcode_tui::SurfaceSpec>, RootError> {
        todo!()
    }

    /// The `serve`-shaped view of this root — **no TUI type**, so a second host
    /// could pass it straight to `serve_at`/`serve_stdio` (candidate reuse,
    /// finding 6 open): the root tuple those take
    /// (`crates/wcode-protocol/src/server.rs:41` `pub type Root = (SessionId,
    /// SessionHandle);`) plus the live roster.
    pub fn as_protocol_root(&self) -> (wcode_protocol::Root, Roster) {
        todo!()
    }
}

// ==========================================================================
// §8 — tests: one per invariant named in the brief
// ==========================================================================

#[cfg(test)]
mod tests {
    //! Behaviour-named stubs. A `todo!()` here is an assertion the developer
    //! must make true; none may be deleted to make the suite green.

    use super::*;

    /// A `RunInputs` whose builds never touch the network: the refused-port
    /// endpoint trick (`AGENTS.md` "Gotchas" — `http://127.0.0.1:9/v1`), the
    /// session dir redirected to a `tempdir`.
    fn inputs(dir: &Path) -> RunInputs {
        todo!()
    }

    #[test]
    fn two_roots_do_not_share_a_registry_or_roster() {
        // Build c0 and c1; assert `!Arc::ptr_eq(&a.registry.inner, &b.registry.inner)`
        // (via a public probe: registering `agent:x` in c0 leaves c1's
        // `contains(&agent:x)` false) and that c1's roster never lists c0's ids.
        // §13.2 item 1: each conversation is an INDEPENDENT root runtime.
        todo!()
    }

    #[test]
    fn closing_c2_kills_only_c2_background_tasks() {
        // Spawn a real `bash { background: true }` in each of two roots (or
        // register two fake `Background`s under two scopes), `Roots::close(c2)`,
        // then assert c2's entries are `Killed` and c0's still `Running`.
        // Then `Roots::close(c0)` for symmetry. This is §13.2 item 3's gate:
        // `shutdown_scope`, not `shutdown_all`.
        todo!()
    }

    #[test]
    fn a_root_whose_actor_task_dies_reports_failed_and_leaves_the_others_live() {
        // Build c0 + c1; abort c1's supervisor-owned actor by dropping its
        // `SessionHandle` and `tokio::task::yield_now()`; assert exactly ONE
        // `RootEvent::Died { id: c1 }` arrives, `roots.state_of(c1) == Dead`,
        // `roots.state_of(c0) == Live`, and c0 still answers a `Request::GetHistory`.
        // §13.2 item 4: "its exit marks ConversationState::Failed, never unwinds
        // the process".
        todo!()
    }

    #[test]
    fn every_members_surface_id_is_minted_not_its_a2a_address() {
        // FINDING 3's gate. Build a two-member `[team]` in c0 and in c1; assert
        //  - `c1.surface_specs()`' member ids are `agent:w1@c1`/`agent:w2@c1`, i.e.
        //    `mint_surface_id(c1.id, &SessionId::agent("w1"))`, and that NO two
        //    entries across the two roots share an id (the flat-map collision);
        //  - `c0`'s are the bare `agent:w1`/`agent:w2` (N = 1 byte-identical);
        //  - `Root.members` still holds the A2A ADDRESSES (`agent:w1`), so
        //    `registry.contains(&member)` is true while
        //    `registry.contains(&spec.id)` is FALSE for c1 — the invariant-6
        //    separation, asserted rather than described;
        //  - the LABEL is `short_name(address)`, never `short_name(id)` (no
        //    `w1@c1` in the sidebar);
        //  - a runtime spawn through `install_surface_sink` lands in the same space:
        //    the forwarded spec's id is minted and its tag is `c1`.
        todo!()
    }

    #[test]
    fn closing_a_conversation_ends_its_actor_task() {
        // FINDING 5's gate — the leak, not the kill. Build a root, spawn a
        // `bash { background: true }`, `Roots::close(id)`, then poll
        // (`tokio::task::yield_now` × N, N small and fixed — never `sleep`) until
        // the supervisor's `JoinHandle` resolves; assert it does. Without
        // `release_notify()` it never does: `notify: Mutex<Option<SessionHandle>>`
        // keeps `inbox_tx` alive through
        // `Agent → tools → Arc<Background> → notify`, so `inbox.recv()` parks
        // forever and every `Registry` watcher for that root leaks with it.
        // Assert also that the number of live `Registry` watcher tasks returns to
        // the pre-build count (the one-task-per-peer consequence).
        todo!()
    }

    #[test]
    fn a_close_does_not_report_death() {
        // The same death, but via `Roots::close(c1)`: assert NO `RootEvent::Died`
        // reaches the channel (step 1's `closing` flag) — otherwise the client
        // paints a tab it just closed as `Failed`.
        todo!()
    }

    #[test]
    fn fresh_mints_a_distinct_group_dir_and_resume_reuses_one() {
        // Two `Fresh` builds under one tempdir → two distinct `group.dir`s, both
        // matching `session_groups::is_group_dir_name`;
        // `build_root(…, RootSpec::Resume { dir })` on the first → the SAME dir,
        // and `contains_session_path` refuses a second resume of it.
        todo!()
    }

    #[test]
    fn a_closed_roots_files_remain_on_disk() {
        // Build a session-backed root, close it, then assert
        // `<group>/root.jsonl` + `<group>/members/` still exist and
        // `session_groups::open_group` still opens the dir.
        todo!()
    }

    #[test]
    fn conversation_zero_keeps_todays_surface_id() {
        // `mint_surface_id(ConversationId::first(), &SessionId::agent("orchestrator"))`
        // == `agent:orchestrator`, and a seq-1 id yields a DIFFERENT string that
        // still ends with the base. N = 1 byte-identical.
        todo!()
    }

    #[test]
    fn a_failed_build_inserts_nothing_and_burns_the_id() {
        // FINDING 8's gate, PINNED (the stub used to defer the choice).
        // `Roots::new(..)` + a factory returning `Err` for the first `start`:
        //  - `len() == 0` (nothing half-inserted) and no id is occupied;
        //  - `next` advanced, so the next successful `start` returns `c2`, NOT the
        //    `c1` the failure consumed — "monotone, never reused" is the rule, and
        //    reuse would need that sentence changed first.
        // And the RESERVATION half: `Roots::new` must never hand out
        // `ConversationId::first()`; the first `mint()` is `c1`, so a startup root
        // inserted with `ConversationId::first()` and the first `/tab new` cannot
        // collide (the double-booking the finding named).
        todo!()
    }

    #[test]
    fn a_failed_send_tears_the_new_root_down() {
        // Drop the client's `events` receiver before `run_host` handles a
        // `Start`; assert `roots.len()` is unchanged afterwards and the built
        // root's `bg` scope was shut down (no leak on "new tab then quit").
        todo!()
    }

    #[test]
    fn the_request_and_event_vocabulary_round_trips() {
        // `ConversationRequest::Start(RootSpec::Fresh { label: None })` and
        // `Stop(id)` survive a channel hop; `ConversationEvent::Added(…)` carries
        // a `ConversationSpec` whose `surfaces[0].is_root` is true and whose
        // `surfaces[0].id` equals `mint_surface_id(id, &address)`.
        // This is the test that FAILS if the CLI and the TUI sketches drift.
        todo!()
    }
}
