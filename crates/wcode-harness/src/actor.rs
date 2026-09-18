//! In-process session actor — one mailbox, one stream.
//!
//! [`SessionActor`] turns an [`Agent`] into a task with an inbox and a
//! subscribable outbox, so a caller drives the session entirely through
//! [`Request`]s and observes it through [`AgentEvent`]s — never by holding the
//! `Agent` itself:
//!
//! ```text
//!   caller  (REPL / TUI / future peer session — all identical here)
//!     │  send(Request)                          ▲  subscribe() → recv()
//!     ▼                                         │  (own cursor per subscriber)
//!   inbox: mpsc::Unbounded<Request>       outbox: broadcast<AgentEvent>
//!     │                                         ▲  events.send(ev)
//!     └───────────► SessionActor task ◄─────────┘
//!                   owns the Agent; serves one Request at a time;
//!                   `deferred: VecDeque<Request>` parks mid-run commands
//! ```
//!
//! This is the seam the whole protocol design hangs on. A REPL, a TUI, and
//! (later) a peer session are all just callers that hold a [`SessionHandle`];
//! they differ only in how they render the stream and which address they hold,
//! not in the mechanism they speak. Today the inbox and outbox are in-process
//! channels; the shapes are chosen so that a socket ([`Frame`]s over NDJSON,
//! stage S2) can replace the transport without changing this API.
//!
//! ## Flow
//!
//! `serve()` — no run in flight: pull ONE Request, service it, repeat.
//!
//! ```text
//!   deferred non-empty? ──yes──► take from deferred (FIFO, order kept)
//!        │ no
//!        ▼
//!   inbox.recv().await ──► match Request:
//!        Submit    {..} ─► run(...)              → enters the in-run map below
//!        Notify    {..} ─► agent.notify(tagged)  → append, no turn
//!        Interrupt {..} ─► agent.steer(tagged)   → queued for the next turn
//!        Wake      {..} ─► run(tagged)           → a turn even when idle
//!        Cancel         ─► (no-op — nothing is running)
//!        SetModel  {..} ─► agent.set_model(..)
//!        SetEffort {..} ─► agent.set_effort(..)
//!        Compact   {..} ─► agent.compact(..).await
//!        GetHistory     ─► reply with the context
//!        Unknown        ─► (ignored)
//! ```
//!
//! `run()` — the Agent is borrowed for the whole run, so it is reached only
//! through clones taken *before* the borrow.
//!
//! ```text
//!   steer     = agent.steer_sender()
//!   follow_up = agent.follow_up_sender()
//!   cancel    = agent.cancel_token()
//!
//!   loop { tokio::select! { biased; ... } }   ← biased: inbox polled FIRST
//!        ├─ Some(req) = inbox.recv() ─► Cancel   ─► cancel.cancel()         ┐ to the
//!        │                              Steer    ─► steer.send(msg)         │ live run
//!        │                              FollowUp ─► follow_up.send(msg)     │
//!        │                              other    ─► deferred.push_back(req) ┘ waits
//!        ├─ result  = &mut run  ──────► run finished → break
//!        └─ Some(ev) = sink_rx.recv() ► events.send(ev)  (fan-out to subscribers)
//!   }
//!   then: drain leftover sink_rx → events.send ; on Err(run) → Error event
//! ```
//!
//! [`Frame`]: crate::protocol::Frame
//!
//! ## Delivery rules
//!
//! A run holds the agent's `&mut` for its whole life, so the actor splits the
//! inbox by what each request needs:
//!
//! | request | while a run is in flight |
//! |---------|--------------------------|
//! | `Cancel` | delivered at once — the token is cancelled; the loop aborts |
//! | `Interrupt` / `Wake` | forwarded to the run's channels at once |
//! | `SetModel` / `SetEffort` / `Compact` | deferred, applied when the run ends |
//! | another `Submit` | deferred, becomes the next run |
//!
//! The interactive path therefore never waits on a run. The deferred set is
//! exactly the requests that need `&mut Agent` — you cannot swap the model or
//! compact the context out from under a run — and deferring them is also the
//! documented meaning: `set_model`/`set_effort` "take effect on the next run".
//!
//! One caveat lives in the kernel, not here — the actor hands a `Cancel` over at
//! once, but the *loop* is what acts on it, and its latency depends on where the
//! run is:
//!
//! ```text
//! where the run is when Cancel arrives          when it stops
//! ---------------------------------------------  -----------------------
//! model streaming     (loop races the token)     immediately
//! between turns       (checked at turn start)    immediately
//! inside a tool call  (no race on the future)    when the tool returns *
//! ```
//!
//! `*` unless the tool watches `ToolContext::cancel` itself (e.g. `bash` kills
//! its child) — then it can stop early.
//!
//! ## Requests and replies
//!
//! `send` is fire-and-forget; `ask` awaits a reply. The reply is an
//! [`AgentEvent`] reply variant — [`AgentEvent::History`] for `GetHistory`,
//! [`AgentEvent::Ack`]/[`AgentEvent::Error`] for `SetModel`/`SetEffort`/
//! [`AgentEvent::Compaction`] for `Compact` — correlated in-process by a
//! one-shot channel. Replies are never fanned out on the event stream; the wire
//! (S2) will carry the same variants correlated by the envelope's `reply_to`.
//!
//! Scope so far: every request variant that maps onto an existing [`Agent`]
//! method is serviced. Still to come (S1b): pointing the CLI at a handle.

