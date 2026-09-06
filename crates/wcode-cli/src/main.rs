//! CLI entry: arg parsing, config → LlmOpts → Agent, one-shot or REPL.

use std::io::Write as _;

use tokio::sync::mpsc;
use wcode_harness::agent::Agent;
use wcode_harness::event::AgentEvent;
use wcode_harness::message::{AgentMessage, StopReason};
use wcode_harness::session::Session;

mod config;
mod repl;
mod tools;

use crate::config::{Config, ConfigError, EnvLike, FileConfig, merge, parse_endpoint};
use crate::repl::{build_agent, list_sessions, resolve_session_path, session_dir};

const USAGE: &str = "\
wcode — minimal coding agent

usage: wcode [-p <prompt>] [--resume [path]] [--no-session] [--model <id>] [--base-url <url>] [--endpoint <chat|responses>]

  -p <prompt>        run once with <prompt>, print the reply, exit
  --resume [path]    resume a session (default: latest in the session dir)
  --no-session       don't record a session file
   --model <id>       override the configured model
   --base-url <url>   override the configured base URL
   --endpoint <e>     override the configured endpoint (chat|responses)
   -h, --help         show this help

config: ~/.config/wcode/config.toml
  model = \"...\"      (required)
  base_url = \"...\"   (optional, any OpenAI-compatible endpoint)
  api_key = \"...\"    (optional)
  endpoint = \"...\"   (optional, chat|responses, default chat)
env: WCODE_BASE_URL and WCODE_API_KEY override the toml; OPENAI_API_KEY is a key fallback
env: WCODE_ENDPOINT overrides the toml endpoint";

#[derive(Debug, Default, PartialEq)]
struct Args {
    prompt: Option<String>,
    /// None = flag absent; Some(None) = latest; Some(Some(path)) = that file.
    resume: Option<Option<String>>,
    no_session: bool,
    model: Option<String>,
    base_url: Option<String>,
    endpoint: Option<String>,
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
                a.endpoint = Some(args.get(i).ok_or("--endpoint requires chat|responses")?.clone());
                i += 1;
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(Parsed::Args(a))
}

/// MissingModel rescue: re-run merge() with the parsed file config so env
/// AND toml base_url/api_key survive a file that lacks `model`; the flag
/// model fills the gap. Flag overrides are applied by the caller below.
fn rescue(model: String, file: FileConfig, env: EnvLike) -> Config {
    merge(
        env,
        FileConfig {
            model: Some(model),
            ..file
        },
    )
    .expect("model set, merge cannot fail")
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
            rescue(
                args.model.clone().expect("--model"),
                file,
                EnvLike::from_env(),
            )
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
    let llm = cfg.to_llm_opts();

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

    let mut agent = build_agent(llm.clone(), session, context);

    match args.prompt {
        Some(prompt) => std::process::exit(one_shot(&mut agent, &prompt).await),
        None => repl::run(agent, llm).await,
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
        );
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
        );
        assert_eq!(cfg.model, "flag-model");
        assert_eq!(cfg.base_url.as_deref(), Some("http://toml"));
        assert_eq!(cfg.api_key.as_deref(), Some("k-toml"));
    }

    #[test]
    fn parse_errors() {
        assert!(parse_args(&args(&["-p"])).is_err());
        assert!(parse_args(&args(&["--model"])).is_err());
        assert!(parse_args(&args(&["--base-url"])).is_err());
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
