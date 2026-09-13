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
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
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
        let events = vec![
            LlmStreamEvent::TextDelta(reply),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ];
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
    wcode_tui::run(Backend::Local(handle), Status::new("demo")).await
}
