//! CLI entry: arg parsing, config → LlmOpts → Agent, one-shot or REPL.

use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};

use wcode_harness::actor::SessionActor;
use wcode_harness::event::AgentEvent;
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::session::Session;
use wcode_harness::streamfn::rig_stream_fn;
use wcode_protocol::Backend;

mod agents;
mod config;
mod instructions;
mod repl;
mod rtk;
mod skills;
mod tools;

use crate::config::{
    Config, ConfigError, EnvLike, FileConfig, TeamMember, config_dir, merge, parse_endpoint,
};
use crate::instructions::{InstructionSet, Mode, load as load_instructions};
use crate::skills::{SkillSet, discover as discover_skills};
use crate::repl::{
    AgentSpec, build_agent, default_hooks, list_sessions, resolve_session_path, session_dir,
    system_prompt,
};

const USAGE: &str = "\
wcode — minimal coding agent

usage: wcode [-p <prompt>] [--resume [path]] [--no-session] [--model <id>] [--base-url <url>] [--config <path>] [--endpoint <chat|responses>] [--effort <level>] [--list-models] [--no-instructions] [--no-skills] [--dump-system-prompt] [--agents] [--peer <name>=<socket>] [--name <id>] [--owner <addr>] [serve] [--socket <path>] [--tui|--no-tui]

  -p <prompt>        run once with <prompt>, print the reply, exit
  --resume [path]    resume a session (default: latest in the session dir)
  --no-session       don't record a session file
   --model <id>       override the configured model
   --base-url <url>   override the configured base URL
   --config <path>    overlay config file, deep-merged over the global config (also WCODE_CONFIG)
   --endpoint <e>     override the configured endpoint (chat|responses)
   --effort <level>   override the reasoning effort (free-style, e.g. high; '-'/'none'/'off' clears it)
   --list-models      list models from GET {base_url}/models and exit
   --sequential       run tool calls one at a time (default: independent calls in a
                      batch run in parallel)
   --no-instructions  don't load instruction files (AGENTS.md/CLAUDE.md)
   --no-skills        don't discover skills (SKILL.md)
  --dump-system-prompt  print the composed system prompt and exit
  serve              own the session and serve it at --socket (default: ~/.config/wcode/wcode.sock)
  --socket <path>    connect to a session served elsewhere (with -p; a remote REPL is next)
  --tui | --no-tui   force the full-screen TUI, or the line REPL (default: TUI on a TTY)
   -h, --help         show this help

config: ~/.config/wcode/config.toml
  model = \"...\"      (required)
  base_url = \"...\"   (optional, any OpenAI-compatible endpoint)
  api_key = \"...\"    (optional)
  endpoint = \"...\"   (optional, chat|responses, default chat)
  effort = \"...\"     (optional, free-style reasoning effort, omitted = not sent)

  [hooks]
  rtk = \"...\"        (optional, auto|true|false; route bash output through the rtk proxy to cut tokens)

  [tools]
  grep = \"...\"       (optional, true|false; register the grep tool. Off by default — bash can search)
  find = \"...\"       (optional, true|false; register the find tool. Off by default — bash can list files)

  [instructions]
  file = \"...\"       (optional; load exactly this file instead of discovery. \"off\" disables)
  names = [...]      (optional; candidate names to discover per directory. Default: AGENTS.override.md, AGENTS.md, CLAUDE.md)
  global = true      (optional; also load the candidate file from ~/.config/wcode. Default true)

  [skills]
  enabled = true     (optional; discover SKILL.md packages. Default true)
  dirs = [...]       (optional; extra skill roots, scanned first)
  disabled = [...]   (optional; skill names to skip)

  [retry]
  max = 3            (optional; retry transient connect errors this many times; 0 disables)
  base_ms = 500      (optional; base backoff for the first retry)
  cap_ms = 8000      (optional; cap on a single backoff wait)
