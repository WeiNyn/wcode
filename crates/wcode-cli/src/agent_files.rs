//! Parse markdown agent role definitions into [`config::TeamMember`].
//!
//! A role file becomes a team MEMBER only when a `[[team]]` entry points at it —
//! `[[team]] file = ".wcode/agents/<role>.md"`, resolved at boot by
//! `main.rs::resolve_team_files` and in the `team` tool by
//! [`resolve_member_files`]. `.wcode/agents/` is a CONVENTION, not a mechanism:
//! a `.md` nobody references contributes nothing (W033/D037 — membership is
//! declared, never discovered).
//!
//! The frontmatter supplies `name`, `model`, `tools`, `effort`, `read_only`,
//! `base_url` and `api_key`; the body becomes the member's `role` (D-C3/D-C4).

use std::path::Path;

use serde::Deserialize;

use crate::config::TeamMember;

/// Raw YAML frontmatter of an agent `.md`. Unknown keys (`license`, …) are
/// ignored — serde's default, matching `SKILL.md`.
#[derive(Debug, Default, Deserialize)]
struct AgentFrontmatter {
    /// Required phonebook key; absent → the file is skipped.
    name: Option<String>,
    /// Informational in v1 (D-C4); the `role` fallback when the body is empty.
    description: Option<String>,
    model: Option<String>,
    /// CSV string OR YAML list, normalized (D-C3). Absent = `None` (inherit all);
    /// present-but-empty = `Some(vec![])` (only `message`).
    #[serde(default, deserialize_with = "de_tools")]
    tools: Option<Vec<String>>,
    base_url: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    effort: Option<String>,
}

/// Deserialize `tools` from EITHER a comma-separated string (`"Read, Grep"`) OR a
/// YAML list (`[read, grep]`) — the custom-deserializer seam (D-C3). Normalized
/// to `Vec<String>` (each entry trimmed, empties dropped). An explicitly empty
/// string/list/null → `Some(vec![])`; a wrong type (e.g. a map) → a serde error
/// (skipped-with-warning upstream). The *absent* key is handled by the field's
/// `#[serde(default)]` and never reaches this function.
fn de_tools<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        List(Vec<String>),
        Str(String),
    }

    Ok(match Option::<Repr>::deserialize(deserializer)? {
        // A present-but-null `tools:` means "none": only `message`.
        None => Some(Vec::new()),
        Some(Repr::List(items)) => Some(normalize(items)),
        Some(Repr::Str(s)) => Some(normalize(s.split(',').map(str::to_string))),
    })
}

/// Trim each entry and drop the empties.
fn normalize(items: impl IntoIterator<Item = String>) -> Vec<String> {
    items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse one agent `.md` into a [`TeamMember`], or `None` to SKIP (never fatal)
/// when the frontmatter is missing/unterminated, the YAML is unparseable, or
/// `name` is absent/blank. `role` is the (trimmed) markdown body, or the
/// `description` when the body is empty (D-C4). `tools` are NOT validated here
/// (`validate_tools` runs at spawn — D-C3).
pub fn parse_agent_md(text: &str) -> Option<TeamMember> {
    let (yaml, body) = crate::frontmatter::split(text).ok()?;
    let fm: AgentFrontmatter = serde_yaml_ng::from_str(yaml).ok()?;

    let name = fm.name.map(|s| s.trim().to_string()).unwrap_or_default();
    if name.is_empty() {
        return None;
    }
    let body = body.trim();
    let role = if body.is_empty() {
        fm.description
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        Some(body.to_string())
    };

    Some(TeamMember {
        name,
        // D003: `file` is a load-time routing hint, resolved (and cleared) by
        // `main.rs::resolve_team_files`.
        file: None,
        model: fm.model,
        role,
        tools: fm.tools,
        base_url: fm.base_url,
        api_key: fm.api_key,
        read_only: fm.read_only,
        effort: fm.effort,
    })
}

/// Resolve an agent NAME against the declared pool. `Ok(&TeamMember)` when
/// found; `Err` naming the available names otherwise (`spawn { agent }`, op 5).
/// After W033/D037 the pool IS the `[[team]]` roster, so only a DECLARED role can
/// be named here; an unknown name is the intended error.
///
/// Pure: no filesystem access — the pool was built from the `[[team]]`
/// declarations by `main.rs::resolve_team_files` + [`parse_agent_md`] at startup.
/// Names are listed SORTED.
pub fn resolve_role<'a>(pool: &'a [TeamMember], name: &str) -> Result<&'a TeamMember, String> {
    pool.iter().find(|m| m.name == name).ok_or_else(|| {
        let mut names: Vec<&str> = pool.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        format!("no agent `{name}` (available: {})", names.join(", "))
    })
}

