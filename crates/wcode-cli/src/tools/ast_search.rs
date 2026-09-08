//! Optional AST-structural search backed by an ast-grep binary (`ast-grep` or
//! its legacy `sg` alias; discovery lives in [`super::ast`]).
//!
//! Mirrors wcode's existing "auto if binary on PATH" pattern (rtk): the tool
//! is only registered when one of the binaries is present, so the model never
//! sees a tool it can't use.

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AstSearchArgs {
    /// Structural pattern; `$UPPERCASE` are AST wildcards. Example:
    /// `if $COND { $BODY }` matches any if-statement regardless of condition
    /// text. Write the pattern as ordinary code.
    pub pattern: String,
    /// File or directory to search; defaults to the working directory.
    pub path: Option<String>,
    /// Force the language (e.g. "rust", "ts", "py"). Auto-detected when omitted.
    pub lang: Option<String>,
    /// Comma-separated include globs, e.g. "**/*.rs", "!target/**".
    pub glob: Option<String>,
}

pub struct AstSearch;

/// Whether either ast-grep binary is on PATH. If false, the tool is simply
/// not registered (see tools::default_tools).
pub fn available() -> bool {
    super::ast::available()
}

#[async_trait::async_trait]
impl TypedTool for AstSearch {
    type Args = AstSearchArgs;
    fn name(&self) -> &str {
        "ast_search"
    }
    fn description(&self) -> &str {
        "AST-structural code search via ast-grep (`sg`). Pattern is ordinary code with `$UPPERCASE` as wildcards, e.g. `await $X` finds every await regardless of operand. Use it when regex grep cannot express the structure; regex grep is cheaper for text matches. Currently search-only — structural rewrite is a follow-up."
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let Some(bin) = super::ast::find_bin() else {
            return ToolOutput {
                output:
                    "ast_search: no ast-grep binary on PATH (tool should not have been registered)"
                        .into(),
                is_error: true,
                details: None,
            };
        };
        let mut cmd = tokio::process::Command::new(bin);
        cmd.arg("-p").arg(&args.pattern);
        if let Some(lang) = &args.lang {
            cmd.arg("--lang").arg(lang);
        }
        let mut target = args.path.clone().unwrap_or_default();
        if let Some(glob) = &args.glob {
            for pat in glob.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                if let Some(ex) = pat.strip_prefix('!') {
                    cmd.arg("--ignore").arg(ex);
                } else if target.is_empty() {
                    target = pat.to_string();
                }
            }
        }
        if !target.is_empty() {
            cmd.arg(&target).current_dir(&ctx.working_dir);
        } else {
            cmd.current_dir(&ctx.working_dir);
        }
        // Machine-readable output: no ANSI, no color.
        cmd.env("NO_COLOR", "1");

        let out = match cmd.output().await {
            Ok(o) => o,
            Err(e) => {
                return ToolOutput {
                    output: format!("ast_search: failed to run `{bin}`: {e}"),
                    is_error: true,
                    details: None,
                };
            }
        };
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stderr = super::ast::clean_stderr(&stderr);
        if !out.status.success() {
            return ToolOutput {
                output: format!(
                    "ast_search: `{bin}` exited {}:\n{}\n{}",
                    out.status,
                    stdout,
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!("[stderr]\n{stderr}")
                    }
                ),
                is_error: true,
                details: None,
            };
        }
        let mut text = stdout;
        if !stderr.is_empty() {
            text.push_str(&format!("\n[stderr]\n{stderr}"));
        }
        ToolOutput {
            output: if text.is_empty() {
                "ast_search: no structural matches".to_string()
            } else {
                text
            },
            is_error: false,
            details: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_detects_binary_on_path() {
        // Tool presence is PATH-dependent; just ensure it doesn't panic and
        // returns a bool.
        let _ = available();
    }
}
