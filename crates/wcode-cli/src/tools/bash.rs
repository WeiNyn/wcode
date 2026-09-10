use std::time::Duration;

use serde::Deserialize;
use wcode_harness::event::AgentEvent;
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

/// Stream one output pipe line-by-line: each line is emitted as a live
/// `ToolExecutionUpdate` (so the UI shows partial output as it arrives) and
/// accumulated for the final `ToolOutput`. The `live_prefix` (`""` for stdout,
/// `"[stderr] "` for stderr) tags the live stream only — the final result
/// labels stderr once, in the standard `[stderr]\n…` block.
async fn drain_lines(
    reader: impl tokio::io::AsyncBufRead + Unpin,
    events: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    call_id: &str,
    name: &str,
    live_prefix: &str,
) -> String {
    use tokio::io::AsyncBufReadExt;
    let mut reader = reader;
    let mut buf = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF: pipe closed
            Ok(_) => {
                buf.push_str(&line);
                let _ = events.send(AgentEvent::ToolExecutionUpdate {
                    call_id: call_id.to_string(),
                    name: name.to_string(),
                    partial: format!("{live_prefix}{line}"),
                });
            }
            Err(_) => break,
        }
    }
    buf
}

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

        let events = ctx.events.clone();
        let call_id = ctx.call_id.clone();
        let name = ctx.name.clone();

        // Owned pipes read concurrently; `child` stays free for kill()/wait().
        // stdout/stderr are streamed line-by-line as ToolExecutionUpdate so
        // the UI renders output live; the same lines accumulate for the final
        // ToolOutput, so the LLM still sees the complete result.
        let drained = async {
            let (out, err) = futures::join!(
                async {
                    match stdout_pipe {
                        Some(pipe) => {
                            drain_lines(
                                tokio::io::BufReader::new(pipe),
                                &events,
                                &call_id,
                                &name,
                                "",
                            )
                            .await
                        }
                        None => String::new(),
                    }
                },
                async {
                    match stderr_pipe {
                        Some(pipe) => {
                            drain_lines(
                                tokio::io::BufReader::new(pipe),
                                &events,
                                &call_id,
                                &name,
                                "[stderr] ",
                            )
                            .await
                        }
                        None => String::new(),
                    }
                },
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
            is_error: status.code() != Some(0),
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
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "kill must be prompt"
        );
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

    #[tokio::test]
    async fn streams_partial_output_updates() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, mut rx) = super::super::test_ctx(dir.path());
        let out = Bash
            .execute(
                BashArgs {
                    command: "echo a; sleep 0.05; echo b".into(),
                    timeout_secs: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        // Lines stream as ToolExecutionUpdate in order, keeping their newline.
        let mut updates = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let wcode_harness::event::AgentEvent::ToolExecutionUpdate { partial, .. } = ev {
                updates.push(partial);
            }
        }
        assert_eq!(updates, vec!["a\n".to_string(), "b\n".to_string()]);
    }

    #[tokio::test]
    async fn streams_stderr_lines_with_live_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, mut rx) = super::super::test_ctx(dir.path());
        let out = Bash
            .execute(
                BashArgs {
                    command: "printf 'oops\\n' >&2".into(),
                    timeout_secs: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        let mut updates = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let wcode_harness::event::AgentEvent::ToolExecutionUpdate { partial, .. } = ev {
                updates.push(partial);
            }
        }
        assert_eq!(updates, vec!["[stderr] oops\n".to_string()]);
        // The final ToolOutput still labels stderr once in the standard block.
        assert!(out.output.contains("[stderr]"));
        assert!(out.output.contains("oops"));
    }
}
