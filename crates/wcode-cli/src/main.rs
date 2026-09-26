//! CLI entry: arg parsing, config → LlmOpts → Agent, one-shot or REPL.

use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wcode_harness::actor::SessionActor;
use wcode_harness::agent::Agent;
use wcode_harness::event::AgentEvent;
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::session::Session;
use wcode_harness::streamfn::{LlmOpts, rig_stream_fn};
use wcode_protocol::Backend;

mod agents;
mod config;
mod instructions;
mod repl;
mod rtk;
mod scheduler;
mod session_groups;
mod skills;
mod tasks;
mod tools;
mod verify_gate;
mod workspace;

use crate::config::{
    Config, ConfigError, DETECT_TIMEOUT, EnvLike, FileConfig, OPENAI_DEFAULT_BASE_URL, Source,
    TeamMember, config_dir, detect_candidates, endpoint_label, merge, parse_endpoint,
};
use crate::instructions::{InstructionSet, Mode, load as load_instructions};
use crate::repl::{
    AgentSpec, build_agent, default_hooks, list_sessions, resolve_session_path, session_dir,
    system_prompt,
};
use crate::skills::{SkillSet, discover as discover_skills};
use crate::tasks::TaskList;
use crate::tools::background::Background;

const USAGE: &str = "\
wcode — minimal coding agent

usage: wcode [-p <prompt>] [--resume [path]] [--no-session] [--model <id>] [--base-url <url>] [--config <path>] [--endpoint <chat|responses>] [--effort <level>] [--list-models] [--no-instructions] [--no-skills] [--dump-system-prompt] [--agents] [--peer <name>=<socket>] [--name <id>] [--owner <addr>] [serve] [--socket <path>] [--tui|--no-tui] [--task <text>] [--timeout <secs>]

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
  --detect-endpoint  probe common local endpoints (OLLAMA_HOST, :11434, :1234) when base_url is unset; opt-in
  --dump-config      print the resolved endpoint/base_url/model and key SOURCES (never the secret), then exit
  serve              own the session and serve it at --socket (default: ~/.config/wcode/wcode.sock)
  --socket <path>    connect to a session served elsewhere (with -p; a remote REPL is next)
  --task <text>      run a [workflow] headless on <text> (requires [workflow]
                     and --agents; WCODE_TASK is the fallback)
  --timeout <secs>   cap the headless run (0/absent = no cap)
  --tui | --no-tui   force the full-screen TUI, or the line REPL (default: TUI on a TTY)
   -V, --version      show version
   -h, --help         show this help

