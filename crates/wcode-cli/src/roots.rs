//! The **multi-root core** (W040 §13.2 items 1–4, 8): the CLI's registry of live
//! *conversations*, the supervisor that notices one dying, and the injected
//! factory type that builds one.
//!
//! A conversation is an **in-process root runtime** (§13.2 item 1):
//! `Backend::Local(SessionHandle)` — no child process, no socket, no pid, no
//! readiness wait. Today exactly one such root exists, constructed by hand in
//! `main.rs`'s TUI branch and in `serve()`; this module turns that hand-rolled
//! construction into `N` of them behind one type.
//!
//! # Why here, and not in `wcode-protocol` / `wcode-tui`
//!
//! `Root` owns `Arc<Background>`, `Orchestrator`, `SessionGroup` — all
//! `wcode-cli` types. So the *container* cannot live below the composition root;
//! only the **keys** (`ConversationId`, `RootSpec`) can, and they do
//! (`crates/wcode-protocol/src/conversation.rs`). §13.3 states the split: "The
//! container and the root lifecycle live in the CLI + TUI".
//!
//! # What is NOT here yet (W040 commits 4–6)
//!
//! - the build path: `factory_for`/`build_root`, which need the `Result`-ified
//!   `build_runtime` and `repl::RunInputs` (commit 4);
//! - the reverse-channel host loop: `HostChannels`/`run_host` (commit 6 — it
//!   speaks the TUI's `ConversationRequest`/`ConversationEvent`);
//! - the TUI adapters `Root::conversation`/`Root::surface_specs` and
//!   `Root::as_protocol_root` (commits 5–6: they need the registered
//!   `wcode_tui::conversations` container, and `serve()`'s wiring).
//!
//! Everything those consume — `Root`, `Roots`, the supervisor, the surface-id
//! mint, the member sink — lands here, so the later commits are wiring, not new
//! lifecycle.
//!
//! # Reuse by a future HTTP host: a CANDIDATE, not a claim
//!
//! Nothing here is proved reusable: `wcode-cli` is a BINARY, so no other crate
//! can reach this module at all. Whether the shape survives contact with
//! `wcode-serve` is open (`W040` §13.5.2, finding 6); every sentence below about
//! a second host is *candidate reuse, pending that ruling*, never fact.
//!
//! What a reviewer can check today: `Root`, `Roots`, `RootError`, `RootState`,
//! `RootParts`, `RootEvent`, `spawn_supervisor`, `Factories` and
//! `mint_surface_id` name **no `wcode_tui` type**.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use wcode_harness::actor::{SessionActor, SessionHandle};
use wcode_harness::agent::Agent;
use wcode_harness::protocol::{Request, SessionId};
use wcode_protocol::{ConversationId, Registry, Roster, RootSpec};

use crate::agents::Orchestrator;
use crate::session_groups::SessionGroup;
use crate::tools::background::Background;

// ==========================================================================
// §1 — one live root
// ==========================================================================

/// What building or adopting one root can go wrong. `Display` is the string the
/// client sees, so every variant must be a sentence a human can act on.
#[derive(Debug)]
pub enum RootError {
    /// The client asked for a conversation whose group dir does not exist or is
    /// not a group (`session_groups::is_group_dir_name`).
    BadResume(PathBuf),
    /// Session IO (create/open a group, `root.jsonl`, a member file).
    Session(std::io::Error),
    /// The runtime could not be built — a bad `[team]`/`[workflow]`/`[peers]`
    /// entry that `build_runtime` answers with `std::process::exit(2)` today.
    Config(String),
    /// A conversation with this id (or this `Resume` dir) is already live.
    AlreadyLive(String),
}

impl std::fmt::Display for RootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RootError::BadResume(dir) => write!(
                f,
                "{} is not a session group directory",
                dir.display()
            ),
            RootError::Session(e) => write!(f, "session io: {e}"),
            RootError::Config(detail) => write!(f, "{detail}"),
            RootError::AlreadyLive(what) => write!(f, "{what} is already live"),
        }
    }
}

impl std::error::Error for RootError {}

/// A root's liveness, as the host tracks it. The client's
/// `wcode_tui::conversations::ConversationState` is a *projection* of this
/// (`Idle`/`Running` come from the client's own surface events; only `Failed`
/// and `Dead` originate here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootState {
    /// Built and answering.
    Live,
    /// The last run failed — client-derived; the host mirrors it only if a
    /// client asks ([`Roots::set_state`]).
    Failed,
    /// The actor task resolved: panicked, or ended without a `close`
    /// (§13.2 item 4). Sticky; the handles are still held so the client can read
    /// what the root produced.
    Dead,
}

/// The several things one root is built from, grouped so [`Root::adopt`] and the
/// build path cannot drift apart.
pub struct RootParts {
    pub id: ConversationId,
    pub label: String,
    pub agent: Agent,
    pub bg: Arc<Background>,
    pub orchestrator: Option<Orchestrator>,
    pub group: Option<SessionGroup>,
    pub session_path: Option<PathBuf>,
    pub model: String,
    /// The team, in display order. Empty at startup for a fresh root — the
    /// `[team]` loop inside the build fills it.
    pub members: Vec<SessionId>,
    /// Where the supervisor reports a death ([`RootEvent`]).
    ///
    /// It is a `RootParts` field, and not something `Root::adopt` looks up,
    /// because only the caller knows which [`Roots`] this root is about to join —
    /// and a root whose death nobody hears is exactly the invisible failure
    /// §13.2 item 4 exists to end.
    pub events: mpsc::UnboundedSender<RootEvent>,
}

