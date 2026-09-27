//! The `member` tool: inspect or stop a teammate mid-run (§10.1).
//!
//! The kernel answers `Status`/`SideAsk` from a turn-boundary snapshot while a
//! run is in flight, and services `Cancel` immediately — so all three ops work
//! while the member is thinking or inside a tool call, without waiting for its
//! run to end. `Status` is a lean read (one string), not the whole transcript.

use std::time::{Duration, Instant};

use serde::Deserialize;

use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{MemberState, Request, SessionId};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};
use wcode_protocol::{AskError, Registry};

use crate::agents::Phonebook;
use crate::tools::message::address;

/// Budget for `status` — a lean `Request::Status` read, answered from the
/// member's latest turn snapshot even while it runs. Instant in practice.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
/// Budget for `ask` — the member's own model call.
const ASK_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `cancel` waits for the member's run to actually stop.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);
/// Poll interval while waiting for a cancel to take effect.
const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MemberArgs {
    /// The member's name or address (`w1` or `agent:w1`).
    to: String,
    /// `status` — the member's last completed work, no model call; `ask` — a
    /// tool-free question answered by the member's model from its current
    /// context (needs `text`); `cancel` — stop its in-flight run.
    op: String,
    /// The question for `op = "ask"`.
    #[serde(default)]
    text: Option<String>,
}

/// Inspects or stops a peer without waiting for its run to end.
pub struct Member {
    registry: Registry,
    me: SessionId,
    phonebook: Phonebook,
}

impl Member {
    pub fn new(registry: Registry, me: SessionId, phonebook: Phonebook) -> Self {
        Self {
            registry,
            me,
            phonebook,
        }
    }

    /// Resolve `to`: a bare name through the phonebook, else `agent:<name>`.
    fn target(&self, to: &str) -> SessionId {
        self.phonebook.get(to).unwrap_or_else(|| address(to))
    }

