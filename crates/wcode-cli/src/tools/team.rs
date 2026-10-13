//! The `team` tool — W020 op 6: apply a named project team file
//! (`./.wcode/teams/<name>.toml`, D017) to the *live* session.
//!
//! **Members join only.** Each `[[team]]` entry is spawned through
//! [`Orchestrator::spawn_worker`](crate::agents::Orchestrator::spawn_worker)
//! (`crates/wcode-cli/src/agents.rs`), so the loud validations that method owns —
//! tool allow-list (D14) and duplicate-name rejection — apply, and the
//! roster/phonebook simply grow. The root's `# Your team` **system-prompt block is
//! NOT recomposed**; it stays as composed at startup
//! (`repl::system_prompt`, `crates/wcode-cli/src/repl.rs:71`). That boundary is
//! deliberate, not a bug (W020 §0 amendment 1, §2): `Agent` exposes no system-swap
//! API, only `set_plan_mode` recomposes `base_system`
//! (`crates/wcode-harness/src/agent.rs:132`).
//!
//! **Root-only**: registered in `Orchestrator::tools`
//! (`crates/wcode-cli/src/agents.rs`), never in `default_tools` — a worker
//! cannot apply a team (mirrors `spawn`/`member`).

use std::path::Path;

use serde::Deserialize;

use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::agent_files;
use crate::agents::{Orchestrator, WorkerSpec, short_name};
use crate::config::TeamMember;
use crate::teams;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TeamArgs {
    /// The team NAME — the file stem under `./.wcode/teams/`
    /// (`scout` → `./.wcode/teams/scout.toml`).
    name: String,
}

/// A team file, team-only: just the `[[team]]` array. Parsing the whole overlay
/// through `Config` would demand a `model` and the rest of a full config
/// (`config.rs`), which a team file does not carry — so we deserialize the one
/// field we need (W020 B4).
#[derive(Deserialize)]
struct TeamFile {
    #[serde(default)]
    team: Vec<TeamMember>,
}

/// Applies a project team file's members to the current session (W020 op 6).
///
/// Holds a clone of the [`Orchestrator`] so it reaches
/// [`Orchestrator::spawn_worker`](crate::agents::Orchestrator::spawn_worker) and
/// the applied-team guard. The clone is a value (Arc-shared internals); it never
/// calls back into this tool, so there is no cycle.
pub struct Team {
    orchestrator: Orchestrator,
}

impl Team {
    pub fn new(orchestrator: Orchestrator) -> Self {
        Self { orchestrator }
    }

    /// Resolve `./.wcode/teams/<name>.toml` (relative to `cwd`) into its member
    /// specs. Pure read — no session touched, no tool allow-list validated (that
    /// is `spawn_worker`'s job, D14) and no duplicate-name check (also
    /// `spawn_worker`'s).
    ///
    /// The file is an ordinary config overlay, so its `[[team]]` array **is** the
    /// team (an array is replaced wholesale at merge time; see `teams.rs`'s module
    /// doc). A `[[team]] file = "<md>"` entry is resolved against `cwd` by
    /// [`crate::agent_files::resolve_member_files`]. When the team is empty,
    /// [`WorkerSpec`]s for `[]` are returned (the caller marks the session a team).
    ///
    /// # Errors
    /// - the file is absent → `Err` naming the available teams
    ///   (`./.wcode/teams/*.toml` via [`crate::teams::discover`]);
    /// - the file does not parse, or a `file =` member is missing/invalid.
    fn load_team_file(cwd: &Path, name: &str) -> Result<Vec<TeamMember>, String> {
        let path = cwd.join(teams::overlay_path(name));
        if !path.is_file() {
            let available: Vec<String> = teams::discover(cwd).into_iter().map(|t| t.name).collect();
            let available = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            return Err(format!(
                "no team `{name}` at {} (available: {available})",
                path.display()
            ));
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("read team {}: {e}", path.display()))?;
        let mut team = toml::from_str::<TeamFile>(&text)
            .map_err(|e| format!("team `{name}` ({}): {e}", path.display()))?
            .team;
        agent_files::resolve_member_files(&mut team, cwd)?;
        Ok(team)
    }

