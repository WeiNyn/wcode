pub mod anchor;
pub mod ast;
pub mod ast_edit;
pub mod ast_search;
pub mod bash;
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
    // ponytail: full mutation queue when parallel exec lands
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
        });
        assert!(n.iter().any(|s| s == "grep"));
        assert!(n.iter().any(|s| s == "find"));
    }
}
