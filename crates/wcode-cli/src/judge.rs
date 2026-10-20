//! The workflow-edge **judge** (W025 §4.3, B2j): a one-shot, **zero-tool** LLM
//! completion that decides whether a conditional edge is taken.
//!
//! Locked design: `docs/work/W025-workflow-edges-and-lateral-messaging.md`
//! §4.3 (APPROVED rev. 2). The judge is **a CLI trait owned by the scheduler** —
//! never a plan node, never a session. [`Scheduler`](crate::scheduler::Scheduler)
//! holds an `Arc<dyn Judge>` and `tokio::spawn`s [`Judge::decide`] for each
//! `judge` edge; the verdict comes back on the scheduler's `judge_rx` channel, so
//! **every** `TaskList` mutation stays on the loop thread.
//!
//! ## Why a trait and not a spawned worker
//!
//! A worker's `ReportBack` (`crates/wcode-cli/src/agents.rs:493 struct ReportBack`)
//! forwards its final reply to its **owner**
//! (`agents.rs:518 .deliver(&self.me, &self.owner, Request::Wake { content })`).
//! A worker-judge's verdict would therefore land at the *root*, never at the
//! scheduler — the exact hole design option (iii) falls into. So the judge is
//! not a session: it is one `stream_fn` call with `tools = &[]`.
//!
//! ## The `judger` member supplies role + model only
//!
//! [`ModelJudge`] is built from a resolved `TeamMember` (`.wcode/agents/judger.md`
//! or a `[[team]]` entry) — its `model`/`base_url`/`api_key`/`effort`/`role`.
//! The member is **never spawned**, so its `tools` field is ignored: the
//! completion has zero tools by construction. This is what resolves the D3
//! "a worker keeps `message`" ambiguity for the judge. No `judger` member →
//! [`SessionFactory::judge`](crate::agents::SessionFactory::judge) falls back to
//! the root template's `llm`/`stream_fn` + the built-in contract.
//!
//! ## Status
//!
//! Registered (`mod judge;` in `crates/wcode-cli/src/main.rs`) and wired by
//! [`SessionFactory::judge`](crate::agents::SessionFactory::judge) into the
//! scheduler (`crate::scheduler::Scheduler`).

use std::future::Future;
use std::pin::Pin;
use std::sync::LazyLock;

use futures::StreamExt as _;
use regex::Regex;
use wcode_harness::event::LlmStreamEvent;
use wcode_harness::message::AgentMessage;
use wcode_harness::streamfn::{LlmOpts, StreamFn};

/// One edge verdict. `None` = the reply carried no parseable verdict — the
/// caller's contract (§4.3) is **block-and-escalate**, never a silent skip.
pub type Verdict = Option<bool>;

/// Decide whether a conditional edge is taken.
///
/// Object-safe and `Send + Sync` so the scheduler can hold `Arc<dyn Judge>` and
/// spawn `decide` from its loop thread. The returned future borrows `self` and
/// both inputs for `'a`, so an implementation may read a member's role text
/// without cloning.
///
/// # Contract
///
/// - `criterion` — the edge's `condition.criterion` (never empty; the config
///   validator rejects an empty one at load, `config.rs`).
/// - `upstream` — the **source** node's `artifact` (`Dep`'s `from`), or `None`
///   when the source produced no artifact. Implementations must accept `None`
///   and still answer.
/// - Returns `Some(true)` = take the edge, `Some(false)` = skip the target,
///   `None` = unparseable → the scheduler records the node `unjudgeable`, leaves
///   it `Todo`, and escalates once. `None` NEVER means "skip".
/// - **Latency is bounded by the EDGE**, not by this trait: the scheduler wraps
///   `decide` in the edge's `timeout_secs` (unset or `0` = no bound; D033 as
///   amended) and maps an elapsed deadline onto `None` — block-and-escalate,
///   never a skip (W028). An implementation need not bound itself.
///
/// # Errors
///
/// None: a transport error, a fatal stream error, or an empty reply all map to
/// `None` (the escalate fallback) rather than propagating.
pub trait Judge: Send + Sync {
    /// Return the verdict for `criterion` over `upstream`. See the trait docs;
    /// this is the only method.
    fn decide<'a>(
        &'a self,
        criterion: &'a str,
        upstream: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Verdict> + Send + 'a>>;
}

/// The built-in judge system contract (W025 §4.3). Constant so a test can pin
/// its exact bytes and the `judger`-member path can prepend a role to it.
pub const JUDGE_SYSTEM_CONTRACT: &str = "You are a workflow-edge judge. You have no tools. \
     Answer only with:\nVERDICT: TAKE\nREASON: <one short line>\nor VERDICT: SKIP\nREASON: \
     <one short line>";

/// The static verdict matcher: `(?i)^\s*VERDICT:\s*(TAKE|SKIP)\b` (`config.rs`
/// is where the *shape* is validated; this is where the *reply* is parsed).
/// Compiled once (`LazyLock`), mirroring `tools/webfetch.rs:428`.
static VERDICT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*VERDICT:\s*(TAKE|SKIP)\b").expect("static verdict regex")
});