config: ~/.config/wcode/config.toml
  model = \"...\"      (required)
  base_url = \"...\"   (optional, any OpenAI-compatible endpoint)
  api_key = \"...\"    (optional)
  endpoint = \"...\"   (optional, chat|responses, default chat)
  effort = \"...\"     (optional, free-style reasoning effort, omitted = not sent)

  [models.\"<id>\"]     (optional; overrides the DEFAULT provider for that model id)
  endpoint = \"...\"    (optional; chat|responses — overrides the default endpoint)
  base_url = \"...\"    (optional; overrides the default base_url)
  api_key = \"...\"    (optional; overrides the default api_key)

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
env: WCODE_BASE_URL and WCODE_API_KEY override the toml globals (the DEFAULT provider); a [models.<id>] entry overrides it for that id; OPENAI_API_KEY is a key fallback
env: WCODE_ENDPOINT overrides the toml endpoint; WCODE_EFFORT overrides the toml effort; a [models.<id>] entry overrides the default for that id
env: WCODE_RTK overrides the toml hooks.rtk (auto|true|false)
env: WCODE_GREP and WCODE_FIND override the toml tools.grep/find (true|false)
env: WCODE_INSTRUCTIONS overrides the toml instructions.file (a name/path, or \"off\")
env: WCODE_SKILLS discovers skills from extra roots, or \"off\" disables
env: WCODE_TASK supplies --task when the flag is absent
env: WCODE_RETRY_MAX, WCODE_RETRY_BASE_MS, WCODE_RETRY_CAP_MS, WCODE_RETRY_TTFT_MS, WCODE_RETRY_IDLE_MS override the toml retry table";

#[derive(Debug, Default, PartialEq)]
struct Args {
    /// `--task <text>`: run a `[workflow]` headless on `<text>`. Requires
    /// `[workflow]` + `--agents`; `WCODE_TASK` is the env fallback.
    task: Option<String>,
    /// `--timeout <secs>`: cap the headless run; 0/absent = disabled.
    timeout: Option<u64>,
    prompt: Option<String>,
    /// None = flag absent; Some(None) = latest; Some(Some(path)) = that file.
    resume: Option<Option<String>>,
    no_session: bool,
    model: Option<String>,
    /// `--theme <name>`: select a catalog preset (overrides the config preset).
    theme: Option<String>,
    /// `--list-themes`: print the catalog names and exit.
    list_themes: bool,
    base_url: Option<String>,
    /// `--config <path>`: an overlay config file, deep-merged over the global one.
    config: Option<String>,
    endpoint: Option<String>,
    /// None = flag absent; Some(None) = clear; Some(Some(level)) = set.
    effort: Option<Option<String>>,
    list_models: bool,
    no_instructions: bool,
    dump_system_prompt: bool,
    /// `--detect-endpoint`: before deriving `LlmOpts`, probe common local
    /// endpoints (OLLAMA_HOST, :11434, :1234) when `base_url` is unset. Strictly
    /// opt-in (`WCODE_DETECT_ENDPOINT=1` also enables it) — no network by default.
    detect_endpoint: bool,
    /// `--dump-config`: print the resolved effective provider (sources, never a
    /// secret) and the resolved config, then exit. Touches no session.
    dump_config: bool,
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
    Version,
}

fn parse_args(args: &[String]) -> Result<Parsed, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        i += 1;
        match flag {
            "-V" | "--version" => return Ok(Parsed::Version),
            "-h" | "--help" => return Ok(Parsed::Help),
            "-p" => {
                a.prompt = Some(args.get(i).ok_or("-p requires a prompt")?.clone());
                i += 1;
            }
            "--task" => {
                a.task = Some(args.get(i).ok_or("--task requires text")?.clone());
                i += 1;
            }
            "--timeout" => {
                let v = args.get(i).ok_or("--timeout requires seconds")?;
                a.timeout = Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("--timeout expects seconds, got `{v}`"))?,
                );
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
            "--theme" => {
                a.theme = Some(args.get(i).ok_or("--theme requires a name")?.clone());
                i += 1;
            }
            "--list-themes" => a.list_themes = true,
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
            "--detect-endpoint" => a.detect_endpoint = true,
            "--dump-config" => a.dump_config = true,
            "--no-skills" => a.no_skills = true,
            "--sequential" => a.sequential = true,
            "--agents" => a.agents = true,
            "--peer" => {
                a.peers.push(
                    args.get(i)
                        .ok_or("--peer requires <name>=<socket>")?
                        .clone(),
                );
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

// ---------------------------------------------------------------------------
// main() phases. Pure decomposition — each helper keeps the exact `eprintln!`
// text, `std::process::exit` code, and side-effect order of the pre-split
// `main()`. `std::process::exit` stays inside the helper that used it.
// ---------------------------------------------------------------------------

/// P1: read `argv` (skip(1)) and parse it, resolving `--help`/`-h`. The only
/// argv reader; both exits are byte-identical to the pre-split `main`.
fn parse_cli() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse_args(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    match parsed {
        Parsed::Version => {
            println!("wcode {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        Parsed::Help => {
            print!("{USAGE}");
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        Parsed::Args(a) => a,
    }
}

/// P2: resolve the `--config`/`WCODE_CONFIG` overlay, load the config (rescuing
/// a model-less file when `--model`/`--dump-system-prompt` is set), apply the
/// flag overrides in order, then derive the `LlmOpts`. `to_llm_opts()` runs
/// LAST, after every override, so the launch provider it captures already
/// includes the flags; `settle_provider()` then overlays the selected model's
/// `[models.<id>]` profile (precedence: profile > flag > env/config globals).
/// Where opt-in detection is enabled: the flag, or `WCODE_DETECT_ENDPOINT`.
fn detect_opt_in(args: &Args) -> bool {
    args.detect_endpoint || env_detect_flag(std::env::var("WCODE_DETECT_ENDPOINT").ok().as_deref())
}

/// The env half of [`detect_opt_in`], split out so it is testable without
/// touching the process environment.
fn env_detect_flag(value: Option<&str>) -> bool {
    matches!(value, Some("1") | Some("true") | Some("yes"))
}

/// Skip probing when the SELECTED model already pins a `[models.<id>].base_url`
/// — a profile base beats anything detection could set. The launch base still
/// matters for a later switch to an *unmapped* model, so detection's result is
/// only a DEFAULT; a mapped model is never overridden by it.
fn selected_model_pins_base(cfg: &Config) -> bool {
    cfg.models
        .get(&cfg.model)
        .is_some_and(|profile| profile.base_url.is_some())
}

/// Try the CLI candidate list (`config::detect_candidates`) in order via the
/// harness probe (`streamfn::probe_endpoint`); the first answer wins. `None`
/// when none answers. Bounded by `config::DETECT_TIMEOUT` per site.
async fn detect_endpoint() -> Option<String> {
    for candidate in detect_candidates(&EnvLike::from_env()) {
        if wcode_harness::streamfn::probe_endpoint(&candidate, DETECT_TIMEOUT).await {
            return Some(candidate);
        }
    }
    None
}

/// Visibility (unconditional): one provider line to STDERR (never stdout — it
/// would corrupt `-p` output and the TUI alt-screen). Warns when `base_url` is
/// effective base is implicit (profile-aware), so the fallback to
/// api.openai.com is no longer silent and a pinned model is never misreported.
fn print_provider_diagnostic(cfg: &Config) {
    eprintln!("provider: {}", cfg.provider_summary());
    if cfg.effective_base_url_is_implicit(&cfg.model) {
        eprintln!(
            "warning: base_url is unset — requests go to {OPENAI_DEFAULT_BASE_URL} (set base_url or WCODE_BASE_URL)"
        );
    }
}

/// `--dump-config` (phase 3a): print the resolved config and exit — provider /
/// model / key SOURCES only, NEVER a key value. Touches no session.
fn print_config(cfg: &Config) -> ! {
    print!("{}", config_dump(cfg));
    std::process::exit(0)
}

/// The `--dump-config` body. An explicit field list (never `{cfg:?}`), and every
/// key slot rendered as `(set)`/`(none)` — the secret never reaches the output.
fn config_dump(cfg: &Config) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "provider: {}", cfg.provider_summary());
    let _ = writeln!(
        out,
        "endpoint: {} (source: {})",
        endpoint_label(cfg.endpoint),
        cfg.provenance.endpoint
    );
    let effective_base = cfg.effective_base_url(&cfg.model);
    let base_source = if cfg
        .models
        .get(&cfg.model)
        .is_some_and(|profile| profile.base_url.is_some())
    {
        "model profile".to_string()
    } else {
        cfg.provenance.base_url.to_string()
    };
    let _ = writeln!(
        out,
        "base_url: {} (source: {base_source})",
        effective_base.unwrap_or(OPENAI_DEFAULT_BASE_URL)
    );
    let _ = writeln!(out, "model: {} (source: {})", cfg.model, cfg.provenance.model);
    let _ = writeln!(
        out,
        "api_key: {} (source: {})",
        redact_key(cfg.api_key.as_deref()),
        cfg.provenance.api_key
    );
    let _ = writeln!(out, "effort: {}", cfg.effort.as_deref().unwrap_or("(none)"));
    for (id, profile) in &cfg.models {
        let endpoint = profile.endpoint.map(endpoint_label).unwrap_or("(inherit)");
        let _ = writeln!(
            out,
            "models.{id}: endpoint {endpoint} · base_url {} · api_key {}",
            profile.base_url.as_deref().unwrap_or("(inherit)"),
            redact_key(profile.api_key.as_deref())
        );
    }
    for member in &cfg.team {
        let _ = writeln!(
            out,
            "team.{}: model {} · base_url {} · api_key {}",
            member.name,
            member.model.as_deref().unwrap_or("(inherit)"),
            member.base_url.as_deref().unwrap_or("(inherit)"),
            redact_key(member.api_key.as_deref())
        );
    }
    out
}

/// `(set)`/`(none)` for a key slot — never the value itself.
fn redact_key(key: Option<&str>) -> &'static str {
    if key.is_some() { "(set)" } else { "(none)" }
}
fn load_config_raw(args: &Args) -> Config {
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
        cfg.provenance.model = Source::Flag;
    }
    // Sweep spill files a previous run left behind (> 24h old) — best-effort,
    // once per process, before agent construction on every path.
    let _ = crate::tools::bash::sweep_stale_spills(&crate::tools::bash::spill_root());
    // Flags land on the config FIRST, so the launch provider `to_llm_opts`
    // captures (what an unmapped model — or a later switch — falls back to)
    // already includes them. The loud `--endpoint` failure is preserved.
    if let Some(u) = args.base_url.clone() {
        cfg.base_url = Some(u);
        cfg.provenance.base_url = Source::Flag;
    }
    if let Some(e) = args.endpoint.as_deref() {
        match parse_endpoint(Some(e)) {
            Ok(endpoint) => {
                cfg.endpoint = endpoint;
                cfg.provenance.endpoint = Source::Flag;
            }
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
    // `--theme <name>` overrides the config PRESET but keeps the config's role
    // keys (mirrors the runtime `/theme` composition).
    if let Some(name) = &args.theme {
        cfg.theme = match cfg.theme.clone().with_preset(name) {
            Ok(spec) => spec,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
        };
    }
    cfg
}

/// P3a: `--dump-system-prompt` — print the composed prompt (instructions
/// included) and exit. Needs no endpoint, model, or session. Diverges.
fn print_system_prompt(args: &Args, cfg: &Config) -> ! {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mode = if args.no_instructions {
        Mode::Off
    } else {
        cfg.instructions.mode()
    };
    let instructions = load_instructions(&mode, &cwd, config_dir().as_deref());
    let skills = discover_skills_for(args, cfg, &cwd);
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
    std::process::exit(0)
}

/// P3b: `--list-models` — resolve against the same base_url/key as chat, print
/// sorted ids (`*` marks the configured model), exit. No session touched.
/// Diverges.
async fn list_models_and_exit(llm: &LlmOpts) -> ! {
    match wcode_harness::streamfn::list_models(llm).await {
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
    std::process::exit(0)
}

/// `branch`, with a trailing `*` when the worktree is dirty; `None` outside a
/// work tree (or when git is absent). Best-effort: any probe failure is `None`.
fn git_branch_dirty() -> Option<String> {
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())?;
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    Some(if dirty { format!("{branch}*") } else { branch })
}

/// P3c: `--socket` (without `serve`): connect to a session served elsewhere and
/// drive it (one-shot, remote TUI, or remote line REPL). Every handled path
/// diverges; returns `false` to fall through when no socket is being handled.
async fn run_socket_client(args: &Args, cfg: &Config, llm: &LlmOpts) -> bool {
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
        // The server's roster (root first). A failed ask keeps today's single
        // connection-wide view.
        let roster = match client.ask(Request::ListSessions).await {
            Ok(AgentEvent::Sessions { sessions }) => sessions,
            _ => Vec::new(),
        };
        match args.prompt.clone() {
            Some(prompt) => {
                let backend = match roster.first() {
                    Some(root) => Backend::from(client.with_session(root.id.clone())),
                    None => Backend::from(client),
                };
                let code = one_shot(backend, &prompt).await;
                // Write any queued fire-and-forget frames (a `message`/`spawn`
                // sent during the turn) before the runtime is dropped.
                wcode_protocol::flush_all(wcode_protocol::FLUSH_TIMEOUT).await;
                // A socket client owns no local Background, but the seam is
                // shared: signal any live group before exit (D8).
                Background::shutdown_all();
                std::process::exit(code)
            }
            None => {
                if choose_tui(args, is_tty()) {
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
                        plan: false,
                    };
                    let cwd = std::env::current_dir().ok().and_then(|p| {
                        p.file_name().map(|n| n.to_string_lossy().into_owned())
                    });
                    let git = git_branch_dirty();
                    let options = wcode_tui::Options {
                        status,
                        models: wcode_harness::streamfn::list_models(llm)
                            .await
                            .unwrap_or_default(),
                        // The server owns the session; a socket client cannot see
                        // the session dir, so `/resume` has nothing to offer.
                        sessions: Vec::new(),
                        tasks: Vec::new(),
                        theme: cfg.theme.clone(),
                        history: Some(repl::history_path()),
                        remote: true,
                        cwd,
                        git,
                    };
                    // One surface per served session (root first), all over the
                    // one connection. A failed roster falls back to the legacy
                    // single connection-wide root surface.
                    let root_id = roster.first().map(|info| info.id.clone());
                    let surfaces: Vec<wcode_tui::SurfaceSpec> = if roster.is_empty() {
                        vec![wcode_tui::SurfaceSpec {
                            id: SessionId::agent("root"),
                            label: "root".to_string(),
                            model: llm.model.clone(),
                            is_root: true,
                            backend: Backend::from(client.clone()),
                        }]
                    } else {
                        let root = &roster[0].id;
                        roster
                            .iter()
                            .map(|info| wcode_tui::SurfaceSpec {
                                id: info.id.clone(),
                                label: crate::agents::short_name(&info.id),
                                model: info.model.clone().unwrap_or_else(|| llm.model.clone()),
                                is_root: &info.id == root,
                                backend: Backend::from(client.with_session(info.id.clone())),
                            })
                            .collect()
                    };
                    // Runtime-spawned sessions reach the TUI through this feed:
                    // the server pushes the connection-wide roster when it grows,
                    // and a task emits a surface for each id not already seeded
                    // from the initial `ListSessions`.
                    let new_surfaces = {
                        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                        let mut roster_rx = client.subscribe_roster();
                        let mut seeded: std::collections::HashSet<SessionId> =
                            roster.iter().map(|info| info.id.clone()).collect();
                        let client = client.clone();
                        let model = llm.model.clone();
                        tokio::spawn(async move {
                            loop {
                                for info in roster_rx.borrow_and_update().clone() {
                                    if !seeded.insert(info.id.clone()) {
                                        continue;
                                    }
                                    let spec = wcode_tui::SurfaceSpec {
                                        id: info.id.clone(),
                                        label: crate::agents::short_name(&info.id),
                                        model: info.model.clone().unwrap_or_else(|| model.clone()),
                                        is_root: Some(&info.id) == root_id.as_ref(),
                                        backend: Backend::from(
                                            client.with_session(info.id.clone()),
                                        ),
                                    };
                                    // A closed receiver means the TUI has exited.
                                    if tx.send(spec).is_err() {
                                        return;
                                    }
                                }
                                if roster_rx.changed().await.is_err() {
                                    return;
                                }
                            }
                        });
                        Some(rx)
                    };
                    // Tasks are unavailable across a socket: the server owns the
                    // plan, and the client holds no `TaskList`.
                    let new_tasks = None;
                    match wcode_tui::run(surfaces, options, new_surfaces, new_tasks).await {
                        Ok(wcode_tui::Outcome::Quit) => {
                            // A socket client owns no local Background, but the
                            // seam is shared: signal any live group before exit (D8).
                            Background::shutdown_all();
                            std::process::exit(0)
                        }
                        Ok(wcode_tui::Outcome::Abandoned) => {
                            // A force-quit abandoned the run: no relaunch banner,
                            // just kill any live background groups and exit 0.
                            Background::shutdown_all();
                            std::process::exit(0)
                        }
                        Ok(wcode_tui::Outcome::Reload { .. }) => {
                            eprintln!("tui: cannot reload over a socket");
                            std::process::exit(1);
                        }
                        Ok(wcode_tui::Outcome::Resume(_)) => {
                            eprintln!("tui: cannot resume over a socket");
                            std::process::exit(1);
                        }
                        Ok(wcode_tui::Outcome::New) => {
                            eprintln!("tui: cannot start a new session over a socket");
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
                    match roster.first() {
                        Some(root) => {
                            repl::SessionSource::Remote(client.with_session(root.id.clone()))
                        }
                        None => repl::SessionSource::Remote(client),
                    },
                    llm.clone(),
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
                    cfg.workspace.digest_cas,
                )
                .await;
                std::process::exit(0)
            }
        }
    }
    #[cfg(not(unix))]
    if !args.serve && args.socket.is_some() {
        eprintln!("error: `--socket` is not supported on this platform");
        std::process::exit(2);
    }
    false
}

/// P4: the session a run resumes or creates, plus the working dir and the group
/// state the later phases need.
struct SessionSetup {
    cwd: PathBuf,
    active_group: Option<session_groups::SessionGroup>,
    resuming_group: bool,
    session: Option<Session>,
    context: Vec<AgentMessage>,
}

/// P4: compute the working dir; on `--resume` resolve and open the session (a
/// group dir resumes its whole team; a bare flat file resumes as before),
/// restoring the session's model/effort unless the flag was set; otherwise
/// create a fresh group unless `--no-session`. Mutates `llm` in place because
/// the resume branch restores model/effort from the session.
fn load_session(args: &Args, llm: &mut LlmOpts) -> SessionSetup {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    // The active session group, when this run owns one (a fresh root always
    // creates one; a resumed group sets it below). Threaded into the worker
    // template so spawned/rebuilt members persist into its `members/`.
    let mut active_group: Option<session_groups::SessionGroup> = None;
    // True when the root came back from a group dir: the team is rebuilt from
    // the files (not `[team]`), after the orchestrator exists.
    let mut resuming_group = false;
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
            // A group dir (or a `<dir>/root.jsonl` from a `/reload`, 6b) resumes
            // the whole team; a bare flat file resumes as today.
            match session_groups::group_dir_of(&p) {
                Some(dir) => match session_groups::open_group(&dir) {
                    Ok(group) => match session_groups::load_root(&group) {
                        Ok((s, ctx)) => {
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
                            resuming_group = true;
                            active_group = Some(group);
                            (Some(s), ctx)
                        }
                        Err(e) => {
                            eprintln!("error: open {}: {e}", group.root().display());
                            std::process::exit(1);
                        }
                    },
                    Err(e) => {
                        eprintln!("error: open group {}: {e}", dir.display());
                        std::process::exit(1);
                    }
                },
                None => match Session::open(&p) {
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
                },
            }
        }
        None => match (!args.no_session).then(|| {
            // Every fresh root is a session group (Q(G)): a `<millis>_<id8>/`
            // dir holding `root.jsonl` (and a `members/` for any team). A group
            // with no members resumes exactly as a bare root.
            session_groups::create_group(&session_dir()).and_then(|group| {
                let session = session_groups::create_root_session(&group.dir, &cwd)?;
                Ok((group, session))
            })
        }) {
            Some(Ok((group, s))) => {
                active_group = Some(group);
                (Some(s), Vec::new())
            }
            Some(Err(e)) => {
                eprintln!("error: create session: {e}");
                std::process::exit(1);
            }
            None => (None, Vec::new()),
        },
    };
    SessionSetup {
        cwd,
        active_group,
        resuming_group,
        session,
        context,
    }
}

/// P5: the run's discovered context — instructions, skills, hooks, and the
/// orchestrator wiring (present when `--agents`).
struct Runtime {
    instructions: InstructionSet,
    skills: SkillSet,
    hooks: wcode_harness::hooks::HooksSet,
    orchestrator: Option<crate::agents::Orchestrator>,
}

/// P5: load instructions (`--no-instructions` → `Mode::Off`) and skills; build
/// the hooks; build the orchestrator when `--agents` (its `WorkerTemplate` needs
/// `cwd` and `setup.active_group.members_dir`). Then the guards: `--peer` /
/// `[team]` / non-empty `[orchestrator].guidelines` each `exit(2)` without
/// `--agents`; register `--peer` remotes; non-unix `--peer` `exit(2)`; register
/// `[peers]` aliases/remotes; rebuild a resumed group's team; spawn `[team]`
/// members; print the `team: ...` line.
/// The `--task`/`--timeout` boot guards (§4.5): `Err(message)` for a
/// misconfiguration the caller prints and exits 2 on. Pure, so each case is
/// unit-tested; the `eprintln!` + `exit(2)` shape lives at the call site.
fn check_task_args(
    args: &Args,
    workflow: Option<&crate::config::Workflow>,
    has_agents: bool,
) -> Result<(), String> {
    if args.task.is_some() {
        if workflow.is_none() {
            return Err("--task requires [workflow]".into());
        }
        if args.prompt.is_some() {
            return Err("--task cannot be combined with -p".into());
        }
        if !has_agents {
            return Err("--task requires --agents".into());
        }
    }
    if args.timeout.is_some() && args.task.is_none() {
        return Err("--timeout requires --task".into());
    }
    if workflow.is_some_and(|w| w.uses_task()) && args.task.is_none() {
        return Err("[workflow] uses {{task}} but no --task/WCODE_TASK was given".into());
    }
    Ok(())
}

fn build_runtime(args: &Args, cfg: &Config, llm: &LlmOpts, setup: &SessionSetup) -> Runtime {
    let cwd = &setup.cwd;
    // Instruction ("reference") files: discover the configured candidates
    // from the working dir up to the repo root (plus the config-dir global
    // file), unless --no-instructions disables it.
    let mode = if args.no_instructions {
        Mode::Off
    } else {
        cfg.instructions.mode()
    };
    let instructions = load_instructions(&mode, cwd, config_dir().as_deref());
    let skills = discover_skills_for(args, cfg, cwd);

    let mut hooks = default_hooks(&cfg.hooks);
    // A2A (opt-in): an orchestrator wiring — the registry + factory + the root's
    // `spawn`/`message` tools. Only the root gets them, so only it spawns
    // (§10.1).
    let orchestrator = args.agents.then(|| {
        let template = crate::agents::WorkerTemplate {
            system: system_prompt(&cfg.tools, &instructions, &skills, &[], None, cwd),
            llm: llm.clone(),
            stream_fn: rig_stream_fn(),
            hooks: hooks.clone(),
            tools: cfg.tools,
            compaction: cfg.compaction,
            working_dir: cwd.clone(),
            members_dir: setup.active_group.as_ref().map(|g| g.members_dir.clone()),
            digest_cas: cfg.workspace.digest_cas,
            sessions_dir: crate::repl::session_dir(),
        };
        // Build the plan BEFORE the orchestrator (so `--resume` reloads it and a
        // fresh `[team]` session journals to `<groupdir>/plan.ndjson`) and, hard
        // constraint, BEFORE `spawn_scheduler`: `load`'s reconcile turns every
        // `Doing` → `Todo` while the sink is active — start the scheduler first and
        // a `Doing` node never re-enters `ready_ids()`.
        // `[workflow] max_attempts` (else 3) reaches the journaled list BEFORE any
        // mutation — both the fresh and the reloaded path.
        let cap = cfg.workflow.as_ref().and_then(|w| w.max_attempts).unwrap_or(3);
        let tasks = match &setup.active_group {
            Some(group) => {
                let path = group.plan_path();
                if path.exists() {
                    TaskList::load_at(&path, cap).unwrap_or_else(|e| {
                        eprintln!("warning: plan not loaded: {e}");
                        TaskList::with_journal_at(path, cap)
                    })
                } else {
                    TaskList::with_journal_at(path, cap)
                }
            }
            None => TaskList::with_sink(None, false, cap),
        };
        crate::agents::Orchestrator::with_tasks(wcode_protocol::Registry::new(), template, tasks)
    });

    // C4 — the verify gate (`VerifyGateHooks`, `crate::verify_gate`): once the
    // root holds a plan, refuse `task{op:"complete"}` for a `Session` work node
    // that no gate consumes (rule R′). Pushed AFTER the `WorkerTemplate` cloned
    // `hooks` (`GkFtU`), so only the ROOT gates — a worker has no `task` tool.
    // The set rides `Runtime.hooks` (`V5hC5`) into the root agent
    // (`build_agent_for`, `XEp0a`) and `repl::run`'s rebuild set (`kywRV` ->
    // repl.rs 813/969/1018), so it survives `/new`/`/resume`. `spawn_scheduler`
    // gets a clone too (`WFtxI`/`kDC4A`): a harmless no-op (it only ever pipes
    // `bash`). A run with no `--agents`, or a socket client (`Qr5L7`, no
    // orchestrator), never gets it.
    if let Some(o) = &orchestrator {
        // `agents.rs:IueNb` (the `Orchestrator`); `n9aLi` = `Orchestrator::tasks()`.
        hooks.push(std::sync::Arc::new(
            crate::verify_gate::VerifyGateHooks::new(o.tasks().clone()),
        ));
    }
    // Register remote peers (`--peer name=socket`): a served session becomes an
    // addressable A2A peer reachable over its socket (S4-4).
    if !args.peers.is_empty() && orchestrator.is_none() {
        eprintln!("error: --peer requires --agents");
        std::process::exit(2);
    }
    if let Err(msg) = check_task_args(args, cfg.workflow.as_ref(), orchestrator.is_some()) {
        eprintln!("error: {msg}");
        std::process::exit(2);
    }
    if cfg.workflow.is_some() && orchestrator.is_none() {
        eprintln!("error: [workflow] requires --agents");
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
    // A resumed group's members come from its files, not `[team]`: rebuild the
    // team through the orchestrator's own factory/phonebook (amendment 6a),
    // seeding each member with its full persisted transcript (D1). Deferred to
    // here because `rebuild_team` needs the orchestrator (friction #3).
    if setup.resuming_group
        && let (Some(group), Some(o)) = (&setup.active_group, &orchestrator)
    {
        let mut names = Vec::new();
        let mut seeded = 0usize;
        for outcome in session_groups::rebuild_team(group, o, o.id()) {
            match outcome {
                session_groups::MemberResume::Restored { id, messages } => {
                    seeded += messages;
                    names.push(crate::agents::short_name(&id));
                }
                session_groups::MemberResume::Skipped { name, reason } => {
                    eprintln!("warning: member `{name}` not resumed: {reason}");
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
    // `[team]` (F3): spawn each member through the orchestrator, which validates
    // the tool allow-list (D14) and registers the name in the phonebook. Only the
    // root orchestrator spawns — a served `--owner` worker does not.
    if args.owner.is_none()
        && !setup.resuming_group
        && let Some(o) = &orchestrator
    {
        for member in &cfg.team {
            let spec = crate::agents::WorkerSpec {
                name: Some(member.name.clone()),
                model: member.model.clone(),
                system: member.role.clone(),
                tools: member.tools.clone(),
                base_url: member.base_url.clone(),
                api_key: member.api_key.clone(),
                read_only: member.read_only,
                effort: member.effort.clone(),
            };
            if let Err(e) = o.spawn_worker(spec) {
                eprintln!("error: team member `{}`: {e}", member.name);
                std::process::exit(2);
            }
        }
    }
    if args.owner.is_none() && !cfg.team.is_empty() && !setup.resuming_group {
        let names: Vec<&str> = cfg.team.iter().map(|m| m.name.as_str()).collect();
        println!("team: {}", names.join(", "));
    }
    // Instantiate a `[workflow]` template AFTER the `[team]` loop (so member names
    // resolve) and BEFORE the scheduler spawn. Nodes are created in TOPOLOGICAL
    // order (Blocker 1), mapping each id to its numeric Task id before resolving
    // the deps.
    if should_instantiate_workflow(setup.resuming_group, cfg.workflow.is_some())
        && let Some(o) = &orchestrator
        && let Some(workflow) = &cfg.workflow
    {
        let n = instantiate_workflow(o.tasks(), workflow, args.task.as_deref());
        println!("workflow: {n} nodes");
    }
    // Spawn the DAG scheduler here — after the `[team]` loop and before
    // `Runtime { .. }`, so it is live for one-shot / REPL / TUI alike. The
    // `[team]` members are registered+owned by now, so their ownership edges
    // exist; `Registry::resolve` needs no registered root, so an early spawn is
    // safe (a dispatch simply stays `Todo` if nothing can be delivered). A sync
    // call inside `build_runtime`, itself run under `#[tokio::main]`, so a
    // runtime is present. A served `--owner` worker (no orchestrator) spawns
    // nothing.
    if let Some(o) = &orchestrator {
        o.spawn_scheduler(
            cwd.clone(),
            hooks.clone(),
            tokio_util::sync::CancellationToken::new(),
        );
    }
    Runtime {
        instructions,
        skills,
        hooks,
        orchestrator,
    }
}

/// The root's team and guidelines: `&[]`/`None` for a served `--owner` worker,
/// `cfg.team`/`cfg.orchestrator.guidelines` otherwise. A borrow of `cfg`, so
/// `dispatch` can pass it alongside the owned `llm`/`rt`/`agent` it moves into
/// `repl::run`. Copy so both `build_agent_for` and `dispatch` can take it.
#[derive(Clone, Copy)]
struct RootCtx<'a> {
    team: &'a [TeamMember],
    guidelines: Option<&'a str>,
}

/// P6: build the root agent from the runtime, session, and context. A session
/// with `--owner` is a served worker: push the `ReportBack` hook and take the
/// worker tools (`--owner` without `--agents` → `exit(2)`). `root_team` /
/// `root_guidelines` are computed in `main` (not here) because `dispatch`
/// (P8) also needs them.
fn build_agent_for(
    args: &Args,
    cfg: &Config,
    llm: &LlmOpts,
    rt: &Runtime,
    root: RootCtx<'_>,
    session: Option<Session>,
    context: Vec<AgentMessage>,
) -> (Agent, Arc<Background>) {
    // A session with `--owner` is a **served worker** (S4-4 reply): it gets a
    // `message` tool bound to its owner and a `ReportBack` hook, and the
    // ownership edge is recorded so its report is permitted.
    let mut agent_hooks = rt.hooks.clone();
    let extra_tools = match (&rt.orchestrator, &args.owner) {
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
    build_agent(
        AgentSpec {
            llm: llm.clone(),
            hooks: agent_hooks,
            tools: &cfg.tools,
            compaction: cfg.compaction,
            instructions: &rt.instructions,
            skills: &rt.skills,
            team: root.team,
            guidelines: root.guidelines,
        },
        session,
        context,
        extra_tools,
        cfg.workspace.digest_cas,
    )
}

/// P7: `serve` — spawn the `SessionActor`, register the root, build the
/// registry/roster/`define` handler, print the `serving ...` lines, and run
/// `serve_at`. Non-unix `serve` is `exit(2)`. Diverges.
async fn serve(
    args: &Args,
    llm: &LlmOpts,
    agent: Agent,
    bg: Arc<Background>,
    orchestrator: &Option<crate::agents::Orchestrator>,
) -> ! {
    #[cfg(unix)]
    {
        let session_id = SessionId::new(agent.session_id().unwrap_or_else(|| "local".to_string()));
        let path = args
            .socket
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(default_socket_path);
        let handle = SessionActor::spawn(agent);
        bg.bind(handle.clone());
        if let Some(o) = orchestrator {
            o.register_root(handle.clone());
        }
        // Serve the root first (labelled by its session id), then the live
        // team: the registry's local sessions minus the root's own
        // `agent:orchestrator` alias. The roster stays live, so a worker
        // spawned at runtime is served without a restart.
        // The registry the server reads each served session's model from, so
        // its `Sessions` push names the models it knows (S2).
        let registry = orchestrator
            .as_ref()
            .map(|o| o.registry().clone())
            .unwrap_or_default();
        // Record the **root**'s effective model too, keyed by the id the server
        // pushes first — `session_id`, this agent's own session, *not* the
        // registry's `agent:orchestrator` alias (a distinct id, never served).
        // Only with `--agents`: without it there is no orchestrator and the
        // registry stays a bare `Registry::default()`, so nothing is registered
        // and a client falls back to its own model (acceptable).
        if orchestrator.is_some() {
            registry.set_model(session_id.clone(), llm.model.clone());
        }
        let roster = match orchestrator {
            Some(o) => live_roster(o.registry(), o.id().clone()),
            None => tokio::sync::watch::channel(Vec::new()).1,
        };
        let ids: Vec<String> = std::iter::once(session_id.to_string())
            .chain(roster.borrow().iter().map(|(id, _)| id.to_string()))
            .collect();
        // With `--agents`, the served root can define workers for a peer:
        // install the handler over this orchestrator's factory. Without it a
        // `Define` is refused cleanly (the protocol default). The factory
        // registers into the same registry the roster watches, so a defined
        // worker is pushed to every client with no extra work.
        let define = orchestrator.as_ref().map(|o| {
            let o = o.clone();
            std::sync::Arc::new(move |args: wcode_protocol::DefineArgs| {
                o.spawn_worker(crate::agents::WorkerSpec {
                    name: args.name,
                    model: args.model,
                    system: args.role,
                    tools: args.tools,
                    base_url: args.base_url,
                    api_key: args.api_key,
                    read_only: args.read_only,
                    effort: args.effort.clone(),
                })
                .map(|worker| worker.id)
            }) as wcode_protocol::DefineHandler
        });
        println!("serving session on {}", path.display());
        println!("serving {} session(s): {}", ids.len(), ids.join(", "));
        let _ = std::io::stdout().flush();
        if let Err(e) =
            wcode_protocol::serve_at(registry, roster, (session_id, handle), define, &path).await
        {
            eprintln!("serve: {e}");
            std::process::exit(1);
        }
        std::process::exit(0)
    }
    #[cfg(not(unix))]
    {
        eprintln!("error: `serve` is not supported on this platform");
        std::process::exit(2)
    }
}

/// P8: dispatch the constructed agent — one-shot (`-p`), the TUI, or the line
/// REPL. `root_team`/`root_guidelines` borrow `cfg`; `llm`, `rt` and `agent` are
/// moved into `repl::run`. The one-shot and TUI branches diverge; the line REPL
/// falls through and returns, exactly as before the split.
#[allow(clippy::too_many_arguments)]
async fn dispatch(
    args: &Args,
    cfg: &Config,
    llm: LlmOpts,
    rt: Runtime,
    root: RootCtx<'_>,
    setup: &SessionSetup,
    agent: Agent,
    bg: Arc<Background>,
) -> () {
    if let Some(prompt) = &args.prompt {
        let handle = SessionActor::spawn(agent);
        bg.bind(handle.clone());
        if let Some(o) = &rt.orchestrator {
            o.register_root(handle.clone());
        }
        let code = one_shot(Backend::from(handle), prompt).await;
        // Flush queued fire-and-forget deliveries to any `--peer`/`[peers]`
        // remotes (a `message`/`spawn { to }` sent during the turn) before
        // the runtime is dropped.
        #[cfg(unix)]
        wcode_protocol::flush_all(wcode_protocol::FLUSH_TIMEOUT).await;
        Background::shutdown_all();
        std::process::exit(code)
    } else if args.task.is_some() {
        let handle = SessionActor::spawn(agent);
        bg.bind(handle.clone());
        if let Some(o) = &rt.orchestrator {
            o.register_root(handle.clone());
        }
        let tasks = rt
            .orchestrator
            .as_ref()
            .expect("--task guard ensured --agents")
            .tasks()
            .clone();
        let seed = workflow_seed(args.task.as_deref().unwrap_or_default());
        let timeout = args
            .timeout
            .filter(|&s| s > 0)
            .map(std::time::Duration::from_secs);
        let code = run_workflow(Backend::from(handle), tasks, seed, timeout).await;
        #[cfg(unix)]
        wcode_protocol::flush_all(wcode_protocol::FLUSH_TIMEOUT).await;
        Background::shutdown_all();
        std::process::exit(code)
    } else {
        if choose_tui(args, is_tty()) {
            let status = wcode_tui::Status {
                model: llm.model.clone(),
                effort: llm.effort.clone(),
                session: agent.session_id(),
                context_limit: wcode_harness::limits::model_limit(
                    llm.base_url.as_deref(),
                    &llm.model,
                )
                .map(|l| l.context),
                plan: false,
            };
            let cwd = std::env::current_dir().ok().and_then(|p| {
                p.file_name().map(|n| n.to_string_lossy().into_owned())
            });
            let git = git_branch_dirty();
            let options = wcode_tui::Options {
                status,
                models: wcode_harness::streamfn::list_models(&llm)
                    .await
                    .unwrap_or_default(),
                sessions: session_items(),
                tasks: task_items(&rt.orchestrator),
                theme: cfg.theme.clone(),
                history: Some(repl::history_path()),
                remote: false,
                cwd,
                git,
            };
            let session_path = agent.session_path().map(Path::to_path_buf);
            let handle = SessionActor::spawn(agent);
            bg.bind(handle.clone());
            if let Some(o) = &rt.orchestrator {
                o.register_root(handle.clone());
            }
            // Surface list: the root, then one per team member — only for
            // the root orchestrator (a served `--owner` worker has no team).
            let mut surfaces = vec![wcode_tui::SurfaceSpec {
                id: rt
                    .orchestrator
                    .as_ref()
                    .map(|o| o.id().clone())
                    .unwrap_or_else(|| SessionId::agent("root")),
                label: "root".to_string(),
                model: llm.model.clone(),
                is_root: true,
                backend: Backend::from(handle),
            }];
            if args.owner.is_none()
                && let Some(o) = &rt.orchestrator
            {
                // A resumed group's members come from its files; otherwise
                // from `[team]`. Either way the phonebook resolves each name.
                let member_names: Vec<String> =
                    match (setup.resuming_group, &setup.active_group) {
                        (true, Some(group)) => session_groups::scan_members(group)
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|p| {
                                p.file_stem().and_then(|s| s.to_str()).map(str::to_string)
                            })
                            .collect(),
                        _ => cfg.team.iter().map(|m| m.name.clone()).collect(),
                    };
                for name in member_names {
                    if let Some(backend) = o.worker_backend(&name) {
                        let model = match (setup.resuming_group, &setup.active_group) {
                            (true, Some(group)) => {
                                session_groups::member_spec(group, &name).model
                            }
                            _ => cfg
                                .team
                                .iter()
                                .find(|m| m.name == name)
                                .and_then(|m| m.model.clone()),
                        };
                        surfaces.push(wcode_tui::SurfaceSpec {
                            id: SessionId::agent(&name),
                            label: name.clone(),
                            model: model.unwrap_or_else(|| llm.model.clone()),
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
                match &rt.orchestrator {
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
            // Seed the plan and forward live updates: the TUI holds no
            // `TaskList`, so the composition root maps each snapshot to the
            // `TaskItem` view type over a feed shaped like `new_surfaces`.
            let new_tasks = rt.orchestrator.as_ref().map(|o| {
                let (tx, rx) =
                    tokio::sync::mpsc::unbounded_channel::<Vec<wcode_tui::TaskItem>>();
                let mut updates = o.tasks().subscribe();
                tokio::spawn(async move {
                    loop {
                        let items: Vec<wcode_tui::TaskItem> = updates
                            .borrow_and_update()
                            .iter()
                            .map(task_item)
                            .collect();
                        // A closed receiver means the TUI has exited.
                        if tx.send(items).is_err() {
                            return;
                        }
                        if updates.changed().await.is_err() {
                            return;
                        }
                    }
                });
                rx
            });
            match wcode_tui::run(surfaces, options, new_surfaces, new_tasks).await {
                Ok(wcode_tui::Outcome::Quit) => {
                    // The terminal is already restored: print the exact
                    // command to bring this session back (nothing when there
                    // is no local session path).
                    repl::print_relaunch(
                        &llm,
                        session_path.as_deref(),
                        args.agents,
                        args.config.as_deref(),
                        args.owner.as_deref(),
                        args.name.as_deref(),
                    );
                    Background::shutdown_all();
                    std::process::exit(0)
                }
                Ok(wcode_tui::Outcome::Abandoned) => {
                    // A force-quit abandoned the run: skip the relaunch
                    // banner, kill any background groups, exit 0.
                    Background::shutdown_all();
                    std::process::exit(0)
                }
                Ok(wcode_tui::Outcome::Reload { no_session }) => {
                    // Rebuild + re-exec into the same session (the REPL's
                    // `/reload`); the terminal is already restored.
                    repl::reload(
                        &llm,
                        session_path.as_deref(),
                        no_session,
                        args.agents,
                        args.config.as_deref(),
                        args.owner.as_deref(),
                        args.name.as_deref(),
                        None,
                    )
                    .await;
                    std::process::exit(1); // reached only if the build failed
                }
                Ok(wcode_tui::Outcome::Resume(path)) => {
                    // The TUI cannot rebuild an agent: hand off by re-exec'ing
                    // with `--resume <path>` (the terminal is already restored).
                    repl::exec_self(&repl::reload_args(
                        &llm,
                        Some(&path),
                        false,
                        args.agents,
                        args.config.as_deref(),
                        args.owner.as_deref(),
                        args.name.as_deref(),
                    ));
                    std::process::exit(1); // only reached if the exec failed
                }
                Ok(wcode_tui::Outcome::New) => {
                    // A fresh session: re-exec with no `--resume` (startup
                    // mints a new session file, or a fresh group for a team).
                    // The terminal is already restored.
                    println!("starting a new session ...");
                    repl::exec_self(&repl::new_session_args(
                        &llm,
                        args.agents,
                        args.config.as_deref(),
                        args.owner.as_deref(),
                        args.name.as_deref(),
                    ));
                    std::process::exit(1); // reached only if the exec failed
                }
                Err(e) => {
                    eprintln!("tui: {e}");
                    std::process::exit(1);
                }
            }
        }
        let Runtime {
            instructions,
            skills,
            hooks,
            orchestrator,
        } = rt;
        repl::run(
            repl::SessionSource::Local(Box::new(agent), bg),
            llm,
            hooks,
            cfg.tools,
            cfg.compaction,
            instructions,
            skills,
            root.team,
            root.guidelines,
            args.config.as_deref(),
            args.owner.as_deref(),
            args.name.as_deref(),
            orchestrator,
            cfg.workspace.digest_cas,
        )
        .await;
    }
}
#[tokio::main]
async fn main() {
    // Phase 1: parse argv, resolving the `--help`/`--version` early exits.
    let mut args = parse_cli();
    // A whitespace-only `WCODE_TASK` counts as unset; the `--task` flag wins
    // (a direct read, like `WCODE_CONFIG` — an invocation input, not provider
    // config, so it is not part of `EnvVars`).
    if args.task.is_none() {
        args.task = std::env::var("WCODE_TASK")
            .ok()
            .filter(|s| !s.trim().is_empty());
    }
    // Phase 2: load config raw (flags applied, `to_llm_opts` not yet run), then
    // optional endpoint detection, the provider diagnostic, and finally the
    // launch provider. Detection runs HERE so `to_llm_opts` captures the
    // detected base (a post-`settle_provider` set would be reverted).
    let mut cfg = load_config_raw(&args);
    if detect_opt_in(&args) && cfg.base_url.is_none() && !selected_model_pins_base(&cfg) {
        match detect_endpoint().await {
            Some(url) => {
                eprintln!("detected endpoint: {url}");
                cfg.base_url = Some(url);
                cfg.provenance.base_url = Source::Detect;
            }
            None => eprintln!("detect: no local endpoint answered; using {OPENAI_DEFAULT_BASE_URL}"),
        }
    }
    print_provider_diagnostic(&cfg);
    if args.dump_config {
        print_config(&cfg);
    }
    let mut llm = cfg.to_llm_opts();
    llm.settle_provider();
    // Phase 3a/3b: the request-reply one-shots that never touch a session.
    if args.dump_system_prompt {
        print_system_prompt(&args, &cfg);
    }
    if args.list_models {
        list_models_and_exit(&llm).await;
    }
    if args.list_themes {
        for name in wcode_tui::theme_names() {
            println!("{name}");
        }
        std::process::exit(0);
    }
    // Phase 3c: a `--socket` client (no local session). Falls through otherwise.
    if run_socket_client(&args, &cfg, &llm).await {
        unreachable!("run_socket_client only returns when it handled nothing");
    }
    // Phase 4: resolve/create the session (may restore model/effort into `llm`).
    let mut setup = load_session(&args, &mut llm);
    // Phase 5: instructions/skills/hooks + orchestrator wiring.
    let rt = build_runtime(&args, &cfg, &llm, &setup);
    // The root orchestrator sees the team; a served worker (`--owner`) does not.
    let root = RootCtx {
        team: if args.owner.is_some() { &[] } else { &cfg.team },
        guidelines: if args.owner.is_some() {
            None
        } else {
            cfg.orchestrator.guidelines.as_deref()
        },
    };
    // Phase 6: build the root agent (moving the session/context out of `setup`
    // without partially moving the struct, so `dispatch` can still borrow it).
    let (agent, bg) = build_agent_for(
        &args,
        &cfg,
        &llm,
        &rt,
        root,
        setup.session.take(),
        std::mem::take(&mut setup.context),
    );
    // Phase 7: `serve` owns the session; it diverges when taken.
    if args.serve {
        serve(&args, &llm, agent, bg.clone(), &rt.orchestrator).await;
    }
    // Phase 8: one-shot / TUI / line REPL. Diverges.
    dispatch(
        &args,
        &cfg,
        llm,
        rt,
        root,
        &setup,
        agent,
        bg,
    )
    .await;
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
    if args.no_tui || args.prompt.is_some() || args.serve || args.task.is_some() {
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

/// A live roster for `serve`: the registry's local sessions, minus `exclude`
/// (the root's own `agent:orchestrator` alias — the served root is labelled by
/// its session id instead). Kept fresh, so a worker spawned at runtime is
/// served without a restart.
#[cfg(unix)]
fn live_roster(
    registry: &wcode_protocol::Registry,
    exclude: SessionId,
) -> tokio::sync::watch::Receiver<Vec<(SessionId, wcode_harness::actor::SessionHandle)>> {
    fn filter(
        sessions: &[(SessionId, wcode_harness::actor::SessionHandle)],
        exclude: &SessionId,
    ) -> Vec<(SessionId, wcode_harness::actor::SessionHandle)> {
        sessions
            .iter()
            .filter(|(id, _)| id != exclude)
            .cloned()
            .collect()
    }
    let mut src = registry.subscribe();
    let (tx, rx) = tokio::sync::watch::channel(filter(&src.borrow_and_update(), &exclude));
    tokio::spawn(async move {
        loop {
            if src.changed().await.is_err() {
                break;
            }
            let _ = tx.send(filter(&src.borrow_and_update(), &exclude));
        }
    });
    rx
}

/// The `/resume` picker's list: one entry per session file, newest first, each
/// with a `id · age · first user line` label and the path to hand back for
/// `--resume`. Failures degrade to a bare file name rather than break the picker.
fn session_items() -> Vec<wcode_tui::SessionItem> {
    session_groups::list_groups(&session_dir())
        .unwrap_or_default()
        .into_iter()
        .map(|entry| {
            let path = entry.path().to_path_buf();
            let mut label = session_label(&path);
            // Mark a group that actually carries a team, so the picker
            // distinguishes it from a bare legacy file.
            if let session_groups::GroupEntry::Group(group) = &entry
                && !session_groups::scan_members(group)
                    .unwrap_or_default()
                    .is_empty()
            {
                label.push_str(" · team");
            }
            wcode_tui::SessionItem { label, path }
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
    // A group entry resolves to its `root.jsonl` (friction #4); a legacy file
    // is already the transcript.
    let root = match session_groups::group_dir_of(path) {
        Some(dir) => dir.join("root.jsonl"),
        None => path.to_path_buf(),
    };
    Session::open(&root)
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

/// The headless `--task` run: the plan is already seeded by boot instantiation;
/// this seeds the root and waits for the plan to reach a terminal state.
/// Exit: 0 once every node is `Done`, 1 for a `Failed` node (fail-fast) or a
/// timeout.
async fn run_workflow(
    backend: Backend,
    tasks: crate::tasks::TaskList,
    seed: String,
    timeout: Option<std::time::Duration>,
) -> i32 {
    // A whole-run drain (A3): unlike `one_shot`'s, it does NOT stop at the first
    // `AgentEnd`, so the subscription stays live across the run's many root
    // turns. Non-load-bearing otherwise — `ask` replies over a oneshot and a
    // broadcast with no reader never blocks the actor.
    let mut rx = backend.subscribe();
    tokio::spawn(async move { while rx.recv().await.is_ok() {} });

    // The root's first turn (A2): if it errors or exhausts turns, no node
    // transitions, and without a `--timeout` the wait below would never end.
    match backend.ask(Request::Submit { text: seed }).await {
        Ok(AgentEvent::Stopped {
            stop_reason: StopReason::Error | StopReason::MaxTurns,
        })
        | Err(_) => {
            print_workflow_summary(&tasks);
            return 1;
        }
        Ok(_) => {}
    }

    // ONE wall-clock deadline for the whole wait (F1): a per-`changed()` timer
    // would reset on every publish, so a continuous rework storm would never
    // trip it.
    let deadline = timeout.map(|d| tokio::time::Instant::now() + d);
    let mut updates = tasks.subscribe();
    let code = loop {
        if let Some(code) = tasks.terminal_code() {
            break code;
        }
        match deadline {
            Some(deadline) => {
                tokio::select! {
                    changed = updates.changed() => {
                        // A dropped sender means no further progress is possible (F3).
                        if changed.is_err() {
                            break tasks.terminal_code().unwrap_or(1);
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => break 1,
                }
            }
            None => {
                if updates.changed().await.is_err() {
                    break 1;
                }
            }
        }
    };

    print_workflow_summary(&tasks);
    code
}

/// The headless run's final per-node summary: `#id [state] title`, in id order.
fn print_workflow_summary(tasks: &crate::tasks::TaskList) {
    for task in tasks.snapshot() {
        println!("#{} [{}] {}", task.id, task.state.label(), task.title);
    }
}

/// The root's opening seed: the plan is ALREADY instantiated, so its one job is
/// `task complete` / `task reject` per report — NOT to author a duplicate plan
/// (obedience is a soft guarantee, §8).
fn workflow_seed(task: &str) -> String {
    format!(
        "The task is:\n{task}\n\nA `[workflow]` plan is ALREADY instantiated \
         (see `task list`). Do NOT create nodes — the scheduler dispatches each \
         ready node to its member. As each worker reports, call \
         `task complete <id>` with the report as `artifact`; for a gate, call \
         `task reject <id> <reason>` with the reason. Stop when no node is \
         ready and none is running."
    )
}

#[cfg(test)]
mod workflow_seed_tests {
    use super::*;

    #[test]
    fn seed_names_the_task_and_forbids_new_nodes() {
        let seed = workflow_seed("fix bug 123");
        assert!(seed.contains("fix bug 123"), "names the task: {seed}");
        assert!(
            seed.contains("ALREADY instantiated"),
            "says the plan exists: {seed}"
        );
        assert!(
            seed.contains("Do NOT create nodes"),
            "forbids a duplicate plan: {seed}"
        );
        assert!(seed.contains("task complete"), "says how to complete: {seed}");
        assert!(seed.contains("task reject"), "says how to reject: {seed}");
    }
}

/// Whether to instantiate a `[workflow]` template at boot: a fresh (non-resumed)
/// session with a workflow configured. A resume loads the persisted plan (P4) and
/// must NOT re-seed it (else the plan grows 2N after N resumes).
fn should_instantiate_workflow(resuming_group: bool, has_workflow: bool) -> bool {
    !resuming_group && has_workflow
}

/// Materialize a `[workflow]` template onto `tasks` in TOPOLOGICAL order (so a
/// `depends_on` may name a later-authored sibling — Blocker 1), mapping each
/// string id to its numeric task id before resolving deps. Returns the count.
fn instantiate_workflow(
    tasks: &crate::tasks::TaskList,
    workflow: &crate::config::Workflow,
    task: Option<&str>,
) -> usize {
    use std::collections::HashMap;
    let order = crate::config::topo_order(workflow)
        .unwrap_or_else(|e| workflow_fail(&e.to_string()));
    let n = order.len();
    let mut ids: HashMap<&str, u32> = HashMap::new();
    for node in order {
        let mut deps: Vec<u32> = Vec::with_capacity(node.depends_on.len());
        for d in &node.depends_on {
            match ids.get(d.as_str()) {
                Some(&id) => deps.push(id),
                None => workflow_fail(&format!(
                    "node `{}`: depends_on `{d}` was not created",
                    node.id
                )),
            }
        }
        let t = match tasks.create(crate::config::node_title(node, task), deps, node.gate) {
            Ok(t) => t,
            Err(e) => workflow_fail(&format!("node `{}`: {e}", node.id)),
        };
        ids.insert(node.id.as_str(), t.id);
        match (&node.member, &node.script) {
            (Some(m), None) => {
                if let Err(e) = tasks.assign(t.id, crate::tools::message::address(m)) {
                    workflow_fail(&format!("node `{}`: {e}", node.id));
                }
            }
            (None, Some(cmd)) => {
                if let Err(e) = tasks.configure(
                    t.id,
                    crate::tasks::RunSpec::Script {
                        command: cmd.clone(),
                    },
                    node.gate,
                ) {
                    workflow_fail(&format!("node `{}`: {e}", node.id));
                }
            }
            _ => {} // validated at load (exactly one of member|script)
        }
    }
    n
}

/// Print a `[workflow]` instantiation error and exit 2, on the same path as the
/// `[team]` loop. Unreachable after load validation — belt and suspenders.
fn workflow_fail(msg: &str) -> ! {
    eprintln!("error: workflow: {msg}");
    std::process::exit(2);
}

/// Map a task-list entry to the TUI's view type (owner shortened to `w1`).
fn task_item(task: &crate::tasks::Task) -> wcode_tui::TaskItem {
    wcode_tui::TaskItem {
        id: task.id,
        title: task.title.clone(),
        owner: task.owner.as_ref().map(crate::agents::short_name),
        state: task.state.label().to_string(),
        deps: task.deps.clone(),
        attempts: task.attempts,
    }
}

/// The initial plan for the TUI: empty when there is no orchestrator (a served
/// `--owner` worker holds no `TaskList`).
fn task_items(orchestrator: &Option<crate::agents::Orchestrator>) -> Vec<wcode_tui::TaskItem> {
    orchestrator
        .as_ref()
        .map(|o| o.tasks().snapshot().iter().map(task_item).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_detect_and_dump_config_flags() {
        let Parsed::Args(a) =
            parse_args(&args(&["--detect-endpoint", "--dump-config"])).unwrap()
        else {
            panic!("not args");
        };
        assert!(a.detect_endpoint);
        assert!(a.dump_config);
        assert!(!Args::default().detect_endpoint);
        assert!(!Args::default().dump_config);
    }

    #[test]
    fn env_detect_flag_matches_truthy_values() {
        for yes in ["1", "true", "yes"] {
            assert!(env_detect_flag(Some(yes)), "{yes} enables detection");
        }
        for no in ["0", "false", "no", "on", "", "TRUE"] {
            assert!(!env_detect_flag(Some(no)), "{no} does not enable detection");
        }
        assert!(!env_detect_flag(None));
    }

    #[test]
    fn detect_opt_in_reads_the_flag() {
        assert!(detect_opt_in(&Args {
            detect_endpoint: true,
            ..Args::default()
        }));
    }

    #[test]
    fn config_dump_never_prints_a_key_value() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                base_url: Some("https://api.example.com/v1".into()),
                api_key: Some("sk-secret-12345".into()),
                team: vec![TeamMember {
                    name: "w".into(),
                    api_key: Some("sk-team-999".into()),
                    ..TeamMember::default()
                }],
                models: std::collections::BTreeMap::from([(
                    "m2".to_string(),
                    crate::config::ModelProfile {
                        base_url: Some("http://m2/v1".into()),
                        api_key: Some("sk-model-777".into()),
                        ..Default::default()
                    },
                )]),
                ..FileConfig::default()
            },
        )
        .unwrap();
        let dump = config_dump(&cfg);
        // The visible fields are there...
        assert!(dump.contains("https://api.example.com/v1"), "{dump}");
        assert!(dump.contains("model m"), "{dump}");
        assert!(dump.contains("(set)"), "{dump}");
        // ...and no key VALUE ever reaches the output.
        for secret in ["sk-secret-12345", "sk-team-999", "sk-model-777"] {
            assert!(!dump.contains(secret), "secret leaked: {secret}\n{dump}");
        }
    }

    #[test]
    fn config_dump_names_a_pinned_model_base_not_openai() {
        // Global base unset + the SELECTED model pins a base: neither the summary
        // nor the dump may name api.openai.com as the destination.
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                models: std::collections::BTreeMap::from([(
                    "m".to_string(),
                    crate::config::ModelProfile {
                        base_url: Some("http://pinned/v1".into()),
                        ..Default::default()
                    },
                )]),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(cfg.base_url.is_none());
        let dump = config_dump(&cfg);
        assert!(dump.contains("base_url: http://pinned/v1"), "{dump}");
        assert!(dump.contains("(source: model profile)"), "{dump}");
        assert!(!dump.contains(OPENAI_DEFAULT_BASE_URL), "{dump}");
        // The startup diagnostic is likewise profile-aware: no implicit warning.
        assert!(!cfg.effective_base_url_is_implicit(&cfg.model));
    }

    #[tokio::test]
    async fn detection_lands_before_to_llm_opts_and_settle_keeps_it() {
        // POSITIVE: main's ordering is raw-config (detected base) THEN derive +
        // settle. `settle_provider` overlays only a `[models.<id>]` profile, and
        // a model with none keeps the detected launch base.
        let mut cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(cfg.base_url.is_none());
        cfg.base_url = Some("http://detected:11434/v1".into());
        let mut llm = cfg.to_llm_opts();
        llm.settle_provider();
        assert_eq!(llm.base_url.as_deref(), Some("http://detected:11434/v1"));

        // NEGATIVE CONTROL: set the base AFTER `to_llm_opts` (the wrong order) —
        // the captured launch provider is `None`, so `settle_provider` REVERTS
        // the late set. This is what a detection phase moved below the derive
        // would hit, so the pair fails if the ordering regresses.
        let late = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        let mut llm = late.to_llm_opts();
        llm.base_url = Some("http://too-late:11434/v1".into());
        llm.settle_provider();
        assert_eq!(
            llm.base_url, None,
            "a post-derive base set must be reverted by settle_provider"
        );
    }

    #[test]
    fn selected_model_pins_base_skips_probing() {
        let pinned = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                models: std::collections::BTreeMap::from([(
                    "m".to_string(),
                    crate::config::ModelProfile {
                        base_url: Some("http://pinned/v1".into()),
                        ..Default::default()
                    },
                )]),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(selected_model_pins_base(&pinned));
        let plain = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(!selected_model_pins_base(&plain));
    }

    #[test]
    fn workflow_is_not_instantiated_on_resume() {
        assert!(
            should_instantiate_workflow(false, true),
            "a fresh session with a [workflow] instantiates it"
        );
        assert!(
            !should_instantiate_workflow(true, true),
            "a resume must use the loaded plan, never re-seed it"
        );
        assert!(
            !should_instantiate_workflow(false, false),
            "no [workflow] → nothing to instantiate"
        );
    }

    #[test]
    fn instantiate_workflow_handles_a_later_authored_dependency() {
        use crate::config::{Workflow, WorkflowNode};
        // `b` (authored first) depends on `a` (authored second) — author order is
        // NOT topological, so a naive `ids[d]` would panic (Blocker 1).
        let w = Workflow {
            max_attempts: None,
            nodes: vec![
                WorkflowNode {
                    id: "b".into(),
                    member: Some("w1".into()),
                    script: None,
                    depends_on: vec!["a".into()],
                    gate: false,
                    title: None,
                },
                WorkflowNode {
                    id: "a".into(),
                    member: Some("w1".into()),
                    script: None,
                    depends_on: vec![],
                    gate: false,
                    title: None,
                },
            ],
        };
        let tasks = crate::tasks::TaskList::new();
        assert_eq!(instantiate_workflow(&tasks, &w, None), 2);
        let snap = tasks.snapshot();
        assert_eq!(snap.len(), 2);
        let b = snap.iter().find(|t| t.title == "b").unwrap();
        assert_eq!(b.deps, vec![1], "b depends on a's assigned (numeric) id");
        assert_eq!(b.owner.as_ref().map(|o| o.as_str()), Some("agent:w1"));
    }

    #[test]
    fn instantiate_workflow_resolves_node_titles() {
        use crate::config::{Workflow, WorkflowNode};
        let w = Workflow {
            max_attempts: None,
            nodes: vec![
                WorkflowNode {
                    id: "explore".into(),
                    member: Some("w1".into()),
                    title: Some("Explore: {{task}}".into()),
                    ..Default::default()
                },
                WorkflowNode {
                    id: "plain".into(),
                    member: Some("w1".into()),
                    title: None,
                    ..Default::default()
                },
            ],
        };
        let tasks = crate::tasks::TaskList::new();
        assert_eq!(instantiate_workflow(&tasks, &w, Some("fix 123")), 2);
        let snap = tasks.snapshot();
        assert_eq!(snap[0].title, "Explore: fix 123", "{{task}} substituted");
        assert_eq!(snap[1].title, "plain", "no title -> the node id");
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
        assert!(
            !label.contains("second line"),
            "only the first line: {label}"
        );
        assert!(
            label.contains(" · 0s · "),
            "a fresh session reads as 0s: {label}"
        );
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
    fn parse_theme_flag() {
        let Parsed::Args(a) = parse_args(&args(&["--theme", "nord"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.theme.as_deref(), Some("nord"));
        assert!(parse_args(&args(&["--theme"])).is_err());

        let Parsed::Args(a) = parse_args(&args(&["--list-themes"])).unwrap() else {
            panic!("not args");
        };
        assert!(a.list_themes);
        assert!(!Args::default().list_themes);
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse_args(&args(&["-h"])), Ok(Parsed::Help));
        assert_eq!(parse_args(&args(&["--help"])), Ok(Parsed::Help));
    }

    #[test]
    fn parse_version() {
        assert_eq!(parse_args(&args(&["-V"])), Ok(Parsed::Version));
        assert_eq!(parse_args(&args(&["--version"])), Ok(Parsed::Version));
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
        assert!(choose_tui(
            &Args {
                tui: true,
                ..Args::default()
            },
            false
        ));
        assert!(!choose_tui(
            &Args {
                no_tui: true,
                ..Args::default()
            },
            true
        ));
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
        // A `--task` run is headless: never the TUI.
        assert!(!choose_tui(
            &Args {
                task: Some("x".into()),
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
    fn parse_task_and_timeout() {
        let v = args(&["--task", "fix bug 123", "--timeout", "30"]);
        let Parsed::Args(a) = parse_args(&v).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.task.as_deref(), Some("fix bug 123"));
        assert_eq!(a.timeout, Some(30));
        // `0` (disabled) parses as-is; the driver filters it out.
        let Parsed::Args(a) = parse_args(&args(&["--timeout", "0"])).unwrap() else {
            panic!("not args");
        };
        assert_eq!(a.timeout, Some(0));
        assert!(parse_args(&args(&["--task"])).is_err(), "--task needs text");
        assert!(
            parse_args(&args(&["--timeout"])).is_err(),
            "--timeout needs seconds"
        );
    }

    #[test]
    fn timeout_rejects_junk() {
        let err = parse_args(&args(&["--timeout", "abc"])).unwrap_err();
        assert!(err.contains("abc"), "echoes the bad value: {err}");
    }

    /// Parse a `&[&str]` argv to `Args` (test helper).
    fn parsed(v: &[&str]) -> Args {
        let Parsed::Args(a) = parse_args(&args(v)).unwrap() else {
            panic!("not args");
        };
        a
    }

    #[test]
    fn task_guards_reject_misconfiguration() {
        use crate::config::{Workflow, WorkflowNode};
        let plain = Workflow::default();
        let templated = Workflow {
            max_attempts: None,
            nodes: vec![WorkflowNode {
                id: "a".into(),
                title: Some("Explore: {{task}}".into()),
                ..Default::default()
            }],
        };
        // --task without [workflow].
        assert_eq!(
            check_task_args(&parsed(&["--task", "x"]), None, true),
            Err("--task requires [workflow]".into())
        );
        // --task together with -p.
        assert_eq!(
            check_task_args(&parsed(&["--task", "x", "-p", "y"]), Some(&plain), true),
            Err("--task cannot be combined with -p".into())
        );
        // --task without --agents.
        assert_eq!(
            check_task_args(&parsed(&["--task", "x"]), Some(&plain), false),
            Err("--task requires --agents".into())
        );
        // --timeout without --task.
        assert_eq!(
            check_task_args(&parsed(&["--timeout", "5"]), Some(&plain), true),
            Err("--timeout requires --task".into())
        );
        // A {{task}} template without a task.
        assert_eq!(
            check_task_args(&parsed(&[]), Some(&templated), true),
            Err("[workflow] uses {{task}} but no --task/WCODE_TASK was given".into())
        );
    }

    #[test]
    fn task_guards_accept_a_well_formed_run() {
        use crate::config::Workflow;
        assert_eq!(
            check_task_args(&parsed(&[]), Some(&Workflow::default()), false),
            Ok(())
        );
        assert_eq!(
            check_task_args(
                &parsed(&["--timeout", "30", "--task", "x"]),
                Some(&Workflow::default()),
                true
            ),
            Ok(())
        );
        assert_eq!(
            check_task_args(&parsed(&["--task", "x"]), Some(&Workflow::default()), true),
            Ok(())
        );
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
        let argv: Vec<String> = ["--no-instructions"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let Parsed::Args(a) = parse_args(&argv).unwrap() else {
            panic!("not args");
        };
        assert!(a.no_instructions);
        assert!(!Args::default().no_instructions);
    }
}
