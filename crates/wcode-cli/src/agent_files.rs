//! Discover + parse markdown agent definitions into [`config::TeamMember`].
//!
//! Location (D-C1, working dir only — no repo-root walk-up):
//!   project: `./.wcode/agents/**/*.md`
//!   global:  `<home>/agents/**/*.md` (recursive)
//!
//! Precedence (high → low, D-C2/A2): folded TOML `[team]` > project `.md` >
//! global `.md`. `discover` returns the LOW-precedence set with PROJECT first
//! (they win), and `main.rs::load_config_raw` folds it UNDER the TOML `[team]`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::TeamMember;

/// Max directory depth scanned under an agents root (mirrors `skills.rs`).
const MAX_SCAN_DEPTH: usize = 4;

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
        // `main.rs::resolve_team_files`; a scanned `.md` member is already inline.
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

/// Discover agent members for `cwd`, HIGHEST-precedence-first: project `.md`s
/// first (they WIN over global), then global `.md`s; deduped by `name` with the
/// first seen winning (D-C2/A2). The caller folds this UNDER the TOML `[team]`.
/// A same-name file *within one directory* is dropped with a `warning:` on stderr
/// (A3 — never `ConfigError::DuplicateTeamMember`, which stays TOML-only).
/// `home` is the `.config/wcode` dir (`config_dir()`); `None` skips the global scan.
pub fn discover(cwd: &Path, home: Option<&Path>) -> Vec<TeamMember> {
    let mut out = discover_project(cwd);
    let mut seen: HashSet<String> = out.iter().map(|m| m.name.clone()).collect();
    if let Some(home) = home {
        for member in discover_global(home) {
            // Project already won a name clash; a global file loses silently.
            if seen.insert(member.name.clone()) {
                out.push(member);
            }
        }
    }
    out
}

/// Project scan only: `cwd/.wcode/agents/**/*.md`.
fn discover_project(cwd: &Path) -> Vec<TeamMember> {
    let mut files = Vec::new();
    collect_agent_files(&cwd.join(".wcode/agents"), &mut files);
    load_members(&files)
}

/// Global scan only: `<home>/agents/**/*.md`.
///
/// `pub(crate)` because `main.rs::fold_discovered_members` calls it directly
/// under `--no-project-config` (D005): the opt-out governs PROJECT discovery, so
/// the global agent dir survives it — as the global `config.toml` always does.
pub(crate) fn discover_global(home: &Path) -> Vec<TeamMember> {
    let mut files = Vec::new();
    collect_agent_files(&home.join("agents"), &mut files);
    load_members(&files)
}

/// Read + parse `files` in order, keeping the FIRST member seen per name and
/// warning on a same-scan collision (A3). Unreadable/unparseable files are
/// skipped with a warning.
fn load_members(files: &[PathBuf]) -> Vec<TeamMember> {
    let mut out: Vec<TeamMember> = Vec::new();
    let mut seen: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
    for file in files {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("warning: skipping agent file {}: {e}", file.display());
                continue;
            }
        };
        let Some(member) = parse_agent_md(&text) else {
            eprintln!(
                "warning: skipping agent file {}: missing or invalid frontmatter/`name`",
                file.display()
            );
            continue;
        };
        if let Some(first) = seen.get(&member.name) {
            eprintln!(
                "warning: skipping agent file {}: duplicate member `{}` (keeping {})",
                file.display(),
                member.name,
                first.display()
            );
            continue;
        }
        seen.insert(member.name.clone(), file.clone());
        out.push(member);
    }
    out
}

/// Recursively collect `*.md` under `depth`-bounded `root` (dot-dirs skipped,
/// sorted for determinism). Mirrors `skills.rs`'s `collect_skill_files`; the
/// design leaves depth open, so this reuses the same `MAX_SCAN_DEPTH` bound.
fn collect_agent_files(root: &Path, out: &mut Vec<PathBuf>) {
    collect_agent_files_at(root, 0, out);
}

fn collect_agent_files_at(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH || !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if !hidden {
                subdirs.push(path);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            files.push(path);
        }
    }
    files.sort();
    subdirs.sort();
    out.extend(files);
    for sub in subdirs {
        collect_agent_files_at(&sub, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // --- discover (collision rules, D-C2/A2/A3) -------------------------------

    #[test]
    fn project_md_beats_global_md_with_the_same_name() {
        let cwd = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        write_md(
            &cwd.path().join(".wcode/agents"),
            "dup.md",
            "---\nname: dup\n---\nproject\n",
        );
        write_md(
            &home.path().join("agents"),
            "dup.md",
            "---\nname: dup\n---\nglobal\n",
        );

        let members = discover(cwd.path(), Some(home.path()));
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].role.as_deref(), Some("project"));
    }

    #[test]
    fn same_name_within_one_dir_keeps_the_first() {
        let cwd = tempfile::tempdir().unwrap();
        let agents = cwd.path().join(".wcode/agents");
        // Sorted read order: `a.md` first.
        write_md(&agents, "a.md", "---\nname: dup\n---\nfirst\n");
        write_md(&agents, "b.md", "---\nname: dup\n---\nsecond\n");

        let members = discover(cwd.path(), None);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].role.as_deref(), Some("first"));
    }

    #[test]
    fn global_scan_is_skipped_when_home_is_none() {
        let cwd = tempfile::tempdir().unwrap();
        write_md(
            &cwd.path().join(".wcode/agents"),
            "glob.md",
            "---\nname: g\n---\nbody\n",
        );
        assert_eq!(discover(cwd.path(), None).len(), 1);
    }
}
