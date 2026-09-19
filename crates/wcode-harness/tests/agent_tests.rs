//! Integration tests for the stateful `Agent` wrapper.
//!
//! Reuses the fake-stream harness from `loop_tests.rs` (copied: test files
//! don't share code without a common module, ~60 lines duplication is fine).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::compaction::{CompactOutcome, CompactionPolicy};
use wcode_harness::event::{AgentEvent, LlmStreamEvent};
use wcode_harness::hooks::HooksSet;
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::session::{Session, SessionEntry};
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
            diff: None,
            path: None,
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
        hooks: HooksSet::default(),
        session,
        context: Vec::new(),
        working_dir: std::path::PathBuf::new(),
        max_turns: wcode_harness::loop_::DEFAULT_MAX_TURNS,
        parallel_tools: true,
        compaction: wcode_harness::compaction::CompactionPolicy::default(),
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
async fn error_before_any_delta_persists_only_user_message() {
    // Finding #1 regression: a stream that errors before producing any content
    // must not leave an empty assistant message in ctx or the session.
    let stream_fn: StreamFn = Arc::new(|_ctx, _sys, _tools, _opts| {
        Box::pin(futures::stream::iter(vec![LlmStreamEvent::Error {
            message: "boom".into(),
            fatal: true,
        }])) as LlmStream
    });
    let dir = tempfile::tempdir().unwrap();
    let session = Session::create(dir.path()).unwrap();
    let path = session.path().unwrap().to_path_buf();
    let mut agent = Agent::new(agent_config(stream_fn, vec![], Some(session)));

    let (tx, _rx) = mpsc::unbounded_channel();
    let res = agent.run("hi", tx).await;

    assert_eq!(res.unwrap(), StopReason::Error);
    assert_eq!(
        agent.messages().len(),
        1,
        "only the user message, no empty assistant: {:?}",
        agent.messages()
    );

    let reopened = Session::open(&path).unwrap();
    let msgs = reopened.messages();
    assert_eq!(
        msgs.len(),
        1,
        "session must not contain an empty assistant: {msgs:?}"
    );
    assert!(matches!(&msgs[0], AgentMessage::User { .. } if msgs[0].as_text() == "hi"));
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

/// Finding #1: persistence is incremental. While a tool is still blocked, the
/// user message and the tool-calling assistant are already on disk — a crash
/// at this point re-resumes with those, losing only the in-flight tool.
#[tokio::test]
async fn session_persists_messages_before_run_finishes() {
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
        LlmStreamEvent::TextDelta("done".into()),
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

    let dir = tempfile::tempdir().unwrap();
    let session = Session::create(dir.path()).unwrap();
    let path = session.path().unwrap().to_path_buf();
    let mut agent = Agent::new(agent_config(
        fake_stream_fn(&rec),
        vec![gate],
        Some(session),
    ));

    let (tx, _rx) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move { agent.run("hi", tx).await.unwrap() });
    entered_rx.recv().await.unwrap(); // gate entered: the run is mid-flight

    let mid = Session::open(&path).unwrap();
    let msgs = mid.messages();
    assert_eq!(
        msgs.len(),
        2,
        "messages must be on disk pre-finish: {msgs:?}"
    );
    assert!(matches!(&msgs[0], AgentMessage::User { .. } if msgs[0].as_text() == "hi"));

    release.notify_one();
    assert_eq!(handle.await.unwrap(), StopReason::Stop);

    let after = Session::open(&path).unwrap();
    let msgs = after.messages();
    assert_eq!(
        msgs.len(),
        4,
        "user + tool-call assistant + ToolResult + final assistant"
    );
    assert!(matches!(&msgs[3], AgentMessage::Assistant { .. } if msgs[3].as_text() == "done"));
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

#[tokio::test]
async fn steer_sender_clone_survives_runs() {
    // Finding #5: run_loop hands the lent channels back at the end of a run,
    // so a sender clone taken *before* run1 stays valid for run2 — a message
    // sent in between reaches run2. Previously the receivers were re-paired
    // with fresh channels and the clone's channel was dropped, silently
    // losing the message.
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
    let steer = agent.steer_sender(); // pre-run1 clone

    let (tx1, _rx1) = mpsc::unbounded_channel();
    agent.run("hi", tx1).await.unwrap();
    assert_eq!(rec.calls().len(), 1);

    let _ = steer.send(AgentMessage::user_text("straggler"));

    let (tx2, _rx2) = mpsc::unbounded_channel();
    let res = agent.run("again", tx2).await;

    assert_eq!(res.unwrap(), StopReason::Stop);
    let calls = rec.calls();
    assert_eq!(calls.len(), 2);
    let ctx2 = &calls[1].ctx;
    assert!(
        matches!(&ctx2[3], AgentMessage::User { .. } if ctx2[3].as_text() == "straggler"),
        "pre-run steer clone must reach the next run, got: {:?}",
        ctx2
    );
}

/// Run 1 is canceled mid-stream; run 2 must execute normally on a fresh
/// token (a canceled token must not poison the next run).
#[tokio::test]
async fn cancel_token_refreshed_between_runs() {
    let call = Arc::new(AtomicUsize::new(0));
    let stream_fn: StreamFn = {
        let call = call.clone();
        Arc::new(move |_ctx, _sys, _tools, _opts| {
            if call.fetch_add(1, Ordering::SeqCst) == 0 {
                // Run 1: one delta, then hang until the watcher cancels.
                Box::pin(
                    futures::stream::iter(vec![LlmStreamEvent::TextDelta("part".into())])
                        .chain(futures::stream::pending()),
                ) as LlmStream
            } else {
                Box::pin(futures::stream::iter(vec![
                    LlmStreamEvent::TextDelta("after".into()),
                    LlmStreamEvent::Done {
                        stop_reason: StopReason::Stop,
                        usage: None,
                    },
                ])) as LlmStream
            }
        })
    };
    let mut agent = Agent::new(agent_config(stream_fn, vec![], None));

    let token = agent.cancel_token();
    let (tx1, mut rx1) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(ev) = rx1.recv().await {
            if matches!(ev, AgentEvent::MessageUpdate { .. }) {
                token.cancel();
                break;
            }
        }
    });
    let res1 = agent.run("hi", tx1).await;
    assert_eq!(res1.unwrap(), StopReason::Aborted);

    // Run 2: the stale canceled token must not abort it.
    let (tx2, _rx2) = mpsc::unbounded_channel();
    let res2 = agent.run("again", tx2).await;
    assert_eq!(res2.unwrap(), StopReason::Stop);
    assert_eq!(call.load(Ordering::SeqCst), 2);
    assert!(
        matches!(
            agent.messages().last().unwrap(),
            AgentMessage::Assistant { stop_reason: StopReason::Stop, .. }
                if agent.messages().last().unwrap().as_text() == "after"
        ),
        "run 2 must stream normally, got: {:?}",
        agent.messages()
    );
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

