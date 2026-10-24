use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;
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
    /// Auto-title after this many assistant turns; `0` disables it (W031
    /// **D2**). Use [`crate::title::DEFAULT_TITLE_AFTER_TURNS`]. Plumbed into
    /// `LoopConfig::title_after_turns` by [`Agent::run`] — the kernel never
    /// reads it outside the loop, so there is no `Agent` mirror of it.
    pub title_after_turns: usize,
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
    /// W031 D2: assistant turns before the auto-titler fires; `0` = off.
    title_after_turns: usize,
    /// The in-memory mirror of the session's persisted `Title` entry. Only
    /// authoritative when no session is open — see [`Agent::title`].
    title: Option<String>,
    ctx: Vec<AgentMessage>,
    steer_tx: UnboundedSender<AgentMessage>,
    steer_rx: Option<UnboundedReceiver<AgentMessage>>,
    follow_tx: UnboundedSender<AgentMessage>,
    follow_rx: Option<UnboundedReceiver<AgentMessage>>,
    cancel: CancellationToken,
    /// The context snapshot the run republishes at each turn boundary (see
    /// `LoopConfig::ctx_snapshot`); [`Agent::side_handle`] clones it so the actor
    /// can answer a mid-run read without the `&mut` the run holds.
    ctx_snapshot: watch::Sender<Vec<AgentMessage>>,
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
        // Seeded with the initial context so a read before the first turn sees
        // it; `run_loop` republishes at each boundary. `send_replace` is
        // infallible even with no receivers, so only the sender is held.
        let (ctx_snapshot, _ctx_rx) = watch::channel(cfg.context.clone());
        // An empty working_dir means "process cwd" (the historical default).
        let working_dir = if cfg.working_dir.as_os_str().is_empty() {
            default_working_dir()
        } else {
            cfg.working_dir
        };
        // The title mirrors the session's persisted `Title` entry, seeded here
        // because `cfg.session` is MOVED into the literal below — the same
        // before-the-move shape as the routing id above.
        let title = cfg.session.as_ref().and_then(|s| s.title());
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
            title_after_turns: cfg.title_after_turns,
            title,
            ctx: cfg.context,
            steer_tx,
            steer_rx: Some(steer_rx),
            follow_tx,
            follow_rx: Some(follow_rx),
            cancel: CancellationToken::new(),
            ctx_snapshot,
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

    /// The shared plan-mode switch — the read side of [`Agent::set_plan_mode`].
    /// The actor clones it before `agent.run` borrows the agent (as it does
    /// `hooks`), so a mid-run `Request::Settings` can report plan mode without
    /// the `&mut`.
    ///
    /// Distinct from [`crate::agent::AgentConfig::plan_mode`], which is the
    /// STARTUP flag (`[plan] on` in config); this is the live handle that flag
    /// seeds. Two names, two facts — kept apart rather than aliased, so a reader
    /// cannot mistake the config's intent for the session's current state.
    pub(crate) fn plan_mode(&self) -> PlanModeHandle {
        self.plan_mode.clone()
    }

    /// Assistant turns before the auto-titler fires (`0` = off) — the resolved
    /// `[session] title_after_turns` this agent was built with (W031).
    pub fn title_after_turns(&self) -> usize {
        self.title_after_turns
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

    /// Give this session a name: append a `Title` entry (last write wins) and
    /// mirror it in memory. The EXPLICIT rename path — `/rename` on both human
    /// surfaces, `--title`, and `crate::protocol::Request::SetTitle` — never
    /// calls a model.
    ///
    /// Appends BEFORE the in-memory write, deliberately (the [`Agent::set_model`]
    /// rationale, "a failed append must not leave the agent and the session
    /// disagreeing"): if the append fails the caller gets the `io::Error` and
    /// [`Agent::title`] is unchanged, so a UI never claims a name the file does
    /// not have.
    ///
    /// # Errors
    /// The session append's `io::Error`, unchanged. The in-memory mirror is NOT
    /// updated on failure.
    ///
    /// # Does not
    /// Sanitize (`/rename` takes the user's text verbatim; the AUTO path
    /// sanitizes, in [`crate::title::sanitize`]), call a model, or refuse an
    /// empty string — `set_title("")` stores an empty title and [`Agent::title`]
    /// then returns `Some("")`.
    pub fn set_title(&mut self, title: &str) -> std::io::Result<()> {
        if let Some(session) = &mut self.session {
            session.append(SessionEntry::Title {
                id: uuid::Uuid::new_v4().to_string(),
                title: title.to_string(),
            })?;
        }
        self.title = Some(title.to_string());
        Ok(())
    }

    /// The session's current title (`None` = never titled).
    ///
    /// Reads the SESSION first, the in-memory mirror second — and that order is
    /// the whole contract. The auto-titler runs inside `run_loop`, which holds
    /// `&mut Session` and no `&mut Agent`, so a title it appends lands in the
    /// session and NOT in an `Agent` field; a mirror-only getter would report
    /// `None` right after a successful auto-title, and a UI asking the actor
    /// would disagree with the file. Session-first makes the persisted value
    /// authoritative; the mirror covers `--no-session`, where there is no
    /// session to ask.
    pub fn title(&self) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|s| s.title())
            .or_else(|| self.title.clone())
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
            let appended = session
                .append(SessionEntry::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    parent_id: None,
                    message: user.clone(),
                })
                .map_err(LoopError::Session);
            if let Err(e) = appended {
                // A session-append I/O failure on this very first write exits the
                // run *before* the loop, on a path the `Ok` arm far below never
                // sees. Wake the owner here too, so a worker's report can never
                // silently vanish; `StopReason::Error` makes `ReportBack`
                // synthesize a status line rather than forward a stale prior reply.
                self.hooks.after_run(&self.ctx, StopReason::Error).await;
                return Err(e);
            }
        }
        self.ctx.push(user);

        // Cover the window between the submit and the first turn boundary: a
        // read arriving before the loop's first publish still sees the question.
        let _ = self.ctx_snapshot.send_replace(self.ctx.clone());

        let steering = self.steer_rx.take().expect("steer_rx present between runs");
        let follow_ups = self
            .follow_rx
            .take()
            .expect("follow_rx present between runs");
        // Read before the literal: the accessor takes `&self` (the WHOLE struct),
        // and `session: self.session.as_mut()` inside the literal holds a `&mut`
        // borrow of it. A disjoint field READ (`self.title_after_turns`) would be
        // fine there; the method call is not.
        let title_after_turns = self.title_after_turns();
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
            ctx_snapshot: self.ctx_snapshot.clone(),
            session: self.session.as_mut(),
            title_after_turns,
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
                // A `LoopError` never reaches the `Ok` arm above, so the owner of a
                // worker would block forever on a report that never fires. Wake it
                // here too; `StopReason::Error` makes `ReportBack` synthesize a
                // status line rather than forward a stale prior reply.
                self.hooks.after_run(&self.ctx, StopReason::Error).await;
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
        side_ask_with(&self.stream_fn, &self.ctx, &self.system, &self.llm, text).await
    }

    /// A read-only handle the actor takes *before* a run borrows the agent, so a
    /// mid-run `GetHistory`/`SideAsk` is answered from the context snapshot
    /// `run_loop` republishes each turn, instead of waiting for the whole run.
    /// See [`SideHandle`].
    pub fn side_handle(&self) -> SideHandle {
        SideHandle {
            stream_fn: Arc::clone(&self.stream_fn),
            system: self.system.clone(),
            llm: self.llm.clone(),
            ctx: self.ctx_snapshot.clone(),
        }
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

    /// The provider options this agent bills against — the `base_url`/`model`
    /// pair `pricing::model_price` keys on. Read-only, so the actor can price a
    /// usage report without the `&mut` a run holds.
    pub(crate) fn llm(&self) -> &LlmOpts {
        &self.llm
    }
}

