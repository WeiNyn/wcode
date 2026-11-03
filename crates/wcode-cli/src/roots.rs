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
use crate::repl::RunInputs;
use crate::session_groups::SessionGroup;
use crate::tools::background::Background;

// ==========================================================================
// §1 — one live root
// ==========================================================================

/// What building or adopting one root can go wrong. `Display` is the string the
/// client sees, so every variant must be a sentence a human can act on.
#[derive(Debug)]
pub enum RootError {
    /// The client asked for a conversation whose path is neither a group dir
    /// (resolved by `group_dir_of`) nor a legacy flat `.jsonl` file.
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
                "{} is not a session",
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
    ///
    /// The actor task resolved: panicked, or ended without a `close`
    /// (§13.2 item 4). Sticky; the handles are still held so the client can read
    /// what the root produced.
    Dead,
}

/// The several things one root is built from, grouped so [`Roots::adopt`] and the
/// build path cannot drift apart. No event sink: that is [`Roots`]'s
/// (see [`Roots::adopt`]).
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
    /// What the build would have printed; the host forwards these to the tab,
    /// and the single-root boot prints them instead.
    pub notices: Vec<String>,
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
    ///
    /// Read only by [`Root::as_protocol_root`] (amendment (k)); retires with it.
    #[allow(dead_code)]
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
    /// and for a served worker with no local file. Feeds `surface_specs`'s
    /// `resume_path` (W046 dedupe).
    pub session_path: Option<PathBuf>,
    /// What the build would have printed; forwarded to the tab.
    pub notices: Vec<String>,
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
    /// - `bg` carries `id` ([`Background::new`]`(id)` where the agent was built):
    ///   teardown finds a conversation's **own and its members'** `Background`s by
    ///   that tag, so a mis-tagged root leaks an actor and, worse, is signalled by
    ///   another conversation's close. Debug-asserted (§13.2.1(a)).
    /// - Registers the supervisor ([`spawn_supervisor`]) on `events`.
    ///
    /// Infallible: every step is (`spawn_supervised`, `bind` — a no-op on a
    /// second call, `register_root`, `mint_surface_id`, `watch::channel`,
    /// `Registry::default`), so a `Result` here only forced dead
    /// `unwrap_or_else(|e| … exit(1))` at every call site (W040 §13.2.1(d)).
    ///
    /// Private, and reached only through [`Roots::adopt`]: that is what makes a
    /// root reporting to a sink other than its registry's unrepresentable
    /// (§13.2.1(c)).
    fn adopt(parts: RootParts, events: mpsc::UnboundedSender<RootEvent>) -> Root {
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
            notices,
        } = parts;
        debug_assert_eq!(
            bg.scope(),
            id,
            "a root's Background carries its own conversation tag"
        );
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
        Root {
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
            notices,
            state: RootState::Live,
            closing,
            supervisor: Some(supervisor),
        }
    }

    /// Take the actor task's `JoinHandle` — for a caller that wants to await the
    /// death itself. **Test-only** (amendment (k)): the host lets the supervisor
    /// report and never joins.
    #[cfg(test)]
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
        let parts = factory(id, spec)?;
        let root = self.adopt(parts);
        self.insert(root)?;
        Ok(id)
    }

    /// The `Stop` verb — [`Roots::close`]'s alias, so both hosts speak the same
    /// vocabulary as the channel. Returns `false` when the id is unknown.
    ///
    /// Called by the tests only (amendment (k)): `run_host`'s `Stop` arm still
    /// calls `close` directly, so this retires when a second host speaks the
    /// channel, or when the arm switches to it.
    #[allow(dead_code)]
    pub fn stop(&mut self, id: ConversationId) -> bool {
        self.close(id)
    }

    /// **Build one root from already-built parts**, wired to THIS registry's
    /// event sink. Infallible.
    ///
    /// # Why this shape (§13.2.1(c))
    /// The sketch had two independent parameters — `Roots::new(events)` and
    /// `RootParts { events }` — and `Root` does not retain the sender, so a root
    /// built with one sink and inserted into a `Roots` listening on another could
    /// never be detected: its death would be reported into the void. One method
    /// owning the sink makes that pair unrepresentable, and it is the same object
    /// that owns the id reservation (`next`).
    ///
    /// The caller registers the result with [`Roots::insert`], which keeps its
    /// `Result`: a duplicate id and a live `Resume` dir are real failures.
    pub fn adopt(&mut self, parts: RootParts) -> Root {
        Root::adopt(parts, self.events.clone())
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

    /// How many live roots. **Test-only** (amendment (k)).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Never true after `Roots::new` + one `insert`. **Test-only** (amendment (k));
    /// it exists for `clippy::len_without_is_empty` beside the test-only `len`.
    #[cfg(test)]
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
        // A legacy root has `group: None`, so it is invisible to the group-only
        // check above; fall back to its `session_path` (the flat file). `insert`'s
        // own guard is narrower for the same reason, and `start`'s pre-build
        // check (`:449`-region) is what catches a legacy duplicate.
        self.items.iter().any(|root| {
            root.group.as_ref().is_some_and(|g| g.dir == dir)
                || root.session_path.as_deref() == Some(dir)
        })
    }

    /// Take the root out **without** tearing it down — for a caller that needs the
    /// pieces. The caller then owns the teardown obligation.
    pub fn remove(&mut self, id: ConversationId) -> Option<Root> {
        let idx = self.items.iter().position(|root| root.id == id)?;
        Some(self.items.remove(idx))
    }

    /// The liveness the host reports for `id`. **Test-only** (amendment (k)): the
    /// host writes state through `set_state` and the client reads the tab.
    #[cfg(test)]
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
/// # Why it returns `RootParts`, not `Root` (§13.2.1(c))
/// A `Root` cannot be built without the supervisor's event sink, and the sink
/// belongs to the [`Roots`] the root joins — `Root::adopt` is private for exactly
/// that reason. So the factory hands back the parts and [`Roots::start`] adopts
/// them with its own sink. A factory that could mint a `Root` would be able to
/// report its death into the void.
///
/// # Why the id is an ARGUMENT (first-layer finding 7)
/// The id is minted by the host ([`Roots::mint`]) because it is the host's
/// uniqueness authority — but the build cannot produce a `Root` without it: it
/// stamps `Root.id`, and [`mint_surface_id`] derives every surface id from it. If
/// the factory did not receive it, the closure would have to invent one, and two
/// authorities minting ids is how a `c0` gets reused. The closure is pure in it
/// (no state), which keeps it `Send + Sync` and `Fn` rather than `FnMut`.
pub type Factories =
    Arc<dyn Fn(ConversationId, RootSpec) -> Result<RootParts, RootError> + Send + Sync>;

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

/// **Build the closure the host calls.** Pure: it captures `inputs` and returns;
/// the building happens on call.
///
/// `pub(crate)`, not `pub`: its parameter is a `pub(crate)` type.
///
/// # Contract
/// - The returned `Fn` takes the already-minted [`ConversationId`] and never mints
///   one (finding 7: [`Roots::mint`] is the only authority).
/// - **One build at a time.** The build mutates no process-global state, but the
///   client's `Failed` reply carries no id, so the host serialises `Start`s.
/// - It never `exit`s and never prints: every failure comes back as
///   [`RootError`] (`build_runtime`/`build_agent_for` now return them).
pub(crate) fn factory_for(inputs: RunInputs) -> Factories {
    Arc::new(move |id, spec| build_root(&inputs, id, spec))
}

