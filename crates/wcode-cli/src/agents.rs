//! A2A orchestration: build and register worker sessions (§10.1).
//!
//! A [`SessionFactory`] is the orchestrator's spawn primitive: it turns the
//! parent's config ([`WorkerTemplate`]) plus a [`WorkerSpec`] into a live worker
//! session — `Agent` built, actor spawned, registered in the [`Registry`] under
//! its owner. Only the root holds a factory, so only the root spawns (bounded
//! fan-out by construction); a worker's tool set has no `spawn`.
//!
//! v1 workers are **in-memory** (no session file) and inherit the parent's
//! model, hooks, and tools. The `WorkerSpec` is the seam for the orchestrator to
//! customize a worker later (system prompt, tool subset, path restrictions) —
//! those become a child `Hooks` impl, no config.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use wcode_harness::actor::{SessionActor, SessionHandle};
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::hooks::HooksSet;
use wcode_harness::loop_::DEFAULT_MAX_TURNS;
use wcode_harness::protocol::SessionId;
use wcode_harness::streamfn::{LlmOpts, StreamFn};
use wcode_harness::tool::{Tool, erased};
use wcode_protocol::Registry;

use crate::config::ToolsConfig;
use crate::tools::default_tools;
use crate::tools::message::Message;
use crate::tools::spawn::Spawn;

/// The fixed configuration a worker inherits from its orchestrator. Owned (not
/// borrowed) so a [`SessionFactory`] can be shared across tool calls.
#[derive(Clone)]
pub struct WorkerTemplate {
    pub system: String,
    pub llm: LlmOpts,
    pub stream_fn: StreamFn,
    pub hooks: HooksSet,
    pub tools: ToolsConfig,
    pub compaction: CompactionPolicy,
    pub working_dir: PathBuf,
}

/// How to build one worker. v1 uses only `name`; the remaining slots are the
/// seam for per-worker customization (§10.1), left at their defaults.
#[derive(Clone, Default)]
pub struct WorkerSpec {
    /// The worker's address. Auto-assigned (`w1`, `w2`, …) when absent.
    pub name: Option<String>,
}

/// A freshly spawned, registered worker.
pub struct SpawnedWorker {
    pub id: SessionId,
}

/// Builds worker sessions and registers each under its orchestrator.
pub struct SessionFactory {
    registry: Registry,
    template: WorkerTemplate,
    seq: AtomicUsize,
}

