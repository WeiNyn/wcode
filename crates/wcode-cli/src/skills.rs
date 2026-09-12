//! Skills: on-demand capability packages (`SKILL.md`) discovered from disk.
//!
//! Only each skill's `name` + `description` are folded into the system prompt;
//! the body is loaded on demand by the agent with the `read` tool. That is
//! progressive disclosure — N skills cost N one-line entries in context, not N
//! bodies.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Max characters of a skill description kept in the prompt section.
const MAX_PROMPT_DESCRIPTION_CHARS: usize = 200;
/// Max bytes of the whole "Available skills" prompt section.
const MAX_SECTION_BYTES: usize = 16_384;
/// Max directory depth scanned under a skills root.
const MAX_SCAN_DEPTH: usize = 4;
/// Max length of a skill name (Agent Skills standard).
const MAX_NAME_CHARS: usize = 64;
/// Max length of a description (Agent Skills standard).
const MAX_DESCRIPTION_CHARS: usize = 1024;

/// One discovered skill, from a `SKILL.md` frontmatter block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Path of the `SKILL.md` (the model loads this with `read`).
    pub path: PathBuf,
}

/// Discovered skills, in discovery order (first occurrence of a name wins).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillSet {
    pub skills: Vec<Skill>,
}

impl SkillSet {
    /// The skill with this name, if it was discovered.
    pub fn find(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// The `# Available skills` prompt section, or `None` when there are none.
    /// Paths are relative to `cwd` when under it, so the section is compact and
    /// stable; overflow past the section cap is noted, not silently dropped.
    pub fn render(&self, cwd: &Path) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut out = String::from(
            "# Available skills\n\n\
             Load one with `read` when a task matches; a skill may reference files\n\
             (scripts/, references/, assets/) relative to its own directory.\n",
        );
        let mut omitted = 0;
        for skill in &self.skills {
            let line = format!(
                "\n- {} — {} (read {})",
                skill.name,
                clip(&skill.description, MAX_PROMPT_DESCRIPTION_CHARS),
                display_path(&skill.path, cwd),
            );
            if out.len() + line.len() > MAX_SECTION_BYTES {
                omitted += 1;
                continue;
            }
            out.push_str(&line);
        }
        if omitted > 0 {
            out.push_str(&format!(
                "\n\n… {omitted} skill(s) omitted (prompt section over budget)"
            ));
        }
        Some(out)
    }
}

/// Where to look for skills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Explicit extra roots, scanned first (highest priority).
    pub roots: Vec<PathBuf>,
    /// Also scan the global roots under the home dir.
    pub global: bool,
    /// Skill names to skip entirely.
    pub disabled: Vec<String>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            global: true,
            disabled: Vec::new(),
        }
    }
}

/// Discover skills for `cwd`. Roots are scanned highest-priority first, and the
/// first skill with a given name wins:
///
/// 1. explicit `spec.roots`,
/// 2. the project chain — the working dir and its ancestors up to the repo
///    root, nearest first (`.wcode/skills`, then the shared pair),
/// 3. the global roots (`~/.local/share/wcode/skills`, then the shared pair).
///
/// The shared pair prefers `.agents/skills` and falls back to `.claude/skills`
/// where `.agents` is absent. Invalid or unreadable skills warn on stderr and
/// are skipped — discovery is best-effort.
pub fn discover(spec: &Spec, cwd: &Path, home: Option<&Path>) -> SkillSet {
    let mut roots: Vec<PathBuf> = spec.roots.clone();
    for dir in project_dirs(cwd) {
        roots.push(dir.join(".wcode/skills"));
        roots.extend(shared_pair(&dir));
    }
    if spec.global
        && let Some(home) = home
    {
        roots.push(home.join(".local/share/wcode/skills"));
        roots.extend(shared_pair(home));
    }

    let mut skills = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for root in roots {
        let mut files = Vec::new();
        collect_skill_files(&root, 0, &mut files);
        for file in files {
            match load_skill(&file) {
                Ok(skill) => {
                    if spec.disabled.iter().any(|d| d == &skill.name) {
                        continue;
                    }
                    if seen.insert(skill.name.clone()) {
                        skills.push(skill);
                    }
                }
                Err(reason) => {
                    eprintln!("warning: skipping skill {}: {reason}", file.display());
                }
            }
        }
    }
    SkillSet { skills }
}

