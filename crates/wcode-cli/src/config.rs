use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wcode_harness::compaction::CompactionPolicy;
use wcode_harness::streamfn::{
    LlmEndpoint, LlmOpts, LlmProfile, LlmProvider, ModelProfiles, RetryPolicy,
};

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

/// Workspace concurrency, from the `[workspace]` config table. One honored
/// toggle: whole-file digest CAS (design §3).
///
/// Default ON: every agent gets a fresh `WorkspaceHooks` (built in
/// `build_agent`/`worker_config_with`) that auto-attaches the last-read digest
/// to a mutation and refuses a stale write. Set `digest_cas = false` to turn
/// the policy off (the hook short-circuits both of its seams).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Auto-attach last-read digests + refuse stale writes (design §3).
    #[serde(default = "default_true")]
    pub digest_cas: bool,
}

fn default_true() -> bool {
    true
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self { digest_cas: true }
    }
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
/// 500 ms base, 8 s cap, 60 s ttft, 120 s idle); `max = 0` disables retrying and
/// a `0` timeout disables that stall deadline.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct RetryConfig {
    /// Retries after the first attempt (`0` disables).
    pub max: Option<u32>,
    /// Base backoff, in milliseconds.
    pub base_ms: Option<u64>,
    /// Cap on a single backoff wait, in milliseconds.
    pub cap_ms: Option<u64>,
    /// Time-to-first-token timeout, in ms (`0` disables).
    pub ttft_ms: Option<u64>,
    /// Inter-item idle timeout, in ms (`0` disables).
    pub idle_ms: Option<u64>,
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
            ttft: self
                .ttft_ms
                .map(std::time::Duration::from_millis)
                .unwrap_or(d.ttft),
            idle: self
                .idle_ms
                .map(std::time::Duration::from_millis)
                .unwrap_or(d.idle),
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

/// A `[team]` member (F3): a worker the orchestrator starts with. Reuses the F2
/// `WorkerSpec` fields; `name` is required and must be unique.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct TeamMember {
    /// The worker's name — its phonebook key (`message`'s `to`). Must be unique.
    pub name: String,
    /// Model id override; `None` inherits the orchestrator's model.
    pub model: Option<String>,
    /// Role text appended to the worker's system prompt.
    pub role: Option<String>,
    /// Tool allow-list; `None` = all, `Some(vec![])` = only `message`.
    pub tools: Option<Vec<String>>,
    /// Provider base URL override; `None` inherits the orchestrator's.
    pub base_url: Option<String>,
    /// Provider API key override; `None` inherits the orchestrator's.
    pub api_key: Option<String>,
    /// Enforce read-only for this member (D1/D2); default false.
    #[serde(default)]
    pub read_only: bool,

    /// Reasoning-effort override (D3). `None` inherits; `"-"`/`"none"`/`"off"`
    /// clears; any other value sets it. `#[serde(default)]` = None.
    #[serde(default)]
    pub effort: Option<String>,
}

/// The `[orchestrator]` table (F3b): guidance for the root orchestrator. An
/// absent table adds nothing to the prompt.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct OrchestratorConfig {
    /// Free-text workflow appended to the root's prompt as `# Orchestrator
    /// workflow`; `None`/empty (or whitespace) adds nothing.
    pub guidelines: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct Workflow {
    /// Rework cap; `None` keeps the built-in 3 (`tasks.rs`, P2).
    pub max_attempts: Option<u32>,
    /// Nodes in author order; `depends_on` names refer to sibling `id`s. TOML
    /// spells this `[[workflow.node]]`.
    #[serde(rename = "node", default)]
    pub nodes: Vec<WorkflowNode>,
}

/// One node in a `[workflow]` template.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkflowNode {
    /// A string label, unique within the workflow (resolved to a numeric task id).
    pub id: String,
    /// Optional display title. `{{task}}` is substituted with the run's task;
    /// absent → the node's `id` is used (back-compat).
    #[serde(default)]
    pub title: Option<String>,
    /// Exactly one of `member` | `script` (validated at load).
    #[serde(default)]
    pub member: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    /// Sibling node ids this node runs after; resolved to task ids at boot.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// A gate (judgment or physical): its `reject` re-opens its deps (§5).
    #[serde(default)]
    pub gate: bool,
}

/// The one placeholder v1 recognizes in a node title.
pub const TASK_PLACEHOLDER: &str = "{{task}}";

/// Replace every `{{task}}` with `task`. Pure; no other brace form is special
/// (unknown braces pass through verbatim).
pub fn substitute_task(text: &str, task: &str) -> String {
    text.replace(TASK_PLACEHOLDER, task)
}

/// A node's resolved title: its `title`, with `{{task}}` substituted when a task
/// is present, else its `id`. Pure; `main.rs::instantiate_workflow` calls it.
// P1 config seam: `instantiate_workflow` wires this in P2; the tests here cover
// it until then.
#[allow(dead_code)]
pub fn node_title(node: &WorkflowNode, task: Option<&str>) -> String {
    match (node.title.as_deref(), task) {
        (Some(t), Some(task)) => substitute_task(t, task),
        (Some(t), None) => t.to_string(),
        (None, _) => node.id.clone(),
    }
}

impl Workflow {
    /// `true` iff any node title contains the `{{task}}` placeholder.
    // P1 config seam: the `--task` boot guard wires this once it lands; the
    // tests here cover it until then.
    #[allow(dead_code)]
    pub fn uses_task(&self) -> bool {
        self.nodes.iter().any(|n| {
            n.title
                .as_deref()
                .is_some_and(|t| t.contains(TASK_PLACEHOLDER))
        })
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
    /// `[models.<id>]`: a per-model provider profile (endpoint/base_url/api_key).
    /// A model without an entry inherits the global provider. No env var for
    /// this table — it is data, not a scalar knob.
    #[serde(default)]
    pub models: std::collections::BTreeMap<String, ModelProfile>,
    /// `[workspace]`: whole-file digest CAS (default ON).
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// `[peers]`: a name → peer map. A value is a socket path (a served peer,
    /// reachable over `--peer`/`register_remote`) or an address alias
    /// (`agent:<id>`/`user`). The phonebook (§13.15) resolves `message`'s `to`.
    #[serde(default)]
    pub peers: std::collections::HashMap<String, String>,
    /// `[team]`: workers the orchestrator starts with (F3). Passed through as-is;
    /// no env var (a team is data, not a scalar knob).
    #[serde(default)]
    pub team: Vec<TeamMember>,
    /// `[orchestrator]`: root-only workflow guidance (F3b), passed through.
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    /// `[theme]`: a role → color overlay, passed to the TUI and validated there
    /// (`wcode_tui::parse_theme`); absent roles keep the default palette.
    #[serde(default)]
    pub theme: std::collections::BTreeMap<String, String>,
    /// `[workflow]`: a plan template instantiated at boot (P5). `None` = no
    /// template (the model authors the DAG at runtime, as today).
    #[serde(default)]
    pub workflow: Option<Workflow>,
}

/// One `[models.<id>]` entry: the provider for a single model id. Every field
/// is optional — absent = inherit the process default — so an entry may
/// override only `base_url` without resetting the endpoint. Resolved into
/// [`wcode_harness::streamfn::ModelProfiles`] by [`merge`].
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct ModelProfile {
    pub endpoint: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

/// Where a resolved provider field came from (startup diagnostic and
/// `--dump-config`). Never affects behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// `--base-url` / `--endpoint` / `--model`.
    Flag,
    /// The named environment variable supplied it.
    Env(&'static str),
    /// `config.toml`.
    Toml,
    /// The built-in default (endpoint = chat; base_url = OpenAI).
    Default,
    /// Opt-in local endpoint detection supplied it.
    Detect,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Flag => write!(f, "flag"),
            Source::Env(name) => write!(f, "env {name}"),
            Source::Toml => write!(f, "config"),
            Source::Default => write!(f, "default"),
            Source::Detect => write!(f, "detected"),
        }
    }
}