impl SessionFactory {
    pub fn new(registry: Registry, template: WorkerTemplate) -> Arc<Self> {
        Arc::new(Self {
            registry,
            template,
            seq: AtomicUsize::new(1),
        })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Build a worker, spawn its actor, and register it under `owner` — the
    /// `report_back_to` edge the permitted set reads (§10.1).
    pub fn spawn(&self, owner: &SessionId, spec: WorkerSpec) -> SpawnedWorker {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let name = spec.name.unwrap_or_else(|| format!("w{n}"));
        let id = SessionId::agent(name);
        let handle = SessionActor::spawn(self.build(&id, owner));
        self.registry.register(id.clone(), handle);
        self.registry.set_owner(id.clone(), owner.clone());
        SpawnedWorker { id }
    }

    fn build(&self, id: &SessionId, owner: &SessionId) -> Agent {
        let t = &self.template;
        // The worker is told who it is and who it reports to, so it can address
        // its orchestrator once it has a way to send (the `message` tool, S4-3b).
        let system = format!(
            "{}\n\n# You are a worker\nYou are the session `{id}`, spawned by `{owner}`. \
             Complete the task you are given and report your result back to `{owner}`.",
            t.system
        );
        let mut tools = default_tools(&t.tools);
        tools.push(erased(Message::new(
            self.registry.clone(),
            id.clone(),
            Some(owner.clone()),
        )));
        Agent::new(AgentConfig {
            system,
            tools,
            llm: t.llm.clone(),
            stream_fn: t.stream_fn.clone(),
            hooks: t.hooks.clone(),
            session: None,
            context: Vec::new(),
            working_dir: t.working_dir.clone(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: t.compaction,
        })
    }
}

/// The root session's A2A wiring: the address book, the factory, and the root's
/// own address. Cloneable — shared into the root's tools and, via `repl`, across
/// `/new`.
#[derive(Clone)]
pub struct Orchestrator {
    registry: Registry,
    factory: Arc<SessionFactory>,
    id: SessionId,
}

impl Orchestrator {
    pub fn new(registry: Registry, template: WorkerTemplate) -> Self {
        Self {
            factory: SessionFactory::new(registry.clone(), template),
            registry,
            id: SessionId::agent("orchestrator"),
        }
    }

    /// The root's A2A tools — `spawn` and `message`.
    pub fn tools(&self) -> Vec<Tool> {
        vec![
            erased(Spawn::new(self.factory.clone(), self.id.clone())),
            erased(Message::new(self.registry.clone(), self.id.clone(), None)),
        ]
    }

    /// Register the root's mailbox so a worker can report back to it.
    pub fn register_root(&self, handle: SessionHandle) {
        self.registry.register(self.id.clone(), handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use wcode_harness::event::AgentEvent;
    use wcode_harness::protocol::Request;
    use wcode_harness::streamfn::LlmStream;

    /// A factory whose workers run on a never-streaming model — enough to prove
    /// spawn/registration, since the messages we deliver are `Notify` (no turn).
    fn factory() -> (Arc<SessionFactory>, Registry) {
        let registry = Registry::new();
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        let template = WorkerTemplate {
            system: "sys".into(),
            llm: LlmOpts::default(),
            stream_fn,
            hooks: HooksSet::default(),
            tools: ToolsConfig::default(),
            compaction: CompactionPolicy::default(),
            working_dir: std::env::temp_dir(),
        };
        (SessionFactory::new(registry.clone(), template), registry)
    }

    #[tokio::test]
    async fn spawn_registers_and_owns_the_worker() {
        let (factory, registry) = factory();
        let orch = SessionId::agent("orch");

        let w1 = factory.spawn(&orch, WorkerSpec::default());
        let w2 = factory.spawn(
            &orch,
            WorkerSpec {
                name: Some("reviewer".into()),
            },
        );

        assert_eq!(w1.id.as_str(), "agent:w1");
        assert_eq!(w2.id.as_str(), "agent:reviewer");

        // Registered and owned: the orchestrator may address each worker...
        assert!(registry.permitted(&orch, &w1.id));
        assert!(registry.resolve(&orch, &w1.id).is_ok());
        // ...each worker may address the orchestrator (report back)...
        assert!(registry.permitted(&w1.id, &orch));
        // ...but the workers may not address each other.
        assert!(!registry.permitted(&w1.id, &w2.id));
    }

    #[tokio::test]
    async fn a_spawned_worker_is_a_live_session() {
        let (factory, registry) = factory();
        let orch = SessionId::agent("orch");
        let worker = factory.spawn(&orch, WorkerSpec::default());
        let mut rx = registry.resolve(&orch, &worker.id).unwrap().subscribe();

        registry
            .deliver(
                &orch,
                &worker.id,
                Request::Notify {
                    content: "hello".into(),
                },
            )
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("an event")
            .expect("open");
        assert!(
            matches!(&event, AgentEvent::MessageReceived { from, content }
                if from == &orch && content == "hello"),
            "{event:?}"
        );
    }

    fn session() -> SessionHandle {
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        SessionActor::spawn(Agent::new(AgentConfig {
            system: "sys".into(),
            tools: Vec::new(),
            llm: LlmOpts::default(),
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
        }))
    }

    /// A worker's `message` tool (with `to` omitted) reaches its orchestrator.
    #[tokio::test]
    async fn a_worker_reports_to_its_orchestrator() {
        let registry = Registry::new();
        let orch = SessionId::agent("orch");
        let root = session();
        registry.register(orch.clone(), root.clone());

        let worker = SessionId::agent("w1");
        registry.set_owner(worker.clone(), orch.clone());

        let tool = erased(Message::new(
            registry.clone(),
            worker.clone(),
            Some(orch.clone()),
        ));
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = wcode_harness::tool::ToolContext {
            call_id: "m1".into(),
            name: "message".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
        };

        let mut root_rx = root.subscribe();
        let out = tool
            .execute(serde_json::json!({ "content": "done" }), ctx)
            .await;
        assert!(!out.is_error, "{out:?}");

        let event = tokio::time::timeout(Duration::from_secs(2), root_rx.recv())
            .await
            .expect("an event")
            .expect("open");
        assert!(
            matches!(&event, AgentEvent::MessageReceived { from, content }
                if from == &worker && content == "done"),
            "{event:?}"
        );
    }
}
