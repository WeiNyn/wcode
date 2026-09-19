//! The `spawn` tool: create a worker session and hand it a task (§10.1).

use std::sync::Arc;

use serde::Deserialize;
#[cfg(unix)]
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};
#[cfg(unix)]
use wcode_protocol::Backend;

use crate::agents::{Phonebook, SessionFactory, WorkerSpec, short_name};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SpawnArgs {
    /// The task the worker should carry out.
    task: String,
    /// Optional remote target: a peer name (`--peer`/`[peers]`) or `agent:<id>`.
    /// When it resolves to a served peer, the worker is defined **there** (a
    /// `Request::Define` over the socket) instead of in this process.
    #[serde(default)]
    to: Option<String>,
    /// Optional address for the worker (default: `w1`, `w2`, …).
    #[serde(default)]
    name: Option<String>,
    /// Optional model id override for the worker (default: the orchestrator's).
    #[serde(default)]
    model: Option<String>,
    /// Optional role text appended to the worker's system prompt as a `# Role`.
    #[serde(default)]
    role: Option<String>,
    /// Optional tool allow-list for the worker (`message` is always kept; an
    /// unknown name is rejected). Omit for the full default set.
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// Optional provider base URL for the worker (default: the orchestrator's).
    #[serde(default)]
    base_url: Option<String>,
    /// Optional provider API key for the worker (default: the orchestrator's).
    #[serde(default)]
    api_key: Option<String>,
}

impl SpawnArgs {
    /// Map the tool args onto a worker spec: `role` → `system`; `name`, `model`,
    /// `tools`, and the provider (`base_url`/`api_key`) pass through unchanged.
    fn to_worker_spec(&self) -> WorkerSpec {
        WorkerSpec {
            name: self.name.clone(),
            model: self.model.clone(),
            system: self.role.clone(),
            tools: self.tools.clone(),
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
        }
    }
}

/// Spawns a worker and starts it on a task.
///
/// Only the orchestrator holds one: a worker's tool set has no `spawn`, so only
/// the root spawns (bounded fan-out by construction, §10.1).
pub struct Spawn {
    factory: Arc<SessionFactory>,
    me: SessionId,
    phonebook: Phonebook,
}

impl Spawn {
    pub fn new(factory: Arc<SessionFactory>, me: SessionId, phonebook: Phonebook) -> Self {
        Self {
            factory,
            me,
            phonebook,
        }
    }

    /// The in-process path: validate, spawn, register, and hand over the task.
    fn spawn_local(&self, args: SpawnArgs) -> ToolOutput {
        let spec = args.to_worker_spec();
        // Fail loudly on a bad allow-list entry rather than silently dropping it
        // (D14) — the model must learn the name was wrong.
        if let Err(e) = self.factory.validate_tools(&spec) {
            return ToolOutput {
                output: e,
                is_error: true,
                ..ToolOutput::default()
            };
        }
        let worker = match self.factory.spawn(&self.me, spec) {
            Ok(worker) => worker,
            Err(e) => {
                return ToolOutput {
                    output: e,
                    is_error: true,
                    ..ToolOutput::default()
                };
            }
        };
        // The worker joins the phonebook, so the model can address it by name.
        self.phonebook
            .insert(short_name(&worker.id), worker.id.clone());
        // Hand the task over at once — as a `Wake`, so the worker runs even if
        // idle.
        match self.factory.registry().deliver(
            &self.me,
            &worker.id,
            Request::Wake {
                content: args.task,
            },
        ) {
            Ok(()) => ToolOutput {
                output: format!("spawned {}", worker.id),
                ..ToolOutput::default()
            },
            Err(e) => ToolOutput {
                output: format!("spawned {} but could not deliver its task: {e}", worker.id),
                is_error: true,
                ..ToolOutput::default()
            },
        }
    }