    async fn status(&self, to: &SessionId) -> ToolOutput {
        let state = self.registry.state_of(to).label();
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        match backend.ask_within(Request::Status, STATUS_TIMEOUT).await {
            Ok(AgentEvent::Status { last_assistant_text }) => {
                let work = last_assistant_text
                    .unwrap_or_else(|| "(no completed work yet)".to_string());
                ToolOutput {
                    output: format!("[{state}] {to} · {work}"),
                    ..ToolOutput::default()
                }
            }
            Ok(other) => ToolOutput {
                output: format!("[{state}] {to} · unexpected reply: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!("[{state}] {to} · no reply within {STATUS_TIMEOUT:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    async fn ask(&self, to: &SessionId, text: &str) -> ToolOutput {
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        let request = Request::SideAsk {
            text: text.to_string(),
        };
        match backend.ask_within(request, ASK_TIMEOUT).await {
            Ok(AgentEvent::SideAnswer { text, .. }) => ToolOutput {
                output: text,
                ..ToolOutput::default()
            },
            Ok(other) => ToolOutput {
                output: format!("{to} replied unexpectedly: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!("{to} is busy: no answer within {ASK_TIMEOUT:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    async fn cancel(&self, to: &SessionId) -> ToolOutput {
        if let Err(e) = self.registry.resolve(&self.me, to) {
            return error(format!("cannot reach {to}: {e}"));
        }
        let state = self.registry.state_of(to);
        if state != MemberState::Running {
            return ToolOutput {
                output: format!("{to} is not running ({})", state.label()),
                ..ToolOutput::default()
            };
        }
        if let Err(e) = self.registry.deliver(&self.me, to, Request::Cancel) {
            return error(format!("could not cancel {to}: {e}"));
        }
        // Wait for the run to actually end rather than reporting a blind send.
        let deadline = Instant::now() + CANCEL_TIMEOUT;
        loop {
            let now = self.registry.state_of(to);
            if now != MemberState::Running {
                return ToolOutput {
                    output: format!("cancelled {to} (now {})", now.label()),
                    ..ToolOutput::default()
                };
            }
            if Instant::now() >= deadline {
                return ToolOutput {
                    output: format!("cancel sent to {to}; still running after {CANCEL_TIMEOUT:?}"),
                    ..ToolOutput::default()
                };
            }
            tokio::time::sleep(CANCEL_POLL).await;
        }
    }
}

fn error(message: String) -> ToolOutput {
    ToolOutput {
        output: message,
        is_error: true,
        ..ToolOutput::default()
    }
}

#[async_trait::async_trait]
impl TypedTool for Member {
    type Args = MemberArgs;

    fn name(&self) -> &str {
        "member"
    }

    fn description(&self) -> &str {
        "Inspect or stop a teammate without waiting for its turn to end. \
         `op = \"status\"` returns the member's last completed work (no model \
         call); `op = \"ask\"` puts a tool-free question to the member's own \
         model from its current context (needs `text`); `op = \"cancel\"` stops \
         the member's in-flight run and waits for it to stop."
    }

    async fn execute(&self, args: MemberArgs, _ctx: &ToolContext) -> ToolOutput {
        let to = self.target(&args.to);
        match args.op.as_str() {
            "status" => self.status(&to).await,
            "ask" => match args.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(text) => self.ask(&to, text).await,
                None => error("`ask` needs `text` (the question)".to_string()),
            },
            "cancel" => self.cancel(&to).await,
            other => error(format!(
                "unknown op `{other}` (expected status | ask | cancel)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use wcode_harness::actor::SessionActor;
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::LlmStreamEvent;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::message::{AgentMessage, StopReason};
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    fn ctx() -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "m1".into(),
            name: "member".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
            session_path: None,
        }
    }

    /// Poll until `id` reaches `want` (the registry mirrors each member's run
    /// state through a watcher task, so it updates on its own).
    async fn wait_until(registry: &Registry, id: &SessionId, want: MemberState) {
        for _ in 0..300 {
            if registry.state_of(id) == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{id} never reached {want:?}");
    }

    /// A registered member with `context` seeded and a caller-supplied stream.
    fn member_session(
        registry: &Registry,
        name: &str,
        context: Vec<AgentMessage>,
        stream_fn: StreamFn,
    ) -> SessionId {
        let agent = Agent::new(AgentConfig {
            system: "sys".into(),
            tools: vec![],
            llm: LlmOpts {
                model: "m1".into(),
                ..LlmOpts::default()
            },
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context,
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
            plan_mode: wcode_harness::hooks::PlanModeHandle::new(),
        });
        let handle = SessionActor::spawn(agent);
        let id = SessionId::agent(name);
        registry.register(id.clone(), handle);
        id
    }

    fn text_stream(text: &'static str) -> StreamFn {
        Arc::new(move |_c, _s, _t, _o| {
            Box::pin(futures::stream::iter(vec![
                LlmStreamEvent::TextDelta(text.into()),
                LlmStreamEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: None,
                },
            ])) as LlmStream
        })
    }

    fn hanging_stream() -> StreamFn {
        Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::pending::<LlmStreamEvent>()) as LlmStream
        })
    }

    /// The orchestrator's side: a registry that owns the member + a `member` tool.
    fn orchestrator(registry: &Registry) -> Member {
        Member::new(registry.clone(), SessionId::agent("orchestrator"), Phonebook::default())
    }

    #[tokio::test]
    async fn status_reports_state_and_last_work() {
        let registry = Registry::new();
        let assistant = AgentMessage::Assistant {
            content: vec![wcode_harness::message::ContentBlock::Text {
                text: "running cargo test".into(),
            }],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        };
        let id = member_session(&registry, "w1", vec![assistant], text_stream("unused"));
        // Wire the ownership edge so `resolve` permits it.
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "status".into(),
                    text: None,
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("running cargo test"), "{}", out.output);
    }

    #[tokio::test]
    async fn ask_returns_the_members_side_answer() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("I am debugging X"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "ask".into(),
                    text: Some("what are you doing?".into()),
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(out.output, "I am debugging X");
    }

    #[tokio::test]
    async fn cancel_stops_a_running_member() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), hanging_stream());
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        // Start a run that hangs, and wait until it is observably running.
        registry
            .resolve(&SessionId::agent("orchestrator"), &id)
            .unwrap()
            .send(Request::Submit { text: "work".into() })
            .unwrap();
        wait_until(&registry, &id, MemberState::Running).await;

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "cancel".into(),
                    text: None,
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("cancelled"), "{}", out.output);
        assert_eq!(registry.state_of(&id), MemberState::Done);
    }

    #[tokio::test]
    async fn ask_without_text_is_an_error() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));
        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "ask".into(),
                    text: None,
                },
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("needs `text`"), "{}", out.output);
    }

    #[tokio::test]
    async fn unknown_op_is_an_error() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));
        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "poke".into(),
                    text: None,
                },
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("unknown op"), "{}", out.output);
    }

    #[tokio::test]
    #[ignore = "live test; run with WCODE_BASE_URL=... (and WCODE_MODEL=...)"]
    async fn live_ask_against_a_real_model() {
        let base = std::env::var("WCODE_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".into());
        let model = std::env::var("WCODE_MODEL").unwrap_or_else(|_| "gemma4:e4b".into());

        let registry = Registry::new();
        let agent = Agent::new(AgentConfig {
            system: "You are a teammate. Answer in one short sentence.".into(),
            tools: vec![],
            llm: LlmOpts {
                model,
                base_url: Some(base),
                ..LlmOpts::default()
            },
            stream_fn: wcode_harness::streamfn::rig_stream_fn(),
            hooks: HooksSet::default(),
            session: None,
            context: vec![AgentMessage::user_text("I am refactoring the parser.")],
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
            plan_mode: wcode_harness::hooks::PlanModeHandle::new(),
        });
        let id = SessionId::agent("w1");
        registry.register(id.clone(), SessionActor::spawn(agent));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "ask".into(),
                    text: Some("what are you working on?".into()),
                },
                &ctx(),
            )
            .await;
        println!("LIVE member ask -> {}", out.output);
        assert!(!out.is_error, "{}", out.output);
        assert!(!out.output.trim().is_empty());
    }
}