/// **Build one root** — [`RootSpec::Fresh`](../../wcode_protocol/enum.RootSpec.html)
/// or `Resume`, from nothing, in one synchronous pass.
///
/// # Contract
///
/// **Session first.** `Fresh` creates a group dir + its `root.jsonl`; `Resume {
/// dir }` validates the dir name, opens the group and loads the root (restoring
/// the session's model/effort unless the flags were given); `--no-session` does
/// neither. Distinct dirs come from the fs: `create_group` uses `fs::create_dir`,
/// which fails on a collision instead of merging.
///
/// **The runtime and the agent** are `build_runtime`/`build_agent_for` in their
/// `Result` form, carrying the SAME id this root was minted with — one value for
/// `Background`'s tag, the orchestrator's workers and every surface id derived
/// from it.
///
/// **The label** (`W040` §13.6.1): `agent.title()` → the requested seed → the
/// cwd's basename. `title()` first because it is the same source the header
/// reads, so a `--title` root and its tab cannot disagree.
///
/// **The members** are A2A addresses in display order: `[team]` for a fresh root,
/// `scan_members` for a resumed group, none for a served `--owner` worker.
///
/// It does NOT spawn: [`Roots::adopt`] does, so the sink is the registry's.
///
/// **It emits the same informational stderr lines the single-root boot does** —
/// `workflow: present but inert (no --task)`, `team: …`, `workflow: {n} nodes` —
/// because it runs the same `build_runtime`. "Never `exit`, never a stdout write"
/// is literally true and misses the point: a multi-root host would interleave
/// those lines on one terminal. **Commit 6 decides** whether they move into
/// [`RootParts`]/`ConversationSpec`; until then a client must tolerate them.
///
/// # Errors
///
/// [`RootError`]: `BadResume`/`Session` from the group, `Config` from the
/// previously-exiting build path. Never `exit`, never a stdout write, never a
/// panic on a user-supplied path.
///
/// **A failed build tears itself down.** By the time the runtime's or the
/// agent's step fails, `build_runtime` has already spawned `[team]` members (and
/// possibly the scheduler), and each worker's `Background` holds its own
/// notifier — dropping the local `Orchestrator` would leave every one of those
/// actors parked for the life of the process, plus a `Registry` watcher each.
/// So the error path runs the same [`Background::shutdown_scope`] that
/// [`Roots::close`] uses; it is harmless on the session-step failures, where
/// nothing is tagged `id` yet. A `Fresh` root that got as far as creating its
/// group dir also REMOVES that dir: `list_groups` has no content filter, so a
/// leftover empty group would be offered by `/resume`. An existing dir (a
/// `Resume`) is never ours to delete.
pub(crate) fn build_root(
    inputs: &RunInputs,
    id: ConversationId,
    spec: RootSpec,
) -> Result<RootParts, RootError> {
    // Which group dir (if any) THIS build created — the only one it may delete.
    let created: std::cell::Cell<Option<PathBuf>> = std::cell::Cell::new(None);
    let created_ref = &created;
    let outcome = build_root_inner(inputs, id, &spec, created_ref);
    if let Err(err) = &outcome {
        // The teardown slice A made possible: release every `Background` tagged
        // with this conversation (the workers `build_runtime` already spawned),
        // so no actor parks and no registry watcher outlives the failure.
        crate::tools::background::Background::shutdown_scope(id);
        if let Some(dir) = created.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
        let _ = err;
    }
    outcome
}

/// The body of [`build_root`]: no teardown, no cleanup — the wrapper owns both.
fn build_root_inner(
    inputs: &RunInputs,
    id: ConversationId,
    spec: &RootSpec,
    created: &std::cell::Cell<Option<PathBuf>>,
) -> Result<RootParts, RootError> {
    let args = &inputs.args;
    let cfg = &inputs.cfg;
    let mut llm = inputs.llm.clone();

    // 1. The session — created for `Fresh`, loaded for `Resume`, absent for
    // `--no-session`.
    let (session, context, group, resuming_group) = match &spec {
        RootSpec::Fresh { .. } => {
            if args.no_session {
                (None, Vec::new(), None, false)
            } else {
                let group =
                    crate::session_groups::create_group(&inputs.dir).map_err(RootError::Session)?;
                // Ours from here on: a later failure removes it, so `/resume`
                // never offers a half-built conversation.
                created.set(Some(group.dir.clone()));
                let session = crate::session_groups::create_root_session(&group.dir, &inputs.cwd)
                    .map_err(RootError::Session)?;
                (Some(session), Vec::new(), Some(group), false)
            }
        }
        RootSpec::Resume { dir } => {
            // Mirror `main.rs`'s boot dispatch (`group_dir_of`): a group dir (or
            // a `<dir>/root.jsonl`) resumes the whole team; a flat `.jsonl`
            // resumes as itself. Anything else is not a session.
            match crate::session_groups::group_dir_of(dir) {
                Some(group_dir) => {
                    let group = crate::session_groups::open_group(&group_dir)
                        .map_err(RootError::Session)?;
                    let (session, context) =
                        crate::session_groups::load_root(&group).map_err(RootError::Session)?;
                    // A resumed root restores the session's own model/effort unless
                    // the flags said otherwise (the root keeps what it was using).
                    if args.model.is_none()
                        && let Some(model) = session.model()
                    {
                        llm.model = model;
                    }
                    if args.effort.is_none()
                        && let Some(effort) = session.effort()
                    {
                        llm.effort = effort;
                    }
                    (Some(session), context, Some(group), true)
                }
                None => {
                    if !dir.is_file() {
                        return Err(RootError::BadResume(dir.clone()));
                    }
                    let session = crate::Session::open(dir).map_err(RootError::Session)?;
                    let context = session.messages();
                    if args.model.is_none()
                        && let Some(model) = session.model()
                    {
                        llm.model = model;
                    }
                    if args.effort.is_none()
                        && let Some(effort) = session.effort()
                    {
                        llm.effort = effort;
                    }
                    (Some(session), context, None, false)
                }
            }
        }
    };
    // `build_runtime` reads only these three fields; the owned session/context
    // are single-use and stay here (they are now `Copy`-free locals, not state
    // captured by the factory — finding 9's edge).
    let setup = crate::SessionSetup {
        cwd: inputs.cwd.clone(),
        active_group: group.clone(),
        resuming_group,
        session: None,
        context: Vec::new(),
        notices: Vec::new(),
    };

    // 2. The runtime, then the agent — both carrying THIS root's id.
    let (rt, notices) = crate::build_runtime(args, cfg, &llm, &setup, id)
        .map_err(|e| RootError::Config(e.detail()))?;

    let root = crate::RootCtx {
        team: if args.owner.is_some() { &[] } else { &cfg.team },
        guidelines: if args.owner.is_some() {
            None
        } else {
            cfg.orchestrator.guidelines.as_deref()
        },
    };
    let (agent, bg) = crate::build_agent_for(args, cfg, &llm, &rt, root, session, context, id)
        .map_err(|e| RootError::Config(e.detail()))?;

    // 3. The label: `agent.title()` → the requested seed → the cwd basename.
    let session_path = agent.session_path().map(std::path::Path::to_path_buf);
    let seed = match spec {
        RootSpec::Fresh { label } => label.clone(),
        RootSpec::Resume { dir } => {
            if dir.is_file() {
                dir.file_stem()
            } else {
                dir.file_name()
            }
            .map(|n| n.to_string_lossy().into_owned())
        }
    };
    let label = agent
        .title()
        .or(seed)
        .unwrap_or_else(|| crate::label_from_cwd(&inputs.cwd));

    // 4. The team, in display order — A2A addresses, never surface ids.
    let members: Vec<SessionId> = if args.owner.is_some() {
        Vec::new()
    } else if let (true, Some(group)) = (resuming_group, group.as_ref()) {
        crate::session_groups::scan_members(group)
            .unwrap_or_default()
            .iter()
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(SessionId::agent))
            .collect()
    } else {
        cfg.team
            .iter()
            .map(|m| SessionId::agent(&m.name))
            .collect()
    };

    Ok(RootParts {
        id,
        label,
        agent,
        bg,
        orchestrator: rt.orchestrator.clone(),
        group,
        session_path,
        model: llm.model.clone(),
        members,
        notices,
    })
}

