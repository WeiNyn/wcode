//! Background tasks: `bash { background: true }` starts a command that outlives
//! the turn; the `bg` tool lists/peeks/waits/kills it; and on the terminal state
//! the supervisor PUSHES the completion back to the agent through the actor seam
//! (`Request::Wake`, D15) — no kernel change, no new `Request` variant.
//!
//! The design is locked in `docs/backgrounding-plan.md` (D1–D17). D17 (a UI
//! `AgentEvent`) is **out of v1**: the UI signal is the
//! `AgentEvent::MessageReceived` the `Wake` already emits.
//!
//! Ground truth this module stands on (cited by function):
//! - `crates/wcode-cli/src/tools/bash.rs` — `spill_root`, `kill_group`,
//!   `process_group(0)` in `Bash::execute`, the `sh -c` + piped stdio spawn, the
//!   head+tail spill policy (`Capture`), `drain_lines`.
//! - `crates/wcode-harness/src/actor.rs` — `SessionHandle::send_from`, the
//!   delivery rules (`Wake` idle→run / mid-run→follow_up), and `tag`
//!   (`[message from {sender}]\n{content}`).
//! - `crates/wcode-harness/src/protocol.rs` — `Request::Wake`, `is_inbound`
//!   (Wake is one of the three inbound verbs).
//! - `crates/wcode-harness/src/tool.rs` — `TypedTool`, `ToolOutput`, `erased`.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio::sync::{oneshot, Notify};
use tokio_util::sync::CancellationToken;
use wcode_harness::actor::SessionHandle;
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::bash::{
    MAX_INLINE_BYTES, MAX_LINE_BYTES, PREVIEW_HEAD_CHARS, PREVIEW_TAIL_CHARS, spill_root,
};

// --------------------------------------------------------------------------
// The addr the completion is attributed to (D15). Not `agent:*`; see the
// `push_completion` note about inbound policy.
// --------------------------------------------------------------------------
const BG_SENDER: &str = "bg";

/// `wait` cap defaults / bounds (D6 / Q3).
const WAIT_DEFAULT_SECS: u64 = 60;
const WAIT_MAX_SECS: u64 = 300;
/// Lines of buffered tail sent on the completion push (D15) and by `output`.
const PUSH_TAIL_LINES: usize = 40;
/// How long the supervisor waits for both pipes to reach EOF after the child is
/// terminal, before giving up and pushing anyway (BLOCK-2).
const DRAIN_GRACE: Duration = Duration::from_secs(2);

// ==========================================================================
// State + registry (D3, D10)
// ==========================================================================

/// A task's lifecycle state (D3). `Exited { code: None }` mirrors a
/// signal-terminated child (`bash` renders that as "terminated by signal").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Exited { code: Option<i32> },
    Killed,
}

impl State {
    /// Whether the task has reached a terminal state — the supervisor's cue to
    /// push (D15) and `wait`'s exit condition (D6).
    pub fn is_terminal(&self) -> bool {
        matches!(self, State::Exited { .. } | State::Killed)
    }

    /// The human/model label: `"running"`, `"exited (0)"`, `"killed"`.
    pub fn label(&self) -> String {
        match self {
            State::Running => "running".to_string(),
            State::Exited { code: Some(code) } => format!("exited ({code})"),
            State::Exited { code: None } => "exited (signal)".to_string(),
            State::Killed => "killed".to_string(),
        }
    }
}

/// One stream's bounded capture: mirrors `bash::Capture` (head+tail preview,
/// spill to a file once `MAX_INLINE_BYTES` is passed) but writes under the
/// `bash-bg-<id>-<stream>.log` name so `sweep_stale_spills`'s `bash-*.log`
/// filter still reaps it (D4).
#[derive(Default)]
struct StreamSink {
    /// `spill_root()/bash-bg-<id>-<stream>.log`.
    spill_path: PathBuf,
    /// Accumulated text before the spill threshold; emptied once spilled.
    buf: String,
    /// Frozen first `PREVIEW_HEAD_CHARS` once the spill begins.
    head: String,
    /// Rolling last `PREVIEW_TAIL_CHARS` while spilling.
    tail: String,
    /// Total bytes seen (spilled or not).
    total: u64,
    /// `true` once the spill file has been created.
    spilled: bool,
}

/// The model-visible preview of one stream at a point in time (D4): the last
/// `tail_lines` lines, the total byte count, and the spill path when one exists.
struct StreamView {
    text: String,
    total: u64,
    spill: Option<PathBuf>,
}

impl StreamSink {
    /// New sink for `id`/`stream` under the shared spill dir.
    fn new(id: &str, stream: &'static str) -> Self {
        Self {
            spill_path: spill_root().join(format!("bash-bg-{id}-{stream}.log")),
            buf: String::new(),
            head: String::new(),
            tail: String::new(),
            total: 0,
            spilled: false,
        }
    }

    /// Append one decoded line (mirror `bash::Capture::push`: accumulate while
    /// small, else write through). `raw_len` is the pre-lossy byte length.
    fn push(&mut self, text: &str, raw_len: usize) {
        self.total += raw_len as u64;
        if self.spilled {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .append(true)
                .open(&self.spill_path)
            {
                let _ = file.write_all(text.as_bytes());
            }
            keep_tail(&mut self.tail, text, PREVIEW_TAIL_CHARS);
        } else {
            self.buf.push_str(text);
            if self.buf.len() > MAX_INLINE_BYTES {
                self.spill();
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
        let Ok(mut file) = std::fs::File::create(&self.spill_path) else {
            return;
        };
        let _ = file.write_all(self.buf.as_bytes());
        self.head = head_chars(&self.buf, PREVIEW_HEAD_CHARS);
        self.tail = tail_chars(&self.buf, PREVIEW_TAIL_CHARS);
        self.buf.clear();
        self.buf.shrink_to_fit();
        self.spilled = true;
    }

    /// The `tail_lines`-bounded preview + total + spill path.
    fn view(&self, tail_lines: usize) -> StreamView {
        let text = if self.spilled { &self.tail } else { &self.buf };
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(tail_lines);
        StreamView {
            text: lines[start..].join("\n"),
            total: self.total,
            spill: self.spilled.then(|| self.spill_path.clone()),
        }
    }
}

/// First `max` chars of `s` (mirror of `bash::head_chars`).
fn head_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Last `max` chars of `s` (mirror of `bash::tail_chars`).
fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(max)).collect()
}

