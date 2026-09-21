//! Integration tests for the agent loop kernel (`loop_::run_loop` + `hooks`).
//!
//! The LLM seam is a fake `StreamFn` scripting `LlmStreamEvent`s from a
//! shared queue (one `Vec<LlmStreamEvent>` per stream call, popped in order)
//! and recording what it received per call: `(ctx snapshot, system, tools)`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::StreamExt as _;
use serde::Deserialize;
use tokio::sync::mpsc;

use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::event::{AgentEvent, LlmStreamEvent};
use wcode_harness::hooks::{Hooks, HooksSet, ToolCall as HookToolCall};
use wcode_harness::loop_::{LoopConfig, LoopError, run_loop};
use wcode_harness::message::{AgentMessage, ContentBlock, StopReason};
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
            diff: None,
            path: None,
        }
    }
}

fn echo_tool() -> (Tool, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    (erased(RecordingTool { seen: seen.clone() }), seen)
}

// ---------------------------------------------------------------------------
#[derive(Deserialize, schemars::JsonSchema)]
struct NoArgs {}

struct CwdProbe {
    seen: Arc<Mutex<std::path::PathBuf>>,
}

#[async_trait::async_trait]
impl TypedTool for CwdProbe {
    type Args = NoArgs;
    fn name(&self) -> &str {
        "cwd"
    }
    fn description(&self) -> &str {
        "records the tool working_dir"
    }
    async fn execute(&self, _args: NoArgs, ctx: &ToolContext) -> ToolOutput {
        *self.seen.lock().unwrap() = ctx.working_dir.clone();
        ToolOutput {
            output: "ok".into(),
            is_error: false,
            diff: None,
            path: None,
        }
    }
}

/// Finding #2: tools execute against `LoopConfig.working_dir`, not the
/// process cwd.
#[tokio::test]
async fn tool_working_dir_comes_from_config() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "cwd".into(),
            arguments: serde_json::json!({}),
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

    let seen = Arc::new(Mutex::new(std::env::current_dir().unwrap_or_default()));
    let probe = erased(CwdProbe { seen: seen.clone() });
    let mut ts = setup(fake_stream_fn(&rec), vec![probe], HooksSet::default());

    let dir = tempfile::tempdir().unwrap();
    let wd = dir.path().to_path_buf();
    ts.cfg.working_dir = wd.clone();

    let mut ctx = Vec::new();
    let (res, _events) = run(ts.cfg, &mut ctx).await;
    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(*seen.lock().unwrap(), wd);
}

/// The per-run turn cap stops a runaway tool loop: a model that re-issues the
/// same tool call forever must end after `max_turns` turns (not run until
/// cancelled), on a completed turn boundary, with `StopReason::MaxTurns`.
#[tokio::test]
async fn max_turns_caps_runaway_tool_loop() {
    let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
        Box::pin(futures::stream::iter(vec![
            LlmStreamEvent::ToolCall {
                id: "c1".into(),
                name: "echo".into(),
                arguments: serde_json::json!({ "text": "x" }),
            },
            LlmStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: None,
            },
        ])) as LlmStream
    });
    let (tool, seen) = echo_tool();
    let TestSetup { mut cfg, .. } = setup(stream_fn, vec![tool], HooksSet::default());
    cfg.max_turns = 3;

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::MaxTurns);
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "exactly max_turns tool turns ran"
    );
    // Complete history: the user message plus one assistant + one ToolResult
    // per turn — the cap never leaves a ToolCall without its ToolResult.
    assert_eq!(ctx.len(), 1 + 3 * 2);
    assert_eq!(*tags(&events).last().unwrap(), "agent_end");
}

// Config + run helpers
// ---------------------------------------------------------------------------

struct TestSetup {
    cfg: LoopConfig<'static>,
    steer_tx: mpsc::UnboundedSender<AgentMessage>,
    follow_tx: mpsc::UnboundedSender<AgentMessage>,
}

fn setup(stream_fn: StreamFn, tools: Vec<Tool>, hooks: HooksSet) -> TestSetup {
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
            working_dir: std::path::PathBuf::from("."),
            session: None,
            max_turns: wcode_harness::loop_::DEFAULT_MAX_TURNS,
            parallel: true,
            compaction: CompactionPolicy::default(),
        },
        steer_tx,
        follow_tx,
    }
}

