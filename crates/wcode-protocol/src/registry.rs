//! In-process address book + router — the A2A bus (§8 #3, §10.1).
//!
//! Two pieces: an **address book** (`SessionId` → `Backend`: an in-process
//! handle or a remote `Client`) and an **ownership** map (the `report_back_to`
//! **ownership** map (the `report_back_to` edge: worker → orchestrator). Routing
//! a message checks the **permitted set** — an orchestrator reaches its workers,
//! a worker its orchestrator, and nothing else (v1 star, §10.1) — then delivers
//! through the target's handle carrying the sender, so the recipient can
//! attribute it (`AgentEvent::MessageReceived { from }`).
//!
//! Addressing lives *here*, one layer above the kernel: a [`Request`] is
//! target-side and names no peer; the router turns a `to` address into a mailbox
//! given the sender's scope. The future **group** mode widens only the permitted
//! set (§10.1) — nothing else here changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use wcode_harness::actor::SessionHandle;
use wcode_harness::protocol::{MemberState, Request, SessionId};

use crate::backend::Backend;
#[cfg(unix)]
use crate::client::Client;

/// Why a message could not be routed (§10.1 enforcement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// No peer is registered at that address.
    Unknown(SessionId),
    /// The sender may not address that peer — outside the permitted set.
    NotPermitted { from: SessionId, to: SessionId },
    /// The target's session has shut down (its actor ended).
    Closed,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::Unknown(id) => write!(f, "no peer at {id}"),
            RouteError::NotPermitted { from, to } => {
                write!(f, "{from} may not address {to}")
            }
            RouteError::Closed => f.write_str("the target session is closed"),
        }
    }
}

impl std::error::Error for RouteError {}

/// An in-process address book and router over session mailboxes.
///
/// Cloneable and shared: every clone refers to the same registry. There is no
/// automatic deregistration — a peer that shuts down stays listed, and a delivery
/// to it fails with [`RouteError::Closed`] (a sweep can come later; §10.1 keeps
/// this minimal).
#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<Inner>,
}

