//! CLI entry: arg parsing, config → LlmOpts → Agent, one-shot or REPL.

use std::io::Write as _;

use tokio::sync::mpsc;
use wcode_harness::agent::Agent;
use wcode_harness::event::AgentEvent;
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::session::Session;

mod config;
mod repl;
mod rtk;
mod tools;

use crate::config::{Config, ConfigError, EnvLike, FileConfig, merge, parse_endpoint};
use crate::repl::{build_agent, default_hooks, list_sessions, resolve_session_path, session_dir};

const USAGE: &str = "\
wcode — minimal coding agent

usage: wcode [-p <prompt>] [--resume [path]] [--no-session] [--model <id>] [--base-url <url>] [--endpoint <chat|responses>] [--effort <level>] [--list-models]

  -p <prompt>        run once with <prompt>, print the reply, exit
  --resume [path]    resume a session (default: latest in the session dir)
  --no-session       don't record a session file
   --model <id>       override the configured model
   --base-url <url>   override the configured base URL
   --endpoint <e>     override the configured endpoint (chat|responses)
   --effort <level>   override the reasoning effort (free-style, e.g. high; '-'/'none'/'off' clears it)
   --list-models      list models from GET {base_url}/models and exit
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
env: WCODE_BASE_URL and WCODE_API_KEY override the toml; OPENAI_API_KEY is a key fallback
env: WCODE_ENDPOINT overrides the toml endpoint; WCODE_EFFORT overrides the toml effort
env: WCODE_RTK overrides the toml hooks.rtk (auto|true|false)
env: WCODE_GREP and WCODE_FIND override the toml tools.grep/find (true|false)";

#[derive(Debug, Default, PartialEq)]
struct Args {
    prompt: Option<String>,
    /// None = flag absent; Some(None) = latest; Some(Some(path)) = that file.
    resume: Option<Option<String>>,
    no_session: bool,
    model: Option<String>,
    base_url: Option<String>,
    endpoint: Option<String>,
    /// None = flag absent; Some(None) = clear; Some(Some(level)) = set.
    effort: Option<Option<String>>,
    list_models: bool,
}

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
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(Parsed::Args(a))
}

/// MissingModel rescue: re-run merge() with the parsed file config so env
/// AND toml base_url/api_key survive a file that lacks `model`; the flag
/// model fills the gap. Flag overrides are applied by the caller below.
#[allow(clippy::result_large_err)] // ConfigError carries FileConfig for the --model rescue
fn rescue(model: String, file: FileConfig, env: EnvLike) -> Result<Config, ConfigError> {
    merge(
        env,
        FileConfig {
            model: Some(model),
            ..file
        },
    )
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
    let mut cfg = match Config::load() {
        Ok(c) => c,
        Err(ConfigError::MissingModel(file)) if args.model.is_some() => {
            // merge() never ran (file lacked model): rescue re-runs it with
            // the parsed file so toml/env base_url+api_key survive;
            // `--model`/`--base-url` flags are applied below and still win.
            // An invalid env override (endpoint/rtk/grep/find) still surfaces
            // as a clean error here — not a panic.
            match rescue(
                args.model.clone().expect("--model"),
                file,
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
            eprintln!(
                "  set `model = \"...\"` in {} or pass --model",
                Config::default_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "~/.config/wcode/config.toml".into())
            );
            eprintln!("  env: WCODE_BASE_URL, WCODE_API_KEY (OPENAI_API_KEY fallback)");
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
    let mut llm = cfg.to_llm_opts();

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

    let hooks = default_hooks(&cfg.hooks);
    let mut agent = build_agent(llm.clone(), hooks.clone(), &cfg.tools, session, context);

    match args.prompt {
        Some(prompt) => std::process::exit(one_shot(&mut agent, &prompt).await),
        None => repl::run(agent, llm, hooks, cfg.tools).await,
    }
}

/// `-p` mode: no streaming output; print the final assistant text.
async fn one_shot(agent: &mut Agent, prompt: &str) -> i32 {
    let (tx, mut rx) = mpsc::unbounded_channel();
    // Draining keeps the channel from growing; the run dies without a reader.
    // The last stream error is kept so a failed run can show why.
    let drain = tokio::spawn(async move {
        let mut last_error: Option<String> = None;
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::Error { message } = ev {
                last_error = Some(message);
            }
        }
        last_error
    });
    let res = agent.run(prompt, tx).await;
    let last_error = drain.await.unwrap_or(None);
    match res {
        Ok(StopReason::Error) => {
            match last_error {
                Some(msg) => eprintln!("error: {msg}"),
                None => eprintln!("run failed"),
            }
            1
        }
        Ok(_) => {
            let text = agent
                .messages()
                .iter()
                .rev()
                .find_map(|m| match m {
                    AgentMessage::Assistant { .. } => Some(m.as_text()),
                    _ => None,
                })
                .unwrap_or_default();
            if !text.trim().is_empty() {
                println!("{text}");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
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
}