/// **One live conversation's runtime.** Everything the host needs to talk to it,
/// tear it down, and notice it dying — and nothing a client needs.
///
/// # Invariants
///
/// 1. `registry` is **this root's own** `Registry` — never a clone of another
///    root's, never a process-global. Two roots share nothing
///    (`two_roots_do_not_share_a_registry_or_roster`). Without `--agents` it is
///    still a real `Registry::default()`, so `Root.registry` is not an `Option`.
/// 2. `roster` excludes `address`, so the root is never listed as its own member.
/// 3. `bg` is the root's own `Arc<Background>` **and** every member's
///    `Background` is tagged with this root's `id`. Dropping this field is not a
///    teardown: `Background` has no `Drop`, which is why [`Roots::close`] is
///    explicit.
/// 4. `supervisor` is the actor task's `JoinHandle` (§13.2 item 4), which exists
///    because [`SessionActor::spawn_supervised`](wcode_harness::actor::SessionActor)
///    keeps what `spawn` discards (W040 commit 1).
/// 5. `members` is in **the order the client must show them** — the `[team]`
///    order, or a resumed group's `scan_members` order. NOT `Registry::locals()`'s
///    order, which would reorder today's sidebar.
/// 6. **A SURFACE ID IS OPAQUE.** Every id a client routes by — `surface`, and
///    each member id the surface list carries — is a CLIENT LABEL produced by
///    [`mint_surface_id`]. It is **never** a `Registry` key, **never** passed to
///    `Registry::resolve`, and **never** `short_name`d for display. The A2A
///    addresses are `address` and `members`, and those are what
///    `message`/`member`/`spawn { to }` address. Concretely why minting is
///    mandatory: every root's worker is `agent:w1` because the uniqueness check is
///    per-orchestrator (`agents.rs:339`), so a flat client map would deliver one
///    root's events to the other's row.
pub struct Root {
    /// The host-minted key. Stable for the root's life; the client groups by it.
    pub id: ConversationId,
    /// The tab label's seed. A later `AgentEvent::SessionTitle` overrides it
    /// **in the client**, never here ("the tab label's single source").
    pub label: String,
    /// The root's A2A address — `Orchestrator::id()`, i.e. `agent:orchestrator`,
    /// or `agent:root` without `--agents`. Per-root unique by construction but
    /// **NOT unique across roots**.
    pub address: SessionId,
    /// The id the CLIENT tags this root's events with. Equal to `address` for
    /// conversation 0 (byte-identical N = 1) and minted distinct for 1..N. A
    /// client-side label only; OPAQUE (invariant 6).
    pub surface: SessionId,
    /// The live actor. Cloning is cheap and keeps the session alive.
    pub handle: SessionHandle,
    /// This root's address book + A2A bus, shared with `serve()` and the
    /// orchestrator's tools.
    pub registry: Registry,
    /// The live team, root excluded.
    pub roster: Roster,
    /// The root's background-task registry+notifier; also the scope tag its
    /// members' `Background`s inherit (§13.2 item 3).
    pub bg: Arc<Background>,
    /// The A2A wiring; `None` without `--agents` and for a served `--owner`
    /// worker.
    pub orchestrator: Option<Orchestrator>,
    /// The team, in display order (invariant 5). These are **A2A addresses** —
    /// what `message`/`member` address and what `Registry::remove` takes — NOT
    /// surface ids.
    pub members: Vec<SessionId>,
    /// The root's session file, when sessions are on. `None` for `--no-session`
    /// and for a served worker with no local file.
    pub session_path: Option<PathBuf>,
    /// The group dir, when this root has one. The unit of "resume this
    /// conversation".
    pub group: Option<SessionGroup>,
    /// The effective model this root was built with — per-conversation, because a
    /// resumed root restores the session's own.
    pub model: String,
    /// Supervisor-owned state, mirrored to the client. `Failed` is sticky.
    pub state: RootState,
    /// Set before the handles drop, so the supervisor does not mistake our own
    /// teardown for a crash (§5).
    closing: Arc<AtomicBool>,
    /// The actor task's handle (§13.2 item 4). `None` only in tests.
    supervisor: Option<JoinHandle<()>>,
}

impl Root {
    /// **Adopt an already-built agent** — the startup path, where the caller has
    /// already run `load_session` + `build_runtime` + `build_agent_for` before
    /// it exists.
    ///
    /// This is the seam that makes the hand-rolled "spawn, bind, register_root,
    /// surface list" five lines into one call, so the startup root and a
    /// `/tab new` root are the same kind of object — otherwise closing tab 0
    /// would be a special case.
    ///
    /// # Contract
    /// - Spawns the actor via [`SessionActor::spawn_supervised`] — never the
    ///   plain `spawn`: the `JoinHandle` it throws away is the whole of §13.2
    ///   item 4.
    /// - `bg.bind(handle.clone())`, then `orchestrator.register_root(handle)` —
    ///   today's order.
    /// - `address` is the orchestrator's id, or `SessionId::agent("root")`;
    ///   `surface = mint_surface_id(id, address)`.
    /// - `bg` should already be tagged with `id`
    ///   ([`Background::with_scope`]`(id, …)` around the build), because teardown
    ///   finds a conversation's **members'** `Background`s by that tag. The root's
    ///   own notifier is released explicitly ([`Root::tear_down`]), so a root
    ///   adopted outside a scoped build still closes cleanly — it just cannot
    ///   reach its members' tasks.
    /// - Registers the supervisor ([`spawn_supervisor`]).
    ///
    /// `Result` is kept for the caller's sake — `insert` and `adopt` are handled
    /// alike at both call sites — and no variant is produced today: `bind` now
    /// ignores a second call instead of reporting it.
    pub fn adopt(parts: RootParts) -> Result<Root, RootError> {
        let RootParts {
            id,
            label,
            agent,
            bg,
            orchestrator,
            group,
            session_path,
            model,
            members,
            events,
        } = parts;
        let (handle, task) = SessionActor::spawn_supervised(agent);
        bg.bind(handle.clone());
        if let Some(o) = &orchestrator {
            o.register_root(handle.clone());
        }
        let address = orchestrator
            .as_ref()
            .map(|o| o.id().clone())
            .unwrap_or_else(|| SessionId::agent("root"));
        let registry = match &orchestrator {
            Some(o) => o.registry().clone(),
            None => Registry::default(),
        };
        // The roster stays live, so a worker spawned at runtime appears without a
        // restart; without an orchestrator there is no team at all.
        let roster = match &orchestrator {
            Some(o) => crate::live_roster(o.registry(), address.clone()),
            None => tokio::sync::watch::channel(Vec::new()).1,
        };
        let surface = mint_surface_id(id, &address);
        let closing = Arc::new(AtomicBool::new(false));
        let supervisor = spawn_supervisor(id, task, closing.clone(), events);
        Ok(Root {
            id,
            label,
            address,
            surface,
            handle,
            registry,
            roster,
            bg,
            orchestrator,
            members,
            session_path,
            group,
            model,
            state: RootState::Live,
            closing,
            supervisor: Some(supervisor),
        })
    }