/// Append `text` to `dst`, trimming from the front to keep at most `max` chars
/// (mirror of `bash::keep_tail`).
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

/// Buffered output for one task — one [`StreamSink`] per pipe.
#[derive(Default)]
struct Output {
    stdout: StreamSink,
    stderr: StreamSink,
}

/// One registered background task (D3).
pub struct Entry {
    pub id: String,
    pub command: String,
    /// Where the command ran (recorded for `bg status`/debugging).
    #[allow(dead_code)]
    pub cwd: PathBuf,
    pub started: Instant,
    /// `Child::id()` while the child is **un-reaped**, `None` afterwards.
    /// Cleared in the SAME lock scope that records the terminal state, *before*
    /// the (bounded) drain — that is what upholds `Running ⇒ pid.is_some()`
    /// (BLOCK-1).
    pub pid: Option<u32>,
    pub state: State,
    /// The ONE signal path (D5): `bg kill` / `shutdown` `send(())` here; the
    /// supervisor owns the matching `Receiver`. `None` once terminal (so a
    /// late `kill` is a clean no-op, D9). Taken out by the killer.
    kill_tx: Option<oneshot::Sender<()>>,
    /// Woken when `state` reaches a terminal value, so `bg wait` (D6/D11) parks
    /// without polling. The supervisor `notify_waiters()`s after the flip.
    state_changed: Arc<Notify>,
    /// Capped inline head+tail; the full stream spills to `bash-bg-*.log` (D4).
    output: Output,
}

/// The task table for one agent (D2/D3).
#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<String, Entry>,
}

impl Registry {
    /// Insert a freshly started entry (id unique within the registry, D13).
    fn insert(&mut self, entry: Entry) {
        self.entries.insert(entry.id.clone(), entry);
    }

    /// The entry, if the id is known (D9: an unknown id is a caller error, not a
    /// panic).
    fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// **BLOCK-1 — the ONE lock scope that records a terminal state.** It sets
    /// `state`, clears `pid` to `None` (so the reusable pid can never be
    /// signalled again), and takes the `kill_tx` (so a later `kill` is a no-op,
    /// D9) — all atomically, and *before* the supervisor awaits the drain.
    /// Returns the entry's [`Notify`] so the caller can wake waiters **after**
    /// the lock is released (never across an `.await`, D10).
    ///
    /// Invariant restored here: `Running ⇒ pid.is_some() ⇒ child un-reaped`.
    fn mark_terminal(&mut self, id: &str, state: State) -> Arc<Notify> {
        match self.entries.get_mut(id) {
            Some(entry) => {
                entry.state = state;
                entry.pid = None;
                entry.kill_tx = None;
                entry.state_changed.clone()
            }
            None => Arc::new(Notify::new()),
        }
    }

    /// All entries, insertion order.
    fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }
}

// ==========================================================================
// Background — one per agent, CLI-owned (D2, D8, D13, D16)
// ==========================================================================

/// Process-global weak list of live [`Background`]s (D8). `Drop` never runs on
/// `exec_self` or the ~29 `process::exit` sites, so teardown is an EXPLICIT
/// `shutdown_all()` at the `flush_all` seams; this list is what makes one call
/// cover every agent.
static LIVE: OnceLock<Mutex<Vec<Weak<Background>>>> = OnceLock::new();

/// One per agent: the task registry AND the late-bound notifier (D2/D16). Built
/// by the CLI where it builds the agent and threaded in as a parameter.
pub struct Background {
    /// Short critical sections only — never held across an `.await` (D10).
    registry: Mutex<Registry>,
    /// Late-bound push target (D16): filled by [`Background::bind`] right after
    /// the actor spawns. A task whose owner never bound has no one to notify.
    notify: OnceLock<SessionHandle>,
    /// Per-registry monotonic id source (`bg1`, `bg2`, …) (D13).
    next_id: AtomicU64,
}

impl Background {
    /// Build the per-agent handle and register it on the global weak list (D8).
    pub fn new() -> Arc<Self> {
        let bg = Arc::new(Self {
            registry: Mutex::new(Registry::default()),
            notify: OnceLock::new(),
            next_id: AtomicU64::new(1),
        });
        let list = LIVE.get_or_init(|| Mutex::new(Vec::new()));
        list.lock().unwrap().push(Arc::downgrade(&bg));
        bg
    }

    /// Late-bind the notifier — the CLI calls this immediately after
    /// `SessionActor::spawn` returns the handle (D16). A second call is ignored
    /// (`OnceLock`): the owner is fixed at spawn. (Consequence: each agent must
    /// have its OWN `Background`; reusing one across a rebuild would keep the
    /// stale, dead actor's handle.)
    pub fn bind(&self, handle: SessionHandle) {
        let _ = self.notify.set(handle);
    }