/// Parse a judge reply into a verdict.
///
/// Pure. Scans `text` line by line and returns on the **first** line matching
/// `(?i)^\s*VERDICT:\s*(TAKE|SKIP)\b` — so a markdown-wrapped (`**VERDICT: TAKE**`
/// is NOT matched: the `**` breaks `^\s*`) probed case returns `None`, while a
/// reply that names both tokens across lines returns the **first** one. `TAKE` ⇒
/// `Some(true)`, `SKIP` ⇒ `Some(false)`, no match ⇒ `None`.
///
/// # Does not promise
///
/// - It does not read `REASON:`; that line is for a human reading the journal.
/// - It does not tolerate a mid-line `VERDICT:` (the anchor is `^`).
/// - It is not case-sensitive on the token (the regex is `(?i)`).
///
/// # Panics
///
/// Never on input: the static regex is compiled once and `expect`ed (a
/// programming error, not a data error).
pub fn parse_verdict(text: &str) -> Verdict {
    for line in text.lines() {
        if let Some(caps) = VERDICT_RE.captures(line) {
            let token = caps.get(1).expect("group 1 is always present");
            return Some(token.as_str().eq_ignore_ascii_case("TAKE"));
        }
    }
    None
}

/// The production judge: one zero-tool completion through the shared
/// [`StreamFn`].
///
/// Fields are private; build it with [`ModelJudge::new`] (or, in the crate,
/// [`SessionFactory::judge`](crate::agents::SessionFactory::judge)).
pub struct ModelJudge {
    /// The resolved `judger` member's opts (model/base_url/api_key/effort), or
    /// the root template's `llm` when no member exists.
    llm: LlmOpts,
    /// The shared stream adapter (`streamfn.rs:284 pub type StreamFn =`). Reused,
    /// not rebuilt: the rig keystore pools by `(base_url, api_key, session_id)`.
    stream_fn: StreamFn,
    /// The system prompt: [`JUDGE_SYSTEM_CONTRACT`], plus the member's role text
    /// as a `# Role` section when the member has one.
    system: String,
}

impl ModelJudge {
    /// Build a judge from already-resolved pieces.
    ///
    /// `system` should be [`JUDGE_SYSTEM_CONTRACT`], optionally with a member's
    /// role appended — the caller (`SessionFactory::judge`) owns the resolution,
    /// so this ctor has no config dependency.
    ///
    /// **Does not promise** anything about `llm.tools` — there is no tools field
    /// on `LlmOpts`; zero tools are passed at call time, not configured here.
    pub fn new(llm: LlmOpts, stream_fn: StreamFn, system: String) -> Self {
        Self {
            llm,
            stream_fn,
            system,
        }
    }
}

impl Judge for ModelJudge {
    /// Build `ctx = [one user message: criterion + upstream artifact]`,
    /// `tools = &[]`, `sys = &self.system`, call `self.stream_fn(ctx, sys, &[],
    /// &self.llm)`, drain the stream collecting every
    /// [`LlmStreamEvent::TextDelta`] (`event.rs:6`), and return
    /// [`parse_verdict`] of the concatenation.
    ///
    /// # Errors
    ///
    /// A `LlmStreamEvent::Error` (fatal or not), an empty reply, or an
    /// unparseable one ⇒ `None` (the escalate fallback).
    fn decide<'a>(
        &'a self,
        criterion: &'a str,
        upstream: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Verdict> + Send + 'a>> {
        Box::pin(async move {
            // One user message: the criterion, plus the source node's artifact
            // when it produced one.
            let mut prompt = format!("Criterion: {criterion}");
            if let Some(up) = upstream {
                prompt.push_str("\n\nUpstream artifact:\n");
                prompt.push_str(up);
            }
            let ctx = vec![AgentMessage::user_text(prompt)];
            // Zero tools: the empty slice IS the judge's no-tool contract.
            let mut stream = (self.stream_fn)(&ctx, &self.system, &[], &self.llm);
            let mut text = String::new();
            while let Some(ev) = stream.next().await {
                match ev {
                    LlmStreamEvent::TextDelta(t) => text.push_str(&t),
                    // Any error (fatal or not) → the escalate fallback, never skip.
                    LlmStreamEvent::Error { .. } => return None,
                    _ => {}
                }
            }
            if text.trim().is_empty() {
                return None;
            }
            parse_verdict(&text)
        })
    }
}

/// A scripted [`Judge`] for tests — no live model.
///
/// `#[cfg(test)]` so it never ships. Visible crate-wide under `cfg(test)`, so
/// `scheduler.rs`'s tests can inject it (the W025 §5.5 gate).
#[cfg(test)]
pub(crate) struct FakeJudge {
    /// Verdicts handed out in call order; a call past the end panics so a test
    /// that expects N decisions fails loudly rather than hanging.
    verdicts: std::sync::Mutex<std::collections::VecDeque<Verdict>>,
    /// Every `(criterion, upstream)` seen, so a test can assert the scheduler
    /// passed the right upstream artifact.
    seen: std::sync::Mutex<Vec<(String, Option<String>)>>,
}