/// Provenance for the four resolved provider fields, captured at merge time so
/// the startup diagnostic and `--dump-config` can name each field's SOURCE
/// (never the secret value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderProvenance {
    pub endpoint: Source,
    /// [`Source::Default`] here means the implicit `api.openai.com` fallback.
    pub base_url: Source,
    pub api_key: Source,
    pub model: Source,
}

/// The base URL rig falls back to when `base_url` is unset — named once so the
/// startup warning, the REPL banner and `--dump-config` cannot drift.
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Per-candidate probe budget for opt-in endpoint detection (loopback answers
/// instantly when up; a refused port fails immediately). Worst case = N × this.
pub const DETECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// The wire label for an endpoint (`chat` / `responses`).
pub fn endpoint_label(endpoint: LlmEndpoint) -> &'static str {
    match endpoint {
        LlmEndpoint::Chat => "chat",
        LlmEndpoint::Responses => "responses",
    }
}

/// Normalize an `$OLLAMA_HOST` value to a `…/v1` base URL. Rules: strip a
/// trailing `/`; keep an existing `/v1`; otherwise add a scheme if missing,
/// append the default `:11434` port when none is present, then append `/v1`.
fn normalize_ollama_host(raw: &str) -> Option<String> {
    let stripped = raw.trim().trim_end_matches('/');
    if stripped.is_empty() {
        return None;
    }
    if stripped.ends_with("/v1") {
        return Some(stripped.to_string());
    }
    let with_scheme = if stripped.contains("://") {
        stripped.to_string()
    } else {
        format!("http://{stripped}")
    };
    let authority = with_scheme
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("");
    // A bracketed IPv6 literal `[::1]` carries an optional `:port` after `]`.
    let has_port = match authority.strip_prefix('[') {
        Some(rest) => rest
            .split_once(']')
            .is_some_and(|(_, after)| after.starts_with(':')),
        None => authority.contains(':'),
    };
    let with_port = if has_port {
        with_scheme
    } else {
        format!("{with_scheme}:11434")
    };
    Some(format!("{with_port}/v1"))
}

