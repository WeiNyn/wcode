//! Interactive REPL: stdin line loop, `/commands`, event printing, Ctrl-C.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::event::AgentEvent;
use wcode_harness::hooks::DefaultHooks;
use wcode_harness::message::{AgentMessage, ContentBlock, StopReason};
use wcode_harness::session::Session;
use wcode_harness::streamfn::{LlmOpts, rig_stream_fn};

use crate::tools::default_tools;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const SYSTEM_PROMPT: &str = "You are wcode, a minimal coding agent working in the user's \
current directory. Use the read, bash, edit and write tools to inspect and modify files. \
Be concise.";

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Exit,
    New,
    /// Some(id) = switch; None = `/model` without an id (usage error).
    Model(Option<String>),
    /// Some(path) = open that session; None = latest in the session dir.
    Resume(Option<String>),
    Sessions,
}

/// `/command` lines parse to a Command; anything else (including unknown
/// `/...`) is prompt text for the LLM.
pub fn parse_command(line: &str) -> Option<Command> {
    let rest = line.trim().strip_prefix('/')?;
    let (cmd, arg) = match rest.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (rest, ""),
    };
    let arg = (!arg.is_empty()).then(|| arg.to_string());
    match cmd {
        "exit" => Some(Command::Exit),
        "new" => Some(Command::New),
        "model" => Some(Command::Model(arg)),
        "resume" => Some(Command::Resume(arg)),
        "sessions" => Some(Command::Sessions),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Event printing (pure diff core)
// ---------------------------------------------------------------------------

/// Diffs streamed assistant snapshots into what still needs printing.
/// Content only ever grows (deltas append), so "new length minus printed
/// length" is a valid char-boundary suffix.
#[derive(Default)]
pub struct MessagePrinter {
    text_len: usize,
    thinking_len: usize,
    mid_line: bool,
}

impl MessagePrinter {
    /// A new message started: forget accumulated lengths.
    pub fn reset(&mut self) {
        self.text_len = 0;
        self.thinking_len = 0;
    }

    /// Returns (text delta for stdout, thinking delta for stderr).
    /// Non-assistant messages (e.g. steering) emit nothing.
    pub fn update(&mut self, msg: &AgentMessage) -> (String, String) {
        let AgentMessage::Assistant { content, .. } = msg else {
            return (String::new(), String::new());
        };
        let text: String = content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let thinking: String = content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Thinking { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();

        let mut text_out = String::new();
        if text.len() > self.text_len {
            text_out = text[self.text_len..].to_string();
            self.text_len = text.len();
            self.mid_line = true;
        }
        let mut think_out = String::new();
        if thinking.len() > self.thinking_len {
            think_out = thinking[self.thinking_len..].to_string();
            self.thinking_len = thinking.len();
        }
        (text_out, think_out)
    }

    pub fn mid_line(&self) -> bool {
        self.mid_line
    }

    pub fn clear_mid_line(&mut self) {
        self.mid_line = false;
    }
}

/// First line of tool output, truncated to 120 chars, for the tool-end suffix.
pub fn tool_output_note(output: &str) -> String {
    let first = output.lines().next().unwrap_or("");
    let mut note: String = first.chars().take(120).collect();
    if first.chars().count() > 120 {
        note.push('…');
    }
    note
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

pub fn session_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/wcode/sessions")
}

/// Session files, newest first. Names are `{millis}_{hex}.jsonl`, so
/// lexicographic order is chronological.
pub fn list_sessions(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        // No dir yet = no sessions; callers report it as empty.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |e| e == "jsonl"))
        .collect();
    files.sort_unstable();
    files.reverse();
    Ok(files)
}

/// A `/resume` arg: use it verbatim when it exists, else relative to the
/// session dir (so listing output can be pasted back).
pub fn resolve_session_path(arg: &str) -> PathBuf {
    let p = PathBuf::from(arg);
    if p.exists() {
        p
    } else {
        session_dir().join(arg)
    }
}

pub fn build_agent(llm: LlmOpts, session: Option<Session>, context: Vec<AgentMessage>) -> Agent {
    Agent::new(AgentConfig {
        system: SYSTEM_PROMPT.into(),
        tools: default_tools(),
        llm,
        stream_fn: rig_stream_fn(),
        hooks: Arc::new(DefaultHooks),
        session,
        context,
    })
}

// ---------------------------------------------------------------------------
// REPL
// ---------------------------------------------------------------------------

pub async fn run(mut agent: Agent, mut llm: LlmOpts) {
    let in_flight = Arc::new(AtomicBool::new(false));
    // Ctrl-C lives on a separate task that must reach the token of whatever
    // run is active; the slot is refreshed after each run / agent swap.
    let cancel_slot: Arc<Mutex<CancellationToken>> = Arc::new(Mutex::new(agent.cancel_token()));
    {
        let in_flight = in_flight.clone();
        let cancel_slot = cancel_slot.clone();
        tokio::spawn(async move {
            while tokio::signal::ctrl_c().await.is_ok() {
                if in_flight.load(Ordering::SeqCst) {
                    cancel_slot.lock().unwrap().cancel();
                } else {
                    println!();
                    std::process::exit(0);
                }
            }
        });
    }

    println!("wcode {} — model: {}", env!("CARGO_PKG_VERSION"), llm.model);
    match agent.session_path() {
        Some(p) => println!("session: {}", p.display()),
        None => println!("(no session)"),
    }

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("❯ ");
        let _ = io::stdout().flush();
        let Ok(Some(line)) = lines.next_line().await else {
            break; // EOF or stdin error: exit
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_command(line) {
            Some(Command::Exit) => break,
            Some(Command::New) => {
                let session = (agent.session_path().is_some())
                    .then(|| Session::create(&session_dir()))
                    .transpose();
                match session {
                    Ok(session) => {
                        let path = session.as_ref().and_then(|s| s.path().map(Path::to_path_buf));
                        agent = build_agent(llm.clone(), session, Vec::new());
                        *cancel_slot.lock().unwrap() = agent.cancel_token();
                        match path {
                            Some(p) => println!("new session: {}", p.display()),
                            None => println!("new conversation"),
                        }
                    }
                    Err(e) => eprintln!("create session: {e}"),
                }
            }
            Some(Command::Model(arg)) => match arg {
                Some(id) => match agent.set_model(id.clone()) {
                    Ok(()) => {
                        llm.model = id;
                        println!("model: {}", llm.model);
                    }
                    Err(e) => eprintln!("model: {e}"),
                },
                None => println!("usage: /model <id>"),
            },
            Some(Command::Resume(arg)) => {
                let path = match arg {
                    Some(a) => resolve_session_path(&a),
                    None => match list_sessions(&session_dir()) {
                        Ok(list) => match list.into_iter().next() {
                            Some(p) => p,
                            None => {
                                println!("no sessions in {}", session_dir().display());
                                continue;
                            }
                        },
                        Err(e) => {
                            eprintln!("list sessions: {e}");
                            continue;
                        }
                    },
                };
                match Session::open(&path) {
                    Ok(s) => {
                        let messages = s.messages();
                        if let Some(m) = s.model() {
                            llm.model = m;
                        }
                        let n = messages.len();
                        agent = build_agent(llm.clone(), Some(s), messages);
                        *cancel_slot.lock().unwrap() = agent.cancel_token();
                        println!("resumed {} ({n} messages)", path.display());
                    }
                    Err(e) => eprintln!("open {}: {e}", path.display()),
                }
            }
            Some(Command::Sessions) => match list_sessions(&session_dir()) {
                Ok(list) if list.is_empty() => {
                    println!("no sessions in {}", session_dir().display())
                }
                Ok(list) => {
                    let current = agent.session_path();
                    for p in list {
                        let mark = if current == Some(p.as_path()) { "*" } else { " " };
                        println!(
                            "{mark} {}",
                            p.file_name().unwrap_or_default().to_string_lossy()
                        );
                    }
                }
                Err(e) => eprintln!("list sessions: {e}"),
            },
            None => run_turn(&mut agent, line, &in_flight, &cancel_slot).await,
        }
    }
}

/// One user turn: spawn the printer over a fresh sink, run, clean up.
async fn run_turn(
    agent: &mut Agent,
    input: &str,
    in_flight: &AtomicBool,
    cancel_slot: &Mutex<CancellationToken>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let printer = tokio::spawn(print_events(rx));
    in_flight.store(true, Ordering::SeqCst);
    let res = agent.run(input, tx).await;
    in_flight.store(false, Ordering::SeqCst);
    *cancel_slot.lock().unwrap() = agent.cancel_token();
    let _ = printer.await;
    match res {
        Ok(StopReason::Aborted) => println!("{DIM}(aborted){RESET}"),
        Ok(StopReason::Error) => println!("{DIM}✗ run failed{RESET}"),
        Ok(_) => {}
        Err(e) => println!("{DIM}error: {e}{RESET}"),
    }
}

async fn print_events(mut rx: mpsc::UnboundedReceiver<AgentEvent>) {
    let mut p = MessagePrinter::default();
    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::MessageStart { .. } => p.reset(),
            AgentEvent::MessageUpdate { message } => emit(&mut p, &message),
            AgentEvent::MessageEnd { message } => {
                emit(&mut p, &message);
                if p.mid_line() {
                    println!();
                    p.clear_mid_line();
                }
            }
            AgentEvent::ToolExecutionStart { name, .. } => {
                out(&format!("\r{DIM}⚙ {name}"));
            }
            AgentEvent::ToolExecutionUpdate { partial, .. } => out(&partial),
            AgentEvent::ToolExecutionEnd { output, is_error, .. } => {
                let mark = if is_error { "✗" } else { "✓" };
                let note = tool_output_note(&output);
                if note.is_empty() {
                    println!(" {mark}{RESET}");
                } else {
                    println!(" {mark} {DIM}{note}{RESET}");
                }
            }
            _ => {}
        }
    }
    if p.mid_line() {
        println!();
    }
}

