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

use wcode_harness::actor::SessionHandle;
use wcode_harness::protocol::{Request, SessionId};

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

#[derive(Default)]
struct Inner {
    peers: Mutex<HashMap<SessionId, Backend>>,
    owners: Mutex<HashMap<SessionId, SessionId>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a peer's mailbox at `id`; re-registering replaces it.
    pub fn register(&self, id: SessionId, handle: SessionHandle) {
        self.inner
            .peers
            .lock()
            .unwrap()
            .insert(id, Backend::Local(handle));
    }

    /// Register a peer reachable **across a socket** — a served session this
    /// process can message, and which can reach back if it knows us (§8, S4-4).
    #[cfg(unix)]
    pub fn register_remote(&self, id: SessionId, client: Client) {
        self.inner
            .peers
            .lock()
            .unwrap()
            .insert(id, Backend::Remote(client));
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
    use wcode_harness::event::AgentEvent;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    /// A live session whose model never streams — enough for routing tests (the
    /// messages we send are `Notify`, which run no turn).
    fn session() -> SessionHandle {
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
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