/// The reply to a `/btw` side question: the answer text and the turn's token
/// usage (`None` when the provider reported none). Harness data — no styling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SideAnswer {
    pub text: String,
    pub usage: Option<Usage>,
}

/// A read-only view of an [`Agent`] the actor can mine *while a run holds the
/// agent's `&mut`. It carries the same clones the run uses (`stream_fn`/
/// `system`/`llm`) plus the context snapshot `run_loop` republishes each turn,
/// so a mid-run `GetHistory`/`SideAsk` is answered from the latest completed
/// turns instead of waiting for the whole run to finish.
#[derive(Clone)]
pub struct SideHandle {
    stream_fn: StreamFn,
    system: String,
    llm: LlmOpts,
    ctx: watch::Sender<Vec<AgentMessage>>,
}

impl SideHandle {
    /// The latest turn-boundary context snapshot, owned (`send_replace` keeps
    /// the value even with no receivers, so a read after the run still sees it).
    pub fn history(&self) -> Vec<AgentMessage> {
        self.ctx.subscribe().borrow().to_vec()
    }

    /// The latest snapshot's last-assistant text — a compact status read
    /// (`crate::protocol::Request::Status`) that borrows the watch value instead
    /// of cloning the whole transcript.
    pub fn last_assistant_text(&self) -> Option<String> {
        let rx = self.ctx.subscribe();
        let messages = rx.borrow();
        crate::message::last_assistant_text(&messages[..])
    }

    /// The provider options captured when the handle was taken — what a
    /// MID-RUN usage report is priced with (`Request::Usage`).
    pub(crate) fn llm(&self) -> &LlmOpts {
        &self.llm
    }

    /// Answer `text` tool-free from the latest snapshot; semantics match
    /// [`Agent::side_ask`].
    pub async fn side_ask(&self, text: &str) -> Result<SideAnswer, LoopError> {
        side_ask_with(&self.stream_fn, &self.history(), &self.system, &self.llm, text).await
    }
}

