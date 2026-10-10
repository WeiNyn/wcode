//! Project **team files** — `./.wcode/teams/<name>.toml` (D017).
//!
//! A named alternative to the single auto-discovered `./.wcode/team.toml`: one
//! file per team, selected with `--team <name>` (or the TUI/VS Code `/team`
//! command). The file is an ordinary config overlay, so its `[[team]]` array
//! REPLACES the lower layer's (`merge_values`: an array is replaced wholesale) —
//! exactly "use this team instead".
//!
//! Discovery is PRESENTATION data (a name list for a picker); the overlay path is
//! what `--team` resolves to. No serde: this module only walks a directory.
use std::path::{Path, PathBuf};

/// The project teams directory, relative to the working dir (matching the
/// `./.wcode/` conventions of `config.toml` / `team.toml`).
pub const TEAMS_DIR: &str = ".wcode/teams";

/// One discoverable team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamFile {
    /// The team NAME — the file stem (`scout.toml` → `scout`).
    pub name: String,
    /// The overlay path (`./.wcode/teams/scout.toml`).
    pub path: PathBuf,
}

/// The team name a directory entry yields, or `None` when it is not a
/// `<name>.toml` — a dotfile, a non-`toml`, a bare `.toml`, a directory. Pure.
pub fn team_name(file_name: &str) -> Option<String> {
    if file_name.starts_with('.') {
        return None;
    }
    let stem = file_name.strip_suffix(".toml")?;
    if stem.is_empty() {
        return None;
    }
    Some(stem.to_string())
}

/// Every team under `<root>/.wcode/teams`, sorted by name. A missing (or
/// unreadable) directory is an EMPTY list, never an error — the caller decides
/// whether "no teams" matters.
pub fn discover(root: &Path) -> Vec<TeamFile> {
    let dir = root.join(TEAMS_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut teams: Vec<TeamFile> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            // A FILE only: a directory named `x.toml` is not a team (and neither
            // is a broken symlink). `fs::metadata` follows a symlink to a file.
            if !std::fs::metadata(&path).map(|meta| meta.is_file()).unwrap_or(false) {
                return None;
            }
            let name = team_name(&entry.file_name().to_string_lossy())?;
            Some(TeamFile { name, path })
        })
        .collect();
    teams.sort_by(|a, b| a.name.cmp(&b.name));
    teams
}

/// The overlay path `--team <name>` resolves to (`.wcode/teams/<name>.toml`,
/// relative — the same style the auto-discovered overlays use). Whether it
/// exists is the caller's check.
pub fn overlay_path(name: &str) -> PathBuf {
    Path::new(TEAMS_DIR).join(format!("{name}.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_name_accepts_a_toml_stem_and_rejects_noise() {
        assert_eq!(team_name("scout.toml").as_deref(), Some("scout"));
        assert_eq!(
            team_name("review-team.toml").as_deref(),
            Some("review-team"),
            "a dash is part of the name"
        );
        assert_eq!(team_name("scout.md"), None, "only .toml");
        assert_eq!(team_name(".hidden.toml"), None, "a dotfile is skipped");
        assert_eq!(team_name(".toml"), None, "an empty stem is skipped");
        assert_eq!(team_name("scout"), None, "no extension");
    }

    #[test]
    fn discover_lists_teams_sorted_and_skips_noise() {
        let dir = std::env::temp_dir().join(format!("wcode-teams-{}", std::process::id()));
        let teams = dir.join(TEAMS_DIR);
        std::fs::create_dir_all(&teams).unwrap();
        std::fs::write(teams.join("scout.toml"), "").unwrap();
        std::fs::write(teams.join("beta.toml"), "").unwrap();
        std::fs::write(teams.join("notes.md"), "").unwrap(); // not a team
        std::fs::write(teams.join(".hidden.toml"), "").unwrap(); // skipped
        std::fs::create_dir_all(teams.join("adir.toml")).unwrap(); // a dir named *.toml

        let found: Vec<String> = discover(&dir).into_iter().map(|t| t.name).collect();
        assert_eq!(found, vec!["beta", "scout"], "sorted, noise skipped");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_missing_dir_is_empty() {
        let root = std::env::temp_dir().join(format!("wcode-no-teams-{}", std::process::id()));
        assert!(discover(&root).is_empty());
    }

    #[test]
    fn overlay_path_is_the_teams_dir() {
        assert_eq!(
            overlay_path("scout"),
            Path::new(".wcode/teams/scout.toml"),
            "the relative overlay path --team resolves to"
        );
    }
}
