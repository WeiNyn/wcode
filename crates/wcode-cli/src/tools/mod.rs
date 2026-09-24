pub mod anchor;
pub mod ast;
pub mod ast_edit;
pub mod ast_search;
pub mod bash;
pub mod background;
pub mod diff;
pub mod edit;
pub mod edits;
pub mod find;
pub mod grep;
pub mod message;
pub mod peers;
pub mod read;
pub mod replace;
pub mod spawn;
pub mod task;
pub mod session_search;
pub mod todo;
pub mod webfetch;
pub mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wcode_harness::tool::{Tool, ToolOutput, erased};

use crate::config::ToolsConfig;

pub fn default_tools(
    cfg: &ToolsConfig,
    sessions_dir: &Path,
    bg: Arc<background::Background>,
) -> Vec<Tool> {
    // Mutating tools share one lock: read-only tools may run concurrently with
    // each other (see `TypedTool::parallel_safe`), but no two mutations and no
    // read-vs-mutation interleave inside a batch.
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let mut tools = vec![
        erased(read::Read),
        // A core read-only capability (like `read`, D6): always on, not behind
        // the mutation `lock`; `WebFetch::new()` builds its client once (D10).
        erased(webfetch::WebFetch::new()),
        erased(bash::Bash::new(bg.clone())),
        // Always registered, beside `bash` (D14): `bash { background: true }`
        // starts a task and `bg` manages it.
        erased(background::Bg::new(bg.clone())),
        erased(edit::Edit::new(lock.clone())),
        erased(edits::Edits::new(lock.clone())),
        erased(replace::Replace::new(lock.clone())),
        erased(write::Write::new(lock.clone())),
        // Session-local and event-sourced: every session (workers included) gets
        // it; not behind the mutation `lock` (its `parallel_safe` default makes a
        // write a barrier anyway).
        erased(todo::Todo::new()),
        // A core read-only capability (like `read`): always on, not behind the
        // mutation `lock`.
        erased(session_search::SessionSearch::new(sessions_dir.to_path_buf())),
    ];
    // grep/find are redundant with `bash` (it can grep/find itself), so they
    // register only when explicitly enabled in `[tools]`.
    if cfg.grep {
        tools.push(erased(grep::Grep));
    }
    if cfg.find {
        tools.push(erased(find::Find));
    }
    // ast-grep shell-out follows the rtk "auto" pattern: registered only when
    // the `sg` binary is on PATH so the model never holds an unusable tool.
    if ast_search::available() {
        tools.push(erased(ast_search::AstSearch));
    }
    // ast_edit rewrites files, so it shares the mutation lock; registered only
    // when an ast-grep binary is on PATH (same auto pattern).
    if ast_edit::available() {
        tools.push(erased(ast_edit::AstEdit::new(lock.clone())));
    }
    tools
}

pub(crate) fn resolve(working_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        working_dir.join(p)
    }
}

/// Lexically normalize a path for comparison: drop `.` components and resolve
/// `..` against the preceding component, without touching the filesystem. So
/// `a`, `./a`, and `sub/../a` compare equal even though [`resolve`] returns
/// different `PathBuf`s for them.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // `..` at the root is a no-op; otherwise keep it unresolved.
                Some(Component::RootDir) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Whether a walked path passes the include/exclude globs compiled by
/// [`grep::parse_globs`]: an excluded path is out; an empty include set admits
/// everything; otherwise any include match admits it. Shared by the `grep` and
/// `find` tools so their filter semantics cannot drift apart.
pub(crate) fn include_path(
    path: &str,
    includes: &[globset::GlobMatcher],
    excludes: &[globset::GlobMatcher],
) -> bool {
    if excludes.iter().any(|m| m.is_match(path)) {
        return false;
    }
    if includes.is_empty() {
        return true;
    }
    includes.iter().any(|m| m.is_match(path))
}

/// Whole-file compare-and-swap guard shared by the mutating tools (`edit`,
/// `edits`, `replace`, `write`): when the caller supplied an `expected_digest`,
/// refuse (an error `ToolOutput` naming both digests) if the file's current
/// content no longer matches it. `None` means the digest matched (or none was
/// given) and the caller proceeds. `content` is the file's current text; `path`
/// is used only to name the file in the message.
pub(crate) fn stale_digest_guard(
    path: &str,
    content: &str,
    expected: Option<&str>,
) -> Option<ToolOutput> {
    let expected = expected?;
    let actual = anchor::file_digest(content.as_bytes());
    if actual == expected {
        return None;
    }
    Some(ToolOutput {
        output: crate::workspace::stale_digest(path, expected, &actual),
        is_error: true,
        ..ToolOutput::default()
    })
}

