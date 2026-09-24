//! Interactive REPL: stdin line loop, `/commands`, event printing, Ctrl-C.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::AsyncBufReadExt;
use tokio::sync::broadcast;
use wcode_harness::actor::SessionActor;
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::event::{AgentEvent, TodoItem, TodoStatus};
use wcode_harness::hooks::{BashRiskHooks, HooksSet, PlanModeHandle, PlanModeHooks};
use wcode_harness::limits::model_limit;
use wcode_harness::loop_::DEFAULT_MAX_TURNS;
use wcode_harness::message::{AgentMessage, ContentBlock, StopReason};
use wcode_harness::protocol::Request;
use wcode_harness::session::Session;
use wcode_harness::stats::session_stats;
use wcode_harness::streamfn::{LlmEndpoint, LlmOpts, list_models, rig_stream_fn};
use wcode_harness::tool::Tool;
use wcode_protocol::Backend;
#[cfg(unix)]
use wcode_protocol::Client;

use crate::config::{HooksConfig, TeamMember, ToolsConfig};
use crate::instructions::InstructionSet;
use crate::skills::SkillSet;
use crate::rtk::RtkHooks;
use crate::tools::background::Background;
use crate::tools::default_tools;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// System prompt derives from the registered tool set. The read/edit anchor
/// contract is constant; grep/find are named only when those tools are
/// actually registered (both are off by default — `bash` covers search).
pub fn system_prompt(
    tools: &ToolsConfig,
    instructions: &InstructionSet,
    skills: &SkillSet,
    team: &[TeamMember],
    guidelines: Option<&str>,
    cwd: &Path,
) -> String {
    let inspect = match (tools.grep, tools.find) {
        (true, true) => "read, grep and find",
        (true, false) => "read and grep",
        (false, true) => "read and find",
        (false, false) => "read",
    };
    let base = format!(
        "You are wcode, a minimal coding agent working in the user's current directory. \
         Inspect with {inspect}; modify with edit and write; run anything else through bash. \
         read emits a 5-char anchor per line and edit targets lines by those anchors \
         (content-addressed, drift-proof). Be concise."
    );
    let mut prompt = match instructions.render() {
        Some(block) => format!("{base}\n\n{block}"),
        None => base,
    };
    if let Some(section) = skills.render(cwd) {
        prompt.push_str("\n\n");
        prompt.push_str(&section);
    }
    // The team block is rendered only when the root actually has a team (F3):
    // a teamless orchestrator's prompt is byte-identical to before.
    if !team.is_empty() {
        let mut block = String::from(
            "# Your team\n\n\
             You orchestrate a team of worker agents. Address a member by name with \
             `message` (`to: \"<name>\"`); `spawn` adds a worker; `peers` lists them. \
             Delegate by role and prefer messaging an existing member over spawning a \
             duplicate.\n\nMembers:",
        );
        for member in team {
            block.push_str("\n- ");
            block.push_str(&member.name);
            if let Some(role) = &member.role {
                block.push_str(" — ");
                block.push_str(role);
            }
        }
        prompt.push_str("\n\n");
        prompt.push_str(&block);
    }
    // Root-only workflow guidance (F3b): after the roster, so it can reference
    // the team. Empty/whitespace adds nothing.
    if let Some(g) = guidelines.filter(|g| !g.trim().is_empty()) {
        prompt.push_str("\n\n# Orchestrator workflow\n\n");
        prompt.push_str(g.trim());
    }
    prompt
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
    /// Ask a tool-free side question (`/btw <question>`): answered from the
    /// current context, NEVER recorded in ctx or the session.
    Btw(String),
    /// Toggle plan mode (`/plan` = toggle; `/plan on|off` = set explicitly).
    Plan(Option<bool>),
    /// `/verify`: render the plan's progress — the cached todo checklist, the
    /// done count, and whether anything is unfinished.
    Verify,
    /// Print aggregate token usage for the current conversation.
    Usage,
    /// Summarize older messages now, optionally focused by <prompt>.
    Compact(Option<String>),
    /// List the discovered skills (`/skills`).
    Skills,
    /// `/skill <name> [args]` — force-load a skill's body as a turn, for when
    /// the model does not pick it up from the prompt section on its own.
    Skill(Option<String>),
    /// `/theme <name>` selects a preset live; bare `/theme` lists the names.
    Theme(Option<String>),
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
        "theme" => Some(Command::Theme(arg)),
        "resume" => Some(Command::Resume(arg)),
        "sessions" if arg.is_none() => Some(Command::Sessions),
        "reload" => match arg.as_deref() {
            None => Some(Command::Reload { no_session: false }),
            Some("--no-session") => Some(Command::Reload { no_session: true }),
            _ => None,
        },
        "btw" => Some(Command::Btw(arg.unwrap_or_default())),
        "plan" => Some(Command::Plan(match arg.as_deref() {
            None => None, // bare `/plan` toggles
            Some("on") => Some(true),
            Some("off") => Some(false),
            _ => return None, // unknown arg → prompt text
        })),
        "verify" => Some(Command::Verify),
        "usage" => Some(Command::Usage),
        "compact" => Some(Command::Compact(arg)),
        "skills" => Some(Command::Skills),
        "skill" => Some(Command::Skill(arg)),
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
// Sessions
// ---------------------------------------------------------------------------

pub fn session_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/wcode/sessions")
}

/// Prompt-history file, alongside the session files.
pub fn history_path() -> PathBuf {
    session_dir().join("history")
}

/// Session files, newest first. Names are `{millis}_{hex}.jsonl`, so
/// lexicographic order is chronological.
/// Resumable session paths, newest first: one path per **session group** (its
/// dir) or **legacy** flat file — the flat view the `/resume` default, `/sessions`
/// and `main`'s latest-session lookup consume. Names are `{millis}_{hex}`, so
/// lexicographic order is chronological. Thin wrapper over
/// [`crate::session_groups::list_groups`], which owns the listing rule (member
/// files are never listed).
pub fn list_sessions(dir: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(crate::session_groups::list_groups(dir)?
        .into_iter()
        .map(|entry| entry.path().to_path_buf())
        .collect())
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
/// gets a slot here — the bash risk gate (always on) first, then rtk — so
/// future integrations each add one entry (plus a field in [`HooksConfig`]) and
/// nothing in the loop changes.
pub fn default_hooks(cfg: &HooksConfig) -> HooksSet {
    let mut hooks = HooksSet::new();
    hooks.push(Arc::new(BashRiskHooks::new())); // safety gate first
    hooks.push(Arc::new(RtkHooks::new(cfg.rtk))); // rtk proxy second
    hooks
}

/// The agent configuration that is fixed for a run; `/new` and `/resume`
/// rebuild the agent from this plus a fresh session and context.
pub struct AgentSpec<'a> {
    pub llm: LlmOpts,
    pub hooks: HooksSet,
    pub tools: &'a ToolsConfig,
    pub compaction: CompactionPolicy,
    pub instructions: &'a InstructionSet,
    pub skills: &'a SkillSet,
    pub team: &'a [TeamMember],
    pub guidelines: Option<&'a str>,
}