env: WCODE_BASE_URL and WCODE_API_KEY override the toml; OPENAI_API_KEY is a key fallback
env: WCODE_ENDPOINT overrides the toml endpoint; WCODE_EFFORT overrides the toml effort
env: WCODE_RTK overrides the toml hooks.rtk (auto|true|false)
env: WCODE_GREP and WCODE_FIND override the toml tools.grep/find (true|false)
env: WCODE_INSTRUCTIONS overrides the toml instructions.file (a name/path, or \"off\")
env: WCODE_SKILLS discovers skills from extra roots, or \"off\" disables
env: WCODE_RETRY_MAX, WCODE_RETRY_BASE_MS, WCODE_RETRY_CAP_MS override the toml retry table";

#[derive(Debug, Default, PartialEq)]
struct Args {
    prompt: Option<String>,
    /// None = flag absent; Some(None) = latest; Some(Some(path)) = that file.
    resume: Option<Option<String>>,
    no_session: bool,
    model: Option<String>,
    base_url: Option<String>,
    /// `--config <path>`: an overlay config file, deep-merged over the global one.
    config: Option<String>,
    endpoint: Option<String>,
    /// None = flag absent; Some(None) = clear; Some(Some(level)) = set.
    effort: Option<Option<String>>,
    list_models: bool,
    no_instructions: bool,
    dump_system_prompt: bool,
    no_skills: bool,
    /// `--agents`: register the `spawn` tool so this session can spawn worker
    /// agents (A2A, §10.1).
    agents: bool,
    /// `--peer <name>=<socket>`: register a remote peer (a served session) so
    /// A2A messages reach it over its socket (§8, S4-4). Repeatable.
    peers: Vec<String>,
    /// `--name <id>`: this session's A2A address (`agent:<id>`), for `--owner`.
    name: Option<String>,
    /// `--owner <addr>`: make this (served) session a worker of `<addr>` — it
    /// reports back over the socket (S4-4 reply). Requires `--agents`.
    owner: Option<String>,
    sequential: bool,
    /// `wcode serve`: own the session and serve it over a socket.
    serve: bool,
    /// Connect to a session served elsewhere instead of running one locally.
    /// Connect to a session served elsewhere instead of running one locally.
    socket: Option<String>,
    /// `--tui`: force the full-screen TUI.
    tui: bool,
    /// `--no-tui`: force the line REPL (pipes/CI).
    no_tui: bool,
}

// `Args` is much larger than `Help`; boxing it would only churn the call sites.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
enum Parsed {
    Args(Args),
    Help,
}

fn parse_args(args: &[String]) -> Result<Parsed, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        i += 1;
        match flag {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-p" => {
                a.prompt = Some(args.get(i).ok_or("-p requires a prompt")?.clone());
                i += 1;
            }
            "--resume" => {
                let path = args.get(i).filter(|v| !v.starts_with('-')).cloned();
                if path.is_some() {
                    i += 1;
                }
                a.resume = Some(path);
            }
            "--no-session" => a.no_session = true,
            "--model" => {
                a.model = Some(args.get(i).ok_or("--model requires an id")?.clone());
                i += 1;
            }
            "--base-url" => {
                a.base_url = Some(args.get(i).ok_or("--base-url requires a url")?.clone());
                i += 1;
            }
            "--config" => {
                a.config = Some(args.get(i).ok_or("--config requires a path")?.clone());
                i += 1;
            }
            "--endpoint" => {
                a.endpoint = Some(
                    args.get(i)
                        .ok_or("--endpoint requires chat|responses")?
                        .clone(),
                );
                i += 1;
            }
            "--effort" => {
                let level = args.get(i).ok_or("--effort requires a level")?.clone();
                i += 1;
                // "-", "none" and "off" clear back to send-nothing (mirror
                // the REPL's /effort clear synonyms).
                a.effort = Some(match level.as_str() {
                    "-" | "none" | "off" => None,
                    _ => Some(level),
                });
            }
            "--list-models" => a.list_models = true,
            "--no-instructions" => a.no_instructions = true,
            "--dump-system-prompt" => a.dump_system_prompt = true,
            "--no-skills" => a.no_skills = true,
            "--sequential" => a.sequential = true,
            "--agents" => a.agents = true,
            "--peer" => {
                a.peers
                    .push(args.get(i).ok_or("--peer requires <name>=<socket>")?.clone());
                i += 1;
            }
            "--name" => {
                a.name = Some(args.get(i).ok_or("--name requires an id")?.clone());
                i += 1;
            }
            "--owner" => {
                a.owner = Some(args.get(i).ok_or("--owner requires an address")?.clone());
                i += 1;
            }
            "serve" => a.serve = true,
            "--socket" => {
                a.socket = Some(args.get(i).ok_or("--socket requires a path")?.clone());
                i += 1;
            }
            "--tui" => a.tui = true,
            "--no-tui" => a.no_tui = true,
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(Parsed::Args(a))
}

/// MissingModel rescue: re-run merge() with the parsed file config so env
/// AND toml base_url/api_key survive a file that lacks `model`; the flag
/// model fills the gap. Flag overrides are applied by the caller below.
fn rescue(model: String, file: FileConfig, env: EnvLike) -> Result<Config, ConfigError> {
    merge(
        env,
        FileConfig {
            model: Some(model),
            ..file
        },
    )
}

