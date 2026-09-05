use std::sync::{Arc, Mutex};

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WriteArgs {
    /// File path (relative to the working directory unless absolute); parents are created.
    pub path: String,
    /// Full file contents (overwrites).
    pub content: String,
}

pub struct Write {
    lock: Arc<Mutex<()>>,
}

impl Write {
    pub fn new(lock: Arc<Mutex<()>>) -> Self {
        Self { lock }
    }
}

#[async_trait::async_trait]
impl TypedTool for Write {
    type Args = WriteArgs;
    fn name(&self) -> &str {
        "write"
    }
    fn description(&self) -> &str {
        "Write full contents to a file, creating parent directories. Overwrites existing files."
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let _guard = self.lock.lock().unwrap();
        let path = super::resolve(&ctx.working_dir, &args.path);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolOutput {
                    output: format!("write {}: {e}", args.path),
                    is_error: true,
                    details: None,
                };
            }
        }
        // ponytail: tmp+rename so a crash mid-write can't truncate the original (same-fs rename).
        let tmp = path.with_extension("tmp-wcode");
        match std::fs::write(&tmp, &args.content).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => ToolOutput {
                output: format!("wrote {} bytes to {}", args.content.len(), args.path),
                is_error: false,
                details: None,
            },
            Err(e) => ToolOutput {
                output: format!("write {}: {e}", args.path),
                is_error: true,
                details: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let lock = Arc::new(Mutex::new(()));
        let out = Write::new(lock)
            .execute(
                WriteArgs {
                    path: "a/b/c.txt".into(),
                    content: "hello".into(),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert_eq!(std::fs::read_to_string(dir.path().join("a/b/c.txt")).unwrap(), "hello");
        assert!(out.output.contains("5 bytes"));
    }
}