pub fn build_agent(
    spec: AgentSpec<'_>,
    session: Option<Session>,
    context: Vec<AgentMessage>,
    extra_tools: Vec<Tool>,
    digest_cas: bool,
) -> (Agent, Arc<Background>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let bg = Background::new();
    let mut tools = default_tools(spec.tools, &session_dir(), bg.clone());
    tools.extend(extra_tools);
    // One handle shared by the `PlanModeHooks` and the agent, so `set_plan_mode`
    // flips both. Created here (per build) — see the doc's toggle flag.
    let plan_mode = PlanModeHandle::new();
    // A FRESH WorkspaceHooks per agent (per-session digest cache). It must NOT
    // live in the shared `spec.hooks` set: `/new` and `/resume` rebuild the
    // agent in-process from that shared set, so a shared instance would either
    // vanish or leak one session's cache into another (reviewer #1).
    let mut hooks = spec.hooks;
    hooks.push(std::sync::Arc::new(crate::workspace::WorkspaceHooks::new(
        digest_cas,
    )));
    hooks.push(std::sync::Arc::new(PlanModeHooks::new(plan_mode.clone())));
    let agent = Agent::new(AgentConfig {
        system: system_prompt(
            spec.tools,
            spec.instructions,
            spec.skills,
            spec.team,
            spec.guidelines,
            &cwd,
        ),
        tools,
        llm: spec.llm,
        stream_fn: rig_stream_fn(),
        hooks,
        session,
        context,
        working_dir: cwd,
        max_turns: DEFAULT_MAX_TURNS,
        parallel_tools: spec.tools.parallel.unwrap_or(true),
        compaction: spec.compaction,
        plan_mode,
    });
    (agent, bg)
}

/// Re-exec argv for `/reload`: resume the session (or `--no-session`) and
/// forward the effective LLM opts so flag overrides survive the re-exec
/// (the session only records model/effort *changes*, not launch flags).
pub fn reload_args(
    llm: &LlmOpts,
    session: Option<&Path>,
    no_session: bool,
    agents: bool,
    config: Option<&str>,
    owner: Option<&str>,
    name: Option<&str>,
) -> Vec<String> {
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
    // Keep the orchestrator/team across a re-exec: a resumed session must still
    // satisfy the `[team] requires --agents` guard (D16).
    if agents {
        args.push("--agents".to_string());
    }
    // Forward a flag-supplied config overlay; a `WCODE_CONFIG` overlay survives
    // on its own (the environment is inherited across the re-exec).
    if let Some(path) = config {
        args.push("--config".to_string());
        args.push(path.to_string());
    }
    // A served worker (`--owner`) resumed here must come back as a worker:
    // forward the ownership edge and its worker id so the re-exec'd `main`
    // rebuilds the worker branch instead of collapsing to a root.
    if let Some(addr) = owner {
        args.push("--owner".to_string());
        args.push(addr.to_string());
    }
    if let Some(id) = name {
        args.push("--name".to_string());
        args.push(id.to_string());
    }
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
/// build is not specially cancellable — Ctrl-C is a developer-session
/// non-case, so a build is simply awaited to completion.
#[allow(clippy::too_many_arguments)]
pub async fn reload(
    llm: &LlmOpts,
    session: Option<&Path>,
    no_session: bool,
    agents: bool,
    overlay: Option<&str>,
    owner: Option<&str>,
    name: Option<&str>,
    in_flight: Option<&AtomicBool>,
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
    // Swallow Ctrl-C for the duration of the build (`in_flight` routes it to
    // the actor as a `Cancel`, a no-op while idle) so a stray Ctrl-C does not
    // kill the session mid-build.
    if let Some(f) = in_flight {
        f.store(true, Ordering::SeqCst);
    }
    let status = tokio::process::Command::new("cargo")
        .arg("build")
        .arg("--bin")
        .arg("wcode")
        .current_dir(&dir)
        .status()
        .await;
    if let Some(f) = in_flight {
        f.store(false, Ordering::SeqCst);
    }
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("reload: build failed ({s}); staying on current binary");
            return;
        }
        Err(e) => {
            eprintln!("reload: cargo: {e}");
            return;
        }
    }
    let args = reload_args(llm, session, no_session, agents, overlay, owner, name);
    println!("reloading {} ...", exe.display());
    let _ = io::stdout().flush();
    exec_self(&args);
}
/// Replace the current process with this binary re-invoked as `args` — the
/// `/reload` and `/resume` handoff. Returns only if the exec failed; the caller
/// decides what to do then.
pub fn exec_self(args: &[String]) {
    // `exec` replaces the process image — no destructors run — so every live
    // background group must be signalled HERE first (D8).
    Background::shutdown_all();
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("exec: current exe: {e}");
            return;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let err = std::process::Command::new(&exe).args(args).exec();
        eprintln!("exec: {err}");
    }
    #[cfg(not(unix))]
    {
        match std::process::Command::new(&exe).args(args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => eprintln!("exec: spawn: {e}"),
        }
    }
}

/// Print the exact relaunch command for the current session on a clean
/// interactive exit. A remote client (or `--no-session`) owns no local path, so
/// nothing is printed — a bogus local command would be worse than silence.
/// Mirrors the startup banner's `session: <path>` style.
pub fn print_relaunch(
    llm: &LlmOpts,
    session: Option<&Path>,
    agents: bool,
    overlay: Option<&str>,
    owner: Option<&str>,
    name: Option<&str>,
) {
    let Some(session) = session else {
        return;
    };
    let args = reload_args(llm, Some(session), false, agents, overlay, owner, name);
    println!("resume: {}", relaunch_line(&args));
}

