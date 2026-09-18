//! An offline demo session: a scripted "echo" stream fn, so the TUI can stream
//! a turn without a network. `cargo run -p wcode-tui --example demo`, then type
//! a line and press Enter.

use std::path::PathBuf;
use std::sync::Arc;

use wcode_harness::actor::SessionActor;
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::event::LlmStreamEvent;
use wcode_harness::hooks::HooksSet;
use wcode_harness::loop_::DEFAULT_MAX_TURNS;
use wcode_harness::message::{AgentMessage, StopReason, Usage};
use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
use wcode_harness::protocol::SessionId;
use wcode_protocol::Backend;
use wcode_tui::Status;

fn echo_stream_fn() -> StreamFn {
    Arc::new(|ctx: &[AgentMessage], _system, _tools, _opts: &LlmOpts| {
        let prompt = ctx
            .iter()
            .rev()
            .find_map(|m| match m {
                AgentMessage::User { .. } => Some(m.as_text()),
                _ => None,
            })
            .unwrap_or_default();
        let reply = format!("You said: {prompt}\n\n(This is the scripted demo session.)");
        // Stream word by word, like a real provider.
        let mut events: Vec<LlmStreamEvent> = reply
            .split_inclusive(' ')
            .map(|word| LlmStreamEvent::TextDelta(word.to_string()))
            .collect();
        events.push(LlmStreamEvent::Done {
            stop_reason: StopReason::Stop,
            usage: Some(Usage {
                input_tokens: 150_000,
                output_tokens: 42,
                cache_read_tokens: Some(120_000),
                cache_write_tokens: None,
            }),
        });
        Box::pin(futures::stream::iter(events)) as LlmStream
    })
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = AgentConfig {
        system: "You are the wcode demo session.".into(),
        tools: vec![],
        llm: LlmOpts {
            model: "demo".into(),
            ..LlmOpts::default()
        },
        stream_fn: echo_stream_fn(),
        hooks: HooksSet::default(),
        session: None,
        context: Vec::new(),
        working_dir: PathBuf::new(),
        max_turns: DEFAULT_MAX_TURNS,
        parallel_tools: false,
        compaction: CompactionPolicy::default(),
    };
    let handle = SessionActor::spawn(Agent::new(config));
    let status = Status {
        model: "demo".into(),
        effort: None,
        session: Some("demo-session".into()),
        context_limit: Some(200_000),
    };
    let options = wcode_tui::Options {
        status,
        models: vec!["demo".to_string(), "demo-mini".to_string()],
        sessions: Vec::new(),
        history: None,
    };
    // A static roster so the example shows the team sidebar (root + two members).
    let surfaces = vec![
        wcode_tui::SurfaceSpec {
            id: SessionId::agent("root"),
            label: "root".to_string(),
            model: "demo".to_string(),
            is_root: true,
            backend: Backend::from(handle.clone()),
        },
        wcode_tui::SurfaceSpec {
            id: SessionId::agent("explorer"),
            label: "explorer".to_string(),
            model: "demo".to_string(),
            is_root: false,
            backend: Backend::from(handle.clone()),
        },
        wcode_tui::SurfaceSpec {
            id: SessionId::agent("reviewer"),
            label: "reviewer".to_string(),
            model: "demo-mini".to_string(),
            is_root: false,
            backend: Backend::from(handle),
        },
    ];
    wcode_tui::run(surfaces, options, None).await.map(|_| ())
}
