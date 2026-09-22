use std::sync::Arc;

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WriteArgs {
    /// File path (relative to the working directory unless absolute); parents are created.
    pub path: String,
    /// Full file contents (overwrites).
    pub content: String,
    /// Whole-file digest from your last read of this file (the `# <path>
    /// digest <hex>` line `read` prints). Auto-filled by the harness; normally
    /// leave unset. A mismatch refuses the call (E_STALE_DIGEST) — re-read.
    #[serde(default)]
    pub expected_digest: Option<String>,
}

pub struct Write {
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl Write {
    pub fn new(lock: Arc<tokio::sync::Mutex<()>>) -> Self {
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
        "Write full contents to a file, creating parent directories. Overwrites existing files. content is written byte-for-byte — leading whitespace preserved exactly; never reformats."
    }
    /// Mutates the workspace — blocked in plan mode (`MUTATING_TOOLS`).
    fn mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let _guard = self.lock.lock().await;
        let path = super::resolve(&ctx.working_dir, &args.path);
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return ToolOutput {
                output: format!("write {}: {e}", args.path),
                is_error: true,
                diff: None,
                path: None,
            };
        }
        // Best-effort read of the previous contents so the diff shows what an
        // overwrite replaced; a new file diffs as all additions.
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        // A8 — whole-file CAS, verified HERE (after the `old` read, after
        // `create_dir_all`, before the diff/write). A missing file reads as "",
        // so a stale real digest mismatches (correct) while digest("") proceeds.
        if let Some(expected) = &args.expected_digest {
            let actual = anchor::file_digest(old.as_bytes());
            if &actual != expected {
                return ToolOutput {
                    output: crate::workspace::stale_digest(&args.path, expected, &actual),
                    is_error: true,
                    ..ToolOutput::default()
                };
            }
        }
        let diff = super::diff::unified(&old, &args.content);
        // ponytail: tmp+rename so a crash mid-write can't truncate the original (same-fs rename).
        let tmp = super::temp_path(&path);
        match std::fs::write(&tmp, &args.content).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => ToolOutput {
                output: format!("wrote {} bytes to {}", args.content.len(), args.path),
                is_error: false,
                diff,
                path: Some(args.path.clone()),
            },
            Err(e) => ToolOutput {
                output: format!("write {}: {e}", args.path),
                is_error: true,
                diff: None,
                path: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_digest_refuses_the_write() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "old").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Write::new(Arc::new(tokio::sync::Mutex::new(())))
            .execute(
                WriteArgs {
                    path: "f.txt".into(),
                    content: "new".into(),
                    expected_digest: Some("000000000000".into()),
                },
                &ctx,
            )
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("E_STALE_DIGEST"), "{}", out.output);
        assert!(out.output.contains("re-read"), "{}", out.output);
        // Nothing written.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "old"
        );
    }

    #[tokio::test]
    async fn matching_digest_allows_the_write() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "old").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Write::new(Arc::new(tokio::sync::Mutex::new(())))
            .execute(
                WriteArgs {
                    path: "f.txt".into(),
                    content: "new".into(),
                    expected_digest: Some(anchor::file_digest(b"old")),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "new"
        );
    }

    #[tokio::test]
    async fn a_missing_file_hashes_as_empty() {
        // The documented edge: a missing file reads as "", so digest("") is a
        // match and the write re-creates it; a real digest would mismatch.
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Write::new(Arc::new(tokio::sync::Mutex::new(())))
            .execute(
                WriteArgs {
                    path: "new.txt".into(),
                    content: "hello".into(),
                    expected_digest: Some(anchor::file_digest(b"")),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let out = Write::new(lock)
            .execute(
                WriteArgs {
                    path: "a/b/c.txt".into(),
                    content: "hello".into(),
                    expected_digest: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a/b/c.txt")).unwrap(),
            "hello"
        );
        assert!(out.output.contains("5 bytes"));
    }
}

