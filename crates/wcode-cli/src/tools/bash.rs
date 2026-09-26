use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use wcode_harness::event::AgentEvent;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::background::{self, Background, State};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BashArgs {
    /// Shell command, run as `sh -c` inside the working directory.
    pub command: String,
    /// Kill the command after this many seconds (default 30).
    pub timeout_secs: Option<u64>,
    /// Run the command in the background and return a task handle (`bg3`)
    /// instead of waiting. `timeout_secs` is IGNORED when this is `true` —
    /// the task runs until it exits or `bg kill` (D12).
    pub background: Option<bool>,
}

pub struct Bash {
    bg: Arc<Background>,
}

impl Bash {
    pub fn new(bg: Arc<Background>) -> Self {
        Self { bg }
    }

    /// `bash { background: true }` — spawn the command in its own process group
    /// and hand the un-reaped child to a supervisor, returning a `bg<N>` handle
    /// at once (D3). `timeout_secs` is deliberately NOT read here (D12).
    async fn spawn_background(&self, args: BashArgs, ctx: &ToolContext) -> ToolOutput {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(&args.command)
            .current_dir(&ctx.working_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        background::spawn_group(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("sh spawn: {e}"),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        };
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        // Register (state Running) and hand the CHILD + kill_rx to the supervisor.
        let (id, kill_rx) = self.bg.register(&args.command, &ctx.working_dir, pid);
        background::spawn_supervisor(self.bg.clone(), id.clone(), child, stdout, stderr, kill_rx);
        // The task may already have finished (`true`): the supervisor flips the
        // state under the lock, so this peek is racy-but-safe — either a handle
        // line or an immediate terminal line, never a wrong state.
        match self.bg.peek_state(&id) {
            Some(State::Running) => ToolOutput {
                output: format!("started {id}: {}", args.command),
                ..ToolOutput::default()
            },
            Some(state) => {
                let preview = self
                    .bg
                    .tail_of(&id, 10)
                    .map(|(text, _)| text)
                    .unwrap_or_default();
                let suffix = if preview.is_empty() {
                    String::new()
                } else {
                    format!(": {preview}")
                };
                ToolOutput {
                    output: format!("{id} {}{suffix}", state.label()),
                    ..ToolOutput::default()
                }
            }
            None => ToolOutput {
                output: format!("{id} vanished"),
                is_error: true,
                ..ToolOutput::default()
            },
        }
    }
}

/// Buffered bytes above this spill to a file instead of growing unbounded.
pub(crate) const MAX_INLINE_BYTES: usize = 24_000;
/// Chars of a spilled stream kept inline — the first and the last, so both the
/// command's opening output and its error tail stay visible in the result.
pub(crate) const PREVIEW_HEAD_CHARS: usize = 12_000;
pub(crate) const PREVIEW_TAIL_CHARS: usize = 6_000;
/// Live `ToolExecutionUpdate` lines per stream before we stop streaming (a
/// flooding command shouldn't spam the UI; the file still holds everything).
const LIVE_LINE_CAP: usize = 200;
/// Bytes per pseudo-line when a no-newline stream overflows the line buffer —
/// a legit JSON blob fits; a megabyte of log noise is chunked. See
/// [`drain_lines`].
pub(crate) const MAX_LINE_BYTES: usize = 64_000;

/// Per-process counter so repeated/concurrent bash calls get distinct spill
/// files even within the same millisecond.
static SPILL_SEQ: AtomicU64 = AtomicU64::new(0);

/// Directory for spilled output — under the OS temp dir, so it is absolute
/// (the `read` tool resolves absolute paths) and the OS reaps it eventually.
pub(crate) fn spill_root() -> PathBuf {
    std::env::temp_dir().join("wcode")
}

/// The age past which a spill file is presumed abandoned ([`sweep_stale_spills`])
/// — a younger file may belong to a live concurrent session mid-write.
const STALE_SPILL_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Remove stale `bash-*.log` spill files under `root`, best-effort. Only
/// entries whose name matches the spill pattern (`bash-<pid>-<seq>-<stream>.log`)
/// are touched — anything else that landed in the directory is left alone — and
/// a file younger than [`STALE_SPILL_AGE`] is kept (the idle margin is the
/// safety against a live session). A missing directory is fine; the return
/// counts what was removed and errors are meant to be ignored by the caller.
pub(crate) fn sweep_stale_spills(root: &Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !(name.starts_with("bash-") && name.ends_with(".log")) {
            continue;
        }
        // Best-effort per entry: an unreadable or unterminated (future-mtime)
        // file is left alone rather than guessed about.
        let Ok(metadata) = entry.metadata() else { continue };
        let Ok(modified) = metadata.modified() else { continue };
        if now.duration_since(modified).is_ok_and(|age| age > STALE_SPILL_AGE)
            && entry.path().is_file()
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// A bounded preview of one stream with a lazy spill-to-file. Holds at most
/// `MAX_INLINE_BYTES` before spilling and `PREVIEW_HEAD_CHARS` +
/// `PREVIEW_TAIL_CHARS` after — never the whole stream. The full output is
/// written to `spill_path` (created only once the inline budget is exceeded),
/// so the model can `read` the rest after seeing the notice.
struct Capture {
    stream: &'static str,
    spill_path: PathBuf,
    /// Accumulated text before the spill threshold; emptied once spilled.
    buf: String,
    /// First `PREVIEW_HEAD_CHARS` chars, frozen when the spill begins.
    head: String,
    /// Rolling last `PREVIEW_TAIL_CHARS` chars, maintained while spilling.
    tail: String,
    /// Open spill file; `None` until the output outgrows the inline budget.
    file: Option<std::io::BufWriter<std::fs::File>>,
    /// Total bytes seen on the stream (spilled or not).
    total: u64,
}

impl Capture {
    fn new(stream: &'static str, dir: &Path, seq: u64) -> Self {
        let spill_path = dir.join(format!("bash-{}-{seq}-{stream}.log", std::process::id()));
        Self {
            stream,
            spill_path,
            buf: String::new(),
            head: String::new(),
            tail: String::new(),
            file: None,
            total: 0,
        }
    }

    /// Feed one decoded line: accumulate while small, otherwise write through
    /// to the spill file. `raw_len` is the line's byte length before any lossy
    /// UTF-8 decoding, for an accurate total.
    fn push(&mut self, text: &str, raw_len: usize) {
        self.total += raw_len as u64;
        match self.file.as_mut() {
            Some(file) => {
                let _ = file.write_all(text.as_bytes());
                keep_tail(&mut self.tail, text, PREVIEW_TAIL_CHARS);
            }
            None => {
                self.buf.push_str(text);
                if self.buf.len() > MAX_INLINE_BYTES {
                    self.spill();
                }
            }
        }
    }

    /// Open the spill file and flush what's buffered. If the file can't be
    /// opened we keep buffering (no data loss, just unbounded memory).
    fn spill(&mut self) {
        if let Some(parent) = self.spill_path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let Ok(file) = std::fs::File::create(&self.spill_path) else {
            return;
        };
        let mut file = std::io::BufWriter::new(file);
        let _ = file.write_all(self.buf.as_bytes());
        self.head = head_chars(&self.buf, PREVIEW_HEAD_CHARS);
        self.tail = tail_chars(&self.buf, PREVIEW_TAIL_CHARS);
        self.buf.clear();
        self.buf.shrink_to_fit();
        self.file = Some(file);
    }

    /// The model-visible block for this stream: the whole text when it never
    /// spilled, else a head+tail preview with a notice carrying the file path.
    fn finish(mut self) -> String {
        if self.file.is_none() {
            return self.buf;
        }
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
        format!(
            "{head}\n… [bash: {stream} output is {total} bytes; middle elided; \
             full output at {path}] …\n{tail}",
            head = self.head,
            stream = self.stream,
            total = self.total,
            path = self.spill_path.display(),
            tail = self.tail,
        )
    }
}

/// First `max` chars of `s`.
fn head_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Last `max` chars of `s`.
pub(crate) fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(max)).collect()
}