/// The shared body of a side ask: frame `text` as a tool-free aside, run the raw
/// `stream_fn` once with an EMPTY tool list, and fold `TextDelta` → text until
/// `Done` (capturing `usage`). [`Agent::side_ask`] passes `&self.ctx`;
/// [`SideHandle::side_ask`] passes the latest mid-run snapshot.
async fn side_ask_with(
    stream_fn: &StreamFn,
    ctx: &[AgentMessage],
    system: &str,
    llm: &LlmOpts,
    text: &str,
) -> Result<SideAnswer, LoopError> {
    let mut msgs = ctx.to_vec();
    msgs.push(AgentMessage::user_text(frame_btw(text)));
    let mut stream = (stream_fn)(&msgs, system, &[], llm);
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
            // W031 test double: the kernel default, like every non-CLI site.
            title_after_turns: crate::title::DEFAULT_TITLE_AFTER_TURNS,
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
    fn agent_config_title_after_turns_reaches_the_agent() {
        // The `AgentConfig` → `Agent` half of the plumbing (the CLI half is pinned
        // in `repl.rs`). A non-default value, `0`, and the default all survive, so
        // a "hardcoded default" or an "off is silently ignored" regression fails.
        fn build(title_after_turns: usize, session: Option<Session>) -> Agent {
            Agent::new(AgentConfig {
                system: "sys".into(),
                tools: vec![],
                llm: LlmOpts::default(),
                stream_fn: scripted(vec![]),
                hooks: HooksSet::default(),
                session,
                context: vec![],
                working_dir: PathBuf::new(),
                max_turns: crate::loop_::DEFAULT_MAX_TURNS,
                parallel_tools: true,
                compaction: CompactionPolicy::default(),
                plan_mode: PlanModeHandle::new(),
                title_after_turns,
            })
        }

        let dir = tempfile::tempdir().unwrap();
        let session = Session::create(dir.path()).unwrap();
        assert_eq!(build(7, Some(session)).title_after_turns(), 7);
        assert_eq!(
            build(0, None).title_after_turns(),
            0,
            "0 = off is carried verbatim, not read as unset"
        );
        assert_eq!(
            build(crate::title::DEFAULT_TITLE_AFTER_TURNS, None).title_after_turns(),
            crate::title::DEFAULT_TITLE_AFTER_TURNS
        );
    }

    #[test]
    fn set_title_appends_a_title_entry() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::create(dir.path()).unwrap();
        let path = session.path().unwrap().to_path_buf();
        let mut agent = make_agent(Some(session), scripted(vec![]), vec![]);
        assert_eq!(agent.title(), None, "a fresh session has no title");

        agent.set_title("retry backoff fix").unwrap();

        assert_eq!(agent.title().as_deref(), Some("retry backoff fix"));
        // The SESSION, not just the mirror: an entry is in the file, exactly one,
        // and the reader a `/resume` uses agrees.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""type":"title""#), "{raw}");
        let reopened = Session::open(&path).unwrap();
        assert_eq!(reopened.title().as_deref(), Some("retry backoff fix"));
        assert_eq!(
            reopened
                .entries()
                .iter()
                .filter(|e| matches!(e, SessionEntry::Title { .. }))
                .count(),
            1,
            "one appended entry, not a rewrite"
        );

        // A second rename appends again and last write wins.
        agent.set_title("second name").unwrap();
        assert_eq!(agent.title().as_deref(), Some("second name"));
        assert_eq!(
            Session::open(&path).unwrap().title().as_deref(),
            Some("second name")
        );
    }

    #[test]
    fn set_title_on_a_failed_append_leaves_the_title_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::create(dir.path()).unwrap();
        let mut agent = make_agent(Some(session), scripted(vec![]), vec![]);
        // The session's directory removed out from under the open handle, so the
        // O_APPEND open fails.
        std::fs::remove_dir_all(dir.path()).unwrap();

        assert!(
            agent.set_title("never persisted").is_err(),
            "the append must fail"
        );
        assert_eq!(
            agent.title(),
            None,
            "the append ran FIRST, so nothing was claimed"
        );
    }

    #[test]
    fn title_reads_the_session_first() {
        // The auto-titler appends behind the agent's back — it holds `&mut
        // Session` and no `&mut Agent` — so the mirror can go STALE while the
        // session holds the truth. A mirror-first getter would return the older
        // name here and a UI would disagree with the file.
        let dir = tempfile::tempdir().unwrap();
        let session = Session::create(dir.path()).unwrap();
        let path = session.path().unwrap().to_path_buf();
        let mut agent = make_agent(Some(session), scripted(vec![]), vec![]);

        agent.set_title("from the agent").unwrap();
        agent
            .session
            .as_mut()
            .unwrap()
            .append(SessionEntry::Title {
                id: "t-loop".into(),
                title: "titled by the loop".into(),
            })
            .unwrap();

        assert_eq!(
            agent.title().as_deref(),
            Some("titled by the loop"),
            "the persisted entry wins over the stale mirror"
        );
        assert_eq!(
            Session::open(&path).unwrap().title().as_deref(),
            Some("titled by the loop")
        );
    }

    #[test]
    fn title_without_a_session_uses_the_mirror() {
        let mut agent = make_agent(None, scripted(vec![]), vec![]);
        assert_eq!(agent.title(), None);
        agent.set_title("no file to write").unwrap();
        assert_eq!(agent.title().as_deref(), Some("no file to write"));
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
