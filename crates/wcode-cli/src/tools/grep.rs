use std::io::Read as _;

use regex::Regex;
use serde::Deserialize;
use ignore::{DirEntry, Walk, WalkBuilder};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor;

/// Directories never searched by default (deps/build noise).
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", "build", "dist", ".venv"];

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GrepArgs {
    /// Regex pattern to search for (line-based matching).
    pub pattern: String,
    /// File or directory to search; defaults to the working directory.
    pub path: Option<String>,
    /// Comma-separated include globs, e.g. "**/*.rs", "!target/**". Default:
    /// all text files except noise dirs (.git, target, node_modules, ...).
    pub glob: Option<String>,
    /// Lines of context to show before/after each match.
    pub context: Option<u64>,
    /// Case-insensitive matching.
    pub ignore_case: Option<bool>,
    /// Maximum number of result lines to report — matches plus the context
    /// lines around them (default 200).
    pub max: Option<u64>,
    /// Search files that `.gitignore` excludes too (default false).
    pub no_ignore: Option<bool>,
}

pub struct Grep;

#[async_trait::async_trait]
impl TypedTool for Grep {
    type Args = GrepArgs;
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Regex-search files. Every result line carries its `read`-style anchor so it can be targeted directly with `edit`. Output: `path:lineno  ANCHOR│content`. Match lines are marked ` <--`; context lines are unmarked. Respects .gitignore and skips .git/target/node_modules and binary files by default (pass no_ignore:true to search ignored files too). For AST-structural search use ast_search."
    }

    /// Read-only: safe to run alongside other calls in the same batch.
    fn parallel_safe(&self) -> bool {
        true
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let base = match &args.path {
            Some(p) => super::resolve(&ctx.working_dir, p),
            None => ctx.working_dir.clone(),
        };
        let re = match build_regex(&args.pattern, args.ignore_case.unwrap_or(false)) {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput {
                    output: format!("grep: invalid pattern: {e}"),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        };
        let (includes, excludes) = parse_globs(args.glob.as_deref());

        let mut out = String::new();
        let mut shown = 0usize;
        let max = args.max.unwrap_or(200) as usize;

        let mut files: Vec<std::path::PathBuf> = Vec::new();
        if base.is_file() {
            files.push(base.clone());
        } else {
            for entry in walker(&base, args.no_ignore.unwrap_or(false)) {
                // Cancel-checked per entry: discovery can be long on a big
                // tree, and the loop honors cancel only after the tool returns.
                if ctx.cancel.is_cancelled() {
                    return ToolOutput {
                        output: "cancelled".to_string(),
                        is_error: true,
                        diff: None,
                        path: None,
                    };
                }
                if let Ok(entry) = entry
                    && entry.file_type().is_some_and(|t| t.is_file())
                {
                    files.push(entry.into_path());
                }
            }
        }

        for file in files {
            // Reading + regexing every matched file can also be long; stop the
            // scan promptly when a cancel lands mid-run.
            if ctx.cancel.is_cancelled() {
                return ToolOutput {
                    output: "cancelled".to_string(),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
            let rel = file.strip_prefix(&ctx.working_dir).unwrap_or(&file);
            let rel_str = rel.to_string_lossy();
            if !include_path(&rel_str, &includes, &excludes) {
                continue;
            }
            let Ok(mut f) = std::fs::File::open(&file) else {
                continue;
            };
            let mut data = Vec::new();
            if f.read_to_end(&mut data).is_err() {
                continue;
            }
            if data.contains(&0) {
                continue; // binary
            }
            let Ok(content) = String::from_utf8(data) else {
                continue;
            };
            let lines = anchor::split_lines(&content);
            let ctx_n = args.context.unwrap_or(0) as usize;
            let match_idx: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| re.is_match(l))
                .map(|(i, _)| i)
                .collect();
            if match_idx.is_empty() {
                continue;
            }
            // Emit merged context windows around matches, in file order.
            let mut emit = std::collections::BTreeSet::new();
            for &m in &match_idx {
                for i in m.saturating_sub(ctx_n)..=m.saturating_add(ctx_n).min(lines.len() - 1) {
                    emit.insert(i);
                }
            }
            for i in emit {
                if shown >= max {
                    let unit = if max == 1 { "line" } else { "lines" };
                    out.push_str(&format!("[grep: truncated at {max} {unit}]\n"));
                    return ToolOutput {
                        output: out,
                        is_error: false,
                        diff: None,
                        path: None,
                    };
                }
                let line = &lines[i];
                let h = anchor::anchor(line);
                let marker = if match_idx.contains(&i) { "  <--" } else { "" };
                out.push_str(&format!(
                    "{}:{}  {}{}{}{}\n",
                    rel_str,
                    i + 1,
                    h,
                    anchor::ANCHOR_SEP,
                    line,
                    marker
                ));
                shown += 1;
            }
        }
        if out.is_empty() {
            out.push_str(&format!(
                "grep: no matches for /{}/ in {}\n",
                args.pattern,
                base.display()
            ));
        }
        ToolOutput {
            output: out,
            is_error: false,
            diff: None,
            path: None,
        }
    }
}

fn build_regex(pattern: &str, ignore_case: bool) -> Result<Regex, regex::Error> {
    let mut b = regex::RegexBuilder::new(pattern);
    b.case_insensitive(ignore_case);
    b.unicode(true);
    b.build()
}

fn is_skipped_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_some_and(|t| t.is_dir())
        && SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref())
}