use std::collections::VecDeque;
use std::fmt;

use tokio::sync::{broadcast, mpsc, oneshot};

use crate::agent::Agent;
use crate::compaction::CompactOutcome;
use crate::event::AgentEvent;
use crate::hooks::HooksSet;
use crate::loop_::LoopError;
use crate::message::{AgentMessage, StopReason};
use crate::protocol::{Request, SessionId};

/// Default outbox buffer: how far a subscriber may lag before it starts losing
/// events (`broadcast` semantics — the slowest reader drops, it never blocks the
/// run).
pub const EVENT_BUFFER: usize = 1024;

/// Handle to a running [`SessionActor`].
///
/// Cloning gives another caller the same mailbox and stream. When the last
/// handle drops, the session shuts down and its [`Agent`] is dropped.
#[derive(Clone)]
pub struct SessionHandle {
    inbox: mpsc::UnboundedSender<Message>,
    events: broadcast::Sender<AgentEvent>,
}

impl SessionHandle {
    /// Submit a request to the session's inbox.
    ///
    /// Fails only if the session has shut down (every handle was dropped and the
    /// actor task has ended). A `Submit` runs to completion before the next
    /// non-interrupt request is serviced, but `Interrupt`/`Wake`/`Cancel` sent
    /// during a run are handled immediately (see the actor's delivery rules).
    pub fn send(&self, request: Request) -> Result<(), SessionClosed> {
        self.tell(None, request)
    }

    /// Like [`SessionHandle::send`], but attributed to a sender address so the
    /// recipient can attribute and policy-check the message (a peer session
    /// passes its own `"agent:<id>"` address).
    pub fn send_from(&self, from: SessionId, request: Request) -> Result<(), SessionClosed> {
        self.tell(Some(from), request)
    }

    fn tell(&self, from: Option<SessionId>, request: Request) -> Result<(), SessionClosed> {
        self.inbox
            .send(Message::Tell { from, request })
            .map_err(|_| SessionClosed)
    }

    /// Send a request and await its reply.
    ///
    /// `ask` is for requests that expect a reply: `GetHistory` (→
    /// [`AgentEvent::History`]), `SetModel`/`SetEffort` (→ [`AgentEvent::Ack`]
    /// or [`AgentEvent::Error`]), `Compact` (→ [`AgentEvent::Compaction`],
    /// [`AgentEvent::Ack`], or [`AgentEvent::Error`]). It also works for
    /// `Submit`, replying when the run completes. Interrupts
    /// (`Interrupt`/`Wake`/`Cancel`) want no reply — use `send`; an `ask` on one
    /// is deferred like any command while a run is in flight.
    ///
    /// Fails only if the session has shut down.
    pub async fn ask(&self, request: Request) -> Result<AgentEvent, SessionClosed> {
        self.question(None, request).await
    }

    /// Like [`SessionHandle::ask`], attributed to a sender address.
    pub async fn ask_from(
        &self,
        from: SessionId,
        request: Request,
    ) -> Result<AgentEvent, SessionClosed> {
        self.question(Some(from), request).await
    }

    async fn question(
        &self,
        from: Option<SessionId>,
        request: Request,
    ) -> Result<AgentEvent, SessionClosed> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.inbox
            .send(Message::Ask {
                from,
                request,
                reply: reply_tx,
            })
            .map_err(|_| SessionClosed)?;
        reply_rx.await.map_err(|_| SessionClosed)
    }

    /// Subscribe to the session's event stream. Each subscriber gets its own
    /// receiver; a lagging subscriber drops events rather than stalling the run.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }
}