async fn run(
    cfg: LoopConfig<'_>,
    ctx: &mut Vec<AgentMessage>,
) -> (Result<StopReason, LoopError>, Vec<AgentEvent>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let res = run_loop(ctx, cfg, tx).await.map(|r| r.stop_reason);
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
        AgentEvent::Error { .. } => "error",
        AgentEvent::Compaction { .. } => "compaction",
        AgentEvent::Retrying { .. } => "retrying",
        AgentEvent::MessageReceived { .. } => "message_received",
        AgentEvent::Ack => "ack",
        AgentEvent::Stopped { .. } => "stopped",
        AgentEvent::History { .. } => "history",
        AgentEvent::Sessions { .. } => "sessions",
        AgentEvent::Spawned { .. } => "spawned",
        AgentEvent::Todo { .. } => "todo",
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
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());

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
async fn assistant_message_carries_the_model() {
    // The loop stamps the model that produced each assistant message, so a
    // session keeps provenance across a mid-conversation model swap.
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("hi".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("q")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert!(
        matches!(&ctx[1], AgentMessage::Assistant { model: Some(m), .. } if m == "m1"),
        "assistant must carry the run's model (m1): {:?}",
        ctx[1]
    );
}

#[tokio::test]
async fn tool_use_without_streamed_call_feeds_back_and_continues() {
    // Finding #3: rig only emits the complete ToolCall on ToolInputEnd, so a
    // stream that finishes with tool_use but never streams a call has lost the
    // model's requested action. That is a feedable failure now: it surfaces as
    // an error notice, is written back into ctx, and the loop runs another
    // turn so the model can re-issue the call.
    let rec = Recorder::default();
    rec.push(vec![LlmStreamEvent::Done {
        stop_reason: StopReason::ToolUse,
        usage: None,
    }]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("ok".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(rec.calls().len(), 2, "loop continues to a second stream call");
    // User, fed-back failure notice, assistant answer.
    assert_eq!(ctx.len(), 3);
    assert_eq!(
        ctx[1].as_text(),
        "your previous response failed: model requested tool use but no tool call was streamed"
    );
    let notice_seen = rec.calls()[1]
        .ctx
        .iter()
        .any(|m| m.as_text().contains("model requested tool use but no tool call was streamed"));
    assert!(notice_seen, "second stream call receives the fed-back notice");
    assert_eq!(*tags(&events).last().unwrap(), "agent_end");
}

#[tokio::test]
async fn transient_stream_error_feeds_back_and_loop_continues() {
    // A transient stream error no longer kills the run: turn 1 fails with a
    // non-fatal Error, the failure notice lands in ctx, and a second stream
    // call happens and completes normally.
    let rec = Recorder::default();
    rec.push(vec![LlmStreamEvent::Error {
        message: "boom".into(),
        fatal: false,
    }]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("recovered".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(rec.calls().len(), 2, "a second stream call happens after the turn-1 error");
    // User, fed-back failure notice, assistant answer.
    assert_eq!(ctx.len(), 3);
    assert_eq!(ctx[1].as_text(), "your previous response failed: boom");
    let notice_seen = rec.calls()[1]
        .ctx
        .iter()
        .any(|m| m.as_text() == "your previous response failed: boom");
    assert!(notice_seen, "second stream call receives the fed-back notice");
    assert_eq!(*tags(&events).last().unwrap(), "agent_end");
}

#[tokio::test]
async fn hard_fatal_stream_error_ends_run() {
    // Hard-fatal classes (bad request/auth/schema) cannot be fixed by retrying
    // or re-feeding, so the run still ends with StopReason::Error — no second
    // stream call is made.
    let rec = Recorder::default();
    rec.push(vec![LlmStreamEvent::Error {
        message: "invalid_request_error: bad schema".into(),
        fatal: true,
    }]);
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Error);
    assert_eq!(rec.calls().len(), 1, "no second turn after a fatal error");
    assert_eq!(ctx.len(), 1, "no empty assistant may be recorded");
    assert_eq!(*tags(&events).last().unwrap(), "agent_end");
}

#[tokio::test]
async fn consecutive_stream_error_turns_are_capped() {
    // A provider that keeps failing on every turn must not loop forever: after
    // DEFAULT_MAX_STREAM_ERROR_TURNS consecutive fed-back errors, the run ends
    // with StopReason::Error exactly as it did before.
    let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
        Box::pin(futures::stream::iter(vec![LlmStreamEvent::Error {
            message: "boom".into(),
            fatal: false,
        }])) as LlmStream
    });
    let TestSetup { cfg, .. } = setup(stream_fn, vec![], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Error);
    // One fed-back notice per capped turn; the turn past the cap ends the run
    // without pushing another.
    assert_eq!(
        ctx.len(),
        1 + wcode_harness::loop_::DEFAULT_MAX_STREAM_ERROR_TURNS
    );
    assert!(
        ctx.iter().all(|m| matches!(m, AgentMessage::User { .. })),
        "no assistant content was produced: {ctx:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_streamfn_that_never_yields_trips_the_backstop() {
    // #23: a `StreamFn` that never yields (a pathological custom impl, or a
    // stalled adapter) must not wedge the run. The kernel's idle arm surfaces
    // an `AgentEvent::Error`, feeds it back as a corrective turn, and ends the
    // run after the cap — never hangs.
    let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
        Box::pin(futures::stream::pending::<LlmStreamEvent>()) as LlmStream
    });
    let TestSetup { mut cfg, .. } = setup(stream_fn, vec![], HooksSet::default());
    cfg.llm.retry.idle = std::time::Duration::from_millis(100);

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Error);
    assert!(
        tags(&events).contains(&"error"),
        "the stall surfaced an error: {:?}",
        tags(&events)
    );
    assert_eq!(
        ctx.len(),
        1 + wcode_harness::loop_::DEFAULT_MAX_STREAM_ERROR_TURNS,
        "the stall is fed back, capped like any stream error"
    );
    assert!(
        ctx.iter().all(|m| matches!(m, AgentMessage::User { .. })),
        "no assistant content was produced: {ctx:?}"
    );
}

