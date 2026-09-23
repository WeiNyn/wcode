use std::sync::Arc;

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor::{self, ANCHOR_SEP, Range};

/// One anchor-range edit, identical in shape to the `edit` tool's args.
#[derive(Deserialize, schemars::JsonSchema, Debug, Clone)]
pub struct EditOp {
    /// File path (relative to the working directory unless absolute).
    pub path: String,
    /// Anchor (as shown by `read`) of the first line of the range to replace.
    pub from: String,
    /// Anchor (as shown by `read`) of the last line of the range to replace.
    /// Omit to address a single line.
    pub to: Option<String>,
    /// Replacement text for the range (may be multi-line).
    pub replacement: String,
    /// Optional verification: this text must appear ending at the range or in
    /// the line(s) immediately before it (its own line span bounds how far up
    /// it may reach). Use it to pin one of several identical lines.
    pub old_string: Option<String>,
    /// Replace every matching range instead of requiring exactly one.
    pub replace_all: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct EditsArgs {
    /// The batch of edits to apply. Every op must target the SAME file; the
    /// batch is atomic — any stale/ambiguous/overlapping op aborts the whole
    /// call and nothing is written.
    pub edits: Vec<EditOp>,
    /// Whole-file digest from your last read of this file (the `# <path>
    /// digest <hex>` line `read` prints). Auto-filled by the harness; normally
    /// leave unset. A mismatch refuses the call (E_STALE_DIGEST) — re-read.
    #[serde(default)]
    pub expected_digest: Option<String>,
}

/// Cap on the anchored `Region now:` echo: a batch that spans more than this
/// many lines replies with a re-read hint instead of a huge echo.
const MAX_ECHO_LINES: usize = 120;

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

#[async_trait::async_trait]
impl TypedTool for Edits {
    type Args = EditsArgs;
    fn name(&self) -> &str {
        "edits"
    }
    fn description(&self) -> &str {
        "Apply a batch of anchor-range edits (same args as edit) to ONE file atomically: every op resolves against the same snapshot, and any stale/ambiguous/overlapping op aborts the whole batch with nothing written. Use for several same-file edits in one round-trip."
    }
    /// Mutates the workspace — blocked in plan mode (`MUTATING_TOOLS`).
    fn mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let _guard = self.lock.lock().await;
        if args.edits.is_empty() {
            return ToolOutput {
                output: "[E_EMPTY_BATCH] `edits` needs at least one op.".into(),
                is_error: true,
                diff: None,
                path: None,
            };
        }

