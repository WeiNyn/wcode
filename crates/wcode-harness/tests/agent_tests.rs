//! Integration tests for the stateful `Agent` wrapper.
//!
//! Reuses the fake-stream harness from `loop_tests.rs` (copied: test files
//! don't share code without a common module, ~60 lines duplication is fine).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::event::{AgentEvent, LlmStreamEvent};
use wcode_harness::hooks::DefaultHooks;
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::session::Session;
use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool, erased};

// ---------------------------------------------------------------------------
// Fake StreamFn: scripted events + call recording (copy of loop_tests harness)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StreamCall {
    ctx: Vec<AgentMessage>,
    system: String,
    model: String,
}

#[derive(Clone, Default)]
struct Recorder {
    script: Arc<Mutex<VecDeque<Vec<LlmStreamEvent>>>>,
    calls: Arc<Mutex<Vec<StreamCall>>>,
}

impl Recorder {
    fn push(&self, events: Vec<LlmStreamEvent>) {
        self.script.lock().unwrap().push_back(events);
    }

    fn calls(&self) -> Vec<StreamCall> {
        self.calls.lock().unwrap().clone()
    }
}

fn fake_stream_fn(rec: &Recorder) -> StreamFn {
    let rec = rec.clone();
    Arc::new(
        move |ctx: &[AgentMessage],
              system: &str,
              _tools: &[rig::completion::ToolDefinition],
              _opts: &LlmOpts| {
            rec.calls.lock().unwrap().push(StreamCall {
                ctx: ctx.to_vec(),
                system: system.to_string(),
                model: _opts.model.clone(),
            });
            let events = rec.script.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(futures::stream::iter(events)) as LlmStream
        },
    )
}

// ---------------------------------------------------------------------------
// Channel-waiting (blocked) tool
// ---------------------------------------------------------------------------

#[derive(Deserialize, schemars::JsonSchema)]
struct GateArgs {
    text: String,
}

/// Blocks on a Notify until released; the watcher steers through the agent's
/// steering channel while the tool is blocked.
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
            output: format!("ran:{}", args.text),
            is_error: false,
            details: None,
        }
    }
}

fn agent_config(
    stream_fn: StreamFn,
    tools: Vec<wcode_harness::tool::Tool>,
    session: Option<Session>,
) -> AgentConfig {
    AgentConfig {
        system: "sys".into(),
        tools,
        llm: LlmOpts {
            model: "m1".into(),
            ..LlmOpts::default()
        },
        stream_fn,
        hooks: Arc::new(DefaultHooks),
        session,
        context: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn steer_during_blocked_tool_reaches_next_turn_stream() {
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
        LlmStreamEvent::TextDelta("after steer".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    // The agent owns the steering channel and run() holds &mut, so the test
    // steers through a sender clone while the tool is blocked.
    let slot: Arc<OnceLock<mpsc::UnboundedSender<AgentMessage>>> = Arc::new(OnceLock::new());
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel::<()>();
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = erased(GateTool {
        entered: entered_tx,
        release: release.clone(),
    });

    let mut agent = Agent::new(agent_config(fake_stream_fn(&rec), vec![gate], None));
    slot.set(agent.steer_sender()).unwrap();
    let steer = slot.get().unwrap().clone();

    tokio::spawn(async move {
        entered_rx.recv().await.unwrap();
        steer
            .send(AgentMessage::user_text("mid-run steer"))
            .unwrap();
        release.notify_one();
    });

    let (tx, _rx) = mpsc::unbounded_channel();
    let res = agent.run("hi", tx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let calls = rec.calls();
    assert_eq!(calls.len(), 2);
    // Turn-2 snapshot: User, Assistant(ToolUse), ToolResult, then the steer
    // drained at turn start before the next assistant streams.
    let ctx2 = &calls[1].ctx;
    assert_eq!(ctx2.len(), 4);
    assert!(
        matches!(&ctx2[3], AgentMessage::User { .. } if ctx2[3].as_text() == "mid-run steer"),
        "steer must appear before the next turn's stream call, got: {:?}",
        ctx2
    );
}

#[tokio::test]
async fn cancel_returns_aborted() {
    // One delta, then the stream hangs forever; the watcher cancels via the
    // agent's token as soon as the delta is rendered.
    let stream_fn: StreamFn = Arc::new(move |_ctx, _sys, _tools, _opts| {
        let delta = futures::stream::iter(vec![LlmStreamEvent::TextDelta("part".into())]);
        Box::pin(delta.chain(futures::stream::pending())) as LlmStream
    });
    let mut agent = Agent::new(agent_config(stream_fn, vec![], None));

    let token = agent.cancel_token();
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AgentEvent::MessageUpdate { .. }) {
                token.cancel();
                break;
            }
        }
    });

    let res = agent.run("hi", tx).await;

    assert_eq!(res.unwrap(), StopReason::Aborted);
    assert_eq!(agent.messages().len(), 2);
    assert!(matches!(
        &agent.messages()[1],
        AgentMessage::Assistant { stop_reason: StopReason::Aborted, .. }
            if agent.messages()[1].as_text() == "part"
    ));
}