#[tokio::test]
async fn complete_thinking_block_replaces_accumulated_deltas() {
    // A provider that streams reasoning deltas and then restates the full
    // block must not leave duplicated thinking in the conversation context.
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ThinkingDelta("partial ".into()),
        LlmStreamEvent::ThinkingReplace("full thought".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());
    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(ctx.len(), 2);
    match &ctx[1] {
        AgentMessage::Assistant { content, .. } => {
            let thinking_blocks = content
                .iter()
                .filter(|b| matches!(b, ContentBlock::Thinking { .. }))
                .count();
            assert_eq!(
                thinking_blocks, 1,
                "replacement must not leave the delta block behind"
            );
            let thinking: String = content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Thinking { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(thinking, "full thought");
        }
        _ => panic!("expected assistant message"),
    }
}

#[tokio::test]
async fn stop_hook_runs_on_tool_call_free_turn() {
    // Finding #6: should_stop_after_turn was only consulted after tool
    // execution, so a final answer (no tool calls) never triggered it despite
    // the documented "after a turn's tool execution" contract. It must fire
    // on tool-call-free turns too.
    struct CountStop(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl Hooks for CountStop {
        async fn should_stop_after_turn(&self, _ctx: &[AgentMessage]) -> bool {
            self.0.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    let rec = Recorder::default();
    rec.push(vec![LlmStreamEvent::Done {
        stop_reason: StopReason::Stop,
        usage: None,
    }]);
    let count = Arc::new(AtomicUsize::new(0));
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![],
        HooksSet::one(Arc::new(CountStop(count.clone()))),
    );

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "hook must fire once on the tool-call-free final turn"
    );
    assert_eq!(ctx.len(), 1, "no empty assistant persisted");
}

#[tokio::test]
async fn tool_call_roundtrip() {
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
        LlmStreamEvent::TextDelta("done".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    let (tool, seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], HooksSet::default());

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
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], HooksSet::default());

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

#[tokio::test]
async fn error_tool_result_is_annotated_in_wire_context() {
    // Finding #2: a failed tool's output must carry an error marker in the
    // context the next stream call actually receives — the wire has no
    // structured error flag, so the marker is what tells the LLM it failed.
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

    let (tool, _seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let calls = rec.calls();
    assert_eq!(calls.len(), 2, "loop continues to the next stream call");
    // The second stream call's recorded ctx is what reaches the LLM. The
    // ERROR: marker itself is applied by `to_rig_message` at wire conversion
    // (unit-tested in streamfn.rs); what the loop must guarantee here is that
    // the error *flag* survives into the stream context so that conversion
    // actually prefixes it.
    let (wire_output, wire_is_error) = calls[1]
        .ctx
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult {
                output, is_error, ..
            } => Some((output.clone(), *is_error)),
            _ => None,
        })
        .expect("a ToolResult reached the second stream call");
    assert!(wire_is_error, "error flag must reach the next stream call");
    assert!(
        wire_output.starts_with("invalid arguments for tool `echo`"),
        "raw output passes through unmodified into the stream context: {:?}",
        wire_output
    );
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
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![tool],
        HooksSet::one(Arc::new(BlockingHooks)),
    );

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

