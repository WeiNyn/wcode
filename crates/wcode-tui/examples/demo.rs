//! An offline demo session: a scripted "echo" stream fn, so the TUI can stream
//! a turn without a network. `cargo run -p wcode-tui --example demo`, then type
//! a line and press Enter.
//!
//! The first turn calls one trivial read-only tool that returns a long,
//! multi-line listing, so the collapsed/expanded tool output can be eyeballed
//! live — `Ctrl-T` toggles all tools, or `Ctrl-G` → `Enter` toggles one.

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
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool, erased};
use wcode_harness::protocol::SessionId;
use wcode_protocol::Backend;
use wcode_tui::Status;

/// A trivial read-only tool whose fixed output is long enough that the collapsed
/// preview elides it — the whole point of the demo.
#[derive(Clone)]
struct DemoRead;

#[async_trait::async_trait]
impl TypedTool for DemoRead {
    type Args = ();

    fn name(&self) -> &str {
        "demo_read"
    }

    fn description(&self) -> &str {
        "Return a fixed 30-line listing so the TUI can show (and expand) tool output."
    }

    fn parallel_safe(&self) -> bool {
        true
    }

    async fn execute(&self, _args: (), _ctx: &ToolContext) -> ToolOutput {
        let output = (1..=30)
            .map(|i| format!("    {i:>2}  src/module_{i:02}.rs  {} bytes", i * 137 % 521))
            .collect::<Vec<_>>()
            .join("\n");
        ToolOutput {
            output,
            is_error: false,
            diff: None,
            path: None,
        }
    }
}

fn scripted_stream_fn() -> StreamFn {
    Arc::new(|ctx: &[AgentMessage], _system, _tools, _opts: &LlmOpts| {
        // First turn: call the scripted tool. The loop then runs it and calls us
        // again with the result, which is where we answer in prose.
        let answered = ctx
            .iter()
            .any(|m| matches!(m, AgentMessage::ToolResult { .. }));
        if !answered {
            return Box::pin(futures::stream::iter(vec![
                LlmStreamEvent::ToolCall {
                    id: "demo-1".to_string(),
                    name: "demo_read".to_string(),
                    arguments: Default::default(),
                },
                LlmStreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    usage: Some(Usage {
                        input_tokens: 150_000,
                        output_tokens: 12,
                        cache_read_tokens: Some(120_000),
                        cache_write_tokens: None,
                    }),
                },
            ])) as LlmStream;
        }

        let prompt = ctx
            .iter()
            .rev()
            .find_map(|m| match m {
                AgentMessage::User { .. } => Some(m.as_text()),
                _ => None,
            })
            .unwrap_or_default();
        let reply =
            format!("You said: {prompt}\n\n(That was a scripted demo tool result above.)");
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
        tools: vec![erased(DemoRead)],
        llm: LlmOpts {
            model: "demo".into(),
            ..LlmOpts::default()
        },
        stream_fn: scripted_stream_fn(),
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
