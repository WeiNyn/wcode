use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::compaction::{self, CompactOutcome, CompactionPolicy};
use crate::event::{AgentEvent, LlmStreamEvent};
use crate::hooks::{HooksSet, PlanModeHandle};
use crate::loop_::{LoopConfig, LoopError, run_loop};
use crate::message::{AgentMessage, StopReason, Usage};
use crate::session::{Session, SessionEntry};
use crate::streamfn::{LlmOpts, StreamFn};
use crate::tool::Tool;

/// The system-prompt section appended while plan mode is on (D2). The kernel owns
/// it so a socket client never has to compose the server's prompt.
pub const PLAN_SECTION: &str = "# Plan mode\nYou are planning, not executing. Explore and propose only — do NOT modify the\nworkspace: the editing tools are disabled and mutating shell commands are refused.\nFine-tune the plan with the user first. When the plan is final, record it as a\ntodo list (status pending), then tell the user to run `/plan off` to execute.";

pub struct AgentConfig {
    pub system: String,
    pub tools: Vec<Tool>,
    pub llm: LlmOpts,
    pub stream_fn: StreamFn,
    pub hooks: HooksSet,
    pub session: Option<Session>, // None = no persistence
    /// Prior messages seeding the conversation (e.g. resumed session history).
    pub context: Vec<AgentMessage>,
    /// Working directory for tool execution; empty falls back to the process
    /// cwd (bash runs here, file paths resolve) — plumb it so embeddings can
    /// run against a directory other than `current_dir()`.
    pub working_dir: PathBuf,
    /// Maximum number of LLM turns in a single `run`; caps a runaway tool loop
    /// (see [`crate::loop_::LoopConfig::max_turns`]). Use
    /// [`crate::loop_::DEFAULT_MAX_TURNS`] for the kernel default.
    pub max_turns: usize,
    /// Run concurrency-safe tool calls in one batch in parallel. `false`
    /// restores strictly-sequential execution.
    pub parallel_tools: bool,
    /// When to compact and how much recent context to keep
    /// (see [`crate::compaction`]). Use [`CompactionPolicy::default`].
    pub compaction: CompactionPolicy,
    /// The shared plan-mode switch; the same handle is held by the
    /// `PlanModeHooks` in `hooks`, so `set_plan_mode` flips both.
    pub plan_mode: PlanModeHandle,
}

pub struct Agent {
    system: String,
    /// The composed prompt as passed (plan mode off); `system` derives from it.
    base_system: String,
    tools: Vec<Tool>,
    llm: LlmOpts,
    stream_fn: StreamFn,
    hooks: HooksSet,
    session: Option<Session>,
    working_dir: PathBuf,
    max_turns: usize,
    parallel_tools: bool,
    compaction: CompactionPolicy,
    plan_mode: PlanModeHandle,
    ctx: Vec<AgentMessage>,
    steer_tx: UnboundedSender<AgentMessage>,
    steer_rx: Option<UnboundedReceiver<AgentMessage>>,
    follow_tx: UnboundedSender<AgentMessage>,
    follow_rx: Option<UnboundedReceiver<AgentMessage>>,
    cancel: CancellationToken,
}

impl Agent {
    pub fn new(mut cfg: AgentConfig) -> Agent {
        // Stable routing id for providers (OpenCode Go `x-opencode-session`):
        // session header wins (a resumed Agent inherits `llm` from the old
        // one, so the opt may be stale), else explicit opt, else a per-agent
        // id (covers --no-session / in-memory).
        cfg.llm.session_id = cfg
            .session
            .as_ref()
            .and_then(session_header_id)
            .or_else(|| cfg.llm.session_id.clone())
            .or_else(|| Some(uuid::Uuid::new_v4().to_string()));
        let (steer_tx, steer_rx) = mpsc::unbounded_channel();
        let (follow_tx, follow_rx) = mpsc::unbounded_channel();
        // An empty working_dir means "process cwd" (the historical default).
        let working_dir = if cfg.working_dir.as_os_str().is_empty() {
            default_working_dir()
        } else {
            cfg.working_dir
        };
        Agent {
            system: cfg.system.clone(),
            base_system: cfg.system,
            tools: cfg.tools,
            llm: cfg.llm,
            stream_fn: cfg.stream_fn,
            hooks: cfg.hooks,
            session: cfg.session,
            working_dir,
            max_turns: cfg.max_turns,
            parallel_tools: cfg.parallel_tools,
            compaction: cfg.compaction,
            plan_mode: cfg.plan_mode,
            ctx: cfg.context,
            steer_tx,
            steer_rx: Some(steer_rx),
            follow_tx,
            follow_rx: Some(follow_rx),
            cancel: CancellationToken::new(),
        }
    }