/// Append `text` to `dst`, trimming from the front to keep at most `max` chars.
fn keep_tail(dst: &mut String, text: &str, max: usize) {
    dst.push_str(text);
    if dst.len() > max {
        let mut idx = dst.len() - max;
        while idx < dst.len() && !dst.is_char_boundary(idx) {
            idx += 1;
        }
        dst.drain(..idx);
    }
}

/// Identity and labels for one streamed pipe, so `drain_lines` keeps a small
/// signature.
struct StreamLabel<'a> {
    call_id: &'a str,
    name: &'a str,
    live_prefix: &'a str,
    stream: &'static str,
}

/// Stream one output pipe line-by-line: each line is emitted as a live
/// `ToolExecutionUpdate` (so the UI shows partial output as it arrives) and
/// accumulated for the final `ToolOutput`. The `live_prefix` (`""` for stdout,
/// `"[stderr] "` for stderr) tags the live stream only — the final result
/// labels stderr once, in the standard `[stderr]\n…` block.
async fn drain_lines(
    reader: impl tokio::io::AsyncBufRead + Unpin,
    events: Option<&tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    label: StreamLabel<'_>,
    spill_dir: &Path,
    seq: u64,
) -> String {
    use tokio::io::AsyncBufReadExt;
    let mut reader = reader;
    let mut capture = Capture::new(label.stream, spill_dir, seq);
    let mut live = 0usize;
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) => break, // EOF: pipe closed
            Ok(_) => {
                // Chunk a pathological no-newline stream: `read_until` would
                // otherwise grow `line` unboundedly and bypass both the spill
                // threshold and the live cap (they fire only per full line).
                // A `from_utf8_lossy` split mid-codepoint renders a
                // replacement char — the whole path is already lossy, and no
                // data is dropped (the file still holds every byte).
                let mut emit = |text: &str, raw_len: usize| {
                    if live < LIVE_LINE_CAP
                        && let Some(events) = events
                    {
                        let _ = events.send(AgentEvent::ToolExecutionUpdate {
                            call_id: label.call_id.to_string(),
                            name: label.name.to_string(),
                            partial: format!("{}{text}", label.live_prefix),
                        });
                        live += 1;
                    }
                    capture.push(text, raw_len);
                };
                let mut start = 0;
                while line.len().saturating_sub(start) > MAX_LINE_BYTES {
                    let end = start + MAX_LINE_BYTES;
                    let text = String::from_utf8_lossy(&line[start..end]);
                    emit(&text, end - start);
                    start = end;
                }
                if start < line.len() {
                    let text = String::from_utf8_lossy(&line[start..]);
                    emit(&text, line.len() - start);
                }
            }
            Err(_) => break,
        }
    }
    capture.finish()
}