// ==========================================================================
// §6 — the reverse channel (the host loop, §13.2 item 8)
// ==========================================================================

/// The three channels one host owns — the two the TUI already has
/// (`new_surfaces`, `new_tasks`), plus the reverse pair from
/// `wcode_tui::conversations::ConversationFeeds`.
///
/// | direction | type |
/// |---|---|
/// | client → host | `UnboundedSender<ConversationRequest>` (`ConversationFeeds::outbound`) |
/// | host → client | `UnboundedReceiver<ConversationEvent>` (`ConversationFeeds::incoming`) |
/// | host → client (members) | `UnboundedSender<(ConversationId, SurfaceSpec)>` — the TAGGED `new_surfaces` |
///
/// **This is the TUI edge of the module.** §§1–5 name no `wcode_tui` type;
/// `HostChannels` and [`Root::surface_specs`] do. Whether a second host shares
/// them VERBATIM is the open finding-6 question (`W040` §13.5.2) — **candidate
/// reuse, pending that ruling**, never a claim. The handlers behind the edge
/// ([`Roots::start`]/[`Roots::stop`]) are the part that might carry over.
pub struct HostChannels {
    /// `Start`/`Stop` from the client.
    pub requests: mpsc::UnboundedReceiver<wcode_tui::conversations::ConversationRequest>,
    /// `Added`/`Failed`/`Stopped`/`Died` to the client.
    pub events: mpsc::UnboundedSender<wcode_tui::conversations::ConversationEvent>,
    /// Members spawned at runtime, tagged with their conversation.
    pub surfaces: mpsc::UnboundedSender<(ConversationId, wcode_tui::SurfaceSpec)>,
    /// The supervisor end. `Roots::new` gets the matching SENDER, so the receiver
    /// must be handed to the loop that selects on it — this field. Splitting the
    /// pair is deliberate: `Roots` owns the sink (so no root can be inserted
    /// without one) while the loop owns the wake-up.
    pub root_events: mpsc::UnboundedReceiver<RootEvent>,
}

