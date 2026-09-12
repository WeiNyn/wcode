use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::streamfn::{LlmEndpoint, LlmOpts, RetryPolicy};

use crate::instructions::Mode;
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
    /// Run concurrency-safe tool calls in one batch in parallel (default true;
    /// `--sequential` forces it off). `Option` so an absent table keeps the
    /// default ON rather than picking up `bool`'s `false`.
    #[serde(default)]
    pub parallel: Option<bool>,
}

/// Instruction ("reference") files, loaded from the `[instructions]` table.
///
/// Absent table = discover the default candidate names from the working dir up
/// to the repo root, plus a global file in the config directory. `file`
/// overrides discovery with a single name/path; `"off"` (or empty) disables.
/// Env: `WCODE_INSTRUCTIONS` (sets `file`).
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct InstructionsConfig {
    /// Explicit override: load exactly this name (walked up) or path, instead
    /// of discovering the candidate names. `"off"`/empty disables.
    pub file: Option<String>,
    /// Candidate names to discover per directory (default:
    /// [`crate::instructions::DEFAULT_INSTRUCTION_NAMES`]).
    pub names: Option<Vec<String>>,
    /// Also load a candidate file from the config dir (default true).
    pub global: Option<bool>,
}

impl InstructionsConfig {
    /// Resolve to a discovery mode. `"off"`/empty `file` disables everything.
    pub fn mode(&self) -> Mode {
        match self.file.as_deref().map(str::trim) {
            Some("") | Some("off") => Mode::Off,
            Some(name) => Mode::Explicit(name.to_string()),
            None => Mode::Discover {
                names: self.names.clone().unwrap_or_else(|| {
                    crate::instructions::DEFAULT_INSTRUCTION_NAMES
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect()
                }),
                global: self.global.unwrap_or(true),
            },
        }
    }
}

/// Skills, loaded from the `[skills]` table.
///
/// Absent table = discover `SKILL.md` packages from the standard roots (see
/// [`crate::skills::discover`]). Env: `WCODE_SKILLS` (`off` disables; otherwise
/// a path-list of extra roots).
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct SkillsConfig {
    /// Discover skills at all (default true).
    pub enabled: Option<bool>,
    /// Extra skill roots, scanned before the standard ones.
    pub dirs: Option<Vec<String>>,
    /// Skill names to skip.
    pub disabled: Option<Vec<String>>,
}

impl SkillsConfig {
    /// The discovery spec, or `None` when discovery is disabled.
    pub fn to_spec(&self) -> Option<crate::skills::Spec> {
        if self.enabled == Some(false) {
            return None;
        }
        Some(crate::skills::Spec {
            roots: self
                .dirs
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            global: true,
            disabled: self.disabled.clone().unwrap_or_default(),
        })
    }
}

/// Retry policy, loaded from the `[retry]` table. Absent = defaults (3 retries,
/// 500 ms base, 8 s cap); `max = 0` disables retrying.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct RetryConfig {
    /// Retries after the first attempt (`0` disables).
    pub max: Option<u32>,
    /// Base backoff, in milliseconds.
    pub base_ms: Option<u64>,
    /// Cap on a single backoff wait, in milliseconds.
    pub cap_ms: Option<u64>,
}

impl RetryConfig {
    /// Fold the configured overrides over the harness defaults.
    pub fn to_policy(self) -> RetryPolicy {
        let d = RetryPolicy::default();
        RetryPolicy {
            max: self.max.unwrap_or(d.max),
            base: self
                .base_ms
                .map(std::time::Duration::from_millis)
                .unwrap_or(d.base),
            cap: self
                .cap_ms
                .map(std::time::Duration::from_millis)
                .unwrap_or(d.cap),
        }
    }
}

/// Compaction policy, loaded from the `[compaction]` table. Every field is
/// optional so an absent table (or key) keeps the harness defaults; see
/// [`CompactionPolicy`].
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct CompactionConfig {
    /// Working ceiling in tokens; compaction triggers near it. `None` = the
    /// model window.
    pub budget: Option<u64>,
    /// Context-window override, used only when the model is unknown.
    pub window: Option<u64>,
    /// Tokens held free below the ceiling before compacting.
    pub min_remaining: Option<u64>,
    /// Recent verbatim context kept after compacting, in tokens.
    pub keep_recent_tokens: Option<u64>,
    /// Complete turns always kept, whatever their token size.
    pub keep_recent_turns: Option<usize>,
}