    /// Allocate the next id (`bg{n}`, D13).
    fn mint_id(&self) -> String {
        format!("bg{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Register a just-spawned child and hand back the `kill_rx` the supervisor
    /// will select on (D5). `pid` is `Child::id()`.
    pub(crate) fn register(
        &self,
        command: &str,
        cwd: &Path,
        pid: Option<u32>,
    ) -> (String, oneshot::Receiver<()>) {
        let id = self.mint_id();
        let (kill_tx, kill_rx) = oneshot::channel();
        let entry = Entry {
            id: id.clone(),
            command: command.to_string(),
            cwd: cwd.to_path_buf(),
            started: Instant::now(),
            pid,
            state: State::Running,
            kill_tx: Some(kill_tx),
            state_changed: Arc::new(Notify::new()),
            output: Output {
                stdout: StreamSink::new(&id, "stdout"),
                stderr: StreamSink::new(&id, "stderr"),
            },
        };
        self.lock().insert(entry);
        (id, kill_rx)
    }

    /// Snapshot one entry's state (for the immediate `bash` return and `bg
    /// status`).
    pub(crate) fn peek_state(&self, id: &str) -> Option<State> {
        self.lock().get(id).map(|entry| entry.state.clone())
    }

    /// The last `tail_lines` lines of the task's combined output plus the spill
    /// path when a stream spilled (D4). Used by the immediate `bash` return and
    /// `bg output`/`bg wait`.
    pub(crate) fn tail_of(&self, id: &str, tail_lines: usize) -> Option<(String, Option<PathBuf>)> {
        let reg = self.lock();
        let entry = reg.get(id)?;
        let out = entry.output.stdout.view(tail_lines);
        let err = entry.output.stderr.view(tail_lines);
        let mut parts: Vec<String> = Vec::new();
        let out_text = out.text.trim_end_matches('\n');
        if !out_text.is_empty() {
            parts.push(out_text.to_string());
        }
        let err_text = err.text.trim_end_matches('\n');
        if !err_text.is_empty() {
            parts.push(format!("[stderr]\n{err_text}"));
        }
        Some((parts.join("\n"), out.spill.or(err.spill)))
    }

    /// Signal every RUNNING group synchronously and return — no wait, a no-op
    /// when empty (D8).
    ///
    /// ## Invariant (BLOCK-1)
    /// A `shutdown` (or `bg kill`) signal is only ever sent for a pid that is
    /// **still un-reaped**, so it cannot hit a *reused* pid:
    ///
    /// ```text
    ///   Running  ⇒  pid.is_some()  ⇒  the child is un-reaped (not reusable)
    /// ```
    ///
    /// The supervisor upholds the `Running ⇒ pid.is_some()` half by clearing
    /// `pid` in the SAME lock scope that records the terminal state, *before* it
    /// awaits the (bounded) drain. So signals ONLY entries whose `pid.is_some()`
    /// — a `Running` entry is never a long-reaped one.
    ///
    /// It signals the stored `pid` directly (not via `kill_tx`): the caller may
    /// `exec_self` (replace the image) immediately after and would never give
    /// the supervisor task a turn.
    ///
    /// **Residual TOCTOU:** the `pid.is_some()` check and the `kill(-pid)` are
    /// not atomic with the reaper's `wait()`. Fully closing it needs a pidfd
    /// (`pidfd_open` + `pidfd_send_signal`) — a follow-up, not v1.
    pub fn shutdown(&self) {
        let reg = self.lock();
        for entry in reg.entries.values() {
            if let Some(pid) = entry.pid {
                signal_group(pid);
            }
        }
    }

    /// `shutdown()` every registered `Background` (D8). The single call the exit
    /// paths make next to `flush_all(FLUSH_TIMEOUT)`.
    pub fn shutdown_all() {
        let Some(list) = LIVE.get() else {
            return;
        };
        let mut list = list.lock().unwrap();
        // Prune dead handles; signal the live ones.
        list.retain(|weak| match weak.upgrade() {
            Some(bg) => {
                bg.shutdown();
                true
            }
            None => false,
        });
    }

    /// The registry lock, for the tools and the supervisor (short sections only,
    /// D10).
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap()
    }
}

// ==========================================================================
// Group signalling (D5) — `#[cfg(unix)]` mirrors `bash::kill_group`
// ==========================================================================

/// Own process group for a to-be-spawned child, so a kill can target the whole
/// group. The mirror of the `#[cfg(unix)] cmd.process_group(0)` line in
/// `Bash::execute`; off-unix it is a no-op (`child.kill()` is the fallback).
pub(crate) fn spawn_group(cmd: &mut tokio::process::Command) {
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(not(unix))]
    let _ = cmd;
}