/// A `.gitignore`-aware walker over `base`.
///
/// Hidden files are still searched — only the *ignore rules* change, so this is
/// strictly narrower than the old `walkdir` walk. [`SKIP_DIRS`] stays as a
/// built-in floor, so `target/`/`node_modules/` are skipped even when a repo
/// does not ignore them. `require_git(false)` applies `.gitignore` outside a git
/// repo too (pi makes the same call). `no_ignore` is the model's escape hatch:
/// it turns every ignore source off.
pub(crate) fn walker(base: &std::path::Path, no_ignore: bool) -> Walk {
    let mut b = WalkBuilder::new(base);
    b.hidden(false).parents(true).require_git(false);
    if no_ignore {
        b.git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .ignore(false);
    }
    b.filter_entry(|e| !is_skipped_dir(e));
    b.build()
}

pub(crate) fn parse_globs(
    glob: Option<&str>,
) -> (Vec<globset::GlobMatcher>, Vec<globset::GlobMatcher>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    if let Some(g) = glob {
        for pat in g.split(',') {
            let pat = pat.trim();
            if pat.is_empty() {
                continue;
            }
            if let Some(ex) = pat.strip_prefix('!') {
                if let Ok(gm) = globset::Glob::new(ex).map(|g| g.compile_matcher()) {
                    excludes.push(gm);
                }
            } else if let Ok(gm) = globset::Glob::new(pat).map(|g| g.compile_matcher()) {
                includes.push(gm);
            }
        }
    }
    (includes, excludes)
}

fn include_path(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn greps_with_anchors_and_context() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn a() {}\nfn target() {}\nfn c() {}\n",
        )
        .unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Grep
            .execute(
                GrepArgs {
                    pattern: "target".into(),
                    path: None,
                    glob: None,
                    context: Some(1),
                    ignore_case: None,
                    max: None,
                    no_ignore: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        let lines: Vec<&str> = out.output.lines().collect();
        assert!(
            lines
                .clone()
                .into_iter()
                .any(|l| l.contains("fn target() {}") && l.contains("<--"))
        );
        assert!(
            lines
                .into_iter()
                .any(|l| l.contains("fn a() {}") && !l.contains("<--"))
        );
    }

    #[tokio::test]
    async fn respoects_glob_excludes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/x.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("ok.txt"), "needle\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Grep
            .execute(
                GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    glob: None,
                    context: None,
                    ignore_case: None,
                    max: None,
                    no_ignore: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert!(out.output.contains("ok.txt"));
        assert!(!out.output.contains("target"));
    }

    #[tokio::test]
    async fn max_caps_result_lines_including_context() {
        // `max` bounds the emitted result lines (matches + context), not just
        // the matches: with context:1 and max:1 the first emitted line is the
        // context line above the match, then the scan truncates.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nneedle\nb\nneedle\nc\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Grep
            .execute(
                GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    glob: None,
                    context: Some(1),
                    ignore_case: None,
                    max: Some(1),
                    no_ignore: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(
            out.output.contains("[grep: truncated at 1 line]"),
            "{}",
            out.output
        );
        // one result line (the context line above the first match) + the note
        assert_eq!(out.output.lines().count(), 2, "{}", out.output);
    }

    fn put(dir: &std::path::Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// Run grep for the literal `needle` over `dir`.
    async fn needle(dir: &std::path::Path, no_ignore: Option<bool>) -> String {
        let (ctx, _rx) = super::super::test_ctx(dir);
        Grep
            .execute(
                GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    glob: None,
                    context: None,
                    ignore_case: None,
                    max: None,
                    no_ignore,
                },
                &ctx,
            )
            .await
            .output
    }

    /// `.gitignore` applies even though a `tempfile` dir is not a git repo —
    /// `require_git(false)`, matching pi.
    #[tokio::test]
    async fn respects_gitignore_without_a_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), ".gitignore", "ignored.txt\n");
        put(dir.path(), "ignored.txt", "needle\n");
        put(dir.path(), "kept.txt", "needle\n");

        let out = needle(dir.path(), None).await;
        assert!(out.contains("kept.txt"), "{out}");
        assert!(!out.contains("ignored.txt"), "gitignore not honored: {out}");

        // The escape hatch reaches ignored files again.
        let out = needle(dir.path(), Some(true)).await;
        assert!(out.contains("ignored.txt"), "no_ignore did nothing: {out}");
    }

    #[tokio::test]
    async fn gitignore_negation_re_includes_a_file() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), ".gitignore", "*.log\n!keep.log\n");
        put(dir.path(), "drop.log", "needle\n");
        put(dir.path(), "keep.log", "needle\n");

        let out = needle(dir.path(), None).await;
        assert!(out.contains("keep.log"), "negation ignored: {out}");
        assert!(!out.contains("drop.log"), "{out}");
    }

    #[tokio::test]
    async fn gitignore_from_a_parent_directory_applies() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), ".gitignore", "skip/\n");
        put(dir.path(), "sub/skip/x.txt", "needle\n");
        put(dir.path(), "sub/ok.txt", "needle\n");

        // Searching the subdir still honors the parent's `.gitignore`.
        let out = needle(&dir.path().join("sub"), None).await;
        assert!(out.contains("ok.txt"), "{out}");
        assert!(!out.contains("x.txt"), "parent gitignore not applied: {out}");
    }

    /// The built-in noise floor still applies when a repo ignores nothing.
    #[tokio::test]
    async fn builtin_skip_dirs_apply_without_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "target/x.txt", "needle\n");
        put(dir.path(), "ok.txt", "needle\n");

        let out = needle(dir.path(), Some(true)).await;
        assert!(out.contains("ok.txt"), "{out}");
        assert!(!out.contains("target"), "SKIP_DIRS floor lost: {out}");
    }
}
