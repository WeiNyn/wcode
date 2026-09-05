pub mod bash;
pub mod edit;
pub mod read;
pub mod write;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use wcode_harness::tool::{Tool, erased};

pub fn default_tools() -> Vec<Tool> {
    // ponytail: full mutation queue when parallel exec lands
    let lock = Arc::new(Mutex::new(()));
    vec![
        erased(read::Read),
        erased(bash::Bash),
        erased(edit::Edit::new(lock.clone())),
        erased(write::Write::new(lock)),
    ]
}

pub(crate) fn resolve(working_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        working_dir.join(p)
    }
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