/// The relaunch command as a copy-pasteable shell line: the invoked program
/// (`argv[0]`, else `wcode`) plus the shell-quoted [`reload_args`].
pub fn relaunch_line(args: &[String]) -> String {
    let program = std::env::args_os()
        .next()
        .map(|a| a.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "wcode".to_string());
    let mut line = shell_quote(&program);
    for arg in args {
        line.push(' ');
        line.push_str(&shell_quote(arg));
    }
    line
}

/// POSIX-shell-safe `arg`: as-is when it is already safe, else single-quoted
/// (escaping any embedded `'`).
fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'-' | b'_' | b'.' | b'/' | b'=' | b':' | b'@' | b',' | b'+')
        });
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
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

/// Lock a shared slot, tolerating a poisoned mutex: a panic elsewhere while the
/// lock was held must not take down the REPL's Ctrl-C / run plumbing.
fn lock_slot<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Await `f` unless the idle-Ctrl-C signal fires first (`None`). The REPL's
/// prompt read races this: a signal leaves the loop so its after-loop relaunch
/// line prints, exactly like `/exit`/EOF. An in-flight run is unaffected (its
/// Ctrl-C routes to the actor as a `Cancel`, not this signal).
async fn unless_idle_ctrl_c<F: std::future::Future>(
    signal: &tokio::sync::Notify,
    f: F,
) -> Option<F::Output> {
    tokio::select! {
        _ = signal.notified() => None,
        out = f => Some(out),
    }
}

/// Where a REPL session comes from: a local agent to hand to the actor, or a
/// client already connected to a session served elsewhere.
pub enum SessionSource {
    /// Boxed: the enum is small when a `Client` (an `Arc`) is the variant.
    Local(Box<Agent>, Arc<Background>),
    #[cfg(unix)]
    Remote(Client),
}