impl CompactionConfig {
    /// Fold the configured overrides over the harness defaults.
    pub fn to_policy(&self) -> CompactionPolicy {
        let d = CompactionPolicy::default();
        CompactionPolicy {
            window: self.window,
            budget: self.budget,
            min_remaining: self.min_remaining.unwrap_or(d.min_remaining),
            keep_recent_tokens: self.keep_recent_tokens.unwrap_or(d.keep_recent_tokens),
            keep_recent_turns: self.keep_recent_turns.unwrap_or(d.keep_recent_turns),
        }
    }
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
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default)]
    pub instructions: InstructionsConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub retry: RetryConfig,
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
    pub compaction: CompactionPolicy,
    pub instructions: InstructionsConfig,
    pub skills: SkillsConfig,
    pub retry: RetryPolicy,
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
    pub wcode_compact_budget: Option<String>,
    pub wcode_compact_window: Option<String>,
    pub wcode_compact_min_remaining: Option<String>,
    pub wcode_compact_keep_recent_tokens: Option<String>,
    pub wcode_compact_keep_recent_turns: Option<String>,
    pub wcode_instructions: Option<String>,
    pub wcode_skills: Option<String>,
    pub wcode_retry_max: Option<String>,
    pub wcode_retry_base_ms: Option<String>,
    pub wcode_retry_cap_ms: Option<String>,
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
            wcode_compact_budget: std::env::var("WCODE_COMPACT_BUDGET").ok(),
            wcode_compact_window: std::env::var("WCODE_COMPACT_WINDOW").ok(),
            wcode_compact_min_remaining: std::env::var("WCODE_COMPACT_MIN_REMAINING").ok(),
            wcode_compact_keep_recent_tokens: std::env::var("WCODE_COMPACT_KEEP_RECENT_TOKENS").ok(),
            wcode_compact_keep_recent_turns: std::env::var("WCODE_COMPACT_KEEP_RECENT_TURNS").ok(),
            wcode_instructions: std::env::var("WCODE_INSTRUCTIONS").ok(),
            wcode_skills: std::env::var("WCODE_SKILLS").ok(),
            wcode_retry_max: std::env::var("WCODE_RETRY_MAX").ok(),
            wcode_retry_base_ms: std::env::var("WCODE_RETRY_BASE_MS").ok(),
            wcode_retry_cap_ms: std::env::var("WCODE_RETRY_CAP_MS").ok(),
        }
    }
}

/// Typed config failure: `MissingModel` is recoverable via `--model` (it
/// carries the parsed file so the rescue keeps base_url/api_key), `Io`
/// (unreadable/corrupt file) is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    MissingModel(Box<FileConfig>),
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
/// Parse a non-negative integer env override (e.g. a token count).
fn parse_count(name: &str, value: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid {name} {value:?}: expected a non-negative integer"))
}

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

