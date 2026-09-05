use std::time::Duration;

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BashArgs {
    /// Shell command, run as `sh -c` inside the working directory.
    pub command: String,
    /// Kill the command after this many seconds (default 30).
    pub timeout_secs: Option<u64>,
}

pub struct Bash;

#[async_trait::async_trait]
impl TypedTool for Bash {
    type Args = BashArgs;
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Run a shell command (`sh -c`) in the working directory. Returns stdout, labeled stderr and the exit code. Non-zero exit marks the result as an error."
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let timeout = Duration::from_secs(args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
        let mut child = match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .current_dir(&ctx.working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("sh spawn: {e}"),
                    is_error: true,
                    details: None,
                };
            }
        };
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        // Owned pipes read concurrently; `child` stays free for kill()/wait().
        let drained = async {
            let mut out = String::new();
            let mut err = String::new();
            let ((), ()) = futures::join!(
                async {
                    if let Some(mut p) = stdout_pipe {
                        use tokio::io::AsyncReadExt as _;
                        let _ = p.read_to_string(&mut out).await;
                    }
                },
                async {
                    if let Some(mut p) = stderr_pipe {
                        use tokio::io::AsyncReadExt as _;
                        let _ = p.read_to_string(&mut err).await;
                    }
                }
            );
            (out, err)
        };
        tokio::pin!(drained);
        // ponytail: no process-group kill; daemonized grandchildren hold pipe until timeout — setsid+killpg if it bites
        let (stdout, stderr) = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return ToolOutput {
                    output: "cancelled".to_string(),
                    is_error: true,
                    details: None,
                };
            }
            _ = tokio::time::sleep(timeout) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return ToolOutput {
                    output: format!("timed out after {}s (command killed)", timeout.as_secs()),
                    is_error: true,
                    details: None,
                };
            }
            drained_res = &mut drained => drained_res,
        };
        let status = match child.wait().await {
            Ok(s) => s,
            Err(e) => {
                return ToolOutput {
                    output: format!("bash wait: {e}"),
                    is_error: true,
                    details: None,
                };
            }
        };

        let mut parts: Vec<String> = Vec::new();
        let out = stdout.trim_end_matches('\n');
        if !out.is_empty() {
            parts.push(out.to_string());
        }
        let err = stderr.trim_end_matches('\n');
        if !err.is_empty() {
            parts.push(format!("[stderr]\n{err}"));
        }
        parts.push(format!(
            "exit code: {}",
            status
                .code()
                .map_or_else(|| "terminated by signal".to_string(), |c| c.to_string())
        ));
        ToolOutput {
            output: parts.join("\n"),
            is_error: status.code().map_or(true, |c| c != 0),
            details: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn echo_reports_exit_code_zero() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Bash
            .execute(
                BashArgs {
                    command: "echo hello".into(),
                    timeout_secs: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert!(out.output.contains("hello"));
        assert!(out.output.contains("exit code: 0"));
    }

    #[tokio::test]
    async fn non_zero_exit_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Bash
            .execute(
                BashArgs {
                    command: "echo oops >&2; exit 3".into(),
                    timeout_secs: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("[stderr]"));
        assert!(out.output.contains("oops"));
        assert!(out.output.contains("exit code: 3"));
    }

    #[tokio::test]
    async fn timeout_kills_command() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let started = Instant::now();
        let out = Bash
            .execute(
                BashArgs {
                    command: "sleep 5".into(),
                    timeout_secs: Some(1),
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("timed out after 1s"));
        assert!(started.elapsed() < Duration::from_secs(4), "kill must be prompt");
    }

    #[tokio::test]
    async fn cancel_token_kills_command() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        });
        let out = Bash
            .execute(
                BashArgs {
                    command: "sleep 5".into(),
                    timeout_secs: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert_eq!(out.output, "cancelled");
    }
}
