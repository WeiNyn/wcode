//! Interactive REPL: stdin line loop, `/commands`, event printing, Ctrl-C.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wcode_harness::compaction::{CompactOutcome, CompactionPolicy};
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::event::AgentEvent;
use wcode_harness::hooks::HooksSet;
use wcode_harness::limits::model_limit;
use wcode_harness::loop_::DEFAULT_MAX_TURNS;
use wcode_harness::message::{AgentMessage, ContentBlock, StopReason};
use wcode_harness::session::Session;
use wcode_harness::streamfn::{LlmEndpoint, LlmOpts, list_models, rig_stream_fn};

use crate::config::{HooksConfig, ToolsConfig};
use crate::rtk::RtkHooks;
use crate::tools::default_tools;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
/// System prompt derives from the registered tool set. The read/edit anchor
/// contract is constant; grep/find are named only when those tools are
/// actually registered (both are off by default — `bash` covers search).
fn system_prompt(tools: &ToolsConfig) -> String {
    let inspect = match (tools.grep, tools.find) {
        (true, true) => "read, grep and find",
        (true, false) => "read and grep",
        (false, true) => "read and find",
        (false, false) => "read",
    };
    format!(
        "You are wcode, a minimal coding agent working in the user's current directory. \
         Inspect with {inspect}; modify with edit and write; run anything else through bash. \
         read emits a 5-char anchor per line and edit targets lines by those anchors \
         (content-addressed, drift-proof). Be concise."
    )
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Exit,
    New,
    /// Some(id) = switch; None = list models (same as `/models`).
    Model(Option<String>),
    /// Some(filter) = substring filter; None = list all.
    Models(Option<String>),
    /// Some(level) = set; Some("-") = clear; None = show current.
    Effort(Option<String>),
    /// Some(path) = open that session; None = latest in the session dir.
    Resume(Option<String>),
    Sessions,
    /// Rebuild (`cargo build --bin wcode`) and re-exec into the same
    /// session. `no_session` = start fresh with `--no-session`.
    Reload {
        no_session: bool,
    },
    /// Print aggregate token usage for the current conversation.
    Usage,
    /// Summarize older messages now, optionally focused by <prompt>.
    Compact(Option<String>),
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
        "models" => Some(Command::Models(arg)),
        "effort" => Some(Command::Effort(arg)),
        "resume" => Some(Command::Resume(arg)),
        "sessions" if arg.is_none() => Some(Command::Sessions),
        "reload" => match arg.as_deref() {
            None => Some(Command::Reload { no_session: false }),
            Some("--no-session") => Some(Command::Reload { no_session: true }),
            _ => None,
        },
        "usage" => Some(Command::Usage),
        "compact" => Some(Command::Compact(arg)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Event printing (pure diff core)
// ---------------------------------------------------------------------------

/// Diffs streamed assistant snapshots into what still needs printing.
///
/// Text only ever grows (deltas append), so "new length minus printed
/// length" is a valid char-boundary suffix. Thinking is different: a
/// `ThinkingReplace` restates the whole block and can diverge from — shrink
/// or not extend — what was already printed, so the printer keeps the last
/// full thinking text and flags that case for a full re-render.
#[derive(Default)]
pub struct MessagePrinter {
    text_len: usize,
    last_thinking: String,
    /// Set when the latest update replaced thinking instead of appending to
    /// it: the UI closes the partially-streamed block and re-renders.
    thinking_replaced: bool,
    mid_line: bool,
}

impl MessagePrinter {
    /// A new message started: forget accumulated text/thinking state.
    pub fn reset(&mut self) {
        self.text_len = 0;
        self.last_thinking.clear();
        self.thinking_replaced = false;
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
        let think_out = if thinking.starts_with(&self.last_thinking) {
            // Monotonic stream, or a superset restate: the new thinking extends
            // what's already printed, so emit the new suffix.
            self.thinking_replaced = false;
            thinking[self.last_thinking.len()..].to_string()
        } else {
            // Wholesale replacement (shrunk or divergent text): the printed
            // prefix is stale — emit the full reasoning and flag a re-render.
            self.thinking_replaced = true;
            thinking.clone()
        };
        self.last_thinking = thinking;
        (text_out, think_out)
    }

    pub fn mid_line(&self) -> bool {
        self.mid_line
    }

    pub fn clear_mid_line(&mut self) {
        self.mid_line = false;
    }

    /// True when the latest `update` replaced thinking instead of appending
    /// to it — the UI then closes the partial block and re-renders.
    pub fn thinking_replaced(&self) -> bool {
        self.thinking_replaced
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
// Usage
// ---------------------------------------------------------------------------

/// Aggregate token usage across the assistant messages that reported it.
/// Cache counts absent per-call are treated as 0.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct UsageTotals {
    /// Assistant messages whose `usage` was Some.
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

/// Sum `Usage` over every assistant message in `messages` that carried one.
pub fn usage_totals(messages: &[AgentMessage]) -> UsageTotals {
    let mut t = UsageTotals::default();
    for m in messages {
        let AgentMessage::Assistant { usage: Some(u), .. } = m else {
            continue;
        };
        t.turns += 1;
        t.input_tokens += u.input_tokens;
        t.output_tokens += u.output_tokens;
        t.cache_read_tokens += u.cache_read_tokens.unwrap_or(0);
        t.cache_write_tokens += u.cache_write_tokens.unwrap_or(0);
    }
    t
}

/// Provider-reported input tokens of the most recent assistant message: how
/// full the context was on the last request. `None` until a turn reports usage.
pub fn last_input_tokens(messages: &[AgentMessage]) -> Option<u64> {
    messages.iter().rev().find_map(|m| match m {
        AgentMessage::Assistant { usage: Some(u), .. } => Some(u.input_tokens),
        _ => None,
    })
}

/// `/usage`: one line of totals for the current conversation, or a note when
/// no turn has reported usage yet.
pub fn format_usage(t: &UsageTotals) -> String {
    if t.turns == 0 {
        return "(no usage reported)".to_string();
    }
    let mut parts = vec![
        format!("{} turn{}", t.turns, if t.turns == 1 { "" } else { "s" }),
        format!("{} in", t.input_tokens),
        format!("{} out", t.output_tokens),
    ];
    if t.cache_read_tokens > 0 {
        parts.push(format!("{} cache read", t.cache_read_tokens));
    }
    if t.cache_write_tokens > 0 {
        parts.push(format!("{} cache write", t.cache_write_tokens));
    }
    format!("usage: {}", parts.join(", "))
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
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
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

/// Materialize the default hook set from config. Every built-in native hook
/// gets a slot here — rtk first — so future integrations each add one entry
/// (plus a field in [`HooksConfig`]) and nothing in the loop changes.
pub fn default_hooks(cfg: &HooksConfig) -> HooksSet {
    HooksSet::one(Arc::new(RtkHooks::new(cfg.rtk)))
}

pub fn build_agent(
    llm: LlmOpts,
    hooks: HooksSet,
    tools: &ToolsConfig,
    session: Option<Session>,
    context: Vec<AgentMessage>,
    compaction: CompactionPolicy,
) -> Agent {
    Agent::new(AgentConfig {
        system: system_prompt(tools),
        tools: default_tools(tools),
        llm,
        stream_fn: rig_stream_fn(),
        hooks,
        session,
        context,
        working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        max_turns: DEFAULT_MAX_TURNS,
        compaction,
    })
}

/// Re-exec argv for `/reload`: resume the session (or `--no-session`) and
/// forward the effective LLM opts so flag overrides survive the re-exec
/// (the session only records model/effort *changes*, not launch flags).
pub fn reload_args(llm: &LlmOpts, session: Option<&Path>, no_session: bool) -> Vec<String> {
    let mut args = Vec::new();
    if no_session || session.is_none() {
        args.push("--no-session".to_string());
    } else if let Some(p) = session {
        args.push("--resume".to_string());
        args.push(p.display().to_string());
    }
    args.push("--model".to_string());
    args.push(llm.model.clone());
    if let Some(url) = &llm.base_url {
        args.push("--base-url".to_string());
        args.push(url.clone());
    }
    args.push("--endpoint".to_string());
    args.push(
        match llm.endpoint {
            LlmEndpoint::Chat => "chat",
            LlmEndpoint::Responses => "responses",
        }
        .to_string(),
    );
    args.push("--effort".to_string());
    args.push(llm.effort.clone().unwrap_or_else(|| "-".to_string()));
    args
}

/// Cargo root for the rebuild. The running binary is authoritative: a
/// cargo-built `wcode` lives at `<root>/target/{debug,release}/wcode`, so
/// walking up from the exe finds the workspace that owns it. The cwd is a
/// fallback, in case the binary was copied out of `target/`.
fn build_dir() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        roots.push(dir.to_path_buf());
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    for root in roots {
        let mut d = root.as_path();
        loop {
            if d.join("Cargo.toml").exists() {
                return Some(d.to_path_buf());
            }
            match d.parent() {
                Some(p) => d = p,
                None => break,
            }
        }
    }
    None
}

/// `/reload [--no-session]`: `cargo build --bin wcode` first — a failed
/// build keeps the old binary running. On success re-exec the (possibly
/// replaced) binary with `--resume <current>` so the session continues.
/// Ctrl-C during the build kills it and stays in the REPL.
async fn reload(
    agent: &Agent,
    llm: &LlmOpts,
    no_session: bool,
    in_flight: &AtomicBool,
    cancel_slot: &Mutex<CancellationToken>,
) {
    use std::sync::atomic::Ordering;
    let Some(dir) = build_dir() else {
        eprintln!("reload: no Cargo.toml above cwd or binary");
        return;
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("reload: current exe: {e}");
            return;
        }
    };
    println!("rebuilding in {} ...", dir.display());
    // Build-scoped token: Ctrl-C during the build must not poison the
    // agent's own cancel token (shared with the next `run()`).
    let build_cancel = CancellationToken::new();
    *lock_cancel_slot(cancel_slot) = build_cancel.clone();
    in_flight.store(true, Ordering::SeqCst);
    let mut child = match tokio::process::Command::new("cargo")
        .arg("build")
        .arg("--bin")
        .arg("wcode")
        .current_dir(&dir)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            in_flight.store(false, Ordering::SeqCst);
            *lock_cancel_slot(cancel_slot) = agent.cancel_token();
            eprintln!("reload: cargo: {e}");
            return;
        }
    };
    let status = tokio::select! {
        biased;
        _ = build_cancel.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            None
        }
        s = child.wait() => s.ok(),
    };
    in_flight.store(false, Ordering::SeqCst);
    *lock_cancel_slot(cancel_slot) = agent.cancel_token();
    match status {
        Some(s) if s.success() => {}
        Some(s) => {
            eprintln!("reload: build failed ({s}); staying on current binary");
            return;
        }
        None => {
            println!("(reload cancelled)");
            return;
        }
    }
    let args = reload_args(llm, agent.session_path(), no_session);
    println!("reloading {} ...", exe.display());
    let _ = io::stdout().flush();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let err = std::process::Command::new(&exe).args(&args).exec();
        eprintln!("reload: exec: {err}");
    }
    #[cfg(not(unix))]
    {
        match std::process::Command::new(&exe).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => eprintln!("reload: spawn: {e}"),
        }
    }
}
/// `/models [filter]`: list `GET {base_url}/models` ids, `*` marks the
/// current model. Filter is a case-insensitive substring on the id.
async fn print_models(llm: &LlmOpts, filter: Option<&str>) {
    match list_models(llm).await {
        Ok(ids) => {
            let ids = ids
                .iter()
                .filter(|id| filter.is_none_or(|f| id.to_lowercase().contains(&f.to_lowercase())));
            let mut empty = true;
            for id in ids {
                empty = false;
                let mark = if *id == llm.model { "*" } else { " " };
                println!("{mark} {id}");
            }
            if empty {
                println!("(no models)");
            }
        }
        Err(e) => eprintln!("models: {e}"),
    }
}