    /// Queued for injection at the next turn start (this run or a later one).
    pub fn steer(&self, m: AgentMessage) {
        let _ = self.steer_tx.send(m);
    }

    /// Toggle plan mode: flips the shared handle and recomposes `system` from
    /// `base_system` — appending [`PLAN_SECTION`] when on, restoring the base
    /// prompt exactly when off. Takes effect on the NEXT run (the in-flight one
    /// cloned `system` into its `LoopConfig` at start). Not persisted.
    pub fn set_plan_mode(&mut self, on: bool) {
        self.plan_mode.set(on);
        self.system = if on {
            format!("{}\n\n{}", self.base_system, PLAN_SECTION)
        } else {
            self.base_system.clone()
        };
    }
    /// Swaps the model mid-conversation (takes effect on the next run) and
    /// logs a `ModelChange` entry when a session is open. The new id's
    /// `[models.<id>]` profile re-points `endpoint`/`base_url`/`api_key` by
    /// settling on the launch provider first, so an unmapped id — and a switch
    /// back to a previous model — falls back to it (the switch is reversible).
    pub fn set_model(&mut self, model: String) -> std::io::Result<()> {
        // Log first: a failed append must not leave the agent and the session
        // disagreeing about which model runs next.
        if let Some(session) = &mut self.session {
            session.append(SessionEntry::ModelChange {
                id: uuid::Uuid::new_v4().to_string(),
                model: model.clone(),
            })?;
        }
        self.llm.model = model;
        // Settle the new id's provider: the launch provider, then this model's
        // `[models.<id>]` overlay (a no-op when no profiles are configured).
        self.llm.settle_provider();
        Ok(())
    }

    /// Current reasoning effort (None = send nothing).
    pub fn effort(&self) -> Option<&str> {
        self.llm.effort.as_deref()
    }

    /// Swaps the reasoning effort mid-conversation (next run) and logs an
    /// `EffortChange` entry when a session is open. None clears it.
    pub fn set_effort(&mut self, effort: Option<String>) -> std::io::Result<()> {
        if let Some(session) = &mut self.session {
            session.append(SessionEntry::EffortChange {
                id: uuid::Uuid::new_v4().to_string(),
                effort: effort.clone(),
            })?;
        }
        self.llm.effort = effort;
        Ok(())
    }

    /// Sender clone for steering while `run` holds the `&mut` borrow (UI tasks).
    ///
    /// `run_loop` hands the lent channels back at the end of every run, so a
    /// clone taken once stays valid across runs and a message sent while a run
    /// is in flight — even one the loop never drained — reaches the next run.
    /// (Only an *error* exit reboots the channels, invalidating old clones.)
    pub fn steer_sender(&self) -> UnboundedSender<AgentMessage> {
        self.steer_tx.clone()
    }

    /// Runs after the loop would stop.
    pub fn follow_up(&self, m: AgentMessage) {
        let _ = self.follow_tx.send(m);
    }