struct RenameHooks;

#[async_trait::async_trait]
impl Hooks for RenameHooks {
    async fn transform_tool_input(&self, call: &mut HookToolCall) {
        if let Some(obj) = call.arguments.as_object_mut()
            && let Some(text) = obj.get("text").and_then(|t| t.as_str())
        {
            obj.insert(
                "text".to_string(),
                serde_json::json!(format!("rewritten:{text}")),
            );
        }
    }
}

#[tokio::test]
async fn transform_tool_input_rewrites_args_before_execution() {
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
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![tool],
        HooksSet::one(Arc::new(RenameHooks)),
    );

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(
        &*seen.lock().unwrap(),
        &vec!["rewritten:hello".to_string()],
        "tool sees the rewritten argument"
    );
    let end = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolExecutionEnd { output, .. } => Some(output.clone()),
            _ => None,
        })
        .expect("ToolExecutionEnd emitted");
    assert_eq!(end, "echo:rewritten:hello");
    assert!(matches!(
        &ctx[2],
        AgentMessage::ToolResult { output, .. } if output == "echo:rewritten:hello"
    ));
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
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![tool],
        HooksSet::one(Arc::new(PatchingHooks)),
    );

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
    let TestSetup { cfg, .. } = setup(stream_fn, vec![], HooksSet::default());
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
    let res = run_loop(&mut ctx, cfg, tx).await.map(|r| r.stop_reason);

    assert_eq!(res.unwrap(), StopReason::Aborted);
    assert_eq!(ctx.len(), 2);
    assert!(matches!(
        &ctx[1],
        AgentMessage::Assistant { stop_reason: StopReason::Aborted, .. }
            if ctx[1].as_text() == "part"
    ));
}

#[tokio::test]
async fn cancel_before_first_delta_leaves_no_assistant() {
    // Pre-cancelled token: the biased select aborts before any delta, so the
    // run must end Aborted without persisting an empty assistant message.
    // MessageEnd/TurnEnd are still emitted so the event framing stays balanced.
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let stream_fn: StreamFn =
        Arc::new(|_ctx, _sys, _tools, _opts| Box::pin(futures::stream::pending()) as LlmStream);
    let TestSetup { cfg, .. } = setup(stream_fn, vec![], HooksSet::default());
    let cfg = LoopConfig { cancel, ..cfg };

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Aborted);
    assert_eq!(ctx.len(), 1, "no empty assistant may be recorded");
    assert_eq!(
        tags(&events),
        vec![
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end",
        ]
    );
}

#[tokio::test]
async fn cancel_before_turn_skips_llm_call() {
    // Finding #7: a cancel that lands before a turn starts must not mint an
    // LLM request at all — the biased select would abort it on the first poll
    // anyway, wasting a round-trip. stream_fn must never be invoked.
    let rec = Recorder::default();
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());
    let cfg = LoopConfig { cancel, ..cfg };

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Aborted);
    assert!(
        rec.calls().is_empty(),
        "stream_fn must not be invoked on an already-cancelled run"
    );
    assert_eq!(ctx.len(), 1, "no assistant persisted");
    assert_eq!(
        tags(&events),
        vec![
            "agent_start",
            "turn_start",
            "message_start",
            "message_end",
            "turn_end",
            "agent_end",
        ]
    );
}

#[tokio::test]
async fn stream_error_before_any_delta_leaves_no_assistant() {
    // A hard-fatal stream error on its first item produces no content; the run
    // must end Error without persisting an empty assistant message, yet still
    // close the turn framing (Error surfaces after MessageStart).
    let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
        Box::pin(futures::stream::iter(vec![LlmStreamEvent::Error {
            message: "boom".into(),
            fatal: true,
        }])) as LlmStream
    });
    let TestSetup { cfg, .. } = setup(stream_fn, vec![], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Error);
    assert_eq!(ctx.len(), 1, "no empty assistant may be recorded");
    assert_eq!(
        tags(&events),
        vec![
            "agent_start",
            "turn_start",
            "message_start",
            "error",
            "message_end",
            "turn_end",
            "agent_end",
        ]
    );
}