/// Returned when a [`SessionHandle::send`] targets a session that has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionClosed;

impl fmt::Display for SessionClosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("session actor has shut down")
    }
}

impl std::error::Error for SessionClosed {}

/// A mailbox item: fire-and-forget, or a request that expects a reply.
enum Message {
    /// No reply wanted. `from` is the sender's address — `None` for a local
    /// caller (the human), `Some(peer)` when another session sends.
    Tell {
        from: Option<SessionId>,
        request: Request,
    },
    /// The reply to the request is sent here when it is serviced.
    Ask {
        from: Option<SessionId>,
        request: Request,
        reply: oneshot::Sender<AgentEvent>,
    },
}

/// Spawns the actor task that owns an [`Agent`].
///
/// The `Agent` moves into a background task; all interaction returns through the
/// [`SessionHandle`]. The task exits when the last handle drops.
pub struct SessionActor;

impl SessionActor {
    pub fn spawn(agent: Agent) -> SessionHandle {
        let (inbox_tx, inbox_rx) = mpsc::unbounded_channel();
        let (events_tx, _) = broadcast::channel(EVENT_BUFFER);
        let events = events_tx.clone();
        tokio::spawn(serve(agent, inbox_rx, events_tx));
        SessionHandle {
            inbox: inbox_tx,
            events,
        }
    }
}

/// The actor loop: pull one request at a time, service it, repeat.
async fn serve(
    mut agent: Agent,
    mut inbox: mpsc::UnboundedReceiver<Message>,
    events: broadcast::Sender<AgentEvent>,
) {
    // Requests that arrive *during* a run but cannot be applied to it (a
    // second `Submit`, a model swap) wait here and are serviced once the run
    // ends, preserving their order.
    let mut deferred: VecDeque<Message> = VecDeque::new();

    loop {
        let message = match deferred.pop_front() {
            Some(message) => message,
            None => match inbox.recv().await {
                Some(message) => message,
                None => break, // every handle dropped: shut down
            },
        };

        dispatch(&mut agent, message, &mut inbox, &events, &mut deferred).await;
    }
}

/// Prefix an inbound peer message with its sender, so the model sees who sent
/// it and knows the address to reply to (§13.13).
fn tag(sender: &SessionId, content: &str) -> String {
    format!("[message from {sender}]\n{content}")
}

/// Run the inbound policy on an A2A message (the mirror of
/// `before_tool_call`). Returns `Some(reason)` if the message was dropped; the
/// request may have been rewritten in place by the hook either way.
async fn gate_inbound(hooks: &HooksSet, message: &mut Message) -> Option<String> {
    let (from, request) = match message {
        Message::Tell { from, request } | Message::Ask { from, request, .. } => (from, request),
    };
    if !crate::protocol::is_inbound(request) {
        return None;
    }
    hooks.before_inbound(from.as_ref(), request).await
}