    /// Define a worker on a served peer: resolve `to` (a phonebook name or
    /// `agent:<id>`) to a backend and, when it is remote, send `Request::Define`
    /// and return the peer's `Spawned` address. A local or unroutable target is
    /// an error; the in-process path is unchanged.
    #[cfg(unix)]
    async fn spawn_remote(&self, to: &str, args: SpawnArgs) -> ToolOutput {
        let target = self
            .phonebook
            .get(to)
            .unwrap_or_else(|| SessionId::new(to));
        let backend = match self.factory.registry().resolve(&self.me, &target) {
            Ok(backend) => backend,
            Err(e) => {
                return ToolOutput {
                    output: format!("cannot reach `{to}`: {e}"),
                    is_error: true,
                    ..ToolOutput::default()
                };
            }
        };
        // The peer's client, narrowed to the worker's own session once it is
        // known: the server routes inbound frames on `frame.session`.
        let Backend::Remote(client) = backend else {
            return ToolOutput {
                output: format!("`{to}` is not a served peer; `to` needs a socket peer"),
                is_error: true,
                ..ToolOutput::default()
            };
        };
        match client
            .ask(Request::Define {
                name: args.name,
                model: args.model,
                role: args.role,
                tools: args.tools,
                base_url: args.base_url,
                api_key: args.api_key,
            })
            .await
        {
            Ok(AgentEvent::Spawned { worker }) => {
                // Register the worker as a remote peer — narrowed to its own
                // session — so the caller can reach it, and record the ownership
                // edge (the caller owns it). Mirrors `Orchestrator::register_remote`
                // and the local path's post-spawn semantics.
                let registry = self.factory.registry();
                registry.register_remote(worker.clone(), client.with_session(worker.clone()));
                registry.set_owner(worker.clone(), self.me.clone());
                self.phonebook.insert(short_name(&worker), worker.clone());
                // Hand the task over at once — a `Wake`, so the worker runs even
                // if idle, exactly as the in-process path does.
                match registry.deliver(&self.me, &worker, Request::Wake { content: args.task }) {
                    Ok(()) => ToolOutput {
                        output: format!("spawned {worker} on {to}"),
                        ..ToolOutput::default()
                    },
                    Err(e) => ToolOutput {
                        output: format!(
                            "spawned {worker} on {to} but could not deliver its task: {e}"
                        ),
                        is_error: true,
                        ..ToolOutput::default()
                    },
                }
            }
            Ok(AgentEvent::Error { message }) => ToolOutput {
                output: message,
                is_error: true,
                ..ToolOutput::default()
            },
            Ok(other) => ToolOutput {
                output: format!("unexpected reply from `{to}`: {other:?}"),
                is_error: true,
                ..ToolOutput::default()
            },
            Err(_) => ToolOutput {
                output: format!("peer `{to}` is closed"),
                is_error: true,
                ..ToolOutput::default()
            },
        }
    }
}

#[async_trait::async_trait]
impl TypedTool for Spawn {
    type Args = SpawnArgs;

    fn name(&self) -> &str {
        "spawn"
    }

    fn description(&self) -> &str {
        "Spawn a worker agent for a task. The worker runs in its own session, \
         works on the task, and reports its result back to you. Returns the \
         worker's address (use it with the `message` tool). Optional: `model` \
         overrides the worker's model, `role` appends a role section to its \
         system prompt, `tools` restricts it to the named tools (`message` is \
         always kept; an unknown tool name is rejected), `base_url`/`api_key` \
         put it on its own provider, and `to` defines it on a served peer \
         (a `--peer`/`[peers]` name or `agent:<id>`) instead of in-process."
    }