pub fn merge(env: EnvLike, file: FileConfig) -> Result<Config, ConfigError> {
    let model = match file.model.clone() {
        Some(m) => m,
        None => return Err(ConfigError::MissingModel(Box::new(file))),
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
    let mut compaction = file.compaction.to_policy();
    if let Some(v) = env.wcode_compact_budget.as_deref() {
        compaction.budget = Some(parse_count("compaction.budget", v).map_err(ConfigError::Io)?);
    }
    if let Some(v) = env.wcode_compact_window.as_deref() {
        compaction.window = Some(parse_count("compaction.window", v).map_err(ConfigError::Io)?);
    }
    if let Some(v) = env.wcode_compact_min_remaining.as_deref() {
        compaction.min_remaining =
            parse_count("compaction.min_remaining", v).map_err(ConfigError::Io)?;
    }
    if let Some(v) = env.wcode_compact_keep_recent_tokens.as_deref() {
        compaction.keep_recent_tokens =
            parse_count("compaction.keep_recent_tokens", v).map_err(ConfigError::Io)?;
    }
    if let Some(v) = env.wcode_compact_keep_recent_turns.as_deref() {
        compaction.keep_recent_turns = parse_count("compaction.keep_recent_turns", v)
            .map_err(ConfigError::Io)? as usize;
    }
    let mut retry = file.retry.to_policy();
    if let Some(v) = env.wcode_retry_max.as_deref() {
        retry.max = parse_count("retry.max", v).map_err(ConfigError::Io)? as u32;
    }
    if let Some(v) = env.wcode_retry_base_ms.as_deref() {
        retry.base = std::time::Duration::from_millis(
            parse_count("retry.base_ms", v).map_err(ConfigError::Io)?,
        );
    }
    if let Some(v) = env.wcode_retry_cap_ms.as_deref() {
        retry.cap = std::time::Duration::from_millis(
            parse_count("retry.cap_ms", v).map_err(ConfigError::Io)?,
        );
    }
    // Env beats toml: an explicit `WCODE_INSTRUCTIONS` (or `off`) wins for the
    // override; the discovery names/global come from the toml.
    let mut instructions = file.instructions;
    if let Some(v) = env.wcode_instructions {
        instructions.file = Some(v);
    }
    // Skill roots: `WCODE_SKILLS=off` disables; otherwise a path-list of extra
    // roots scanned before the standard ones.
    let mut skills = file.skills;
    if let Some(v) = env.wcode_skills.as_deref() {
        let v = v.trim();
        if v.is_empty() || v == "off" || v == "false" || v == "0" {
            skills.enabled = Some(false);
        } else {
            let mut dirs = skills.dirs.unwrap_or_default();
            dirs.extend(std::env::split_paths(v).map(|p| p.display().to_string()));
            skills.dirs = Some(dirs);
        }
    }
    Ok(Config {
        base_url: env.wcode_base_url.or(file.base_url),
        api_key: env.wcode_api_key.or(env.openai_api_key).or(file.api_key),
        model,
        endpoint,
        effort: env.wcode_effort.or(file.effort),
        hooks,
        tools,
        compaction,
        instructions,
        skills,
        retry,
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

/// wcode's config directory (`~/.config/wcode`): `config.toml` and the default
/// global instruction file. The spec mandates this path even on macOS (not
/// `dirs::config_dir()`).
pub fn config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|p| p.join(".config/wcode"))
}

impl Config {
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
        config_dir().map(|d| d.join("config.toml"))
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
            retry: self.retry,
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
                    ..ToolsConfig::default()
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

#[cfg(test)]
mod compaction_cfg_tests {
    use super::*;

    #[test]
    fn absent_table_keeps_harness_defaults() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.compaction, CompactionPolicy::default());
    }

    #[test]
    fn toml_sets_individual_fields() {
        let file: FileConfig =
            toml::from_str("model = \"m\"\n[compaction]\nbudget = 100000\nkeep_recent_turns = 4")
                .unwrap();
        let cfg = merge(EnvLike::default(), file).unwrap();
        assert_eq!(cfg.compaction.budget, Some(100_000));
        assert_eq!(cfg.compaction.keep_recent_turns, 4);
        assert_eq!(
            cfg.compaction.min_remaining,
            CompactionPolicy::default().min_remaining
        );
    }

    #[test]
    fn env_beats_toml_for_compaction() {
        let cfg = merge(
            EnvLike {
                wcode_compact_keep_recent_tokens: Some("1234".into()),
                ..EnvLike::default()
            },
            toml::from_str("model = \"m\"\n[compaction]\nkeep_recent_tokens = 999").unwrap(),
        )
        .unwrap();
        assert_eq!(cfg.compaction.keep_recent_tokens, 1234);
    }

    #[test]
    fn invalid_compaction_env_is_config_error_naming_the_field() {
        let err = merge(
            EnvLike {
                wcode_compact_budget: Some("lots".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        let ConfigError::Io(msg) = &err else {
            panic!("wrong error: {err:?}")
        };
        assert!(msg.contains("compaction.budget"), "got: {msg}");
    }
}

#[cfg(test)]
mod instructions_cfg_tests {
    use super::*;

    #[test]
    fn mode_defaults_to_discovery_and_honors_off_and_override() {
        assert_eq!(
            InstructionsConfig::default().mode(),
            Mode::Discover {
                names: crate::instructions::DEFAULT_INSTRUCTION_NAMES
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
                global: true,
            }
        );
        assert_eq!(
            InstructionsConfig { file: Some("off".into()), ..Default::default() }.mode(),
            Mode::Off
        );
        assert_eq!(
            InstructionsConfig { file: Some("   ".into()), ..Default::default() }.mode(),
            Mode::Off
        );
        assert_eq!(
            InstructionsConfig { file: Some("CLAUDE.md".into()), ..Default::default() }.mode(),
            Mode::Explicit("CLAUDE.md".into())
        );
    }

    #[test]
    fn env_instructions_overrides_file() {
        let cfg = merge(
            EnvLike {
                wcode_instructions: Some("off".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m".into()),
                instructions: InstructionsConfig {
                    file: Some("X.md".into()),
                    ..Default::default()
                },
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.instructions.file.as_deref(), Some("off"));
    }
}
