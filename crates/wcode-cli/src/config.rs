use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wcode_harness::streamfn::{LlmEndpoint, LlmOpts};

use crate::rtk::RtkPreference;

/// Hook integrations, loaded from the `[hooks]` config table.
///
/// Built-in native hooks always have a default even when the table is absent:
/// `rtk = "auto"` — enabled only if the rtk binary is reachable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    #[serde(default)]
    pub rtk: RtkPreference,
}

/// Per-tool registration flags, loaded from the `[tools]` config table.
///
/// `grep` and `find` are redundant with `bash` (which can run `grep`/`find`
/// itself), so they register **off by default** to keep the model's tool
/// surface lean; setting a flag to `true` opts that native tool back in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ToolsConfig {
    /// Register the `grep` tool (anchor-carrying regex search).
    #[serde(default)]
    pub grep: bool,
    /// Register the `find` tool (glob file/dir listing).
    #[serde(default)]
    pub find: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct FileConfig {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub effort: Option<String>,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: String,
    pub endpoint: LlmEndpoint,
    pub effort: Option<String>,
    pub hooks: HooksConfig,
    pub tools: ToolsConfig,
}

/// Snapshot of the relevant environment variables, so merging is testable.
#[derive(Debug, Default)]
pub struct EnvLike {
    pub wcode_base_url: Option<String>,
    pub wcode_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub wcode_endpoint: Option<String>,
    pub wcode_effort: Option<String>,
    pub wcode_rtk: Option<String>,
    pub wcode_grep: Option<String>,
    pub wcode_find: Option<String>,
}

impl EnvLike {
    pub fn from_env() -> Self {
        Self {
            wcode_base_url: std::env::var("WCODE_BASE_URL").ok(),
            wcode_api_key: std::env::var("WCODE_API_KEY").ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            wcode_endpoint: std::env::var("WCODE_ENDPOINT").ok(),
            wcode_effort: std::env::var("WCODE_EFFORT").ok(),
            wcode_rtk: std::env::var("WCODE_RTK").ok(),
            wcode_grep: std::env::var("WCODE_GREP").ok(),
            wcode_find: std::env::var("WCODE_FIND").ok(),
        }
    }
}

/// Typed config failure: `MissingModel` is recoverable via `--model` (it
/// carries the parsed file so the rescue keeps base_url/api_key), `Io`
/// (unreadable/corrupt file) is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    MissingModel(FileConfig),
    Io(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingModel(_) => write!(
                f,
                "no model configured: set `model = \"...\"` in ~/.config/wcode/config.toml"
            ),
            ConfigError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

/// Precedence: env (WCODE_* with OPENAI_API_KEY fallback) > toml. Model is
/// required and comes from the toml only; error tells main what to prompt for.
/// Effort is optional, free-style, passed through verbatim.
/// Parse a `true`/`false` env override for an optional native tool (accepts
/// on/off/1/0 as synonyms). Unlike hooks.rtk these have no `auto` state —
/// off is the default and only an explicit enable registers the tool.
fn parse_tool_flag(tool: &str, value: &str) -> Result<bool, String> {
    match value.trim() {
        "true" | "on" | "1" => Ok(true),
        "false" | "off" | "0" => Ok(false),
        other => Err(format!(
            "unknown {tool} {other:?}: valid values are \"true\", \"false\""
        )),
    }
}

#[allow(clippy::result_large_err)] // ConfigError carries FileConfig for the --model rescue
pub fn merge(env: EnvLike, file: FileConfig) -> Result<Config, ConfigError> {
    let model = match file.model.clone() {
        Some(m) => m,
        None => return Err(ConfigError::MissingModel(file)),
    };
    let endpoint = parse_endpoint(env.wcode_endpoint.as_deref().or(file.endpoint.as_deref()))
        .map_err(ConfigError::Io)?;
    let mut hooks = file.hooks;
    if let Some(v) = env.wcode_rtk.as_deref() {
        hooks.rtk = RtkPreference::parse(Some(v)).map_err(ConfigError::Io)?;
    }
    let mut tools = file.tools;
    if let Some(v) = env.wcode_grep.as_deref() {
        tools.grep = parse_tool_flag("grep", v).map_err(ConfigError::Io)?;
    }
    if let Some(v) = env.wcode_find.as_deref() {
        tools.find = parse_tool_flag("find", v).map_err(ConfigError::Io)?;
    }
    Ok(Config {
        base_url: env.wcode_base_url.or(file.base_url),
        api_key: env.wcode_api_key.or(env.openai_api_key).or(file.api_key),
        model,
        endpoint,
        effort: env.wcode_effort.or(file.effort),
        hooks,
        tools,
    })
}

pub fn parse_endpoint(value: Option<&str>) -> Result<LlmEndpoint, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(LlmEndpoint::Chat),
        Some("chat") => Ok(LlmEndpoint::Chat),
        Some("responses") => Ok(LlmEndpoint::Responses),
        Some(other) => Err(format!(
            "unknown endpoint {other:?}: valid values are \"chat\", \"responses\""
        )),
    }
}