/// Same-directory temp name for atomic write+rename mutations. PID-suffixed so
/// two wcode processes editing the same file can't clobber each other's temp
/// (rename is still atomic — last writer wins, never a truncation), and it
/// never collides with a user file that happens to be named `*.tmp-wcode`.
pub(crate) fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{name}.tmp-wcode-{}", std::process::id()))
}

#[cfg(test)]
pub(crate) fn test_ctx(
    dir: &Path,
) -> (
    wcode_harness::tool::ToolContext,
    tokio::sync::mpsc::UnboundedReceiver<wcode_harness::event::AgentEvent>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let ctx = wcode_harness::tool::ToolContext {
        call_id: "t1".to_string(),
        name: "test".to_string(),
        working_dir: dir.to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        events: tx,
        session_path: None,
    };
    (ctx, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(cfg: &ToolsConfig) -> Vec<String> {
        default_tools(cfg, &std::env::temp_dir(), background::Background::new())
            .iter()
            .map(|t| t.name().to_string())
            .collect()
    }

    #[test]
    fn normalize_collapses_dot_and_dotdot() {
        let n = |s: &str| normalize(Path::new(s));
        assert_eq!(n("./f.txt"), PathBuf::from("f.txt"));
        assert_eq!(n("f.txt"), PathBuf::from("f.txt"));
        assert_eq!(n("sub/../f.txt"), PathBuf::from("f.txt"));
        assert_eq!(n("a/b/../c"), PathBuf::from("a/c"));
        assert_eq!(n("/x/../y"), PathBuf::from("/y"));
        assert_eq!(n("a/../../b"), PathBuf::from("../b"));
        assert_eq!(n("/.."), PathBuf::from("/"));
        assert_eq!(n(""), PathBuf::from("."));
    }

    #[test]
    fn default_tools_omit_grep_and_find() {
        // bash can grep/find, so the native tools are off unless enabled.
        let n = names(&ToolsConfig::default());
        assert!(!n.iter().any(|s| s == "grep"));
        assert!(!n.iter().any(|s| s == "find"));
        // `session_search` is a core read-only capability (like `read`): always
        // registered, never gated. Finding #1: this pins the registration.
        for core in [
            "read",
            "bash",
            "bg",
            "webfetch",
            "edit",
            "edits",
            "replace",
            "write",
            "session_search",
            "todo",
        ] {
            assert!(n.contains(&core.to_string()), "{core} missing from {n:?}");
        }
    }

    #[test]
    fn default_tools_include_grep_and_find_when_enabled() {
        let n = names(&ToolsConfig {
            grep: true,
            find: true,
            ..ToolsConfig::default()
        });
        assert!(n.iter().any(|s| s == "grep"));
        assert!(n.iter().any(|s| s == "find"));
    }

    #[test]
    #[ignore = "diagnostic: print tool definition sizes"]
    fn print_tool_definition_sizes() {
        let tools = default_tools(
            &ToolsConfig::default(),
            &std::env::temp_dir(),
            background::Background::new(),
        );
        let mut total = 0usize;
        for t in &tools {
            let d = t.definition();
            let json = serde_json::to_string(&d).unwrap();
            total += json.len();
            println!(
                "{:<12} desc={:>4}ch params={:>6}ch json={:>6}ch  ~tok={:>3}",
                d.name,
                d.description.len(),
                d.parameters.to_string().len(),
                json.len(),
                json.len() / 4
            );
            println!("    params: {}", d.parameters);
        }
        println!(
            "TOTAL tool-def JSON: {total} chars ~= {} tokens (chars/4)",
            total / 4
        );
    }

    #[test]
    fn every_mutating_tool_is_in_the_plan_mode_denylist() {
        // Keep `MUTATING_TOOLS` (plan mode's denylist) in sync with the tools
        // that declare themselves `mutating`.
        let cfg = ToolsConfig {
            grep: true,
            find: true,
            ..ToolsConfig::default()
        };
        for tool in default_tools(&cfg, &std::env::temp_dir(), background::Background::new()) {
            if tool.mutating() {
                assert!(
                    wcode_harness::hooks::MUTATING_TOOLS.contains(&tool.name()),
                    "`{}` is mutating but missing from MUTATING_TOOLS",
                    tool.name()
                );
            }
        }
    }
}