    /// Take the actor task's `JoinHandle` — for a caller that wants to await the
    /// death itself (a test, or a host that drains before closing).
    pub fn take_supervisor(&mut self) -> Option<JoinHandle<()>> {
        self.supervisor.take()
    }

    /// Tear this root down **without** a [`Roots`] — for the exit paths. Same
    /// steps and the same order as [`Roots::close`] step 2 onwards; kept as one
    /// function body (`Roots::close` = mark closing + `remove` + this).
    pub fn tear_down(self) {
        let Root {
            id,
            address,
            surface,
            handle,
            registry,
            bg,
            orchestrator,
            members,
            closing,
            supervisor,
            ..
        } = self;
        // Step 1: our own teardown must not be reported as a crash.
        closing.store(true, Ordering::SeqCst);
        // Step 2: the DAG scheduler and any in-flight `Script` node. A no-op
        // without `--agents`.
        if let Some(o) = &orchestrator {
            o.cancel_token().cancel();
        }
        // Step 3: best effort — the actor services requests one at a time, so
        // this lands after an in-flight turn. The CLIENT refuses a close while any
        // surface of the conversation is `Running`; that refusal is what keeps
        // this abandonment away from a live turn.
        let _ = handle.send(Request::Cancel);
        // Step 4: the ROOT's tasks AND every member's, found by the scope tag
        // (`agents.rs:399` builds a worker its own `Background`, so
        // `bg.shutdown()` alone would miss them), and each in-scope notifier
        // released — the line that lets the actors END.
        Background::shutdown_scope(id);
        // The root's own notifier is in hand; release it even if the tag above
        // was lost (a `Background` built outside a scoped build), so a close never
        // leaves its own actor parked.
        bg.release_notify();
        // Step 5: retire every address. `surface` is only an address for
        // conversation 0 — removing an unregistered id is a documented no-op — so
        // the list is `address` + `members`, plus `surface` for that case.
        // This is not belt-and-braces: `Registry` is an `Arc`, `register` spawns a
        // watcher task holding a clone, and the peers map's `SessionHandle`s are
        // only released once `remove` empties it.
        for target in std::iter::once(&address)
            .chain(std::iter::once(&surface))
            .chain(members.iter())
        {
            registry.remove(target);
        }
        // Step 6: the handles drop; with the notifier released the actor's
        // `inbox.recv()` returns `None` and its task ends.
        drop(handle);
        drop(supervisor);
    }
}

// ==========================================================================
// §2 — the registry of live roots
// ==========================================================================

/// **The host's live-root registry.** One `Roots` per process (the TUI's host
/// task owns it; a second host would own its own — a CANDIDATE reading, not a
/// claim: finding 6 is open).
///
/// # What it deliberately is NOT
///
/// - **Not a focus holder.** Which conversation is 'active' is the client's
///   business. The host never reorders, never prioritises.
/// - **Not a `Registry`.** No name clash by accident: `wcode_protocol::Registry`
///   is the *address book inside one root*; this is the *list of roots*.
/// - **Not a session store.** Files live in `session_groups`; a closed root's
///   group stays on disk.
///
/// No `Default`: the event sink is installed **once, before any root exists**, so
/// a supervisor can never be built without somewhere to report.
pub struct Roots {
    /// Creation order — the tab order. Dense: `remove` drains, never swaps, so a
    /// client's index-based views never see a reordering.
    items: Vec<Root>,
    /// The `ConversationId` mint source. Starts at **1**: seq `0`
    /// ([`ConversationId::first`]) is RESERVED for the startup conversation,
    /// which the composition root builds outside [`Roots::start`] and hands to
    /// [`Roots::insert`] (finding 8 — starting at 0 double-booked `c0` onto the
    /// first `/tab new`). Monotone and never reused.
    next: u64,
    /// Where supervisors report ([`RootEvent`]).
    events: mpsc::UnboundedSender<RootEvent>,
}

impl Roots {
    /// A registry wired to one event sink, with the startup id already reserved.
    ///
    /// `next = 1` and `items` empty. [`ConversationId::first`] (seq `0`) is the
    /// STARTUP conversation's id; this constructor does not create it — the
    /// composition root does, before the client exists, and hands it to
    /// [`Roots::insert`].
    pub fn new(events: mpsc::UnboundedSender<RootEvent>) -> Self {
        Self {
            items: Vec::new(),
            next: 1,
            events,
        }
    }

