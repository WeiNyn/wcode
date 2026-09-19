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
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc;
use wcode_harness::actor::{SessionActor, SessionHandle};
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::hooks::{Hooks, HooksSet};
use wcode_harness::loop_::DEFAULT_MAX_TURNS;
use wcode_harness::message::{AgentMessage, ContentBlock, StopReason};
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::streamfn::{LlmOpts, StreamFn};
use wcode_harness::tool::{Tool, erased};
use wcode_protocol::{Backend, Registry};
use wcode_tui::SurfaceSpec;

use crate::config::ToolsConfig;
use crate::tools::default_tools;
use crate::tools::message::Message;
use crate::tools::spawn::Spawn;

/// A **name → address** map above the registry (§13.15): the phonebook. The
/// orchestrator's model addresses a peer by name (`to: "reviewer"`), and the
/// human can declare peers in `[peers]`. Cheap to clone (shared).
#[derive(Clone, Default)]
pub struct Phonebook {
    names: Arc<std::sync::Mutex<std::collections::HashMap<String, SessionId>>>,
}

impl Phonebook {
    /// Record `name → address` (an alias, or a peer's short name).
    pub fn insert(&self, name: impl Into<String>, address: SessionId) {
        self.names.lock().unwrap().insert(name.into(), address);
    }

    /// The address a name resolves to, if known.
    pub fn get(&self, name: &str) -> Option<SessionId> {
        self.names.lock().unwrap().get(name).cloned()
    }

