//! Instruction ("reference") files folded into the system prompt.
//!
//! Two sources, merged in this order:
//!
//! 1. an optional **global** file in wcode's config directory
//!    (`~/.config/wcode/`), and
//! 2. the **ancestor chain** from the repo root down to the working directory,
//!    so the nearest (most specific) file is rendered last.
//!
//! Per directory a candidate-name list is tried and the first hit wins. A bare
//! name is discovered by walking up; a name containing a separator (or an
//! absolute path) is taken verbatim. Everything is best-effort: unreadable or
//! empty files are skipped, never fatal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Max bytes of a single instruction file folded into the system prompt.
/// Beyond this it is truncated — a huge `AGENTS.md` would crowd out the task.
pub const MAX_FILE_BYTES: usize = 32_768;

/// Max bytes of the combined instructions block. Files that would exceed it are
/// dropped from the least specific end (the global file first), so the nearest
/// project file always survives.
pub const MAX_TOTAL_BYTES: usize = 65_536;

/// Default candidate names, tried in order per directory (first hit wins).
/// `CLAUDE.md` is included for portability, as pi and jcode do.
pub const DEFAULT_INSTRUCTION_NAMES: &[&str] =
    &["AGENTS.override.md", "AGENTS.md", "CLAUDE.md"];

/// How instruction files are resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No instructions.
    Off,
    /// Load exactly this file — a name walked up from the working dir, or a
    /// path (relative to it, or absolute). No discovery of the other names.
    Explicit(String),
    /// Discover the candidate `names` per directory (plus the global file when
    /// `global`).
    Discover { names: Vec<String>, global: bool },
}

/// One discovered instruction file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instructions {
    pub path: PathBuf,
    pub text: String,
    /// True for the global (config-dir) file.
    pub global: bool,
}

/// The discovered set, in display order: global first, then repo root → working
/// directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstructionSet {
    pub files: Vec<Instructions>,
}

impl InstructionSet {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The files as `# (Project|Global) instructions (<path>)` blocks joined by
    /// blank lines, or `None` when there are none.
    pub fn render(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut out = String::new();
        for (i, f) in self.files.iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            let scope = if f.global { "Global" } else { "Project" };
            out.push_str(&format!(
                "# {scope} instructions ({})\n{}",
                f.path.display(),
                f.text.trim_end()
            ));
        }
        Some(out)
    }
}

/// Discover and read the instruction files for `cwd`. `config_dir` is the
/// global directory (`~/.config/wcode`), consulted only in discovery mode.
pub fn load(mode: &Mode, cwd: &Path, config_dir: Option<&Path>) -> InstructionSet {
    let mut files = Vec::new();
    match mode {
        Mode::Off => {}
        Mode::Explicit(spec) => {
            if let Some(path) = resolve_explicit(cwd, spec)
                && let Some(text) = read_capped(&path)
            {
                files.push(Instructions {
                    path,
                    text,
                    global: false,
                });
            }
        }
        Mode::Discover { names, global } => {
            let mut seen = HashSet::new();
            if *global
                && let Some(dir) = config_dir
                && let Some((path, text)) = first_in(dir, names)
            {
                seen.insert(canonical(&path));
                files.push(Instructions {
                    path,
                    text,
                    global: true,
                });
            }
            for dir in repo_chain(cwd) {
                let Some((path, text)) = first_in(&dir, names) else {
                    continue;
                };
                if !seen.insert(canonical(&path)) {
                    continue;
                }
                files.push(Instructions {
                    path,
                    text,
                    global: false,
                });
            }
        }
    }
    cap(files)
}

/// The first existing candidate in `dir` (with its capped contents).
fn first_in(dir: &Path, names: &[String]) -> Option<(PathBuf, String)> {
    names.iter().find_map(|name| {
        let path = dir.join(name);
        read_capped(&path).map(|text| (path, text))
    })
}

/// Read a file, capped, or `None` when unreadable/blank.
fn read_capped(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let text = truncate(std::fs::read_to_string(path).ok()?, MAX_FILE_BYTES);
    (!text.trim().is_empty()).then_some(text)
}

/// Resolve an explicit spec to an existing file: an absolute path or one with a
/// separator is taken verbatim; a bare name walks up from `cwd`.
fn resolve_explicit(cwd: &Path, spec: &str) -> Option<PathBuf> {
    let p = Path::new(spec);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    if spec.contains('/') || spec.contains(std::path::MAIN_SEPARATOR) {
        let candidate = cwd.join(spec);
        return candidate.is_file().then_some(candidate);
    }
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let candidate = d.join(spec);
        if candidate.is_file() {
            return Some(candidate);
        }
        if d.join(".git").exists() {
            break; // repo root: don't search above it
        }
        dir = d.parent();
    }
    None
}

/// `cwd` and its ancestors up to (and including) the repo root — the first dir
/// containing `.git`, or the filesystem root when there is none. Returned
/// repo-root-first, so the working dir is last.
fn repo_chain(cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        dirs.push(d.to_path_buf());
        if d.join(".git").exists() {
            break;
        }
        dir = d.parent();
    }
    dirs.reverse();
    dirs
}

