use std::io::Read as _;

use regex::Regex;
use serde::Deserialize;
use walkdir::WalkDir;
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
    /// Maximum number of match lines to report.
    pub max: Option<u64>,
}

pub struct Grep;

#[async_trait::async_trait]
impl TypedTool for Grep {
    type Args = GrepArgs;
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Regex-search files. Every result line carries its `read`-style anchor so it can be targeted directly with `edit`. Output: `path:lineno  ANCHOR│content`. Match lines are marked ` <--`; context lines are unmarked. Skips .git/target/node_modules and binary files by default. For AST-structural search use ast_search."
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
                    details: None,
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
            for entry in WalkDir::new(&base)
                .into_iter()
                .filter_entry(|e| !is_skipped_dir(e))
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() {
                    files.push(entry.into_path());
                }
            }
        }

        for file in files {
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
                    out.push_str(&format!("[grep: truncated at {max} matches]\n"));
                    return ToolOutput {
                        output: out,
                        is_error: false,
                        details: None,
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
            details: None,
        }
    }
}

fn build_regex(pattern: &str, ignore_case: bool) -> Result<Regex, regex::Error> {
    let mut b = regex::RegexBuilder::new(pattern);
    b.case_insensitive(ignore_case);
    b.unicode(true);
    b.build()
}

pub(crate) fn is_skipped_dir(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir() && SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref())
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
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert!(out.output.contains("ok.txt"));
        assert!(!out.output.contains("target"));
    }
}