// The rebuild inputs (llm/hooks/tools/…/orchestrator) are threaded positionally;
// grouping them into a struct would not earn its keep yet.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    source: SessionSource,
    mut llm: LlmOpts,
    hooks: HooksSet,
    tools: ToolsConfig,
    compaction: CompactionPolicy,
    instructions: InstructionSet,
    skills: SkillSet,
    team: &[TeamMember],
    guidelines: Option<&str>,
    overlay: Option<&str>,
    owner: Option<&str>,
    name: Option<&str>,
    orchestrator: Option<crate::agents::Orchestrator>,
    digest_cas: bool,
) {
    #[cfg(unix)]
    let remote = matches!(source, SessionSource::Remote(_));
    #[cfg(not(unix))]
    let remote = false;

    let in_flight = Arc::new(AtomicBool::new(false));
    let mut session_path;
    let mut backend;
    // The Local agent's background registry+notifier, kept for the `/new`/
    // `/resume` rebuild arms (a `Remote` source has none).
    let mut bg: Option<Arc<Background>> = None;
    match source {
        SessionSource::Local(agent, background) => {
            session_path = agent.session_path().map(Path::to_path_buf);
            let handle = SessionActor::spawn(*agent);
            background.bind(handle.clone());
            bg = Some(background);
            if let Some(o) = &orchestrator {
                o.register_root(handle.clone());
            }
            backend = Backend::from(handle);
        }
        #[cfg(unix)]
        SessionSource::Remote(client) => {
            session_path = None;
            backend = Backend::from(client);
        }
    }
    // Ctrl-C lives on a separate task. The slot holds the *current* backend
    // (swapped by `/new`/`/resume`) so Ctrl-C always reaches the live run; when
    // idle it ends the loop (so the relaunch line still prints).
    // Idle Ctrl-C ends the loop (rather than exiting in the task) so the
    // after-loop relaunch line prints there too; the task signals via this
    // `Notify`.
    let ctrl_c_exit = Arc::new(tokio::sync::Notify::new());
    let backend_slot: Arc<Mutex<Backend>> = Arc::new(Mutex::new(backend.clone()));
    {
        let in_flight = in_flight.clone();
        let ctrl_c_exit = ctrl_c_exit.clone();
        let backend_slot = backend_slot.clone();
        tokio::spawn(async move {
            while tokio::signal::ctrl_c().await.is_ok() {
                if in_flight.load(Ordering::SeqCst) {
                    let _ = lock_slot(&backend_slot).send(Request::Cancel);
                } else {
                    println!();
                    // Break the main loop (it prints the relaunch line once).
                    ctrl_c_exit.notify_one();
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
    match &session_path {
        Some(p) => println!("session: {}", p.display()),
        None => println!("(no session)"),
    }

    for instr in &instructions.files {
        let scope = if instr.global { "global" } else { "project" };
        println!("{scope} instructions: {}", instr.path.display());
    }
    if !skills.is_empty() {
        println!("skills: {}", skills.skills.len());
    }

    if remote {
        replay(&backend).await;
    }

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    // Local mirror of plan mode (the REPL may be a socket client, so it does not
    // hold the agent's handle). Updated on a successful `SetPlanMode`; reset when
    // `/new`/`/resume` rebuild the agent (which starts with plan mode off).
    let mut plan = false;
    // The latest `AgentEvent::Todo` from a run, cached for `/verify` (a client
    // render of the event — NOT `Session::todo()`, whose own-session write the
    // agent's in-memory entries never see). Shared with the printer task.
    let last_todos: Arc<Mutex<Option<Vec<TodoItem>>>> = Arc::new(Mutex::new(None));
    loop {
        print!("❯ ");
        let _ = io::stdout().flush();
        let Some(line) = unless_idle_ctrl_c(&ctrl_c_exit, lines.next_line()).await else {
            break; // idle Ctrl-C: leave the loop normally (the relaunch line prints)
        };
        let Ok(Some(line)) = line else {
            break; // EOF or stdin error: exit
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_command(line) {
            Some(Command::Exit) => break,
            Some(Command::New) if remote => {
                println!(
                    "{DIM}/new is unavailable over a socket (the server owns the session){RESET}"
                )
            }
            Some(Command::New) => {
                // A session boundary: signal the outgoing agent's groups (D8).
                if let Some(old) = &bg {
                    old.shutdown();
                }
                let session = session_path
                    .is_some()
                    .then(|| Session::create(&session_dir()))
                    .transpose();
                match session {
                    Ok(session) => {
                        let path = session
                            .as_ref()
                            .and_then(|s| s.path().map(Path::to_path_buf));
                        let (new_agent, new_bg) = build_agent(
                            AgentSpec {
                                llm: llm.clone(),
                                hooks: hooks.clone(),
                                tools: &tools,
                                compaction,
                                instructions: &instructions,
                                skills: &skills,
                                team,
                                guidelines,
                            },
                            session,
                            Vec::new(),
                            orchestrator
                                .as_ref()
                                .map(|o| o.tools())
                                .unwrap_or_default(),
                            digest_cas,
                        );
                        let handle = SessionActor::spawn(new_agent);
                        new_bg.bind(handle.clone());
                        bg = Some(new_bg);
                        if let Some(o) = &orchestrator {
                            o.register_root(handle.clone());
                        }
                        backend = Backend::from(handle);
                        *lock_slot(&backend_slot) = backend.clone();
                        session_path = path;
                        plan = false; // the fresh agent starts with plan mode off
                        match &session_path {
                            Some(p) => println!("new session: {}", p.display()),
                            None => println!("new conversation"),
                        }
                    }
                    Err(e) => eprintln!("create session: {e}"),
                }
            }
            Some(Command::Theme(Some(name))) => {
                if wcode_tui::theme_names().iter().any(|n| n == &name) {
                    let _ = wcode_tui::set_theme(&name); // the ONE runtime entry
                    println!("theme: {name}");
                } else {
                    println!("unknown theme: {name}");
                }
            }
            Some(Command::Theme(None)) => println!("{}", wcode_tui::theme_names().join(", ")),
            Some(Command::Model(arg)) => match arg {
                Some(id) => match backend.ask(Request::SetModel { model: id.clone() }).await {
                    Ok(AgentEvent::Ack) => {
                        llm.model = id;
                        println!("model: {}", llm.model);
                    }
                    Ok(AgentEvent::Error { message }) => eprintln!("model: {message}"),
                    Ok(_) => eprintln!("model: unexpected reply"),
                    Err(_) => eprintln!("model: session closed"),
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
                Some("-") | Some("none") | Some("off") => {
                    match backend.ask(Request::SetEffort { effort: None }).await {
                        Ok(AgentEvent::Ack) => {
                            llm.effort = None;
                            println!("(no effort)");
                        }
                        Ok(AgentEvent::Error { message }) => eprintln!("effort: {message}"),
                        Ok(_) => eprintln!("effort: unexpected reply"),
                        Err(_) => eprintln!("effort: session closed"),
                    }
                }
                Some(level) => {
                    match backend
                        .ask(Request::SetEffort {
                            effort: Some(level.to_string()),
                        })
                        .await
                    {
                        Ok(AgentEvent::Ack) => {
                            llm.effort = Some(level.to_string());
                            println!("effort: {level}");
                        }
                        Ok(AgentEvent::Error { message }) => eprintln!("effort: {message}"),
                        Ok(_) => eprintln!("effort: unexpected reply"),
                        Err(_) => eprintln!("effort: session closed"),
                    }
                }
            },
            Some(Command::Resume(_)) if remote => {
                println!(
                    "{DIM}/resume is unavailable over a socket (the server owns the session){RESET}"
                )
            }
            Some(Command::Resume(arg)) => {
                // A session boundary: signal the outgoing agent's groups (D8).
                if let Some(old) = &bg {
                    old.shutdown();
                }
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
                if let Some(dir) = crate::session_groups::group_dir_of(&path) {
                    // A session group: resume the root and rebuild the whole team
                    // through the orchestrator's own factory/phonebook (6a).
                    match crate::session_groups::open_group(&dir) {
                        Ok(group) => match crate::session_groups::load_root(&group) {
                            Ok((s, messages)) => {
                                if let Some(m) = s.model() {
                                    llm.model = m;
                                }
                                if let Some(e) = s.effort() {
                                    llm.effort = e;
                                }
                                if let Some(o) = &orchestrator {
                                    let mut names = Vec::new();
                                    let mut seeded = 0usize;
                                    for outcome in
                                        crate::session_groups::rebuild_team(&group, o, o.id())
                                    {
                                        match outcome {
                                            crate::session_groups::MemberResume::Restored {
                                                id,
                                                messages,
                                            } => {
                                                seeded += messages;
                                                names.push(crate::agents::short_name(&id));
                                            }
                                            crate::session_groups::MemberResume::Skipped {
                                                name,
                                                reason,
                                            } => {
                                                eprintln!(
                                                    "warning: member `{name}` not resumed: {reason}"
                                                )
                                            }
                                        }
                                    }
                                    if !names.is_empty() {
                                        println!(
                                            "team: {} (restored, {seeded} prior messages)",
                                            names.join(", ")
                                        );
                                    }
                                }
                                let n = messages.len();
                                let (new_agent, new_bg) = build_agent(
                                    AgentSpec {
                                        llm: llm.clone(),
                                        hooks: hooks.clone(),
                                        tools: &tools,
                                        compaction,
                                        instructions: &instructions,
                                        skills: &skills,
                                        team,
                                        guidelines,
                                    },
                                    Some(s),
                                    messages,
                                    orchestrator
                                        .as_ref()
                                        .map(|o| o.tools())
                                        .unwrap_or_default(),
                                    digest_cas,
                                );
                                let handle = SessionActor::spawn(new_agent);
                                new_bg.bind(handle.clone());
                                bg = Some(new_bg);
                                if let Some(o) = &orchestrator {
                                    o.register_root(handle.clone());
                                }
                                backend = Backend::from(handle);
                                *lock_slot(&backend_slot) = backend.clone();
                                // `root.jsonl` is the live session path: `/reload`
                                // re-execs `--resume <dir>/root.jsonl`, mapped back
                                // up to the group on the way in (amendment 6b).
                                session_path = Some(group.root().to_path_buf());
                                plan = false; // the rebuilt agent starts with plan mode off
                                println!("resumed {} ({n} messages)", group.root().display());
                            }
                            Err(e) => eprintln!("open {}: {e}", group.root().display()),
                        },
                        Err(e) => eprintln!("open group {}: {e}", dir.display()),
                    }
                } else {
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
                        let (new_agent, new_bg) = build_agent(
                            AgentSpec {
                                llm: llm.clone(),
                                hooks: hooks.clone(),
                                tools: &tools,
                                compaction,
                                instructions: &instructions,
                                skills: &skills,
                                team,
                                guidelines,
                            },
                            Some(s),
                            messages,
                            orchestrator
                                .as_ref()
                                .map(|o| o.tools())
                                .unwrap_or_default(),
                            digest_cas,
                        );
                        let handle = SessionActor::spawn(new_agent);
                        new_bg.bind(handle.clone());
                        bg = Some(new_bg);
                        if let Some(o) = &orchestrator {
                            o.register_root(handle.clone());
                        }
                        backend = Backend::from(handle);
                        *lock_slot(&backend_slot) = backend.clone();
                        session_path = Some(path.clone());
                        plan = false; // the rebuilt agent starts with plan mode off
                        println!("resumed {} ({n} messages)", path.display());
                    }
                    Err(e) => eprintln!("open {}: {e}", path.display()),
                }
                }
            }
            Some(Command::Sessions) => match list_sessions(&session_dir()) {
                Ok(list) if list.is_empty() => {
                    println!("no sessions in {}", session_dir().display())
                }
                Ok(list) => {
                    let current = session_path.as_deref();
                    // A group-resumed session's path is `<dir>/root.jsonl`; map
                    // it back up so the group dir row is still marked.
                    let current_group = current.and_then(crate::session_groups::group_dir_of);
                    for p in list {
                        let mark = if current == Some(p.as_path())
                            || current_group.as_deref() == Some(p.as_path())
                        {
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
            Some(Command::Reload { .. }) if remote => {
                println!("{DIM}/reload (rebuild + re-exec) is unavailable over a socket{RESET}")
            }
            Some(Command::Reload { no_session }) => {
reload(&llm, session_path.as_deref(), no_session, orchestrator.is_some(), overlay, owner, name, Some(&in_flight)).await
            }
            Some(Command::Btw(q)) => {
                if q.trim().is_empty() {
                    println!("usage: /btw <question>");
                } else {
                    match backend.ask(Request::SideAsk { text: q }).await {
                        Ok(AgentEvent::SideAnswer { text, usage: _ }) => {
                            println!("{DIM}btw: {text}{RESET}");
                        }
                        Ok(AgentEvent::Error { message }) => eprintln!("btw: {message}"),
                        Ok(_) => eprintln!("btw: unexpected reply"),
                        Err(_) => eprintln!("btw: session closed"),
                    }
                }
            }
            Some(Command::Plan(want)) => {
                // The REPL may be a socket client, so it keeps a local mirror
                // rather than the agent's handle; the toggle reaches the agent
                // (flipping the shared `PlanModeHooks`) via `SetPlanMode`.
                let on = want.unwrap_or(!plan);
                match backend.ask(Request::SetPlanMode { on }).await {
                    Ok(AgentEvent::Ack) => {
                        plan = on;
                        println!("{DIM}plan mode: {}{RESET}", if on { "on" } else { "off" });
                    }
                    Ok(AgentEvent::Error { message }) => eprintln!("plan: {message}"),
                    Ok(_) => eprintln!("plan: unexpected reply"),
                    Err(_) => eprintln!("plan: session closed"),
                }
            }
            Some(Command::Verify) => {
                let cached = last_todos.lock().unwrap_or_else(|e| e.into_inner()).clone();
                // The last assistant reply (a remote-safe read).
                let last_text = match backend.ask(Request::GetHistory).await {
                    Ok(AgentEvent::History { messages }) => messages.iter().rev().find_map(|m| match m {
                        AgentMessage::Assistant { .. } => {
                            let text = m.as_text();
                            (!text.is_empty()).then_some(text)
                        }
                        _ => None,
                    }),
                    _ => None,
                };
                print_verify(cached.as_deref(), last_text.as_deref());
            }
            Some(Command::Usage) => match backend.ask(Request::GetHistory).await {
                Ok(AgentEvent::History { messages }) => {
                    let stats = session_stats(&messages);
                    println!("{}", stats.summary());
                    if let Some(limit) = model_limit(llm.base_url.as_deref(), &llm.model) {
                        let used = stats.last_input_tokens.unwrap_or(0);
                        let pct = used * 100 / limit.context.max(1);
                        println!("context: {used}/{} ({pct}%)", limit.context);
                    }
                }
                Ok(_) => eprintln!("usage: unexpected reply"),
                Err(_) => eprintln!("usage: session closed"),
            },
            Some(Command::Compact(arg)) => {
                match backend.ask(Request::Compact { instructions: arg }).await {
                    // `/compact` is a request/reply (`Request::Compact`), not the
                    // auto-compaction stream, so it never answers with
                    // `CompactionSkipped`; the wildcard below covers any
                    // unexpected reply.
                    Ok(AgentEvent::Compaction { summarized, kept }) => {
                        println!("compacted: summarized {summarized}, kept {kept} (+ summary)")
                    }
                    Ok(AgentEvent::Ack) => println!("(nothing to compact)"),
                    Ok(AgentEvent::Error { message }) => eprintln!("compact: {message}"),
                    Ok(_) => eprintln!("compact: unexpected reply"),
                    Err(_) => eprintln!("compact: session closed"),
                }
            }
            Some(Command::Skills) => print_skills(&skills),
            Some(Command::Skill(arg)) => match arg.as_deref() {
                None => println!("usage: /skill <name> [args]   (/skills lists them)"),
                Some(arg) => {
                    let (name, extra) = match arg.split_once(char::is_whitespace) {
                        Some((n, rest)) => (n, Some(rest.trim())),
                        None => (arg, None),
                    };
                    match skills.find(name) {
                        None => eprintln!("no such skill: {name}   (/skills lists them)"),
                        Some(skill) => match std::fs::read_to_string(&skill.path) {
                            Err(e) => eprintln!("read {}: {e}", skill.path.display()),
                            Ok(body) => {
                                let input = skill_turn(&skill.name, &body, extra);
                                run_turn(&backend, &input, &in_flight, &last_todos).await;
                            }
                        },
                    }
                }
            },
            None => run_turn(&backend, line, &in_flight, &last_todos).await,
        }
    }

    // Interactive close: the exact command to bring this session back (the same
    // argv `/reload` would exec). A remote client owns no local session, so
    // `print_relaunch` prints nothing for it.
    print_relaunch(
        &llm,
        session_path.as_deref(),
        orchestrator.is_some(),
        overlay,
        owner,
        name,
    );
}

/// `/skills`: what was discovered, one block per skill, with the file the
/// model would `read`.
/// `/verify`: the last cached `AgentEvent::Todo` checklist (a client render of the
/// emitted event — NOT `Session::todo()`), the done count, remaining items, and
/// the last assistant reply.
fn print_verify(todos: Option<&[TodoItem]>, last_text: Option<&str>) {
    match todos {
        None => println!("{DIM}(no plan/todos recorded yet){RESET}"),
        Some([]) => println!("{DIM}(no todos — the plan list is empty){RESET}"),
        Some(list) => {
            let done = list
                .iter()
                .filter(|t| t.status == TodoStatus::Completed)
                .count();
            println!("{}", crate::tools::todo::render(list));
            println!("{done}/{} done", list.len());
            let remaining: Vec<&str> = list
                .iter()
                .filter(|t| t.status != TodoStatus::Completed)
                .map(|t| t.content.as_str())
                .collect();
            if remaining.is_empty() {
                println!("{DIM}finished — all items completed{RESET}");
            } else {
                println!(
                    "{DIM}not finished — {} remaining: {}{RESET}",
                    remaining.len(),
                    remaining.join(", ")
                );
            }
        }
    }
    if let Some(text) = last_text {
        println!("{DIM}— last reply —{RESET}");
        println!("{text}");
    }
}

fn print_skills(skills: &SkillSet) {
    if skills.is_empty() {
        println!("(no skills discovered)");
        return;
    }
    for s in &skills.skills {
        println!("  {} — {}", s.name, tool_output_note(&s.description));
        println!("{DIM}    {}{RESET}", s.path.display());
    }
    println!(
        "{DIM}  {} skill(s) — force one with /skill <name>{RESET}",
        skills.skills.len()
    );
}

/// Turn text for `/skill <name> [args]`. Framed as a directive so the model
/// follows the skill rather than treating it as reference material; extra args
/// become the task it applies them to.
fn skill_turn(name: &str, body: &str, args: Option<&str>) -> String {
    let mut out = format!("# Skill: {name}\n\n{}", body.trim_end());
    if let Some(args) = args {
        out.push_str("\n\n## Task\n");
        out.push_str(args);
    }
    out
}

/// When attaching to a session served elsewhere, print the transcript so far —
/// the "replay from the log" a (re)connecting client shows. Local sessions are
/// not replayed: the REPL never has.
async fn replay(backend: &Backend) {
    let Ok(AgentEvent::History { messages }) = backend.ask(Request::GetHistory).await else {
        return;
    };
    if messages.is_empty() {
        return;
    }
    println!("{DIM}— {} earlier message(s) —{RESET}", messages.len());
    for message in &messages {
        match message {
            AgentMessage::User { .. } => println!("{DIM}❯ {}{RESET}", message.as_text()),
            AgentMessage::Assistant { .. } => {
                let text = message.as_text();
                if !text.is_empty() {
                    println!("{text}");
                }
            }
            AgentMessage::ToolResult { .. } => {}
        }
    }
}

/// One user turn: subscribe for the printer, submit, and await the run's stop
/// reason.
async fn run_turn(
    backend: &Backend,
    input: &str,
    in_flight: &AtomicBool,
    last_todos: &Arc<Mutex<Option<Vec<TodoItem>>>>,
) {
    let mut rx = backend.subscribe();
    let todos = last_todos.clone();
    let printer = tokio::spawn(async move { print_events(&mut rx, &todos).await });
    in_flight.store(true, Ordering::SeqCst);
    let reply = backend
        .ask(Request::Submit {
            text: input.to_string(),
        })
        .await;
    in_flight.store(false, Ordering::SeqCst);
    let _ = printer.await;
    match reply {
        Ok(AgentEvent::Stopped { stop_reason }) => match stop_reason {
            StopReason::Aborted => println!("{DIM}(aborted){RESET}"),
            StopReason::Error => eprintln!("{DIM}✗ run failed{RESET}"),
            StopReason::MaxTurns => println!("{DIM}(hit max turns){RESET}"),
            _ => {}
        },
        Ok(_) => {}
        Err(_) => eprintln!("{DIM}error: session closed{RESET}"),
    }
}

async fn print_events(
    rx: &mut broadcast::Receiver<AgentEvent>,
    last_todos: &Mutex<Option<Vec<TodoItem>>>,
) {
    let mut p = MessagePrinter::default();
    let mut st = PrintState::default();
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let done = matches!(ev, AgentEvent::AgentEnd);
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
            AgentEvent::CompactionSkipped { reason } => {
                // Non-fatal: a best-effort auto-compaction miss. Rendered like the
                // success line below, never through the `Error` arm above (which
                // would print the run-error path).
                close_blocks(&mut p, &mut st);
                out(&format!("{DIM}⋯ compaction skipped: {reason}{RESET}\n"));
            }
            AgentEvent::Compaction { summarized, kept } => {
                close_blocks(&mut p, &mut st);
                out(&format!(
                    "{DIM}⋯ compacted {summarized} messages, kept {kept}{RESET}\n"
                ));
            }
            AgentEvent::Retrying { attempt, max, reason } => {
                close_blocks(&mut p, &mut st);
                out(&format!(
                    "{DIM}⋯ retrying ({attempt}/{max}): {reason}{RESET}\n"
                ));
            }
            AgentEvent::Todo { todos } => {
                // Cache for `/verify` (a client render of the last emitted list).
                *last_todos.lock().unwrap_or_else(|e| e.into_inner()) = Some(todos);
            }
            _ => {}
        }
        if done {
            break;
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

    #[tokio::test]
    async fn build_agent_leaves_the_shared_hook_set_untouched() {
        // Reviewer #1's correction: WorkspaceHooks is built INSIDE build_agent,
        // so the shared set that `/new` and `/resume` clone never carries a
        // per-session digest cache — the policy is rebuilt with every agent.
        let shared = HooksSet::default();
        assert!(shared.is_empty());
        let (agent, _bg) = build_agent(
            AgentSpec {
                llm: LlmOpts::default(),
                hooks: shared.clone(),
                tools: &ToolsConfig::default(),
                compaction: CompactionPolicy::default(),
                instructions: &InstructionSet::default(),
                skills: &SkillSet::default(),
                team: &[],
                guidelines: None,
            },
            None,
            Vec::new(),
            Vec::new(),
            true,
        );
        assert!(
            shared.is_empty(),
            "the shared set must not gain the per-session WorkspaceHooks"
        );
        assert!(
            !agent.hooks().is_empty(),
            "build_agent must push its fresh WorkspaceHooks"
        );
    }
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
        assert_eq!(parse_command("/btw why?"), Some(Command::Btw("why?".into())));
        assert_eq!(parse_command("/btw"), Some(Command::Btw(String::new())));
        assert_eq!(parse_command("/plan"), Some(Command::Plan(None)));
        assert_eq!(parse_command("/plan on"), Some(Command::Plan(Some(true))));
        assert_eq!(parse_command("/plan off"), Some(Command::Plan(Some(false))));
        assert_eq!(parse_command("/plan bogus"), None);
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
        assert_eq!(parse_command("/verify"), Some(Command::Verify));
        assert_eq!(parse_command("/usage"), Some(Command::Usage));
        assert_eq!(parse_command("/compact"), Some(Command::Compact(None)));
        assert_eq!(
            parse_command("/compact focus on the API"),
            Some(Command::Compact(Some("focus on the API".into())))
        );
        assert_eq!(parse_command("/skills"), Some(Command::Skills));
        assert_eq!(parse_command("/skill"), Some(Command::Skill(None)));
        assert_eq!(
            parse_command("/skill pdf-tools"),
            Some(Command::Skill(Some("pdf-tools".into())))
        );
        assert_eq!(
            parse_command("/skill pdf-tools extract page 2"),
            Some(Command::Skill(Some("pdf-tools extract page 2".into())))
        );
    }

    #[test]
    fn skill_turn_frames_the_body_and_appends_args() {
        let plain = skill_turn("pdf-tools", "# PDF Tools\n\nRun extract.sh.\n", None);
        assert_eq!(plain, "# Skill: pdf-tools\n\n# PDF Tools\n\nRun extract.sh.");

        let with_args = skill_turn("pdf-tools", "body\n", Some("merge a.pdf b.pdf"));
        assert!(
            with_args.ends_with("\n\n## Task\nmerge a.pdf b.pdf"),
            "{with_args}"
        );
    }

    #[test]
    fn system_prompt_names_only_registered_inspect_tools() {
        let off = system_prompt(
            &ToolsConfig::default(),
            &crate::instructions::InstructionSet::default(),
            &SkillSet::default(),
            &[],
            None,
            Path::new("/"),
        );
        assert!(off.contains("Inspect with read;"), "{off}");
        assert!(!off.contains("grep") && !off.contains("find"));

        let on = system_prompt(
            &ToolsConfig {
                grep: true,
                find: true,
                ..ToolsConfig::default()
            },
            &crate::instructions::InstructionSet::default(),
            &SkillSet::default(),
            &[],
            None,
            Path::new("/"),
        );
        assert!(on.contains("Inspect with read, grep and find;"), "{on}");

        let only_grep = system_prompt(
            &ToolsConfig {
                grep: true,
                find: false,
                ..ToolsConfig::default()
            },
            &crate::instructions::InstructionSet::default(),
            &SkillSet::default(),
            &[],
            None,
            Path::new("/"),
        );
        assert!(
            only_grep.contains("Inspect with read and grep;"),
            "{only_grep}"
        );
    }

    #[test]
    fn system_prompt_renders_the_team_block_only_with_a_team() {
        let team = vec![
            TeamMember {
                name: "explorer".into(),
                role: Some("recon only; never edit; cite file:line".into()),
                ..Default::default()
            },
            TeamMember {
                name: "reviewer".into(),
                ..Default::default()
            },
        ];
        let prompt = system_prompt(
            &ToolsConfig::default(),
            &crate::instructions::InstructionSet::default(),
            &SkillSet::default(),
            &team,
            None,
            Path::new("/"),
        );
        assert!(prompt.contains("# Your team"), "{prompt}");
        assert!(prompt.contains("to: \"<name>\""), "{prompt}");
        // A member with a role renders `- name — role`...
        assert!(
            prompt.contains("- explorer — recon only; never edit; cite file:line"),
            "{prompt}"
        );
        // ...a roleless one renders `- name` only (no dash-role tail).
        assert!(prompt.ends_with("- reviewer"), "{prompt}");
        assert!(!prompt.contains("- reviewer —"), "{prompt}");

        // No team → no block at all.
        let bare = system_prompt(
            &ToolsConfig::default(),
            &crate::instructions::InstructionSet::default(),
            &SkillSet::default(),
            &[],
            None,
            Path::new("/"),
        );
        assert!(!bare.contains("# Your team"), "{bare}");
    }

    #[test]
    fn system_prompt_renders_guidelines_only_when_non_empty() {
        let with = system_prompt(
            &ToolsConfig::default(),
            &crate::instructions::InstructionSet::default(),
            &SkillSet::default(),
            &[],
            Some("Step 1: delegate.\nStep 2: verify."),
            Path::new("/"),
        );
        assert!(with.contains("# Orchestrator workflow"), "{with}");
        assert!(with.contains("Step 1: delegate."), "{with}");

        // Absent / empty / whitespace-only → no section (prompt unchanged).
        for missing in [None, Some(""), Some("   \n\t ")] {
            let p = system_prompt(
                &ToolsConfig::default(),
                &crate::instructions::InstructionSet::default(),
                &SkillSet::default(),
                &[],
                missing,
                Path::new("/"),
            );
            assert!(!p.contains("# Orchestrator workflow"), "{missing:?}: {p}");
        }

        // With a team, the workflow section follows the roster.
        let both = system_prompt(
            &ToolsConfig::default(),
            &crate::instructions::InstructionSet::default(),
            &SkillSet::default(),
            &[TeamMember {
                name: "a".into(),
                ..Default::default()
            }],
            Some("after the team"),
            Path::new("/"),
        );
        let team_at = both.find("# Your team").expect("team block");
        let wf_at = both.find("# Orchestrator workflow").expect("workflow block");
        assert!(team_at < wf_at, "workflow must follow the roster: {both}");
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
            retry: wcode_harness::streamfn::RetryPolicy::default(),
        };
        assert_eq!(
            reload_args(&llm, Some(Path::new("/s/a.jsonl")), false, false, None, None, None),
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
            reload_args(&llm, Some(Path::new("/s/a.jsonl")), true, false, None, None, None)[..2],
            ["--no-session".to_string(), "--model".to_string()],
        );
        // no session file: fresh start, cleared effort round-trips as "-"
        let args = reload_args(&llm, None, false, false, None, None, None);
        assert_eq!(args[0], "--no-session");
        assert!(args.windows(2).any(|w| w == ["--effort", "-"]));
        assert!(args.windows(2).any(|w| w == ["--endpoint", "chat"]));
        assert!(!args.iter().any(|a| a == "--base-url"));
    }

    #[test]
    fn reload_args_forwards_agents_only_when_set() {
        let llm = LlmOpts {
            model: "m".to_string(),
            ..LlmOpts::default()
        };
        // An orchestrator survives a re-exec (`--agents` forwarded)...
        let with = reload_args(&llm, Some(Path::new("/s/a.jsonl")), false, true, None, None, None);
        assert!(with.iter().any(|a| a == "--agents"), "{with:?}");
        // ...a plain session does not grow the flag.
        let without = reload_args(&llm, Some(Path::new("/s/a.jsonl")), false, false, None, None, None);
        assert!(!without.iter().any(|a| a == "--agents"), "{without:?}");
    }

    #[test]
    fn reload_args_forwards_config_overlay_when_set() {
        let llm = LlmOpts {
            model: "m".to_string(),
            ..LlmOpts::default()
        };
        // A flag-supplied overlay must survive the re-exec...
        let with = reload_args(
            &llm,
            Some(Path::new("/s/a.jsonl")),
            false,
            false,
            Some(".wcode/team.toml"),
            None,
            None,
        );
        assert!(
            with.windows(2)
                .any(|w| w == ["--config", ".wcode/team.toml"]),
            "{with:?}"
        );
        // ...with no overlay, no `--config` is emitted.
        let without = reload_args(&llm, Some(Path::new("/s/a.jsonl")), false, false, None, None, None);
        assert!(!without.iter().any(|a| a == "--config"), "{without:?}");
    }

    #[test]
    fn reload_args_forwards_owner_and_name_when_set() {
        let llm = LlmOpts {
            model: "m".to_string(),
            ..LlmOpts::default()
        };
        // A served worker resumed here re-execs back into the worker branch:
        // both the ownership edge and the worker id must survive the handoff.
        let with = reload_args(
            &llm,
            Some(Path::new("/s/a.jsonl")),
            false,
            true,
            None,
            Some("agent:root"),
            Some("w1"),
        );
        assert!(
            with.windows(2).any(|w| w == ["--owner", "agent:root"]),
            "{with:?}"
        );
        assert!(with.windows(2).any(|w| w == ["--name", "w1"]), "{with:?}");
        // A plain session grows neither flag.
        let without =
            reload_args(&llm, Some(Path::new("/s/a.jsonl")), false, false, None, None, None);
        assert!(!without.iter().any(|a| a == "--owner"), "{without:?}");
        assert!(!without.iter().any(|a| a == "--name"), "{without:?}");
    }

    #[test]
    fn relaunch_line_prefixes_the_program_and_quotes_unsafe_args() {
        let line = relaunch_line(&[
            "--resume".into(),
            "/s/a b.jsonl".into(),
            "--model".into(),
            "gpt-x".into(),
        ]);
        // The program token comes first, unquoted here (argv[0] in tests).
        assert!(!line.starts_with(' '), "{line}");
        // Safe args pass through; the path with a space is single-quoted.
        assert!(
            line.ends_with("--resume '/s/a b.jsonl' --model gpt-x"),
            "{line}"
        );
    }

    #[test]
    fn shell_quote_escapes_only_when_needed() {
        assert_eq!(shell_quote("plain-1.2/3"), "plain-1.2/3");
        assert_eq!(shell_quote("http://x/v1"), "http://x/v1");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote(""), "''");
        // An embedded single quote is escaped portably.
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[tokio::test]
    async fn an_idle_ctrl_c_signal_ends_the_prompt_wait() {
        // The fix: idle Ctrl-C notifies instead of exiting, so the prompt read
        // is abandoned and the loop breaks — then the after-loop relaunch line
        // prints, exactly like `/exit`/EOF.
        let signal = tokio::sync::Notify::new();
        let pending = std::future::pending::<Option<String>>();
        let fut = unless_idle_ctrl_c(&signal, pending);
        signal.notify_one();
        assert!(fut.await.is_none(), "the signal must end the prompt wait");
    }

    #[tokio::test]
    async fn a_ready_read_is_returned_without_a_signal() {
        let signal = tokio::sync::Notify::new();
        let out = unless_idle_ctrl_c(&signal, async { Some("hi".to_string()) }).await;
        assert_eq!(out, Some(Some("hi".to_string())));
    }

    #[test]
    fn lock_slot_tolerates_poisoning() {
        let m = Mutex::new(0);
        // Poison the mutex by panicking while the lock is held.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = m.lock().unwrap();
            panic!("poison on purpose");
        }));
        // The helper must still hand back the guard rather than panicking.
        let guard = lock_slot(&m);
        assert_eq!(*guard, 0);
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

    #[tokio::test]
    async fn print_events_caches_the_latest_todo_event() {
        let (tx, mut rx) = broadcast::channel(8);
        let cache: Mutex<Option<Vec<TodoItem>>> = Mutex::new(None);
        // A stale value is replaced by the newest event.
        *cache.lock().unwrap() = Some(vec![TodoItem {
            content: "old".into(),
            status: TodoStatus::Pending,
        }]);
        let list = vec![TodoItem {
            content: "new".into(),
            status: TodoStatus::InProgress,
        }];
        tx.send(AgentEvent::Todo { todos: list.clone() }).unwrap();
        tx.send(AgentEvent::AgentEnd).unwrap();

        print_events(&mut rx, &cache).await;
        assert_eq!(cache.lock().unwrap().as_deref(), Some(list.as_slice()));
    }
}

#[cfg(test)]
mod system_prompt_tests {
    use super::*;
    use crate::instructions::{InstructionSet, Instructions};

    #[test]
    fn appends_project_instructions_and_omits_when_empty() {
        let set = InstructionSet {
            files: vec![Instructions {
                path: PathBuf::from("/repo/AGENTS.md"),
                text: "Build: cargo build\n".into(),
                global: false,
            }],
        };
        let with = system_prompt(&ToolsConfig::default(), &set, &SkillSet::default(), &[], None, Path::new("/"));
        assert!(with.contains("You are wcode"), "{with}");
        assert!(
            with.contains("# Project instructions (/repo/AGENTS.md)"),
            "{with}"
        );
        assert!(with.contains("Build: cargo build"), "{with}");

        let base = system_prompt(
            &ToolsConfig::default(),
            &InstructionSet::default(),
            &SkillSet::default(),
            &[],
            None,
            Path::new("/"),
        );
        assert!(!base.contains("Project instructions"), "{base}");
    }
}