    /// Sender clone for injecting a follow-up while `run` holds the `&mut`
    /// borrow (the mirror of [`Agent::steer_sender`], for a UI/actor task that
    /// cannot reach `follow_up` directly).
    pub fn follow_up_sender(&self) -> UnboundedSender<AgentMessage> {
        self.follow_tx.clone()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Token for the run in flight or the next one; re-acquire after each
    /// `run()` (a fresh token is minted at the end of every run).
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// The hook set, exposed so the actor can run the inbound policy
    /// ([`HooksSet::before_inbound`]) while `run` holds the `&mut` borrow.
    pub fn hooks(&self) -> &HooksSet {
        &self.hooks
    }

    pub async fn run(
        &mut self,
        user_text: &str,
        sink: UnboundedSender<AgentEvent>,
    ) -> Result<StopReason, LoopError> {
        // The user message is persisted before the loop takes over; the loop
        // records every subsequent message as it enters ctx, so a crash
        // mid-run loses at most the in-flight message instead of the whole
        // turn (see `record_session` in loop_).
        let user = AgentMessage::user_text(user_text);
        if let Some(session) = &mut self.session {
            session
                .append(SessionEntry::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    parent_id: None,
                    message: user.clone(),
                })
                .map_err(LoopError::Session)?;
        }
        self.ctx.push(user);

        let steering = self.steer_rx.take().expect("steer_rx present between runs");
        let follow_ups = self
            .follow_rx
            .take()
            .expect("follow_rx present between runs");
        let cfg = LoopConfig {
            system: self.system.clone(),
            tools: self.tools.clone(),
            llm: self.llm.clone(),
            stream_fn: Arc::clone(&self.stream_fn),
            hooks: self.hooks.clone(),
            steering,
            follow_ups,
            cancel: self.cancel.clone(),
            working_dir: self.working_dir.clone(),
            max_turns: self.max_turns,
            parallel: self.parallel_tools,
            compaction: self.compaction,
            session: self.session.as_mut(),
        };
        let res = run_loop(&mut self.ctx, cfg, sink).await;

        // Fresh token for the next run: CancellationToken has no reset, so a
        // cancel from this run must not poison the next one.
        self.cancel = CancellationToken::new();

        match res {
            Ok(result) => {
                // run_loop hands the lent channels back, so steer()/follow_up()
                // and any sender clone taken before the run keep working, and a
                // message sent while the run was in flight — even one the loop
                // never drained — reaches the next run.
                self.steer_rx = Some(result.steering);
                self.follow_rx = Some(result.follow_ups);
                // An `after_run` hook may act on the finished run (the A2A
                // auto-report forwards a worker's result to its orchestrator).
                self.hooks.after_run(&self.ctx, result.stop_reason).await;
                Ok(result.stop_reason)
            }
            Err(e) => {
                // Error exit: the channels were dropped with the error. Reboot
                // both sides so a later steer()/follow_up() still work
                // (pre-run sender clones are stale after an error).
                let (steer_tx, steer_rx) = mpsc::unbounded_channel();
                self.steer_tx = steer_tx;
                self.steer_rx = Some(steer_rx);
                let (follow_tx, follow_rx) = mpsc::unbounded_channel();
                self.follow_tx = follow_tx;
                self.follow_rx = Some(follow_rx);
                Err(e)
            }
        }
    }

    pub fn messages(&self) -> &[AgentMessage] {
        &self.ctx
    }

    /// Record an inbound peer message into the conversation *without* starting a
    /// turn (the `Request::Notify` semantic): persist it as a user message and
    /// push it onto the context, so the next run sees it. Returns the recorded
    /// message so the actor can surface it. Idle-only — the actor routes a
    /// `Notify` that arrives mid-run through steering instead.
    pub fn notify(&mut self, content: String) -> std::io::Result<AgentMessage> {
        let message = AgentMessage::user_text(content);
        if let Some(session) = &mut self.session {
            session.append(SessionEntry::Message {
                id: uuid::Uuid::new_v4().to_string(),
                parent_id: None,
                message: message.clone(),
            })?;
        }
        self.ctx.push(message.clone());
        Ok(message)
    }