/// Kill a spawned shell *and its descendants*.
///
/// The child is spawned with `process_group(0)`, so its pid is also the pgid;
/// signalling the negative pid targets the whole group — pipelines and
/// backgrounded jobs included. Off unix, or once its pid is gone, this degrades
/// to killing just the child.
async fn kill_group(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: a negative pid signals a process group; a failure (group
        // already gone) is ignored, as is the redundant `child.kill` below.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// Run `command` via `sh -c` in `working_dir`, killing the whole process group
/// on `timeout` or when `cancel` fires. Returns `(exit_code, output)`: `exit_code`
/// is `None` when the process was killed (signal/timeout/cancel) or failed to
/// spawn; `output` is the SAME assembled block `execute` returns (trimmed stdout,
/// a `[stderr]\n…` section, a trailing `exit code: …` line) with identical
/// spill/limit behaviour. `events` (when `Some`) streams `ToolExecutionUpdate`s
/// for the live UI; `label` is the `call_id` (the event `name` is the literal
/// "bash"). The scheduler passes `None`. `cancel` kills the group and returns
/// here cleanly, so the tool finishes on its own — the loop's race is only a
/// backstop for a tool that ignores the token.
pub(crate) async fn run_command(
    command: &str,
    working_dir: &Path,
    timeout: Duration,
    cancel: &tokio_util::sync::CancellationToken,
    events: Option<&tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    label: &str,
) -> (Option<i32>, String) {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Own process group so a timeout/cancel can kill the shell *and* its
    // descendants (pipelines, backgrounded jobs), not just `sh` itself.
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (None, format!("sh spawn: {e}")),
    };
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let root = spill_root();
    let seq = SPILL_SEQ.fetch_add(1, Ordering::Relaxed);
    let drained = async {
        let (out, err) = futures::join!(
            async {
                match stdout_pipe {
                    Some(pipe) => {
                        drain_lines(
                            tokio::io::BufReader::new(pipe),
                            events,
                            StreamLabel {
                                call_id: label,
                                name: "bash",
                                live_prefix: "",
                                stream: "stdout",
                            },
                            &root,
                            seq,
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
                            events,
                            StreamLabel {
                                call_id: label,
                                name: "bash",
                                live_prefix: "[stderr] ",
                                stream: "stderr",
                            },
                            &root,
                            seq,
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
    // Timeout/cancel kills the whole process group (see kill_group), so a
    // daemonized grandchild can't survive and hold the pipes open.
    let (stdout, stderr) = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            kill_group(&mut child).await;
            return (None, "cancelled".to_string());
        }
        _ = tokio::time::sleep(timeout) => {
            kill_group(&mut child).await;
            return (None, format!("timed out after {}s (command killed)", timeout.as_secs()));
        }
        drained_res = &mut drained => drained_res,
    };
    let status = match child.wait().await {
        Ok(s) => s,
        Err(e) => return (None, format!("bash wait: {e}")),
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
    (status.code(), parts.join("\n"))
}
#[async_trait::async_trait]
impl TypedTool for Bash {
    type Args = BashArgs;
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Run a shell command (`sh -c`) in the working directory. Returns stdout, labeled stderr and the exit code. Non-zero exit marks the result as an error. Output beyond ~24K is elided inline and the full output is saved to a file whose path is shown — read it (offset/limit, or from) to page the rest. Set `background: true` to run it as a background task and get a `bg<N>` handle; `timeout_secs` is then ignored and the task runs until it exits or you kill it with the `bg` tool."
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        if args.background == Some(true) {
            return self.spawn_background(args, ctx).await;
        }
        let timeout = Duration::from_secs(args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
        let (code, output) = run_command(
            &args.command,
            &ctx.working_dir,
            timeout,
            &ctx.cancel,
            Some(&ctx.events),
            &ctx.call_id,
        )
        .await;
        ToolOutput {
            output,
            is_error: code != Some(0),
            ..ToolOutput::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use std::time::SystemTime;

    /// Rewind (or pin) a file's mtime, mimicking a file written long ago.
    fn set_mtime(path: &Path, when: SystemTime) {
        let file = std::fs::File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(when))
            .unwrap();
    }

    #[test]
    fn sweep_stale_spills_removes_old_spills_and_keeps_fresh_and_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let old = SystemTime::now() - Duration::from_secs(48 * 60 * 60); // 2 days
        let fresh = SystemTime::now();

        // Old spill files are removed regardless of which stream/seq.
        set_mtime(&root.join("bash-111-0-stdout.log"), old);
        set_mtime(&root.join("bash-111-0-stderr.log"), old);
        // A fresh spill file is kept — a live session may still be writing it.
        set_mtime(&root.join("bash-222-5-stdout.log"), fresh);
        // A non-matching name is never touched, even when ancient.
        set_mtime(&root.join("notes.txt"), old);

        let removed = sweep_stale_spills(root).unwrap();
        assert_eq!(removed, 2, "only the two old bash-*.log files");
        assert!(!root.join("bash-111-0-stdout.log").exists());
        assert!(!root.join("bash-111-0-stderr.log").exists());
        assert!(root.join("bash-222-5-stdout.log").exists(), "fresh kept");
        assert!(root.join("notes.txt").exists(), "foreign file kept");

        // A second sweep finds nothing.
        assert_eq!(sweep_stale_spills(root).unwrap(), 0);
    }

    #[test]
    fn sweep_stale_spills_handles_a_missing_directory_and_entries() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing there yet: not an error.
        let missing = dir.path().join("no-such-dir");
        assert_eq!(sweep_stale_spills(&missing).unwrap(), 0);

        // A subdirectory named like a spill file is left alone (not a file).
        std::fs::create_dir_all(dir.path().join("bash-333-0-stdout.log")).unwrap();
        assert_eq!(sweep_stale_spills(dir.path()).unwrap(), 0);
        assert!(
            dir.path().join("bash-333-0-stdout.log").is_dir(),
            "a directory is not a spill file to remove"
        );
    }

    #[tokio::test]
    async fn echo_reports_exit_code_zero() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "echo hello".into(),
                    timeout_secs: None,
                    background: None,
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
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "echo oops >&2; exit 3".into(),
                    timeout_secs: None,
                    background: None,
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
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "sleep 5".into(),
                    timeout_secs: Some(1),
                    background: None,
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
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "sleep 5".into(),
                    timeout_secs: None,
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert_eq!(out.output, "cancelled");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_whole_process_group() {
        // A backgrounded grandchild must die with the shell, not outlive the
        // timeout still holding the pipes open.
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("bg.pid");
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: format!("sleep 30 & echo $! > '{}'; wait", pidfile.display()),
                    timeout_secs: Some(1),
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("timed out after 1s"), "{}", out.output);

        let pid: i32 = std::fs::read_to_string(&pidfile)
            .expect("background pid was written")
            .trim()
            .parse()
            .expect("pid parses");
        let mut gone = false;
        for _ in 0..100 {
            // A signal of 0 probes for existence; ESRCH means the process is gone.
            let rc = unsafe { libc::kill(pid, 0) };
            if rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            gone,
            "background grandchild (pid {pid}) survived the timeout"
        );
    }

    #[tokio::test]
    async fn streams_partial_output_updates() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, mut rx) = super::super::test_ctx(dir.path());
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "echo a; sleep 0.05; echo b".into(),
                    timeout_secs: None,
                    background: None,
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
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "printf 'oops\\n' >&2".into(),
                    timeout_secs: None,
                    background: None,
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

    #[tokio::test]
    async fn small_output_stays_inline_without_spilling() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "echo hello".into(),
                    timeout_secs: None,
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert!(out.output.contains("hello"));
        assert!(
            !out.output.contains("full output at"),
            "small output must not spill: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn large_output_spills_and_points_at_the_full_file() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        // ~100 KiB, well over MAX_INLINE_BYTES.
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "seq 1 20000".into(),
                    timeout_secs: None,
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        // The inline result stays bounded (head + notice + tail).
        assert!(
            out.output.len() < MAX_INLINE_BYTES + 2000,
            "inline output bounded: {} bytes",
            out.output.len()
        );
        // The notice carries the spill path...
        let path = out
            .output
            .split("full output at ")
            .nth(1)
            .expect("spill notice")
            .split(']')
            .next()
            .unwrap()
            .to_string();
        assert!(path.ends_with("-stdout.log"), "stdout spill path: {path}");
        // ...and the file holds the whole stream, start to finish.
        let full = std::fs::read_to_string(&path).expect("spill file readable");
        assert!(full.starts_with("1\n"), "starts at the head");
        assert!(full.trim_end().ends_with("20000"), "ends at the tail");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn stderr_spills_independently_of_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "seq 1 20000 >&2".into(),
                    timeout_secs: None,
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert!(out.output.contains("[stderr]"));
        let path = out
            .output
            .split("full output at ")
            .nth(1)
            .expect("stderr spill notice")
            .split(']')
            .next()
            .unwrap()
            .to_string();
        assert!(path.ends_with("-stderr.log"), "stderr spill path: {path}");
        let full = std::fs::read_to_string(&path).expect("spill file readable");
        assert!(full.contains("20000"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn live_updates_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, mut rx) = super::super::test_ctx(dir.path());
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "seq 1 1000".into(),
                    timeout_secs: None,
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        let mut updates = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AgentEvent::ToolExecutionUpdate { .. }) {
                updates += 1;
            }
        }
        assert_eq!(updates, LIVE_LINE_CAP, "live stream is capped");
    }

    #[tokio::test]
    async fn a_no_newline_stream_is_chunked_and_never_grows_unboundedly() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, mut rx) = super::super::test_ctx(dir.path());
        // 200 KiB of 'a' with no newline: `read_until` would otherwise grow one
        // line to the full 200 KiB; the chunker turns it into MAX_LINE_BYTES
        // pseudo-lines so the live updates and the memory stay bounded.
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "dd if=/dev/zero bs=200000 count=1 2>/dev/null | tr '\\0' 'a'"
                        .into(),
                    timeout_secs: None,
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);

        let mut updates = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let AgentEvent::ToolExecutionUpdate { partial, .. } = ev {
                updates.push(partial);
            }
        }
        let full_chunks = 200_000 / MAX_LINE_BYTES;
        let tail = 200_000 % MAX_LINE_BYTES;
        assert!(tail > 0, "200_000 bytes must span more than whole chunks");
        assert_eq!(updates.len(), full_chunks + 1, "one pseudo-line per chunk");
        for update in &updates[..full_chunks] {
            assert_eq!(update.chars().count(), MAX_LINE_BYTES, "a chunk is capped");
        }
        assert_eq!(updates.last().unwrap().chars().count(), tail);

        // Content is preserved across the chunk boundaries (lossy, but here
        // pure ASCII, so byte-identical).
        let joined: String = updates.concat();
        assert!(joined.chars().all(|c| c == 'a'));
        assert_eq!(joined.chars().count(), 200_000);

        // The spill file still holds the whole stream, byte for byte.
        let path = out
            .output
            .split("full output at ")
            .nth(1)
            .expect("spill notice")
            .split(']')
            .next()
            .unwrap()
            .to_string();
        let full = std::fs::read(&path).expect("spill file readable");
        assert_eq!(full.len(), 200_000, "no byte is dropped from the file");
        let _ = std::fs::remove_file(&path);
    }

    /// `run_command` is the extracted seam: it returns the exit code and the
    /// same assembled output block `execute` produces.
    #[tokio::test]
    async fn run_command_returns_exit_code_and_output() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();

        let (code, out) = run_command(
            "echo ok",
            dir.path(),
            Duration::from_secs(30),
            &cancel,
            None,
            "t",
        )
        .await;
        assert_eq!(code, Some(0));
        assert!(out.contains("ok") && out.contains("exit code: 0"), "{out}");

        let (code, out) = run_command(
            "exit 3",
            dir.path(),
            Duration::from_secs(30),
            &cancel,
            None,
            "t",
        )
        .await;
        assert_eq!(code, Some(3));
        assert!(out.contains("exit code: 3"), "{out}");
    }

    /// The child's stdin is null, never the terminal's: under the raw-mode TUI
    /// an inherited stdin lets an interactive child (a pager, `read`, a bare
    /// `cat`) swallow the UI's keystrokes — ESC/Ctrl-C included. A `read` must
    /// instead see EOF at once and let the shell finish on its own.
    #[tokio::test]
    async fn stdin_is_null_so_a_reading_child_gets_eof() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let started = Instant::now();
        let out = Bash::new(Background::new())
            .execute(
                BashArgs {
                    command: "read x; echo \"got:$x\"".into(),
                    timeout_secs: None,
                    background: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        // `read` hit EOF immediately (empty `$x`); it never blocked on stdin.
        assert!(out.output.contains("got:"), "{}", out.output);
        assert!(
            !out.output.contains("got:x"),
            "stdin must not carry data: {}",
            out.output
        );
        assert!(out.output.contains("exit code: 0"), "{}", out.output);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "must not block waiting on stdin"
        );
    }
}