    async fn execute(&self, args: SpawnArgs, _ctx: &ToolContext) -> ToolOutput {
        // A remote target defines the worker on a served peer instead: send
        // `Request::Define` over the socket and surface its reply.
        #[cfg(unix)]
        if let Some(to) = args.to.clone() {
            return self.spawn_remote(&to, args).await;
        }
        #[cfg(not(unix))]
        if args.to.is_some() {
            return ToolOutput {
                output: "spawning on a remote peer (`to`) is not supported on this platform"
                    .into(),
                is_error: true,
                ..ToolOutput::default()
            };
        }
        self.spawn_local(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use wcode_harness::actor::{SessionActor, SessionHandle};
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
    use wcode_harness::tool::erased;
    use wcode_protocol::Registry;

    use crate::agents::WorkerTemplate;
    use crate::config::ToolsConfig;

    /// A never-streaming session, enough to be a served root.
    fn session() -> SessionHandle {
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
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

    fn factory() -> Arc<SessionFactory> {
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        SessionFactory::new(
            Registry::new(),
            WorkerTemplate {
                system: "sys".into(),
                llm: LlmOpts::default(),
                stream_fn,
                hooks: HooksSet::default(),
                tools: ToolsConfig::default(),
                compaction: CompactionPolicy::default(),
                working_dir: std::env::temp_dir(),
            },
        )
    }

    fn ctx() -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "s1".into(),
            name: "spawn".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_name_fails_loudly() {
        let tool = erased(Spawn::new(
            factory(),
            SessionId::agent("orch"),
            Phonebook::default(),
        ));
        let out = tool
            .execute(
                serde_json::json!({ "task": "hi", "tools": ["read", "bogus"] }),
                ctx(),
            )
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("bogus"), "names the bad entry: {}", out.output);
        assert!(out.output.contains("read"), "lists a valid tool: {}", out.output);
    }

    #[test]
    fn spawn_args_map_onto_a_worker_spec() {
        let args = SpawnArgs {
            task: "do it".into(),
            to: None,
            name: Some("w9".into()),
            model: Some("m".into()),
            role: Some("reviewer".into()),
            tools: Some(vec!["read".into()]),
            base_url: Some("http://w/v1".into()),
            api_key: Some("wk".into()),
        };
        let spec = args.to_worker_spec();
        assert_eq!(spec.name.as_deref(), Some("w9"));
        assert_eq!(spec.model.as_deref(), Some("m"));
        // `role` lands in `system`; the rest pass through.
        assert_eq!(spec.system.as_deref(), Some("reviewer"));
        assert_eq!(spec.tools, Some(vec!["read".to_string()]));
        assert_eq!(spec.base_url.as_deref(), Some("http://w/v1"));
        assert_eq!(spec.api_key.as_deref(), Some("wk"));
    }

    /// R2: `spawn { to }` defines a worker on a served peer, registers it as a
    /// reachable **remote peer**, and delivers the task as a `Wake` — the same
    /// post-spawn semantics the local path has.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_spawn_to_a_remote_peer_defines_and_starts_the_worker() {
        // Peer A: a live roster; its Define handler registers a real worker it
        // can then route a task to.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("A.sock");
        let peer_registry = Registry::new();
        let worker_handle = session();
        let mut worker_events = worker_handle.subscribe();
        let (seen_tx, mut seen_rx) =
            tokio::sync::mpsc::unbounded_channel::<wcode_protocol::DefineArgs>();
        let handler: wcode_protocol::DefineHandler = {
            let peer_registry = peer_registry.clone();
            let worker_handle = worker_handle.clone();
            Arc::new(move |args| {
                let name = args.name.clone().unwrap_or_else(|| "w1".into());
                let id = SessionId::agent(name);
                let _ = seen_tx.send(args);
                peer_registry.register(id.clone(), worker_handle.clone());
                Ok(id)
            })
        };
        let listener = wcode_protocol::bind(&sock).await.unwrap();
        tokio::spawn(wcode_protocol::serve(
            peer_registry.clone(),
            peer_registry.subscribe(),
            (SessionId::new("A"), session()),
            Some(handler),
            listener,
        ));

        // This process registers the peer `r`, then spawns on it with a task.
        let factory = factory();
        let registry = factory.registry().clone();
        let client = wcode_protocol::Client::connect(&sock).await.unwrap();
        registry.register_remote(SessionId::agent("r"), client);
        registry.set_owner(SessionId::agent("r"), SessionId::agent("orch"));

        let root = SessionId::agent("orch");
        let book = Phonebook::default();
        let tool = erased(Spawn::new(factory.clone(), root.clone(), book.clone()));
        let out = tool
            .execute(
                serde_json::json!({ "task": "DO THE THING", "to": "agent:r", "name": "reviewer" }),
                ctx(),
            )
            .await;
        assert!(!out.is_error, "{out:?}");
        assert!(out.output.contains("agent:reviewer"), "{}", out.output);
        // The `Define` crossed the socket with the requested name...
        let args = seen_rx.try_recv().expect("the peer saw a Define");
        assert_eq!(args.name.as_deref(), Some("reviewer"));

        // (a) ...and the worker was woken with the task (B1): its
        // `MessageReceived` carries the exact task text.
        let mut got_task = false;
        while let Ok(Ok(event)) =
            tokio::time::timeout(std::time::Duration::from_secs(3), worker_events.recv()).await
        {
            if let AgentEvent::MessageReceived { content, .. } = event {
                assert_eq!(content, "DO THE THING");
                got_task = true;
                break;
            }
        }
        assert!(got_task, "the worker was woken with the task");

        // (b) The caller can reach the worker it just defined (B2).
        let worker = SessionId::agent("reviewer");
        assert!(
            registry.resolve(&root, &worker).is_ok(),
            "the defined worker is a reachable remote peer"
        );
        assert_eq!(book.get("reviewer"), Some(worker));
    }

    /// A `to` that names no served peer is an error, not a silent in-process
    /// spawn.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_spawn_to_an_unknown_peer_is_an_error() {
        let tool = erased(Spawn::new(
            factory(),
            SessionId::agent("orch"),
            Phonebook::default(),
        ));
        let out = tool
            .execute(serde_json::json!({ "task": "hi", "to": "ghost" }), ctx())
            .await;
        assert!(out.is_error, "{out:?}");
        assert!(out.output.contains("ghost"), "{}", out.output);
    }
}