// ---------------------------------------------------------------------------
// REPL
// ---------------------------------------------------------------------------

/// Lock the shared cancel slot, tolerating a poisoned mutex: a panic elsewhere
/// while the lock was held must not take down the REPL's Ctrl-C / run plumbing.
fn lock_cancel_slot(
    slot: &Mutex<CancellationToken>,
) -> std::sync::MutexGuard<'_, CancellationToken> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub async fn run(
    mut agent: Agent,
    mut llm: LlmOpts,
    hooks: HooksSet,
    tools: ToolsConfig,
    compaction: CompactionPolicy,
) {
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
                    lock_cancel_slot(&cancel_slot).cancel();
                } else {
                    println!();
                    std::process::exit(0);
                }
            }
        });
    }

    println!("wcode {} — model: {}", env!("CARGO_PKG_VERSION"), llm.model);
    match &llm.effort {
        Some(e) => println!("effort: {e}"),
        None => println!("(no effort)"),
    }
    if let Some(limit) = model_limit(llm.base_url.as_deref(), &llm.model) {
        println!("context: {} tokens", limit.context);
    }
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
                        let path = session
                            .as_ref()
                            .and_then(|s| s.path().map(Path::to_path_buf));
                        agent =
                            build_agent(
                                llm.clone(),
                                hooks.clone(),
                                &tools,
                                session,
                                Vec::new(),
                                compaction,
                            );
                        *lock_cancel_slot(&cancel_slot) = agent.cancel_token();
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
                // Bare `/model` lists available models (same as `/models`).
                None => print_models(&llm, None).await,
            },
            Some(Command::Models(filter)) => print_models(&llm, filter.as_deref()).await,
            Some(Command::Effort(arg)) => match arg.as_deref() {
                // `/effort` with no arg shows the current level.
                None => match &llm.effort {
                    Some(e) => println!("effort: {e}"),
                    None => println!("(no effort)"),
                },
                // `/effort -` clears back to send-nothing.
                Some("-") | Some("none") | Some("off") => match agent.set_effort(None) {
                    Ok(()) => {
                        llm.effort = None;
                        println!("(no effort)");
                    }
                    Err(e) => eprintln!("effort: {e}"),
                },
                Some(level) => match agent.set_effort(Some(level.to_string())) {
                    Ok(()) => {
                        llm.effort = Some(level.to_string());
                        println!("effort: {level}");
                    }
                    Err(e) => eprintln!("effort: {e}"),
                },
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
                        if let Some(e) = s.effort() {
                            llm.effort = e;
                        }
                        let n = messages.len();
                        agent = build_agent(
                            llm.clone(),
                            hooks.clone(),
                            &tools,
                            Some(s),
                            messages,
                            compaction,
                        );
                        *lock_cancel_slot(&cancel_slot) = agent.cancel_token();
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
                        let mark = if current == Some(p.as_path()) {
                            "*"
                        } else {
                            " "
                        };
                        println!(
                            "{mark} {}",
                            p.file_name().unwrap_or_default().to_string_lossy()
                        );
                    }
                }
                Err(e) => eprintln!("list sessions: {e}"),
            },
            Some(Command::Reload { no_session }) => {
                reload(&agent, &llm, no_session, &in_flight, &cancel_slot).await
            }
            Some(Command::Usage) => {
                println!("{}", format_usage(&usage_totals(agent.messages())));
                if let Some(limit) = model_limit(llm.base_url.as_deref(), &llm.model) {
                    let used = last_input_tokens(agent.messages()).unwrap_or(0);
                    let pct = used * 100 / limit.context.max(1);
                    println!("context: {used}/{} ({pct}%)", limit.context);
                }
            }
            Some(Command::Compact(arg)) => match agent.compact(arg.as_deref()).await {
                Ok(CompactOutcome::NothingToDo) => println!("(nothing to compact)"),
                Ok(CompactOutcome::Done {
                    summarized, kept, ..
                }) => println!("compacted: summarized {summarized}, kept {kept} (+ summary)"),
                Err(e) => eprintln!("compact: {e}"),
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
    *lock_cancel_slot(cancel_slot) = agent.cancel_token();
    let _ = printer.await;
    match res {
        Ok(StopReason::Aborted) => println!("{DIM}(aborted){RESET}"),
        Ok(StopReason::Error) => eprintln!("{DIM}✗ run failed{RESET}"),
        Ok(StopReason::MaxTurns) => println!("{DIM}(hit max turns){RESET}"),
        Ok(_) => {}
        Err(e) => eprintln!("{DIM}error: {e}{RESET}"),
    }
}

