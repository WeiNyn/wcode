//! Integration tests for the agent loop kernel (`loop_::run_loop` + `hooks`).
//!
//! The LLM seam is a fake `StreamFn` scripting `LlmStreamEvent`s from a
//! shared queue (one `Vec<LlmStreamEvent>` per stream call, popped in order)
//! and recording what it received per call: `(ctx snapshot, system, tools)`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures::StreamExt as _;
use serde::Deserialize;
use tokio::sync::mpsc;

use wcode_harness::event::{AgentEvent, LlmStreamEvent};
use wcode_harness::hooks::{DefaultHooks, Hooks, ToolCall as HookToolCall};
use wcode_harness::loop_::{LoopConfig, LoopError, run_loop};
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
use wcode_harness::tool::{Tool, ToolContext, ToolOutput, TypedTool, erased};

// ---------------------------------------------------------------------------
// Fake StreamFn: scripted events + call recording
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StreamCall {
    ctx: Vec<AgentMessage>,
    system: String,
    tools: Vec<String>,
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
              tools: &[rig::completion::ToolDefinition],
              _opts: &LlmOpts| {
            rec.calls.lock().unwrap().push(StreamCall {
                ctx: ctx.to_vec(),
                system: system.to_string(),
                tools: tools.iter().map(|t| t.name.clone()).collect(),
            });
            let events = rec.script.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(futures::stream::iter(events)) as LlmStream
        },
    )
}

// ---------------------------------------------------------------------------
// Recording tool
// ---------------------------------------------------------------------------

#[derive(Deserialize, schemars::JsonSchema)]
struct EchoArgs {
    text: String,
}

struct RecordingTool {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl TypedTool for RecordingTool {
    type Args = EchoArgs;
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes text back"
    }
    async fn execute(&self, args: EchoArgs, _ctx: &ToolContext) -> ToolOutput {
        self.seen.lock().unwrap().push(args.text);
        ToolOutput {
            output: format!("echo:{}", self.seen.lock().unwrap().last().unwrap()),
            is_error: false,
            details: None,
        }
    }
}

fn echo_tool() -> (Tool, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    (erased(RecordingTool { seen: seen.clone() }), seen)
}

// ---------------------------------------------------------------------------
// Config + run helpers
// ---------------------------------------------------------------------------

struct TestSetup {
    cfg: LoopConfig,
    steer_tx: mpsc::UnboundedSender<AgentMessage>,
    follow_tx: mpsc::UnboundedSender<AgentMessage>,
}

fn setup(stream_fn: StreamFn, tools: Vec<Tool>, hooks: Arc<dyn Hooks>) -> TestSetup {
    let (steer_tx, steering) = mpsc::unbounded_channel();
    let (follow_tx, follow_ups) = mpsc::unbounded_channel();
    TestSetup {
        cfg: LoopConfig {
            system: "sys".into(),
            tools,
            llm: LlmOpts {
                model: "m1".into(),
                ..LlmOpts::default()
            },
            stream_fn,
            hooks,
            steering,
            follow_ups,
            cancel: tokio_util::sync::CancellationToken::new(),
        },
        steer_tx,
        follow_tx,
    }
}

async fn run(
    cfg: LoopConfig,
    ctx: &mut Vec<AgentMessage>,
) -> (Result<StopReason, LoopError>, Vec<AgentEvent>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let res = run_loop(ctx, cfg, tx).await;
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    (res, events)
}

fn tag(e: &AgentEvent) -> &'static str {
    match e {
        AgentEvent::AgentStart => "agent_start",
        AgentEvent::TurnStart => "turn_start",
        AgentEvent::MessageStart { .. } => "message_start",
        AgentEvent::MessageUpdate { .. } => "message_update",
        AgentEvent::MessageEnd { .. } => "message_end",
        AgentEvent::ToolExecutionStart { .. } => "tool_execution_start",
        AgentEvent::ToolExecutionUpdate { .. } => "tool_execution_update",
        AgentEvent::ToolExecutionEnd { .. } => "tool_execution_end",
        AgentEvent::TurnEnd { .. } => "turn_end",
        AgentEvent::AgentEnd => "agent_end",
    }
}