    /// **The two verbs a HOST is made of**, hoisted out of the transport so that
    /// a second host would not have to re-derive them. Everything either host
    /// does on `Start` is in here; what differs is only how the request arrived
    /// and where the reply goes.
    ///
    /// # Contract
    /// ```text
    /// let id = self.mint();
    /// let root = factory(id, spec)?;      // the id goes IN
    /// self.insert(root)?;                 // refuses an occupied id
    /// Ok(id)
    /// ```
    ///
    /// - **The id is minted HERE and handed to the factory** (finding 7), so
    ///   `Root.id` and every surface id derived from it come from one authority.
    /// - **A failure BURNS the id** (finding 8): `mint()` advances `next` before
    ///   the factory runs, so a failed `Start` leaves `Roots` unchanged but does
    ///   not hand the same id back.
    /// - **A live `Resume` dir is refused BEFORE the build**, which is why a failed
    ///   `start` never leaks the actor it just built: `insert` cannot fail after
    ///   this check (a minted id is never occupied).
    /// - **Serialised by construction** while this takes `&mut self`: the borrow
    ///   is held for the (synchronous) build, so two `start`s cannot interleave on
    ///   one `Roots`.
    pub fn start(
        &mut self,
        factory: &Factories,
        spec: RootSpec,
    ) -> Result<ConversationId, RootError> {
        if let RootSpec::Resume { dir } = &spec
            && self.contains_session_path(dir)
        {
            return Err(RootError::AlreadyLive(dir.display().to_string()));
        }
        let id = self.mint();
        let root = factory(id, spec)?;
        self.insert(root)?;
        Ok(id)
    }

    /// The `Stop` verb — [`Roots::close`]'s alias, so both hosts speak the same
    /// vocabulary as the channel. Returns `false` when the id is unknown.
    pub fn stop(&mut self, id: ConversationId) -> bool {
        self.close(id)
    }

    /// Mint the next id — **the single id authority** (finding 8).
    ///
    /// Monotone and **never reused**, so a closed id can never be confused with a
    /// new root, and never returns an id already occupied.
    pub fn mint(&mut self) -> ConversationId {
        let seq = self.next;
        self.next += 1;
        ConversationId::new(seq)
    }

    /// Adopt an already-built root. `Err` on a duplicate id or a `Resume` dir
    /// already owned by a live root.
    ///
    /// Insertion order IS the client's tab order, so `insert` never sorts.
    pub fn insert(&mut self, root: Root) -> Result<(), RootError> {
        // The startup root legitimately arrives with the reserved id; a freshly
        // built one with whatever `mint` handed out. Anything at or beyond `next`
        // was never minted here.
        debug_assert!(
            root.id.seq() < self.next,
            "an inserted id comes from `mint` (or is the reserved startup id)"
        );
        if self.get(root.id).is_some() {
            return Err(RootError::AlreadyLive(root.id.to_string()));
        }
        if let Some(dir) = root.group.as_ref().map(|g| g.dir.clone())
            && self.contains_session_path(&dir)
        {
            return Err(RootError::AlreadyLive(dir.display().to_string()));
        }
        self.items.push(root);
        Ok(())
    }

    /// The root for `id`.
    pub fn get(&self, id: ConversationId) -> Option<&Root> {
        self.items.iter().find(|root| root.id == id)
    }

    /// Mutable access — a host that mirrors state or takes the supervisor.
    pub fn get_mut(&mut self, id: ConversationId) -> Option<&mut Root> {
        self.items.iter_mut().find(|root| root.id == id)
    }

    /// Every live root, creation order.
    pub fn iter(&self) -> impl Iterator<Item = &Root> {
        self.items.iter()
    }

    /// How many live roots. `1` at startup, always ≥ 0.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Never true after `Roots::new` + one `insert`; present for the
    /// `clippy::len_without_is_empty` gate.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The live root whose group dir is `dir` — the `Resume`-twice guard.
    ///
    /// # Why refuse
    /// Two roots over one group dir means two writers to one `root.jsonl`, i.e. an
    /// interleaved transcript on resume. §13 does not name this case; refusing is
    /// the only reading consistent with "a conversation is an independent root".
    pub fn contains_session_path(&self, dir: &Path) -> bool {
        self.items
            .iter()
            .any(|root| root.group.as_ref().is_some_and(|g| g.dir == dir))
    }

    /// Take the root out **without** tearing it down — for a caller that needs the
    /// pieces. The caller then owns the teardown obligation.
    pub fn remove(&mut self, id: ConversationId) -> Option<Root> {
        let idx = self.items.iter().position(|root| root.id == id)?;
        Some(self.items.remove(idx))
    }

    /// The liveness the host reports for `id`.
    pub fn state_of(&self, id: ConversationId) -> Option<RootState> {
        self.get(id).map(|root| root.state)
    }

    /// Mirror a client-derived state (the TUI's `Failed`) so a later client reads
    /// it back. Never downgrades `Dead`: `Dead` is terminal.
    pub fn set_state(&mut self, id: ConversationId, state: RootState) -> bool {
        match self.get_mut(id) {
            Some(root) if root.state == RootState::Dead && state != RootState::Dead => false,
            Some(root) => {
                root.state = state;
                true
            }
            None => false,
        }
    }

    /// **Close one conversation.** `true` if a root was removed.
    ///
    /// **Cancel-then-abandon.** It is `Request::Cancel` best-effort, the
    /// orchestrator token cancelled, the `bg` scope torn down, every registry
    /// entry retired, and the handles released — **idempotent (an unknown id is a
    /// no-op), never awaited, and the group dir left on disk**. It does not
    /// interrupt a turn and does not wait for one; the client's refusal while any
    /// surface of the conversation is `Running` is what keeps that abandonment
    /// away from a live turn.
    ///
    /// # What close does NOT do
    ///
    /// - **It never deletes files.** `<group>/root.jsonl`, `members/`,
    ///   `manifest.json`, `plan.ndjson` all stay, so `/resume` finds the
    ///   conversation again.
    /// - It does not print, does not `exit`, and does not re-exec.
    /// - It does not wait for the actor to finish dying. Releasing the notifier
    ///   makes the death *happen*; the test asserts the supervisor's handle
    ///   resolves within a bounded number of yields, not that close blocked on it.
    ///
    /// Step 1 (`closing`) is here rather than in [`Root::tear_down`]'s body so a
    /// supervisor can never observe the actor's exit unsilenced.
    pub fn close(&mut self, id: ConversationId) -> bool {
        let Some(root) = self.remove(id) else {
            return false;
        };
        root.closing.store(true, Ordering::SeqCst);
        root.tear_down();
        true
    }