/// Signal the whole process group led by `pid` (`kill(-pid, SIGKILL)`).
///
/// The bg sibling of `bash::kill_group`: it ONLY signals — the supervisor owns
/// the `wait()` (D5), so there is no `child.kill()`/`child.wait()` here. Only
/// called by the supervisor (unix) or `shutdown` while the child is un-reaped.
#[cfg(unix)]
fn signal_group(pid: u32) {
    // SAFETY: a negative pid signals a process group; failure (already gone) is
    // ignored, exactly as in `bash::kill_group`.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// Off-unix: the supervisor calls `child.kill()` directly; a stray group signal
/// is a no-op.
#[cfg(not(unix))]
fn signal_group(_pid: u32) {}

// ==========================================================================
// The supervisor — one task per background command (D3, D5, D15)
// ==========================================================================

/// Owns the un-reaped [`tokio::process::Child`]; drains both pipes into the
/// entry's bounded output; records the terminal state; and pushes the completion
/// (D3/D5/D15).
///
/// **BLOCK-1 (ordering).** `child.wait()` returning means the pid is now
/// *reaped and reusable*. The state is recorded — and `pid` cleared — in the
/// same lock scope **before** the drain is awaited, so a `Running` entry is
/// never a long-reaped one and `shutdown()`/`bg kill` can never signal a reused
/// pid.
///
/// **BLOCK-2 (drain hang).** A daemonized grandchild can hold a pipe open after
/// the child exits, so the drain is bounded by [`DRAIN_GRACE`]; on expiry the
/// pipe readers are **dropped**, closing our ends so the grandchild's next write
/// gets `EPIPE`, and the push proceeds with the tail captured so far.
///
/// The supervisor is the **sole signaler** (D5).
pub(crate) fn spawn_supervisor(
    bg: Arc<Background>,
    id: String,
    mut child: tokio::process::Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    kill_rx: oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // The pipes drain CONCURRENTLY with the reap: a command whose output
        // exceeds the pipe buffer would otherwise block on write and never exit.
        // The task owns the readers, so aborting it closes our ends (BLOCK-2).
        let drain_bg_arc = bg.clone();
        let drain_id = id.clone();
        let mut drain_handle = tokio::spawn(async move {
            futures::join!(
                drain_bg(drain_bg_arc.clone(), drain_id.clone(), "stdout", stdout),
                drain_bg(drain_bg_arc, drain_id, "stderr", stderr),
            );
        });

        // Phase A — the terminal state FIRST, in ONE lock scope (BLOCK-1). The
        // `select!` bodies only produce an `Outcome` (no `child` borrow), so the
        // borrow from `child.wait()` is released before the kill arm signals.
        enum Outcome {
            Exited(std::io::Result<std::process::ExitStatus>),
            Killed,
        }
        let outcome = tokio::select! {
            biased;
            st = child.wait() => Outcome::Exited(st),
            _ = kill_rx => Outcome::Killed,
        };
        let state = match outcome {
            Outcome::Exited(st) => State::Exited {
                code: st.ok().and_then(|status| status.code()),
            },
            Outcome::Killed => {
                #[cfg(unix)]
                if let Some(pid) = child.id() {
                    signal_group(pid);
                }
                #[cfg(not(unix))]
                {
                    let _ = child.kill().await;
                }
                let _ = child.wait().await;
                State::Killed
            }
        };
        // state set AND pid→None, atomically, before the bounded drain (BLOCK-1).
        let state_notify = bg.lock().mark_terminal(&id, state.clone());

        // Phase B — BOUNDED drain (BLOCK-2): a held pipe (a daemonized grandchild)
        // must not stall the push. On expiry, abort the task so its readers drop
        // (closing our ends) and the push proceeds with the tail captured so far.
        if tokio::time::timeout(DRAIN_GRACE, &mut drain_handle)
            .await
            .is_err()
        {
            drain_handle.abort();
        }

        state_notify.notify_waiters();
        push_completion(&bg, &id, &state);
    })
}

/// Read one pipe line-by-line and append each decoded line to the entry's
/// bounded buffer under a SHORT registry lock (never across the `.await`, D10).
/// Mirrors `bash::drain_lines`'s chunking (`MAX_LINE_BYTES`) so a no-newline
/// flood stays bounded even though `read_until` sees one giant line. Dropping
/// the returned future (on the BLOCK-2 timeout) drops the reader, closing our
/// end of the pipe.
async fn drain_bg<R>(bg: Arc<Background>, id: String, stream: &'static str, pipe: Option<R>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let Some(pipe) = pipe else {
        return;
    };
    let mut reader = tokio::io::BufReader::new(pipe);
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) => break, // EOF: pipe closed
            Ok(_) => {
                // Chunk a pathological no-newline stream so memory stays
                // bounded (see `bash::drain_lines`).
                let mut start = 0;
                while line.len().saturating_sub(start) > MAX_LINE_BYTES {
                    let end = start + MAX_LINE_BYTES;
                    let text = String::from_utf8_lossy(&line[start..end]);
                    push_line(&bg, &id, stream, &text, end - start);
                    start = end;
                }
                if start < line.len() {
                    let text = String::from_utf8_lossy(&line[start..]);
                    push_line(&bg, &id, stream, &text, line.len() - start);
                }
            }
            Err(_) => break,
        }
    }
}

/// Append one decoded chunk to the named stream's sink under a short lock.
fn push_line(bg: &Background, id: &str, stream: &'static str, text: &str, raw_len: usize) {
    let mut reg = bg.lock();
    if let Some(entry) = reg.entries.get_mut(id) {
        let sink = if stream == "stdout" {
            &mut entry.output.stdout
        } else {
            &mut entry.output.stderr
        };
        sink.push(text, raw_len);
    }
}

// ==========================================================================
// Completion push (D15, D16) — reuses the peer-`message` seam, no kernel change
// ==========================================================================

/// Frame the terminal state as the body of a peer message and deliver it through
/// the actor seam (D15):
///
/// ```text
///   handle.send_from(SessionId::new("bg"), Request::Wake { content })
/// ```
///
/// By the actor's delivery rules this **wakes an idle agent into a full turn**
/// and, mid-run, becomes a `follow_up` appended to the next turn's context. Both
/// paths also emit `AgentEvent::MessageReceived { from, content }`.
///
/// **The `[message from bg]` prefix is added by the actor's `tag`**, so `content`
/// here is the BODY only — passing a pre-tagged string would double-tag.
///
/// A missing `notify` (never bound — e.g. a one-shot `-p`) or a closed session
/// (`send_from` → `Err(SessionClosed)`) drops silently (D16).
fn push_completion(bg: &Background, id: &str, state: &State) {
    let Some(handle) = bg.notify.get() else {
        return;
    };
    let content = completion_message(bg, id, state);
    let _ = handle.send_from(SessionId::new(BG_SENDER), Request::Wake { content });
}

