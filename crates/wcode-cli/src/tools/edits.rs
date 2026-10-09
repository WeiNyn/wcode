use std::sync::Arc;

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

/// One literal-text edit, the same shape as a single `edit` call.
#[derive(Deserialize, schemars::JsonSchema, Debug, Clone)]
pub struct EditOp {
    /// The exact text to replace — byte-exact, so whitespace counts. Must occur
    /// exactly once in the content as of this op's turn unless `replace_all`.
    pub old_string: String,
    /// The text to put in its place, verbatim. Empty deletes the match.
    pub new_string: String,
    /// Replace every occurrence instead of requiring a unique match.
    pub replace_all: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct EditsArgs {
    /// File path (relative to the working directory unless absolute). The whole
    /// batch targets this ONE file.
    pub path: String,
    /// The ops, applied **in order** to the accumulating content. The batch is
    /// atomic: if any op fails, nothing is written.
    pub edits: Vec<EditOp>,
}

pub struct Edits {
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl Edits {
    pub fn new(lock: Arc<tokio::sync::Mutex<()>>) -> Self {
        Self { lock }
    }
}

fn op_label(idx: usize) -> String {
    format!("op{}", idx + 1)
}

fn refuse(output: String) -> ToolOutput {
    ToolOutput {
        output,
        is_error: true,
        diff: None,
        path: None,
    }
}

#[async_trait::async_trait]
impl TypedTool for Edits {
    type Args = EditsArgs;
    fn name(&self) -> &str {
        "edits"
    }
    fn description(&self) -> &str {
        "Apply several literal-string edits to ONE file atomically. Ops run in order against the accumulating content; each `old_string` must occur exactly once at its turn unless `replace_all`. If any op fails, the whole batch is refused and nothing is written — so a batch is all-or-nothing, unlike sequential `edit` calls."
    }
    /// Mutates the workspace — blocked in plan mode (`MUTATING_TOOLS`).
    fn mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        // Async lock + sync fs: the guard may cross awaits, but the fs work stays
        // serialized and synchronous.
        let _guard = self.lock.lock().await;
        let path = super::resolve(&ctx.working_dir, &args.path);

        if args.edits.is_empty() {
            return refuse(format!(
                "[E_EMPTY_BATCH] `edits` is empty — nothing to apply to {}.",
                args.path
            ));
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return refuse(format!("edits {}: {e}", args.path));
            }
        };


        // Apply in order against the accumulating content. Op order therefore
        // matters — an op sees the effect of the ops before it, which is what
        // makes a batch expressible that a set of independent ops is not.
        let mut updated = content.clone();
        for (idx, op) in args.edits.iter().enumerate() {
            let label = op_label(idx);
            if op.old_string.is_empty() {
                return refuse(format!(
                    "[E_EMPTY_OLD_STRING] {label} has an empty `old_string`, so there is nothing to match. Nothing was written."
                ));
            }
            let replace_all = op.replace_all.unwrap_or(false);
            let matches = updated.matches(&op.old_string).count();
            if matches == 0 {
                return refuse(format!(
                    "[E_NO_MATCH] {label} `old_string` does not occur in {} as of its turn. The match is byte-exact (whitespace counts); note ops run in order, so an earlier op may have rewritten it. Nothing was written.",
                    args.path
                ));
            }
            if matches > 1 && !replace_all {
                return refuse(format!(
                    "[E_AMBIGUOUS_MATCH] {label} `old_string` occurs {matches} times in {} as of its turn. Include more surrounding lines to pin one, or set `replace_all: true` on that op. Nothing was written.",
                    args.path
                ));
            }
            updated = if replace_all {
                updated.replace(&op.old_string, &op.new_string)
            } else {
                updated.replacen(&op.old_string, &op.new_string, 1)
            };
        }

        if updated == content {
            return ToolOutput {
                output: format!(
                    "no-op: {} already matches the batch — nothing written",
                    args.path
                ),
                is_error: false,
                diff: None,
                path: None,
            };
        }

        let diff = super::diff::unified(&content, &updated);
        // tmp+rename so a crash mid-write can't truncate the original (same-fs rename).
        let tmp = super::temp_path(&path);
        match std::fs::write(&tmp, &updated).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => {
                let n = args.edits.len();
                ToolOutput {
                    output: format!(
                        "edited {} ({n} op{})",
                        args.path,
                        if n == 1 { "" } else { "s" }
                    ),
                    is_error: false,
                    diff,
                    path: Some(args.path.clone()),
                }
            }
            Err(e) => refuse(format!("edits {}: {e}", args.path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> Edits {
        Edits::new(Arc::new(tokio::sync::Mutex::new(())))
    }

    fn op(old: &str, new: &str) -> EditOp {
        EditOp {
            old_string: old.into(),
            new_string: new.into(),
            replace_all: None,
        }
    }

    fn args(path: &str, edits: Vec<EditOp>) -> EditsArgs {
        EditsArgs {
            path: path.into(),
            edits,
        }
    }

    #[tokio::test]
    async fn applies_ops_in_order_against_the_accumulating_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        // op2 matches text that only exists once op1 has run.
        let out = tool()
            .execute(
                args("f.txt", vec![op("a\n", "x\n"), op("x\nb", "x\nY")]),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x\nY\n"
        );
        assert!(out.output.contains("2 ops"), "{}", out.output);
    }

    #[tokio::test]
    async fn a_failing_op_aborts_the_whole_batch_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(
                args("f.txt", vec![op("a", "x"), op("NOT THERE", "y")]),
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("op2"), "{}", out.output);
        assert!(out.output.contains("E_NO_MATCH"), "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a\nb\n",
            "op1 must not land when op2 fails — the batch is atomic"
        );
    }

    #[tokio::test]
    async fn an_ambiguous_op_aborts_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "same\nsame\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(args("f.txt", vec![op("same", "x")]), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_AMBIGUOUS_MATCH"), "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "same\nsame\n"
        );
    }

    #[tokio::test]
    async fn replace_all_on_one_op_covers_every_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x = 1\nx = 1\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let mut o = op("x = 1", "y = 1");
        o.replace_all = Some(true);
        let out = tool().execute(args("f.txt", vec![o]), &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "y = 1\ny = 1\n"
        );
    }

    #[tokio::test]
    async fn an_empty_batch_refuses() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool().execute(args("f.txt", vec![]), &ctx).await;
        assert!(out.is_error);
        assert!(out.output.contains("E_EMPTY_BATCH"), "{}", out.output);
    }

    #[tokio::test]
    async fn a_noop_batch_writes_nothing_and_emits_no_trailer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool().execute(args("f.txt", vec![op("a", "a")]), &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.starts_with("no-op"), "{}", out.output);
    }
}