/// The ordered local endpoints to probe for opt-in detection, deduped.
/// `$OLLAMA_HOST` first (normalized to a `/v1` base), then the well-known
/// default ports. Pure — no network, no I/O.
pub fn detect_candidates(env: &EnvLike) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(url) = env.ollama_host.as_deref().and_then(normalize_ollama_host) {
        out.push(url);
    }
    for url in ["http://localhost:11434/v1", "http://localhost:1234/v1"] {
        if !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
    }
    out
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
    /// Resolved `[models.<id>]`: id → provider profile (empty = inherit the
    /// launch provider). [`Config::to_llm_opts`] turns it into `ModelProfiles`.
    pub models: std::collections::BTreeMap<String, LlmProfile>,
    /// Resolved `[workspace]`: whole-file digest CAS (default ON).
    pub workspace: WorkspaceConfig,
    /// Resolved `[peers]` (name → socket path or address): the phonebook (§13.15).
    pub peers: std::collections::HashMap<String, String>,
    /// Resolved `[team]` (F3): the workers the orchestrator starts with.
    pub team: Vec<TeamMember>,
    /// Resolved `[orchestrator]` (F3b): the root's workflow guidance.
    pub orchestrator: OrchestratorConfig,
    /// Resolved `[theme]`: a validated overlay on the default TUI palette.
    pub theme: wcode_tui::ThemeSpec,
    /// Resolved `[workflow]`: the plan template, passed to `main.rs`.
    /// Resolved `[workflow]`: the plan template, passed to `main.rs`.
    pub workflow: Option<Workflow>,
    /// Provenance of the resolved provider fields (diagnostic / `--dump-config`).
    pub provenance: ProviderProvenance,
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
    pub wcode_retry_ttft_ms: Option<String>,
    pub wcode_retry_idle_ms: Option<String>,
    /// `OLLAMA_HOST`: fed to `detect_candidates` (opt-in detection only).
    pub ollama_host: Option<String>,
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
            wcode_retry_ttft_ms: std::env::var("WCODE_RETRY_TTFT_MS").ok(),
            wcode_retry_idle_ms: std::env::var("WCODE_RETRY_IDLE_MS").ok(),
            ollama_host: std::env::var("OLLAMA_HOST").ok(),
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
    /// Two `[team]` members share a `name` (names are unique phonebook keys).
    DuplicateTeamMember(String),
    /// An invalid `[theme]` overlay: an unknown role or an unparseable color.
    Theme(String),
    /// An invalid `[workflow]`: an unknown member, a cycle, missing-or-both of
    /// `member`|`script`, a duplicate/unknown id, or an empty script.
    Workflow(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingModel(_) => write!(
                f,
                "no model configured: set `model = \"...\"` in ~/.config/wcode/config.toml"
            ),
            ConfigError::Io(msg) => write!(f, "{msg}"),
            ConfigError::DuplicateTeamMember(name) => write!(
                f,
                "duplicate [team] member `{name}`: names must be unique"
            ),
            ConfigError::Theme(msg) => write!(f, "invalid [theme]: {msg}"),
            ConfigError::Workflow(msg) => write!(f, "invalid [workflow]: {msg}"),
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

/// Deep-merge `overlay` into `base` at the `toml::Value` level: table keys
/// recurse; anything else (a scalar or array) is replaced wholesale by the
/// overlay. Used to fold a `--config` overlay over the global config.
fn merge_values(base: toml::Value, overlay: toml::Value) -> toml::Value {
    match (base, overlay) {
        (toml::Value::Table(mut b), toml::Value::Table(o)) => {
            for (key, val) in o {
                let merged = match b.remove(&key) {
                    Some(existing) => merge_values(existing, val),
                    None => val,
                };
                b.insert(key, merged);
            }
            toml::Value::Table(b)
        }
        // A scalar/array (or a table/type change) is replaced wholesale.
        (_, over) => over,
    }
}

pub fn merge(env: EnvLike, file: FileConfig) -> Result<Config, ConfigError> {
    let model = match file.model.clone() {
        Some(m) => m,
        None => return Err(ConfigError::MissingModel(Box::new(file))),
    };
    let endpoint = parse_endpoint(env.wcode_endpoint.as_deref().or(file.endpoint.as_deref()))
        .map_err(ConfigError::Io)?;
    // Capture WHERE each provider field came from, before values are moved into
    // the `Config` below. Env beats toml; a missing source is the built-in default.
    let provenance = ProviderProvenance {
        endpoint: if env.wcode_endpoint.is_some() {
            Source::Env("WCODE_ENDPOINT")
        } else if file.endpoint.is_some() {
            Source::Toml
        } else {
            Source::Default
        },
        base_url: if env.wcode_base_url.is_some() {
            Source::Env("WCODE_BASE_URL")
        } else if file.base_url.is_some() {
            Source::Toml
        } else {
            Source::Default
        },
        api_key: if env.wcode_api_key.is_some() {
            Source::Env("WCODE_API_KEY")
        } else if env.openai_api_key.is_some() {
            Source::Env("OPENAI_API_KEY")
        } else if file.api_key.is_some() {
            Source::Toml
        } else {
            Source::Default
        },
        model: if file.model.is_some() {
            Source::Toml
        } else {
            Source::Default
        },
    };
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
    if let Some(v) = env.wcode_retry_ttft_ms.as_deref() {
        retry.ttft = std::time::Duration::from_millis(
            parse_count("retry.ttft_ms", v).map_err(ConfigError::Io)?,
        );
    }
    if let Some(v) = env.wcode_retry_idle_ms.as_deref() {
        retry.idle = std::time::Duration::from_millis(
            parse_count("retry.idle_ms", v).map_err(ConfigError::Io)?,
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
    // `[team]` names are phonebook keys: they must be unique (fail loudly).
    {
        let mut seen = std::collections::HashSet::new();
        for member in &file.team {
            if !seen.insert(member.name.as_str()) {
                return Err(ConfigError::DuplicateTeamMember(member.name.clone()));
            }
        }
    }
    // `[workflow]` validation — after the team block, so member names are known.
    if let Some(workflow) = &file.workflow {
        let team_names: Vec<String> = file.team.iter().map(|m| m.name.clone()).collect();
        validate_workflow(workflow, &team_names)?;
    }

    // `[theme]` is presentation-only, but a bad role or color is a hard error —
    // the "unparseable overlay fails loudly" style (§4 T3b).
    let theme = wcode_tui::parse_theme_table(&file.theme).map_err(ConfigError::Theme)?;
    // `[models.<id>]`: resolve each entry into an `LlmProfile`, validating the
    // endpoint value loudly (the same rule as the global `endpoint`). An absent
    // or empty endpoint means INHERIT (`None`), not `parse_endpoint(None)`'s
    // `Chat`, so a profile may override only `base_url`.
    let mut models: std::collections::BTreeMap<String, LlmProfile> =
        std::collections::BTreeMap::new();
    for (id, entry) in &file.models {
        let endpoint = match entry.endpoint.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(value) => Some(
                parse_endpoint(Some(value))
                    .map_err(|e| ConfigError::Io(format!("[models.{id:?}] {e}")))?,
            ),
        };
        models.insert(
            id.clone(),
            LlmProfile {
                endpoint,
                base_url: entry.base_url.clone(),
                api_key: entry.api_key.clone(),
            },
        );
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
        workspace: file.workspace,
        peers: file.peers,
        team: file.team,
        orchestrator: file.orchestrator,
        theme,
        workflow: file.workflow,
        models,
        provenance,
    })
}

/// Reject a malformed `[workflow]` at load time (D14 style): unique node ids;
/// every `depends_on` names a sibling id; exactly one of `member`|`script`; a
/// `member` names a `[team]` member; a `script` is non-empty; a `gate` has
/// deps; and the graph is ACYCLIC. `Err(ConfigError::Workflow(msg))`.
fn validate_workflow(w: &Workflow, team_names: &[String]) -> Result<(), ConfigError> {
    let bad = ConfigError::Workflow;
    if w.nodes.is_empty() {
        return Err(bad(
            "a [workflow] needs at least one [[workflow.node]]".into(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for node in &w.nodes {
        if !ids.insert(node.id.as_str()) {
            return Err(bad(format!("duplicate node id `{}`", node.id)));
        }
    }
    for node in &w.nodes {
        match (&node.member, &node.script) {
            (Some(_), Some(_)) => {
                return Err(bad(format!(
                    "node `{}`: exactly one of `member`|`script`",
                    node.id
                )));
            }
            (None, None) => {
                return Err(bad(format!(
                    "node `{}`: needs one of `member`|`script`",
                    node.id
                )));
            }
            (Some(m), None) => {
                if !team_names.iter().any(|n| n == m) {
                    return Err(bad(format!(
                        "node `{}`: member `{m}` is not a [team] member",
                        node.id
                    )));
                }
            }
            (None, Some(cmd)) => {
                if cmd.trim().is_empty() {
                    return Err(bad(format!("node `{}`: empty script", node.id)));
                }
            }
        }
        for dep in &node.depends_on {
            if !ids.contains(dep.as_str()) {
                return Err(bad(format!(
                    "node `{}`: depends_on `{dep}` is not a node id",
                    node.id
                )));
            }
        }
        if node.gate && node.depends_on.is_empty() {
            return Err(bad(format!(
                "node `{}`: a gate needs at least one depends_on",
                node.id
            )));
        }
    }
    topo_order(w)?;
    Ok(())
}

/// A topological order of `[workflow]` nodes (Kahn's algorithm), so boot
/// instantiation creates each dependency before its dependents regardless of
/// author order. `Err` on a cycle — including a self-edge. Shared by
/// [`validate_workflow`] and `main.rs::instantiate_workflow`.
pub(crate) fn topo_order(w: &Workflow) -> Result<Vec<&WorkflowNode>, ConfigError> {
    use std::collections::{HashMap, VecDeque};
    let pos: HashMap<&str, usize> = w
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut indeg = vec![0usize; w.nodes.len()];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); w.nodes.len()];
    for (i, node) in w.nodes.iter().enumerate() {
        for dep in &node.depends_on {
            let Some(&d) = pos.get(dep.as_str()) else {
                return Err(ConfigError::Workflow(format!(
                    "node `{}`: depends_on `{dep}` is not a node id",
                    node.id
                )));
            };
            adj[d].push(i);
            indeg[i] += 1;
        }
    }
    let mut queue: VecDeque<usize> = (0..w.nodes.len()).filter(|&i| indeg[i] == 0).collect();
    let mut out = Vec::with_capacity(w.nodes.len());
    while let Some(i) = queue.pop_front() {
        out.push(&w.nodes[i]);
        for &j in &adj[i] {
            indeg[j] -= 1;
            if indeg[j] == 0 {
                queue.push_back(j);
            }
        }
    }
    if out.len() != w.nodes.len() {
        return Err(ConfigError::Workflow("the graph is not acyclic".into()));
    }
    Ok(out)
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
        Self::load_with(None)
    }

    /// Load the global `config.toml`, then optionally deep-merge an overlay file
    /// on top of it (`--config`/`WCODE_CONFIG`). The overlay wins per key: two
    /// tables recurse, a scalar or array is replaced wholesale. Both files are
    /// parsed to `toml::Value` first, so a partial table (e.g. `[tools] grep`)
    /// overrides without clobbering its siblings. A missing or unparseable
    /// overlay is a hard error; only a missing *global* file counts as absent.
    pub fn load_with(overlay: Option<&Path>) -> Result<Config, ConfigError> {
        Self::load_paths(Self::default_path().as_deref(), overlay)
    }

    /// Load `global` (if present), then deep-merge `overlay` on top. Split out so
    /// tests can point both at temp files.
    fn load_paths(global: Option<&Path>, overlay: Option<&Path>) -> Result<Config, ConfigError> {
        let base = match global.map(std::fs::read_to_string) {
            Some(Ok(text)) => Some(
                toml::from_str::<toml::Value>(&text)
                    .map_err(|e| ConfigError::Io(format!("config parse error: {e}")))?,
            ),
            // Only a missing global file counts as absent; unreadable/corrupt surfaces the real cause.
            Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
            Some(Err(e)) => return Err(ConfigError::Io(format!("config read error: {e}"))),
            None => None,
        };
        let value = match overlay {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        ConfigError::Io(format!("config overlay not found: {}", path.display()))
                    } else {
                        ConfigError::Io(format!(
                            "config overlay read error ({}): {e}",
                            path.display()
                        ))
                    }
                })?;
                let over = toml::from_str::<toml::Value>(&text).map_err(|e| {
                    ConfigError::Io(format!(
                        "config overlay parse error ({}): {e}",
                        path.display()
                    ))
                })?;
                merge_values(base.unwrap_or_else(|| toml::Value::Table(toml::Table::new())), over)
            }
            None => base.unwrap_or_else(|| toml::Value::Table(toml::Table::new())),
        };
        // The post-overlay file is what a `MissingModel` rescue must carry.
        let file = FileConfig::deserialize(value)
            .map_err(|e| ConfigError::Io(format!("config parse error: {e}")))?;
        merge(EnvLike::from_env(), file)
    }

    pub fn default_path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("config.toml"))
    }

    /// `chat · base_url <url|https://api.openai.com/v1 (implicit)> · model <id>
    /// · key <source>` — one line, no secret. Used by the startup diagnostic,
    /// the REPL banner and `--dump-config`.
    pub fn provider_summary(&self) -> String {
        let base = match self.effective_base_url(&self.model) {
            Some(url) => url.to_string(),
            None => format!("{OPENAI_DEFAULT_BASE_URL} (implicit)"),
        };
        format!(
            "{} · base_url {base} · model {} · key {}",
            endpoint_label(self.endpoint),
            self.model,
            self.provenance.api_key
        )
    }

    /// The base URL the SELECTED `model` actually talks to: its
    /// `[models.<id>].base_url` when set (it wins in `settle_provider`), else the
    /// global base. `None` means the implicit `api.openai.com` fallback.
    pub fn effective_base_url(&self, model: &str) -> Option<&str> {
        self.models
            .get(model)
            .and_then(|profile| profile.base_url.as_deref())
            .or(self.base_url.as_deref())
    }

    /// True when the SELECTED `model`'s EFFECTIVE base is implicit (it would hit
    /// api.openai.com). A profile-pinned base is NOT implicit, so the warning
    /// never misfires when `[models.<id>]` routes the model elsewhere.
    pub fn effective_base_url_is_implicit(&self, model: &str) -> bool {
        self.effective_base_url(model).is_none()
    }
    pub fn to_llm_opts(&self) -> LlmOpts {
        let mut opts = LlmOpts {
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            temperature: None,
            endpoint: self.endpoint,
            effort: self.effort.clone(),
            session_id: None,
            retry: self.retry,
            model_profiles: ModelProfiles::default(),
        };
        // The launch provider (after the env/config globals) is what an unmapped
        // model falls back to; each `[models.<id>]` overlays the fields it sets.
        let mut profiles = ModelProfiles::new(LlmProvider::of(&opts));
        for (id, profile) in &self.models {
            profiles.insert(id.clone(), profile.clone());
        }
        opts.model_profiles = profiles;
        opts
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
        assert_eq!(cfg.endpoint, LlmEndpoint::Responses);
        assert_eq!(cfg.to_llm_opts().endpoint, LlmEndpoint::Responses);
    }

    #[test]
    fn a_models_profile_parses_and_resolves() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m1".into()),
                models: std::collections::BTreeMap::from([(
                    "m2".to_string(),
                    ModelProfile {
                        endpoint: Some("responses".into()),
                        base_url: Some("http://m2/v1".into()),
                        api_key: Some("k2".into()),
                    },
                )]),
                ..FileConfig::default()
            },
        )
        .unwrap();
        let profile = cfg.models.get("m2").expect("m2 is mapped");
        assert_eq!(profile.endpoint, Some(LlmEndpoint::Responses));
        assert_eq!(profile.base_url.as_deref(), Some("http://m2/v1"));
        assert_eq!(profile.api_key.as_deref(), Some("k2"));
        // A model with no entry is unmapped: it inherits the global provider.
        assert!(!cfg.models.contains_key("m1"));
    }

    #[test]
    fn a_models_profile_bad_endpoint_fails_at_load_naming_the_value() {
        let err = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m1".into()),
                models: std::collections::BTreeMap::from([(
                    "m2".to_string(),
                    ModelProfile {
                        endpoint: Some("grpc".into()),
                        ..ModelProfile::default()
                    },
                )]),
                ..FileConfig::default()
            },
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("grpc"), "names the bad value: {msg}");
        assert!(msg.contains("m2"), "names the model: {msg}");
    }

    #[test]
    fn a_models_profile_absent_or_empty_endpoint_inherits() {
        // AMENDMENT ruling #3: an absent/empty endpoint means INHERIT (None),
        // not `parse_endpoint(None)`'s `Chat`.
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m1".into()),
                models: std::collections::BTreeMap::from([
                    (
                        "empty".to_string(),
                        ModelProfile {
                            endpoint: Some("".into()),
                            base_url: Some("http://empty/v1".into()),
                            ..ModelProfile::default()
                        },
                    ),
                    (
                        "absent".to_string(),
                        ModelProfile {
                            base_url: Some("http://absent/v1".into()),
                            ..ModelProfile::default()
                        },
                    ),
                ]),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(cfg.models.get("empty").unwrap().endpoint.is_none());
        assert!(cfg.models.get("absent").unwrap().endpoint.is_none());
    }

    #[test]
    fn a_models_profile_beats_the_flag_folded_into_the_config() {
        // Precedence `profile > flag(=base) > globals`: a `--base-url`-style
        // flag folded into the config, a `[models.<id>]` entry for that model,
        // then `to_llm_opts()` → `settle_provider()` ⇒ the PROFILE wins.
        let mut cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                models: std::collections::BTreeMap::from([(
                    "m".to_string(),
                    ModelProfile {
                        endpoint: Some("responses".into()),
                        base_url: Some("http://profile".into()),
                        ..ModelProfile::default()
                    },
                )]),
                ..FileConfig::default()
            },
        )
        .unwrap();
        // The `--base-url` flag lands on the config first...
        cfg.base_url = Some("http://flag".into());
        let mut llm = cfg.to_llm_opts();
        // ...so the launch base (post-flag) is the flag-derived provider...
        assert_eq!(llm.base_url.as_deref(), Some("http://flag"));
        // ...and the `[models.<id>]` entry overlays it for that model.
        llm.settle_provider();
        assert_eq!(llm.base_url.as_deref(), Some("http://profile"));
        assert_eq!(llm.endpoint, LlmEndpoint::Responses);
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

    #[test]
    fn toml_team_parses_members() {
        let file: FileConfig = toml::from_str(
            r#"
model = "m"
[[team]]
name = "explorer"
model = "x"
role = "recon"
tools = ["read"]
base_url = "http://w/v1"
api_key = "wk"
read_only = true
effort = "high"
[[team]]
name = "reviewer"
"#,
        )
        .unwrap();
        assert_eq!(file.team.len(), 2);
        assert_eq!(file.team[0].name, "explorer");
        assert_eq!(file.team[0].model.as_deref(), Some("x"));
        assert_eq!(file.team[0].role.as_deref(), Some("recon"));
        assert_eq!(file.team[0].tools, Some(vec!["read".to_string()]));
        assert_eq!(file.team[0].base_url.as_deref(), Some("http://w/v1"));
        assert_eq!(file.team[0].api_key.as_deref(), Some("wk"));
        assert!(file.team[0].read_only);
        assert_eq!(file.team[0].effort.as_deref(), Some("high"));
        assert_eq!(file.team[1].name, "reviewer");
        assert_eq!(file.team[1].model, None);
        assert_eq!(file.team[1].base_url, None);
        assert!(!file.team[1].read_only, "absent read_only defaults false");
        assert_eq!(file.team[1].effort, None, "absent effort inherits");
    }

    #[test]
    fn merge_passes_team_through_and_absent_is_empty() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                team: vec![TeamMember {
                    name: "explorer".into(),
                    ..Default::default()
                }],
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.team.len(), 1);
        assert_eq!(cfg.team[0].name, "explorer");

        let none = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(none.team.is_empty());
    }

    #[test]
    fn duplicate_team_names_are_a_config_error() {
        let err = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                team: vec![
                    TeamMember {
                        name: "dup".into(),
                        ..Default::default()
                    },
                    TeamMember {
                        name: "dup".into(),
                        ..Default::default()
                    },
                ],
                ..FileConfig::default()
            },
        )
        .unwrap_err();
        let ConfigError::DuplicateTeamMember(name) = &err else {
            panic!("wrong error: {err:?}")
        };
        assert_eq!(name, "dup");
        assert!(err.to_string().contains("dup"));
    }

    #[test]
    fn nameless_team_member_fails_to_parse() {
        // `name` is mandatory — a `[[team]]` without it is a parse error.
        let err = toml::from_str::<FileConfig>("model = \"m\"\n[[team]]\nrole = \"no name\"\n")
            .unwrap_err();
        assert!(err.to_string().contains("name"), "names the field: {err}");
    }

    #[test]
    fn toml_workflow_parses_nodes() {
        let file: FileConfig = toml::from_str(
            "model = \"m\"\n[workflow]\nmax_attempts = 5\n\
             [[workflow.node]]\nid = \"a\"\nmember = \"w1\"\n\
             [[workflow.node]]\nid = \"b\"\ndepends_on = [\"a\"]\ngate = true\nscript = \"true\"\n",
        )
        .unwrap();
        let w = file.workflow.unwrap();
        assert_eq!(w.max_attempts, Some(5));
        assert_eq!(w.nodes.len(), 2);
        assert_eq!(w.nodes[1].depends_on, vec!["a"]);
        assert!(w.nodes[1].gate);
        assert_eq!(w.nodes[1].script.as_deref(), Some("true"));
    }

    /// Merge a `model` + one `w1` team + `toml`, expecting a config error.
    fn merge_workflow(toml: &str) -> ConfigError {
        let file: FileConfig =
            toml::from_str(&format!("model = \"m\"\n[[team]]\nname = \"w1\"\n{toml}")).unwrap();
        merge(EnvLike::default(), file).unwrap_err()
    }

    #[test]
    fn workflow_rejects_a_bad_graph() {
        let dup = merge_workflow(
            "[workflow]\n[[workflow.node]]\nid = \"a\"\nmember = \"w1\"\n\
             [[workflow.node]]\nid = \"a\"\nmember = \"w1\"\n",
        );
        assert!(matches!(dup, ConfigError::Workflow(m) if m.contains("duplicate")));
        let unknown =
            merge_workflow("[workflow]\n[[workflow.node]]\nid = \"a\"\nmember = \"w1\"\ndepends_on = [\"ghost\"]\n");
        assert!(matches!(unknown, ConfigError::Workflow(m) if m.contains("not a node id")));
        let cycle = merge_workflow(
            "[workflow]\n[[workflow.node]]\nid = \"a\"\nmember = \"w1\"\ndepends_on = [\"b\"]\n\
             [[workflow.node]]\nid = \"b\"\nmember = \"w1\"\ndepends_on = [\"a\"]\n",
        );
        assert!(matches!(cycle, ConfigError::Workflow(m) if m.contains("acyclic")));
        let self_edge =
            merge_workflow("[workflow]\n[[workflow.node]]\nid = \"a\"\nmember = \"w1\"\ndepends_on = [\"a\"]\n");
        assert!(matches!(self_edge, ConfigError::Workflow(m) if m.contains("acyclic")));
    }

    #[test]
    fn workflow_rejects_a_bad_node() {
        let both = merge_workflow(
            "[workflow]\n[[workflow.node]]\nid = \"a\"\nmember = \"w1\"\nscript = \"true\"\n",
        );
        assert!(matches!(both, ConfigError::Workflow(m) if m.contains("exactly one")));
        let neither = merge_workflow("[workflow]\n[[workflow.node]]\nid = \"a\"\n");
        assert!(matches!(neither, ConfigError::Workflow(m) if m.contains("needs one")));
        let bad_member =
            merge_workflow("[workflow]\n[[workflow.node]]\nid = \"a\"\nmember = \"ghost\"\n");
        assert!(matches!(bad_member, ConfigError::Workflow(m) if m.contains("not a [team] member")));
        let empty_script =
            merge_workflow("[workflow]\n[[workflow.node]]\nid = \"a\"\nscript = \"  \"\n");
        assert!(matches!(empty_script, ConfigError::Workflow(m) if m.contains("empty script")));
        let dep_less_gate =
            merge_workflow("[workflow]\n[[workflow.node]]\nid = \"a\"\nmember = \"w1\"\ngate = true\n");
        assert!(matches!(dep_less_gate, ConfigError::Workflow(m) if m.contains("gate needs")));
    }

    /// An empty (or `[[workflow.nodes]]`-typo'd) `[workflow]` is a loud error, not
    /// a silent no-op with an empty plan.
    #[test]
    fn workflow_rejects_an_empty_workflow() {
        let typo = merge_workflow("[workflow]\n[[workflow.nodes]]\nid = \"a\"\nmember = \"w1\"\n");
        assert!(matches!(typo, ConfigError::Workflow(m) if m.contains("at least one")));
        let bare = merge_workflow("[workflow]\n");
        assert!(matches!(bare, ConfigError::Workflow(m) if m.contains("at least one")));
    }

    /// A valid workflow passes merge and is carried into the resolved `Config`.
    #[test]
    fn a_valid_workflow_is_carried_through() {
        let file: FileConfig = toml::from_str(
            "model = \"m\"\n[[team]]\nname = \"w1\"\n\
             [workflow]\nmax_attempts = 5\n[[workflow.node]]\nid = \"a\"\nmember = \"w1\"\n",
        )
        .unwrap();
        let cfg = merge(EnvLike::default(), file).unwrap();
        assert_eq!(cfg.workflow.unwrap().max_attempts, Some(5));
    }

    // ---- workflow task injection: {{task}} substitution (P1) ----

    fn wf_node(id: &str, title: Option<&str>) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            title: title.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn substitute_task_replaces_every_placeholder() {
        assert_eq!(substitute_task("x {{task}} y", "T"), "x T y");
        assert_eq!(
            substitute_task("{{task}}/{{task}}", "fix 123"),
            "fix 123/fix 123"
        );
        // No placeholder → unchanged.
        assert_eq!(substitute_task("plain", "T"), "plain");
        // An empty task replaces the placeholder with nothing.
        assert_eq!(substitute_task("a{{task}}b", ""), "ab");
    }

    #[test]
    fn unknown_braces_are_left_literal() {
        // Only `{{task}}` is special; `{a}`, `{{b}}`, and a single-braced
        // `{task}` pass through verbatim (locked decision — not an error).
        assert_eq!(substitute_task("{a} {{b}} {task}", "T"), "{a} {{b}} {task}");
        // `{{foo}}`, a single-braced `{task}`, an unclosed `{{task}`, and a lone
        // `{` are all left verbatim.
        assert_eq!(
            substitute_task("{a} {{foo}} {task} {{task} { ", "T"),
            "{a} {{foo}} {task} {{task} { "
        );
    }

    #[test]
    fn uses_task_detects_a_title_only() {
        let w = Workflow {
            max_attempts: None,
            nodes: vec![wf_node("a", Some("Explore: {{task}}"))],
        };
        assert!(w.uses_task());
        // A placeholder in a `script` (not a title) does not count.
        let w = Workflow {
            max_attempts: None,
            nodes: vec![WorkflowNode {
                script: Some("echo {{task}}".into()),
                ..Default::default()
            }],
        };
        assert!(!w.uses_task());
        // A title carrying *other* brace syntax is not the placeholder.
        let w = Workflow {
            max_attempts: None,
            nodes: vec![wf_node("a", Some("Explore: {task} {{foo}}"))],
        };
        assert!(!w.uses_task());
        // A plain title with no placeholder is not a use of the task.
        let w = Workflow {
            max_attempts: None,
            nodes: vec![wf_node("a", Some("Explore the codebase"))],
        };
        assert!(!w.uses_task());
    }

    #[test]
    fn node_title_resolves_and_falls_back_to_id() {
        let titled = wf_node("explore", Some("Explore: {{task}}"));
        assert_eq!(node_title(&titled, Some("fix 123")), "Explore: fix 123");
        // No task present → the literal title, braces left literal.
        assert_eq!(node_title(&titled, None), "Explore: {{task}}");
        // No title → the node's id.
        assert_eq!(node_title(&wf_node("explore", None), Some("T")), "explore");
        // An empty task substitutes the placeholder down to nothing.
        assert_eq!(node_title(&titled, Some("")), "Explore: ");
    }

    #[test]
    fn toml_round_trips_a_title_with_the_placeholder() {
        let file: FileConfig = toml::from_str(
            "model = \"m\"\n[[team]]\nname = \"w1\"\n[workflow]\n\
             [[workflow.node]]\nid = \"a\"\nmember = \"w1\"\ntitle = \"Explore: {{task}}\"\n",
        )
        .unwrap();
        // A `{{task}}` title passes load validation (unchanged `validate_workflow`).
        let cfg = merge(EnvLike::default(), file).unwrap();
        assert_eq!(
            cfg.workflow.unwrap().nodes[0].title.as_deref(),
            Some("Explore: {{task}}")
        );
    }

    #[test]
    fn orchestrator_guidelines_parse_and_pass_through() {
        let file: FileConfig =
            toml::from_str("model = \"m\"\n[orchestrator]\nguidelines = \"do X\"\n").unwrap();
        assert_eq!(file.orchestrator.guidelines.as_deref(), Some("do X"));

        let cfg = merge(EnvLike::default(), file).unwrap();
        assert_eq!(cfg.orchestrator.guidelines.as_deref(), Some("do X"));

        // An absent `[orchestrator]` table → no guidelines.
        let none = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(none.orchestrator.guidelines, None);
    }

    #[test]
    fn overlay_deep_merge_scalars_tables_and_arrays() {
        let base: toml::Value =
            toml::from_str("model = \"global\"\n[tools]\ngrep = false\nfind = true\n").unwrap();
        let over: toml::Value =
            toml::from_str("model = \"overlay\"\n[team]\nx = 1\n[tools]\ngrep = true\n").unwrap();
        let merged = merge_values(base, over);
        let t = merged.as_table().unwrap();
        // A scalar is replaced by the overlay.
        assert_eq!(t["model"].as_str(), Some("overlay"));
        // An overlay-only table is added.
        assert!(t["team"].is_table());
        // A nested table recurses: overlay `grep` wins, global `find` survives.
        let tools = t["tools"].as_table().unwrap();
        assert_eq!(tools["grep"].as_bool(), Some(true));
        assert_eq!(tools["find"].as_bool(), Some(true));
    }

    #[test]
    fn overlay_replaces_arrays_wholesale() {
        let base: toml::Value = toml::from_str("xs = [1, 2, 3]\n").unwrap();
        let over: toml::Value = toml::from_str("xs = [9]\n").unwrap();
        let merged = merge_values(base, over);
        let xs = merged.get("xs").unwrap().as_array().unwrap();
        assert_eq!(xs.len(), 1);
        assert_eq!(xs[0].as_integer(), Some(9));
    }

    #[test]
    fn overlay_replaces_the_team_array_wholesale() {
        // The real shape: an overlay `[[team]]` replaces the global `[[team]]`
        // (an array-of-tables); it must not concatenate.
        let base: toml::Value =
            toml::from_str("[[team]]\nname = \"global-a\"\n[[team]]\nname = \"global-b\"\n")
                .unwrap();
        let over: toml::Value = toml::from_str("[[team]]\nname = \"overlay-a\"\n").unwrap();
        let merged = merge_values(base, over);
        let file: FileConfig = FileConfig::deserialize(merged).unwrap();
        assert_eq!(file.team.len(), 1);
        assert_eq!(file.team[0].name, "overlay-a");
    }

    #[test]
    fn load_merges_an_overlay_over_the_global_file() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("config.toml");
        let overlay = dir.path().join("team.toml");
        std::fs::write(
            &global,
            "model = \"g\"\nbase_url = \"http://g\"\n[tools]\nfind = true\n",
        )
        .unwrap();
        std::fs::write(
            &overlay,
            "[[team]]\nname = \"a\"\n[orchestrator]\nguidelines = \"do\"\n[tools]\ngrep = true\n",
        )
        .unwrap();

        let cfg = Config::load_paths(Some(&global), Some(&overlay)).unwrap();
        // Provider/model come from the global file, untouched by the overlay.
        assert_eq!(cfg.model, "g");
        assert_eq!(cfg.base_url.as_deref(), Some("http://g"));
        // The overlay added the team, guidelines, and one tool flag...
        assert_eq!(cfg.team.len(), 1);
        assert_eq!(cfg.team[0].name, "a");
        assert_eq!(cfg.orchestrator.guidelines.as_deref(), Some("do"));
        assert!(cfg.tools.grep);
        // ...without clobbering the global `[tools]` sibling.
        assert!(cfg.tools.find, "the global sibling key survives");
    }

    #[test]
    fn a_missing_overlay_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("config.toml");
        std::fs::write(&global, "model = \"g\"\n").unwrap();
        let missing = dir.path().join("nope.toml");
        let err = Config::load_paths(Some(&global), Some(&missing)).unwrap_err();
        let ConfigError::Io(msg) = &err else {
            panic!("wrong error: {err:?}")
        };
        assert!(msg.contains("overlay not found"), "{msg}");
        assert!(msg.contains("nope.toml"), "{msg}");
    }

    #[test]
    fn missing_model_rescue_carries_the_overlaid_file() {
        // The global file lacks `model`; the overlay adds a team. The MissingModel
        // error must carry the *overlaid* file so rescue() re-merges it.
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("config.toml");
        let overlay = dir.path().join("team.toml");
        std::fs::write(&global, "base_url = \"http://g\"\n").unwrap();
        std::fs::write(&overlay, "[[team]]\nname = \"a\"\n").unwrap();
        let err = Config::load_paths(Some(&global), Some(&overlay)).unwrap_err();
        let ConfigError::MissingModel(file) = &err else {
            panic!("wrong error: {err:?}")
        };
        assert_eq!(file.base_url.as_deref(), Some("http://g"));
        assert_eq!(
            file.team.len(),
            1,
            "the overlay team survives into the rescue file"
        );
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

    #[test]
    fn toml_retry_stall_timeouts_parse_into_the_policy() {
        let file: FileConfig =
            toml::from_str("model = \"m\"\n[retry]\nttft_ms = 5000\nidle_ms = 9000").unwrap();
        let cfg = merge(EnvLike::default(), file).unwrap();
        assert_eq!(cfg.retry.ttft, std::time::Duration::from_millis(5000));
        assert_eq!(cfg.retry.idle, std::time::Duration::from_millis(9000));
        // Unset fields keep the harness defaults.
        assert_eq!(cfg.retry.max, RetryPolicy::default().max);
        assert_eq!(
            cfg.to_llm_opts().retry.idle,
            std::time::Duration::from_millis(9000)
        );
    }

    #[test]
    fn retry_stall_env_beats_toml_and_zero_disables() {
        let cfg = merge(
            EnvLike {
                wcode_retry_ttft_ms: Some("0".into()),
                wcode_retry_idle_ms: Some("250".into()),
                ..EnvLike::default()
            },
            toml::from_str("model = \"m\"\n[retry]\nttft_ms = 5000\nidle_ms = 9000").unwrap(),
        )
        .unwrap();
        assert_eq!(cfg.retry.ttft, std::time::Duration::ZERO, "0 disables");
        assert_eq!(cfg.retry.idle, std::time::Duration::from_millis(250));
        assert_eq!(cfg.to_llm_opts().retry.ttft, std::time::Duration::ZERO);
    }

    #[test]
    fn invalid_retry_stall_env_is_config_error_naming_the_field() {
        let err = merge(
            EnvLike {
                wcode_retry_idle_ms: Some("soon".into()),
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
        assert!(msg.contains("retry.idle_ms"), "got: {msg}");
    }

    #[test]
    fn theme_table_with_named_and_hex_colors_parses() {
        let file: FileConfig =
            toml::from_str("model = \"m\"\n[theme]\nlink = \"light-cyan\"\nmuted = \"#123456\"\n")
                .unwrap();
        assert!(merge(EnvLike::default(), file).is_ok());
    }

    #[test]
    fn an_absent_theme_table_is_the_default_overlay() {
        let file: FileConfig = toml::from_str("model = \"m\"\n").unwrap();
        let cfg = merge(EnvLike::default(), file).unwrap();
        assert_eq!(cfg.theme, wcode_tui::ThemeSpec::default());
    }

    #[test]
    fn an_unknown_theme_role_is_a_config_error() {
        let file =
            toml::from_str::<FileConfig>("model = \"m\"\n[theme]\nnope = \"red\"\n").unwrap();
        let err = merge(EnvLike::default(), file).unwrap_err();
        assert!(matches!(err, ConfigError::Theme(_)), "got: {err:?}");
    }

    #[test]
    fn an_unknown_theme_color_is_a_config_error() {
        let file = toml::from_str::<FileConfig>("model = \"m\"\n[theme]\nlink = \"chartreuse\"\n")
            .unwrap();
        let err = merge(EnvLike::default(), file).unwrap_err();
        assert!(matches!(err, ConfigError::Theme(_)), "got: {err:?}");
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

#[cfg(test)]
mod workspace_cfg_tests {
    use super::*;

    #[test]
    fn digest_cas_defaults_on() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(cfg.workspace.digest_cas, "absent [workspace] means ON");
        assert_eq!(cfg.workspace, WorkspaceConfig::default());
    }

    #[test]
    fn toml_turns_digest_cas_off_and_on() {
        let off: FileConfig =
            toml::from_str("model = \"m\"\n[workspace]\ndigest_cas = false\n").unwrap();
        assert!(!off.workspace.digest_cas);
        assert!(!merge(EnvLike::default(), off).unwrap().workspace.digest_cas);

        let on: FileConfig =
            toml::from_str("model = \"m\"\n[workspace]\ndigest_cas = true\n").unwrap();
        assert!(merge(EnvLike::default(), on).unwrap().workspace.digest_cas);
    }
}

/// Endpoint visibility + opt-in local detection (next-steps item 49).
#[cfg(test)]
mod endpoint_detect_tests {
    use super::*;

    #[test]
    fn candidates_are_ordered_and_deduped() {
        // No OLLAMA_HOST: the two well-known default ports, in order.
        assert_eq!(
            detect_candidates(&EnvLike::default()),
            vec![
                "http://localhost:11434/v1".to_string(),
                "http://localhost:1234/v1".to_string(),
            ]
        );
        // An OLLAMA_HOST that normalizes to the well-known default is not repeated.
        let env = EnvLike {
            ollama_host: Some("http://localhost:11434".into()),
            ..EnvLike::default()
        };
        assert_eq!(
            detect_candidates(&env),
            vec![
                "http://localhost:11434/v1".to_string(),
                "http://localhost:1234/v1".to_string(),
            ]
        );
        // A distinct OLLAMA_HOST comes first.
        let env = EnvLike {
            ollama_host: Some("box:9999".into()),
            ..EnvLike::default()
        };
        assert_eq!(
            detect_candidates(&env),
            vec![
                "http://box:9999/v1".to_string(),
                "http://localhost:11434/v1".to_string(),
                "http://localhost:1234/v1".to_string(),
            ]
        );
    }

    #[test]
    fn ollama_host_is_normalized_to_v1() {
        // Port-less: a scheme is added, the default port appended, then `/v1`.
        let env = EnvLike {
            ollama_host: Some("myhost".into()),
            ..EnvLike::default()
        };
        assert_eq!(detect_candidates(&env)[0], "http://myhost:11434/v1");
        // A trailing slash is stripped; an existing `/v1` is kept.
        let env = EnvLike {
            ollama_host: Some("http://host:7777/v1/".into()),
            ..EnvLike::default()
        };
        assert_eq!(detect_candidates(&env)[0], "http://host:7777/v1");
        // A scheme + host:port with no path gets `/v1` only.
        let env = EnvLike {
            ollama_host: Some("http://host:8080".into()),
            ..EnvLike::default()
        };
        assert_eq!(detect_candidates(&env)[0], "http://host:8080/v1");
        // A bare host:port keeps its port.
        let env = EnvLike {
            ollama_host: Some("192.168.1.5:11434".into()),
            ..EnvLike::default()
        };
        assert_eq!(detect_candidates(&env)[0], "http://192.168.1.5:11434/v1");
        // An empty/whitespace value is ignored (just the defaults).
        let env = EnvLike {
            ollama_host: Some("   ".into()),
            ..EnvLike::default()
        };
        assert_eq!(
            detect_candidates(&env),
            vec![
                "http://localhost:11434/v1".to_string(),
                "http://localhost:1234/v1".to_string(),
            ]
        );
    }

    #[test]
    fn provenance_records_env_over_toml() {
        let cfg = merge(
            EnvLike {
                wcode_base_url: Some("http://env".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m".into()),
                base_url: Some("http://toml".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.provenance.base_url, Source::Env("WCODE_BASE_URL"));
    }

    #[test]
    fn provenance_records_toml_and_default() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                api_key: Some("k".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.provenance.model, Source::Toml);
        assert_eq!(cfg.provenance.api_key, Source::Toml);
        assert_eq!(cfg.provenance.endpoint, Source::Default);
        assert_eq!(cfg.provenance.base_url, Source::Default);
    }

    #[test]
    fn provenance_flags_win_over_env() {
        // The flag bump is applied by `main` after merge; here we assert the
        // summary renders the bump (env -> flag) and the value it carries.
        let mut cfg = merge(
            EnvLike {
                wcode_base_url: Some("http://env".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert_eq!(cfg.provenance.base_url, Source::Env("WCODE_BASE_URL"));
        cfg.base_url = Some("http://flag".into());
        cfg.provenance.base_url = Source::Flag;
        let summary = cfg.provider_summary();
        assert!(summary.contains("base_url http://flag"), "{summary}");
        assert_eq!(cfg.provenance.base_url, Source::Flag);
    }

    #[test]
    fn summary_names_the_implicit_openai_default() {
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(cfg.effective_base_url_is_implicit(&cfg.model));
        let summary = cfg.provider_summary();
        assert!(summary.contains(OPENAI_DEFAULT_BASE_URL), "{summary}");
        assert!(summary.contains("(implicit)"), "{summary}");
        assert!(summary.contains("model m"), "{summary}");
        assert!(summary.starts_with("chat · "), "{summary}");
        assert!(summary.ends_with("key default"), "{summary}");
        // A set base_url renders verbatim and is no longer implicit.
        let cfg = merge(
            EnvLike {
                wcode_base_url: Some("http://x/v1".into()),
                ..EnvLike::default()
            },
            FileConfig {
                model: Some("m".into()),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(!cfg.effective_base_url_is_implicit(&cfg.model));
        assert!(cfg.provider_summary().contains("base_url http://x/v1"));
    }

    #[test]
    fn detect_timeout_is_half_a_second() {
        assert_eq!(DETECT_TIMEOUT, std::time::Duration::from_millis(500));
    }

    #[test]
    fn a_pinned_model_base_is_never_reported_implicit() {
        // Global base_url unset, but the SELECTED model pins its own base: the
        // summary must name THAT base, never api.openai.com.
        let cfg = merge(
            EnvLike::default(),
            FileConfig {
                model: Some("m".into()),
                models: std::collections::BTreeMap::from([(
                    "m".to_string(),
                    ModelProfile {
                        base_url: Some("http://pinned/v1".into()),
                        ..Default::default()
                    },
                )]),
                ..FileConfig::default()
            },
        )
        .unwrap();
        assert!(cfg.base_url.is_none(), "the global base is unset");
        assert!(!cfg.effective_base_url_is_implicit(&cfg.model));
        assert_eq!(cfg.effective_base_url(&cfg.model), Some("http://pinned/v1"));
        let summary = cfg.provider_summary();
        assert!(summary.contains("base_url http://pinned/v1"), "{summary}");
        assert!(!summary.contains(OPENAI_DEFAULT_BASE_URL), "{summary}");
        assert!(!summary.contains("(implicit)"), "{summary}");
        // A DIFFERENT (unpinned) model still falls back to the implicit default.
        assert!(cfg.effective_base_url_is_implicit("other"));
    }
}