/// Canonical path for dedup, falling back to the path itself.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Enforce the total budget by dropping whole files from the least specific end.
fn cap(mut files: Vec<Instructions>) -> InstructionSet {
    while files.len() > 1 && files.iter().map(|f| f.text.len()).sum::<usize>() > MAX_TOTAL_BYTES {
        files.remove(0);
    }
    if let Some(first) = files.first_mut()
        && first.text.len() > MAX_TOTAL_BYTES
    {
        first.text = truncate(std::mem::take(&mut first.text), MAX_TOTAL_BYTES);
    }
    InstructionSet { files }
}

/// Truncate `text` to at most `max` bytes on a char boundary, appending a
/// marker when it was cut.
pub fn truncate(mut text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n… [instructions truncated]");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        DEFAULT_INSTRUCTION_NAMES.iter().map(|s| (*s).to_string()).collect()
    }

    fn discover() -> Mode {
        Mode::Discover {
            names: names(),
            global: true,
        }
    }

    fn repo() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn discovers_ancestors_root_first_nearest_last() {
        let root = repo();
        write(&root.path().join("AGENTS.md"), "ROOT");
        let sub = root.path().join("a/b");
        write(&sub.join("AGENTS.md"), "NEAR");

        let set = load(&discover(), &sub, None);
        let texts: Vec<&str> = set.files.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts, ["ROOT", "NEAR"]);

        // The global file is loaded first when a config dir is given.
        let cfg = tempfile::tempdir().unwrap();
        write(&cfg.path().join("AGENTS.md"), "GLOBAL");
        let set = load(&discover(), &sub, Some(cfg.path()));
        let texts: Vec<&str> = set.files.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts, ["GLOBAL", "ROOT", "NEAR"]);
        assert!(set.files[0].global && !set.files[1].global);
    }

    #[test]
    fn candidate_priority_first_hit_wins() {
        let root = repo();
        write(&root.path().join("AGENTS.md"), "AGENTS");
        write(&root.path().join("AGENTS.override.md"), "OVERRIDE");
        write(&root.path().join("CLAUDE.md"), "CLAUDE");
        let sub = root.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        let set = load(&discover(), &sub, None);
        assert_eq!(set.files.len(), 1);
        assert_eq!(set.files[0].text, "OVERRIDE");

        // With no AGENTS.*, CLAUDE.md is the fallback candidate.
        std::fs::remove_file(root.path().join("AGENTS.md")).unwrap();
        std::fs::remove_file(root.path().join("AGENTS.override.md")).unwrap();
        let set = load(&discover(), &sub, None);
        assert_eq!(set.files[0].text, "CLAUDE");
    }

    #[test]
    fn stops_at_the_repo_root() {
        let outer = tempfile::tempdir().unwrap();
        write(&outer.path().join("AGENTS.md"), "OUTER");
        let repo_root = outer.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();
        let sub = repo_root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        // The repo root (`.git`) bounds the search, so OUTER is never seen.
        assert!(load(&discover(), &sub, None).is_empty());
    }

    #[test]
    fn explicit_override_loads_only_that_file() {
        let root = repo();
        write(&root.path().join("AGENTS.md"), "ROOT");
        write(&root.path().join("docs/guide.md"), "GUIDE");
        let sub = root.path().join("a");
        std::fs::create_dir_all(&sub).unwrap();

        // A bare name walks up, and does *not* also pick up the other names.
        let set = load(&Mode::Explicit("AGENTS.md".into()), &sub, None);
        assert_eq!(set.files.len(), 1);
        assert_eq!(set.files[0].text, "ROOT");

        // A spec with a separator is taken relative to the working dir.
        let set = load(&Mode::Explicit("docs/guide.md".into()), root.path(), None);
        assert_eq!(set.files[0].text, "GUIDE");

        // A missing explicit file yields nothing.
        assert!(load(&Mode::Explicit("nope.md".into()), &sub, None).is_empty());
    }

    #[test]
    fn dedups_a_file_seen_twice() {
        let root = repo();
        write(&root.path().join("AGENTS.md"), "SAME");
        // The global dir *is* the repo root here: the file must appear once.
        let set = load(&discover(), &root.path().join("."), Some(root.path()));
        assert_eq!(set.files.len(), 1);
    }

    #[test]
    fn off_loads_nothing() {
        let root = repo();
        write(&root.path().join("AGENTS.md"), "ROOT");
        assert!(load(&Mode::Off, root.path(), None).is_empty());
    }

    #[test]
    fn truncates_large_files() {
        let root = repo();
        let big = "x".repeat(MAX_FILE_BYTES + 5_000);
        write(&root.path().join("AGENTS.md"), &big);
        let sub = root.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        let set = load(&discover(), &sub, None);
        assert!(set.files[0].text.len() < big.len());
        assert!(set.files[0].text.ends_with("[instructions truncated]"));
    }

    #[test]
    fn render_marks_scope_and_path() {
        let set = InstructionSet {
            files: vec![
                Instructions {
                    path: PathBuf::from("/cfg/AGENTS.md"),
                    text: "global\n".into(),
                    global: true,
                },
                Instructions {
                    path: PathBuf::from("/repo/AGENTS.md"),
                    text: "project\n".into(),
                    global: false,
                },
            ],
        };
        let rendered = set.render().unwrap();
        assert!(rendered.contains("# Global instructions (/cfg/AGENTS.md)"), "{rendered}");
        assert!(rendered.contains("# Project instructions (/repo/AGENTS.md)"), "{rendered}");
        assert!(rendered.contains("\nglobal\n\n# Project"), "{rendered}");
        assert!(InstructionSet::default().render().is_none());
    }
}