    /// The known names, sorted — for the `peers` tool.
    pub fn entries(&self) -> Vec<(String, SessionId)> {
        let mut entries: Vec<_> = self
            .names
            .lock()
            .unwrap()
            .iter()
            .map(|(name, address)| (name.clone(), address.clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

/// The short name of an address (`agent:w1` → `w1`); an unprefixed address is
/// its own name.
pub fn short_name(address: &SessionId) -> String {
    address
        .as_str()
        .strip_prefix("agent:")
        .unwrap_or(address.as_str())
        .to_string()
}

/// The fixed configuration a worker inherits from its orchestrator. Owned (not
/// borrowed) so a [`SessionFactory`] can be shared across tool calls.
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

/// How to build one worker. `Default` reproduces v1 behavior: inherit the
/// orchestrator's model and tool set, with the identity blurb only. The optional
/// slots are the per-worker customization seam (§10.1): the model, and — now
/// that the per-agent provider has landed — the `base_url`/`api_key`, so a worker
/// may run on its own provider. `stream_fn` stays shared: the rig keystore rekeys
/// per `ClientKey(base_url, api_key, session_id)`, so one closure serves them all.
#[derive(Clone, Default)]
pub struct WorkerSpec {
    /// The worker's address. Auto-assigned (`w1`, `w2`, …) when absent.
    pub name: Option<String>,
    /// Override the model id; `None` inherits the orchestrator's model.
    pub model: Option<String>,
    /// Extra role text appended to the identity blurb as a `# Role` section.
    pub system: Option<String>,
    /// Restrict the worker to these tool names. `None` = the full default set;
    /// `Some(vec![])` = only `message`. `message` is always kept (D3).
    pub tools: Option<Vec<String>>,
    /// Override the provider base URL; `None` inherits the orchestrator's.
    pub base_url: Option<String>,
    /// Override the provider API key; `None` inherits the orchestrator's.
    pub api_key: Option<String>,
}

/// A freshly spawned, registered worker.
#[derive(Debug)]
pub struct SpawnedWorker {
    pub id: SessionId,
}

/// Builds worker sessions and registers each under its orchestrator.
pub struct SessionFactory {
    registry: Registry,
    template: WorkerTemplate,
    seq: AtomicUsize,
    /// The runtime feed a live TUI adds surfaces from (set by the composition
    /// root after the `[team]` startup loop). `None` until installed.
    sink: Mutex<Option<mpsc::UnboundedSender<SurfaceSpec>>>,
}

impl SessionFactory {
    pub fn new(registry: Registry, template: WorkerTemplate) -> Arc<Self> {
        Arc::new(Self {
            registry,
            template,
            seq: AtomicUsize::new(1),
            sink: Mutex::new(None),
        })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Install a sink that receives a [`SurfaceSpec`] for each worker spawned
    /// after this call — the runtime feed a live TUI adds surfaces from (D27
    /// removed the old `TeamUpdate` feed; this replaces it).
    pub fn set_spawn_sink(&self, tx: mpsc::UnboundedSender<SurfaceSpec>) {
        *self.sink.lock().unwrap() = Some(tx);
    }

    /// The factory's default tool names — the pickable part of a worker's tool
    /// set. `spawn` is absent by construction (a worker cannot spawn); `message`
    /// is also always kept. Use [`Self::validate_tools`] as the allow-list gate.
    pub fn default_tool_names(&self) -> Vec<String> {
        default_tools(&self.template.tools)
            .iter()
            .map(|tool| tool.name().to_string())
            .collect()
    }

    /// Check a spec's tool allow-list against the tools a worker may be given:
    /// the default set plus the always-kept `message`. Returns `Err` naming the
    /// first bad entry — an unknown tool, or `spawn` (a worker never gets it).
    /// `spawn` and F3's `[team]` startup loop both call this so a bad name fails
    /// loudly (D14) instead of being silently dropped.
    pub fn validate_tools(&self, spec: &WorkerSpec) -> Result<(), String> {
        let Some(allow) = &spec.tools else {
            return Ok(());
        };
        let mut valid = self.default_tool_names();
        valid.push("message".to_string());
        for name in allow {
            if name == "spawn" {
                return Err(format!(
                    "`spawn` is not available to a worker (a worker never has `spawn`); \
                     valid tools: {}",
                    valid.join(", ")
                ));
            }
            if !valid.contains(name) {
                return Err(format!(
                    "unknown tool `{name}`; valid tools: {}",
                    valid.join(", ")
                ));
            }
        }
        Ok(())
    }

    /// Build a worker, spawn its actor, and register it under `owner` — the
    /// `report_back_to` edge the permitted set reads (§10.1).
    pub fn spawn(&self, owner: &SessionId, spec: WorkerSpec) -> Result<SpawnedWorker, String> {
        let name = match &spec.name {
            // An explicit name is the caller's claim: a collision is a mistake to
            // report loudly (D18 rejects the same for `[team]` at config load),
            // not to paper over. Every spawn still consumes a `seq` number.
            Some(n) => {
                if self.registry.contains(&SessionId::agent(n)) {
                    return Err(format!("a worker named `{n}` already exists"));
                }
                let _ = self.seq.fetch_add(1, Ordering::Relaxed);
                n.clone()
            }
            // A generated name must never collide — even with an explicit `w1`
            // already registered — so bump `seq` past any that are taken.
            None => loop {
                let n = self.seq.fetch_add(1, Ordering::Relaxed);
                let candidate = format!("w{n}");
                if !self.registry.contains(&SessionId::agent(&candidate)) {
                    break candidate;
                }
            },
        };
        let id = SessionId::agent(name);
        let handle = self.build(&id, owner, &spec);
        let backend = Backend::from(handle.clone());
        self.registry.register(id.clone(), handle);
        self.registry.set_owner(id.clone(), owner.clone());
        // Record the worker's effective model, so a served roster names it (S2):
        // the spec's override, else the inherited template model.
        let model = spec
            .model
            .clone()
            .unwrap_or_else(|| self.template.llm.model.clone());
        self.registry.set_model(id.clone(), model.clone());
        // Announce the new surface so a running TUI can add it — D27 removed the
        // old `TeamUpdate` feed; this is its replacement. A closed receiver is
        // ignored (the TUI may have exited).
        if let Some(tx) = &*self.sink.lock().unwrap() {
            let _ = tx.send(SurfaceSpec {
                id: id.clone(),
                label: short_name(&id),
                model,
                is_root: false,
                backend,
            });
        }
        Ok(SpawnedWorker { id })
    }

    fn build(&self, id: &SessionId, owner: &SessionId, spec: &WorkerSpec) -> SessionHandle {
        SessionActor::spawn(Agent::new(self.worker_config(id, owner, spec)))
    }

    /// The pure worker config: no actor, no registration — the piece worth
    /// testing. Applies a spec's model / provider / role / tool-subset over the
    /// inherited template, so a worker may run on its own model and provider.
    fn worker_config(&self, id: &SessionId, owner: &SessionId, spec: &WorkerSpec) -> AgentConfig {
        let t = &self.template;
        // A worker is told who it is; its result is forwarded to the orchestrator
        // automatically when its run ends (the `ReportBack` hook), so it is not
        // asked to report by hand.
        let mut system = format!(
            "{}\n\n# You are a worker\nYou are the session `{id}`, spawned by \
             `{owner}`. Complete the task you are given. When you finish, your \
             final message is reported back to `{owner}` automatically — you do \
             not need to send it yourself.",
            t.system
        );
        if let Some(role) = &spec.system {
            system.push_str("\n\n# Role\n");
            system.push_str(role);
        }
        // `None` = the whole default set; `Some(list)` = only the named tools.
        // `message` is always kept — the worker's only way to report (D3).
        let mut tools = default_tools(&t.tools);
        if let Some(allow) = &spec.tools {
            tools.retain(|tool| allow.iter().any(|name| name == tool.name()));
        }
        tools.push(erased(Message::new(
            self.registry.clone(),
            id.clone(),
            Some(owner.clone()),
            // A worker sends only to its owner; it needs no phonebook.
            Phonebook::default(),
        )));
        let mut llm = t.llm.clone();
        if let Some(model) = &spec.model {
            llm.model = model.clone();
        }
        if let Some(base_url) = &spec.base_url {
            llm.base_url = Some(base_url.clone());
        }
        if let Some(api_key) = &spec.api_key {
            llm.api_key = Some(api_key.clone());
        }
        // The worker is its own conversation, so give it its own routing id
        // rather than inheriting the root's `x-opencode-session` (D15).
        llm.session_id = Some(id.as_str().to_string());
        AgentConfig {
            system,
            tools,
            llm,
            stream_fn: t.stream_fn.clone(),
            hooks: {
                let mut hooks = t.hooks.clone();
                hooks.push(Arc::new(ReportBack {
                    registry: self.registry.clone(),
                    me: id.clone(),
                    owner: owner.clone(),
                }));
                hooks
            },
            session: None,
            context: Vec::new(),
            working_dir: t.working_dir.clone(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: t.compaction,
        }
    }
}

/// Forwards a worker's result to its orchestrator when its run ends, so the
/// report does not depend on the model remembering to send it (§10.1). Added to
/// a worker's hooks by [`SessionFactory::build`]; the root has no owner, so it
/// has none.
struct ReportBack {
    registry: Registry,
    me: SessionId,
    owner: SessionId,
}

#[async_trait::async_trait]
impl Hooks for ReportBack {
    async fn after_run(&self, ctx: &[AgentMessage], _stop: StopReason) {
        if let Some(text) = last_assistant_text(ctx) {
            // A `Wake`, so the report runs the orchestrator's turn even if idle.
            let _ = self.registry.deliver(
                &self.me,
                &self.owner,
                Request::Wake { content: text },
            );
        }
    }
}

/// The text of the last assistant message, if the run ended with one (a run
/// that stopped on a bare tool call has none).
fn last_assistant_text(ctx: &[AgentMessage]) -> Option<String> {
    let message = ctx
        .iter()
        .rev()
        .find(|m| matches!(m, AgentMessage::Assistant { .. }))?;
    let AgentMessage::Assistant { content, .. } = message else {
        return None;
    };
    let text = content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

/// The root session's A2A wiring: the address book, the factory, and the root's
/// own address. Cloneable — shared into the root's tools and, via `repl`, across
/// `/new`.
#[derive(Clone)]
pub struct Orchestrator {
    registry: Registry,
    factory: Arc<SessionFactory>,
    id: SessionId,
    phonebook: Phonebook,
}

impl Orchestrator {
    pub fn new(registry: Registry, template: WorkerTemplate) -> Self {
        Self {
            factory: SessionFactory::new(registry.clone(), template),
            registry,
            id: SessionId::agent("orchestrator"),
            phonebook: Phonebook::default(),
        }
    }

    /// The root's A2A tools — `spawn`, `message`, and `peers`.
    pub fn tools(&self) -> Vec<Tool> {
        vec![
            erased(Spawn::new(
                self.factory.clone(),
                self.id.clone(),
                self.phonebook.clone(),
            )),
            erased(Message::new(
                self.registry.clone(),
                self.id.clone(),
                None,
                self.phonebook.clone(),
            )),
            erased(crate::tools::peers::Peers::new(self.phonebook.clone())),
        ]
    }

    /// Forward a spawn sink to the factory, so a live TUI can add runtime-spawned
    /// workers as surfaces (the `[team]` startup members are added by the
    /// composition root directly; only runtime spawns come through this feed).
    pub fn set_spawn_sink(&self, tx: mpsc::UnboundedSender<SurfaceSpec>) {
        self.factory.set_spawn_sink(tx);
    }

    /// Spawn a worker owned by this orchestrator and register its name in the
    /// phonebook (F3). Validates the tool allow-list and rejects a duplicate
    /// name first, so the `[team]` startup path (which also calls this) and the
    /// `spawn` tool both fail loudly (D14) rather than silently overwriting.
    pub fn spawn_worker(&self, spec: WorkerSpec) -> Result<SpawnedWorker, String> {
        self.factory.validate_tools(&spec)?;
        let worker = self.factory.spawn(&self.id, spec)?;
        self.phonebook
            .insert(short_name(&worker.id), worker.id.clone());
        Ok(worker)
    }

    /// A worker's backend, by phonebook name — so the composition root can build
    /// a surface for it. `None` when the name is unknown or the worker cannot be
    /// resolved from this orchestrator (the registry is otherwise private).
    pub fn worker_backend(&self, name: &str) -> Option<wcode_protocol::Backend> {
        let id = self.phonebook.get(name)?;
        self.registry.resolve(&self.id, &id).ok()
    }

    /// This orchestrator's own address (`agent:orchestrator`) — the root
    /// surface's id.
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// The shared address book — so the composition root can enumerate the
    /// **local** sessions to serve ([`Registry::locals`]).
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Record a name → address alias in the phonebook (§13.15) — a `[peers]`
    /// entry whose target is an address rather than a socket.
    pub fn alias(&self, name: impl Into<String>, address: SessionId) {
        self.phonebook.insert(name, address);
    }

    /// Configure this session as a **served worker** of `owner` (S4-4 reply):
    /// returns its tool set (a `message` bound to the owner) and the
    /// `ReportBack` hook, and records the ownership edge so the report is
    /// permitted. `me` is the worker's own address (`agent:<name>`).
    pub fn as_worker(&self, me: SessionId, owner: SessionId) -> (Vec<Tool>, Arc<dyn Hooks>) {
        self.registry.set_owner(me.clone(), owner.clone());
        let tools = vec![erased(Message::new(
            self.registry.clone(),
            me.clone(),
            Some(owner.clone()),
            self.phonebook.clone(),
        ))];
        let hook = Arc::new(ReportBack {
            registry: self.registry.clone(),
            me,
            owner,
        }) as Arc<dyn Hooks>;
        (tools, hook)
    }

    /// Register a remote peer — a session served over a socket — so A2A
    /// messages reach it across the process boundary (§8, S4-4).
    #[cfg(unix)]
    pub fn register_remote(&self, id: SessionId, client: wcode_protocol::Client) {
        self.registry.register_remote(id.clone(), client);
        // A peer we register is one we own — the permitted set admits the edge
        // in both directions (§10.1).
        self.registry.set_owner(id.clone(), self.id.clone());
        self.phonebook.insert(short_name(&id), id);
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
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        factory_with(stream_fn)
    }

    fn factory_with(stream_fn: StreamFn) -> (Arc<SessionFactory>, Registry) {
        factory_full(stream_fn, LlmOpts::default())
    }

    fn factory_full(stream_fn: StreamFn, llm: LlmOpts) -> (Arc<SessionFactory>, Registry) {
        let registry = Registry::new();
        let template = WorkerTemplate {
            system: "sys".into(),
            llm,
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

        let w1 = factory.spawn(&orch, WorkerSpec::default()).unwrap();
        let w2 = factory.spawn(
            &orch,
            WorkerSpec {
                name: Some("reviewer".into()),
                ..Default::default()
            },
        ).unwrap();

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
    async fn a_duplicate_explicit_name_is_rejected() {
        let (factory, _registry) = factory();
        let orch = SessionId::agent("orch");
        factory.spawn(&orch, WorkerSpec::default()).unwrap();

        let err = factory
            .spawn(
                &orch,
                WorkerSpec {
                    name: Some("w1".into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.contains("w1"), "names the clashing worker: {err}");
    }

    #[tokio::test]
    async fn a_default_name_skips_an_explicitly_taken_id() {
        let (factory, _registry) = factory();
        let orch = SessionId::agent("orch");

        let explicit = factory
            .spawn(
                &orch,
                WorkerSpec {
                    name: Some("w2".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(explicit.id.as_str(), "agent:w2");

        // `w2` is taken and `seq` has already passed it, so a generated name
        // only avoids it by bumping further — this exercises the auto-bump loop.
        let generated = factory.spawn(&orch, WorkerSpec::default()).unwrap();
        assert_ne!(generated.id.as_str(), "agent:w2");
    }

    #[tokio::test]
    async fn two_default_spawns_get_distinct_ids() {
        let (factory, _registry) = factory();
        let orch = SessionId::agent("orch");
        let a = factory.spawn(&orch, WorkerSpec::default()).unwrap();
        let b = factory.spawn(&orch, WorkerSpec::default()).unwrap();
        assert_ne!(a.id, b.id);
    }

    #[tokio::test]
    async fn spawn_emits_a_surface_spec_on_success_only() {
        let (factory, _registry) = factory();
        let (tx, mut rx) = mpsc::unbounded_channel::<SurfaceSpec>();
        factory.set_spawn_sink(tx);
        let orch = SessionId::agent("orch");

        // A successful spawn announces exactly one surface for that worker.
        factory
            .spawn(
                &orch,
                WorkerSpec {
                    name: Some("explorer".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let spec = rx.try_recv().expect("one spec");
        assert_eq!(spec.id.as_str(), "agent:explorer");
        assert_eq!(spec.label, "explorer");
        assert!(!spec.is_root);

        // A duplicate name fails (FIX 1) and must NOT announce a surface.
        let err = factory
            .spawn(
                &orch,
                WorkerSpec {
                    name: Some("explorer".into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.contains("explorer"), "{err}");
        assert!(rx.try_recv().is_err(), "no spec on a failed spawn");
    }

    #[tokio::test]
    async fn a_spawned_worker_is_a_live_session() {
        let (factory, registry) = factory();
        let orch = SessionId::agent("orch");
        let worker = factory.spawn(&orch, WorkerSpec::default()).unwrap();
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
            Phonebook::default(),
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

    /// The `ReportBack` hook forwards a worker's final text to its orchestrator
    /// when the worker's run ends — no model cooperation required.
    #[tokio::test]
    async fn a_worker_auto_reports_when_its_run_ends() {
        use wcode_harness::event::LlmStreamEvent;

        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::iter(vec![
                LlmStreamEvent::TextDelta("the answer is 56".into()),
                LlmStreamEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: None,
                },
            ])) as LlmStream
        });
        let (factory, registry) = factory_with(stream_fn);
        let orch = SessionId::agent("orch");
        let root = session();
        registry.register(orch.clone(), root.clone());

        let worker = factory.spawn(&orch, WorkerSpec::default()).unwrap();
        let mut root_rx = root.subscribe();

        // Hand the worker a task — a `Wake` starts its run.
        registry
            .deliver(
                &orch,
                &worker.id,
                Request::Wake {
                    content: "do it".into(),
                },
            )
            .unwrap();

        // The run ends → the worker reports to the orchestrator on its own.
        let event = tokio::time::timeout(Duration::from_secs(3), root_rx.recv())
            .await
            .expect("an event")
            .expect("open");
        assert!(
            matches!(&event, AgentEvent::MessageReceived { from, content }
                if from == &worker.id && content == "the answer is 56"),
            "{event:?}"
        );
    }

    #[test]
    fn phonebook_resolves_names_and_lists_them_sorted() {
        let book = Phonebook::default();
        book.insert("reviewer", SessionId::agent("w1"));
        book.insert("helper", SessionId::new("agent:uuid-7"));

        assert_eq!(book.get("reviewer"), Some(SessionId::agent("w1")));
        assert_eq!(book.get("nope"), None);

        let names: Vec<String> = book.entries().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["helper", "reviewer"]);

        assert_eq!(short_name(&SessionId::agent("w1")), "w1");
        assert_eq!(short_name(&SessionId::user()), "user");
    }

    #[tokio::test]
    async fn spawn_registers_the_worker_in_the_phonebook() {
        let (factory, _registry) = factory();
        let book = Phonebook::default();
        let tool = erased(Spawn::new(
            factory,
            SessionId::agent("orch"),
            book.clone(),
        ));

        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = wcode_harness::tool::ToolContext {
            call_id: "s1".into(),
            name: "spawn".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
        };
        let out = tool
            .execute(serde_json::json!({ "task": "hi", "name": "reviewer" }), ctx)
            .await;
        assert!(!out.is_error, "{out:?}");

        assert_eq!(book.get("reviewer"), Some(SessionId::agent("reviewer")));
    }

    /// S4-4 reply: `as_worker` records the ownership edge and returns a hook
    /// that forwards the worker's final text to its owner.
    #[tokio::test]
    async fn a_served_worker_reports_to_its_owner() {
        let registry = Registry::new();
        let orch = SessionId::agent("orchestrator");
        let root = session();
        registry.register(orch.clone(), root.clone());

        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
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
        let o = Orchestrator::new(registry.clone(), template);

        let me = SessionId::agent("w1");
        let (tools, hook) = o.as_worker(me.clone(), orch.clone());
        assert_eq!(tools.len(), 1, "the worker gets a `message` tool");
        assert!(registry.permitted(&me, &orch), "the ownership edge is recorded");

        let mut root_rx = root.subscribe();
        let ctx = vec![AgentMessage::Assistant {
            content: vec![ContentBlock::Text {
                text: "done".into(),
            }],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        }];
        hook.after_run(&ctx, StopReason::Stop).await;

        let event = tokio::time::timeout(Duration::from_secs(2), root_rx.recv())
            .await
            .expect("an event")
            .expect("open");
        assert!(
            matches!(&event, AgentEvent::MessageReceived { from, content }
                if from == &me && content == "done"),
            "{event:?}"
        );
    }

    /// The pure worker config for `spec`, built on a default factory.
    fn config_for(spec: &WorkerSpec) -> AgentConfig {
        let (factory, _registry) = factory();
        factory.worker_config(&SessionId::agent("w1"), &SessionId::agent("orch"), spec)
    }

    /// An orchestrator over a default (never-streaming) worker template.
    fn orchestrator() -> Orchestrator {
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
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
        Orchestrator::new(Registry::new(), template)
    }

    fn tool_names(cfg: &AgentConfig) -> Vec<String> {
        cfg.tools.iter().map(|t| t.name().to_string()).collect()
    }

    #[test]
    fn a_model_override_changes_only_the_model() {
        let llm = LlmOpts {
            base_url: Some("http://example/v1".into()),
            api_key: Some("k".into()),
            ..Default::default()
        };
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        let (factory, _registry) = factory_full(stream_fn, llm);
        let id = SessionId::agent("w1");
        let owner = SessionId::agent("orch");

        let base = factory.worker_config(&id, &owner, &WorkerSpec::default());
        let cfg = factory.worker_config(
            &id,
            &owner,
            &WorkerSpec {
                model: Some("x".into()),
                ..Default::default()
            },
        );
        assert_eq!(cfg.llm.model, "x");
        // Only the model changes; the shared routing opts are untouched (D2).
        assert_eq!(cfg.llm.base_url.as_deref(), Some("http://example/v1"));
        assert_eq!(cfg.llm.base_url, base.llm.base_url);
        assert_eq!(cfg.llm.api_key.as_deref(), Some("k"));
        assert_eq!(cfg.llm.api_key, base.llm.api_key);
    }

    #[test]
    fn a_provider_override_changes_only_the_provider() {
        let llm = LlmOpts {
            base_url: Some("http://shared/v1".into()),
            api_key: Some("shared-key".into()),
            ..Default::default()
        };
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        let (factory, _registry) = factory_full(stream_fn, llm);
        let id = SessionId::agent("w1");
        let owner = SessionId::agent("orch");

        let base = factory.worker_config(&id, &owner, &WorkerSpec::default());
        let cfg = factory.worker_config(
            &id,
            &owner,
            &WorkerSpec {
                base_url: Some("http://worker/v1".into()),
                api_key: Some("worker-key".into()),
                ..Default::default()
            },
        );
        assert_eq!(cfg.llm.base_url.as_deref(), Some("http://worker/v1"));
        assert_eq!(cfg.llm.api_key.as_deref(), Some("worker-key"));
        // Only the provider changes; the model and the shared routing id are
        // inherited (the worker still gets its own `session_id`, D15).
        assert_eq!(cfg.llm.model, base.llm.model);
        assert_eq!(cfg.llm.session_id, base.llm.session_id);
        assert_eq!(base.llm.base_url.as_deref(), Some("http://shared/v1"));
        assert_eq!(base.llm.api_key.as_deref(), Some("shared-key"));
    }

    #[test]
    fn a_role_is_appended_after_the_identity_blurb() {
        let cfg = config_for(&WorkerSpec {
            system: Some("role text".into()),
            ..Default::default()
        });
        // The who-am-I / auto-report blurb survives verbatim, above the role.
        assert!(
            cfg.system.starts_with("sys\n\n# You are a worker"),
            "{}",
            cfg.system
        );
        assert!(cfg.system.contains("# Role\nrole text"), "{}", cfg.system);
    }

    #[test]
    fn a_tool_allow_list_keeps_only_those_tools_plus_message() {
        let cfg = config_for(&WorkerSpec {
            tools: Some(vec!["read".into(), "bash".into()]),
            ..Default::default()
        });
        assert_eq!(tool_names(&cfg), vec!["read", "bash", "message"]);
        assert!(!tool_names(&cfg).contains(&"spawn".to_string()));
    }

    #[test]
    fn an_empty_tool_list_keeps_only_message() {
        let cfg = config_for(&WorkerSpec {
            tools: Some(vec![]),
            ..Default::default()
        });
        assert_eq!(tool_names(&cfg), vec!["message"]);
    }

    #[test]
    fn message_in_the_allow_list_is_accepted_and_not_duplicated() {
        let (factory, _registry) = factory();
        let spec = WorkerSpec {
            tools: Some(vec!["read".into(), "message".into()]),
            ..Default::default()
        };
        assert!(
            factory.validate_tools(&spec).is_ok(),
            "`message` is always valid"
        );
        let cfg = factory.worker_config(&SessionId::agent("w1"), &SessionId::agent("orch"), &spec);
        assert_eq!(
            tool_names(&cfg),
            vec!["read", "message"],
            "no duplicate `message`"
        );
    }

    #[test]
    fn validate_tools_rejects_spawn_and_unknown_names() {
        let (factory, _registry) = factory();

        // `spawn` is never available to a worker → a distinct error.
        let err = factory
            .validate_tools(&WorkerSpec {
                tools: Some(vec!["spawn".into()]),
                ..Default::default()
            })
            .unwrap_err();
        // The DISTINCT branch, not the generic `unknown tool` one.
        assert!(err.contains("not available"), "distinct wording: {err}");
        assert!(!err.contains("unknown tool"), "not the generic error: {err}");

        // A genuinely unknown name → an error naming it.
        let err = factory
            .validate_tools(&WorkerSpec {
                tools: Some(vec!["bogus".into()]),
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.contains("bogus"), "{err}");

        // An absent allow-list is always fine.
        assert!(factory.validate_tools(&WorkerSpec::default()).is_ok());
    }

    #[test]
    fn a_default_spec_is_the_full_inherited_worker() {
        let (factory, _registry) = factory();
        let id = SessionId::agent("w1");
        let owner = SessionId::agent("orch");
        let cfg = factory.worker_config(&id, &owner, &WorkerSpec::default());

        let mut expected: Vec<String> = default_tools(&factory.template.tools)
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        expected.push("message".into());
        assert_eq!(tool_names(&cfg), expected, "the whole default set + message");
        assert!(!tool_names(&cfg).contains(&"spawn".to_string()));
        // Inherited model, and the identity blurb only (no role section).
        assert_eq!(cfg.llm.model, factory.template.llm.model);
        assert!(cfg.system.contains("# You are a worker"));
        assert!(!cfg.system.contains("# Role"));
    }

    #[test]
    fn a_worker_gets_its_own_session_id_not_the_roots() {
        let llm = LlmOpts {
            session_id: Some("root-session".into()),
            ..Default::default()
        };
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        let (factory, _registry) = factory_full(stream_fn, llm);
        let cfg = factory.worker_config(
            &SessionId::agent("w7"),
            &SessionId::agent("orch"),
            &WorkerSpec::default(),
        );
        assert_eq!(cfg.llm.session_id.as_deref(), Some("agent:w7"));
        assert_ne!(cfg.llm.session_id.as_deref(), Some("root-session"));
    }

    #[tokio::test]
    async fn spawn_worker_registers_the_name_and_rejects_bad_tools() {
        let o = orchestrator();
        let worker = o
            .spawn_worker(WorkerSpec {
                name: Some("explorer".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(worker.id.as_str(), "agent:explorer");
        assert_eq!(o.phonebook.get("explorer"), Some(worker.id.clone()));
        assert!(
            o.registry.permitted(&o.id, &worker.id),
            "owned by the orchestrator"
        );

        // A bad allow-list name fails loudly — the preset path validates (D14).
        let err = o
            .spawn_worker(WorkerSpec {
                name: Some("dup".into()),
                tools: Some(vec!["bogus".into()]),
                ..Default::default()
            })
            .map(|_| ())
            .unwrap_err();
        assert!(err.contains("bogus"), "{err}");
    }
}