/// `/effort` backbone: set_effort swaps LlmOpts for the next stream call and
/// logs an EffortChange entry; None clears back to send-nothing.
#[tokio::test]
async fn set_effort_swaps_llm_and_logs_session_change() {
    let rec = Recorder::default();
    for _ in 0..3 {
        rec.push(vec![
            LlmStreamEvent::TextDelta("x".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]);
    }

    // Recorder only captures model; re-read effort via a wrapping stream_fn.
    let rec2 = rec.clone();
    let effort_seen = Arc::new(Mutex::new(Vec::new()));
    let stream_fn = {
        let seen = effort_seen.clone();
        let inner = fake_stream_fn(&rec2);
        Arc::new(
            move |ctx: &[AgentMessage],
                  system: &str,
                  tools: &[rig::completion::ToolDefinition],
                  opts: &LlmOpts| {
                seen.lock().unwrap().push(opts.effort.clone());
                inner(ctx, system, tools, opts)
            },
        ) as StreamFn
    };

    let dir = tempfile::tempdir().unwrap();
    let session = Session::create(dir.path()).unwrap();
    let path = session.path().unwrap().to_path_buf();
    let mut agent = Agent::new(agent_config(stream_fn, vec![], Some(session)));

    let (tx, _rx) = mpsc::unbounded_channel();
    agent.run("hi", tx).await.unwrap();
    agent.set_effort(Some("high".into())).unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    agent.run("again", tx).await.unwrap();
    agent.set_effort(None).unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    agent.run("third", tx).await.unwrap();

    assert_eq!(
        *effort_seen.lock().unwrap(),
        vec![None, Some("high".to_string()), None]
    );
    let reopened = Session::open(&path).unwrap();
    assert_eq!(reopened.effort(), Some(None));
}

#[tokio::test]
async fn compact_replaces_prefix_with_a_summary() {
    let rec = Recorder::default();
    // The one stream call is the summarizer.
    rec.push(vec![
        LlmStreamEvent::TextDelta("Context: condensed history".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);
    let mut cfg = agent_config(fake_stream_fn(&rec), vec![], None);
    // A seeded history well past the tiny keep budget below.
    cfg.context = (0..20)
        .map(|i| AgentMessage::user_text(format!("message {i} {}", "x".repeat(400))))
        .collect();
    // Keep ~2 of the ~104-token messages.
    cfg.compaction = CompactionPolicy {
        keep_recent_tokens: 200,
        keep_recent_turns: 0,
        ..Default::default()
    };
    let mut agent = Agent::new(cfg);
    let before = agent.messages().len();

    match agent.compact(None).await.unwrap() {
        CompactOutcome::Done {
            summarized, kept, ..
        } => {
            assert_eq!(summarized + kept, before, "accounting covers all of ctx");
            assert_eq!(kept, 2, "kept the newest messages within the budget");
            assert_eq!(agent.messages().len(), kept + 1, "summary + kept");
            assert!(
                agent.messages()[0].as_text().contains("condensed history"),
                "the summary replaces the summarized prefix"
            );
        }
        other => panic!("expected Done, got {other:?}"),
    }
}

#[tokio::test]
async fn compact_is_a_noop_when_history_fits_the_budget() {
    let rec = Recorder::default();
    let mut cfg = agent_config(fake_stream_fn(&rec), vec![], None);
    cfg.context = vec![AgentMessage::user_text("short")];
    let mut agent = Agent::new(cfg);

    let out = agent.compact(None).await.unwrap();
    assert!(matches!(out, CompactOutcome::NothingToDo));
    assert_eq!(agent.messages().len(), 1, "nothing dropped");
    assert!(rec.calls().is_empty(), "the summarizer was never called");
}

#[tokio::test]
async fn compact_persists_and_resumes_to_summary_plus_tail() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::default();
    rec.push(vec![
        LlmStreamEvent::TextDelta("summary body".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: None,
        },
    ]);

    // Persist ten ~104-token messages, then seed the agent with exactly them.
    let mut session = Session::create(dir.path()).unwrap();
    for i in 0..10 {
        session
            .append(SessionEntry::Message {
                id: format!("id{i}"),
                parent_id: None,
                message: AgentMessage::user_text(format!("m{i} {}", "x".repeat(400))),
            })
            .unwrap();
    }
    let path = session.path().unwrap().to_path_buf();
    let seeded = session.messages();

    let mut cfg = agent_config(fake_stream_fn(&rec), vec![], Some(session));
    cfg.context = seeded;
    cfg.compaction = CompactionPolicy {
        keep_recent_tokens: 200, // ~two messages
        keep_recent_turns: 0,
        ..Default::default()
    };
    let mut agent = Agent::new(cfg);

    agent.compact(None).await.unwrap();

    // In memory: [summary, m8, m9].
    let ctx = agent.messages();
    assert_eq!(ctx.len(), 3, "summary + two kept");
    assert!(ctx[0].as_text().contains("summary body"));
    assert!(ctx[1].as_text().starts_with("m8 "));
    assert!(ctx[2].as_text().starts_with("m9 "));

    // On disk, a reopen rebuilds the same compacted view.
    let reopened = Session::open(&path).unwrap();
    let resumed: Vec<String> = reopened.messages().iter().map(|m| m.as_text()).collect();
    assert_eq!(resumed.len(), 3, "summary + two kept after resume");
    assert!(resumed[0].contains("summary body"), "summary leads: {resumed:?}");
    assert!(resumed[1].starts_with("m8 "));
    assert!(resumed[2].starts_with("m9 "));
    assert_eq!(
        reopened.message_count(),
        10,
        "all messages stay on disk; only the view is compacted"
    );
}