/// The working dir and its ancestors up to (and including) the repo root — the
/// first dir containing `.git`, or the filesystem root. Nearest first.
fn project_dirs(cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        dirs.push(d.to_path_buf());
        if d.join(".git").exists() {
            break;
        }
        dir = d.parent();
    }
    dirs
}

/// The shared cross-tool skills dir for `dir`: `.agents/skills` when present,
/// otherwise `.claude/skills`; `None` when neither exists.
fn shared_pair(dir: &Path) -> Option<PathBuf> {
    let agents = dir.join(".agents/skills");
    if agents.is_dir() {
        return Some(agents);
    }
    let claude = dir.join(".claude/skills");
    claude.is_dir().then_some(claude)
}

/// Collect `SKILL.md` paths under `dir` (depth-bounded, sorted for determinism,
/// dot-dirs skipped).
fn collect_skill_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH || !dir.is_dir() {
        return;
    }
    let skill_md = dir.join("SKILL.md");
    if skill_md.is_file() {
        out.push(skill_md);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
        })
        .collect();
    subdirs.sort();
    for sub in subdirs {
        collect_skill_files(&sub, depth + 1, out);
    }
}

/// The frontmatter keys wcode reads. Unknown keys (`license`, `allowed-tools`,
/// …) are ignored — per-skill tool policy is a `Hooks` concern, not config.
#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// Read and validate one `SKILL.md`.
fn load_skill(path: &Path) -> Result<Skill, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let fm = parse_frontmatter(&text)?;

    let name = fm.name.map(|s| s.trim().to_string()).unwrap_or_default();
    if name.is_empty() {
        return Err("missing `name`".into());
    }
    if !valid_name(&name) {
        return Err(format!("invalid skill name `{name}`"));
    }
    let description = fm
        .description
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if description.is_empty() {
        return Err("missing `description`".into());
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "description is {} chars (max {MAX_DESCRIPTION_CHARS})",
            description.chars().count()
        ));
    }
    Ok(Skill {
        name,
        description,
        path: path.to_path_buf(),
    })
}

/// Parse the leading `---` YAML frontmatter block.
fn parse_frontmatter(text: &str) -> Result<Frontmatter, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))
        .ok_or("missing frontmatter (no leading `---`)")?;
    let end = rest.find("\n---").ok_or("unterminated frontmatter")?;
    serde_yaml_ng::from_str(&rest[..end]).map_err(|e| format!("invalid YAML: {e}"))
}