        let path = super::resolve(&ctx.working_dir, &args.edits[0].path);
        let norm = super::normalize(&path);
        for op in &args.edits[1..] {
            if super::normalize(&super::resolve(&ctx.working_dir, &op.path)) != norm {
                return ToolOutput {
                    output: format!(
                        "[E_MIXED_FILES] `edits` is scoped to ONE file per call ({} and {} were mixed). Split them into separate `edits` calls.",
                        args.edits[0].path, op.path
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("edits {}: {e}", args.edits[0].path),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        };

        // Whole-file CAS (D1): the batch touches one file, so a single
        // top-level digest covers every op. Verified before any anchor
        // resolution; a mismatch refuses the whole batch, nothing written.
        if let Some(out) = super::stale_digest_guard(
            &args.edits[0].path,
            &content,
            args.expected_digest.as_deref(),
        ) {
            return out;
        }

        let empty = anchor::is_degenerate_empty(&content);
        let original = content.clone();
        let lines = if empty {
            vec![String::new()]
        } else {
            anchor::split_lines(&content)
        };
        let anchors = anchor::anchors_for(&lines);

        // Resolve every op against the ORIGINAL snapshot (op order never
        // matters for correctness). Collect (range, op index) pairs.
        let mut resolved: Vec<(Range, usize, &EditOp)> = Vec::new();
        for (idx, op) in args.edits.iter().enumerate() {
            let label = op_label(idx);
            if !anchor::is_anchor(&op.from) {
                return ToolOutput {
                    output: format!(
                        "[E_BAD_ANCHOR] {} `from` must be a bare 5-char anchor as shown by read (e.g. \"aB3x1\"), got \"{}\". No line numbers, no `{}content, no surrounding text.",
                        label, op.from, ANCHOR_SEP
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
            if let Some(t) = &op.to
                && !anchor::is_anchor(t)
            {
                return ToolOutput {
                    output: format!(
                        "[E_BAD_ANCHOR] {} `to` must be a bare 5-char anchor as shown by read (e.g. \"aB3x1\"), got \"{t}\".",
                        label
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
            let ranges = anchor::find_ranges(
                &lines,
                &anchors,
                &op.from,
                op.to.as_deref(),
                op.old_string.as_deref(),
            );
            if ranges.is_empty() {
                // No positional info to offer: an anchor is a content hash, so a
                // missing `from` says nothing about where the line was. A
                // re-read is the only way to get fresh anchors.
                return ToolOutput {
                    output: format!(
                        "[E_STALE_ANCHOR] {label} has no edit target for from=`{}`{} in {} — the file changed since read. Re-read the file (read {}) and retry with fresh anchors.",
                        op.from,
                        op.to
                            .as_ref()
                            .map(|t| format!(", to=`{t}`"))
                            .unwrap_or_default(),
                        op.path,
                        op.path
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
            if ranges.len() > 1 && !op.replace_all.unwrap_or(false) {
                let candidates = ranges
                    .iter()
                    .map(|r| format!("  line {}: {}", r.start + 1, lines[r.start]))
                    .collect::<Vec<_>>()
                    .join("\n");
                return ToolOutput {
                    output: format!(
                        "[E_AMBIGUOUS_ANCHOR] {label} `{}` matches {} ranges in {} (identical lines share an anchor). Candidates:\n{}\nSet replace_all:true to apply to every match, or re-send with old_string (or a to-anchor) unique to the range you mean.",
                        op.from,
                        ranges.len(),
                        op.path,
                        candidates
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
            for r in ranges {
                resolved.push((r, idx, op));
            }
        }

        // All ranges across all ops must be disjoint: the apply loop reuses
        // original coordinates against accumulating content, which is correct
        // only for non-overlapping ranges (same invariant as edit's own
        // replace_all, extended to the whole batch).
        resolved.sort_by_key(|(r, _, _)| r.start);
        for i in 1..resolved.len() {
            let (prev, prev_op, _) = &resolved[i - 1];
            let (cur, cur_op, _) = &resolved[i];
            if cur.start <= prev.end {
                return ToolOutput {
                    output: format!(
                        "[E_OVERLAPPING_RANGES] {} (lines {}–{}) and {} (lines {}–{}) overlap. Applying both would rewrite already-replaced text — nothing was written. Make the batch's ranges disjoint (drop one op or narrow a `to` anchor).",
                        op_label(*prev_op),
                        prev.start + 1,
                        prev.end + 1,
                        op_label(*cur_op),
                        cur.start + 1,
                        cur.end + 1,
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        }

        // Per-op span summary + bottom-up application, one read of the file.
        let ends_nl = empty || content.ends_with('\n');
        // Per-op (start line, span label) for the ranges actually applied, so
        // the count/summary below reflect real changes, not resolved no-ops.
        let mut applied_spans: Vec<Vec<(usize, String)>> = vec![Vec::new(); args.edits.len()];

        let mut updated = content;
        let mut applied_per_op = vec![0usize; args.edits.len()];
        for (r, idx, op) in resolved.iter().rev() {
            let cur_lines = if updated.is_empty() {
                vec![String::new()]
            } else {
                anchor::split_lines(&updated)
            };
            let (next, changed) = anchor::apply_replace(&cur_lines, r, &op.replacement, ends_nl);
            if changed.is_some() {
                applied_per_op[*idx] += 1;
                applied_spans[*idx].push((
                    r.start,
                    if r.start == r.end {
                        format!("{}", r.start + 1)
                    } else {
                        format!("{}–{}", r.start + 1, r.end + 1)
                    },
                ));
            }
            updated = next;
        }
        if applied_per_op.iter().all(|&n| n == 0) {
            return ToolOutput {
                output: format!(
                    "no-op: {} already matches every op's replacement",
                    args.edits[0].path
                ),
                is_error: false,
                diff: None,
                path: None,
            };
        }

        let tmp = super::temp_path(&path);
        match std::fs::write(&tmp, &updated).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => {
                // Ranges are applied bottom-up; present each op's spans in file
                // order, and count only the ranges that actually changed.
                for spans in applied_spans.iter_mut() {
                    spans.sort_by_key(|(start, _)| *start);
                }
                let summary = applied_spans
                    .iter()
                    .enumerate()
                    .filter(|(_, spans)| !spans.is_empty())
                    .map(|(i, spans)| {
                        let labels: Vec<&str> =
                            spans.iter().map(|(_, label)| label.as_str()).collect();
                        format!("{} → lines {}", op_label(i), labels.join(", "))
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                let total: usize = applied_per_op.iter().sum();
                // Echo the combined changed span only when it's small; a big
                // batch replies with a re-read hint instead of a huge echo.
                let mut echo = String::new();
                if let Some((first, last)) = anchor::changed_range(&original, &updated) {
                    let span = last - first + 1;
                    if span <= MAX_ECHO_LINES {
                        for line in anchor::split_lines(&updated)
                            .iter()
                            .skip(first - 1)
                            .take(span)
                        {
                            echo.push_str(&anchor::render(&anchor::anchor(line), line));
                            echo.push('\n');
                        }
                        echo.push_str(
                            "anchors above are fresh — chain further edits without re-reading.",
                        );
                    } else {
                        echo.push_str(&format!(
                            "{span} lines changed across the file — re-read {} for fresh anchors, then chain further edits.",
                            args.edits[0].path
                        ));
                    }
                }
                let diff = super::diff::unified(&original, &updated);
                ToolOutput {
                    output: format!(
                        "edited {} ({total} range{} applied: {summary}). Region now:\n{echo}",
                        args.edits[0].path,
                        if total == 1 { "" } else { "s" },
                    ),
                    is_error: false,
                    diff,
                    path: Some(args.edits[0].path.clone()),
                }
            }
            Err(e) => ToolOutput {
                output: format!("edits {}: {e}", args.edits[0].path),
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
    async fn stale_digest_refuses_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: Some("000000000000".into()),
                    edits: vec![op("f.txt", &ha, None, "A")],
                },
                &ctx,
            )
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("E_STALE_DIGEST"), "{}", out.output);
        assert!(out.output.contains("re-read"), "{}", out.output);
        // All-or-nothing: the batch was not written.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a\nb\nc\n"
        );
    }

    #[tokio::test]
    async fn matching_digest_applies_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let digest = anchor::file_digest(b"a\nb\n");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: Some(digest),
                    edits: vec![op("f.txt", &ha, None, "A")],
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "A\nb\n"
        );
    }

    fn tool() -> Edits {
        Edits::new(Arc::new(tokio::sync::Mutex::new(())))
    }

    fn op(path: &str, from: &str, to: Option<&str>, replacement: &str) -> EditOp {
        EditOp {
            path: path.into(),
            from: from.into(),
            to: to.map(String::from),
            replacement: replacement.into(),
            old_string: None,
            replace_all: None,
        }
    }

    #[tokio::test]
    async fn disjoint_batch_applies_atomically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\ne\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hd = anchor::anchor("d");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![
                        op("f.txt", &ha, None, "A\nA"),
                        op("f.txt", &hd, None, "D\nD"),
                    ],
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "A\nA\nb\nc\nD\nD\ne\n"
        );
        assert!(out.output.contains("op1 → lines 1"), "{}", out.output);
        assert!(out.output.contains("op2 → lines 4"), "{}", out.output);
    }

    #[tokio::test]
    async fn one_stale_op_aborts_the_whole_batch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![
                        op("f.txt", &ha, None, "A"),
                        op("f.txt", "zzz99", None, "B"), // valid anchor, never in file
                    ],
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_STALE_ANCHOR"), "{}", out.output);
        assert!(out.output.contains("op2"), "{}", out.output);
        // All-or-nothing: even op1's valid edit was NOT written.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a\nb\nc\n"
        );
    }

    #[tokio::test]
    async fn overlapping_ops_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x\ny\nz\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hx = anchor::anchor("x");
        let hz = anchor::anchor("z");
        let hy = anchor::anchor("y");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![
                        op("f.txt", &hx, Some(&hz), "W"),
                        op("f.txt", &hy, None, "V"), // inside op1's range
                    ],
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(
            out.output.contains("E_OVERLAPPING_RANGES"),
            "{}",
            out.output
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x\ny\nz\n"
        );
    }

    #[tokio::test]
    async fn replace_all_combines_with_other_ops() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "same\nsame\nx\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hs = anchor::anchor("same");
        let hx = anchor::anchor("x");
        let mut op1 = op("f.txt", &hs, None, "diff");
        op1.replace_all = Some(true);
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![op1, op("f.txt", &hx, None, "y")],
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "diff\ndiff\ny\n"
        );
    }

    #[tokio::test]
    async fn mixed_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "b\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hb = anchor::anchor("b");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![op("a.txt", &ha, None, "A"), op("b.txt", &hb, None, "B")],
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_MIXED_FILES"), "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "a\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
            "b\n"
        );
    }

    #[tokio::test]
    async fn equivalent_paths_are_not_mixed_files() {
        // `./f.txt` and `f.txt` resolve to different PathBufs but are the same
        // file; the batch must not be rejected as E_MIXED_FILES.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hb = anchor::anchor("b");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![op("f.txt", &ha, None, "A"), op("./f.txt", &hb, None, "B")],
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "A\nB\n"
        );
    }

    #[tokio::test]
    async fn applied_count_excludes_noop_ops() {
        // A batch with one real edit and one idempotent no-op must report the
        // ranges actually changed, not every range that resolved.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hb = anchor::anchor("b");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![
                        op("f.txt", &ha, None, "A"), // changes
                        op("f.txt", &hb, None, "b"), // already "b" -> no-op
                    ],
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(
            out.output.contains("1 range applied"),
            "count must exclude the no-op op: {}",
            out.output
        );
        assert!(out.output.contains("op1 → lines 1"), "{}", out.output);
        assert!(
            !out.output.contains("op2"),
            "a no-op op must not be listed: {}",
            out.output
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "A\nb\nc\n"
        );
    }

    #[tokio::test]
    async fn empty_batch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(
                EditsArgs {
                    edits: vec![],
                    expected_digest: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_EMPTY_BATCH"));
    }

    #[tokio::test]
    async fn ambiguous_op_names_the_offending_op() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x\n}\ny\n}\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hx = anchor::anchor("x");
        let h = anchor::anchor("}");
        let out = tool()
            .execute(
                EditsArgs {
                    expected_digest: None,
                    edits: vec![
                        op("f.txt", &hx, None, "A"),
                        op("f.txt", &h, None, "z"), // ambiguous: two `}` lines
                    ],
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_AMBIGUOUS_ANCHOR"), "{}", out.output);
        assert!(out.output.contains("op2"), "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x\n}\ny\n}\n"
        );
    }
}