    /// Answer `text` tool-free, from the current context, as a one-off side call.
    ///
    /// CLONES `ctx`, appends the (framed) question, runs the raw `stream_fn` once
    /// with an EMPTY tool list, and folds `TextDelta` → text until `Done`
    /// (capturing `usage`). `self.ctx` and `self.session` are never mutated or
    /// appended — hence `&self`, so it stays callable while the actor holds
    /// `&mut Agent` for a run.
    ///
    /// Errors become [`LoopError::Stream`]: a stream `Error`, or a call that
    /// ended with no text. No retry loop of its own — it inherits whatever
    /// retry/idle-timeout the configured `stream_fn` adapter applies internally,
    /// exactly as [`crate::compaction::summarize`] does. No cancel path in v1.
    pub async fn side_ask(&self, text: &str) -> Result<SideAnswer, LoopError> {
        let mut msgs = self.ctx.clone();
        msgs.push(AgentMessage::user_text(frame_btw(text)));
        let mut stream = (self.stream_fn)(&msgs, &self.system, &[], &self.llm);
        let mut out = String::new();
        let mut usage = None;
        while let Some(ev) = stream.next().await {
            match ev {
                LlmStreamEvent::TextDelta(delta) => out.push_str(&delta),
                LlmStreamEvent::Done { usage: u, .. } => {
                    usage = u;
                    break;
                }
                LlmStreamEvent::Error { message, .. } => return Err(LoopError::Stream(message)),
                _ => {}
            }
        }
        let text = out.trim().to_string();
        if text.is_empty() {
            return Err(LoopError::Stream("side answer produced no text".to_string()));
        }
        Ok(SideAnswer { text, usage })
    }
    /// Summarize the older part of the conversation in place: keep the newest
    /// messages per [`CompactionPolicy`] and replace the rest with a single
    /// summary message. `instructions` focuses the summary (the
    /// `/compact <prompt>` argument).
    ///
    /// When a session is open, the boundary is persisted
    /// ([`crate::session::SessionEntry::Compaction`]), so a resumed session
    /// rebuilds to the same `summary + kept` view instead of re-expanding.
    pub async fn compact(&mut self, instructions: Option<&str>) -> Result<CompactOutcome, String> {
        compaction::compact_ctx(
            &mut self.ctx,
            &self.compaction,
            &self.stream_fn,
            &self.llm,
            instructions,
            self.session.as_mut(),
        )
        .await
    }

    pub fn session_path(&self) -> Option<&Path> {
        self.session.as_ref().and_then(|s| s.path())
    }

    /// The session's stable id (from its `Header`), if a session is open. The
    /// address a [`SessionHandle`](crate::actor::SessionHandle)'s frames carry.
    pub fn session_id(&self) -> Option<String> {
        self.session.as_ref().and_then(session_header_id)
    }
}

/// The reply to a `/btw` side question: the answer text and the turn's token
/// usage (`None` when the provider reported none). Harness data — no styling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SideAnswer {
    pub text: String,
    pub usage: Option<Usage>,
}

/// Frame a raw `/btw` question so the model knows it is a tool-free aside: the
/// tools are simply absent, so without a hint it may try to call one and fail.
fn frame_btw(q: &str) -> String {
    format!("[side question — no tools available; answer briefly and directly from the conversation above]\n{q}")
}