/// A skill name: 1..=64 chars of `[a-z0-9-]`, no leading/trailing/consecutive
/// hyphens. The directory name is *not* required to match (shared skill dirs).
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// One line, whitespace-collapsed, truncated to `max` chars with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let mut out: String = one_line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `path` relative to `cwd` when under it, else absolute.
fn display_path(path: &Path, cwd: &Path) -> String {
    match path.strip_prefix(cwd) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, description: &str) -> PathBuf {
        let path = dir.join("SKILL.md");
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            &path,
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# Body\n"),
        )
        .unwrap();
        path
    }

    fn repo() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        root
    }

    #[test]
    fn parses_and_validates_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_skill(&dir.path().join("ok"), "pdf-tools", "Do PDF things.");
        let skill = load_skill(&path).unwrap();
        assert_eq!(skill.name, "pdf-tools");
        assert_eq!(skill.description, "Do PDF things.");

        // Unknown keys are ignored; quoted/multiline values parse.
        std::fs::write(
            &path,
            "---\nname: quoted\ndescription: \"a: b\"\nlicense: MIT\nallowed-tools: bash\n---\n",
        )
        .unwrap();
        let skill = load_skill(&path).unwrap();
        assert_eq!(skill.description, "a: b");
    }

    #[test]
    fn rejects_invalid_skills() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");

        for (body, needle) in [
            ("# no frontmatter\n", "missing frontmatter"),
            ("---\nname: x\n---\n", "missing `description`"),
            ("---\ndescription: x\n---\n", "missing `name`"),
            ("---\nname: Bad Name\ndescription: x\n---\n", "invalid skill name"),
            ("---\nname: -lead\ndescription: x\n---\n", "invalid skill name"),
            ("---\nname: a--b\ndescription: x\n---\n", "invalid skill name"),
            ("---\nname: [unclosed\ndescription: x\n---\n", "invalid YAML"),
        ] {
            std::fs::write(&path, body).unwrap();
            let err = load_skill(&path).unwrap_err();
            assert!(err.contains(needle), "for {body:?} got {err:?}");
        }
    }

    #[test]
    fn project_skills_win_over_global_and_nearest_wins() {
        let repo = repo();
        let sub = repo.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        write_skill(&repo.path().join(".wcode/skills/root"), "root", "from root");
        write_skill(&sub.join(".wcode/skills/near"), "near", "from near");
        let home = tempfile::tempdir().unwrap();
        write_skill(&home.path().join(".local/share/wcode/skills/glob"), "glob", "from home");
        // Same name in global and project: the project one wins.
        write_skill(&repo.path().join(".wcode/skills/dup"), "dup", "project dup");
        write_skill(&home.path().join(".local/share/wcode/skills/dup"), "dup", "global dup");

        let set = discover(&Spec::default(), &sub, Some(home.path()));
        let names: Vec<&str> = set.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["near", "dup", "root", "glob"]);
        let dup = set.skills.iter().find(|s| s.name == "dup").unwrap();
        assert_eq!(dup.description, "project dup");
    }

    #[test]
    fn shared_pair_prefers_agents_then_falls_back_to_claude() {
        let repo = repo();
        let sub = repo.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_skill(&sub.join(".agents/skills/shared"), "shared", "agents dir");
        write_skill(&sub.join(".claude/skills/claude"), "claude", "claude dir");

        // `.agents` exists, so `.claude` is not read at this location.
        let set = discover(&Spec::default(), &sub, None);
        let names: Vec<&str> = set.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["shared"]);

        // Without `.agents`, the `.claude` dir is the fallback.
        std::fs::remove_dir_all(sub.join(".agents")).unwrap();
        let set = discover(&Spec::default(), &sub, None);
        let names: Vec<&str> = set.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["claude"]);
    }

    #[test]
    fn explicit_roots_scan_first_and_disabled_names_are_skipped() {
        let repo = repo();
        let sub = repo.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_skill(&sub.join(".wcode/skills/proj"), "dup", "project");
        let extra = tempfile::tempdir().unwrap();
        write_skill(&extra.path().join("extra"), "dup", "explicit");

        let spec = Spec {
            roots: vec![extra.path().to_path_buf()],
            ..Spec::default()
        };
        let set = discover(&spec, &sub, None);
        assert_eq!(set.skills.len(), 1);
        assert_eq!(set.skills[0].description, "explicit");

        let spec = Spec {
            disabled: vec!["dup".into()],
            ..Spec::default()
        };
        assert!(discover(&spec, &sub, None).is_empty());
    }

    #[test]
    fn scan_stops_at_the_depth_bound() {
        let repo = repo();
        let deep = repo.path().join("a/b/c/d/e/f");
        std::fs::create_dir_all(&deep).unwrap();
        write_skill(&deep, "too-deep", "x");

        // Depth 6 below the root exceeds MAX_SCAN_DEPTH (4).
        assert!(discover(&Spec::default(), repo.path(), None).is_empty());
    }

    #[test]
    fn find_returns_the_discovered_skill() {
        let repo = repo();
        let sub = repo.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_skill(&sub.join(".wcode/skills/one"), "alpha", "first");
        write_skill(&sub.join(".wcode/skills/two"), "beta", "second");

        let set = discover(&Spec::default(), &sub, None);
        assert_eq!(set.find("alpha").map(|s| s.description.as_str()), Some("first"));
        assert_eq!(set.find("beta").map(|s| s.description.as_str()), Some("second"));
        assert!(set.find("missing").is_none());
    }

    #[test]
    fn render_uses_relative_paths_clips_and_degrades() {
        let repo = repo();
        let cwd = repo.path().join("sub");
        std::fs::create_dir_all(&cwd).unwrap();
        let long = "word ".repeat(80);
        let path = write_skill(&cwd.join(".wcode/skills/s"), "a-skill", long.trim());

        let set = discover(&Spec::default(), &cwd, None);
        let rendered = set.render(&cwd).unwrap();
        assert!(rendered.starts_with("# Available skills"), "{rendered}");
        assert!(rendered.contains("- a-skill — word word"), "{rendered}");
        assert!(rendered.contains('…'), "description should be clipped: {rendered}");
        assert!(
            rendered.contains("(read .wcode/skills/s/SKILL.md)"),
            "relative path expected: {rendered}"
        );
        // A skill outside cwd keeps its absolute path.
        let outside = write_skill(&repo.path().join("other"), "outside", "y");
        assert!(display_path(&outside, &cwd).starts_with('/'));

        assert!(SkillSet::default().render(&cwd).is_none());
        assert!(path.exists());
    }
}