#[tokio::test]
async fn cancel_during_tool_loop_ends_run_without_extra_turn() {
    // The tool cancels the shared token mid-execution; the loop must end the
    // run right there instead of firing another LLM turn.
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "cancel".into(),
            arguments: serde_json::json!({ "text": "hi" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ]);
    let TestSetup { mut cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());
    cfg.tools = vec![erased(CancelTool {
        token: cfg.cancel.clone(),
    })];

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Aborted);
    // No second stream call: cancel during the tool loop ends the run.
    assert_eq!(
        rec.calls().len(),
        1,
        "cancel during a tool must not start another turn"
    );
    // Complete history: User, Assistant(ToolUse), one ToolResult. No orphan.
    assert_eq!(ctx.len(), 3);
    assert!(matches!(
        &ctx[2],
        AgentMessage::ToolResult { output, is_error: false, .. } if output == "ran:hi"
    ));
    assert_eq!(*tags(&events).last().unwrap(), "agent_end");
}

struct CancelTool {
    token: tokio_util::sync::CancellationToken,
}

#[async_trait::async_trait]
impl TypedTool for CancelTool {
    type Args = EchoArgs;
    fn name(&self) -> &str {
        "cancel"
    }
    fn description(&self) -> &str {
        "cancels the run token"
    }
    async fn execute(&self, args: EchoArgs, _ctx: &ToolContext) -> ToolOutput {
        self.token.cancel();
        ToolOutput {
            output: format!("ran:{}", args.text),
            is_error: false,
            diff: None,
            path: None,
        }
    }
}

struct YieldThenBlockHooks;

#[async_trait::async_trait]
impl Hooks for YieldThenBlockHooks {
    async fn before_tool_call(&self, _call: &HookToolCall) -> Option<String> {
        // Yield so the sink-watcher can drop the receiver mid-tool-round.
        tokio::task::yield_now().await;
        Some("nope".into())
    }
}