/// Skills to fold into the system prompt: `--no-skills` and `[skills]
/// enabled = false` disable discovery; otherwise the configured roots plus the
/// standard ones are scanned.
fn discover_skills_for(args: &Args, cfg: &Config, cwd: &std::path::Path) -> SkillSet {
    if args.no_skills {
        return SkillSet::default();
    }
    match cfg.skills.to_spec() {
        Some(spec) => discover_skills(&spec, cwd, dirs::home_dir().as_deref()),
        None => SkillSet::default(),
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse_args(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let args = match parsed {
        Parsed::Help => {
            print!("{USAGE}");
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        Parsed::Args(a) => a,
    };

    // `--model` rescues a config that only lacks the model; other config
    // errors (unreadable/corrupt) still surface.
    // `--config` / `WCODE_CONFIG`: an overlay file deep-merged over the global
    // config (flag beats env; an empty env var is ignored).
    let env_overlay = std::env::var("WCODE_CONFIG").ok();
    let overlay = resolve_overlay(args.config.as_deref(), env_overlay.as_deref());
    let loaded = match overlay.as_deref() {
        Some(path) => Config::load_with(Some(Path::new(path))),
        None => Config::load(),
    };
    let mut cfg = match loaded {
        Ok(c) => c,
        Err(ConfigError::MissingModel(file)) if args.model.is_some() || args.dump_system_prompt => {
            // merge() never ran (file lacked model): rescue re-runs it with
            // the parsed file so toml/env base_url+api_key survive;
            // `--model`/`--base-url` flags are applied below and still win.
            // An invalid env override (endpoint/rtk/grep/find) still surfaces
            // as a clean error here — not a panic.
            match rescue(
                args.model.clone().unwrap_or_else(|| "(unset)".into()),
                *file,
                EnvLike::from_env(),
            ) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            // The "set model" / env hints only fit a missing model; any other
            // error (overlay not found, duplicate team, IO) is shown alone.
            if matches!(e, ConfigError::MissingModel(_)) {
                eprintln!(
                    "  set `model = \"...\"` in {} or pass --model",
                    Config::default_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "~/.config/wcode/config.toml".into())
                );
                eprintln!("  env: WCODE_BASE_URL, WCODE_API_KEY (OPENAI_API_KEY fallback)");
            }
            std::process::exit(1);
        }
    };
    if let Some(m) = args.model.clone() {
        cfg.model = m;
    }
    if let Some(u) = args.base_url.clone() {
        cfg.base_url = Some(u);
    }
    if let Some(e) = args.endpoint.as_deref() {
        match parse_endpoint(Some(e)) {
            Ok(endpoint) => cfg.endpoint = endpoint,
            Err(msg) => {
                eprintln!("error: {msg}");
                std::process::exit(2);
            }
        }
    }
    if let Some(effort) = args.effort.clone() {
        cfg.effort = effort;
    }
    // `--sequential` wins over `[tools] parallel`.
    if args.sequential {
        cfg.tools.parallel = Some(false);
    }
    let mut llm = cfg.to_llm_opts();

    // `--dump-system-prompt`: print the composed prompt (instructions
    // included) and exit. Needs no endpoint, model, or session.
    if args.dump_system_prompt {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let mode = if args.no_instructions {
            Mode::Off
        } else {
            cfg.instructions.mode()
        };
        let instructions = load_instructions(&mode, &cwd, config_dir().as_deref());
        let skills = discover_skills_for(&args, &cfg, &cwd);
        println!(
            "{}",
            repl::system_prompt(
                &cfg.tools,
                &instructions,
                &skills,
                &cfg.team,
                cfg.orchestrator.guidelines.as_deref(),
                &cwd,
            )
        );
        std::process::exit(0);
    }

    // `--list-models`: resolve against the same base_url/key as chat, print
    // sorted ids (`*` marks the configured model), exit. No session touched.
    if args.list_models {
        match wcode_harness::streamfn::list_models(&llm).await {
            Ok(ids) if ids.is_empty() => {
                println!("(no models)");
            }
            Ok(ids) => {
                for id in ids {
                    let mark = if id == llm.model { "*" } else { " " };
                    println!("{mark} {id}");
                }
            }
            Err(e) => {
                eprintln!("error: list models: {e}");
                std::process::exit(1);
            }
        }
        std::process::exit(0);
    }

    // `--socket`: connect to a session served elsewhere. No local session is
    // created — the server owns it.
    #[cfg(unix)]
    if let Some(sock) = args.socket.clone().filter(|_| !args.serve) {
        let path = PathBuf::from(&sock);
        let client = match wcode_protocol::Client::connect(&path).await {
            Ok(client) => client,
            Err(e) => {
                eprintln!("connect {}: {e}", path.display());
                std::process::exit(1);
            }
        };
        match args.prompt.clone() {
            Some(prompt) => std::process::exit(one_shot(Backend::from(client), &prompt).await),
            None => {
                if choose_tui(&args, is_tty()) {
                    let status = wcode_tui::Status {
                        model: llm.model.clone(),
                        effort: llm.effort.clone(),
                        // The server owns the session id; the client cannot learn it yet.
                        session: None,
                        context_limit: wcode_harness::limits::model_limit(
                            llm.base_url.as_deref(),
                            &llm.model,
                        )
                        .map(|l| l.context),
                    };
                    let options = wcode_tui::Options {
                        status,
                        models: wcode_harness::streamfn::list_models(&llm).await.unwrap_or_default(),
                        // The server owns the session; a socket client cannot see
                        // the session dir, so `/resume` has nothing to offer.
                        sessions: Vec::new(),
                        history: Some(repl::history_path()),
                    };
                    // A socket client sees just the root surface.
                    let surfaces = vec![wcode_tui::SurfaceSpec {
                        id: SessionId::agent("root"),
                        label: "root".to_string(),
                        model: llm.model.clone(),
                        is_root: true,
                        backend: Backend::from(client),
                    }];
                    match wcode_tui::run(surfaces, options, None).await {
                        Ok(wcode_tui::Outcome::Quit) => std::process::exit(0),
                        Ok(wcode_tui::Outcome::Resume(_)) => {
                            eprintln!("tui: cannot resume over a socket");
                            std::process::exit(1);
                        }
                        Err(e) => {
                            eprintln!("tui: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                // The server owns the system prompt and skills, so a remote
                // client discovers none of its own.
                repl::run(
                    repl::SessionSource::Remote(client),
                    llm,
                    default_hooks(&cfg.hooks),
                    cfg.tools,
                    cfg.compaction,
                    InstructionSet::default(),
                    SkillSet::default(),
                    &[],
                    None,
                    None,
                    args.owner.as_deref(),
                    args.name.as_deref(),
                    None,
                )
                .await;
                std::process::exit(0);
            }
        }
    }
    #[cfg(not(unix))]
    if !args.serve && args.socket.is_some() {
        eprintln!("error: `--socket` is not supported on this platform");
        std::process::exit(2);
    }

    let (session, context): (Option<Session>, Vec<AgentMessage>) = match args.resume.clone() {
        Some(path) => {
            let p = match path {
                Some(a) => resolve_session_path(&a),
                None => match list_sessions(&session_dir()) {
                    Ok(list) => match list.into_iter().next() {
                        Some(p) => p,
                        None => {
                            eprintln!("no sessions in {}", session_dir().display());
                            std::process::exit(1);
                        }
                    },
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                },
            };
            match Session::open(&p) {
                Ok(s) => {
                    let ctx = s.messages();
                    // `--model`/`--effort` flags win over the session's last
                    // change; otherwise the session restores both.
                    if args.model.is_none()
                        && let Some(m) = s.model()
                    {
                        llm.model = m;
                    }
                    if args.effort.is_none()
                        && let Some(e) = s.effort()
                    {
                        llm.effort = e;
                    }
                    (Some(s), ctx)
                }
                Err(e) => {
                    eprintln!("error: open {}: {e}", p.display());
                    std::process::exit(1);
                }
            }
        }
        None => match (!args.no_session).then(|| Session::create(&session_dir())) {
            Some(Ok(s)) => (Some(s), Vec::new()),
            Some(Err(e)) => {
                eprintln!("error: create session: {e}");
                std::process::exit(1);
            }
            None => (None, Vec::new()),
        },
    };

    // Instruction ("reference") files: discover the configured candidates
    // from the working dir up to the repo root (plus the config-dir global
    // file), unless --no-instructions disables it.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mode = if args.no_instructions {
        Mode::Off
    } else {
        cfg.instructions.mode()
    };
    let instructions = load_instructions(&mode, &cwd, config_dir().as_deref());
    let skills = discover_skills_for(&args, &cfg, &cwd);

    let hooks = default_hooks(&cfg.hooks);
    // A2A (opt-in): an orchestrator wiring — the registry + factory + the root's
    // `spawn`/`message` tools. Only the root gets them, so only it spawns
    // (§10.1).
    let orchestrator = args.agents.then(|| {
        let template = crate::agents::WorkerTemplate {
            system: system_prompt(&cfg.tools, &instructions, &skills, &[], None, &cwd),
            llm: llm.clone(),
            stream_fn: rig_stream_fn(),
            hooks: hooks.clone(),
            tools: cfg.tools,
            compaction: cfg.compaction,
            working_dir: cwd.clone(),
        };
        crate::agents::Orchestrator::new(wcode_protocol::Registry::new(), template)
    });
    // Register remote peers (`--peer name=socket`): a served session becomes an
    // addressable A2A peer reachable over its socket (S4-4).
    if !args.peers.is_empty() && orchestrator.is_none() {
        eprintln!("error: --peer requires --agents");
        std::process::exit(2);
    }
    if !cfg.team.is_empty() && orchestrator.is_none() {
        eprintln!("error: [team] requires --agents");
        std::process::exit(2);
    }
    if cfg
        .orchestrator
        .guidelines
        .as_deref()
        .is_some_and(|g| !g.trim().is_empty())
        && orchestrator.is_none()
    {
        eprintln!("error: [orchestrator] requires --agents");
        std::process::exit(2);
    }
    #[cfg(unix)]
    if let Some(o) = &orchestrator {
        for spec in &args.peers {
            let Some((name, path)) = spec.split_once('=') else {
                eprintln!("error: --peer expects <name>=<socket>, got `{spec}`");
                std::process::exit(2);
            };
            // Lazy: a peer may not be up yet (two peers can be waiting on each
            // other); the supervisor connects once it is.
            o.register_remote(
                SessionId::agent(name),
                wcode_protocol::Client::lazy(Path::new(path)),
            );
        }
    }
    #[cfg(not(unix))]
    if !args.peers.is_empty() {
        eprintln!("error: `--peer` is not supported on this platform");
        std::process::exit(2);
    }
    // `[peers]` (config): persistent peers — a socket path connects a remote
    // peer, an address (`agent:<id>`) is a phonebook alias (§13.15).
    #[cfg(unix)]
    if let Some(o) = &orchestrator {
        for (name, target) in &cfg.peers {
            if target.contains(':') {
                o.alias(name.clone(), SessionId::new(target.clone()));
            } else {
                o.register_remote(
                    SessionId::agent(name),
                    wcode_protocol::Client::lazy(Path::new(target)),
                );
            }
        }
    }
    // `[team]` (F3): spawn each member through the orchestrator, which validates
    // the tool allow-list (D14) and registers the name in the phonebook. Only the
    // root orchestrator spawns — a served `--owner` worker does not.
    if args.owner.is_none()
        && let Some(o) = &orchestrator
    {
        for member in &cfg.team {
            let spec = crate::agents::WorkerSpec {
                name: Some(member.name.clone()),
                model: member.model.clone(),
                system: member.role.clone(),
                tools: member.tools.clone(),
            };
            if let Err(e) = o.spawn_worker(spec) {
                eprintln!("error: team member `{}`: {e}", member.name);
                std::process::exit(2);
            }
        }
    }
    if args.owner.is_none() && !cfg.team.is_empty() {
        let names: Vec<&str> = cfg.team.iter().map(|m| m.name.as_str()).collect();
        println!("team: {}", names.join(", "));
    }

    // A session with `--owner` is a **served worker** (S4-4 reply): it gets a
    // `message` tool bound to its owner and a `ReportBack` hook, and the
    // ownership edge is recorded so its report is permitted.
    let mut agent_hooks = hooks.clone();
    let extra_tools = match (&orchestrator, &args.owner) {
        (Some(o), Some(owner)) => {
            let me = SessionId::agent(args.name.clone().unwrap_or_else(|| "worker".to_string()));
            let (tools, hook) = o.as_worker(me, SessionId::new(owner.clone()));
            agent_hooks.push(hook);
            tools
        }
        (Some(o), None) => o.tools(),
        (None, Some(_)) => {
            eprintln!("error: --owner requires --agents");
            std::process::exit(2);
        }
        (None, None) => Vec::new(),
    };
    // The root orchestrator sees the team; a served worker (`--owner`) does not.
    let root_team: &[TeamMember] = if args.owner.is_some() { &[] } else { &cfg.team };
    let root_guidelines: Option<&str> = if args.owner.is_some() {
        None
    } else {
        cfg.orchestrator.guidelines.as_deref()
    };
    let agent = build_agent(
        AgentSpec {
            llm: llm.clone(),
            hooks: agent_hooks,
            tools: &cfg.tools,
            compaction: cfg.compaction,
            instructions: &instructions,
            skills: &skills,
            team: root_team,
            guidelines: root_guidelines,
        },
        session,
        context,
        extra_tools,
    );

    if args.serve {
        #[cfg(unix)]
        {
            let session_id =
                SessionId::new(agent.session_id().unwrap_or_else(|| "local".to_string()));
            let path = args
                .socket
                .clone()
                .map(PathBuf::from)
                .unwrap_or_else(default_socket_path);
            let handle = SessionActor::spawn(agent);
            if let Some(o) = &orchestrator {
                o.register_root(handle.clone());
            }
            println!("serving session on {}", path.display());
            println!("serving session on {}", path.display());
            let _ = std::io::stdout().flush();
            if let Err(e) = wcode_protocol::serve_at(vec![(session_id, handle)], &path).await {
                eprintln!("serve: {e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        #[cfg(not(unix))]
        {
            eprintln!("error: `serve` is not supported on this platform");
            std::process::exit(2);
        }
    }

    match args.prompt {
        Some(prompt) => {
            let handle = SessionActor::spawn(agent);
            if let Some(o) = &orchestrator {
                o.register_root(handle.clone());
            }
            std::process::exit(one_shot(Backend::from(handle), &prompt).await)
        }
        None => {
            if choose_tui(&args, is_tty()) {
                let status = wcode_tui::Status {
                    model: llm.model.clone(),
                    effort: llm.effort.clone(),
                    session: agent.session_id(),
                    context_limit: wcode_harness::limits::model_limit(
                        llm.base_url.as_deref(),
                        &llm.model,
                    )
                    .map(|l| l.context),
                };
                let options = wcode_tui::Options {
                    status,
                    models: wcode_harness::streamfn::list_models(&llm).await.unwrap_or_default(),
                    sessions: session_items(),
                    history: Some(repl::history_path()),
                };
                let handle = SessionActor::spawn(agent);
                if let Some(o) = &orchestrator {
                    o.register_root(handle.clone());
                }
                // Surface list: the root, then one per team member — only for
                // the root orchestrator (a served `--owner` worker has no team).
                let mut surfaces = vec![wcode_tui::SurfaceSpec {
                    id: orchestrator
                        .as_ref()
                        .map(|o| o.id().clone())
                        .unwrap_or_else(|| SessionId::agent("root")),
                    label: "root".to_string(),
                    model: llm.model.clone(),
                    is_root: true,
                    backend: Backend::from(handle),
                }];
                if args.owner.is_none()
                    && let Some(o) = &orchestrator
                {
                    for member in &cfg.team {
                        if let Some(backend) = o.worker_backend(&member.name) {
                            surfaces.push(wcode_tui::SurfaceSpec {
                                id: SessionId::agent(&member.name),
                                label: member.name.clone(),
                                model: member.model.clone().unwrap_or_else(|| llm.model.clone()),
                                is_root: false,
                                backend,
                            });
                        }
                    }
                }
                // Runtime-spawned workers reach the TUI through this feed. Install
                // the sink only AFTER the `[team]` loop above, so preset members
                // (already in `surfaces`) are not re-emitted. A served worker
                // (`--owner`) has no team surfaces, so it installs nothing.
                let new_surfaces = if args.owner.is_none() {
                    match &orchestrator {
                        Some(o) => {
                            let (tx, rx) =
                                tokio::sync::mpsc::unbounded_channel::<wcode_tui::SurfaceSpec>();
                            o.set_spawn_sink(tx);
                            Some(rx)
                        }
                        None => None,
                    }
                } else {
                    None
                };
                match wcode_tui::run(surfaces, options, new_surfaces).await {
                    Ok(wcode_tui::Outcome::Quit) => std::process::exit(0),
                    Ok(wcode_tui::Outcome::Resume(path)) => {
                        // The TUI cannot rebuild an agent: hand off by re-exec'ing
                        // with `--resume <path>` (the terminal is already restored).
                        repl::exec_self(&repl::reload_args(&llm, Some(&path), false, args.agents, args.config.as_deref(), args.owner.as_deref(), args.name.as_deref()));
                        std::process::exit(1); // only reached if the exec failed
                    }
                    Err(e) => {
                        eprintln!("tui: {e}");
                        std::process::exit(1);
                    }
                }
            }
            repl::run(
                repl::SessionSource::Local(Box::new(agent)),
                llm,
                hooks,
                cfg.tools,
                cfg.compaction,
                instructions,
                skills,
                root_team,
                root_guidelines,
                args.config.as_deref(),
                args.owner.as_deref(),
                args.name.as_deref(),
                orchestrator,
            )
            .await
        }
    }
}

/// The config overlay path: the `--config` flag beats `WCODE_CONFIG`; an empty
/// (or whitespace-only) env var counts as unset.
fn resolve_overlay(flag: Option<&str>, env: Option<&str>) -> Option<String> {
    flag.map(str::to_string)
        .or_else(|| env.filter(|s| !s.trim().is_empty()).map(str::to_string))
}



/// Pick the interactive front-end: the TUI when forced with `--tui`, or by
/// default on a TTY; `--no-tui` (or a pipe/CI) keeps the line REPL. One-shot
/// (`-p`) and `serve` never use the TUI.
fn choose_tui(args: &Args, tty: bool) -> bool {
    if args.no_tui || args.prompt.is_some() || args.serve {
        return false;
    }
    args.tui || tty
}

/// Both ends must be a terminal for an interactive TUI.
fn is_tty() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Default socket for `serve`/`--socket`: alongside the config, so both ends
/// agree without an argument.
fn default_socket_path() -> PathBuf {
    config_dir()
        .map(|d| d.join("wcode.sock"))
        .unwrap_or_else(|| std::env::temp_dir().join("wcode.sock"))
}

/// The `/resume` picker's list: one entry per session file, newest first, each
/// with a `id · age · first user line` label and the path to hand back for
/// `--resume`. Failures degrade to a bare file name rather than break the picker.
fn session_items() -> Vec<wcode_tui::SessionItem> {
    list_sessions(&session_dir())
        .unwrap_or_default()
        .into_iter()
        .map(|path| wcode_tui::SessionItem {
            label: session_label(&path),
            path,
        })
        .collect()
}

/// `id · age · first user line`, omitting whichever parts are unavailable.
fn session_label(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let id = name.strip_suffix(".jsonl").unwrap_or(&name);

    let mut parts = vec![id.to_string()];
    if let Some(age) = age_millis(id) {
        parts.push(humanize_age(age));
    }
    if let Some(first) = first_user_line(path) {
        parts.push(first);
    }
    parts.join(" · ")
}

/// How long ago the session was created, from the `{millis}_` name prefix.
fn age_millis(id: &str) -> Option<u64> {
    let created: u64 = id.split('_').next()?.parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(now.saturating_sub(created))
}

/// `3s` / `12m` / `5h` / `2d` — coarse enough to fit the picker row.
fn humanize_age(ms: u64) -> String {
    const MINUTE: u64 = 60 * 1000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    match ms {
        _ if ms < MINUTE => format!("{}s", ms / 1000),
        _ if ms < HOUR => format!("{}m", ms / MINUTE),
        _ if ms < DAY => format!("{}h", ms / HOUR),
        _ => format!("{}d", ms / DAY),
    }
}

/// The first line of the session's first user message, truncated — the quickest
/// way to recognize a session in the picker.
fn first_user_line(path: &Path) -> Option<String> {
    Session::open(path)
        .ok()?
        .messages()
        .into_iter()
        .find_map(|message| match message {
            AgentMessage::User { .. } => {
                let text = message.as_text();
                let line = text.lines().next().unwrap_or_default().trim();
                (!line.is_empty()).then(|| truncate_chars(line, 48))
            }
            _ => None,
        })
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `-p` mode: no streaming output; print the final assistant text.
async fn one_shot(backend: Backend, prompt: &str) -> i32 {
    let mut rx = backend.subscribe();
    // Draining keeps the subscription live and captures what the reply does not
    // carry: the last stream error and the final assistant text.
    let drain = tokio::spawn(async move {
        let mut last_error: Option<String> = None;
        let mut last_text = String::new();
        loop {
            match rx.recv().await {
                Ok(AgentEvent::Error { message }) => last_error = Some(message),
                Ok(AgentEvent::MessageEnd { message }) => last_text = message.as_text(),
                Ok(AgentEvent::AgentEnd) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        (last_error, last_text)
    });
    let reply = backend
        .ask(Request::Submit {
            text: prompt.to_string(),
        })
        .await;
    let (last_error, last_text) = drain.await.unwrap_or((None, String::new()));
    match reply {
        Ok(AgentEvent::Stopped {
            stop_reason: StopReason::Error,
        }) => {
            match last_error {
                Some(msg) => eprintln!("error: {msg}"),
                None => eprintln!("run failed"),
            }
            1
        }
        Ok(AgentEvent::Stopped {
            stop_reason: StopReason::MaxTurns,
        }) => {
            eprintln!("error: hit maximum turns; task may be incomplete");
            1
        }
        Ok(AgentEvent::Stopped { .. }) => {
            if !last_text.trim().is_empty() {
                println!("{last_text}");
            }
            0
        }
        Ok(AgentEvent::Error { message }) => {
            eprintln!("error: {message}");
            1
        }
        Ok(_) => 1,
        Err(_) => {
            eprintln!("error: session closed");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn session_label_reads_the_first_user_line_and_age() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::create_with_cwd(dir.path(), dir.path()).unwrap();
        session
            .append(wcode_harness::session::SessionEntry::Message {
                id: "m1".into(),
                parent_id: None,
                message: AgentMessage::user_text("fix the flaky test\nsecond line"),
            })
            .unwrap();
        let path = session.path().unwrap().to_path_buf();

        let label = session_label(&path);
        assert!(label.contains("fix the flaky test"), "{label}");
        assert!(!label.contains("second line"), "only the first line: {label}");
        assert!(label.contains(" · 0s · "), "a fresh session reads as 0s: {label}");
    }

    #[test]
    fn session_age_is_humanized() {
        assert_eq!(humanize_age(3_000), "3s");
        assert_eq!(humanize_age(90_000), "1m");
        assert_eq!(humanize_age(60 * 60_000), "1h");
        assert_eq!(humanize_age(2 * 24 * 60 * 60_000), "2d");
    }

    #[test]
    fn truncate_chars_keeps_short_text_and_elides_long() {
        assert_eq!(truncate_chars("short", 48), "short");
        assert_eq!(truncate_chars(&"x".repeat(60), 48).chars().count(), 48);
        assert!(truncate_chars(&"x".repeat(60), 48).ends_with('…'));
    }

    #[test]
    fn parse_defaults() {
        assert_eq!(parse_args(&args(&[])), Ok(Parsed::Args(Args::default())));
    }

    #[test]
    fn parse_prompt() {
        let Parsed::Args(a) = parse_args(&args(&["-p", "hi there"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.prompt.as_deref(), Some("hi there"));
        assert!(!a.no_session);
    }

    #[test]
    fn parse_resume_with_and_without_path() {
        let Parsed::Args(a) = parse_args(&args(&["--resume"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.resume, Some(None));

        let Parsed::Args(a) = parse_args(&args(&["--resume", "s.jsonl"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.resume, Some(Some("s.jsonl".into())));
    }

    #[test]
    fn parse_resume_ignores_next_flag() {
        // `--resume -p` = latest, not a path named "-p"
        let Parsed::Args(a) = parse_args(&args(&["--resume", "-p", "hi"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.resume, Some(None));
        assert_eq!(a.prompt.as_deref(), Some("hi"));
    }

    #[test]
    fn parse_all_flags() {
        let Parsed::Args(a) = parse_args(&args(&[
            "--no-session",
            "--model",
            "m1",
            "--base-url",
            "http://x",
        ]))
        .unwrap() else {
            panic!("not args");
        };
        assert!(a.no_session);
        assert_eq!(a.model.as_deref(), Some("m1"));
        assert_eq!(a.base_url.as_deref(), Some("http://x"));
    }

    #[test]
    fn parse_endpoint_flag() {
        let Parsed::Args(a) = parse_args(&args(&["--endpoint", "responses"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.endpoint.as_deref(), Some("responses"));
        let Parsed::Args(a) = parse_args(&args(&["--endpoint", "chat"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.endpoint.as_deref(), Some("chat"));
        assert!(parse_args(&args(&["--endpoint"])).is_err());
    }

    #[test]
    fn parse_effort_flag() {
        let Parsed::Args(a) = parse_args(&args(&["--effort", "high"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.effort, Some(Some("high".into())));
        // "-", "none" and "off" all clear back to send-nothing.
        for clear in ["-", "none", "off"] {
            let Parsed::Args(a) = parse_args(&args(&["--effort", clear])).unwrap() else {
                panic!("not args");
            };
            assert_eq!(a.effort, Some(None), "clear synonym {clear:?}");
        }
        assert!(parse_args(&args(&["--effort"])).is_err());
    }

    #[test]
    fn parse_list_models_flag() {
        let Parsed::Args(a) = parse_args(&args(&["--list-models"])).unwrap() else {
            panic!("not args");
        };
        assert!(a.list_models);
        assert!(!Args::default().list_models);
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse_args(&args(&["-h"])), Ok(Parsed::Help));
        assert_eq!(parse_args(&args(&["--help"])), Ok(Parsed::Help));
    }

    #[test]
    fn rescue_carries_env_base_url_and_flag_model() {
        // --model rescue keeps env base_url/api_key via merge(); flag model wins.
        let cfg = rescue(
            "flag-model".into(),
            FileConfig::default(),
            EnvLike {
                wcode_base_url: Some("http://env".into()),
                wcode_api_key: Some("k-env".into()),
                ..EnvLike::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.model, "flag-model");
        assert_eq!(cfg.base_url.as_deref(), Some("http://env"));
        assert_eq!(cfg.api_key.as_deref(), Some("k-env"));
    }

    #[test]
    fn rescue_carries_toml_base_url_and_api_key() {
        // toml has base_url+api_key but no model: rescue must keep both
        // (dropping them misrouted traffic to the default endpoint), with
        // the flag model filling the gap. Env still beats toml.
        let cfg = rescue(
            "flag-model".into(),
            FileConfig {
                base_url: Some("http://toml".into()),
                api_key: Some("k-toml".into()),
                model: None,
                ..FileConfig::default()
            },
            EnvLike::default(),
        )
        .unwrap();
        assert_eq!(cfg.model, "flag-model");
        assert_eq!(cfg.base_url.as_deref(), Some("http://toml"));
        assert_eq!(cfg.api_key.as_deref(), Some("k-toml"));
    }

    #[test]
    fn rescue_invalid_env_override_is_error_not_panic() {
        // A model-less config rescued by --model must still surface an
        // invalid env override (WCODE_GREP here) as a clean error — a
        // regression guard for the old `expect("model set, merge cannot fail")`.
        let e = rescue(
            "flag-model".into(),
            FileConfig::default(),
            EnvLike {
                wcode_grep: Some("banana".into()),
                ..EnvLike::default()
            },
        )
        .unwrap_err();
        let ConfigError::Io(msg) = &e else {
            panic!("wrong error: {e:?}")
        };
        assert!(msg.contains("grep"), "names the tool: {msg}");
    }

    #[test]
    fn parse_errors() {
        assert!(parse_args(&args(&["-p"])).is_err());
        assert!(parse_args(&args(&["--model"])).is_err());
        assert!(parse_args(&args(&["--base-url"])).is_err());
        assert!(parse_args(&args(&["--effort"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
        assert!(parse_args(&args(&["stray"])).is_err());
    }

    #[test]
    fn resolve_prefers_existing_path() {
        // existing paths win; missing names fall back to the session dir
        assert_eq!(resolve_session_path("/"), PathBuf::from("/"));
        let fallback = resolve_session_path("no-such-session-file.jsonl");
        assert!(fallback.starts_with(session_dir()));
    }

    #[test]
    fn choose_tui_prefers_flags_then_terminal() {
        assert!(choose_tui(&Args::default(), true));
        assert!(!choose_tui(&Args::default(), false));
        assert!(choose_tui(&Args { tui: true, ..Args::default() }, false));
        assert!(!choose_tui(&Args { no_tui: true, ..Args::default() }, true));
        assert!(!choose_tui(
            &Args {
                prompt: Some("x".into()),
                ..Args::default()
            },
            true
        ));
        assert!(!choose_tui(
            &Args {
                serve: true,
                ..Args::default()
            },
            true
        ));
    }

    #[test]
    fn parse_config_flag() {
        let Parsed::Args(a) = parse_args(&args(&["--config", ".wcode/team.toml"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.config.as_deref(), Some(".wcode/team.toml"));
        assert!(parse_args(&args(&["--config"])).is_err());
    }

    #[test]
    fn overlay_precedence_flag_beats_env() {
        assert_eq!(
            resolve_overlay(Some("flag.toml"), Some("env.toml")),
            Some("flag.toml".to_string())
        );
        assert_eq!(
            resolve_overlay(None, Some("env.toml")),
            Some("env.toml".to_string())
        );
        // An empty / whitespace-only env var counts as unset.
        assert_eq!(resolve_overlay(None, Some("   ")), None);
        assert_eq!(resolve_overlay(None, None), None);
    }

    }

#[cfg(test)]
mod instructions_flag_tests {
    use super::*;

    #[test]
    fn parse_no_instructions_flag() {
        let argv: Vec<String> = ["--no-instructions"].iter().map(|s| s.to_string()).collect();
        let Parsed::Args(a) = parse_args(&argv).unwrap() else {
            panic!("not args");
        };
        assert!(a.no_instructions);
        assert!(!Args::default().no_instructions);
    }
}