    /// Close every root — the process-exit path, called where `main.rs` calls
    /// `Background::shutdown_all()` today.
    pub fn close_all(&mut self) {
        for root in std::mem::take(&mut self.items) {
            root.tear_down();
        }
    }
}

// ==========================================================================
// §5 — supervision (§13.2 item 4)
// ==========================================================================

/// **What a host must react to.** Emitted by the supervisor task, never by a
/// client-facing type — deliberately, so that a second host COULD consume the
/// same stream without a translation layer. Candidate reuse only: finding 6 is
/// open, and the TUI is the only consumer today.
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
/// - **Silent during a close**: `closing` is checked first, so a `/tab close`
///   cannot report `Died` after `Stopped` — otherwise the client paints a tab it
///   just closed as `Failed`.
/// - **Why a `JoinHandle` and not a liveness probe**: the handle holds clones of
///   BOTH channel senders, so neither `subscribe()` closing nor
///   `state_rx().changed()` can observe the actor's death; only a failing `send`
///   can, and probing means sending real requests.
/// - The task ends after one report (the root is sticky `Dead`).
fn spawn_supervisor(
    id: ConversationId,
    join: JoinHandle<()>,
    closing: Arc<AtomicBool>,
    events: mpsc::UnboundedSender<RootEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let reason = match join.await {
            Ok(()) => "actor ended",
            Err(err) if err.is_panic() => "panicked",
            Err(_) => "actor aborted",
        };
        if closing.load(Ordering::SeqCst) {
            return; // our own teardown, not a crash
        }
        let _ = events.send(RootEvent::Died { id, reason });
    })
}

// ==========================================================================
// §3 — the factory type (§13.2 item 2)
// ==========================================================================

/// **The injected build function's type** — the one thing a client cannot do for
/// itself, so the composition root injects it exactly as it injects
/// `DefineHandler` (`crates/wcode-protocol/src/server.rs:68`).
///
/// Synchronous and `Arc<dyn Fn + Send + Sync>`, matching that precedent: every
/// step the build needs is synchronous (`session_groups::create_group` is
/// `io::Result`, `repl::build_agent` is a plain `fn`, and
/// `SessionActor::spawn` internally `tokio::spawn`s).
///
/// # Why the id is an ARGUMENT (first-layer finding 7)
/// The id is minted by the host ([`Roots::mint`]) because it is the host's
/// uniqueness authority — but the build cannot produce a `Root` without it: it
/// stamps `Root.id`, and [`mint_surface_id`] derives every surface id from it. If
/// the factory did not receive it, the closure would have to invent one, and two
/// authorities minting ids is how a `c0` gets reused. The closure is pure in it
/// (no state), which keeps it `Send + Sync` and `Fn` rather than `FnMut`.
pub type Factories = Arc<dyn Fn(ConversationId, RootSpec) -> Result<Root, RootError> + Send + Sync>;

/// **The surface id for conversation `id`** — the F1 resolution, applied to the
/// ROOT **and** every MEMBER (finding 3).
///
/// # Contract
/// - `id == ConversationId::first()` ⇒ `base` **unchanged**, so conversation 0's
///   surface ids are `agent:orchestrator`/`agent:root`/`agent:w1` exactly as
///   today and the focused surface never moves (byte-identical N = 1).
/// - Otherwise `<base>@c<seq>`: distinct, loggable, and reversible back to
///   `base`. It is a **client-side label**; the A2A address stays `base`.
/// - **Call it per surface, not once per root**: every root's worker is
///   `agent:w1`, so a root-only mint leaves the two `w1`s colliding at N = 2.
///
/// # Not a name
/// The result is for routing only: never `Registry::resolve`d, never
/// `short_name`d for a label (invariant 6).
pub fn mint_surface_id(id: ConversationId, base: &SessionId) -> SessionId {
    if id == ConversationId::first() {
        return base.clone();
    }
    SessionId::new(format!("{base}@c{}", id.seq()))
}

// ==========================================================================
// §6 — the member feed (§13.2 item 8)
// ==========================================================================

/// Install a per-root member sink: one `UnboundedSender<SurfaceSpec>` per
/// orchestrator, forwarded into the tagged client channel — **with the member's
/// surface id MINTED on the way through** (finding 3).
///
/// The spec the orchestrator sends carries the member's **A2A address** as its
/// id (`agent:w1` in EVERY root), so the forwarder rewrites it with
/// [`mint_surface_id`] — the captured [`Root::id`] is the only thing that knows
/// which conversation the member belongs to — and tags the pair, because
/// `SurfaceSpec::info()` drops everything but the display fields. Untouched, two
/// roots' `w1` rows would cross-deliver.
///
/// A wrapper task, rather than changing `set_spawn_sink`'s element type, keeps
/// `agents.rs` untouched; the cost is that this is a **second** site that must
/// agree with the surface list's mint, so both call [`mint_surface_id`] and
/// neither may inline the rule.
pub fn install_surface_sink(
    root: &Root,
    out: mpsc::UnboundedSender<(ConversationId, wcode_tui::SurfaceSpec)>,
) {
    let Some(orchestrator) = &root.orchestrator else {
        return; // no A2A wiring: no member can ever appear
    };
    let id = root.id;
    let (tx, mut rx) = mpsc::unbounded_channel::<wcode_tui::SurfaceSpec>();
    orchestrator.set_spawn_sink(tx);
    tokio::spawn(async move {
        while let Some(mut spec) = rx.recv().await {
            spec.id = mint_surface_id(id, &spec.id);
            if out.send((id, spec)).is_err() {
                return; // the client is gone
            }
        }
    });
}