struct Inner {
    peers: Mutex<HashMap<SessionId, Backend>>,
    owners: Mutex<HashMap<SessionId, SessionId>>,
    /// Each session's effective model id (see [`Registry::set_model`]), so a
    /// socket server can name a member's model in the roster it pushes.
    models: Mutex<HashMap<SessionId, String>>,
    /// Server-side liveness per session, mirrored from the actor's `watch` cell
    /// so the roster push (and the `Sessions` reply) can name it — the twin of
    /// `models`. Cheap: one entry per served session. `Idle` for a session the
    /// registry never watched, or an older peer (a `state`-less `SessionInfo`).
    states: Mutex<HashMap<SessionId, MemberState>>,
    /// The live **local** roster (see [`Registry::locals`]), refreshed on every
    /// `register`/`register_remote` so a socket server can fan a session that
    /// appears after it started. Read via [`Registry::subscribe`].
    roster: watch::Sender<Vec<(SessionId, SessionHandle)>>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
            owners: Mutex::new(HashMap::new()),
            models: Mutex::new(HashMap::new()),
            states: Mutex::new(HashMap::new()),
            roster: watch::channel(Vec::new()).0,
        }
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a peer's mailbox at `id`; re-registering replaces it.
    pub fn register(&self, id: SessionId, handle: SessionHandle) {
        // Mirror the session's run-state into the roster: the registry keeps
        // NO back-reference into the kernel (layering) — it watches the
        // handle's `state_rx()` and re-publishes on every transition, so the
        // socket server pushes the change without polling. One task per served
        // session (a handful); it ends when the session drops.
        let mut state_rx = handle.state_rx();
        self.inner
            .peers
            .lock()
            .unwrap()
            .insert(id.clone(), Backend::Local(handle));
        let registry = self.clone();
        tokio::spawn(async move {
            while state_rx.changed().await.is_ok() {
                registry.set_state(id.clone(), *state_rx.borrow_and_update());
            }
        });
        self.refresh_roster();
    }

    /// Register a peer reachable **across a socket** — a served session this
    /// process can message, and which can reach back if it knows us (§8, S4-4).
    #[cfg(unix)]
    pub fn register_remote(&self, id: SessionId, client: Client) {
        self.inner
            .peers
            .lock()
            .unwrap()
            .insert(id, Backend::Remote(client.clone()));
        // Remote liveness rides the peer's own roster push: a served peer
        // stamps each `SessionInfo` with a `state`, so mirroring the remote
        // client's roster into `states` makes `state_of` reflect a remote
        // member with no back-channel of our own.
        let registry = self.clone();
        let mut roster = client.subscribe_roster();
        tokio::spawn(async move {
            while roster.changed().await.is_ok() {
                let sessions = roster.borrow_and_update().clone();
                for info in &sessions {
                    registry.set_state(info.id.clone(), info.state);
                }
            }
        });
        self.refresh_roster();
    }

    /// The **local** peers — the in-process handles this registry holds, sorted
    /// by address for a deterministic roster. A socket server serves exactly
    /// these ([`Self::register`]ed sessions: the root and its team); a peer
    /// reached across a socket ([`Self::register_remote`]) is not a local handle
    /// and is omitted, so it is never re-served.
    pub fn locals(&self) -> Vec<(SessionId, SessionHandle)> {
        let mut locals: Vec<(SessionId, SessionHandle)> = self
            .inner
            .peers
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, backend)| match backend {
                Backend::Local(handle) => Some((id.clone(), handle.clone())),
                #[cfg(unix)]
                Backend::Remote(_) => None,
            })
            .collect();
        locals.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        locals
    }

    /// A live view of [`Self::locals`]: a receiver **seeded** with the current
    /// local roster and updated whenever it changes (a session is
    /// `register`ed). A socket server serves it, so a session that appears
    /// *after* the server started is still reachable — see [`crate::serve`].
    pub fn subscribe(&self) -> watch::Receiver<Vec<(SessionId, SessionHandle)>> {
        self.inner.roster.subscribe()
    }

    /// Recompute the local roster and publish it to every [`Self::subscribe`]r.
    /// `send_replace` stores the value even when nobody is watching yet, so a
    /// session registered *before* the first subscriber — a `[team]` member that
    /// starts ahead of the socket server — still seeds a later [`Self::subscribe`].
    /// (`watch::Sender::send` would drop it: it is a no-op with no receivers.)
    fn refresh_roster(&self) {
        let _ = self.inner.roster.send_replace(self.locals());
    }

    /// Whether a peer is already registered at `id`. A plain existence probe —
    /// unlike [`Self::resolve`], it needs no `from` and enforces no permitted set.
    pub fn contains(&self, id: &SessionId) -> bool {
        self.inner.peers.lock().unwrap().contains_key(id)
    }

    /// Record the `report_back_to` edge: `worker` reports to `owner` (its
    /// orchestrator). This is the whole ownership tree — a star in v1.
    pub fn set_owner(&self, worker: SessionId, owner: SessionId) {
        self.inner.owners.lock().unwrap().insert(worker, owner);
    }

    /// Record a session's effective model id, so the roster this registry feeds
    /// (a socket server's `Sessions` push) can name it. Mirrors [`Self::set_owner`]
    /// — a plain metadata write, no effect on routing.
    pub fn set_model(&self, id: SessionId, model: impl Into<String>) {
        self.inner.models.lock().unwrap().insert(id, model.into());
    }

    /// Mirror a session's run-state. Called by the per-session watcher
    /// `register` spawns — NOT by the actor, so the registry keeps NO
    /// back-reference into the kernel (layering). Re-publishes the roster, so
    /// the socket server pushes the change without polling.
    pub fn set_state(&self, id: SessionId, state: MemberState) {
        self.inner.states.lock().unwrap().insert(id, state);
        self.refresh_roster();
    }

    /// The state last mirrored for `id`; `Idle` when unknown (a remote peer the
    /// registry never watched, or an older server).
    pub fn state_of(&self, id: &SessionId) -> MemberState {
        *self
            .inner
            .states
            .lock()
            .unwrap()
            .get(id)
            .unwrap_or(&MemberState::Idle)
    }
    /// The model recorded for `id`, if any ([`Self::set_model`]). `None` for a
    /// peer registered without one (e.g. a remote reached across a socket).
    pub fn model_of(&self, id: &SessionId) -> Option<String> {
        self.inner.models.lock().unwrap().get(id).cloned()
    }

    /// Whether `from` may address `to` (§10.1): the pair must be joined by an
    /// ownership edge in either direction — an orchestrator and one of its
    /// workers. There are no lateral (worker → worker) edges.
    pub fn permitted(&self, from: &SessionId, to: &SessionId) -> bool {
        let owners = self.inner.owners.lock().unwrap();
        owners.get(from) == Some(to) || owners.get(to) == Some(from)
    }

    /// Resolve `to` for `from`, enforcing the permitted set. Fails
    /// [`RouteError::Unknown`] if no peer is registered, else
    /// [`RouteError::NotPermitted`] if the sender is out of scope.
    pub fn resolve(&self, from: &SessionId, to: &SessionId) -> Result<Backend, RouteError> {
        let endpoint = self
            .inner
            .peers
            .lock()
            .unwrap()
            .get(to)
            .cloned()
            .ok_or_else(|| RouteError::Unknown(to.clone()))?;
        if !self.permitted(from, to) {
            return Err(RouteError::NotPermitted {
                from: from.clone(),
                to: to.clone(),
            });
        }
        Ok(endpoint)
    }

    /// Deliver a target-side `request` to `to`, attributed to `from`. The
    /// recipient sees it as if `from` had sent it directly
    /// ([`SessionHandle::send_from`]).
    pub fn deliver(
        &self,
        from: &SessionId,
        to: &SessionId,
        request: Request,
    ) -> Result<(), RouteError> {
        let endpoint = self.resolve(from, to)?;
        endpoint
            .send_from(from.clone(), request)
            .map_err(|_| RouteError::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use wcode_harness::actor::SessionActor;
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::{AgentEvent, LlmStreamEvent};
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    use futures::StreamExt as _;

    /// A live session whose model never streams — enough for routing tests (the
    /// messages we send are `Notify`, which run no turn).
    fn session() -> SessionHandle {
        session_with(Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::empty()) as LlmStream
        }))
    }

    /// A session over a caller-supplied stream — for tests that drive a real
    /// turn (a hang, a stream error), not just routing.
    fn session_with(stream_fn: StreamFn) -> SessionHandle {
        SessionActor::spawn(Agent::new(AgentConfig {
            system: "sys".into(),
            tools: Vec::new(),
            llm: LlmOpts {
                model: "m".into(),
                ..LlmOpts::default()
            },
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::path::PathBuf::new(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
            plan_mode: wcode_harness::hooks::PlanModeHandle::new(),
        }))
    }

    async fn next_message(
        rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>,
    ) -> AgentEvent {
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("an event")
            .expect("open")
    }

    #[tokio::test]
    async fn orchestrator_and_worker_reach_each_other() {
        let reg = Registry::new();
        let orch = SessionId::agent("orch");
        let worker = SessionId::agent("w1");
        let h_orch = session();
        let h_worker = session();
        reg.register(orch.clone(), h_orch.clone());
        reg.register(worker.clone(), h_worker.clone());
        reg.set_owner(worker.clone(), orch.clone());

        // Orchestrator → worker: the worker receives it, attributed to the orch.
        let mut rx = h_worker.subscribe();
        reg.deliver(
            &orch,
            &worker,
            Request::Notify {
                content: "do the thing".into(),
            },
        )
        .unwrap();
        match next_message(&mut rx).await {
            AgentEvent::MessageReceived { from, content } => {
                assert_eq!(from, orch);
                assert_eq!(content, "do the thing");
            }
            other => panic!("expected MessageReceived, got {other:?}"),
        }

        // Worker → orchestrator (report back): permitted.
        let mut rx = h_orch.subscribe();
        reg.deliver(
            &worker,
            &orch,
            Request::Notify {
                content: "done".into(),
            },
        )
        .unwrap();
        match next_message(&mut rx).await {
            AgentEvent::MessageReceived { from, content } => {
                assert_eq!(from, worker);
                assert_eq!(content, "done");
            }
            other => panic!("expected MessageReceived, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn lateral_message_between_workers_is_refused() {
        let reg = Registry::new();
        let orch = SessionId::agent("orch");
        let w1 = SessionId::agent("w1");
        let w2 = SessionId::agent("w2");
        for id in [&orch, &w1, &w2] {
            reg.register(id.clone(), session());
        }
        reg.set_owner(w1.clone(), orch.clone());
        reg.set_owner(w2.clone(), orch.clone());

        assert_eq!(
            reg.deliver(
                &w1,
                &w2,
                Request::Notify {
                    content: "psst".into()
                }
            ),
            Err(RouteError::NotPermitted {
                from: w1.clone(),
                to: w2.clone()
            })
        );
        assert!(!reg.permitted(&w1, &w2));
    }

    #[tokio::test]
    async fn unknown_target_is_reported() {
        let reg = Registry::new();
        let orch = SessionId::agent("orch");
        reg.register(orch.clone(), session());

        let ghost = SessionId::agent("ghost");
        assert_eq!(
            reg.deliver(&orch, &ghost, Request::Notify { content: "?".into() }),
            Err(RouteError::Unknown(ghost))
        );
    }

    #[tokio::test]
    async fn locals_lists_only_local_peers_sorted_by_id() {
        let reg = Registry::new();
        let b = SessionId::agent("b");
        let a = SessionId::agent("a");
        reg.register(b.clone(), session());
        reg.register(a.clone(), session());
        #[cfg(unix)]
        reg.register_remote(
            SessionId::agent("remote"),
            Client::lazy(std::path::Path::new("/nonexistent-t2.sock")),
        );
        let ids: Vec<SessionId> = reg.locals().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![a, b]);
    }

    #[tokio::test]
    async fn set_model_is_read_back_per_id() {
        let reg = Registry::new();
        let w1 = SessionId::agent("w1");
        let ghost = SessionId::agent("ghost");
        assert_eq!(reg.model_of(&w1), None, "unset id has no model");
        reg.set_model(w1.clone(), "m1");
        assert_eq!(reg.model_of(&w1).as_deref(), Some("m1"));
        assert_eq!(reg.model_of(&ghost), None);
    }

    #[tokio::test]
    async fn set_state_is_read_back_and_republishes() {
        let reg = Registry::new();
        let w1 = SessionId::agent("w1");
        assert_eq!(reg.state_of(&w1), MemberState::Idle, "unset id reads Idle");

        let mut rx = reg.subscribe();
        reg.set_state(w1.clone(), MemberState::Running);
        assert_eq!(reg.state_of(&w1), MemberState::Running);
        // `set_state` re-publishes, so a subscriber (the server's push) wakes.
        tokio::time::timeout(Duration::from_secs(2), rx.changed())
            .await
            .expect("a roster change")
            .expect("open");
    }

    #[tokio::test]
    async fn register_mirrors_the_actor_state_into_the_roster() {
        // A stream that emits one delta then hangs: Running is observable until
        // a cancel ends the run.
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            let head =
                futures::stream::iter(vec![LlmStreamEvent::TextDelta("part".into())]);
            Box::pin(head.chain(futures::stream::pending())) as LlmStream
        });
        let handle = session_with(stream_fn);
        let reg = Registry::new();
        let w1 = SessionId::agent("w1");
        reg.register(w1.clone(), handle.clone());
        assert_eq!(reg.state_of(&w1), MemberState::Idle, "a virgin session is idle");

        handle.send(Request::Submit { text: "hi".into() }).unwrap();
        let mut running = false;
        for _ in 0..500 {
            if reg.state_of(&w1) == MemberState::Running {
                running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(running, "the registry mirrors the actor's Running");

        handle.send(Request::Cancel).unwrap();
        let mut done = false;
        for _ in 0..500 {
            if reg.state_of(&w1) == MemberState::Done {
                done = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(done, "the registry mirrors the actor's Done");
    }

    #[tokio::test]
    async fn subscribe_yields_the_new_roster_after_register() {
        let reg = Registry::new();
        let mut rx = reg.subscribe();
        let w1 = SessionId::agent("w1");
        reg.register(w1.clone(), session());
        rx.changed().await.unwrap();
        let ids: Vec<SessionId> = rx.borrow().iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ids, vec![w1]);
    }

    #[tokio::test]
    async fn subscribe_seeds_a_session_registered_before_it() {
        // A session registered with **no** subscriber in existence must still be
        // visible to a `subscribe()` that starts later — the `[team]` startup
        // members, registered before the socket server's first `subscribe()`.
        // (`watch::Sender::send` drops a value with no receivers; `send_replace`
        // keeps it, which is what `refresh_roster` relies on.)
        let reg = Registry::new();
        let w1 = SessionId::agent("w1");
        reg.register(w1.clone(), session());
        let rx = reg.subscribe();
        let ids: Vec<SessionId> = rx.borrow().iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ids, vec![w1]);
    }

    #[tokio::test]
    async fn an_unowned_pair_is_not_permitted() {
        let reg = Registry::new();
        let a = SessionId::agent("a");
        let b = SessionId::agent("b");
        reg.register(a.clone(), session());
        reg.register(b.clone(), session());
        // No ownership edge recorded → they may not address each other.
        assert_eq!(
            reg.deliver(&a, &b, Request::Notify { content: "?".into() }),
            Err(RouteError::NotPermitted {
                from: a.clone(),
                to: b.clone()
            })
        );
    }
}