/// Service one mailbox item: run the request, then answer an `Ask` with the
/// reply it produced (a `Tell` drops the reply on the floor).
async fn dispatch(
    agent: &mut Agent,
    mut message: Message,
    inbox: &mut mpsc::UnboundedReceiver<Message>,
    events: &broadcast::Sender<AgentEvent>,
    deferred: &mut VecDeque<Message>,
) {
    // Inbound peer-message policy (S4-1): a drop skips delivery entirely; a
    // rewrite (in place) is what gets serviced.
    if let Some(reason) = gate_inbound(agent.hooks(), &mut message).await {
        if let Message::Ask { reply, .. } = message {
            let _ = reply.send(AgentEvent::Error {
                message: format!("inbound blocked: {reason}"),
            });
        }
        return;
    }

    let (from, request, reply) = match message {
        Message::Tell { from, request } => (from, request, None),
        Message::Ask { from, request, reply } => (from, request, Some(reply)),
    };
    let sender = from.unwrap_or_else(SessionId::user);

    let event = match request {
        Request::Submit { text } => match run(agent, &text, inbox, events, deferred).await {
            Ok(stop_reason) => AgentEvent::Stopped { stop_reason },
            Err(e) => AgentEvent::Error {
                message: e.to_string(),
            },
        },
        // A2A delivery verbs (S4-1). `Notify` appends with no turn;
        // `Interrupt`/`Wake` are the canonical interrupt/continue forms.
        Request::Notify { content } => match agent.notify(tag(&sender, &content)) {
            Ok(_) => {
                let _ = events.send(AgentEvent::MessageReceived {
                    from: sender.clone(),
                    content,
                });
                AgentEvent::Ack
            }
            Err(e) => AgentEvent::Error {
                message: e.to_string(),
            },
        },
        Request::Interrupt { content } => {
            agent.steer(AgentMessage::user_text(tag(&sender, &content)));
            let _ = events.send(AgentEvent::MessageReceived {
                from: sender.clone(),
                content,
            });
            AgentEvent::Ack
        }
        // A `Wake` runs even when idle: start a turn with the (tagged) content,
        // like `Submit` — otherwise a worker handed a task would only queue a
        // follow-up and never act on it.
        Request::Wake { content } => {
            let _ = events.send(AgentEvent::MessageReceived {
                from: sender.clone(),
                content: content.clone(),
            });
            match run(agent, &tag(&sender, &content), inbox, events, deferred).await {
                Ok(stop_reason) => AgentEvent::Stopped { stop_reason },
                Err(e) => AgentEvent::Error {
                    message: e.to_string(),
                },
            }
        }
        // An idle cancel is a no-op (see the delivery rules): the token is
        // minted fresh at the end of each run, so cancelling an idle session
        // would otherwise abort the next run on its first poll.
        Request::Cancel => AgentEvent::Ack,
        Request::SetModel { model } => match agent.set_model(model) {
            Ok(()) => AgentEvent::Ack,
            Err(e) => AgentEvent::Error {
                message: e.to_string(),
            },
        },
        Request::SetEffort { effort } => match agent.set_effort(effort) {
            Ok(()) => AgentEvent::Ack,
            Err(e) => AgentEvent::Error {
                message: e.to_string(),
            },
        },
        Request::Compact { instructions } => match agent.compact(instructions.as_deref()).await {
            Ok(CompactOutcome::NothingToDo) => AgentEvent::Ack,
            Ok(CompactOutcome::Done {
                summarized, kept, ..
            }) => AgentEvent::Compaction { summarized, kept },
            Err(e) => AgentEvent::Error { message: e },
        },
        Request::GetHistory => AgentEvent::History {
            messages: agent.messages().to_vec(),
        },
        // A served peer is defined by a socket server's injected handler; an
        // in-process session has no factory, so it can only refuse.
        Request::Define { .. } => AgentEvent::Error {
            message: "agent definition is served, not in-process".into(),
        },
        Request::ListSessions | Request::Unknown => AgentEvent::Ack,
    };

    if let Some(reply) = reply {
        let _ = reply.send(event);
    }
}