/// Process cwd, used only when `AgentConfig.working_dir` is left empty.
fn default_working_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Id from the session's `Header` entry, if present (old files may lack one).
fn session_header_id(session: &Session) -> Option<String> {
    session.entries().iter().find_map(|e| match e {
        SessionEntry::Header { id, .. } => Some(id.clone()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction::CompactionPolicy;
    use crate::hooks::HooksSet;
    use crate::streamfn::{LlmStream, StreamFn};

    /// A `StreamFn` that replays `script` on each call.
    fn scripted(script: Vec<LlmStreamEvent>) -> StreamFn {
        Arc::new(move |_ctx: &[AgentMessage], _system, _tools, _opts: &LlmOpts| {
            Box::pin(futures::stream::iter(script.clone())) as LlmStream
        })
    }

    fn make_agent(session: Option<Session>, stream_fn: StreamFn, context: Vec<AgentMessage>) -> Agent {
        Agent::new(AgentConfig {
            system: "sys".into(),
            tools: vec![],
            llm: LlmOpts {
                model: "m1".into(),
                ..LlmOpts::default()
            },
            stream_fn,
            hooks: HooksSet::default(),
            session,
            context,
            working_dir: PathBuf::new(),
            max_turns: crate::loop_::DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
            plan_mode: PlanModeHandle::new(),
        })
    }

    #[tokio::test]
    async fn side_ask_leaves_ctx_and_session_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::create(dir.path()).unwrap();
        let stream = scripted(vec![
            LlmStreamEvent::TextDelta("an".into()),
            LlmStreamEvent::TextDelta("swer".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);
        let agent = make_agent(Some(session), stream, vec![AgentMessage::user_text("earlier")]);

        let ctx_before = agent.ctx.clone();
        let path = agent.session.as_ref().unwrap().path().unwrap().to_path_buf();
        let entries_before = agent.session.as_ref().unwrap().entries().to_vec();
        let bytes_before = std::fs::read(&path).unwrap();

        let answer = agent.side_ask("why?").await.unwrap();
        assert_eq!(answer.text, "answer");

        assert_eq!(agent.ctx, ctx_before, "ctx is unchanged");
        assert_eq!(agent.messages(), &ctx_before[..]);
        assert_eq!(
            agent.session.as_ref().unwrap().entries(),
            &entries_before[..],
            "session entries unchanged"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes_before,
            "session file bytes unchanged"
        );
    }

    #[tokio::test]
    async fn side_ask_folds_deltas_and_captures_usage() {
        let usage = Usage {
            input_tokens: 7,
            output_tokens: 3,
            cache_read_tokens: Some(2),
            cache_write_tokens: None,
        };
        let stream = scripted(vec![
            LlmStreamEvent::TextDelta("he".into()),
            LlmStreamEvent::TextDelta("llo".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: Some(usage),
            },
        ]);
        let agent = make_agent(None, stream, vec![]);
        assert_eq!(
            agent.side_ask("hi").await.unwrap(),
            SideAnswer {
                text: "hello".into(),
                usage: Some(usage),
            }
        );
    }

    #[tokio::test]
    async fn side_ask_surfaces_a_stream_error() {
        let stream = scripted(vec![LlmStreamEvent::Error {
            message: "boom".into(),
            fatal: true,
        }]);
        let agent = make_agent(None, stream, vec![]);
        let err = agent.side_ask("hi").await.unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
    }

    #[tokio::test]
    async fn side_ask_errors_on_empty_output() {
        let stream = scripted(vec![LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        }]);
        let agent = make_agent(None, stream, vec![]);
        let err = agent.side_ask("hi").await.unwrap_err();
        assert!(err.to_string().contains("no text"), "{err}");
    }

    #[test]
    fn frame_btw_wraps_the_question() {
        let framed = frame_btw("why?");
        assert!(framed.starts_with("[side question"));
        assert!(framed.ends_with("\nwhy?"));
    }

    #[test]
    fn set_plan_mode_appends_and_restores_the_section() {
        let mut agent = make_agent(None, scripted(vec![]), vec![]);
        let base = agent.system.clone();

        agent.set_plan_mode(true);
        assert!(agent.plan_mode.get());
        assert_eq!(agent.system, format!("{base}\n\n{PLAN_SECTION}"));
        assert!(agent.system.contains("# Plan mode"));
        assert!(
            agent.system.contains("record it as a\ntodo list (status pending)"),
            "the P2 plan\u{2192}todo instruction is present"
        );

        agent.set_plan_mode(false);
        assert!(!agent.plan_mode.get());
        assert_eq!(agent.system, base, "restored exactly");
    }
}