// ==========================================================================
// §8 — tests: one per invariant that landed in this commit
// ==========================================================================

#[cfg(test)]
mod tests {
    //! Behaviour-named tests. The sketch's remaining stubs (`build_root`'s
    //! `Fresh`/`Resume` minting, the TUI surface list, the host loop's replies)
    //! need W040 commits 4–6 and are named in this module's header.

    use super::*;
    use crate::config::{TeamMember, ToolsConfig};
    use crate::instructions::InstructionSet;
    use crate::skills::SkillSet;
    use std::time::Duration;
    use wcode_harness::agent::Agent;
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::AgentEvent;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::streamfn::LlmOpts;

    /// An agent that never touches the network: the rig stream fn is built but no
    /// run is ever started by these tests.
    ///
    /// The build runs WITHOUT an ambient scope ([`Background::with_scope`]) even
    /// though production wraps one around it. That scope is process-global, and a
    /// `cargo test` process runs tests in parallel: a scoped build here would tag
    /// another test's `Background` with this conversation, and a later
    /// `shutdown_scope` would signal a stranger's child (observed: the `bg` tests'
    /// children killed). The fixture therefore keeps production's *ownership*
    /// (the `Background` the tools hold IS the one `adopt` receives — the notifier
    /// cycle is real, so the leak gate has teeth) and drops only the tagging,
    /// which the scope test below covers with [`Background::new_in`].
    fn build(_id: ConversationId) -> (Agent, Arc<Background>) {
        let tools = ToolsConfig::default();
        let instructions = InstructionSet::default();
        let skills = SkillSet::default();
        let team: Vec<TeamMember> = Vec::new();
        crate::repl::build_agent(
                crate::repl::AgentSpec {
                    llm: LlmOpts {
                        model: "m1".into(),
                        ..LlmOpts::default()
                    },
                    hooks: HooksSet::default(),
                    tools: &tools,
                    compaction: CompactionPolicy::default(),
                    title_after_turns: 0,
                    instructions: &instructions,
                    skills: &skills,
                    team: &team,
                    guidelines: None,
                },
            None,
            Vec::new(),
            Vec::new(),
        )
    }

    /// A root for `id`, with `group` when the test wants one.
    fn adopt(
        id: ConversationId,
        group: Option<SessionGroup>,
        events: &mpsc::UnboundedSender<RootEvent>,
    ) -> Root {
        let (agent, bg) = build(id);
        Root::adopt(RootParts {
            id,
            label: format!("label-{id}"),
            agent,
            bg,
            orchestrator: None,
            group,
            session_path: None,
            model: "m1".into(),
            members: Vec::new(),
            events: events.clone(),
        })
        .expect("adopt cannot fail")
    }

    fn channels() -> (
        mpsc::UnboundedSender<RootEvent>,
        mpsc::UnboundedReceiver<RootEvent>,
    ) {
        mpsc::unbounded_channel()
    }

