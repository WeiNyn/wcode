use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::event::AgentEvent;
use crate::hooks::Hooks;
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
    pub hooks: Arc<dyn Hooks>,
    pub session: Option<Session>, // None = no persistence
    /// Prior messages seeding the conversation (e.g. resumed session history).
    pub context: Vec<AgentMessage>,
}

pub struct Agent {
    system: String,
    tools: Vec<Tool>,
    llm: LlmOpts,
    stream_fn: StreamFn,
    hooks: Arc<dyn Hooks>,
    session: Option<Session>,
    ctx: Vec<AgentMessage>,
    steer_tx: UnboundedSender<AgentMessage>,
    steer_rx: Option<UnboundedReceiver<AgentMessage>>,
    follow_tx: UnboundedSender<AgentMessage>,
    follow_rx: Option<UnboundedReceiver<AgentMessage>>,
    cancel: CancellationToken,
}

impl Agent {
    pub fn new(cfg: AgentConfig) -> Agent {
        let (steer_tx, steer_rx) = mpsc::unbounded_channel();
        let (follow_tx, follow_rx) = mpsc::unbounded_channel();
        Agent {
            system: cfg.system,
            tools: cfg.tools,
            llm: cfg.llm,
            stream_fn: cfg.stream_fn,
            hooks: cfg.hooks,
            session: cfg.session,
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

    /// Sender clone for steering while `run` holds the `&mut` borrow (UI tasks).
    ///
    /// Receivers are re-paired after each run: clones taken before a run are
    /// invalid for later runs — re-acquire after each `run()` (upgrade path:
    /// run loop hands receivers back between runs).
    pub fn steer_sender(&self) -> UnboundedSender<AgentMessage> {
        self.steer_tx.clone()
    }

    /// Runs after the loop would stop.
    pub fn follow_up(&self, m: AgentMessage) {
        let _ = self.follow_tx.send(m);
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
        let len_before = self.ctx.len();
        self.ctx.push(AgentMessage::user_text(user_text));

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
            hooks: Arc::clone(&self.hooks),
            steering,
            follow_ups,
            cancel: self.cancel.clone(),
        };
        let res = run_loop(&mut self.ctx, cfg, sink).await;

        // The receivers were consumed by run_loop: re-pair so steer()/
        // follow_up() keep working. Messages queued between runs sit in the
        // stored receivers and reach the next run; only messages still queued
        // when the loop exits (any run exit; the outer loop checks only
        // follow_ups) are dropped.
        // ponytail: exit-time stragglers dropped; have run_loop hand the
        // receivers back if that ever matters.
        let (steer_tx, steer_rx) = mpsc::unbounded_channel();
        self.steer_tx = steer_tx;
        self.steer_rx = Some(steer_rx);
        let (follow_tx, follow_rx) = mpsc::unbounded_channel();
        self.follow_tx = follow_tx;
        self.follow_rx = Some(follow_rx);

        // Fresh token for the next run: CancellationToken has no reset, so a
        // cancel from this run must not poison the next one.
        self.cancel = CancellationToken::new();

        if let Some(session) = &mut self.session {
            for m in &self.ctx[len_before..] {
                session
                    .append(SessionEntry::Message {
                        id: uuid::Uuid::new_v4().to_string(),
                        parent_id: None,
                        message: m.clone(),
                    })
                    .map_err(LoopError::Session)?;
            }
        }
        res
    }

    pub fn messages(&self) -> &[AgentMessage] {
        &self.ctx
    }

    pub fn session_path(&self) -> Option<&Path> {
        self.session.as_ref().and_then(|s| s.path())
    }
}