/// Drive one `Agent::run`, forwarding its events to the outbox while still
/// servicing the inbox so `Interrupt`/`Wake`/`Cancel` reach the in-flight run.
async fn run(
    agent: &mut Agent,
    text: &str,
    inbox: &mut mpsc::UnboundedReceiver<Message>,
    events: &broadcast::Sender<AgentEvent>,
    deferred: &mut VecDeque<Message>,
) -> Result<StopReason, LoopError> {
    let (sink_tx, mut sink_rx) = mpsc::unbounded_channel::<AgentEvent>();
    // Sender/token clones taken *before* `run` borrows the agent: steering and
    // cancelling the in-flight run must not need the `&mut`.
    let steer = agent.steer_sender();
    let follow_up = agent.follow_up_sender();
    let cancel = agent.cancel_token();
    let hooks = agent.hooks().clone();

    let mut run = Box::pin(agent.run(text, sink_tx));
    let result = loop {
        // Biased: service the inbox *before* advancing the run. An `Interrupt` must
        // be in the agent's steering channel before the run reaches its next
        // turn-boundary drain, or it is silently a turn late. Polling the run
        // first would let it cross that boundary while the request still sits
        // here.
        tokio::select! {
            biased;
            Some(mut message) = inbox.recv() => {
                if let Some(reason) = gate_inbound(&hooks, &mut message).await {
                    if let Message::Ask { reply, .. } = message {
                        let _ = reply.send(AgentEvent::Error {
                            message: format!("inbound blocked: {reason}"),
                        });
                    }
                    continue;
                }
                match message {
                    Message::Tell {
                        request: Request::Cancel,
                        ..
                    } => cancel.cancel(),
                    Message::Tell {
                        request: Request::Notify { content } | Request::Interrupt { content },
                        from,
                    } => {
                        let from = from.unwrap_or_else(SessionId::user);
                        let _ = steer.send(AgentMessage::user_text(tag(&from, &content)));
                        let _ = events.send(AgentEvent::MessageReceived {
                            from,
                            content,
                        });
                    }
                    Message::Tell {
                        request: Request::Wake { content },
                        from,
                    } => {
                        let from = from.unwrap_or_else(SessionId::user);
                        let _ = follow_up.send(AgentMessage::user_text(tag(&from, &content)));
                        let _ = events.send(AgentEvent::MessageReceived {
                            from,
                            content,
                        });
                    }
                    other => deferred.push_back(other),
                }
            },
            result = &mut run => break result,
            Some(event) = sink_rx.recv() => {
                let _ = events.send(event);
            }
        }
    };

    // The run dropped its sink on completion; forward whatever is still buffered
    // so the last events (e.g. `AgentEnd`) are never stranded.
    while let Ok(event) = sink_rx.try_recv() {
        let _ = events.send(event);
    }

    // A session-write error inside the run is a `LoopError`; the run's own
    // `AgentEvent::Error` covers stream failures, so surface this one the same
    // way rather than dropping it silently.
    if let Err(e) = &result {
        let _ = events.send(AgentEvent::Error {
            message: e.to_string(),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentConfig};
    use crate::compaction::CompactionPolicy;
    use crate::event::LlmStreamEvent;
    use crate::hooks::HooksSet;
    use crate::message::StopReason;
    use crate::streamfn::{LlmOpts, LlmStream, StreamFn};
    use crate::tool::{ToolContext, ToolOutput, TypedTool, erased};
    use futures::StreamExt as _;
    use serde::Deserialize;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone, Default)]
    struct Recorder {
        script: Arc<Mutex<VecDeque<Vec<LlmStreamEvent>>>>,
        calls: Arc<Mutex<Vec<Vec<AgentMessage>>>>,
        models: Arc<Mutex<Vec<String>>>,
    }

    impl Recorder {
        fn push(&self, events: Vec<LlmStreamEvent>) {
            self.script.lock().unwrap().push_back(events);
        }
        fn calls(&self) -> Vec<Vec<AgentMessage>> {
            self.calls.lock().unwrap().clone()
        }
        fn models(&self) -> Vec<String> {
            self.models.lock().unwrap().clone()
        }
    }

    fn fake_stream_fn(rec: &Recorder) -> StreamFn {
        let rec = rec.clone();
        Arc::new(
            move |ctx: &[AgentMessage], _system, _tools, opts: &LlmOpts| {
                rec.calls.lock().unwrap().push(ctx.to_vec());
                rec.models.lock().unwrap().push(opts.model.clone());
                let events = rec.script.lock().unwrap().pop_front().unwrap_or_default();
                Box::pin(futures::stream::iter(events)) as LlmStream
            },
        )
    }

    fn agent_config(stream_fn: StreamFn, tools: Vec<crate::tool::Tool>) -> AgentConfig {
        AgentConfig {
            system: "sys".into(),
            tools,
            llm: LlmOpts {
                model: "m1".into(),
                ..LlmOpts::default()
            },
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::path::PathBuf::new(),
            max_turns: crate::loop_::DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
        }
    }

    /// Block until the run ends, panicking if it stalls.
    async fn wait_for_end(rx: &mut broadcast::Receiver<AgentEvent>) {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("an event within the timeout")
                .expect("the outbox stays open");
            if matches!(event, AgentEvent::AgentEnd) {
                return;
            }
        }
    }

    #[tokio::test]
    async fn submit_streams_events_to_a_subscriber() {
        let rec = Recorder::default();
        rec.push(vec![
            LlmStreamEvent::TextDelta("hello".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let mut rx = handle.subscribe();
        handle.send(Request::Submit { text: "hi".into() }).unwrap();

        let mut saw_text = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("an event")
                .expect("open");
            match event {
                AgentEvent::MessageUpdate { message } if message.as_text() == "hello" => {
                    saw_text = true;
                }
                AgentEvent::AgentEnd => break,
                _ => {}
            }
        }
        assert!(saw_text, "the streamed text reached the subscriber");
    }

    #[tokio::test]
    async fn idle_cancel_does_not_poison_the_next_run() {
        let rec = Recorder::default();
        for text in ["one", "two"] {
            rec.push(vec![
                LlmStreamEvent::TextDelta(text.into()),
                LlmStreamEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: None,
                },
            ]);
        }
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let mut rx = handle.subscribe();

        handle.send(Request::Submit { text: "a".into() }).unwrap();
        wait_for_end(&mut rx).await;

        // No run in flight: this must be swallowed, not arm a kill for run two.
        handle.send(Request::Cancel).unwrap();
        handle.send(Request::Submit { text: "b".into() }).unwrap();

        let mut saw_two = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("an event")
                .expect("open");
            match event {
                AgentEvent::MessageUpdate { message } if message.as_text() == "two" => {
                    saw_two = true;
                }
                AgentEvent::AgentEnd => break,
                _ => {}
            }
        }
        assert!(saw_two, "the second run completed despite the idle cancel");
    }

    #[tokio::test]
    async fn cancel_aborts_an_in_flight_run() {
        // One delta, then the stream hangs: only a working cancel ends the run.
        let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
            let head = futures::stream::iter(vec![LlmStreamEvent::TextDelta("part".into())]);
            Box::pin(head.chain(futures::stream::pending())) as LlmStream
        });
        let handle = SessionActor::spawn(Agent::new(agent_config(stream_fn, vec![])));
        let mut rx = handle.subscribe();
        handle.send(Request::Submit { text: "hi".into() }).unwrap();

        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("an event")
                .expect("open");
            if matches!(event, AgentEvent::MessageUpdate { .. }) {
                handle.send(Request::Cancel).unwrap();
                break;
            }
        }
        // Without a live cancel this would time out on the pending stream.
        wait_for_end(&mut rx).await;
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    struct GateArgs {
        text: String,
    }

    /// Blocks until released, so the actor can be steered while a run is inside
    /// a tool call.
    struct GateTool {
        entered: mpsc::UnboundedSender<()>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl TypedTool for GateTool {
        type Args = GateArgs;
        fn name(&self) -> &str {
            "gate"
        }
        fn description(&self) -> &str {
            "blocks until released"
        }
        async fn execute(&self, args: GateArgs, _ctx: &ToolContext) -> ToolOutput {
            self.entered.send(()).ok();
            self.release.notified().await;
            ToolOutput {
                output: format!("released:{}", args.text),
                is_error: false,
                diff: None,
                path: None,
            }
        }
    }

    #[tokio::test]
    async fn config_request_during_a_run_is_deferred_to_after_it() {
        // Run one: turn 1 blocks in the gate, turn 2 finishes — both on the
        // run's own model. Run two is a fresh request and must see the swap.
        let rec = Recorder::default();
        rec.push(vec![
            LlmStreamEvent::ToolCall {
                id: "c1".into(),
                name: "gate".into(),
                arguments: json!({ "text": "hi" }),
            },
            LlmStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: None,
            },
        ]);
        for text in ["turn two", "run two"] {
            rec.push(vec![
                LlmStreamEvent::TextDelta(text.into()),
                LlmStreamEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: None,
                },
            ]);
        }

        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel::<()>();
        let release = Arc::new(tokio::sync::Notify::new());
        let gate = erased(GateTool {
            entered: entered_tx,
            release: release.clone(),
        });

        let handle =
            SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![gate])));
        let mut rx = handle.subscribe();
        handle.send(Request::Submit { text: "a".into() }).unwrap();

        entered_rx.recv().await.unwrap(); // run one is inside the tool
        handle
            .send(Request::SetModel { model: "m2".into() })
            .unwrap(); // must not take effect until run one is done
        release.notify_one();
        wait_for_end(&mut rx).await; // run one ends on m1

        handle.send(Request::Submit { text: "b".into() }).unwrap();
        wait_for_end(&mut rx).await; // run two

        assert_eq!(
            rec.models(),
            ["m1", "m1", "m2"],
            "the swap waited for the run, then applied to the next one"
        );
    }

    #[tokio::test]
    async fn ask_history_returns_the_conversation() {
        let rec = Recorder::default();
        rec.push(vec![
            LlmStreamEvent::TextDelta("hi back".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let mut rx = handle.subscribe();
        handle
            .send(Request::Submit {
                text: "hello".into(),
            })
            .unwrap();
        wait_for_end(&mut rx).await;

        let reply = handle.ask(Request::GetHistory).await.unwrap();
        let AgentEvent::History { messages } = reply else {
            panic!("expected a History reply, got {reply:?}");
        };
        assert!(
            messages.iter().any(|m| m.as_text() == "hello"),
            "the read-back carries the user turn: {messages:?}"
        );
    }

    #[tokio::test]
    async fn ask_set_model_replies_ack_and_takes_effect() {
        let rec = Recorder::default();
        rec.push(vec![
            LlmStreamEvent::TextDelta("x".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let mut rx = handle.subscribe();

        let reply = handle
            .ask(Request::SetModel { model: "m2".into() })
            .await
            .unwrap();
        assert!(matches!(reply, AgentEvent::Ack), "{reply:?}");

        handle.send(Request::Submit { text: "go".into() }).unwrap();
        wait_for_end(&mut rx).await;
        assert_eq!(rec.models(), ["m2"], "the swap applied to the next run");
    }

    #[tokio::test]
    async fn define_on_a_local_session_is_an_error() {
        let rec = Recorder::default();
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let reply = handle
            .ask(Request::Define {
                name: Some("w1".into()),
                model: None,
                role: None,
                tools: None,
                base_url: None,
                api_key: None,
            })
            .await
            .unwrap();
        assert!(
            matches!(&reply, AgentEvent::Error { message } if message.contains("served")),
            "{reply:?}"
        );
    }

    #[tokio::test]
    async fn ask_compact_with_nothing_to_do_replies_ack() {
        let handle = SessionActor::spawn(Agent::new(agent_config(
            fake_stream_fn(&Recorder::default()),
            vec![],
        )));
        let reply = handle
            .ask(Request::Compact { instructions: None })
            .await
            .unwrap();
        assert!(matches!(reply, AgentEvent::Ack), "{reply:?}");
    }

    #[tokio::test]
    async fn ask_submit_replies_stopped_with_the_reason() {
        let rec = Recorder::default();
        rec.push(vec![
            LlmStreamEvent::TextDelta("done".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let reply = handle
            .ask(Request::Submit { text: "hi".into() })
            .await
            .unwrap();
        assert!(
            matches!(
                reply,
                AgentEvent::Stopped {
                    stop_reason: StopReason::Stop
                }
            ),
            "{reply:?}"
        );
    }

    #[tokio::test]
    async fn notify_appends_without_starting_a_turn() {
        let rec = Recorder::default();
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let mut rx = handle.subscribe();

        let reply = handle
            .ask(Request::Notify {
                content: "ping".into(),
            })
            .await
            .unwrap();
        assert!(matches!(reply, AgentEvent::Ack), "{reply:?}");

        // The message was surfaced to subscribers...
        let surfaced = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("an event")
            .expect("open");
        assert!(
            matches!(&surfaced, AgentEvent::MessageReceived { content, .. } if content == "ping"),
            "{surfaced:?}"
        );

        // ...the conversation carries it...
        let AgentEvent::History { messages } = handle.ask(Request::GetHistory).await.unwrap() else {
            panic!("expected History");
        };
        assert!(
            messages
                .iter()
                .any(|m| m.as_text() == "[message from user]\nping"),
            "{messages:?}"
        );

        // ...and no turn ran (the loop was never entered).
        assert!(rec.calls().is_empty(), "notify must not run the loop");
    }

    #[tokio::test]
    async fn wake_starts_a_run_when_idle() {
        let rec = Recorder::default();
        rec.push(vec![
            LlmStreamEvent::TextDelta("acted".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let mut rx = handle.subscribe();

        let reply = handle
            .ask(Request::Wake {
                content: "do the task".into(),
            })
            .await
            .unwrap();
        assert!(
            matches!(
                reply,
                AgentEvent::Stopped {
                    stop_reason: StopReason::Stop
                }
            ),
            "{reply:?}"
        );
        wait_for_end(&mut rx).await;

        let calls = rec.calls();
        assert_eq!(calls.len(), 1, "Wake ran the loop once");
        assert!(
            calls[0]
                .iter()
                .any(|m| m.as_text() == "[message from user]\ndo the task"),
            "the tagged task is the prompt: {:?}",
            calls[0]
        );
    }

    struct InboundPolicy;

    #[async_trait::async_trait]
    impl crate::hooks::Hooks for InboundPolicy {
        async fn before_inbound(
            &self,
            _from: Option<&SessionId>,
            request: &mut Request,
        ) -> Option<String> {
            let Request::Notify { content } = request else {
                return None;
            };
            if content.starts_with("drop:") {
                return Some("policy".into());
            }
            *content = format!("{content}!");
            None
        }
    }

    #[tokio::test]
    async fn before_inbound_drops_and_rewrites_a_notify() {
        let rec = Recorder::default();
        let mut cfg = agent_config(fake_stream_fn(&rec), vec![]);
        cfg.hooks = HooksSet::one(Arc::new(InboundPolicy));
        let handle = SessionActor::spawn(Agent::new(cfg));

        // Dropped: an `ask` is answered with an Error, and nothing lands.
        let reply = handle
            .ask(Request::Notify {
                content: "drop: me".into(),
            })
            .await
            .unwrap();
        assert!(
            matches!(&reply, AgentEvent::Error { message } if message.contains("policy")),
            "{reply:?}"
        );

        // Rewritten: the hook's edit is what is recorded.
        let reply = handle
            .ask(Request::Notify {
                content: "keep me".into(),
            })
            .await
            .unwrap();
        assert!(matches!(reply, AgentEvent::Ack), "{reply:?}");

        let AgentEvent::History { messages } = handle.ask(Request::GetHistory).await.unwrap() else {
            panic!("expected History");
        };
        let texts: Vec<String> = messages.iter().map(|m| m.as_text()).collect();
        assert!(
            !texts.iter().any(|t| t == "drop: me"),
            "the dropped message never landed: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "[message from user]\nkeep me!"),
            "the rewrite landed: {texts:?}"
        );
    }

    #[tokio::test]
    async fn interrupt_reaches_the_next_turn_while_a_tool_is_blocked() {
        let rec = Recorder::default();
        rec.push(vec![
            LlmStreamEvent::ToolCall {
                id: "c1".into(),
                name: "gate".into(),
                arguments: json!({ "text": "hi" }),
            },
            LlmStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: None,
            },
        ]);
        rec.push(vec![
            LlmStreamEvent::TextDelta("after interrupt".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);

        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel::<()>();
        let release = Arc::new(tokio::sync::Notify::new());
        let gate = erased(GateTool {
            entered: entered_tx,
            release: release.clone(),
        });

        let handle =
            SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![gate])));
        let mut rx = handle.subscribe();
        handle.send(Request::Submit { text: "hi".into() }).unwrap();

        entered_rx.recv().await.unwrap(); // the tool is now blocked
        handle
            .send(Request::Interrupt {
                content: "mid-run interrupt".into(),
            })
            .unwrap();
        release.notify_one();

        wait_for_end(&mut rx).await;

        let calls = rec.calls();
        assert_eq!(calls.len(), 2, "two stream calls: the tool turn and the next");
        assert!(
            calls[1]
                .iter()
                .any(|m| matches!(m, AgentMessage::User { .. })
                    && m.as_text() == "[message from user]\nmid-run interrupt"),
            "the interrupt drained into the next turn: {:?}",
            calls[1]
        );
    }

    #[tokio::test]
    async fn send_from_attributes_the_sender() {
        let rec = Recorder::default();
        let handle = SessionActor::spawn(Agent::new(agent_config(fake_stream_fn(&rec), vec![])));
        let mut rx = handle.subscribe();

        handle
            .send_from(
                SessionId::agent("b"),
                Request::Notify {
                    content: "ping".into(),
                },
            )
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("an event")
            .expect("open");
        assert!(
            matches!(&event, AgentEvent::MessageReceived { from, content }
                if from.as_str() == "agent:b" && content == "ping"),
            "{event:?}"
        );
    }

    struct SenderPolicy;

    #[async_trait::async_trait]
    impl crate::hooks::Hooks for SenderPolicy {
        async fn before_inbound(
            &self,
            from: Option<&SessionId>,
            _request: &mut Request,
        ) -> Option<String> {
            (from.map(SessionId::as_str) == Some("agent:evil")).then(|| "blocked sender".into())
        }
    }

    #[tokio::test]
    async fn before_inbound_sees_the_sender() {
        let rec = Recorder::default();
        let mut cfg = agent_config(fake_stream_fn(&rec), vec![]);
        cfg.hooks = HooksSet::one(Arc::new(SenderPolicy));
        let handle = SessionActor::spawn(Agent::new(cfg));

        // A hostile peer is dropped...
        let reply = handle
            .ask_from(
                SessionId::agent("evil"),
                Request::Notify {
                    content: "let me in".into(),
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(&reply, AgentEvent::Error { message } if message.contains("blocked sender")),
            "{reply:?}"
        );

        // ...a friendly one is not.
        let reply = handle
            .ask_from(
                SessionId::agent("friend"),
                Request::Notify {
                    content: "hello".into(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(reply, AgentEvent::Ack), "{reply:?}");

        let AgentEvent::History { messages } = handle.ask(Request::GetHistory).await.unwrap() else {
            panic!("expected History");
        };
        let texts: Vec<String> = messages.iter().map(|m| m.as_text()).collect();
        assert!(
            texts
                .iter()
                .any(|t| t == "[message from agent:friend]\nhello"),
            "{texts:?}"
        );
        assert!(!texts.iter().any(|t| t.contains("let me in")), "{texts:?}");
    }
}