/// The body: `bg3 exited (0) after 12s` / `bg3 killed` / `bg3 failed (exit 1)`,
/// then the tail of the buffered output (D15). The `[message from bg]` prefix is
/// NOT included here (the actor adds it).
fn completion_message(bg: &Background, id: &str, state: &State) -> String {
    let elapsed = {
        let reg = bg.lock();
        match reg.get(id) {
            Some(entry) => entry.started.elapsed(),
            None => return format!("{id} {}", state.label()),
        }
    };
    let head = match state {
        State::Killed => format!("{id} killed"),
        State::Running => format!("{id} running"),
        State::Exited { code: Some(0) } => {
            format!("{id} exited (0) after {}s", elapsed.as_secs())
        }
        State::Exited { code: Some(code) } => format!("{id} failed (exit {code})"),
        State::Exited { code: None } => format!("{id} terminated by signal"),
    };
    match bg.tail_of(id, PUSH_TAIL_LINES) {
        Some((tail, _)) if !tail.is_empty() => format!("{head}\n{tail}"),
        _ => head,
    }
}

// ==========================================================================
// The `bg` tool (D1, D6, D7, D11)
// ==========================================================================

/// Arguments for `bg` (D6). `action` selects the arm; `task_id` is required for
/// every arm except `list`.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct BgArgs {
    /// One of `"list" | "status" | "output" | "wait" | "kill"`.
    pub action: String,
    /// The target task (`bg3`); required for everything but `list`.
    pub task_id: Option<String>,
    /// Lines of output returned by `output` (default 40).
    pub tail_lines: Option<usize>,
    /// `wait` cap in seconds (default 60, max 300) (D6/Q3).
    pub timeout_secs: Option<u64>,
}

/// The action, parsed from `BgArgs::action` (an unknown value is a `ToolOutput`
/// error, never a panic — D9).
enum Action {
    List,
    Status,
    Output,
    Wait,
    Kill,
}

impl Action {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "list" => Ok(Action::List),
            "status" => Ok(Action::Status),
            "output" => Ok(Action::Output),
            "wait" => Ok(Action::Wait),
            "kill" => Ok(Action::Kill),
            other => Err(format!(
                "unknown bg action `{other}`; expected one of list, status, output, wait, kill"
            )),
        }
    }
}

/// The `bg` tool (D1). Holds the per-agent [`Background`] (D2).
pub struct Bg {
    bg: Arc<Background>,
}

impl Bg {
    pub fn new(bg: Arc<Background>) -> Self {
        Self { bg }
    }
}

#[async_trait::async_trait]
impl TypedTool for Bg {
    type Args = BgArgs;

    fn name(&self) -> &str {
        "bg"
    }

    /// One sentence per action; mentions that a finished task also PUSHES a
    /// `[message from bg]` (D15) so the model knows to expect it.
    fn description(&self) -> &str {
        "Manage background tasks started with `bash { background: true }`. \
         Actions: `list` (all tasks), `status <id>`, `output <id>` (tail + spill \
         path), `wait <id> [timeout_secs]` (block until it finishes), `kill <id>`. \
         A finished task also messages the session automatically."
    }

    // `parallel_safe()` and `mutating()` keep the trait defaults (`false`): the
    // registry is not workspace mutation, so `bg` must NOT join `MUTATING_TOOLS`
    // (D7).

    /// One arm per action (D6). Every failure (unknown id, kill on a terminal
    /// task, a wait that hits the cap or `ctx.cancel`) is a `ToolOutput`
    /// `{ is_error: true }`, never a panic (D9/D11).
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let action = match Action::parse(&args.action) {
            Ok(action) => action,
            Err(message) => return error(message),
        };
        match action {
            Action::List => self.list(),
            Action::Status => match self.require_id(&args.task_id) {
                Ok(id) => self.status(&id),
                Err(message) => error(message),
            },
            Action::Output => match self.require_id(&args.task_id) {
                Ok(id) => self.output(&id, args.tail_lines.unwrap_or(PUSH_TAIL_LINES)),
                Err(message) => error(message),
            },
            Action::Wait => match self.require_id(&args.task_id) {
                Ok(id) => {
                    let timeout = args
                        .timeout_secs
                        .unwrap_or(WAIT_DEFAULT_SECS)
                        .min(WAIT_MAX_SECS);
                    match wait_for_terminal(
                        &self.bg,
                        &id,
                        Duration::from_secs(timeout),
                        &ctx.cancel,
                    )
                    .await
                    {
                        WaitOutcome::Terminal(state) => self.terminal_output(&id, &state),
                        WaitOutcome::Unknown => unknown(&id),
                        WaitOutcome::TimedOut => error(format!(
                            "{id} still running after {timeout}s"
                        )),
                        WaitOutcome::Cancelled => error("cancelled".to_string()),
                    }
                }
                Err(message) => error(message),
            },
            Action::Kill => match self.require_id(&args.task_id) {
                Ok(id) => self.kill(&id, &ctx.cancel).await,
                Err(message) => error(message),
            },
        }
    }
}

