use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::compaction::{self, CompactOutcome, CompactionPolicy};
use crate::event::AgentEvent;
use crate::hooks::HooksSet;
use crate::loop_::{LoopConfig, LoopError, run_loop};
use crate::message::{AgentMessage, StopReason};
use crate::session::{Session, SessionEntry};
use crate::streamfn::{LlmOpts, StreamFn};
use crate::tool::Tool;

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
}

pub struct Agent {
    system: String,
    tools: Vec<Tool>,
    llm: LlmOpts,
    stream_fn: StreamFn,
    hooks: HooksSet,
    session: Option<Session>,
    working_dir: PathBuf,
    max_turns: usize,
    parallel_tools: bool,
    compaction: CompactionPolicy,
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
            system: cfg.system,
            tools: cfg.tools,
            llm: cfg.llm,
            stream_fn: cfg.stream_fn,
            hooks: cfg.hooks,
            session: cfg.session,
            working_dir,
            max_turns: cfg.max_turns,
            parallel_tools: cfg.parallel_tools,
            compaction: cfg.compaction,
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

    /// Swaps the model mid-conversation (takes effect on the next run) and
    /// logs a `ModelChange` entry when a session is open.
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