    /// The live apply. Guard, spawn every member, mark the session.
    ///
    /// # Contract
    /// 1. Refuse when the session already has a team: `applied_team()` is `Some`
    ///    — a `[team]`-at-startup session **or** a prior apply (any name).
    /// 2. Load the team file ([`Self::load_team_file`]).
    /// 3. Spawn each member via `orchestrator.spawn_worker(spec)`.
    /// 4. When at least one member joined, mark the team applied (so a retry is
    ///    refused rather than colliding on a duplicate name).
    ///
    /// # Note (partial apply)
    /// Members spawned before a later member fails **stay** — a live apply is not
    /// transactional (there is no un-spawn). The output names how many joined.
    fn apply(&self, cwd: &Path, name: &str) -> ToolOutput {
        if let Some(applied) = self.orchestrator.applied_team() {
            let msg = if applied == "[config]" {
                "this session already started with a team; a second team cannot be applied".to_string()
            } else {
                format!("this session already has the team `{applied}`; a second team cannot be applied")
            };
            return error(msg);
        }
        let members = match Self::load_team_file(cwd, name) {
            Ok(members) => members,
            Err(e) => return error(e),
        };
        let total = members.len();
        let mut joined: Vec<String> = Vec::new();
        for member in &members {
            let spec = WorkerSpec {
                name: Some(member.name.clone()),
                model: member.model.clone(),
                system: member.role.clone(),
                tools: member.tools.clone(),
                base_url: member.base_url.clone(),
                api_key: member.api_key.clone(),
                read_only: member.read_only,
                effort: member.effort.clone(),
            };
            match self.orchestrator.spawn_worker(spec) {
                Ok(worker) => joined.push(short_name(&worker.id)),
                Err(e) => {
                    // A member joined before the failure → the session has a team
                    // now, so a retry must be refused (non-transactional apply).
                    if !joined.is_empty() {
                        self.orchestrator.mark_team_applied(name);
                    }
                    return error(format!(
                        "team `{name}`: {} of {total} members joined ({}) before the failure: {e}",
                        joined.len(),
                        joined.join(", ")
                    ));
                }
            }
        }
        self.orchestrator.mark_team_applied(name);
        ToolOutput {
            output: format!(
                "team `{name}` applied: {} member(s) joined ({})",
                joined.len(),
                joined.join(", ")
            ),
            ..ToolOutput::default()
        }
    }
}

/// A loud `is_error` output.
fn error(message: String) -> ToolOutput {
    ToolOutput {
        output: message,
        is_error: true,
        ..ToolOutput::default()
    }
}

#[async_trait::async_trait]
impl TypedTool for Team {
    type Args = TeamArgs;

    fn name(&self) -> &str {
        "team"
    }

    fn description(&self) -> &str {
        "Apply a project team file (`./.wcode/teams/<name>.toml`) to this \
         session: its `[[team]]` members join your roster and become addressable \
         by name with `message`. Refused when the session already has a team. \
         The `# Your team` prompt block is NOT changed (a documented v1 limit)."
    }

    async fn execute(&self, args: TeamArgs, ctx: &ToolContext) -> ToolOutput {
        self.apply(&ctx.working_dir, &args.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::protocol::SessionId;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
    use wcode_protocol::Registry;

    use crate::agents::WorkerTemplate;
    use crate::config::ToolsConfig;

    /// A never-streaming worker template — enough to prove spawn/registration.
    fn template() -> WorkerTemplate {
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        WorkerTemplate {
            system: "sys".into(),
            llm: LlmOpts::default(),
            stream_fn,
            hooks: HooksSet::default(),
            tools: ToolsConfig::default(),
            compaction: CompactionPolicy::default(),
            working_dir: std::env::temp_dir(),
            members_dir: None,
            sessions_dir: std::env::temp_dir(),
        }
    }

    fn orchestrator() -> Orchestrator {
        Orchestrator::new(Registry::new(), template())
    }

    fn ctx(dir: &Path) -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "t1".into(),
            name: "team".into(),
            working_dir: dir.to_path_buf(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
            session_path: None,
        }
    }

    /// Write a team file with two members under `<dir>/.wcode/teams/<name>.toml`.
    fn write_team(dir: &Path, name: &str, body: &str) {
        let teams = dir.join(".wcode/teams");
        std::fs::create_dir_all(&teams).unwrap();
        std::fs::write(teams.join(format!("{name}.toml")), body).unwrap();
    }

    const TWO_MEMBERS: &str = r#"
[[team]]
name = "w1"
model = "m1"

[[team]]
name = "reviewer"
read_only = true
"#;

    #[tokio::test]
    async fn applying_a_team_file_joins_its_members() {
        let dir = tempfile::tempdir().unwrap();
        write_team(dir.path(), "scout", TWO_MEMBERS);
        let o = orchestrator();
        let out = Team::new(o.clone())
            .execute(TeamArgs { name: "scout".into() }, &ctx(dir.path()))
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("2 member(s) joined"), "{}", out.output);
        // Both members are registered and addressable by name.
        let w1 = SessionId::agent("w1");
        let reviewer = SessionId::agent("reviewer");
        assert!(o.registry().contains(&w1), "w1 registered");
        assert!(o.registry().contains(&reviewer), "reviewer registered");
        assert!(o.worker_backend("w1").is_some(), "w1 in the phonebook");
        assert!(o.worker_backend("reviewer").is_some());
        assert_eq!(o.applied_team().as_deref(), Some("scout"));
    }