async fn print_events(mut rx: mpsc::UnboundedReceiver<AgentEvent>) {
    let mut p = MessagePrinter::default();
    let mut st = PrintState::default();
    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::MessageStart { .. } => {
                p.reset();
                // Each message owns its blocks from scratch.
                st = PrintState::default();
            }
            AgentEvent::MessageUpdate { message } => emit(&mut p, &message, &mut st),
            AgentEvent::MessageEnd { message } => {
                emit(&mut p, &message, &mut st);
                close_blocks(&mut p, &mut st);
            }
            AgentEvent::ToolExecutionStart { name, .. } => {
                // Break out of any open block so the tool call stands alone.
                close_blocks(&mut p, &mut st);
                // Newline after the marker: streamed ToolExecutionUpdate
                // partials (each ending in '\n') and the ✓/✗ line then render
                // on their own lines rather than gluing onto the ⚙ marker.
                out(&format!("{DIM}⚙ {name}{RESET}\n"));
            }
            AgentEvent::ToolExecutionUpdate { partial, .. } => out(&partial),
            AgentEvent::ToolExecutionEnd {
                output, is_error, ..
            } => {
                let mark = if is_error { "✗" } else { "✓" };
                let note = tool_output_note(&output);
                if note.is_empty() {
                    println!(" {mark}{RESET}");
                } else {
                    println!(" {mark} {DIM}{note}{RESET}");
                }
            }
            AgentEvent::Error { message } => {
                close_blocks(&mut p, &mut st);
                eprintln!("{DIM}error: {message}{RESET}");
                let _ = io::stderr().flush();
            }
            AgentEvent::Compaction { summarized, kept } => {
                close_blocks(&mut p, &mut st);
                out(&format!(
                    "{DIM}⋯ compacted {summarized} messages, kept {kept}{RESET}\n"
                ));
            }
            _ => {}
        }
    }
    close_blocks(&mut p, &mut st);
}