/// Fill each member's `file = "<md>"` in place from its agent `.md` (relative to
/// `cwd` unless absolute), the same override rules `main.rs::resolve_team_files`
/// applies at load. Returns `Err` on a missing/unparseable file, a differing
/// name, or a nameless entry.
///
/// FRICTION: `main.rs::resolve_team_files` returns `ConfigError` and also does
/// the final-uniqueness check; this returns `String` and does NOT (the `team`
/// tool's duplicate check is `spawn_worker`'s). The two share intent but not
/// code — a follow-up could hoist `main.rs`'s body onto this one (W020 A9).
pub fn resolve_member_files(team: &mut [TeamMember], cwd: &Path) -> Result<(), String> {
    for member in team.iter_mut() {
        let Some(file) = member.file.take() else {
            continue;
        };
        let path = if file.is_absolute() {
            file
        } else {
            cwd.join(file)
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("[[team]] file {}: {e}", path.display()))?;
        let resolved = parse_agent_md(&text).ok_or_else(|| {
            format!(
                "{} does not parse into a member (missing or invalid frontmatter/`name`)",
                path.display()
            )
        })?;

        // A name given both inline and in the file must agree.
        if !member.name.is_empty() && member.name != resolved.name {
            return Err(format!(
                "{} names `{}` but the entry names `{}`",
                path.display(),
                resolved.name,
                member.name
            ));
        }
        if member.name.is_empty() {
            member.name = resolved.name;
        }
        // A sibling key in the same `[[team]]` overrides the file's when set.
        if member.role.is_none() {
            member.role = resolved.role;
        }
        if member.model.is_none() {
            member.model = resolved.model;
        }
        if member.tools.is_none() {
            member.tools = resolved.tools;
        }
        if member.base_url.is_none() {
            member.base_url = resolved.base_url;
        }
        if member.api_key.is_none() {
            member.api_key = resolved.api_key;
        }
        if member.effort.is_none() {
            member.effort = resolved.effort;
        }
        // `read_only` is a `bool`, so an inline `false` cannot override a file's
        // `true`; `true` always wins.
        if resolved.read_only {
            member.read_only = true;
        }
    }

    // Neither a `name` nor a `file`: the entry addresses no worker.
    if team.iter().any(|m| m.name.trim().is_empty()) {
        return Err("a `[[team]]` entry needs a `name` or a `file`".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_md(dir: &Path, name: &str, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    // --- parse_agent_md -------------------------------------------------------

    #[test]
    fn parses_name_tools_string_and_list() {
        let csv = "---\nname: reviewer\ntools: read, grep\ndescription: d\n---\nbody\n";
        let member = parse_agent_md(csv).unwrap();
        assert_eq!(member.name, "reviewer");
        assert_eq!(member.tools, Some(vec!["read".into(), "grep".into()]));

        let list = "---\nname: reviewer\ntools: [read, grep]\n---\nbody\n";
        assert_eq!(
            parse_agent_md(list).unwrap().tools,
            Some(vec!["read".into(), "grep".into()])
        );
    }

    #[test]
    fn parses_model_effort_and_read_only() {
        let md = "---\nname: x\nmodel: gpt-5\neffort: high\nread_only: true\n---\nbody\n";
        let m = parse_agent_md(md).unwrap();
        assert_eq!(m.model.as_deref(), Some("gpt-5"));
        assert_eq!(m.effort.as_deref(), Some("high"));
        assert!(m.read_only);
    }

    #[test]
    fn body_becomes_role() {
        let md = "---\nname: x\ndescription: fallback\n---\n\nYou orchestrate.\n";
        assert_eq!(
            parse_agent_md(md).unwrap().role.as_deref(),
            Some("You orchestrate.")
        );
    }

    #[test]
    fn empty_body_falls_back_to_description() {
        let md = "---\nname: x\ndescription: Two gates.\n---\n\n\n";
        assert_eq!(
            parse_agent_md(md).unwrap().role.as_deref(),
            Some("Two gates.")
        );
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let md = "---\nname: x\nlicense: MIT\nallowed-tools: bash\n---\nbody\n";
        assert_eq!(parse_agent_md(md).unwrap().name, "x");
    }

    #[test]
    fn missing_name_is_skipped_with_none() {
        assert!(parse_agent_md("---\ndescription: d\n---\nbody\n").is_none());
        // A blank name is treated as missing.
        assert!(parse_agent_md("---\nname: \"  \"\n---\nbody\n").is_none());
    }

    #[test]
    fn unparseable_yaml_is_skipped_with_none() {
        assert!(parse_agent_md("---\nname: [unclosed\n---\nbody\n").is_none());
        // No frontmatter at all.
        assert!(parse_agent_md("# just a body\n").is_none());
    }

    #[test]
    fn absent_tools_is_none_and_empty_is_some_empty() {
        assert_eq!(parse_agent_md("---\nname: x\n---\nbody\n").unwrap().tools, None);
        for empty in ["tools: \"\"", "tools: []", "tools: null", "tools:"] {
            let md = format!("---\nname: x\n{empty}\n---\nbody\n");
            assert_eq!(
                parse_agent_md(&md).unwrap().tools,
                Some(Vec::new()),
                "{empty:?} must be an explicit empty allow-list"
            );
        }
    }

    // --- resolve_role (op 5) --------------------------------------------------

    fn member(name: &str) -> TeamMember {
        TeamMember {
            name: name.into(),
            file: None,
            model: None,
            role: None,
            tools: None,
            base_url: None,
            api_key: None,
            read_only: false,
            effort: None,
        }
    }

    #[test]
    fn resolve_role_finds_a_pool_member_by_name() {
        let pool = vec![member("judge"), member("scout")];
        let found = resolve_role(&pool, "scout").unwrap();
        assert_eq!(found.name, "scout");
    }

    #[test]
    fn an_unknown_agent_lists_the_pool_names_sorted() {
        let pool = vec![member("scout"), member("judge")];
        let err = resolve_role(&pool, "ghost").unwrap_err();
        assert!(err.contains("no agent `ghost`"), "{err}");
        assert!(err.contains("judge, scout"), "sorted names: {err}");
    }

    // --- resolve_member_files (op 6) ------------------------------------------

    fn member_file(name: &str) -> TeamMember {
        TeamMember {
            file: Some(PathBuf::from(name)),
            ..member("")
        }
    }

    #[test]
    fn resolve_member_files_fills_a_named_file_member() {
        let cwd = tempfile::tempdir().unwrap();
        write_md(
            cwd.path(),
            "judge.md",
            "---\nname: judge\nmodel: m1\ntools: [read]\nread_only: true\n---\nYou judge.\n",
        );
        let mut team = vec![member_file("judge.md")];
        resolve_member_files(&mut team, cwd.path()).unwrap();
        assert_eq!(team[0].name, "judge");
        assert_eq!(team[0].model.as_deref(), Some("m1"));
        assert_eq!(team[0].tools, Some(vec!["read".to_string()]));
        assert_eq!(team[0].role.as_deref(), Some("You judge."));
        assert!(team[0].read_only);
        assert!(team[0].file.is_none(), "the routing hint is consumed");
    }

    #[test]
    fn resolve_member_files_lets_a_sibling_key_win() {
        // The same override rules `main.rs::resolve_team_files` applies: an
        // inline key set in the `[[team]]` entry beats the file's (A9).
        let cwd = tempfile::tempdir().unwrap();
        write_md(
            cwd.path(),
            "judge.md",
            "---\nname: judge\nmodel: from-file\n---\nfile role\n",
        );
        let mut inline = member_file("judge.md");
        inline.model = Some("inline".into());
        inline.role = Some("inline role".into());
        let mut team = vec![inline];
        resolve_member_files(&mut team, cwd.path()).unwrap();
        assert_eq!(team[0].model.as_deref(), Some("inline"), "inline model wins");
        assert_eq!(team[0].role.as_deref(), Some("inline role"));
    }

    #[test]
    fn resolve_member_files_read_only_true_wins_over_an_inline_false() {
        let cwd = tempfile::tempdir().unwrap();
        write_md(
            cwd.path(),
            "judge.md",
            "---\nname: judge\nread_only: true\n---\nbody\n",
        );
        let mut team = vec![member_file("judge.md")];
        team[0].read_only = false; // absent and false are one value
        resolve_member_files(&mut team, cwd.path()).unwrap();
        assert!(team[0].read_only, "`true` in the file always wins");
    }

    #[test]
    fn resolve_member_files_rejects_a_differing_name() {
        let cwd = tempfile::tempdir().unwrap();
        write_md(cwd.path(), "judge.md", "---\nname: judge\n---\nbody\n");
        let mut team = vec![TeamMember {
            name: "other".into(),
            file: Some(PathBuf::from("judge.md")),
            ..member("other")
        }];
        let err = resolve_member_files(&mut team, cwd.path()).unwrap_err();
        assert!(err.contains("names `judge` but the entry names `other`"), "{err}");
    }

    #[test]
    fn resolve_member_files_rejects_a_missing_file() {
        let cwd = tempfile::tempdir().unwrap();
        let mut team = vec![member_file("ghost.md")];
        let err = resolve_member_files(&mut team, cwd.path()).unwrap_err();
        assert!(err.contains("ghost.md"), "{err}");
    }

    #[test]
    fn resolve_member_files_rejects_a_nameless_entry() {
        let cwd = tempfile::tempdir().unwrap();
        let mut team = vec![member("")];
        let err = resolve_member_files(&mut team, cwd.path()).unwrap_err();
        assert!(err.contains("needs a `name` or a `file`"), "{err}");
    }
}