impl Bg {
    /// The `task_id` for every arm but `list`.
    fn require_id(&self, task_id: &Option<String>) -> Result<String, String> {
        task_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| format!("`{}` requires a task_id", self.action_hint()))
    }

    fn action_hint(&self) -> &'static str {
        "action"
    }

    /// `bg list` → one line per task, newest id last.
    fn list(&self) -> ToolOutput {
        let reg = self.bg.lock();
        if reg.entries.is_empty() {
            return ok("(no background tasks)".to_string());
        }
        let mut lines = Vec::new();
        for entry in reg.iter() {
            let bytes =
                entry.output.stdout.view(0).total + entry.output.stderr.view(0).total;
            lines.push(format!(
                "{} {} {}s  {}  ({} output)",
                entry.id,
                entry.state.label(),
                entry.started.elapsed().as_secs(),
                entry.command,
                human_bytes(bytes),
            ));
        }
        ok(lines.join("\n"))
    }

    /// `bg status <id>`.
    fn status(&self, id: &str) -> ToolOutput {
        let reg = self.bg.lock();
        let Some(entry) = reg.get(id) else {
            return unknown(id);
        };
        ok(format!(
            "{} {} {}s  {}",
            entry.id,
            entry.state.label(),
            entry.started.elapsed().as_secs(),
            entry.command,
        ))
    }

    /// `bg output <id> [tail_lines]` — the tail + the spill path (D4).
    fn output(&self, id: &str, tail_lines: usize) -> ToolOutput {
        let state = {
            let reg = self.bg.lock();
            match reg.get(id) {
                Some(entry) => entry.state.clone(),
                None => return unknown(id),
            }
        };
        let mut body = format!("{id} {}", state.label());
        if let Some((tail, spill)) = self.bg.tail_of(id, tail_lines) {
            if !tail.is_empty() {
                body.push('\n');
                body.push_str(&tail);
            }
            if let Some(path) = spill {
                body.push_str(&format!("\nfull output: {}", path.display()));
            }
        }
        ok(body)
    }

    /// `bg wait <id>` — the terminal line + the tail.
    fn terminal_output(&self, id: &str, state: &State) -> ToolOutput {
        let mut body = format!("{id} {}", state.label());
        if let Some((tail, _)) = self.bg.tail_of(id, PUSH_TAIL_LINES)
            && !tail.is_empty()
        {
            body.push('\n');
            body.push_str(&tail);
        }
        ok(body)
    }

    /// `bg kill <id>` — request the kill and report the resulting state (D5/D9).
    /// A second kill on the terminal task is a no-op returning the state.
    async fn kill(&self, id: &str, cancel: &CancellationToken) -> ToolOutput {
        // Take the ONE signal path; a `None` here is either "unknown id" or
        // "already terminal" (D5).
        let taken = {
            let mut reg = self.bg.lock();
            match reg.entries.get_mut(id) {
                Some(entry) => Some(entry.kill_tx.take()),
                None => None,
            }
        };
        let Some(kill_tx) = taken else {
            return unknown(id);
        };
        let Some(tx) = kill_tx else {
            let label = {
                let reg = self.bg.lock();
                reg.get(id).map(|e| e.state.label()).unwrap_or_default()
            };
            return ok(format!("{id} already {label}"));
        };
        let _ = tx.send(());
        // The supervisor owns the signal + the wait; park until it flips.
        match wait_for_terminal(&self.bg, id, Duration::from_secs(5), cancel).await {
            WaitOutcome::Terminal(state) => ok(format!("{id} {}", state.label())),
            WaitOutcome::Unknown => unknown(id),
            WaitOutcome::TimedOut => ok(format!("{id} kill requested")),
            WaitOutcome::Cancelled => error("cancelled".to_string()),
        }
    }
}

/// How a `wait`/`kill` parked wait ended.
enum WaitOutcome {
    Terminal(State),
    Unknown,
    TimedOut,
    Cancelled,
}

/// Park until `id` is terminal, the deadline elapses, or `cancel` fires (D6/D11).
/// Woken by the entry's `state_changed` `Notify` (no polling); the state is
/// re-checked each wake.
async fn wait_for_terminal(
    bg: &Background,
    id: &str,
    timeout: Duration,
    cancel: &CancellationToken,
) -> WaitOutcome {
    let notify = {
        let reg = bg.lock();
        match reg.get(id) {
            Some(entry) if entry.state.is_terminal() => {
                return WaitOutcome::Terminal(entry.state.clone());
            }
            Some(entry) => entry.state_changed.clone(),
            None => return WaitOutcome::Unknown,
        }
    };
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        // Re-check after arming the waiter, so a flip between iterations is seen.
        match bg.lock().get(id).map(|entry| entry.state.clone()) {
            None => return WaitOutcome::Unknown,
            Some(state) if state.is_terminal() => return WaitOutcome::Terminal(state),
            Some(_) => {}
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return WaitOutcome::Cancelled,
            _ = &mut deadline => return WaitOutcome::TimedOut,
            _ = &mut notified => {}
        }
    }
}

/// A non-error `ToolOutput`.
fn ok(output: String) -> ToolOutput {
    ToolOutput {
        output,
        is_error: false,
        diff: None,
        path: None,
    }
}

/// An error `ToolOutput` (D9).
fn error(output: String) -> ToolOutput {
    ToolOutput {
        output,
        is_error: true,
        diff: None,
        path: None,
    }
}

/// The shared "unknown task id" error (D9).
fn unknown(id: &str) -> ToolOutput {
    error(format!("unknown background task `{id}`"))
}

/// Coarse human byte count for `bg list`.
fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let n = bytes as f64;
    if n < KB {
        format!("{n:.0} B")
    } else if n < MB {
        format!("{:.1} KB", n / KB)
    } else {
        format!("{:.1} MB", n / MB)
    }
}