/// Render state distinguishing the output block types (thinking / tool call /
/// message) so each gets its own rows instead of running together.
#[derive(Default)]
struct PrintState {
    /// True while a streaming dim thinking block is open on stderr.
    thinking_open: bool,
}

/// Ends any open output blocks — an in-flight thinking block (stderr) and/or
/// mid-line message text (stdout) — so the next block starts on a fresh line.
fn close_blocks(p: &mut MessagePrinter, st: &mut PrintState) {
    if st.thinking_open {
        eprintln!();
        st.thinking_open = false;
    }
    if p.mid_line() {
        println!();
        p.clear_mid_line();
    }
}

fn emit(p: &mut MessagePrinter, message: &AgentMessage, st: &mut PrintState) {
    let (text, thinking) = p.update(message);
    if st.thinking_open && p.thinking_replaced() {
        // Thinking was replaced wholesale (shrunk, or restated with a
        // divergent prefix): the partially-streamed line is stale. Close it;
        // the block below re-renders the full reasoning on a fresh line.
        eprintln!();
        st.thinking_open = false;
    }
    if !thinking.is_empty() {
        if !st.thinking_open {
            // Start a thinking block on its own line (breaking out of any
            // mid-line text) so it is identified apart from the message.
            if p.mid_line() {
                println!();
                p.clear_mid_line();
            }
            eprint!("{DIM}··· ");
            st.thinking_open = true;
        }
        eprint!("{DIM}{thinking}{RESET}");
        let _ = io::stderr().flush();
    }
    if !text.is_empty() {
        if st.thinking_open {
            // Thinking is done: close the block on its own line before the
            // message text continues.
            eprintln!();
            st.thinking_open = false;
        }
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
    use wcode_harness::message::{StopReason, Usage};

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
        assert_eq!(parse_command("/models"), Some(Command::Models(None)));
        assert_eq!(
            parse_command("/models gpt"),
            Some(Command::Models(Some("gpt".into())))
        );
        assert_eq!(parse_command("/effort"), Some(Command::Effort(None)));
        assert_eq!(
            parse_command("/effort high"),
            Some(Command::Effort(Some("high".into())))
        );
        assert_eq!(
            parse_command("/effort -"),
            Some(Command::Effort(Some("-".into())))
        );
        assert_eq!(parse_command("/resume"), Some(Command::Resume(None)));
        assert_eq!(
            parse_command("/resume /tmp/s.jsonl"),
            Some(Command::Resume(Some("/tmp/s.jsonl".into())))
        );
        assert_eq!(parse_command("/sessions"), Some(Command::Sessions));
        assert_eq!(parse_command("/sessions now"), None);
        assert_eq!(
            parse_command("/reload"),
            Some(Command::Reload { no_session: false })
        );
        assert_eq!(
            parse_command("/reload --no-session"),
            Some(Command::Reload { no_session: true })
        );
        assert_eq!(parse_command("/reload foo"), None);
        assert_eq!(parse_command("/usage"), Some(Command::Usage));
        assert_eq!(parse_command("/compact"), Some(Command::Compact(None)));
        assert_eq!(
            parse_command("/compact focus on the API"),
            Some(Command::Compact(Some("focus on the API".into())))
        );
    }

    #[test]
    fn system_prompt_names_only_registered_inspect_tools() {
        let off = system_prompt(&ToolsConfig::default());
        assert!(off.contains("Inspect with read;"), "{off}");
        assert!(!off.contains("grep") && !off.contains("find"));

        let on = system_prompt(&ToolsConfig {
            grep: true,
            find: true,
        });
        assert!(on.contains("Inspect with read, grep and find;"), "{on}");

        let only_grep = system_prompt(&ToolsConfig {
            grep: true,
            find: false,
        });
        assert!(
            only_grep.contains("Inspect with read and grep;"),
            "{only_grep}"
        );
    }

    #[test]
    fn reload_args_resume_and_forward_opts() {
        let llm = LlmOpts {
            model: "gpt-x".to_string(),
            base_url: Some("http://x/v1".to_string()),
            api_key: None,
            temperature: None,
            endpoint: LlmEndpoint::Chat,
            effort: Some("high".to_string()),
            session_id: None,
        };
        assert_eq!(
            reload_args(&llm, Some(Path::new("/s/a.jsonl")), false),
            vec![
                "--resume",
                "/s/a.jsonl",
                "--model",
                "gpt-x",
                "--base-url",
                "http://x/v1",
                "--endpoint",
                "chat",
                "--effort",
                "high",
            ]
        );
    }

    #[test]
    fn reload_args_no_session_and_defaults() {
        let llm = LlmOpts {
            model: "m".to_string(),
            ..LlmOpts::default()
        };
        // explicit flag wins, even with a session open
        assert_eq!(
            reload_args(&llm, Some(Path::new("/s/a.jsonl")), true)[..2],
            ["--no-session".to_string(), "--model".to_string()],
        );
        // no session file: fresh start, cleared effort round-trips as "-"
        let args = reload_args(&llm, None, false);
        assert_eq!(args[0], "--no-session");
        assert!(args.windows(2).any(|w| w == ["--effort", "-"]));
        assert!(args.windows(2).any(|w| w == ["--endpoint", "chat"]));
        assert!(!args.iter().any(|a| a == "--base-url"));
    }

    #[test]
    fn lock_cancel_slot_tolerates_poisoning() {
        let m = Mutex::new(CancellationToken::new());
        // Poison the mutex by panicking while the lock is held.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = m.lock().unwrap();
            panic!("poison on purpose");
        }));
        // The helper must still hand back the guard rather than panicking.
        let guard = lock_cancel_slot(&m);
        guard.cancel();
        assert!(guard.is_cancelled());
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
            p.update(&assistant(vec![ContentBlock::Text { text: "a".into() }])),
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
        p.update(&assistant(vec![ContentBlock::Thinking {
            text: "t".into(),
        }]));
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
    fn printer_reports_thinking_replace_and_rerenders_full() {
        let mut p = MessagePrinter::default();
        // Streamed deltas, then a divergent (shorter) wholesale restate.
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::Thinking {
                text: "abcdefgh".into()
            }])),
            (String::new(), "abcdefgh".to_string())
        );
        assert!(!p.thinking_replaced());
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::Thinking {
                text: "short".into()
            }])),
            (String::new(), "short".to_string())
        );
        assert!(
            p.thinking_replaced(),
            "a divergent replacement must be flagged for a fresh line"
        );
    }

    #[test]
    fn printer_thinking_delta_resumes_from_replaced_text() {
        let mut p = MessagePrinter::default();
        p.update(&assistant(vec![ContentBlock::Thinking {
            text: "long text".into(),
        }]));
        p.update(&assistant(vec![ContentBlock::Thinking {
            text: "short".into(),
        }]));
        // Deltas after a replace continue from the NEW text, not the old.
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::Thinking {
                text: "short but".into()
            }])),
            (String::new(), " but".to_string())
        );
        assert!(!p.thinking_replaced());
    }

    #[test]
    fn printer_superset_restate_is_plain_append() {
        // A complete-block restate that *extends* the already-printed text is
        // visually just an append — no re-render needed.
        let mut p = MessagePrinter::default();
        p.update(&assistant(vec![ContentBlock::Thinking {
            text: "abc".into(),
        }]));
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::Thinking {
                text: "abcxyz".into()
            }])),
            (String::new(), "xyz".to_string())
        );
        assert!(!p.thinking_replaced());
    }

    #[test]
    fn printer_reset_clears_thinking_replacement_state() {
        let mut p = MessagePrinter::default();
        p.update(&assistant(vec![ContentBlock::Thinking {
            text: "old".into(),
        }]));
        p.update(&assistant(vec![ContentBlock::Thinking {
            text: "x".into(),
        }]));
        assert!(p.thinking_replaced());
        p.reset();
        assert!(!p.thinking_replaced());
        // A fresh message starts streaming like new.
        assert_eq!(
            p.update(&assistant(vec![ContentBlock::Thinking {
                text: "new".into()
            }])),
            (String::new(), "new".to_string())
        );
        assert!(!p.thinking_replaced());
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

    #[test]
    fn last_input_tokens_takes_most_recent_reported_usage() {
        let messages = vec![
            assistant_with_usage(10, 20, None, None),
            AgentMessage::user_text("q"),
            assistant_with_usage(41, 3, None, None),
            assistant(vec![]), // no usage: skipped
        ];
        assert_eq!(last_input_tokens(&messages), Some(41));
        assert_eq!(last_input_tokens(&[]), None);
        assert_eq!(last_input_tokens(&[AgentMessage::user_text("q")]), None);
    }

    fn assistant_with_usage(
        input: u64,
        output: u64,
        cache_read: Option<u64>,
        cache_write: Option<u64>,
    ) -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::Text { text: "a".into() }],
            stop_reason: StopReason::Stop,
            usage: Some(Usage {
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
            }),
            model: None,
        }
    }

    #[test]
    fn usage_totals_sums_reported_usage_only() {
        let messages = vec![
            AgentMessage::user_text("q"),
            assistant_with_usage(10, 20, Some(3), None),
            assistant(
                // no usage reported
                vec![ContentBlock::Text { text: "x".into() }],
            ),
            assistant_with_usage(30, 40, None, Some(5)),
        ];
        let t = usage_totals(&messages);
        assert_eq!(
            t,
            UsageTotals {
                turns: 2,
                input_tokens: 40,
                output_tokens: 60,
                cache_read_tokens: 3,
                cache_write_tokens: 5,
            }
        );
    }

    #[test]
    fn usage_totals_empty_and_no_usage() {
        assert_eq!(usage_totals(&[]), UsageTotals::default());
        assert_eq!(
            usage_totals(&[assistant(vec![]), AgentMessage::user_text("q")]),
            UsageTotals::default()
        );
    }

    #[test]
    fn format_usage_lines_and_empty() {
        let empty = UsageTotals::default();
        assert_eq!(format_usage(&empty), "(no usage reported)");

        let t = UsageTotals {
            turns: 2,
            input_tokens: 40,
            output_tokens: 60,
            cache_read_tokens: 3,
            cache_write_tokens: 5,
        };
        assert_eq!(
            format_usage(&t),
            "usage: 2 turns, 40 in, 60 out, 3 cache read, 5 cache write"
        );

        // No cache hits: omit the cache fields entirely.
        let t = UsageTotals {
            turns: 1,
            input_tokens: 10,
            output_tokens: 20,
            ..UsageTotals::default()
        };
        assert_eq!(format_usage(&t), "usage: 1 turn, 10 in, 20 out");
    }
}
