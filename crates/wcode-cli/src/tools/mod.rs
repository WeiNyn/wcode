pub mod anchor;
pub mod ast;
pub mod ast_edit;
pub mod ast_search;
pub mod bash;
pub mod diff;
pub mod edit;
pub mod edits;
pub mod find;
pub mod grep;
pub mod read;
pub mod replace;
pub mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wcode_harness::tool::{Tool, erased};

use crate::config::ToolsConfig;

pub fn default_tools(cfg: &ToolsConfig) -> Vec<Tool> {
    // Mutating tools share one lock: read-only tools may run concurrently with
    // each other (see `TypedTool::parallel_safe`), but no two mutations and no
    // read-vs-mutation interleave inside a batch.
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let mut tools = vec![
        erased(read::Read),
        erased(bash::Bash),
        erased(edit::Edit::new(lock.clone())),
        erased(edits::Edits::new(lock.clone())),
        erased(replace::Replace::new(lock.clone())),
        erased(write::Write::new(lock.clone())),
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
    };
    (ctx, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(cfg: &ToolsConfig) -> Vec<String> {
        default_tools(cfg)
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
        for core in ["read", "bash", "edit", "edits", "replace", "write"] {
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
        let tools = default_tools(&ToolsConfig::default());
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
}