#[cfg(test)]
impl FakeJudge {
    /// A judge that returns `verdicts` in order.
    pub(crate) fn new(verdicts: Vec<Verdict>) -> Self {
        Self {
            verdicts: std::sync::Mutex::new(verdicts.into()),
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }
    /// The `(criterion, upstream)` pairs recorded so far.
    pub(crate) fn seen(&self) -> Vec<(String, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Judge for FakeJudge {
    /// Pop the next scripted verdict; record `(criterion, upstream)`.
    fn decide<'a>(
        &'a self,
        criterion: &'a str,
        upstream: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Verdict> + Send + 'a>> {
        self.seen
            .lock()
            .unwrap()
            .push((criterion.to_string(), upstream.map(str::to_string)));
        let verdict = self
            .verdicts
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeJudge: more decide() calls than scripted verdicts");
        Box::pin(async move { verdict })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `VERDICT: TAKE` on its own line → `Some(true)`.
    #[test]
    fn parse_verdict_accepts_a_take() {
        assert_eq!(parse_verdict("VERDICT: TAKE"), Some(true));
    }

    /// `VERDICT: SKIP` on its own line → `Some(false)`.
    #[test]
    fn parse_verdict_accepts_a_skip() {
        assert_eq!(parse_verdict("VERDICT: SKIP"), Some(false));
    }

    /// A legal reply carries a `REASON:` line after the verdict — the verdict
    /// line still wins.
    #[test]
    fn parse_verdict_reads_the_first_verdict_line() {
        assert_eq!(
            parse_verdict("VERDICT: TAKE\nREASON: the diff looks correct"),
            Some(true)
        );
    }

    /// Two verdict lines (a confused model) → the FIRST one decides.
    #[test]
    fn parse_verdict_returns_the_first_of_both_tokens() {
        assert_eq!(parse_verdict("VERDICT: SKIP\nVERDICT: TAKE"), Some(false));
    }

    /// A prose-only reply, an empty reply, and a markdown-broken
    /// `**VERDICT: TAKE**` all → `None` (the escalate fallback), never a skip.
    #[test]
    fn parse_verdict_rejects_prose_empty_and_markdown_wrapping() {
        assert_eq!(parse_verdict("I think it should be taken."), None);
        assert_eq!(parse_verdict(""), None);
        assert_eq!(parse_verdict("   \n  \n"), None);
        // The `**` breaks the `^\s*` anchor.
        assert_eq!(parse_verdict("**VERDICT: TAKE**"), None);
    }

    /// The case-insensitive arm: `verdict: take` → `Some(true)`.
    #[test]
    fn parse_verdict_is_case_insensitive() {
        assert_eq!(parse_verdict("verdict: take"), Some(true));
        assert_eq!(parse_verdict("  Verdict: skip\nREASON: no"), Some(false));
    }

    /// `ModelJudge::new` stores its three pieces verbatim, and `decide` over a
    /// fake `StreamFn` that emits `TextDelta("VERDICT: TAKE")` then `Done`
    /// returns `Some(true)` — no live model. The `stream_fn` records the tool
    /// count it was handed, proving the judge passes ZERO tools.
    #[tokio::test]
    async fn model_judge_builds_a_zero_tool_one_shot() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use wcode_harness::message::StopReason;

        let recorded = Arc::new(AtomicUsize::new(usize::MAX));
        let recorder = recorded.clone();
        let stream_fn: StreamFn = Arc::new(move |_ctx, _sys, tools, _opts| {
            recorder.store(tools.len(), Ordering::SeqCst);
            let events = vec![
                LlmStreamEvent::TextDelta("VERDICT: TAKE".into()),
                LlmStreamEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: None,
                },
            ];
            Box::pin(futures::stream::iter(events))
        });
        let judge = ModelJudge::new(
            LlmOpts {
                model: "m".into(),
                ..Default::default()
            },
            stream_fn,
            JUDGE_SYSTEM_CONTRACT.to_string(),
        );
        assert_eq!(
            judge.decide("did it pass?", Some("the diff")).await,
            Some(true)
        );
        assert_eq!(recorded.load(Ordering::SeqCst), 0, "zero tools by construction");
    }

    /// `FakeJudge` hands out its scripted verdicts in order and records the
    /// `(criterion, upstream)` it saw — the injection seam the scheduler tests use.
    #[tokio::test]
    async fn fake_judge_yields_scripted_verdicts() {
        let judge = FakeJudge::new(vec![Some(true), None]);
        assert_eq!(judge.decide("first?", None).await, Some(true));
        assert_eq!(judge.decide("second?", Some("up")).await, None);
        assert_eq!(
            judge.seen(),
            vec![
                ("first?".to_string(), None),
                ("second?".to_string(), Some("up".to_string())),
            ]
        );
    }
}