// ==========================================================================
// Tests — mirror the plan's §7 test list
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use wcode_harness::actor::SessionActor;
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::{AgentEvent, LlmStreamEvent};
    use wcode_harness::hooks::{HooksSet, PlanModeHandle};
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::message::{AgentMessage, StopReason};
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    use super::super::bash::{Bash, BashArgs};

    /// A never-failing stream that ends its turn at once.
    fn done_stream() -> StreamFn {
        Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::iter(vec![LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            }])) as LlmStream
        })
    }

    /// A `SessionActor` running `stream_fn`.
    fn session_with(stream_fn: StreamFn) -> SessionHandle {
        SessionActor::spawn(Agent::new(AgentConfig {
            system: "sys".into(),
            tools: Vec::new(),
            llm: LlmOpts::default(),
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
            plan_mode: PlanModeHandle::new(),
        }))
    }

    /// A `Background` with a bound, recording `SessionHandle` so pushes land
    /// somewhere observable, plus its event receiver.
    fn bg_with_handle() -> (
        Arc<Background>,
        tokio::sync::broadcast::Receiver<AgentEvent>,
    ) {
        let bg = Background::new();
        let handle = session_with(done_stream());
        let rx = handle.subscribe();
        bg.bind(handle);
        (bg, rx)
    }

    fn list_args() -> BgArgs {
        BgArgs {
            action: "list".into(),
            task_id: None,
            tail_lines: None,
            timeout_secs: None,
        }
    }

    fn args(action: &str, id: &str) -> BgArgs {
        BgArgs {
            action: action.into(),
            task_id: Some(id.into()),
            tail_lines: None,
            timeout_secs: None,
        }
    }

    async fn start_bg(bg: &Arc<Background>, ctx: &ToolContext, command: &str) -> ToolOutput {
        Bash::new(bg.clone())
            .execute(
                BashArgs {
                    command: command.into(),
                    timeout_secs: None,
                    background: Some(true),
                },
                ctx,
            )
            .await
    }

    async fn run_bg(bg: &Arc<Background>, ctx: &ToolContext, a: BgArgs) -> ToolOutput {
        Bg::new(bg.clone()).execute(a, ctx).await
    }

    #[tokio::test]
    async fn start_then_list_shows_a_running_task() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        let out = start_bg(&bg, &ctx, "sleep 5").await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.starts_with("started bg1"), "{}", out.output);

        let list = run_bg(&bg, &ctx, list_args()).await;
        assert!(list.output.contains("bg1"), "{}", list.output);
        assert!(list.output.contains("running"), "{}", list.output);
        bg.shutdown();
    }

    #[tokio::test]
    async fn kill_transitions_to_killed_and_is_a_noop_after_exit() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        start_bg(&bg, &ctx, "sleep 30").await;
        let killed = run_bg(&bg, &ctx, args("kill", "bg1")).await;
        assert!(!killed.is_error, "{}", killed.output);
        assert!(killed.output.contains("killed"), "{}", killed.output);

        // A second kill (on the terminal task) is a no-op returning the state.
        let again = run_bg(&bg, &ctx, args("kill", "bg1")).await;
        assert!(!again.is_error, "{}", again.output);
        assert!(again.output.contains("already killed"), "{}", again.output);
    }

    #[tokio::test]
    async fn wait_on_a_short_task_returns_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        start_bg(&bg, &ctx, "true").await;
        let mut a = args("wait", "bg1");
        a.timeout_secs = Some(10);
        let out = run_bg(&bg, &ctx, a).await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("exited (0)"), "{}", out.output);
    }

    #[tokio::test]
    async fn output_is_capped_and_names_the_spill_file() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        // ~100 KiB, well over MAX_INLINE_BYTES.
        start_bg(&bg, &ctx, "seq 1 20000").await;
        let mut wait = args("wait", "bg1");
        wait.timeout_secs = Some(10);
        run_bg(&bg, &ctx, wait).await;

        let mut output = args("output", "bg1");
        output.tail_lines = Some(5);
        let out = run_bg(&bg, &ctx, output).await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("full output:"), "{}", out.output);
        assert!(out.output.contains("bash-bg-bg1-stdout.log"), "{}", out.output);
        // Bounded: 5 tail lines + the state line + the spill line, not 20000.
        assert!(out.output.lines().count() < 20, "{}", out.output);

        // The spill file holds the whole stream, head to tail.
        let (_, spill) = bg.tail_of("bg1", 5).unwrap();
        let path = spill.expect("spill path");
        let full = std::fs::read_to_string(&path).expect("spill file readable");
        assert!(full.starts_with("1\n"), "starts at the head");
        assert!(full.trim_end().ends_with("20000"), "ends at the tail");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn two_concurrent_tasks_get_distinct_ids() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        let a = start_bg(&bg, &ctx, "sleep 5").await;
        let b = start_bg(&bg, &ctx, "sleep 5").await;
        assert!(a.output.starts_with("started bg1"), "{}", a.output);
        assert!(b.output.starts_with("started bg2"), "{}", b.output);

        let list = run_bg(&bg, &ctx, list_args()).await;
        assert!(list.output.contains("bg1"), "{}", list.output);
        assert!(list.output.contains("bg2"), "{}", list.output);
        bg.shutdown();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_leaves_no_orphaned_group() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();
        let pidfile = dir.path().join("bg.pid");

        start_bg(
            &bg,
            &ctx,
            &format!("echo $$ > '{}'; sleep 30", pidfile.display()),
        )
        .await;

        // Wait for the shell to write its pid.
        let mut pid: Option<i32> = None;
        for _ in 0..200 {
            if let Ok(s) = std::fs::read_to_string(&pidfile)
                && let Ok(p) = s.trim().parse::<i32>()
            {
                pid = Some(p);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = pid.expect("shell wrote its pid");

        run_bg(&bg, &ctx, args("kill", "bg1")).await;

        // A signal of 0 probes for existence; ESRCH means the process is gone.
        let mut gone = false;
        for _ in 0..100 {
            let rc = unsafe { libc::kill(pid, 0) };
            if rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(gone, "shell (pid {pid}) survived `bg kill`");
    }

    #[tokio::test]
    async fn shutdown_kills_a_running_group() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        start_bg(&bg, &ctx, "sleep 30").await;
        bg.shutdown();

        let mut wait = args("wait", "bg1");
        wait.timeout_secs = Some(10);
        let out = run_bg(&bg, &ctx, wait).await;
        assert!(!out.is_error, "{}", out.output);
        assert!(
            out.output.contains("exited") || out.output.contains("killed"),
            "{}",
            out.output
        );

        // An empty `shutdown()` is a no-op.
        Background::new().shutdown();
    }

    #[tokio::test]
    async fn wait_aborts_on_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        start_bg(&bg, &ctx, "sleep 30").await;
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        });

        let mut wait = args("wait", "bg1");
        wait.timeout_secs = Some(60);
        let out = run_bg(&bg, &ctx, wait).await;
        assert!(out.is_error, "{}", out.output);
        assert_eq!(out.output, "cancelled");
        bg.shutdown();
    }

    /// BLOCK-1 regression: a task that has exited (pid cleared) is never
    /// signalled by a late `shutdown()`.
    #[tokio::test]
    async fn shutdown_never_signals_an_exited_task() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let bg = Background::new();

        start_bg(&bg, &ctx, "true").await;
        let mut wait = args("wait", "bg1");
        wait.timeout_secs = Some(10);
        run_bg(&bg, &ctx, wait).await;

        let pid = bg.lock().get("bg1").unwrap().pid;
        assert_eq!(pid, None, "pid cleared once terminal (BLOCK-1)");

        // No signal is sent (a no-op); it must not panic or touch a reused pid.
        bg.shutdown();
    }

    /// BLOCK-2 regression: a task whose grandchild holds a pipe still reaches a
    /// terminal push within `DRAIN_GRACE`, not never.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_held_pipe_does_not_stall_the_push() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let (bg, mut rx) = bg_with_handle();

        // `sleep 30 &` leaves a grandchild holding the pipe open after `sh` exits.
        start_bg(&bg, &ctx, "sleep 30 &").await;

        let pushed = tokio::time::timeout(DRAIN_GRACE + Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Ok(AgentEvent::MessageReceived { from, .. })
                        if from == SessionId::new(BG_SENDER) =>
                    {
                        break;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => panic!("session closed before the push"),
                }
            }
        })
        .await;
        pushed.expect("completion push within DRAIN_GRACE despite a held pipe");
    }

    /// A finished task wakes an idle actor into a turn whose prompt is the
    /// tagged `[message from bg]` body (D15).
    #[tokio::test]
    async fn a_finished_task_wakes_an_idle_actor() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = seen.clone();
        let stream_fn: StreamFn = Arc::new(move |ctx, _sys, _tools, _opts| {
            let last = ctx
                .iter()
                .rev()
                .find_map(|m| match m {
                    AgentMessage::User { .. } => Some(m.as_text()),
                    _ => None,
                })
                .unwrap_or_default();
            recorder.lock().unwrap().push(last);
            Box::pin(futures::stream::iter(vec![LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            }])) as LlmStream
        });

        let bg = Background::new();
        let handle = session_with(stream_fn);
        let mut rx = handle.subscribe();
        bg.bind(handle);

        start_bg(&bg, &ctx, "true").await;

        // Wait for the turn the push started to finish.
        let ran = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match rx.recv().await {
                    Ok(AgentEvent::AgentEnd) => break true,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break false,
                }
            }
        })
        .await;
        assert_eq!(ran, Ok(true), "the push started a turn");

        let prompts = seen.lock().unwrap().clone();
        assert!(
            prompts
                .iter()
                .any(|p| p.starts_with("[message from bg]\nbg1 exited (0)")),
            "the turn's prompt is the tagged bg body: {prompts:?}"
        );
    }

    /// A finished task mid-run is delivered as a `follow_up` and lands in the
    /// next turn's context (D15).
    #[tokio::test]
    async fn a_finished_task_mid_run_lands_as_a_follow_up() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());

        // The first run blocks until released, so the push lands mid-run.
        let release = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stream_fn: StreamFn = {
            let release = release.clone();
            let calls = calls.clone();
            let seen = seen.clone();
            Arc::new(move |ctx, _sys, _tools, _opts| {
                let last = ctx
                    .iter()
                    .rev()
                    .find_map(|m| match m {
                        AgentMessage::User { .. } => Some(m.as_text()),
                        _ => None,
                    })
                    .unwrap_or_default();
                seen.lock().unwrap().push(last);
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    let release = release.clone();
                    Box::pin(futures::stream::once(async move {
                        release.notified().await;
                        LlmStreamEvent::Done {
                            stop_reason: StopReason::Stop,
                            usage: None,
                        }
                    })) as LlmStream
                } else {
                    Box::pin(futures::stream::iter(vec![LlmStreamEvent::Done {
                        stop_reason: StopReason::Stop,
                        usage: None,
                    }])) as LlmStream
                }
            })
        };

        let bg = Background::new();
        let handle = session_with(stream_fn);
        let mut rx = handle.subscribe();
        bg.bind(handle.clone());

        // Start a run and wait until it is in flight (first stream_fn call).
        handle
            .send(Request::Submit {
                text: "work".into(),
            })
            .unwrap();
        for _ in 0..200 {
            if calls.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(calls.load(Ordering::SeqCst) >= 1, "the run started");

        // Complete a task mid-run: the push becomes a follow_up.
        start_bg(&bg, &ctx, "true").await;
        let pushed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match rx.recv().await {
                    Ok(AgentEvent::MessageReceived { from, .. })
                        if from == SessionId::new(BG_SENDER) =>
                    {
                        break true;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break false,
                }
            }
        })
        .await;
        assert_eq!(pushed, Ok(true), "the push arrived mid-run");

        // Release the first run; the follow-up starts the next turn.
        release.notify_waiters();
        let mut landed = false;
        for _ in 0..300 {
            if seen
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.starts_with("[message from bg]"))
            {
                landed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            landed,
            "the follow-up landed in the next turn's context: {:?}",
            seen.lock().unwrap()
        );
    }
}