impl Config {
    #[allow(clippy::result_large_err)] // ConfigError carries FileConfig for the --model rescue
    pub fn load() -> Result<Config, ConfigError> {
        let file = match Self::default_path().map(std::fs::read_to_string) {
            Some(Ok(text)) => Some(
                toml::from_str::<FileConfig>(&text)
                    .map_err(|e| ConfigError::Io(format!("config parse error: {e}")))?,
            ),
            // Only a missing file counts as absent; unreadable/corrupt surfaces the real cause.
            Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
            Some(Err(e)) => return Err(ConfigError::Io(format!("config read error: {e}"))),
            None => None,
        };
        merge(EnvLike::from_env(), file.unwrap_or_default())
    }

    pub fn default_path() -> Option<PathBuf> {
        // Spec mandates ~/.config/wcode/config.toml even on macOS (not dirs::config_dir()).
        dirs::home_dir().map(|p| p.join(".config/wcode/config.toml"))
    }

    pub fn to_llm_opts(&self) -> LlmOpts {
        LlmOpts {
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            temperature: None,
            endpoint: self.endpoint,
            effort: self.effort.clone(),
            session_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_missing_model_is_typed_error() {
        let e = merge(EnvLike::default(), FileConfig::default()).unwrap_err();
        let ConfigError::MissingModel(file) = &e else {
            panic!("wrong error: {e:?}")
        };
        assert!(file.model.is_none());
        assert!(e.to_string().contains("no model configured"));
    }

    #[test]
    fn merge_env_overrides_file() {
        let cfg = merge(
            EnvLike {
                wcode_base_url: Some("http://env".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m1".into()),
                base_url: Some("http://file".into()),
                api_key: None,
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.base_url.as_deref(), Some("http://env"));
        assert_eq!(cfg.model, "m1");
    }

    #[test]
    fn endpoint_parses_chat_and_responses() {
        assert_eq!(parse_endpoint(None).unwrap(), LlmEndpoint::Chat);
        assert_eq!(parse_endpoint(Some("chat")).unwrap(), LlmEndpoint::Chat);
        assert_eq!(
            parse_endpoint(Some("responses")).unwrap(),
            LlmEndpoint::Responses
        );
    }

    #[test]
    fn endpoint_rejects_unknown_value() {
        let err = parse_endpoint(Some("grpc")).unwrap_err();
        assert!(err.contains("grpc"), "error names the value: {err}");
        assert!(err.contains("chat") && err.contains("responses"));
    }

    #[test]
    fn endpoint_env_beats_toml() {
        let cfg = merge(
            EnvLike {
                wcode_endpoint: Some("responses".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m1".into()),
                endpoint: Some("chat".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.endpoint, LlmEndpoint::Responses);
        assert_eq!(cfg.to_llm_opts().endpoint, LlmEndpoint::Responses);
    }

    #[test]
    fn effort_env_beats_toml_and_flows_to_llm_opts() {
        let cfg = merge(
            EnvLike {
                wcode_effort: Some("high".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m1".into()),
                effort: Some("low".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.effort.as_deref(), Some("high"));
        assert_eq!(cfg.to_llm_opts().effort.as_deref(), Some("high"));
    }

    #[test]
    fn effort_defaults_to_none() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m1".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.effort, None);
    }

    #[test]
    fn hooks_default_to_auto_rtk() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m1".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        // Built-in native hook: present with its default even with no `[hooks]`.
        assert_eq!(cfg.hooks, HooksConfig::default());
        assert_eq!(cfg.hooks.rtk, RtkPreference::Auto);
    }

    #[test]
    fn rtk_env_beats_toml() {
        let cfg = merge(
            EnvLike {
                wcode_rtk: Some("off".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m1".into()),
                hooks: HooksConfig {
                    rtk: RtkPreference::On,
                },
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.hooks.rtk, RtkPreference::Off);
    }

    #[test]
    fn invalid_rtk_env_is_config_error() {
        let err = merge(
            EnvLike {
                wcode_rtk: Some("sometimes".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m1".into()),
                ..FileConfig::default()
            },
        )
        .unwrap_err();
        let ConfigError::Io(msg) = &err else {
            panic!("wrong error: {err:?}")
        };
        assert!(msg.contains("sometimes"), "names the value: {msg}");
    }

    #[test]
    fn toml_hooks_accept_bool_and_string_rtk() {
        let file: FileConfig = toml::from_str("model = \"m1\"\n[hooks]\nrtk = true").unwrap();
        assert_eq!(file.hooks.rtk, RtkPreference::On);
        let file: FileConfig = toml::from_str("model = \"m1\"\n[hooks]\nrtk = \"auto\"").unwrap();
        assert_eq!(file.hooks.rtk, RtkPreference::Auto);
    }

    #[test]
    fn toml_invalid_rtk_fails_parse() {
        let err = toml::from_str::<FileConfig>("model = \"m1\"\n[hooks]\nrtk = \"sometimes\"")
            .unwrap_err();
        assert!(err.to_string().contains("sometimes"), "names it: {err}");
    }

    #[test]
    fn tools_default_to_off() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m1".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        // grep/find are shadowed by bash, so off unless explicitly enabled.
        assert_eq!(cfg.tools, ToolsConfig::default());
        assert!(!cfg.tools.grep);
        assert!(!cfg.tools.find);
    }

    #[test]
    fn toml_tools_enable_grep_and_find() {
        let file: FileConfig =
            toml::from_str("model = \"m1\"\n[tools]\ngrep = true\nfind = true").unwrap();
        assert!(file.tools.grep);
        assert!(file.tools.find);
        // Absent flags default to off.
        let file: FileConfig = toml::from_str("model = \"m1\"\n[tools]\nfind = true").unwrap();
        assert!(!file.tools.grep);
        assert!(file.tools.find);
    }

    #[test]
    fn tools_env_beats_toml() {
        let cfg = merge(
            EnvLike {
                wcode_grep: Some("false".into()),
                wcode_find: Some("on".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m1".into()),
                tools: ToolsConfig {
                    grep: true,
                    find: false,
                },
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(!cfg.tools.grep);
        assert!(cfg.tools.find);
    }

    #[test]
    fn invalid_tools_env_is_config_error() {
        let err = merge(
            EnvLike {
                wcode_grep: Some("sometimes".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m1".into()),
                ..FileConfig::default()
            },
        )
        .unwrap_err();
        let ConfigError::Io(msg) = &err else {
            panic!("wrong error: {err:?}")
        };
        assert!(msg.contains("grep"), "names the tool: {msg}");
    }
}