fn emit(p: &mut MessagePrinter, message: &AgentMessage) {
    let (text, thinking) = p.update(message);
    if !thinking.is_empty() {
        eprint!("{DIM}{thinking}{RESET}");
        let _ = io::stderr().flush();
    }
    if !text.is_empty() {
        out(&text);
    }
}

fn out(s: &str) {
    print!("{s}");
    let _ = io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wcode_harness::message::StopReason;

    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::Assistant {
            content,
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        }
    }

    #[test]
    fn parse_command_table() {
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("hello"), None);
        assert_eq!(parse_command("/unknown x"), None);
        assert_eq!(parse_command("  /exit  "), Some(Command::Exit));
        assert_eq!(parse_command("/new"), Some(Command::New));
        assert_eq!(
            parse_command("/model gpt-x"),
            Some(Command::Model(Some("gpt-x".into())))
        );
        assert_eq!(parse_command("/model"), Some(Command::Model(None)));
        assert_eq!(parse_command("/resume"), Some(Command::Resume(None)));
        assert_eq!(
            parse_command("/resume /tmp/s.jsonl"),
            Some(Command::Resume(Some("/tmp/s.jsonl".into())))
        );
        assert_eq!(parse_command("/sessions"), Some(Command::Sessions));
        assert_eq!(parse_command("/sessions now"), Some(Command::Sessions));
    }

    #[test]
    fn printer_diffs_text_deltas() {
        let mut p = MessagePrinter::default();
        let m = assistant(vec![ContentBlock::Text {
            text: "Hello".into(),
        }]);
        assert_eq!(p.update(&m), ("Hello".to_string(), String::new()));
        let m = assistant(vec![ContentBlock::Text {
            text: "Hello world".into(),
        }]);
        assert_eq!(p.update(&m), (" world".to_string(), String::new()));
    }

    #[test]
    fn printer_splits_thinking_from_text() {
        let mut p = MessagePrinter::default();
        let m = assistant(vec![ContentBlock::Thinking { text: "hmm".into() }]);
        assert_eq!(p.update(&m), (String::new(), "hmm".to_string()));
        let m = assistant(vec![
            ContentBlock::Thinking {
                text: "hmm ok".into(),
            },
            ContentBlock::Text { text: "hi".into() },
        ]);
        assert_eq!(p.update(&m), ("hi".to_string(), " ok".to_string()));
    }

    #[test]
    fn printer_handles_interleaved_blocks() {
        let mut p = MessagePrinter::default();
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::Text {
                text: "a".into()
            }])),
            ("a".to_string(), String::new())
        );
        assert_eq!(
            p.update(&assistant(vec![
                ContentBlock::Text { text: "a".into() },
                ContentBlock::Thinking { text: "t".into() },
            ])),
            (String::new(), "t".to_string())
        );
        assert_eq!(
            p.update(&assistant(vec![
                ContentBlock::Text { text: "a".into() },
                ContentBlock::Thinking { text: "t".into() },
                // fresh Text block grows from empty deltas
                ContentBlock::Text { text: "b".into() },
            ])),
            ("b".to_string(), String::new())
        );
    }

    #[test]
    fn printer_reset_restarts_diff() {
        let mut p = MessagePrinter::default();
        p.update(&assistant(vec![ContentBlock::Text {
            text: "first".into(),
        }]));
        p.reset();
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::Text {
                text: "next".into()
            }])),
            ("next".to_string(), String::new())
        );
    }

    #[test]
    fn printer_ignores_non_assistant_and_tool_calls() {
        let mut p = MessagePrinter::default();
        assert_eq!(
            p.update(&AgentMessage::user_text("q")),
            (String::new(), String::new())
        );
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::ToolCall {
                id: "t".into(),
                name: "bash".into(),
                arguments: json!({}),
            }])),
            (String::new(), String::new())
        );
    }

    #[test]
    fn printer_mid_line_tracks_text_only() {
        let mut p = MessagePrinter::default();
        assert!(!p.mid_line());
        p.update(&assistant(vec![ContentBlock::Thinking { text: "t".into() }]));
        assert!(!p.mid_line());
        p.update(&assistant(vec![
            ContentBlock::Thinking { text: "t".into() },
            ContentBlock::Text { text: "hi".into() },
        ]));
        assert!(p.mid_line());
        p.clear_mid_line();
        assert!(!p.mid_line());
    }

    #[test]
    fn tool_note_first_line_truncated() {
        assert_eq!(tool_output_note("a\nb"), "a");
        assert_eq!(tool_output_note(""), "");
        assert_eq!(tool_output_note("trailing\n"), "trailing");
        let long = "x".repeat(200);
        let note = tool_output_note(&long);
        assert_eq!(note.chars().count(), 121); // 120 + ellipsis
        assert!(note.ends_with('…'));
    }

    #[test]
    fn list_sessions_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["100_b.jsonl", "200_a.jsonl", "50_c.txt", "300_d.jsonl"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let names: Vec<String> = list_sessions(dir.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["300_d.jsonl", "200_a.jsonl", "100_b.jsonl"]);
    }
}