/// **Service the reverse channel until the client goes away.** The whole of
/// §13.2 item 8's "a reverse channel" is this loop.
///
/// # Contract
///
/// - `Start(spec)` → [`Roots::start`] (mint + build + insert — the verb any other
///   host would call) → `events.send(Added(ConversationSpec { id, label,
///   surfaces }))`. **If that `send` fails** (the client quit while we were
///   building), the root is closed immediately — otherwise a `/tab new` plus an
///   instant `Ctrl-C` leaks an actor, a `bg` scope and a session file.
/// - A failed build answers `Failed { error }`. **That reply carries no id**, so
///   only ONE build may be in flight — which is what `Roots::start`'s `&mut self`
///   guarantees: the borrow is held across the (synchronous) build, and this loop
///   is the only caller, so two `Start`s cannot interleave. A second host would
///   have to keep that property; it is stated here because the type cannot.
/// - `Stop(id)` → [`Roots::close`] → `events.send(Stopped { id })`. `Stopped` is
///   sent even for an unknown id, so a client waiting for its echo cannot hang.
/// - `RootEvent::Died` → `set_state(id, Dead)` then `Died { id }`; the tab keeps
///   its surfaces (a root whose actor panicked must stay readable).
/// - The loop ends when `requests` closes; it returns [`Roots`] so the caller can
///   [`Roots::close_all`].
/// - A blocking build inside the `select!` is accepted: creating a group and an
///   agent is milliseconds and touches no network. `spawn_blocking` is NOT used —
///   it would move `build_runtime`'s `tokio::spawn`s off the reactor's task
///   context for no gain.
pub async fn run_host(roots: Roots, factory: Factories, channel: HostChannels) -> Roots {
    use wcode_tui::conversations::{ConversationEvent, ConversationRequest};

    let HostChannels {
        mut requests,
        events,
        surfaces,
        mut root_events,
    } = channel;
    let mut roots = roots;
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { break }; // the client is gone
                match request {
                    ConversationRequest::Start(spec) => {
                        match roots.start(&factory, spec) {
                            Ok(id) => {
                                // `surface_specs` cannot fail for a root we just
                                // inserted; an error is a bug, and the honest
                                // answer is the same `Failed` the client handles.
                                let reply = match roots.get(id) {
                                    Some(root) => root.surface_specs().map(|surfaces| {
                                        wcode_tui::conversations::ConversationSpec {
                                            id,
                                            label: root.label.clone(),
                                            surfaces,
                                            notices: root.notices.clone(),
                                        }
                                    }),
                                    None => Err(RootError::AlreadyLive(id.to_string())),
                                };
                                match reply {
                                    Ok(conversation) => {
                                        if events
                                            .send(ConversationEvent::Added(conversation))
                                            .is_err()
                                        {
                                            // The client quit mid-build: tear the
                                            // fresh root down rather than leak it.
                                            roots.close(id);
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = events.send(ConversationEvent::Failed {
                                            error: e.to_string(),
                                        });
                                        roots.close(id);
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = events
                                    .send(ConversationEvent::Failed { error: e.to_string() });
                            }
                        }
                    }
                    ConversationRequest::Stop(id) => {
                        roots.close(id);
                        let _ = events.send(ConversationEvent::Stopped { id });
                    }
                }
            }
            event = root_events.recv() => {
                let Some(RootEvent::Died { id, reason }) = event else {
                    // Every sender dropped: no root can report again.
                    continue;
                };
                let _ = reason;
                roots.set_state(id, RootState::Dead);
                let _ = events.send(ConversationEvent::Died { id });
            }
        }
    }
    // The member feed's sender is owned by the caller; this loop only forwards
    // through `install_surface_sink`, so it has nothing to drain here.
    let _ = surfaces;
    roots
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
// §7 — the TUI adapters (the ONLY TUI-shaped code in this file)
// ==========================================================================

impl Root {
    /// The client's view-model row: id, label, root surface, members, state.
    ///
    /// `is_remote` is always `false` here — a locally built root is
    /// `Backend::Local` by §13.2 item 1; a socket client's single conversation is
    /// built by `main.rs` and never comes through this module.
    pub fn conversation(&self) -> wcode_tui::conversations::Conversation {
        use wcode_tui::conversations::{Conversation, ConversationState};

        Conversation {
            id: self.id,
            label: self.label.clone(),
            root: self.surface.clone(),
            members: self
                .members
                .iter()
                .map(|address| mint_surface_id(self.id, address))
                .collect(),
            // The ONLY derivation: a `Dead` root is `Failed` in the container.
            // The host has two facts (live / the actor resolved); a *failed run* is
            // the CLIENT's derivation from its own surfaces, so the container's
            // three-valued state stays the single answer.
            state: match self.state {
                RootState::Live => ConversationState::Idle,
                RootState::Dead => ConversationState::Failed,
            },
            is_remote: false,
            notices: self.notices.clone(),
        }
    }

    /// The client's attachments for this root: the root surface first
    /// (`is_root: true`), then `members` in display order.
    ///
    /// # Every `id` is MINTED (invariant 6, finding 3)
    /// `id: mint_surface_id(self.id, &self.address)` for the root, and
    /// `mint_surface_id(self.id, address)` for each member — the ADDRESS is
    /// `members`' entry, the ID is the label. `short_name` is applied to the
    /// ADDRESS for the label, never to the id (an id may be `agent:w1@c2`, which
    /// would render as `w1@c2` in the sidebar and diverge from the roster).
    ///
    /// # Errors
    /// `RootError::AlreadyLive` is not reachable here; the `Result` exists so a
    /// surface that cannot be built is reported instead of silently dropped — a
    /// surface the client never learns about is a backend it can never address.
    pub fn surface_specs(&self) -> Result<Vec<wcode_tui::SurfaceSpec>, RootError> {
        let mut specs = Vec::with_capacity(1 + self.members.len());
        // The root row: labelled `wcode` (the header's reverse-video run
        // lowercased, D018/O3), its backend the root's own actor.
        specs.push(wcode_tui::SurfaceSpec {
            id: mint_surface_id(self.id, &self.address),
            label: "wcode".to_string(),
            model: self.model.clone(),
            is_root: true,
            resume_path: self.group.as_ref().map(|g| g.dir.clone()).or_else(|| self.session_path.clone()),
            backend: wcode_protocol::Backend::Local(self.handle.clone()),
        });
        for address in &self.members {
            let name = crate::agents::short_name(address);
            // A member's model comes from its recorded launch spec when this root
            // has a group (a resumed team keeps each worker's own), and from the
            // root's otherwise — the member list carries addresses, not models.
            let model = self
                .group
                .as_ref()
                .map(|group| crate::session_groups::member_spec(group, &name).model)
                .and_then(|m| m.or(Some(self.model.clone())))
                .unwrap_or_else(|| self.model.clone());
            let backend = self
                .orchestrator
                .as_ref()
                .and_then(|o| o.worker_backend(&name))
                .ok_or_else(|| RootError::AlreadyLive(format!("member `{name}` is not live")))?;
            specs.push(wcode_tui::SurfaceSpec {
                id: mint_surface_id(self.id, address),
                label: name,
                model,
                is_root: false,
                resume_path: None,
                backend,
            });
        }
        Ok(specs)
    }

    // Read by nobody yet (amendment (k)): `serve()` still hand-rolls its root
    // tuple. It retires when `serve()` adopts a root, or with W041's hub.
    #[allow(dead_code)]
    /// The `serve`-shaped view of this root — **no TUI type**, so a second host
    /// could pass it straight to `serve_at`/`serve_stdio` (candidate reuse,
    /// finding 6 open): the root tuple those take plus the live roster. This is
    /// the seam W041 would consume if the human picks resolution (D).
    pub fn as_protocol_root(&self) -> (wcode_protocol::Root, Roster) {
        (
            (self.address.clone(), self.handle.clone()),
            self.roster.clone(),
        )
    }
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
    /// run is ever started by these tests. Its `Background` — the one the tools
    /// hold and `adopt` receives, so the notifier cycle is the production graph —
    /// is tagged with `id`, exactly as `build_agent`'s callers now do it
    /// (§13.2.1(a): a parameter, not ambient state).
    fn build(id: ConversationId) -> (Agent, Arc<Background>) {
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
            id,
        )
    }

    /// An orchestrator whose workers belong to `id` — the runtime-spawn path
    /// (`SessionFactory::build_with`) without a network.
    fn orchestrator_in(id: ConversationId) -> crate::agents::Orchestrator {
        use wcode_harness::streamfn::{LlmStream, StreamFn};
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        crate::agents::Orchestrator::new(
            Registry::new(),
            crate::agents::WorkerTemplate {
                system: "sys".into(),
                llm: LlmOpts {
                    model: "m1".into(),
                    ..LlmOpts::default()
                },
                stream_fn,
                hooks: HooksSet::default(),
                tools: ToolsConfig::default(),
                compaction: CompactionPolicy::default(),
                working_dir: std::env::temp_dir(),
                members_dir: None,
                sessions_dir: std::env::temp_dir(),
            },
            id,
        )
    }

    /// The parts for a root of `id`, with `group` when the test wants one. The
    /// `Background` is the one `build_agent` created and tagged with `id` — the
    /// production graph — and the sink is the registry's, not ours (there is no
    /// `events` field: `Roots::adopt` supplies it, §13.2.1(c)).
    fn parts(id: ConversationId, group: Option<SessionGroup>) -> RootParts {
        let (agent, bg) = build(id);
        RootParts {
            id,
            label: format!("label-{id}"),
            agent,
            bg,
            orchestrator: None,
            group,
            session_path: None,
            model: "m1".into(),
            members: Vec::new(),
            notices: Vec::new(),
        }
    }

    /// Adopt + insert in one call: the shape every test uses.
    fn add(roots: &mut Roots, id: ConversationId, group: Option<SessionGroup>) {
        let root = roots.adopt(parts(id, group));
        roots.insert(root).expect("inserted");
    }

    fn channels() -> (
        mpsc::UnboundedSender<RootEvent>,
        mpsc::UnboundedReceiver<RootEvent>,
    ) {
        mpsc::unbounded_channel()
    }

    /// A `Roots` whose mint starts at `base`, so each test owns a disjoint id
    /// block. Every `Roots` mint starts at 1 by contract, and these tests run in
    /// parallel in one process: without blocks, a `shutdown_scope(c1)` in one test
    /// would release a `Background` another test tagged `c1`. The skipped ids are
    /// burned (never live, never reused) — `base` is a test allocation, not a
    /// production rule.
    fn roots_in(tx: mpsc::UnboundedSender<RootEvent>, base: u64) -> Roots {
        let mut roots = Roots::new(tx);
        for _ in 1..base {
            roots.mint();
        }
        roots
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
        let mut mints = roots_in(tx.clone(), 90);
        let c0 = ConversationId::first();
        let c1 = mints.mint();
        let mut reg_a = roots_in(tx.clone(), 91);
        let mut reg_b = roots_in(tx.clone(), 92);
        let a = reg_a.adopt(parts(c0, None));
        let b = reg_b.adopt(parts(c1, None));

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
        let mut roots = roots_in(tx.clone(), 20);
        let c1 = roots.mint();
        add(&mut roots, c1, None);

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
        // Scopes nobody else in this test binary closes (see `roots_in`).
        let c0 = ConversationId::new(9001);
        let c1 = ConversationId::new(9002);
        let a = Background::new(c0);
        let b = Background::new(c1);
        let c = Background::new(c1);
        Background::shutdown_scope(c1);
        assert!(b.is_notifier_released());
        assert!(c.is_notifier_released());
        assert!(
            !a.is_notifier_released(),
            "c0's background must survive c1's scope teardown"
        );
        Background::shutdown_scope(ConversationId::new(77)); // unknown: a no-op
        assert!(!a.is_notifier_released());
        // And the same rule at the Roots level: closing a conversation releases
        // ITS root's notifier, and nobody else's.
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 80);
        let root0 = roots.mint();
        let root1 = roots.mint();
        add(&mut roots, root0, None);
        add(&mut roots, root1, None);
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
        let mut roots = roots_in(tx.clone(), 30);
        let c1 = roots.mint();
        add(&mut roots, c0, None);
        add(&mut roots, c1, None);

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
    }

    #[tokio::test]
    async fn dead_is_sticky() {
        let (tx, _rx) = channels();
        let c0 = ConversationId::first();
        let mut roots = roots_in(tx.clone(), 100);
        add(&mut roots, c0, None);
        assert!(roots.set_state(c0, RootState::Dead));
        assert!(
            !roots.set_state(c0, RootState::Live),
            "Dead is terminal: a later Live cannot downgrade it"
        );
        assert_eq!(roots.state_of(c0), Some(RootState::Dead));
        assert!(roots.set_state(c0, RootState::Dead), "re-setting Dead is fine");
        assert!(!roots.set_state(ConversationId::new(99), RootState::Dead));
    }

    #[tokio::test]
    async fn a_second_root_cannot_own_a_live_group_dir() {
        // `Resume` refuses a dir already served: two writers to one `root.jsonl`
        // would interleave two transcripts.
        let tmp = tempfile::tempdir().expect("a tempdir");
        let group = crate::session_groups::create_group(tmp.path()).expect("a group");
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 60);
        add(&mut roots, ConversationId::first(), Some(group.clone()));

        assert!(roots.contains_session_path(&group.dir));
        let dup_id = roots.mint();
        let dup = roots.adopt(parts(dup_id, Some(group.clone())));
        match roots.insert(dup) {
            Err(RootError::AlreadyLive(what)) => {
                assert!(what.contains("_"), "names the group dir: {what}")
            }
            other => panic!("expected AlreadyLive, got {other:?}"),
        }
        assert_eq!(roots.len(), 1, "nothing half-inserted");
    }

    /// The factory's inputs, deterministically: a default config (no file, no
    /// env), no team, no network (the refused-port base URL is never dialled —
    /// these tests build roots, they do not run them).
    fn run_inputs(dir: &Path) -> RunInputs {
        let args = crate::Args {
            no_project_config: true,
            ..crate::Args::default()
        };
        // `merge` insists on a model; the startup path always resolves one from
        // flags/env/config, and here it is the fixture's.
        let cfg = crate::config::merge(
            crate::config::EnvLike::default(),
            crate::config::FileConfig {
                model: Some("m1".into()),
                ..crate::config::FileConfig::default()
            },
        )
        .expect("a default config");
        RunInputs {
            args,
            cfg,
            llm: LlmOpts {
                model: "m1".into(),
                base_url: Some("http://127.0.0.1:9/v1".into()),
                ..LlmOpts::default()
            },
            hooks: HooksSet::default(),
            tools: ToolsConfig::default(),
            compaction: CompactionPolicy::default(),
            title_after_turns: 0,
            instructions: InstructionSet::default(),
            skills: SkillSet::default(),
            team: Vec::new(),
            guidelines: None,
            overlay: None,
            no_project_config: true,
            owner: None,
            name: None,
            orchestrator: None,
            cwd: dir.to_path_buf(),
            dir: dir.to_path_buf(),
        }
    }

    /// The factory half of W040 commit 4: two `Fresh` builds mint two distinct
    /// group dirs (the fs guarantees it — `create_group` uses `create_dir`), and a
    /// `Resume` of one returns THAT dir, with the session's model restored.
    #[tokio::test]
    async fn fresh_mints_a_distinct_group_dir_and_resume_reuses_one() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let factory = factory_for(run_inputs(tmp.path()));
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 200);

        let a = roots
            .start(&factory, RootSpec::Fresh { label: None })
            .expect("a fresh root");
        // Fresh root's surface_specs carries its group dir as resume_path.
        let group_dir = roots.get(a).unwrap().group.as_ref().unwrap().dir.clone();
        assert_eq!(
            roots.get(a).unwrap().surface_specs().unwrap()[0].resume_path.as_deref(),
            Some(group_dir.as_path())
        );
        let b = roots
            .start(&factory, RootSpec::Fresh { label: None })
            .expect("a second fresh root");
        let dir_a = roots
            .get(a)
            .expect("live")
            .group
            .as_ref()
            .expect("a group")
            .dir
            .clone();
        let dir_b = roots
            .get(b)
            .expect("live")
            .group
            .as_ref()
            .expect("a group")
            .dir
            .clone();
        assert_ne!(dir_a, dir_b, "each Fresh root mints its own group dir");
        for dir in [&dir_a, &dir_b] {
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            assert!(
                crate::session_groups::is_group_dir_name(name),
                "the dir name is a group id: {name}"
            );
            assert!(dir.join("root.jsonl").is_file(), "root.jsonl exists");
        }
        // The label's last resort: the cwd basename, since nothing named it.
        assert_eq!(
            roots.get(a).expect("live").label,
            tmp.path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "an unnamed root takes the cwd's basename (§13.6.1)"
        );

        // Resume the first dir: the same dir, and the session's model back. `a`
        // must be closed first — a LIVE root over the dir is what the guard
        // refuses (two writers to one `root.jsonl`).
        assert!(roots.close(a), "the first root closes, its files stay");
        // A SECOND input set whose launch model is `m9` and no `--model` flag: the
        // assertion below can only hold if the resume restored the session's own
        // model. With the fixture's own `m1`, "restored" and "never restored"
        // would be the same value.
        //
        // A never-run group records no `ModelChange`, so write the entry a run
        // would have written — that is exactly what "the session's model" is.
        {
            use wcode_harness::session::{Session, SessionEntry};
            let mut session =
                Session::open(&dir_a.join("root.jsonl")).expect("the root transcript");
            session
                .append(SessionEntry::ModelChange {
                    id: "test".into(),
                    model: "m1".into(),
                })
                .expect("record the model");
        }
        let mut resume_inputs = run_inputs(tmp.path());
        resume_inputs.llm.model = "m9".into();
        let resume_factory = factory_for(resume_inputs);
        let c = roots
            .start(
                &resume_factory,
                RootSpec::Resume {
                    dir: dir_a.clone(),
                },
            )
            .expect("the resume builds");
        let resumed = roots.get(c).expect("live");
        assert_eq!(
            resumed.group.as_ref().expect("a group").dir,
            dir_a,
            "Resume reuses the group dir"
        );
        assert_eq!(
            resumed.model, "m1",
            "the session's model overrode the launch default `m9`"
        );

        // A second live root over one dir is refused BEFORE the build.
        let err = roots
            .start(
                &resume_factory,
                RootSpec::Resume {
                    dir: dir_a.clone(),
                },
            )
            .expect_err("a live Resume dir is refused");
        assert!(matches!(err, RootError::AlreadyLive(_)), "{err}");
    }

    /// The failure path the review caught: `build_runtime` spawns `[team]`
    /// members as it goes, so a member that fails validation AFTER earlier ones
    /// spawned leaves every one of those actors parked (its `Background` holds
    /// the notifier) plus a `Registry` watcher each — and, before this fix, an
    /// empty group dir that `/resume` would offer. `build_root` now tears a
    /// partial build down with the same tool `Roots::close` uses.
    #[tokio::test]
    async fn a_failed_team_build_releases_what_it_already_spawned() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let mut inputs = run_inputs(tmp.path());
        inputs.args.agents = true;
        inputs.cfg.team = vec![
            crate::config::TeamMember {
                name: "w1".into(),
                ..crate::config::TeamMember::default()
            },
            // `spawn` is refused for a worker (`agents.rs:281`) — and refused
            // only AFTER w1 has been spawned and bound.
            crate::config::TeamMember {
                name: "w2".into(),
                tools: Some(vec!["spawn".into()]),
                ..crate::config::TeamMember::default()
            },
        ];
        let factory = factory_for(inputs);
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 300);
        let before = roots.len();

        let err = roots
            .start(&factory, RootSpec::Fresh { label: None })
            .expect_err("the second member is refused");
        assert!(err.to_string().contains("spawn"), "names the tool: {err}");
        assert_eq!(roots.len(), before, "nothing half-inserted");

        // The failed build's id (block 300's first mint). w1's `Background` was
        // tagged with it, so a live entry means the teardown did NOT run.
        let failed = ConversationId::new(300);
        let parked: Vec<_> = Background::live_scopes()
            .into_iter()
            .filter(|scope| *scope == failed)
            .collect();
        assert!(
            parked.is_empty(),
            "the already-spawned worker was released, not parked: {parked:?}"
        );
        // And the group dir this build created is gone: `/resume` must not offer
        // a half-built conversation (`list_groups` has no content filter).
        let left: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("the tempdir")
            .collect();
        assert!(
            left.is_empty(),
            "a failed Fresh build removes the group dir it created"
        );
    }

    #[tokio::test]
    async fn a_closed_roots_files_remain_on_disk() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let group = crate::session_groups::create_group(tmp.path()).expect("a group");
        let root_path = group.root_path.clone();
        let members_dir = group.members_dir.clone();
        crate::session_groups::create_root_session(&group.dir, tmp.path()).expect("a session");

        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 50);
        // A MINTED id, not `first()`: `shutdown_scope(c0)` is process-wide for
        // every `Background` that was built outside a scoped build, which in a
        // `cargo test` process means other tests' fixtures. Conversation 0's own
        // teardown is the same code path.
        let id = roots.mint();
        add(&mut roots, id, Some(group.clone()));
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
        // `Roots::new` reserves seq 0: the first mint is c1, never c0. This test
        // owns block 1 on purpose — it asserts the mint sequence itself.
        assert_eq!(roots.mint(), ConversationId::new(1));

        let failing: Factories =
            Arc::new(|_id, _spec| Err(RootError::Config("bad [team] entry".into())));
        let err = roots
            .start(&failing, RootSpec::Fresh { label: None })
            .expect_err("the factory fails");
        assert!(err.to_string().contains("bad [team] entry"), "{err}");
        assert_eq!(roots.len(), 0, "nothing half-inserted");

        // The next attempt gets c3, not the c2 the failure consumed.
        // A factory returns PARTS (§13.2.1(c)): the sink is the registry's, so a
        // factory cannot even express a root reporting elsewhere.
        let ok: Factories = Arc::new(|id, _spec| Ok(parts(id, None)));
        let id = roots
            .start(&ok, RootSpec::Fresh { label: None })
            .expect("the second build");
        assert_eq!(id, ConversationId::new(3), "the failed id was burned");
        assert_eq!(roots.len(), 1);
        // And the reserved startup id is still free for the startup root.
        add(&mut roots, ConversationId::first(), None);
        assert_eq!(roots.len(), 2);
    }

    #[tokio::test]
    async fn a_resume_of_a_live_dir_is_refused_before_the_build() {
        // The pre-check is what keeps a failed `start` from leaking the actor it
        // just built: `insert` cannot fail after it.
        let tmp = tempfile::tempdir().expect("a tempdir");
        let group = crate::session_groups::create_group(tmp.path()).expect("a group");
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 70);
        add(&mut roots, ConversationId::first(), Some(group.clone()));

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
    async fn resume_from_a_legacy_flat_jsonl_builds_a_root() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let cwd = tmp.path();
        let session = crate::Session::create_with_cwd(cwd, cwd).expect("a session");
        let path = session.path().expect("path").to_path_buf();
        assert!(path.is_file(), "flat file exists");

        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 200);
        let inputs = run_inputs(cwd);
        let factory = factory_for(inputs);
        let id = roots
            .start(&factory, RootSpec::Resume { dir: path.clone() })
            .expect("legacy resume builds");
        let root = roots.get(id).expect("live");
        assert_eq!(root.session_path.as_deref(), Some(path.as_path()));
        assert!(root.group.is_none(), "no group for a legacy file");
        // no sibling members/ dir created for a legacy file
        assert!(!path.parent().unwrap().join("members").exists());
        // The production fill: surface_specs carries the resume path.
        assert_eq!(
            root.surface_specs().unwrap()[0].resume_path.as_deref(),
            Some(path.as_path())
        );
    }

    /// **The backend half of W049 §7.2 item 3.** A resumed tab renders its old
    /// turns from `GetHistory` (`crates/wcode-tui`'s `spawn_replay`) — that is the
    /// display half, checked on a pty. This pins the other half: the resumed
    /// root's agent really carries the session's messages, so what the tab shows
    /// and what the next prompt continues from are the same facts.
    ///
    /// `resume_from_a_legacy_flat_jsonl_builds_a_root` above asserts the paths
    /// (`session_path`, `resume_path`) only — never that the loaded messages
    /// survived.
    #[tokio::test]
    async fn a_resumed_root_answers_get_history_with_its_session() {
        use wcode_harness::message::{AgentMessage, ContentBlock, StopReason};
        use wcode_harness::session::{Session, SessionEntry};

        let tmp = tempfile::tempdir().expect("a tempdir");
        let cwd = tmp.path();
        let session = crate::Session::create_with_cwd(cwd, cwd).expect("a session");
        let path = session.path().expect("path").to_path_buf();
        // Two entries through the real writer — what a run would have recorded.
        {
            let mut session = Session::open(&path).expect("the transcript");
            for (n, message) in [
                AgentMessage::user_text("an earlier question"),
                AgentMessage::Assistant {
                    content: vec![ContentBlock::Text {
                        text: "an earlier answer".into(),
                    }],
                    stop_reason: StopReason::Stop,
                    usage: None,
                    model: None,
                },
            ]
            .into_iter()
            .enumerate()
            {
                session
                    .append(SessionEntry::Message {
                        id: format!("m{n}"),
                        parent_id: None,
                        message,
                    })
                    .expect("record the message");
            }
        }

        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 220);
        let factory = factory_for(run_inputs(cwd));
        let id = roots
            .start(&factory, RootSpec::Resume { dir: path.clone() })
            .expect("the resume builds");

        let reply = roots
            .get(id)
            .expect("live")
            .handle
            .ask(Request::GetHistory)
            .await
            .expect("the resumed root answers");
        let AgentEvent::History { messages } = reply else {
            panic!("expected History, got {reply:?}");
        };
        let text: Vec<String> = messages.iter().map(|m| m.as_text()).collect();
        assert!(
            text.iter().any(|t| t.contains("an earlier question")),
            "the user turn survived the resume: {text:?}"
        );
        assert!(
            text.iter().any(|t| t.contains("an earlier answer")),
            "and the assistant turn: {text:?}"
        );
    }

    #[tokio::test]
    async fn a_legacy_file_resumed_twice_is_refused() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let cwd = tmp.path();
        let session = crate::Session::create_with_cwd(cwd, cwd).expect("a session");
        let path = session.path().expect("path").to_path_buf();

        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 210);
        let inputs = run_inputs(cwd);
        let factory = factory_for(inputs);
        let _id = roots
            .start(&factory, RootSpec::Resume { dir: path.clone() })
            .expect("first resume builds");

        let never_called: Factories = Arc::new(|_id, _spec| {
            panic!("the factory must not run for a live legacy file")
        });
        let err = roots
            .start(&never_called, RootSpec::Resume { dir: path.clone() })
            .expect_err("refused");
        assert!(matches!(err, RootError::AlreadyLive(_)), "{err}");
    }

    #[tokio::test]
    async fn a_fresh_root_s_added_carries_the_team_notice() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let cwd = tmp.path();
        let mut inputs = run_inputs(cwd);
        inputs.args.agents = true;
        inputs.cfg.team = vec![
            crate::config::TeamMember { name: "a".into(), ..Default::default() },
            crate::config::TeamMember { name: "b".into(), ..Default::default() },
        ];
        let factory = factory_for(inputs);
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 300);
        let id = roots
            .start(&factory, RootSpec::Fresh { label: None })
            .expect("fresh team builds");
        let root = roots.get(id).expect("live");
        assert!(root.notices.iter().any(|n| n == "team: a, b"), "{:?}", root.notices);
        // probe second half: a root without a team carries no notice
        let inputs2 = run_inputs(cwd);
        let factory2 = factory_for(inputs2);
        let mut roots2 = roots_in(tx.clone(), 301);
        let id2 = roots2
            .start(&factory2, RootSpec::Fresh { label: None })
            .expect("fresh no-team builds");
        let root2 = roots2.get(id2).expect("live");
        assert!(root2.notices.is_empty(), "{:?}", root2.notices);
    }

    #[tokio::test]
    async fn a_resumed_team_says_it_restored() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let cwd = tmp.path();
        let group = crate::session_groups::create_group(cwd).expect("a group");
        crate::session_groups::create_root_session(&group.dir, cwd).expect("root session");
        // Seed a member transcript so rebuild_team has something to restore.
        let members_dir = group.members_dir.clone();
        std::fs::create_dir_all(&members_dir).expect("members dir");
        let _ = crate::Session::create_named_with_cwd(&members_dir, "w1", cwd).expect("member session");

        let mut inputs = run_inputs(cwd);
        inputs.args.agents = true;
        // team empty so fresh spawn doesn't run; resume uses files
        let factory = factory_for(inputs);
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 310);
        let id = roots
            .start(&factory, RootSpec::Resume { dir: group.dir.clone() })
            .expect("resumed group builds");
        let root = roots.get(id).expect("live");
        assert!(
            root.notices.iter().any(|n| n.contains("restored")),
            "{:?}",
            root.notices
        );
    }

    #[tokio::test]
    async fn a_resume_of_a_path_that_is_neither() {
        let tmp = tempfile::tempdir().expect("a tempdir");
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 220);
        let inputs = run_inputs(tmp.path());
        let factory = factory_for(inputs);
        let err = roots
            .start(
                &factory,
                RootSpec::Resume {
                    dir: tmp.path().join("nope.jsonl"),
                },
            )
            .expect_err("refused");
        assert!(matches!(err, RootError::BadResume(_)), "{err}");
        assert!(
            err.to_string().ends_with("is not a session"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn stop_is_close_and_an_unknown_id_is_a_no_op() {
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 10);
        let c0 = roots.mint();
        add(&mut roots, c0, None);
        assert!(roots.stop(c0));
        assert!(!roots.stop(c0), "idempotent: an unknown id is a no-op");
        assert!(roots.is_empty(), "the last close leaves an empty registry");
        roots.close_all(); // and close_all on an empty registry is a no-op
    }

    /// FINDING 1's gate — the test that would have caught the ambient-scope bug.
    ///
    /// `SessionFactory::build_with` is reached from the `team`/`spawn` tools
    /// *inside an actor's turn* (`tools/team.rs:139` → `agents.rs:734` →
    /// `:296` → `:399`), where no build was in progress. With an ambient scope and
    /// a `c0` default, every runtime-spawned worker of every conversation `c1`+
    /// was tagged `c0`: `close(c1)` never found it (child and actor both leak) and
    /// `close(c0)` signalled it. So: a worker spawned by a `c1` orchestrator must
    /// carry `c1`. Scope `9003` is private to this test, so the count is exact.
    #[tokio::test]
    async fn a_runtime_spawned_worker_is_tagged_with_its_orchestrators_conversation() {
        let id = ConversationId::new(9003);
        let orchestrator = orchestrator_in(id);
        let tagged = |id: ConversationId| {
            Background::live_scopes()
                .into_iter()
                .filter(|scope| *scope == id)
                .count()
        };
        let before = tagged(id);
        let worker = orchestrator
            .spawn_worker(crate::agents::WorkerSpec::default())
            .expect("a worker spawns");
        assert_eq!(
            tagged(id),
            before + 1,
            "the worker's Background carries the orchestrator's conversation ({id}), not c0"
        );
        // Clean up: the worker's own scope teardown releases its notifier.
        Background::shutdown_scope(id);
        drop(worker);
    }

    /// §13.2.1(a)'s guard: a root whose `Background` was built for another
    /// conversation would be torn down by that conversation's close and leak its
    /// own worker's tasks. The assert is what makes the mistake loud at the call
    /// site instead of invisible at teardown.
    #[tokio::test]
    #[should_panic(expected = "a root's Background carries its own conversation tag")]
    async fn a_mis_tagged_background_trips_adopts_guard() {
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx, 9004);
        let (agent, bg) = build(ConversationId::new(9004)); // tagged 9004 …
        let _ = roots.adopt(RootParts {
            id: ConversationId::new(9005), // … adopted as 9005
            label: "mismatch".into(),
            agent,
            bg,
            orchestrator: None,
            group: None,
            session_path: None,
            model: "m1".into(),
            members: Vec::new(),
            notices: Vec::new(),
        });
    }

    /// FINDING 1's destructive half, and the reason the tag exists: closing one
    /// conversation signals ITS children and only its own. The previous gate
    /// observed a released notifier — it never observed a killed child.
    #[cfg(unix)]
    #[tokio::test]
    async fn closing_a_conversation_kills_only_its_own_background_children() {
        use std::os::unix::process::ExitStatusExt as _;

        fn sleeper() -> tokio::process::Child {
            let mut cmd = tokio::process::Command::new("sh");
            cmd.arg("-c").arg("sleep 30");
            // Its own process group, exactly as `Bash::execute` spawns one, so
            // `shutdown`'s group signal reaches it.
            crate::tools::background::spawn_group(&mut cmd);
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("a sleeper")
        }

        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 120);
        let c0 = ConversationId::first();
        let c1 = roots.mint();
        add(&mut roots, c0, None);
        add(&mut roots, c1, None);
        let (bg0, bg1) = (
            roots.get(c0).expect("live").bg.clone(),
            roots.get(c1).expect("live").bg.clone(),
        );

        // One real child per conversation, registered the way the `bash` tool
        // registers one (`Background::register(cmd, cwd, child.id())`).
        let mut child0 = sleeper();
        let mut child1 = sleeper();
        bg0.register("sleep 30", Path::new("."), child0.id());
        bg1.register("sleep 30", Path::new("."), child1.id());

        assert!(roots.close(c1), "the c1 conversation closes");

        // c1's child is SIGKILLed (by process group) …
        let killed = tokio::time::timeout(Duration::from_secs(5), child1.wait())
            .await
            .expect("c1's child ends")
            .expect("wait");
        assert_eq!(
            killed.signal(),
            Some(libc::SIGKILL),
            "c1's child was signalled, not merely abandoned: {killed:?}"
        );
        // … and c0's is untouched.
        assert!(
            child0.try_wait().expect("try_wait").is_none(),
            "c0's child still runs after c1 closed"
        );
        let _ = child0.start_kill();
        let _ = child0.wait().await;
    }

    /// **The vocabulary round-trips through the host loop** (the sketch's
    /// `the_request_and_event_vocabulary_round_trips`): `Start` mints, builds,
    /// inserts and answers `Added` with root-first surfaces, and `Stop` closes and
    /// echoes `Stopped`.
    #[tokio::test]
    async fn the_request_and_event_vocabulary_round_trips() {
        use wcode_tui::conversations::{ConversationEvent, ConversationRequest};

        let tmp = tempfile::tempdir().expect("a tempdir");
        let (tx, rx) = channels();
        let mut roots = roots_in(tx, 400);
        let (requests, req_rx) = mpsc::unbounded_channel();
        let (evt_tx, mut events) = mpsc::unbounded_channel();
        let (surface_tx, _surface_rx) = mpsc::unbounded_channel();
        let host = tokio::spawn(run_host(
            {
                roots.close_all();
                roots
            },
            factory_for(run_inputs(tmp.path())),
            HostChannels {
                requests: req_rx,
                events: evt_tx,
                surfaces: surface_tx,
                root_events: rx,
            },
        ));

        requests
            .send(ConversationRequest::Start(RootSpec::Fresh { label: None }))
            .expect("the host is alive");
        let added = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("a reply")
            .expect("alive");
        let id = match added {
            ConversationEvent::Added(spec) => {
                assert_eq!(spec.id, ConversationId::new(400), "the first mint");
                assert_eq!(spec.surfaces.len(), 1, "a fresh root has no members");
                assert!(spec.surfaces[0].is_root);
                // The root's surface id is `mint_surface_id(id, address)`, so it
                // matches the mint rule for whatever id this host handed out.
                assert_eq!(
                    spec.surfaces[0].id,
                    mint_surface_id(spec.id, &SessionId::agent("root")),
                    "the root row is the minted address"
                );
                assert_eq!(spec.surfaces[0].label, "wcode");
                spec.id
            }
            _ => panic!("expected Added"),
        };

        requests
            .send(ConversationRequest::Stop(id))
            .expect("the host is alive");
        let stopped = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("a reply")
            .expect("alive");
        match stopped {
            ConversationEvent::Stopped { id: gone } => assert_eq!(gone, id),
            _ => panic!("expected Stopped"),
        }

        // FINDING 8's end-to-end prediction: a CLOSED id is never reused — the
        // next `Start` mints the NEXT one (the same rule `a_failed_build_…` pins
        // for a failed build; this is the close/`Stopped` path).
        requests
            .send(ConversationRequest::Start(RootSpec::Fresh { label: None }))
            .expect("the host is alive");
        match tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("a reply")
            .expect("alive")
        {
            ConversationEvent::Added(spec) => assert_eq!(
                spec.id,
                ConversationId::new(401),
                "the closed c400 is not reused; the mint advanced"
            ),
            _ => panic!("expected Added"),
        }

        drop(requests);
        let roots = tokio::time::timeout(Duration::from_secs(5), host)
            .await
            .expect("the host returns")
            .expect("no panic");
        assert_eq!(
            roots.len(),
            1,
            "Start added it, Stop removed it, and the SECOND Start (the id-burn \
             check) is still live when the client goes away"
        );
    }

    /// FINDING 3's gate: a member's surface id is MINTED, its A2A address is not,
    /// and the label comes from the address. Conversation 0 keeps the bare ids.
    #[tokio::test]
    async fn every_members_surface_id_is_minted_not_its_a2a_address() {
        let (tx, _rx) = channels();
        let mut roots = roots_in(tx.clone(), 500);
        let c0 = ConversationId::first();
        let c1 = roots.mint();

        // `surface_specs` needs the workers to be live in the orchestrator, so
        // build them: the same path a `[team]` takes at startup.
        let build = |id: ConversationId| {
            let o = orchestrator_in(id);
            let mut addresses = Vec::new();
            for name in ["w1", "w2"] {
                let worker = o
                    .spawn_worker(crate::agents::WorkerSpec {
                        name: Some(name.to_string()),
                        ..Default::default()
                    })
                    .expect("a worker spawns");
                addresses.push(worker.id);
            }
            let (agent, bg) = build(id);
            RootParts {
                id,
                label: format!("label-{id}"),
                agent,
                bg,
                orchestrator: Some(o),
                group: None,
                session_path: None,
                model: "m1".into(),
                members: addresses,
                notices: Vec::new(),
            }
        };
        for id in [c0, c1] {
            let parts = build(id);
            let root = roots.adopt(parts);
            roots.insert(root).expect("inserted");
        }

        let zero = roots.get(c0).expect("live").surface_specs().expect("specs");
        let one = roots.get(c1).expect("live").surface_specs().expect("specs");
        assert_eq!(zero.len(), 3, "root + two members");
        assert_eq!(one.len(), 3);

        // N = 1: the bare ids and the plain labels (the root's address is the
        // orchestrator's, `agent:orchestrator`, as in any `--agents` run).
        assert_eq!(zero[0].id.to_string(), "agent:orchestrator");
        assert_eq!(zero[1].id.to_string(), "agent:w1");
        assert_eq!(zero[1].label, "w1", "the label is the address's short name");
        // N = 2: minted ids, and the SAME plain labels.
        assert_eq!(one[0].id.to_string(), "agent:orchestrator@c500");
        assert_eq!(one[1].id.to_string(), "agent:w1@c500", "minted per surface");
        assert_eq!(one[1].label, "w1", "never `w1@c500` in the sidebar");
        // No two entries across the two roots share an id (the flat-map collision).
        let all: Vec<String> = zero
            .iter()
            .chain(one.iter())
            .map(|s| s.id.to_string())
            .collect();
        let mut deduped = all.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(all.len(), deduped.len(), "ids are unique across roots: {all:?}");

        // The ADDRESS still works in the registry's space, and the minted id does
        // not — the invariant-6 separation, asserted rather than described.
        let root = roots.get(c1).expect("live");
        let address = SessionId::agent("w1");
        assert!(root.members.contains(&address), "members hold ADDRESSES");
        assert!(
            !root.registry.contains(&one[1].id),
            "a surface id is never a registry key"
        );
    }

    /// A `Start` whose reply cannot be delivered (the client quit mid-build) closes
    /// the root it just built, instead of leaking its actor, its `bg` scope and its
    /// session dir.
    #[tokio::test]
    async fn a_failed_send_tears_the_new_root_down() {
        use wcode_tui::conversations::ConversationRequest;

        let (tx, rx) = channels();
        let mut roots = roots_in(tx, 600);
        let (requests, req_rx) = mpsc::unbounded_channel();
        let (evt_tx, events) = mpsc::unbounded_channel();
        let (surface_tx, _surface_rx) = mpsc::unbounded_channel();
        // Nobody is listening: the `Added` reply will fail.
        drop(events);
        let host = tokio::spawn(run_host(
            {
                roots.close_all();
                roots
            },
            factory_for(run_inputs(Path::new("/tmp"))),
            HostChannels {
                requests: req_rx,
                events: evt_tx,
                surfaces: surface_tx,
                root_events: rx,
            },
        ));
        requests
            .send(ConversationRequest::Start(RootSpec::Fresh { label: None }))
            .expect("the host is alive");
        drop(requests);
        let roots = tokio::time::timeout(Duration::from_secs(5), host)
            .await
            .expect("the host returns")
            .expect("no panic");
        assert_eq!(roots.len(), 0, "the fresh root was closed, not kept");
        let parked: Vec<_> = Background::live_scopes()
            .into_iter()
            .filter(|scope| *scope == ConversationId::new(600))
            .collect();
        assert!(
            parked.is_empty(),
            "and its bg scope was released: {parked:?}"
        );
    }

    #[tokio::test]
    async fn a_root_error_says_what_a_human_must_do() {
        assert_eq!(
            RootError::BadResume(PathBuf::from("/tmp/x")).to_string(),
            "/tmp/x is not a session"
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

    /// `Roots::adopt` wires the supervisor to **the registry's own** sink
    /// (§13.2.1(c)): a root whose death nobody hears is the invisible failure
    /// item 4 exists to end, so the wiring gets its own test.
    #[tokio::test]
    async fn the_supervisor_reports_on_the_registrys_sink() {
        let (tx, mut rx) = channels();
        let mut roots = roots_in(tx.clone(), 40);
        let id = roots.mint();
        let mut root = roots.adopt(parts(id, None));
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