fn tags(events: &[AgentEvent]) -> Vec<&'static str> {
    events.iter().map(tag).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_text_turn() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("hel".into()),
        LlmStreamEvent::TextDelta("lo".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], Arc::new(DefaultHooks));

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(
        tags(&events),
        vec![
            "agent_start",
            "turn_start",
            "message_start",
            "message_update",
            "message_update",
            "message_end",
            "turn_end",
            "agent_end",
        ]
    );
    assert_eq!(ctx.len(), 2);
    assert!(matches!(ctx[0], AgentMessage::User { .. }));
    assert!(matches!(
        &ctx[1],
        AgentMessage::Assistant {
            stop_reason: StopReason::Stop,
            ..
        }
    ));
    assert_eq!(ctx[1].as_text(), "hello");
    // The stream call received the system prompt and no tool definitions.
    let calls = rec.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].system, "sys");
    assert!(calls[0].tools.is_empty());
}

#[tokio::test]
async fn tool_call_roundtrip() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCallStart {
            id: "c1".into(),
            name: "echo".into(),
        },
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "hello" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("done".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let (tool, seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], Arc::new(DefaultHooks));

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    // Tool received typed args.
    assert_eq!(*seen.lock().unwrap(), vec!["hello".to_string()]);
    // ctx: User, Assistant(ToolUse), ToolResult, Assistant(Stop)
    assert_eq!(ctx.len(), 4);
    assert!(matches!(
        &ctx[2],
        AgentMessage::ToolResult { tool_call_id, name, output, is_error: false }
            if tool_call_id == "c1" && name == "echo" && output == "echo:hello"
    ));
    // Second stream call saw the tool result in context.
    let calls = rec.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].tools, vec!["echo".to_string()]);
    assert!(matches!(
        &calls[1].ctx[2],
        AgentMessage::ToolResult { output, .. } if output == "echo:hello"
    ));
}

#[tokio::test]
async fn invalid_tool_args_yield_error_tool_result_and_loop_continues() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "wrong": 42 }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("recovered".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let (tool, seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], Arc::new(DefaultHooks));

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert!(seen.lock().unwrap().is_empty(), "tool body must not run");
    assert!(matches!(
        &ctx[2],
        AgentMessage::ToolResult { output, is_error: true, .. }
            if output.starts_with("invalid arguments for tool `echo`")
    ));
    assert_eq!(
        rec.calls().len(),
        2,
        "loop continues to the next stream call"
    );
    assert_eq!(ctx.len(), 4);
}

struct BlockingHooks;

#[async_trait::async_trait]
impl Hooks for BlockingHooks {
    async fn before_tool_call(&self, _call: &HookToolCall) -> Option<String> {
        Some("nope".into())
    }
}

#[tokio::test]
async fn before_tool_call_blocks() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "hello" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("ok".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let (tool, seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], Arc::new(BlockingHooks));

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert!(
        seen.lock().unwrap().is_empty(),
        "blocked tool must not execute"
    );
    let end = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolExecutionEnd {
                output, is_error, ..
            } => Some((output, *is_error)),
            _ => None,
        })
        .expect("ToolExecutionEnd emitted");
    assert!(end.1, "blocked output must be an error");
    assert!(end.0.contains("nope"), "unexpected output: {}", end.0);
    assert!(matches!(
        &ctx[2],
        AgentMessage::ToolResult { output, is_error: true, .. } if output == "blocked: nope"
    ));
    assert_eq!(rec.calls().len(), 2, "loop continues after a blocked call");
}

struct PatchingHooks;

#[async_trait::async_trait]
impl Hooks for PatchingHooks {
    async fn after_tool_call(&self, _call: &HookToolCall, out: &mut ToolOutput) {
        out.output = format!("patched:{}", out.output);
    }
}

#[tokio::test]
async fn after_tool_call_patches() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "hello" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ]);

    let (tool, _seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], Arc::new(PatchingHooks));

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let end = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolExecutionEnd { output, .. } => Some(output.clone()),
            _ => None,
        })
        .expect("ToolExecutionEnd emitted");
    assert_eq!(end, "patched:echo:hello");
    assert!(matches!(
        &ctx[2],
        AgentMessage::ToolResult { output, .. } if output == "patched:echo:hello"
    ));
}

