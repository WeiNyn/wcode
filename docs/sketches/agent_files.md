# Sketch — `crates/wcode-cli/src/agent_files.rs` (NEW module)

**Status:** interface sketch, review-only. No working logic. Fillable as-is.
**Design:** `docs/project-team-loading.md` Part C (D-C1…D-C4, syOfM, ANALK,
DVMan, bE6YW, `crates/wcode-cli/src/agent_files.rs`). Discover + parse markdown
agent definitions into the **existing** `TeamMember`.

**Grounding (current code, fresh anchors):**
- `TeamMember` (target type): `crates/wcode-cli/src/config.rs:DDmnB`
  - `name` `AOaxw`, `model` `ZHGWa`, `role` `tDVpK`, `tools` `6YmQx`,
    `base_url` `kOvsD`, `api_key` `SgjeN`, `read_only` `0iEhr`, `effort` `44hDX`.
- Tool-name validation is **deferred**: `agents.rs:JFI61` (`validate_tools`) runs
  at spawn, not here (D-C3 says names are wcode lowercase, validated by
  `validate_tools`).
- Sibling discovery pattern to copy: `skills.rs:IQxfj` (`discover`) /
  `skills.rs:0OL3K` (`project_dirs`) / `skills.rs` `collect_skill_files`.
- Uses `crate::frontmatter::split` (see `frontmatter.md`).

## Module contents

```rust
// crates/wcode-cli/src/agent_files.rs
//! Discover + parse markdown agent definitions into [`config::TeamMember`].
//!
//! Location (D-C1, working dir only — no repo-root walk-up):
//!   project: `./.wcode/agents/**/*.md`
//!   global:  `<home>/.config/wcode/agents/**/*.md` (recursive)
//!
//! Precedence (low → high, D-C2): global `.md` < project `.md` < folded TOML
//! `[team]` (the TOML fold happens in `main.rs::load_config_raw`, which folds the
//! `Vec<TeamMember>` this module returns UNDER the TOML team).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::TeamMember;

/// Raw YAML frontmatter of an agent `.md`. Unknown keys (`license`, …) are
/// ignored — serde's default, matching `SKILL.md` (bEkua).
#[derive(Debug, Default, Deserialize)]
struct AgentFrontmatter {
    /// Required phonebook key; absent → the file is skipped (S34LW).
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
/// YAML list (`[read, grep]`) — the untagged-enum / custom-deserializer seam
/// (D-C3). Normalizes to `Vec<String>` (each entry trimmed, empties dropped).
/// Absent key → `None`. An explicitly empty string/list → `Some(vec![])`.
/// A wrong type (e.g. a map) → a serde error (skipped-with-warning upstream).
fn de_tools<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>;

/// Parse one agent `.md` into a [`TeamMember`].
///
/// - Splits frontmatter via `crate::frontmatter::split`.
/// - `role` := the body, or the `description` when the body is empty (D-C4).
/// - Returns `Ok(None)` to SKIP-with-warning (never fatal, S34LW) when:
///   the frontmatter is missing/unterminated, the YAML is unparseable, or `name`
///   is absent/blank. Returns `Ok(Some)` for a usable member.
/// - `Err` is currently unused (skip style is total); kept so a future hard-fail
///   caller can distinguish. `tools` are NOT validated here (validate_tools, JFI61).
pub fn parse_agent_md(text: &str) -> Result<Option<TeamMember>, String>;

/// Discover agent members for `cwd`, LOWEST-precedence-first so the caller can
/// fold this UNDER the TOML `[team]` (D-C2). Order and dedupe:
///   - project `.md`s first (they WIN over global), then global `.md`s;
///   - dedupe by `name`: the first seen wins; a later same-name file in the SAME
///     dir is dropped with a `warning:` on stderr (skills' lenient-warn/skip
///     style, TSY7E — not `ConfigError::DuplicateTeamMember`, which is TOML-only).
/// `home` is the `.config/wcode` dir (`config_dir()`); `None` skips the global scan.
/// Uses `frontmatter.rs`; body → `role`, empty body → `description` (D-C4).
pub fn discover(cwd: &Path, home: Option<&Path>) -> Vec<TeamMember>;

/// Project scan only: `cwd/.wcode/agents/**/*.md`.
fn discover_project(cwd: &Path) -> Vec<TeamMember>;

/// Global scan only: `<home>/agents/**/*.md`.
fn discover_global(home: &Path) -> Vec<TeamMember>;

/// Recursively collect `*.md` under `root` (dot-dirs skipped, sorted for
/// determinism). Mirrors `skills.rs`'s `collect_skill_files`; no depth cap is
/// specified by the design (D-C1 leaves depth to a follow-up) — pick the same
/// `MAX_SCAN_DEPTH`-style bound or none, but document it.
fn collect_agent_files(root: &Path, out: &mut Vec<PathBuf>);

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_agent_md (design "Tests" section) -----------------------------

    #[test]
    fn parses_name_tools_string_and_list() {
        // `tools: read,grep` and `tools: [read, grep]` both -> Some(vec!["read","grep"]).
        todo!()
    }

    #[test]
    fn parses_model_effort_and_read_only() { todo!() }

    #[test]
    fn body_becomes_role() { todo!() }

    #[test]
    fn empty_body_falls_back_to_description() { todo!() }

    #[test]
    fn unknown_keys_are_ignored() { todo!() }

    #[test]
    fn missing_name_is_skipped_with_none() { todo!() }

    #[test]
    fn unparseable_yaml_is_skipped_with_none() { todo!() }

    #[test]
    fn absent_tools_is_none_and_empty_is_some_empty() { todo!() }

    // --- discover (collision rules, D-C2) ------------------------------------

    #[test]
    fn project_md_beats_global_md_with_the_same_name() { todo!() }

    #[test]
    fn same_name_within_one_dir_keeps_the_first() { todo!() }

    #[test]
    fn global_scan_is_skipped_when_home_is_none() { todo!() }
}
```

## Notes / friction
- **`discover` returns the LOW precedence set.** The TOML `[team]` has the highest
  precedence (D-C2), so `main.rs::load_config_raw` folds `discover(...)` *under*
  `cfg.team` — it must NOT be merged by `config::merge_values` (that is a
  `toml::Value` fold, and `cfg` is already a resolved `Config`). See
  `main.rs` sketch for the fold site.
- **`description` is dropped** in v1 (D-C4): the `TeamMember` has no
  `description` field (`config.rs:DDmnB`) and this module does not add one. The
  fallback (body-empty → `description`) is the only use. Open item D-C5.
- **Validation is deferred:** a bad `tools` name (e.g. uppercase `Read`) parses
  fine here and only fails later in `validate_tools` (agents.rs:JFI61) — D-C3
  explicitly declines Claude-style case remapping in v1.