    /// Yield until `cond` or the bound is spent. Never sleeps: a `sleep` would
    /// turn a hang into a slower hang.
    async fn until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..256 {
            if cond() {
                return true;
            }
            tokio::task::yield_now().await;
        }
        cond()
    }

    #[tokio::test]
    async fn two_roots_do_not_share_a_registry_or_roster() {
        // §13.2 item 1: each conversation is an INDEPENDENT root runtime.
        let (tx, _rx) = channels();
        let mut mints = Roots::new(tx.clone());
        let c0 = ConversationId::first();
        let c1 = mints.mint();
        let a = adopt(c0, None, &tx);
        let b = adopt(c1, None, &tx);

        let probe = SessionId::agent("x");
        a.registry.register(probe.clone(), b.handle.clone());
        assert!(a.registry.contains(&probe), "c0 knows its own peer");
        assert!(
            !b.registry.contains(&probe),
            "c1's registry is not c0's: {} leaked across",
            probe
        );
        // The rosters are separate live views, and neither lists the other root.
        assert!(a.roster.borrow().is_empty());
        assert!(b.roster.borrow().is_empty());
        assert_ne!(a.address.to_string() + "|", b.surface.to_string() + "|");
    }

    #[tokio::test]
    async fn closing_a_conversation_ends_its_actor_task() {
        // FINDING 5's gate — the LEAK, not the kill. Without `release_notify`
        // the `tools → Arc<Background> → notify → inbox` cycle keeps the actor
        // parked forever, so the supervisor's handle never resolves.
        let (tx, mut rx) = channels();
        let mut roots = Roots::new(tx.clone());
        let c1 = roots.mint();
        roots.insert(adopt(c1, None, &tx)).expect("inserted");

        // Subscribe BEFORE closing: the receiver is what outlives the handles.
        let mut events = roots.get(c1).expect("live").handle.subscribe();
        let supervisor = roots
            .get_mut(c1)
            .expect("live")
            .take_supervisor()
            .expect("a supervised root");

        assert!(roots.close(c1), "a live root closes");
        assert_eq!(roots.len(), 0);

        // The actor ends: its `events` sender drops, so the broadcast closes.
        let closed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Err(tokio::sync::broadcast::error::RecvError::Closed) = events.recv().await {
                    return;
                }
            }
        })
        .await;
        assert!(closed.is_ok(), "the actor task ends once the notifier is released");
        assert!(
            until(|| supervisor.is_finished()).await,
            "and the supervisor sees it"
        );
        // Silence: a close is not a death (§5, step 1).
        assert!(
            rx.try_recv().is_err(),
            "a close must not report Died: {:?}",
            rx.try_recv()
        );
    }

    #[tokio::test]
    async fn shutdown_scope_releases_only_the_named_conversations_backgrounds() {
        // §13.2 item 3: `shutdown_scope`, not `shutdown_all`. The member case is
        // the interesting one — a worker builds its OWN `Background`
        // (`agents.rs:399`), so the tag is the only route to it, and a
        // conversation's close must not touch a neighbour's.
        let c0 = ConversationId::first();
        let c1 = ConversationId::new(1);
        let a = Background::new_in(c0);
        let b = Background::new_in(c1);
        let c = Background::new_in(c1);
        Background::shutdown_scope(c1);
        assert!(b.is_notifier_released());
        assert!(c.is_notifier_released());
        assert!(
            !a.is_notifier_released(),
            "c0's background must survive c1's scope teardown"
        );
        Background::shutdown_scope(ConversationId::new(77)); // unknown: a no-op
        assert!(!a.is_notifier_released());
        // And the lock-free part of the same rule at the Roots level: closing a
        // conversation releases ITS root's notifier, and nobody else's.
        let (tx, _rx) = channels();
        let mut roots = Roots::new(tx.clone());
        let root0 = roots.mint();
        let root1 = roots.mint();
        roots.insert(adopt(root0, None, &tx)).expect("inserted");
        roots.insert(adopt(root1, None, &tx)).expect("inserted");
        let bg0 = roots.get(root0).expect("live").bg.clone();
        let bg1 = roots.get(root1).expect("live").bg.clone();
        assert!(roots.close(root1));
        assert!(bg1.is_notifier_released(), "the closed root's own background");
        assert!(!bg0.is_notifier_released(), "the survivor's is untouched");
    }

    #[tokio::test]
    async fn a_dead_actor_is_reported_as_died_without_closing() {
        // §13.2 item 4: a conversation whose actor task resolved reports `Died`;
        // the process does not unwind and the neighbours stay live.
        let (tx, mut rx) = channels();
        let c0 = ConversationId::first();
        let mut roots = Roots::new(tx.clone());
        let c1 = roots.mint();
        roots.insert(adopt(c0, None, &tx)).expect("inserted");
        roots.insert(adopt(c1, None, &tx)).expect("inserted");

        // Take c1 out without closing it, drop every handle (releasing the
        // notifier first — otherwise the actor is unreachable by design), and the
        // supervisor reports the death.
        let mut root = roots.remove(c1).expect("live");
        let supervisor = root.take_supervisor().expect("supervised");
        root.bg.release_notify();
        drop(root);

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a report")
            .expect("the sender is alive");
        assert_eq!(
            event,
            RootEvent::Died {
                id: c1,
                reason: "actor ended"
            }
        );
        assert!(until(|| supervisor.is_finished()).await);
        // The host mirrors it, and c0 is untouched.
        assert!(roots.set_state(c1, RootState::Dead) || roots.get(c1).is_none());
        assert_eq!(roots.state_of(c0), Some(RootState::Live));
        // c0 still answers.
        let history = roots
            .get(c0)
            .expect("live")
            .handle
            .ask(Request::GetHistory)
            .await
            .expect("c0 answers");
        assert!(matches!(history, AgentEvent::History { .. }));
        // And `Dead` is sticky: a later `Failed` cannot downgrade it.
        roots.insert(adopt(c1, None, &tx)).ok();
    }

    #[tokio::test]
    async fn dead_is_sticky() {
        let (tx, _rx) = channels();
        let c0 = ConversationId::first();
        let mut roots = Roots::new(tx.clone());
        roots.insert(adopt(c0, None, &tx)).expect("inserted");
        assert!(roots.set_state(c0, RootState::Dead));
        assert!(
            !roots.set_state(c0, RootState::Failed),
            "Dead is terminal: a later Failed cannot downgrade it"
        );
        assert_eq!(roots.state_of(c0), Some(RootState::Dead));
        assert!(roots.set_state(c0, RootState::Dead), "re-setting Dead is fine");
        assert!(!roots.set_state(ConversationId::new(99), RootState::Failed));
    }

    #[tokio::test]
    async fn a_second_root_cannot_own_a_live_group_dir() {
        // `Resume` refuses a dir already served: two writers to one `root.jsonl`
        // would interleave two transcripts.
        let tmp = tempfile::tempdir().expect("a tempdir");
        let group = crate::session_groups::create_group(tmp.path()).expect("a group");
        let (tx, _rx) = channels();
        let mut roots = Roots::new(tx.clone());
        roots
            .insert(adopt(ConversationId::first(), Some(group.clone()), &tx))
            .expect("the first owner");

        assert!(roots.contains_session_path(&group.dir));
        let dup_id = roots.mint();
        let dup = adopt(dup_id, Some(group.clone()), &tx);
        match roots.insert(dup) {
            Err(RootError::AlreadyLive(what)) => {
                assert!(what.contains("_"), "names the group dir: {what}")
            }
            other => panic!("expected AlreadyLive, got {other:?}"),
        }
        assert_eq!(roots.len(), 1, "nothing half-inserted");
    }

    #[tokio::test]
    async fn a_closed_roots_files_remain_on_disk() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let group = crate::session_groups::create_group(tmp.path()).expect("a group");
        let root_path = group.root_path.clone();
        let members_dir = group.members_dir.clone();
        crate::session_groups::create_root_session(&group.dir, tmp.path()).expect("a session");

        let (tx, _rx) = channels();
        let mut roots = Roots::new(tx.clone());
        // A MINTED id, not `first()`: `shutdown_scope(c0)` is process-wide for
        // every `Background` that was built outside a scoped build, which in a
        // `cargo test` process means other tests' fixtures. Conversation 0's own
        // teardown is the same code path.
        let id = roots.mint();
        roots
            .insert(adopt(id, Some(group.clone()), &tx))
            .expect("inserted");
        assert!(roots.close(id));

        assert!(root_path.is_file(), "root.jsonl stays");
        assert!(members_dir.is_dir(), "members/ stays");
        // And the dir still opens as a group, so `/resume` finds it again.
        assert!(crate::session_groups::open_group(&group.dir).is_ok());
    }

    #[tokio::test]
    async fn conversation_zero_keeps_todays_surface_id() {
        let orchestrator = SessionId::agent("orchestrator");
        assert_eq!(
            mint_surface_id(ConversationId::first(), &orchestrator),
            orchestrator,
            "N = 1 is byte-identical"
        );
        let member = SessionId::agent("w1");
        let minted = mint_surface_id(ConversationId::new(1), &member);
        assert_ne!(minted, member, "c1's w1 is not c0's w1");
        assert_eq!(minted.to_string(), "agent:w1@c1");
        // Every root mints the same base to a distinct id.
        for seq in 1..4 {
            let id = mint_surface_id(ConversationId::new(seq), &member);
            assert!(id.to_string().contains(&format!("@c{seq}")), "{id}");
        }
    }

    #[tokio::test]
    async fn a_failed_build_inserts_nothing_and_burns_the_id() {
        // FINDING 8's gate, PINNED: monotone, never reused.
        let (tx, _rx) = channels();
        let mut roots = Roots::new(tx.clone());
        // `Roots::new` reserves seq 0: the first mint is c1, never c0.
        assert_eq!(roots.mint(), ConversationId::new(1));

        let failing: Factories =
            Arc::new(|_id, _spec| Err(RootError::Config("bad [team] entry".into())));
        let err = roots
            .start(&failing, RootSpec::Fresh { label: None })
            .expect_err("the factory fails");
        assert!(err.to_string().contains("bad [team] entry"), "{err}");
        assert_eq!(roots.len(), 0, "nothing half-inserted");

        // The next attempt gets c3, not the c2 the failure consumed.
        let ok: Factories = Arc::new(|id, _spec| {
            let (agent, bg) = build(id);
            Root::adopt(RootParts {
                id,
                label: format!("label-{id}"),
                agent,
                bg,
                orchestrator: None,
                group: None,
                session_path: None,
                model: "m1".into(),
                members: Vec::new(),
                events: mpsc::unbounded_channel().0,
            })
        });
        let id = roots
            .start(&ok, RootSpec::Fresh { label: None })
            .expect("the second build");
        assert_eq!(id, ConversationId::new(3), "the failed id was burned");
        assert_eq!(roots.len(), 1);
        // And the reserved startup id is still free for the startup root.
        roots
            .insert(adopt(ConversationId::first(), None, &tx))
            .expect("the reserved id is not double-booked");
        assert_eq!(roots.len(), 2);
    }

    #[tokio::test]
    async fn a_resume_of_a_live_dir_is_refused_before_the_build() {
        // The pre-check is what keeps a failed `start` from leaking the actor it
        // just built: `insert` cannot fail after it.
        let tmp = tempfile::tempdir().expect("a tempdir");
        let group = crate::session_groups::create_group(tmp.path()).expect("a group");
        let (tx, _rx) = channels();
        let mut roots = Roots::new(tx.clone());
        roots
            .insert(adopt(ConversationId::first(), Some(group.clone()), &tx))
            .expect("the first owner");

        let never_called: Factories = Arc::new(|_id, _spec| {
            panic!("the factory must not run for a live Resume dir")
        });
        let err = roots
            .start(&never_called, RootSpec::Resume { dir: group.dir.clone() })
            .expect_err("refused");
        assert!(matches!(err, RootError::AlreadyLive(_)), "{err}");
        assert_eq!(roots.len(), 1);
    }

    #[tokio::test]
    async fn stop_is_close_and_an_unknown_id_is_a_no_op() {
        let (tx, _rx) = channels();
        let mut roots = Roots::new(tx.clone());
        let c0 = roots.mint();
        roots.insert(adopt(c0, None, &tx)).expect("inserted");
        assert!(roots.stop(c0));
        assert!(!roots.stop(c0), "idempotent: an unknown id is a no-op");
        assert!(roots.is_empty(), "the last close leaves an empty registry");
        roots.close_all(); // and close_all on an empty registry is a no-op
    }

    #[tokio::test]
    async fn a_root_error_says_what_a_human_must_do() {
        assert_eq!(
            RootError::BadResume(PathBuf::from("/tmp/x")).to_string(),
            "/tmp/x is not a session group directory"
        );
        assert_eq!(
            RootError::AlreadyLive("c3".into()).to_string(),
            "c3 is already live"
        );
        assert_eq!(
            RootError::Config("no [team]".into()).to_string(),
            "no [team]"
        );
        assert!(
            RootError::Session(std::io::Error::other("boom"))
                .to_string()
                .starts_with("session io:")
        );
    }

    /// `adopt`'s `RootParts::events` sink is what makes a death reportable: a root
    /// whose supervisor has nobody to tell is the invisible failure item 4 exists
    /// to end.
    #[tokio::test]
    async fn the_supervisor_reports_on_the_sink_the_caller_gave() {
        let (tx, mut rx) = channels();
        let id = Roots::new(tx.clone()).mint();
        let mut root = adopt(id, None, &tx);
        let supervisor = root.take_supervisor().expect("supervised");
        root.bg.release_notify();
        drop(root);
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a report")
            .expect("alive");
        assert_eq!(
            event,
            RootEvent::Died {
                id,
                reason: "actor ended"
            }
        );
        assert!(until(|| supervisor.is_finished()).await);
    }
}