#[tokio::test]
async fn abort_mid_stream() {
    // One delta, then the stream hangs forever; the watcher task cancels the
    // token as soon as the delta is rendered.
    let cancel = tokio_util::sync::CancellationToken::new();
    let stream_fn: StreamFn = Arc::new(move |_ctx, _sys, _tools, _opts| {
        let delta = futures::stream::iter(vec![LlmStreamEvent::TextDelta("part".into())]);
        Box::pin(delta.chain(futures::stream::pending())) as LlmStream
    });
    let TestSetup { cfg, .. } = setup(stream_fn, vec![], Arc::new(DefaultHooks));
    let cfg = LoopConfig { cancel, ..cfg };

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (tx, mut rx) = mpsc::unbounded_channel();
    let watcher_cancel = cfg.cancel.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AgentEvent::MessageUpdate { .. }) {
                watcher_cancel.cancel();
                break;
            }
        }
    });
    let res = run_loop(&mut ctx, cfg, tx).await;

    assert_eq!(res.unwrap(), StopReason::Aborted);
    assert_eq!(ctx.len(), 2);
    assert!(matches!(
        &ctx[1],
        AgentMessage::Assistant { stop_reason: StopReason::Aborted, .. }
            if ctx[1].as_text() == "part"
    ));
}

struct StopAfterTurnHooks;

#[async_trait::async_trait]
impl Hooks for StopAfterTurnHooks {
    async fn should_stop_after_turn(&self, _ctx: &[AgentMessage]) -> bool {
        true
    }
}

#[tokio::test]
async fn should_stop_after_turn() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "hello" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ]);

    let (tool, _seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![tool],
        Arc::new(StopAfterTurnHooks),
    );

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(rec.calls().len(), 1, "hook stops the run after turn 1");
    assert_eq!(ctx.len(), 3);
    assert!(matches!(&ctx[2], AgentMessage::ToolResult { .. }));
}

#[tokio::test]
async fn follow_up_runs_next_turn() {
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

    let TestSetup { cfg, follow_tx, .. } =
        setup(fake_stream_fn(&rec), vec![], Arc::new(DefaultHooks));
    follow_tx.send(AgentMessage::user_text("follow")).unwrap();

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(
        rec.calls().len(),
        2,
        "follow-up triggers a second outer turn"
    );
    assert_eq!(ctx.len(), 4);
    assert!(matches!(&ctx[2], AgentMessage::User { .. } if ctx[2].as_text() == "follow"));
    assert!(matches!(&ctx[3], AgentMessage::Assistant { .. } if ctx[3].as_text() == "b"));
    // Follow-up user message announced, then a second full turn.
    let t = tags(&events);
    assert!(t.windows(2).any(|w| w == ["message_start", "message_end"]));
    assert_eq!(t.iter().filter(|&&x| x == "turn_start").count(), 2);
    assert_eq!(*t.last().unwrap(), "agent_end");
}

#[tokio::test]
async fn steering_is_drained_before_streaming() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("a".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let TestSetup { cfg, steer_tx, .. } =
        setup(fake_stream_fn(&rec), vec![], Arc::new(DefaultHooks));
    steer_tx.send(AgentMessage::user_text("steer")).unwrap();

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let calls = rec.calls();
    assert_eq!(calls.len(), 1);
    // The steering message reached context before the stream call.
    assert_eq!(calls[0].ctx.len(), 2);
    assert_eq!(calls[0].ctx[1].as_text(), "steer");
    assert_eq!(ctx.len(), 3);
    assert!(matches!(&ctx[1], AgentMessage::User { .. } if ctx[1].as_text() == "steer"));
}

/// Smoke: the loop runs a full tool round with pure-default hooks.
#[tokio::test]
async fn default_hooks_smoke() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "hi" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("bye".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let (tool, _seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], Arc::new(DefaultHooks));

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let t = tags(&events);
    assert!(t.contains(&"tool_execution_start"));
    assert!(t.contains(&"tool_execution_end"));
    assert_eq!(*t.last().unwrap(), "agent_end");
    assert_eq!(ctx.len(), 4);
}