#[tokio::test]
async fn session_file_contains_all_messages_after_run() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("hello".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let dir = tempfile::tempdir().unwrap();
    let session = Session::create(dir.path()).unwrap();
    let path = session.path().unwrap().to_path_buf();
    let mut agent = Agent::new(agent_config(fake_stream_fn(&rec), vec![], Some(session)));

    let (tx, _rx) = mpsc::unbounded_channel();
    let res = agent.run("hi", tx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(agent.session_path(), Some(path.as_path()));
    assert_eq!(agent.messages().len(), 2);
    assert_eq!(rec.calls()[0].system, "sys");

    let reopened = Session::open(&path).unwrap();
    let msgs = reopened.messages();
    assert_eq!(msgs.len(), 2);
    assert!(matches!(&msgs[0], AgentMessage::User { .. } if msgs[0].as_text() == "hi"));
    assert!(matches!(&msgs[1], AgentMessage::Assistant { .. } if msgs[1].as_text() == "hello"));
}

#[tokio::test]
async fn steer_between_runs_is_queued_for_next_run() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("one".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("two".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let mut agent = Agent::new(agent_config(fake_stream_fn(&rec), vec![], None));

    let (tx1, _rx1) = mpsc::unbounded_channel();
    agent.run("hi", tx1).await.unwrap();
    assert_eq!(rec.calls().len(), 1);

    agent.steer(AgentMessage::user_text("queued"));

    let (tx2, _rx2) = mpsc::unbounded_channel();
    let res = agent.run("again", tx2).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let calls = rec.calls();
    assert_eq!(calls.len(), 2);
    // Second run's turn-1 snapshot: User hi, Assistant one, User again,
    // then the steer queued between runs.
    let ctx2 = &calls[1].ctx;
    assert_eq!(ctx2.len(), 4);
    assert!(
        matches!(&ctx2[3], AgentMessage::User { .. } if ctx2[3].as_text() == "queued"),
        "steer queued between runs must reach the next run, got: {:?}",
        ctx2
    );
    assert_eq!(agent.messages().len(), 5);
}

/// `/resume` backbone: AgentConfig.context seeds the conversation so the
/// first stream call already sees the restored history.
#[tokio::test]
async fn seeded_context_reaches_first_stream_call() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("ok".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let mut cfg = agent_config(fake_stream_fn(&rec), vec![], None);
    cfg.context = vec![
        AgentMessage::user_text("old question"),
        AgentMessage::Assistant {
            content: vec![wcode_harness::message::ContentBlock::Text {
                text: "old answer".into(),
            }],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        },
    ];
    let mut agent = Agent::new(cfg);

    let (tx, _rx) = mpsc::unbounded_channel();
    agent.run("new question", tx).await.unwrap();

    let ctx = &rec.calls()[0].ctx;
    assert_eq!(ctx.len(), 3);
    assert_eq!(ctx[0].as_text(), "old question");
    assert_eq!(ctx[1].as_text(), "old answer");
    assert_eq!(ctx[2].as_text(), "new question");
}

/// `/model` backbone: next stream call uses the new model and a ModelChange
/// entry lands in the open session.
#[tokio::test]
async fn set_model_swaps_llm_and_logs_session_change() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("a".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("b".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let dir = tempfile::tempdir().unwrap();
    let session = Session::create(dir.path()).unwrap();
    let path = session.path().unwrap().to_path_buf();
    let mut agent = Agent::new(agent_config(fake_stream_fn(&rec), vec![], Some(session)));

    let (tx, _rx) = mpsc::unbounded_channel();
    agent.run("hi", tx).await.unwrap();
    assert_eq!(rec.calls()[0].model, "m1");

    agent.set_model("m2".into()).unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    agent.run("again", tx).await.unwrap();

    assert_eq!(rec.calls()[1].model, "m2");
    let reopened = Session::open(&path).unwrap();
    assert_eq!(reopened.model().as_deref(), Some("m2"));
}
