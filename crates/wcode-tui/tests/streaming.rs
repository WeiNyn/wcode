//! End-to-end: a real `SessionActor` (scripted stream) → the app's reducer,
//! over the same event stream the TUI consumes. No terminal involved.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wcode_harness::actor::SessionActor;
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::event::{AgentEvent, LlmStreamEvent};
use wcode_harness::hooks::HooksSet;
use wcode_harness::loop_::DEFAULT_MAX_TURNS;
use wcode_harness::message::{ContentBlock, StopReason, Usage};
use wcode_harness::protocol::Request;
use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
use wcode_protocol::Backend;
use wcode_tui::{Action, App, AppEvent, Block, Key};

fn scripted(events: Vec<LlmStreamEvent>) -> StreamFn {
    Arc::new(move |_ctx, _system, _tools, _opts| {
        Box::pin(futures::stream::iter(events.clone())) as LlmStream
    })
}

fn config(stream_fn: StreamFn) -> AgentConfig {
    AgentConfig {
        system: "sys".into(),
        tools: vec![],
        llm: LlmOpts {
            model: "m1".into(),
            ..LlmOpts::default()
        },
        stream_fn,
        hooks: HooksSet::default(),
        session: None,
        context: Vec::new(),
        working_dir: PathBuf::new(),
        max_turns: DEFAULT_MAX_TURNS,
        parallel_tools: false,
        compaction: CompactionPolicy::default(),
    }
}

fn text_of(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_submitted_turn_streams_into_the_transcript() {
    let handle = SessionActor::spawn(Agent::new(config(scripted(vec![
        LlmStreamEvent::TextDelta("hel".into()),
        LlmStreamEvent::TextDelta("lo".into()),
        LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: Some(Usage {
                input_tokens: 1234,
                output_tokens: 5,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
        },
    ]))));
    let backend = Backend::Local(handle);
    let mut app = App::new();
    let mut rx = backend.subscribe();

    // Type a prompt and submit, exactly as the event loop would.
    for c in "hi".chars() {
        app.handle(AppEvent::Key(Key::Char(c)));
    }
    app.handle(AppEvent::Key(Key::Enter));
    for action in app.take_actions() {
        if let Action::Submit(text) = action {
            backend.send(Request::Submit { text }).unwrap();
        }
    }

    // Pump the session's stream into the app until the run ends.
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an event within the timeout")
            .expect("the outbox stays open");
        let done = matches!(event, AgentEvent::AgentEnd);
        let id = app.focused_id().clone();
        app.handle(AppEvent::Agent(id, event));
        if done {
            break;
        }
    }

    assert!(!app.running());
    assert_eq!(app.context_used(), Some(1234));
    assert_eq!(app.transcript()[0], Block::User("hi".into()));
    match app.transcript().last() {
        Some(Block::Assistant(content)) => assert_eq!(text_of(content), "hello"),
        other => panic!("expected the assistant block, got {other:?}"),
    }
}