    #[tokio::test]
    async fn a_re_apply_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_team(dir.path(), "scout", TWO_MEMBERS);
        write_team(dir.path(), "beta", TWO_MEMBERS);
        let o = orchestrator();
        let tool = Team::new(o.clone());
        let first = tool
            .execute(TeamArgs { name: "scout".into() }, &ctx(dir.path()))
            .await;
        assert!(!first.is_error, "{}", first.output);
        // ANY second apply (a different name too) is refused.
        let second = tool
            .execute(TeamArgs { name: "beta".into() }, &ctx(dir.path()))
            .await;
        assert!(second.is_error, "{}", second.output);
        assert!(
            second.output.contains("already has the team `scout`"),
            "{}",
            second.output
        );
    }

    #[tokio::test]
    async fn an_unknown_team_lists_the_available_names() {
        let dir = tempfile::tempdir().unwrap();
        write_team(dir.path(), "scout", TWO_MEMBERS);
        let out = Team::new(orchestrator())
            .execute(TeamArgs { name: "ghost".into() }, &ctx(dir.path()))
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("no team `ghost`"), "{}", out.output);
        assert!(out.output.contains("scout"), "lists available: {}", out.output);
    }

    #[tokio::test]
    async fn a_team_session_started_from_config_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_team(dir.path(), "scout", TWO_MEMBERS);
        // A `[team]`-at-startup session: `with_team(.., true)` seeds the marker.
        let o = orchestrator().with_team(Arc::new(Vec::new()), true);
        let out = Team::new(o)
            .execute(TeamArgs { name: "scout".into() }, &ctx(dir.path()))
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("already started with a team"), "{}", out.output);
    }

    #[tokio::test]
    async fn a_duplicate_member_name_is_surfaced_loudly() {
        let dir = tempfile::tempdir().unwrap();
        write_team(dir.path(), "scout", TWO_MEMBERS);
        let o = orchestrator();
        // A live worker already named `w1` collides with the file's first member.
        o.spawn_worker(WorkerSpec {
            name: Some("w1".into()),
            ..Default::default()
        })
        .unwrap();
        let out = Team::new(o)
            .execute(TeamArgs { name: "scout".into() }, &ctx(dir.path()))
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(
            out.output.contains("a worker named `w1` already exists"),
            "{}",
            out.output
        );
    }

    #[tokio::test]
    async fn a_file_member_is_resolved_from_its_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".wcode/agents")).unwrap();
        std::fs::write(
            dir.path().join(".wcode/agents/judge.md"),
            "---\nname: judge\nmodel: from-md\n---\nYou judge.\n",
        )
        .unwrap();
        write_team(
            dir.path(),
            "scout",
            "[[team]]\nfile = \".wcode/agents/judge.md\"\n",
        );
        let o = orchestrator();
        let out = Team::new(o.clone())
            .execute(TeamArgs { name: "scout".into() }, &ctx(dir.path()))
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(o.registry().contains(&SessionId::agent("judge")), "{}", out.output);
        assert_eq!(
            o.registry().model_of(&SessionId::agent("judge")).as_deref(),
            Some("from-md")
        );
    }
}