#[tokio::test]
async fn dead_sink_mid_tool_loop_synthesizes_results() {
    // Two tool calls in one turn; the watcher drops the event receiver once
    // ToolExecutionStart(c1) is observed, so the End(c1) send fails and the
    // loop must synthesize error ToolResults for both calls.
    let events = vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "one" }),
        },
        LlmStreamEvent::ToolCall {
            id: "c2".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "two" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: None,
        },
    ];
    let stream_fn: StreamFn = Arc::new(move |_ctx, _sys, _tools, _opts| {
        Box::pin(futures::stream::iter(events.clone())) as LlmStream
    });
    let (tool, _seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(
        stream_fn,
        vec![tool],
        HooksSet::one(Arc::new(YieldThenBlockHooks)),
    );

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if matches!(ev, AgentEvent::ToolExecutionStart { .. }) {
                break;
            }
        }
    });
    let res = run_loop(&mut ctx, cfg, tx).await.map(|r| r.stop_reason);

    assert_eq!(res.unwrap(), StopReason::Aborted);

    // The contract is pairing, not a message count: whatever the sink-death
    // detection timing (Start now follows preflight, so a whole group starts
    // together), ctx must never keep a ToolCall without its ToolResult.
    let calls: Vec<&str> = ctx
        .iter()
        .flat_map(|m| match m {
            AgentMessage::Assistant { content, .. } => content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolCall { id, .. } => Some(id.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    let results: Vec<&str> = ctx
        .iter()
        .filter_map(|m| match m {
            AgentMessage::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert!(!calls.is_empty(), "expected tool calls in ctx");
    assert_eq!(calls, results, "every ToolCall must have its ToolResult, in order");
    assert!(
        ctx.iter().any(|m| matches!(
            m,
            AgentMessage::ToolResult { is_error: true, .. }
        )),
        "the dead sink must surface as an error ToolResult"
    );
}

#[tokio::test]
async fn captured_error_after_tool_call_pairs_tool_results() {
    // ToolCall followed by Done{Error}: the run ends before tool execution,
    // but ctx must still pair the call with an error ToolResult — an
    // unpaired ToolCall is invalid history and 400s on the next turn.
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "echo".into(),
            arguments: serde_json::json!({ "text": "hello" }),
        },
        LlmStreamEvent::Done {
            stop_reason: StopReason::Error,
            usage: None,
        },
    ]);

    let (tool, seen) = echo_tool();
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Error);
    assert!(seen.lock().unwrap().is_empty(), "tool must not execute");
    // Every ToolCall in ctx is paired with an error ToolResult.
    let calls: Vec<&str> = ctx
        .iter()
        .flat_map(|m| m.tool_calls())
        .filter_map(|b| match b {
            ContentBlock::ToolCall { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let results: Vec<(&str, bool)> = ctx
        .iter()
        .filter_map(|m| match m {
            AgentMessage::ToolResult {
                tool_call_id,
                is_error,
                ..
            } => Some((tool_call_id.as_str(), *is_error)),
            _ => None,
        })
        .collect();
    assert_eq!(calls, vec!["c1"]);
    assert_eq!(results, vec![("c1", true)]);
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
        HooksSet::one(Arc::new(StopAfterTurnHooks)),
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

    let TestSetup { cfg, follow_tx, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());
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

    let TestSetup { cfg, steer_tx, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());
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

#[tokio::test]
async fn a_steer_at_the_tool_free_boundary_starts_a_second_turn() {
    // Point B: a steer queued AFTER turn 1's turn-start drain (here, from inside
    // the first stream call) is injected at the outer tail and starts a second
    // turn. This FAILS if the tail only drains `follow_ups`.
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

    let (steer_tx, steering) = mpsc::unbounded_channel();
    let (_follow_tx, follow_ups) = mpsc::unbounded_channel();

    // Queue the steer on the FIRST stream call: turn 1's turn-start drain has
    // already run, so only the tail can pick it up.
    let steer = steer_tx.clone();
    let rec_fn = rec.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_fn = calls.clone();
    let stream_fn: StreamFn = Arc::new(
        move |ctx: &[AgentMessage],
              system: &str,
              tools: &[rig::completion::ToolDefinition],
              _opts: &LlmOpts| {
            rec_fn.calls.lock().unwrap().push(StreamCall {
                ctx: ctx.to_vec(),
                system: system.to_string(),
                tools: tools.iter().map(|t| t.name.clone()).collect(),
            });
            if calls_fn.fetch_add(1, Ordering::SeqCst) == 0 {
                steer.send(AgentMessage::user_text("steer")).unwrap();
            }
            let events = rec_fn.script.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(futures::stream::iter(events)) as LlmStream
        },
    );

    let cfg = LoopConfig {
        system: "sys".into(),
        tools: vec![],
        llm: LlmOpts {
            model: "m1".into(),
            ..LlmOpts::default()
        },
        stream_fn,
        hooks: HooksSet::default(),
        steering,
        follow_ups,
        cancel: tokio_util::sync::CancellationToken::new(),
        working_dir: std::path::PathBuf::from("."),
        session: None,
        max_turns: wcode_harness::loop_::DEFAULT_MAX_TURNS,
        parallel: true,
        compaction: CompactionPolicy::default(),
    };

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    assert_eq!(rec.calls().len(), 2, "the tail steer starts a second turn");
    assert!(
        ctx.iter().any(|m| m.as_text() == "steer"),
        "the steer reached ctx: {ctx:?}"
    );
    let t = tags(&events);
    assert_eq!(t.iter().filter(|&&x| x == "turn_start").count(), 2);
    assert!(t.windows(2).any(|w| w == ["message_start", "message_end"]));
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
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), vec![tool], HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let t = tags(&events);
    assert!(t.contains(&"tool_execution_start"));
    assert!(t.contains(&"tool_execution_end"));
    assert_eq!(*t.last().unwrap(), "agent_end");
    assert_eq!(ctx.len(), 4);
}

/// Auto-compaction: a turn whose last provider-reported context size is over
/// the ceiling summarizes the older prefix before the request and emits a
/// `Compaction` event.
#[tokio::test]
async fn auto_compacts_when_over_the_ceiling() {
    let rec = Recorder::default();
    // 1st stream call is the summarizer; 2nd is the turn itself.
    rec.push(vec![
        LlmStreamEvent::TextDelta("CONDENSED".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    rec.push(vec![
        LlmStreamEvent::TextDelta("real reply".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    let TestSetup { mut cfg, .. } = setup(fake_stream_fn(&rec), vec![], HooksSet::default());
    cfg.compaction = CompactionPolicy {
        budget: Some(10),
        min_remaining: 0,
        keep_recent_tokens: 1, // keep only the newest message
        keep_recent_turns: 0,
        ..Default::default()
    };

    // Seed a turn whose assistant reported a context far over the ceiling.
    let earlier = AgentMessage::Assistant {
        content: vec![ContentBlock::Text { text: "earlier".into() }],
        stop_reason: StopReason::Stop,
        usage: Some(wcode_harness::message::Usage {
            input_tokens: 9_999,
            ..Default::default()
        }),
        model: None,
    };
    let mut ctx = vec![AgentMessage::user_text("first"), earlier];

    let (res, events) = run(cfg, &mut ctx).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let t = tags(&events);
    assert!(t.contains(&"compaction"), "expected a compaction event: {t:?}");
    assert!(
        ctx[0].as_text().contains("CONDENSED"),
        "summary leads the context: {:?}",
        ctx[0]
    );

    // The turn's request saw the compacted context, not the raw prefix.
    let calls = rec.calls();
    assert_eq!(calls.len(), 2, "summarizer + turn");
    assert!(
        calls[1].ctx[0].as_text().contains("CONDENSED"),
        "the request carries the summary: {:?}",
        calls[1].ctx[0]
    );
}

// ---------------------------------------------------------------------------
// Parallel tool execution
// ---------------------------------------------------------------------------

/// A tool that sleeps, logs when it starts and ends, and opts in or out of
/// concurrent execution. The log is `(name, "start"|"end")` in real order.
#[derive(Clone)]
struct TimedTool {
    name: &'static str,
    delay_ms: u64,
    parallel: bool,
    log: Arc<Mutex<Vec<(&'static str, &'static str)>>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TimedArgs {
    #[serde(default)]
    tag: String,
}

#[async_trait::async_trait]
impl TypedTool for TimedTool {
    type Args = TimedArgs;
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "timed test tool"
    }
    fn parallel_safe(&self) -> bool {
        self.parallel
    }
    async fn execute(&self, args: TimedArgs, _ctx: &ToolContext) -> ToolOutput {
        self.log.lock().unwrap().push((self.name, "start"));
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        self.log.lock().unwrap().push((self.name, "end"));
        ToolOutput {
            output: format!("{}:{}", self.name, args.tag),
            is_error: false,
            diff: None,
            path: None,
        }
    }
}

fn timed(name: &'static str, delay_ms: u64, parallel: bool, log: &Log) -> Tool {
    erased(TimedTool {
        name,
        delay_ms,
        parallel,
        log: Arc::clone(log),
    })
}

type Log = Arc<Mutex<Vec<(&'static str, &'static str)>>>;

fn log_of(log: &Log) -> Vec<(&'static str, &'static str)> {
    log.lock().unwrap().clone()
}

fn at(events: &[(&'static str, &'static str)], name: &str, ev: &str) -> usize {
    events
        .iter()
        .position(|(n, e)| *n == name && *e == ev)
        .unwrap_or_else(|| panic!("missing {name}/{ev} in {events:?}"))
}

/// Script one turn issuing `calls` (id, name), then a plain final turn.
fn script_batches(rec: &Recorder, calls: &[(&str, &'static str)]) {
    let mut first = Vec::new();
    for (id, name) in calls {
        first.push(LlmStreamEvent::ToolCall {
            id: (*id).into(),
            name: (*name).into(),
            arguments: serde_json::json!({ "tag": id }),
        });
    }
    first.push(LlmStreamEvent::Done {
        stop_reason: StopReason::ToolUse,
        usage: None,
    });
    rec.push(first);
    rec.push(vec![
        LlmStreamEvent::TextDelta("done".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
}

fn tool_outputs(ctx: &[AgentMessage]) -> Vec<String> {
    ctx.iter()
        .filter_map(|m| match m {
            AgentMessage::ToolResult { output, .. } => Some(output.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn concurrent_safe_calls_in_one_batch_overlap() {
    let rec = Recorder::default();
    script_batches(&rec, &[("c1", "a"), ("c2", "b")]);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![timed("a", 60, true, &log), timed("b", 60, true, &log)],
        HooksSet::default(),
    );

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _) = run(cfg, &mut ctx).await;
    assert_eq!(res.unwrap(), StopReason::Stop);

    // Both started before either ended: the sleeps overlapped.
    assert_eq!(
        log_of(&log),
        vec![("a", "start"), ("b", "start"), ("a", "end"), ("b", "end")],
        "expected both tools to start before either ended"
    );
}

#[tokio::test]
async fn non_safe_calls_never_overlap() {
    let rec = Recorder::default();
    script_batches(&rec, &[("c1", "a"), ("c2", "b")]);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![timed("a", 30, false, &log), timed("b", 30, false, &log)],
        HooksSet::default(),
    );

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _) = run(cfg, &mut ctx).await;
    assert_eq!(res.unwrap(), StopReason::Stop);

    assert_eq!(
        log_of(&log),
        vec![("a", "start"), ("a", "end"), ("b", "start"), ("b", "end")],
        "a non-parallel-safe call must not overlap its neighbours"
    );
}

#[tokio::test]
async fn safe_run_between_barriers_overlaps_and_keeps_call_order() {
    // a (barrier) | b, c (safe pair; c finishes first) | d (barrier)
    let rec = Recorder::default();
    script_batches(&rec, &[("c1", "a"), ("c2", "b"), ("c3", "c"), ("c4", "d")]);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let tools = vec![
        timed("a", 5, false, &log),
        timed("b", 60, true, &log),
        timed("c", 10, true, &log),
        timed("d", 5, false, &log),
    ];
    let TestSetup { cfg, .. } = setup(fake_stream_fn(&rec), tools, HooksSet::default());

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _) = run(cfg, &mut ctx).await;
    assert_eq!(res.unwrap(), StopReason::Stop);

    // Results land in CALL order even though c completed before b.
    assert_eq!(tool_outputs(&ctx), vec!["a:c1", "b:c2", "c:c3", "d:c4"]);

    let events = log_of(&log);
    assert!(at(&events, "b", "start") < at(&events, "c", "start"), "{events:?}");
    assert!(at(&events, "c", "end") < at(&events, "b", "end"), "c is faster: {events:?}");
    assert!(at(&events, "a", "end") < at(&events, "b", "start"), "{events:?}");
    assert!(at(&events, "b", "end") < at(&events, "d", "start"), "{events:?}");
}

#[tokio::test]
async fn parallel_false_forces_sequential_execution() {
    let rec = Recorder::default();
    script_batches(&rec, &[("c1", "a"), ("c2", "b")]);
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![timed("a", 20, true, &log), timed("b", 20, true, &log)],
        HooksSet::default(),
    );
    let cfg = LoopConfig {
        parallel: false,
        ..cfg
    };

    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (res, _) = run(cfg, &mut ctx).await;
    assert_eq!(res.unwrap(), StopReason::Stop);

    assert_eq!(
        log_of(&log),
        vec![("a", "start"), ("a", "end"), ("b", "start"), ("b", "end")],
        "parallel=false must restore sequential execution"
    );
}

struct DiffTool;

#[async_trait::async_trait]
impl TypedTool for DiffTool {
    type Args = NoArgs;
    fn name(&self) -> &str {
        "patch"
    }
    fn description(&self) -> &str {
        "returns an output plus a UI-only diff"
    }
    async fn execute(&self, _args: NoArgs, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput {
            output: "edited f".into(),
            is_error: false,
            diff: Some("@@ -1 +1 @@\n-old\n+new".into()),
            path: Some("f.txt".into()),
        }
    }
}

/// The diff and the changed path are UI-only: they ride `ToolExecutionEnd` to
/// the client but must NOT enter the `ToolResult` the model sees (that would tax
/// every edit).
#[tokio::test]
async fn the_diff_rides_the_event_but_not_the_model_context() {
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::ToolCall {
            id: "c1".into(),
            name: "patch".into(),
            arguments: serde_json::json!({}),
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

    let TestSetup { cfg, .. } = setup(
        fake_stream_fn(&rec),
        vec![erased(DiffTool)],
        HooksSet::default(),
    );
    let mut ctx = vec![AgentMessage::user_text("hi")];
    let (_res, events) = run(cfg, &mut ctx).await;

    let diff = events.iter().find_map(|e| match e {
        AgentEvent::ToolExecutionEnd { diff, .. } => diff.clone(),
        _ => None,
    });
    assert_eq!(diff.as_deref(), Some("@@ -1 +1 @@\n-old\n+new"));

    // The changed path rides alongside, likewise presentation-only.
    let path = events.iter().find_map(|e| match e {
        AgentEvent::ToolExecutionEnd { path, .. } => path.clone(),
        _ => None,
    });
    assert_eq!(path.as_deref(), Some("f.txt"));

    assert!(matches!(
        &ctx[2],
        AgentMessage::ToolResult { output, .. } if output == "edited f"
    ));
}
