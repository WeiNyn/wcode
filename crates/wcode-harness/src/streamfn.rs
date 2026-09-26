//! LLM stream-function seam + default `rig` (OpenAI-compatible) adapter.
//!
//! This module is the only place rig types appear in the kernel. A
//! [`StreamFn`] turns harness conversation context into a [`LlmStream`] of
//! [`LlmStreamEvent`]s; [`rig_stream_fn`] is the default implementation built
//! on rig's Chat Completions API client.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{Stream, StreamExt};
use rig::client::{CompletionClient, ModelListingClient};
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, FinishReason, ToolDefinition,
};
use rig::http_client::{HeaderMap, HeaderValue};
use rig::message::{
    AssistantContent, Message, Reasoning, ReasoningContent, ToolCall, ToolCallId, ToolFunction,
    UserContent,
};
use rig::providers::openai;
use rig::streaming::{StreamedAssistantContent, StreamingCompletionResponse};

use crate::event::LlmStreamEvent;
use crate::message::{AgentMessage, ContentBlock, StopReason};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LlmEndpoint {
    #[default]
    Chat,
    Responses,
}

#[derive(Clone, Debug, Default)]
pub struct LlmOpts {
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    /// Sampling temperature. No CLI flag surfaces it yet; a library embedder
    /// that builds `LlmOpts` directly can set it.
    pub temperature: Option<f64>,
    pub endpoint: LlmEndpoint,
    /// Stable per-conversation id, sent as `x-opencode-session` for providers
    /// (e.g. OpenCode Go) that route/cache on it. None = header omitted.
    pub session_id: Option<String>,
    /// Free-style reasoning effort. None = send nothing (backends that reject
    /// unknown fields keep working). Some(level) fans out per endpoint in
    /// `build_request()`: Chat forwards any string verbatim under
    /// `reasoning_effort`, while the Responses endpoint validates client-side
    /// against `none|minimal|low|medium|high|xhigh|max` (rig's typed enum) — a
    /// value outside that set fails with a clear error before the request.
    pub effort: Option<String>,
    /// Retry policy for the connect phase (transient failures). See
    /// [`RetryPolicy`].
    pub retry: RetryPolicy,
}

pub type LlmStream = Pin<Box<dyn Stream<Item = LlmStreamEvent> + Send>>;

pub type StreamFn =
    Arc<dyn Fn(&[AgentMessage], &str, &[ToolDefinition], &LlmOpts) -> LlmStream + Send + Sync>;

/// Everything that shapes the underlying rig client: the endpoint, the key, and
/// the routing/session header. `model`, `temperature`, and `effort` ride on the
/// request, not the client, so a mid-conversation model swap still reuses the
/// pooled HTTP connections. (`OPENAI_API_KEY`, the key fallback when `api_key`
/// is None, is read once at build time and treated as constant.)
#[derive(Clone, PartialEq, Eq)]
struct ClientKey {
    base_url: Option<String>,
    api_key: Option<String>,
    session_id: Option<String>,
}

impl ClientKey {
    fn of(opts: &LlmOpts) -> Self {
        Self {
            base_url: opts.base_url.clone(),
            api_key: opts.api_key.clone(),
            session_id: opts.session_id.clone(),
        }
    }
}

/// One-entry client cache. rig's `Client` wraps a pooled `reqwest::Client`, so
/// rebuilding it every turn would drop the pool and re-handshake TLS; this keeps
/// one client alive across turns and rebuilds only when [`ClientKey`] changes.
#[derive(Default)]
struct ClientCache {
    slot: Mutex<Option<(ClientKey, openai::Client)>>,
    #[cfg(test)]
    builds: std::sync::atomic::AtomicUsize,
}

impl ClientCache {
    /// The client for `opts`, reusing the cached one when the key is unchanged.
    fn get(&self, opts: &LlmOpts) -> Result<openai::Client, String> {
        let key = ClientKey::of(opts);
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_key, client)) = slot.as_ref()
            && *cached_key == key
        {
            return Ok(client.clone());
        }
        let client = openai_client(opts)?;
        #[cfg(test)]
        self.builds
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *slot = Some((key, client.clone()));
        Ok(client)
    }
}

/// Default adapter: OpenAI-compatible Chat Completions endpoint via rig.
///
/// The provider client (and its pooled connections) is built once and reused
/// across turns; only an endpoint/key/session change rebuilds it.
pub fn rig_stream_fn() -> StreamFn {
    let cache = ClientCache::default();
    Arc::new(move |messages, system, tools, opts| adapt(&cache, opts, messages, system, tools))
}

/// Build the rig client shared by streaming and model listing, so both hit
/// the same base_url with the same key.
fn openai_client(opts: &LlmOpts) -> Result<rig::providers::openai::Client, String> {
    let key = opts
        .api_key
        .clone()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok());
    let key = match key {
        Some(k) => k,
        None => {
            // Keyless local OpenAI-compatible endpoints (Ollama, llama.cpp,
            // vLLM, ...) accept any bearer token, so a placeholder keeps them
            // usable while a missing key against a real endpoint fails loudly.
            let local = opts.base_url.as_deref().is_some_and(|url| {
                let u = url.to_ascii_lowercase();
                ["http://localhost", "http://127.0.0.1", "http://[::1]"]
                    .iter()
                    .any(|prefix| u.starts_with(prefix))
            });
            if local {
                "wcode-local".to_string()
            } else {
                return Err(
                    "no API key: pass LlmOpts.api_key or set OPENAI_API_KEY (a localhost base_url works keyless)"
                        .to_string(),
                );
            }
        }
    };
    let mut builder = openai::Client::builder()
        .api_key::<rig::client::BearerAuth>(key)
        .http_headers(session_headers(opts));
    if let Some(base_url) = &opts.base_url {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|e| format!("openai client: {e}"))
}

/// Default headers for every provider request: own `User-Agent` (OpenCode Go
/// rejects generic SDK names for routing) plus `x-opencode-session` when set.
fn session_headers(opts: &LlmOpts) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "user-agent",
        HeaderValue::from_static(concat!("wcode/", env!("CARGO_PKG_VERSION"))),
    );
    if let Some(id) = opts.session_id.as_deref()
        && let Ok(value) = HeaderValue::from_str(id)
    {
        headers.insert("x-opencode-session", value);
    }
    headers
}

/// List models via `GET {base_url}/models` using the same client config as
/// streaming. Ids are sorted for stable display. The endpoint only returns
/// id/name metadata — no capability flags — so effort support stays
/// user-managed.
pub async fn list_models(opts: &LlmOpts) -> Result<Vec<String>, String> {
    let client = openai_client(opts)?;
    let list = client.list_models().await.map_err(|e| e.to_string())?;
    let mut ids: Vec<String> = list.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    Ok(ids)
}

fn adapt(
    cache: &ClientCache,
    opts: &LlmOpts,
    messages: &[AgentMessage],
    system: &str,
    tools: &[ToolDefinition],
) -> LlmStream {
    if let Some(message) = invalid_effort(opts) {
        return error_stream(message);
    }
    let client = match cache.get(opts) {
        Ok(client) => client,
        Err(message) => return error_stream(message),
    };
    let request = build_request(messages, system, tools, opts);

    // Both wires yield the same normalized StreamingCompletionResponse, so
    // only model construction branches; the forwarding loop below is shared.
    // Retry the connect (and the first item — rig defers the request into the
    // stream). Once output is forwarded a mid-stream error is surfaced as-is. A
    // fresh model per attempt is cheap — it just wraps the pooled client;
    // `request` is cloned per attempt and `policy` is `Copy`.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LlmStreamEvent>();
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return error_stream("StreamFn requires a tokio runtime".to_string());
    };
    let policy = opts.retry;
    let endpoint = opts.endpoint;
    let model = opts.model.clone();
    runtime.spawn(async move {
        let open = || open_stream(client.clone(), endpoint, &model, request.clone());
        // Retry budget for the whole pre-content window, shared across the
        // connect, the first-item peek (rig defers the request into the stream,
        // so a transport failure surfaces there), and any transient error that
        // slips past the peek before the first content event is forwarded.
        let mut state = RetryState::new(policy.max);
        let (stream, pending) = match connect_with_retry(&policy, &mut state, &open, &tx).await {
            PreContent::Ready { stream, first } => (stream, first),
            PreContent::Cancelled => return,
            PreContent::Failed(error) => {
                let _ = tx.send(LlmStreamEvent::Error {
                    message: error.to_string(),
                    fatal: fatal_class(&error),
                });
                return;
            }
        };
        forward_stream(stream, pending, policy, &open, tx, state).await;
    });
    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|ev| (ev, rx))
    }))
}

/// The forwarding phase of a stream turn: pull items (the buffered `pending`
/// first, then from `stream` under the idle deadline), map them to events, and
/// send them on `tx`. A pre-content stall re-opens via `open` on the shared
/// retry budget (mirroring a transient connect error); a post-content stall (or
/// an exhausted budget) surfaces a non-fatal error and ends the loop.
async fn forward_stream<S, F>(
    mut stream: S,
    mut pending: Option<Result<StreamedAssistantContent, CompletionError>>,
    policy: RetryPolicy,
    open: F,
    tx: tokio::sync::mpsc::UnboundedSender<LlmStreamEvent>,
    mut state: RetryState,
) where
    F: Fn() -> Pin<Box<dyn std::future::Future<Output = Result<S, CompletionError>> + Send>>,
    S: futures::Stream<Item = Result<StreamedAssistantContent, CompletionError>> + Unpin + Send,
{
    let mut errored = false;
    let mut content_forwarded = false;
    let mut done_seen = false;
    loop {
        // The next item: a buffered first item, or one from the stream under the
        // idle deadline. Cancel-safe: a dropped `next_or_stall` drops an
        // in-flight `stream.next()` poll, which is safe to re-enter (the
        // pre-content path re-opens a fresh stream); `biased` with the item arm
        // first so a ready item is never lost.
        let item = match pending.take() {
            Some(item) => item,
            None => tokio::select! {
                biased;
                result = next_or_stall(&mut stream, policy.idle) => match result {
                    // The stream ended. A `Final` record (mapped to `Done`) is the
                    // only clean end: rig emits neither a `Final` nor an `Err` when
                    // the provider SSE body ends early (proxy idle-close / half-close
                    // / EOF), so the normalized stream just ends with `None` — a
                    // truncated turn, not a stop.
                    None => {
                        if done_seen || errored {
                            break;
                        }
                        let error = truncation_error();
                        if !content_forwarded && state.budget > 0 {
                            // Pre-content: retry on the shared budget, exactly like
                            // the idle-stall arm (one retry path, via the helper).
                            match pre_content_retry(&policy, &mut state, &open, &tx, &error).await {
                                PreContent::Ready { stream: s, first } => {
                                    stream = s;
                                    pending = first;
                                    continue;
                                }
                                PreContent::Cancelled => return,
                                PreContent::Failed(e) => {
                                    if tx
                                        .send(LlmStreamEvent::Error {
                                            message: e.to_string(),
                                            fatal: fatal_class(&e),
                                        })
                                        .is_err()
                                    {
                                        return;
                                    }
                                    break;
                                }
                            }
                        } else {
                            // NON-fatal: the loop feeds it back next turn and, if it
                            // persists, ends StopReason::Error.
                            // Chosen Error.message: `truncation_error().to_string()`.
                            if tx
                                .send(LlmStreamEvent::Error {
                                    message: error.to_string(),
                                    fatal: false,
                                })
                                .is_err()
                            {
                                return;
                            }
                            break;
                        }
                    }
                    // One item — the stream's own `Result`.
                    Some(Stall::Item(item)) => item,
                    // The idle deadline fired. Pre-content → a transient retry
                    // on the shared budget (re-open, mirroring a connect error);
                    // post-content (or an exhausted budget) → a NON-fatal error
                    // that ends the turn (a stalled stream will not resume).
                    Some(Stall::Idle) => {
                        let error = stall_error(policy.idle);
                        if !content_forwarded && state.budget > 0 {
                            match pre_content_retry(&policy, &mut state, &open, &tx, &error).await {
                                PreContent::Ready { stream: s, first } => {
                                    stream = s;
                                    pending = first;
                                    continue;
                                }
                                PreContent::Cancelled => return,
                                PreContent::Failed(e) => {
                                    let fatal = fatal_class(&e);
                                    if tx
                                        .send(LlmStreamEvent::Error {
                                            message: e.to_string(),
                                            fatal,
                                        })
                                        .is_err()
                                    {
                                        return;
                                    }
                                    break;
                                }
                            }
                        } else {
                            if tx
                                .send(LlmStreamEvent::Error {
                                    message: error.to_string(),
                                    fatal: false,
                                })
                                .is_err()
                            {
                                return;
                            }
                            break;
                        }
                    }
                },
                _ = tx.closed() => break,
            },
        };
        let events = match item {
            Ok(content) => {
                let is_final = matches!(content, StreamedAssistantContent::Final(_));
                if errored && is_final {
                    continue;
                }
                let events = map_item(content);
                // Replay is only safe before the first forwarded event;
                // once anything reached the consumer a later error cannot
                // be retried (it would duplicate output).
                content_forwarded |= !events.is_empty();
                done_seen |= is_final;
                events
            }
            Err(error) => {
                if !content_forwarded && state.budget > 0 && retryable(&error) {
                    // Nothing forwarded yet: back off and replay the request
                    // from scratch. The re-open shares the same retry budget
                    // and keeps backoff growing monotonically.
                    state.attempts += 1;
                    state.budget -= 1;
                    if !retry_wait(&tx, &policy, state.attempts, &error).await {
                        return;
                    }
                    match connect_with_retry(&policy, &mut state, &open, &tx).await {
                        PreContent::Ready { stream: s, first } => {
                            stream = s;
                            pending = first;
                            continue;
                        }
                        PreContent::Cancelled => return,
                        PreContent::Failed(e) => {
                            errored = true;
                            vec![LlmStreamEvent::Error {
                                message: e.to_string(),
                                fatal: fatal_class(&e),
                            }]
                        }
                    }
                } else {
                    // Final: a hard-fatal class, retries exhausted, or a
                    // mid-stream error after content. The loop decides (via
                    // `fatal`) whether to feed it back or end the run.
                    errored = true;
                    vec![LlmStreamEvent::Error {
                        message: error.to_string(),
                        fatal: fatal_class(&error),
                    }]
                }
            }
        };
        for ev in events {
            if tx.send(ev).is_err() {
                return;
            }
        }
    }
}

fn error_stream(message: String) -> LlmStream {
    // Pre-request failures (invalid effort, client build, missing runtime) are
    // hard-fatal: the request itself can never succeed, so the run must stop.
    Box::pin(futures::stream::iter(vec![LlmStreamEvent::Error {
        message,
        fatal: true,
    }]))
}

/// Retry policy for a stream call: the transient-connect retries plus the stall
/// (idle / time-to-first-token) timeouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt (0 disables retrying).
    pub max: u32,
    /// Base backoff: attempt `n` waits `base * 2^(n-1)`, capped at `cap`.
    pub base: Duration,
    /// Upper bound on a single backoff wait.
    pub cap: Duration,
    /// Time-to-first-token: covers connect + the first stream item. `ZERO` = off.
    pub ttft: Duration,
    /// Inter-item idle: resets on every received item. `ZERO` = off.
    pub idle: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max: 3,
            base: Duration::from_millis(500),
            cap: Duration::from_secs(8),
            ttft: Duration::from_secs(60),
            idle: Duration::from_secs(120),
        }
    }
}

/// One poll of a stream under a stall deadline (see [`next_or_stall`]).
enum Stall<T> {
    /// One item — the stream's OWN item type (for a rig stream, a
    /// `Result<StreamedAssistantContent, CompletionError>`), not an `Option`.
    Item(T),
    /// The deadline fired before an item arrived.
    Idle,
}

/// `stream.next()` under a `deadline`; `ZERO` means "no timeout" and collapses
/// to a plain `stream.next().await`. `biased` toward the item so a ready item
/// wins a tie with an expired deadline. Outer `None` = the stream ended.
async fn next_or_stall<S: Stream + Unpin>(
    stream: &mut S,
    deadline: Duration,
) -> Option<Stall<S::Item>> {
    if deadline.is_zero() {
        return stream.next().await.map(Stall::Item);
    }
    tokio::select! {
        biased;
        item = stream.next() => item.map(Stall::Item),
        _ = tokio::time::sleep(deadline) => Some(Stall::Idle),
    }
}
/// Transient HTTP statuses worth retrying: request timeout/conflict, rate
/// limiting, and the 5xx family. Client errors (400/401/403/404/422) are not.
fn is_transient_status(code: u16) -> bool {
    matches!(code, 408 | 409 | 429 | 500 | 502 | 503 | 504)
}

/// Whether a failed connect is worth retrying. A preserved provider status is
/// authoritative (transient statuses retry, client errors don't); with no
/// status, a transport failure retries and a build/parse error does not.
fn retryable(error: &CompletionError) -> bool {
    if let Some(status) = error.provider_response_status() {
        return is_transient_status(status.as_u16());
    }
    match error {
        // Transport failure with no status (connect refused / reset / timeout).
        CompletionError::HttpError(_) => true,
        // rig's OpenAI path folds some transport failures into a
        // `ProviderError(String)` (reqwest's "error sending request …"); retry
        // those, but not provider rejections that share the variant.
        CompletionError::ProviderError(message) => is_transient_message(message),
        _ => false,
    }
}

/// Heuristic for transport failures that arrive as `ProviderError` strings:
/// connect/DNS/timeout/reset wording, not provider rejections.
fn is_transient_message(message: &str) -> bool {
    const NEEDLES: [&str; 10] = [
        "error sending request",
        "error trying to connect",
        "connection refused",
        "connection reset",
        "connection closed",
        "timed out",
        "timeout",
        "unreachable",
        "dns error",
        "temporary failure",
    ];
    let m = message.to_ascii_lowercase();
    NEEDLES.iter().any(|needle| m.contains(needle))
}

/// Exponential backoff for `attempt` (1-based), capped, with jitter in
/// [75%, 100%] so repeated retries don't align on the same instant.
fn backoff(attempt: u32, base: Duration, cap: Duration) -> Duration {
    let shift = attempt.saturating_sub(1).min(16);
    let capped = base.saturating_mul(1u32 << shift).min(cap);
    let millis = capped.as_millis() as u64;
    if millis <= 1 {
        return capped;
    }
    let jitter = clock_nanos() % (millis / 4 + 1);
    Duration::from_millis(millis - jitter)
}

/// A cheap clock seed for backoff jitter.
fn clock_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0)
}

/// Build one connect attempt: a fresh future per call, so the handshake can be
/// retried. Once the response is in hand a mid-stream error is not retried.
fn open_stream(
    client: rig::providers::openai::Client,
    endpoint: LlmEndpoint,
    model: &str,
    request: CompletionRequest,
) -> Pin<Box<dyn std::future::Future<Output = Result<StreamingCompletionResponse, CompletionError>> + Send>> {
    match endpoint {
        LlmEndpoint::Chat => {
            let model = client.completions_api().completion_model(model.to_string());
            Box::pin(async move { model.stream(request).await })
        }
        LlmEndpoint::Responses => {
            let model = client.completion_model(model.to_string());
            Box::pin(async move { model.stream(request).await })
        }
    }
}

/// Outcome of the pre-content phase of a stream turn: a stream with its first
/// (buffered) item — which may itself be an error, in which case the caller
/// classifies it — or a final failure.
enum PreContent<S> {
    Ready {
        stream: S,
        first: Option<Result<StreamedAssistantContent, CompletionError>>,
    },
    /// Final failure: not in the transient class, or retries exhausted.
    Failed(CompletionError),
    /// The consumer dropped `tx` during a backoff wait (cancellation).
    Cancelled,
}

/// Shared retry bookkeeping for one turn's pre-content window: how many retries
/// remain across the connect/peek and any later pre-content errors, plus how
/// many failed attempts have accumulated so backoff keeps growing monotonically.
#[derive(Clone, Copy)]
struct RetryState {
    budget: u32,
    attempts: u32,
}

impl RetryState {
    fn new(max: u32) -> Self {
        RetryState {
            budget: max,
            attempts: 0,
        }
    }
}

/// Whether a failed stream call is of a hard-fatal class that retrying or
/// re-feeding cannot fix: hard client statuses (400/401/403/404/422) and
/// build/parse failures (Json/Request/Response/Url). Transient-class errors
/// (see [`retryable`]) may exhaust their retries — those surface with
/// `fatal: false` so the loop can feed the failure back to the model instead
/// of ending the run.
fn fatal_class(error: &CompletionError) -> bool {
    !retryable(error)
}

/// Open a stream, retrying transient failures on the connect and on anything
/// the peek surfaces. rig's OpenAI path defers the HTTP request into the stream,
/// so a connection failure surfaces on the first poll; retrying only the
/// connect would miss it. `open` is re-invoked per attempt (a fresh future).
/// The [`RetryState`] budget is shared with the forwarding loop, so a transient
/// error that slips past the peek (still before any content was forwarded) is
/// likewise retried at most `policy.max` times total per turn. Once any content
/// has been forwarded a mid-stream error is surfaced as-is — retrying would
/// duplicate output. Each backoff is abandoned if the consumer drops `tx`.
/// A synthesized provider error for a stream stall. The message carries a
/// needle `is_transient_message` already matches ("timed out"), so `retryable()`
/// is `true` and `fatal_class()` is `false` (B1) — a stall rides the existing
/// transient-retry / feed-back paths unchanged.
fn stall_error(d: Duration) -> CompletionError {
    CompletionError::ProviderError(format!("stream stalled: no item within {d:?} (timed out)"))
}
/// A synthesized stream-truncation failure, shaped like [`stall_error`]. The
/// stream ended without its terminal record: rig emits neither a `Final` nor an
/// `Err` when the provider SSE body ends early (no `[DONE]`/finish_reason), so
/// the normalized stream simply ends with `None`. Not necessarily `retryable()`
/// — the pre-content branch re-opens unconditionally — the message only feeds
/// `retry_wait`'s notice text and the surfaced `Error.message`.
fn truncation_error() -> CompletionError {
    CompletionError::ProviderError(
        "stream ended before its terminal record (truncated)".to_string(),
    )
}

async fn connect_with_retry<S, F>(
    policy: &RetryPolicy,
    state: &mut RetryState,
    mut open: F,
    tx: &tokio::sync::mpsc::UnboundedSender<LlmStreamEvent>,
) -> PreContent<S>
where
    F: FnMut() -> Pin<Box<dyn std::future::Future<Output = Result<S, CompletionError>> + Send>>,
    S: futures::Stream<Item = Result<StreamedAssistantContent, CompletionError>> + Unpin + Send,
{
    loop {
        state.attempts += 1;
        // Race the connect against `ttft` — zero-aware: `ZERO` disables the
        // deadline and awaits the connect directly (never `sleep(ZERO)`, which
        // would fire at once). A connect stall yields a transient `stall_error`.
        let opened = if policy.ttft.is_zero() {
            Ok(open().await)
        } else {
            tokio::select! {
                biased;
                r = open() => Ok(r),
                _ = tokio::time::sleep(policy.ttft) => Err(()),
            }
        };
        let mut stream = match opened {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                if state.budget > 0 && retryable(&error) {
                    state.budget -= 1;
                    if !retry_wait(tx, policy, state.attempts, &error).await {
                        return PreContent::Cancelled;
                    }
                    continue;
                }
                return PreContent::Failed(error);
            }
            Err(()) => {
                let error = stall_error(policy.ttft);
                if state.budget > 0 {
                    state.budget -= 1;
                    if !retry_wait(tx, policy, state.attempts, &error).await {
                        return PreContent::Cancelled;
                    }
                    continue;
                }
                return PreContent::Failed(error);
            }
        };
        // Peek the first item under the SAME `ttft`: rig defers the request into
        // the stream, so a transport failure (or a stall) surfaces here.
        match next_or_stall(&mut stream, policy.ttft).await {
            None => return PreContent::Ready { stream, first: None },
            Some(Stall::Item(Err(error))) => {
                if state.budget > 0 && retryable(&error) {
                    state.budget -= 1;
                    if !retry_wait(tx, policy, state.attempts, &error).await {
                        return PreContent::Cancelled;
                    }
                    continue;
                }
                // A first-item error that cannot be retried is still an error
                // item: hand it to the forwarding loop, which classifies it
                // (fatal vs feed-back) like any other.
                return PreContent::Ready {
                    stream,
                    first: Some(Err(error)),
                };
            }
            Some(Stall::Item(Ok(content))) => {
                return PreContent::Ready {
                    stream,
                    first: Some(Ok(content)),
                };
            }
            Some(Stall::Idle) => {
                let error = stall_error(policy.ttft);
                if state.budget > 0 {
                    state.budget -= 1;
                    if !retry_wait(tx, policy, state.attempts, &error).await {
                        return PreContent::Cancelled;
                    }
                    continue;
                }
                return PreContent::Failed(error);
            }
        }
    }
}

/// One pre-content retry shared by the idle-stall arm and the truncation arm:
/// spend one attempt + one budget unit on the shared [`RetryState`], back off
/// (abandoning if the consumer dropped `tx`), then re-open via
/// [`connect_with_retry`] with the SAME state so the budget stays monotone.
/// Returns the same [`PreContent<S>`] the direct connect path returns, so both
/// callers match identically — no new enum, no duplicated match.
async fn pre_content_retry<S, F>(
    policy: &RetryPolicy,
    state: &mut RetryState,
    open: &F,
    tx: &tokio::sync::mpsc::UnboundedSender<LlmStreamEvent>,
    error: &CompletionError,
) -> PreContent<S>
where
    F: Fn() -> Pin<Box<dyn std::future::Future<Output = Result<S, CompletionError>> + Send>>,
    S: futures::Stream<Item = Result<StreamedAssistantContent, CompletionError>> + Unpin + Send,
{
    state.attempts += 1;
    state.budget -= 1;
    if !retry_wait(tx, policy, state.attempts, error).await {
        return PreContent::Cancelled;
    }
    connect_with_retry(policy, state, open, tx).await
}

/// The server's `Retry-After` header (integer delta-seconds), when the failed
/// response preserved one. `None` when there is no preserved response, no
/// header, a non-UTF-8 value, or a value that is not a non-negative integer.
///
/// SCOPE: integer delta-seconds ONLY. The HTTP-date form of `Retry-After`
/// (RFC 9110 `Retry-After = HTTP-date / delay-seconds`) is deliberately out of
/// scope: a date would need wall-clock parsing against the loop's monotonic
/// `tokio::time` deadline, and a subtly wrong date is worse than ignoring it —
/// a provider that sends one simply gets the normal backoff.
fn retry_after(error: &CompletionError) -> Option<Duration> {
    // `provider_response_headers()` already unwraps both the
    // `HttpError(InvalidStatusCodeWithDetails)` and the `ProviderResponse`
    // variants, so this reads either.
    let raw = error.provider_response_headers()?.get("retry-after")?;
    // A non-UTF-8 header value is ignored, not an error.
    let seconds: u64 = raw.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds))
}

/// Emit a retry notice and sleep the backoff. Returns false if the consumer
/// dropped `tx` (cancellation) during the wait.
async fn retry_wait(
    tx: &tokio::sync::mpsc::UnboundedSender<LlmStreamEvent>,
    policy: &RetryPolicy,
    attempt: u32,
    error: &CompletionError,
) -> bool {
    // A server-supplied `Retry-After` is authoritative and wins over the local
    // backoff, but it is clamped by the existing `policy.cap` so a hostile or
    // buggy `Retry-After: 86400` cannot park the turn for a day.
    let delay = retry_after(error)
        .map(|d| d.min(policy.cap))
        .unwrap_or_else(|| backoff(attempt, policy.base, policy.cap));
    let _ = tx.send(LlmStreamEvent::Retrying {
        attempt,
        max: policy.max,
        reason: error.to_string(),
    });
    tokio::select! {
        biased;
        _ = tx.closed() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

fn build_request(
    messages: &[AgentMessage],
    system: &str,
    tools: &[ToolDefinition],
    opts: &LlmOpts,
) -> CompletionRequest {
    let mut chat_history = Vec::with_capacity(messages.len() + 1);
    if !system.is_empty() {
        chat_history.push(Message::system(system));
    }
    chat_history.extend(messages.iter().filter_map(to_rig_message));
    CompletionRequest {
        model: None,
        // System instructions ride as the leading Message::System, the
        // canonical form rig's own request builder funnels `preamble` into.
        preamble: None,
        chat_history,
        documents: Vec::new(),
        tools: tools.to_vec(),
        temperature: opts.temperature,
        max_tokens: None,
        tool_choice: None,
        additional_params: reasoning_params(opts),
        output_schema: None,
        record_telemetry_content: false,
    }
}

/// Effort levels accepted by the Responses wire. rig deserializes
/// `reasoning.effort` into a snake_case enum client-side (`ReasoningEffort`),
/// so any other string fails with an opaque RequestError at send time;
/// `invalid_effort` surfaces a clear error instead. Chat forwards any string.
const RESPONSES_EFFORT_LEVELS: &[&str] =
    &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// `Some(message)` when `opts.effort` cannot cross the configured wire: the
/// Responses endpoint validates effort client-side against the enum above,
/// while Chat forwards any string to the provider.
fn invalid_effort(opts: &LlmOpts) -> Option<String> {
    if opts.endpoint == LlmEndpoint::Responses
        && let Some(effort) = opts.effort.as_deref()
        && !RESPONSES_EFFORT_LEVELS.contains(&effort)
    {
        return Some(format!(
            "effort `{effort}` is not valid for the Responses endpoint; use one of: {}",
            RESPONSES_EFFORT_LEVELS.join(", ")
        ));
    }
    None
}

/// Reasoning/effort passthrough, fanned out per wire shape.
///
/// Chat: `opts.effort` alone → `{"reasoning_effort": effort}`; None sends
/// nothing (unchanged), so backends that reject unknown fields keep working.
///
/// Responses: ALWAYS emits `{"reasoning": {"summary": "auto"}}`, merging
/// `"effort"` in when `opts.effort` is `Some`. The summary is what makes
/// reasoning DISPLAYABLE: rig auto-injects `include:["reasoning.encrypted_content"]`
/// whenever `additional_params` carries `reasoning` (rig-core-0.42.0
/// responses_api/mod.rs:1461 — `if additional_parameters.reasoning.is_some()`),
/// and `ReasoningSummaryLevel::Auto` serializes to snake_case "auto"
/// (responses_api/mod.rs:2133). Without a summary the model emits an
/// encrypted-only reasoning item whose displayable text is empty (see the
/// `map_item` guard below).
///
/// `invalid_effort` still validates the level client-side before this runs.
fn reasoning_params(opts: &LlmOpts) -> Option<serde_json::Value> {
    match opts.endpoint {
        LlmEndpoint::Chat => opts
            .effort
            .as_ref()
            .map(|effort| serde_json::json!({ "reasoning_effort": effort })),
        LlmEndpoint::Responses => {
            let mut reasoning = serde_json::Map::new();
            reasoning.insert("summary".into(), serde_json::json!("auto"));
            if let Some(effort) = &opts.effort {
                reasoning.insert("effort".into(), serde_json::json!(effort));
            }
            Some(serde_json::json!({ "reasoning": reasoning }))
        }
    }
}

/// Prefix applied to a failed tool's output *on the wire only*. The chat wire
/// has no structured error flag for tool results (rig's `ToolResult` only
/// carries text/content), so a failed tool's output must be annotated in the
/// text the model sees — otherwise a tool failure looks exactly like a
/// success. The UI and the JSONL session keep the raw output; only the wire
/// copy read by the LLM is marked.
const TOOL_OUTPUT_ERROR_PREFIX: &str = "ERROR: ";

fn to_rig_message(m: &AgentMessage) -> Option<Message> {
    match m {
        AgentMessage::User { content } => {
            let content: Vec<UserContent> = content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(UserContent::text(text.clone())),
                    _ => None,
                })
                .collect();
            (!content.is_empty()).then_some(Message::User { content })
        }
        AgentMessage::Assistant { content, .. } => {
            let content: Vec<AssistantContent> = content
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => AssistantContent::text(text.clone()),
                    ContentBlock::Thinking { text } => {
                        AssistantContent::Reasoning(Reasoning::new(text))
                    }
                    ContentBlock::ToolCall {
                        id,
                        name,
                        arguments,
                    } => AssistantContent::ToolCall(ToolCall::new(
                        ToolCallId::new_or_mint(id.clone()),
                        ToolFunction::new(name.clone(), arguments.clone()),
                    )),
                })
                .collect();
            (!content.is_empty()).then_some(Message::Assistant { id: None, content })
        }
        AgentMessage::ToolResult {
            tool_call_id,
            name,
            output,
            is_error,
        } => {
            // The wire has no structured error flag for tool results, so mark
            // failures in the text the model sees. The UI and the session keep
            // the raw output — only this wire copy is prefixed.
            let output = if *is_error {
                format!("{TOOL_OUTPUT_ERROR_PREFIX}{output}")
            } else {
                output.clone()
            };
            Some(Message::tool_result(
                tool_call_id.clone(),
                name.clone(),
                output,
            ))
        }
    }
}

fn map_item(item: StreamedAssistantContent) -> Vec<LlmStreamEvent> {
    match item {
        StreamedAssistantContent::Text(text) => vec![LlmStreamEvent::TextDelta(text.text)],
        StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
            vec![LlmStreamEvent::ThinkingDelta(reasoning)]
        }
        // A complete reasoning block supersedes the streamed deltas: the Responses
        // wire restates the whole reasoning item after the deltas. Dead on
        // chat-completions (deltas only) but LIVE on Responses — it must not be
        // dropped.
        StreamedAssistantContent::Reasoning { reasoning, .. } => {
            // Replacement semantics: the complete block supersedes the
            // accumulated deltas (the loop drops prior thinking on this event).
            let text = reasoning_text(&reasoning);
            if text.is_empty() {
                // Encrypted-only terminal item: no displayable text. Emitting
                // ThinkingReplace("") would reach `loop_.rs:replace_thinking`,
                // which retains-out ALL accumulated thinking and pushes an empty
                // block — erasing the streamed ThinkingDeltas. Yield nothing.
                vec![]
            } else {
                vec![LlmStreamEvent::ThinkingReplace(text)]
            }
        }
        // Deltas are dropped: rig's streaming fold accumulates every tool-call
        // fragment and emits the complete `ToolCall` on `ToolInputEnd` (both
        // chat-completions and responses wires do this), so the reassembled
        // call always follows. A provider that ends input without closing it
        // loses the call inside rig itself; the loop surfaces that as an error
        // rather than silently treating the turn as done.
        StreamedAssistantContent::ToolCallDelta { .. } => vec![],
        StreamedAssistantContent::ToolCall { tool_call, .. } => vec![LlmStreamEvent::ToolCall {
            id: tool_call.id.as_str().to_string(),
            name: tool_call.function.name,
            arguments: tool_call.function.arguments,
        }],
        StreamedAssistantContent::Final(final_record) => {
            vec![LlmStreamEvent::Done {
                stop_reason: final_record
                    .finish_reason
                    .map(finish_reason)
                    .unwrap_or(StopReason::Stop),
                usage: Some(convert_usage(final_record.usage)),
            }]
        }
        StreamedAssistantContent::Unknown(_) => vec![],
    }
}

fn finish_reason(reason: FinishReason) -> StopReason {
    match reason {
        FinishReason::Stop => StopReason::Stop,
        FinishReason::Length => StopReason::Length,
        FinishReason::ToolCalls => StopReason::ToolUse,
        FinishReason::ContentFilter | FinishReason::Other(_) => StopReason::Stop,
    }
}

fn convert_usage(usage: rig::completion::Usage) -> crate::message::Usage {
    crate::message::Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        // Zero is rig's documented sentinel for "not reported".
        cache_read_tokens: (usage.cached_input_tokens > 0).then_some(usage.cached_input_tokens),
        cache_write_tokens: (usage.cache_creation_input_tokens > 0)
            .then_some(usage.cache_creation_input_tokens),
    }
}

fn reasoning_text(reasoning: &Reasoning) -> String {
    reasoning
        .content
        .iter()
        .filter_map(|block| match block {
            ReasoningContent::Text { text, .. } => Some(text.as_str()),
            ReasoningContent::Summary(summary) => Some(summary.as_str()),
            ReasoningContent::Redacted { data } => Some(data.as_str()),
            ReasoningContent::Encrypted(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::Usage as RigUsage;
    use rig::message::{ToolResultContent, UserContent};
    use rig::streaming::{RawStreamingToolCall, StreamFinal, ToolCallDeltaContent};
    use serde_json::json;

    fn tool_call_item() -> StreamedAssistantContent {
        let raw =
            RawStreamingToolCall::new("call_1".to_string(), "run".to_string(), json!({"x": 1}));
        StreamedAssistantContent::ToolCall {
            tool_call: raw.into(),
            internal_call_id: "ic1".to_string(),
        }
    }

    fn final_item(finish_reason: Option<FinishReason>) -> StreamedAssistantContent {
        let usage = RigUsage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cached_input_tokens: 3,
            cache_creation_input_tokens: 2,
            ..RigUsage::default()
        };
        let final_record = StreamFinal::new("test", usage);
        StreamedAssistantContent::Final(match finish_reason {
            Some(reason) => final_record.with_finish_reason(reason),
            None => final_record,
        })
    }

    #[test]
    fn text_item_maps_to_text_delta() {
        let events = map_item(StreamedAssistantContent::Text(rig::message::Text::new(
            "hi",
        )));
        assert_eq!(events, vec![LlmStreamEvent::TextDelta("hi".to_string())]);
    }

    #[test]
    fn reasoning_delta_maps_to_thinking_delta() {
        let events = map_item(StreamedAssistantContent::ReasoningDelta {
            id: "r1".to_string(),
            provider_id: None,
            reasoning: "hmm".to_string(),
        });
        assert_eq!(
            events,
            vec![LlmStreamEvent::ThinkingDelta("hmm".to_string())]
        );
    }

    #[test]
    fn reasoning_block_replaces_with_full_text() {
        let events = map_item(StreamedAssistantContent::Reasoning {
            reasoning: Reasoning::new("full thought"),
            id: "r1".to_string(),
        });
        assert_eq!(
            events,
            vec![LlmStreamEvent::ThinkingReplace("full thought".to_string())]
        );
    }

    #[test]
    fn reasoning_text_joins_summary_text_and_redacted_drops_encrypted() {
        // Exactly rig's `Reasoning::display_text()` contract: Summary | Text |
        // Redacted join with "\n"; Encrypted is skipped.
        let reasoning = Reasoning {
            id: None,
            content: vec![
                ReasoningContent::Summary("a".to_string()),
                ReasoningContent::Text {
                    text: "b".to_string(),
                    signature: None,
                },
                ReasoningContent::Redacted {
                    data: "c".to_string(),
                },
                ReasoningContent::Encrypted("opaque".to_string()),
            ],
        };
        assert_eq!(reasoning_text(&reasoning), "a\nb\nc");
    }

    #[test]
    fn reasoning_text_summary_only_is_the_summary() {
        let reasoning = Reasoning::summaries(vec!["just a summary".to_string()]);
        assert_eq!(reasoning_text(&reasoning), "just a summary");
    }

    #[test]
    fn reasoning_text_encrypted_only_is_empty() {
        let reasoning = Reasoning::encrypted("opaque");
        assert_eq!(reasoning_text(&reasoning), "");
    }

    #[test]
    fn summary_reasoning_block_replaces_with_full_text() {
        let events = map_item(StreamedAssistantContent::Reasoning {
            reasoning: Reasoning::summaries(vec!["a summary".to_string()]),
            id: "rs_1".to_string(),
        });
        assert_eq!(
            events,
            vec![LlmStreamEvent::ThinkingReplace("a summary".to_string())]
        );
    }

    #[test]
    fn encrypted_only_reasoning_block_emits_no_event() {
        // An encrypted-only terminal item has no displayable text; emitting
        // ThinkingReplace("") would erase the streamed ThinkingDeltas downstream
        // (see the `map_item` guard).
        let events = map_item(StreamedAssistantContent::Reasoning {
            reasoning: Reasoning::encrypted("opaque"),
            id: "rs_1".to_string(),
        });
        assert!(events.is_empty(), "got: {events:?}");
    }

    #[test]
    fn tool_call_delta_ignored() {
        for content in [
            ToolCallDeltaContent::Name("run".to_string()),
            ToolCallDeltaContent::Delta("{\"x".to_string()),
        ] {
            let events = map_item(StreamedAssistantContent::ToolCallDelta {
                internal_call_id: "ic1".to_string(),
                content,
            });
            assert!(events.is_empty());
        }
    }

    #[test]
    fn tool_call_maps_to_call_event() {
        let events = map_item(tool_call_item());
        assert_eq!(
            events,
            vec![LlmStreamEvent::ToolCall {
                id: "call_1".to_string(),
                name: "run".to_string(),
                arguments: json!({"x": 1}),
            }]
        );
    }

    #[test]
    fn final_maps_finish_reason_and_usage() {
        let cases = [
            (FinishReason::Stop, StopReason::Stop),
            (FinishReason::Length, StopReason::Length),
            (FinishReason::ToolCalls, StopReason::ToolUse),
            (FinishReason::ContentFilter, StopReason::Stop),
            (FinishReason::Other("weird".to_string()), StopReason::Stop),
        ];
        for (reason, expected) in cases {
            let events = map_item(final_item(Some(reason)));
            assert_eq!(
                events,
                vec![LlmStreamEvent::Done {
                    stop_reason: expected,
                    usage: Some(crate::message::Usage {
                        input_tokens: 10,
                        output_tokens: 5,
                        cache_read_tokens: Some(3),
                        cache_write_tokens: Some(2),
                    }),
                }],
                "wrong mapping for {expected:?}"
            );
        }
    }

    #[test]
    fn final_without_finish_reason_defaults_to_stop() {
        let events = map_item(final_item(None));
        match &events[..] {
            [LlmStreamEvent::Done { stop_reason, .. }] => {
                assert_eq!(*stop_reason, StopReason::Stop)
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn unknown_item_ignored() {
        let events = map_item(StreamedAssistantContent::Unknown(
            rig::streaming::UnknownPayload::new(json!({"mystery": 1})),
        ));
        assert!(events.is_empty());
    }

    #[test]
    fn build_request_round_trips_context() {
        let opts = LlmOpts {
            model: "m1".to_string(),
            base_url: None,
            api_key: None,
            temperature: Some(0.5),
            ..LlmOpts::default()
        };
        let tools = vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: "Weather lookup".to_string(),
            parameters: json!({"type": "object"}),
        }];
        let messages = vec![
            AgentMessage::user_text("What is the weather?"),
            AgentMessage::Assistant {
                content: vec![
                    ContentBlock::Thinking { text: "hmm".into() },
                    ContentBlock::ToolCall {
                        id: "t1".into(),
                        name: "get_weather".into(),
                        arguments: json!({"city": "Tokyo"}),
                    },
                    ContentBlock::Text {
                        text: "Let me check.".into(),
                    },
                ],
                stop_reason: StopReason::ToolUse,
                usage: None,
                model: None,
            },
            AgentMessage::ToolResult {
                tool_call_id: "t1".into(),
                name: "get_weather".into(),
                output: "sunny".into(),
                is_error: false,
            },
        ];

        let request = build_request(&messages, "You are terse.", &tools, &opts);

        assert!(request.preamble.is_none());
        assert_eq!(request.temperature, Some(0.5));
        assert_eq!(request.tools.len(), 1);
        assert_eq!(request.tools[0].name, "get_weather");

        let [
            Message::System { content: system },
            Message::User { content: user },
            Message::Assistant {
                content: assistant, ..
            },
            Message::User {
                content: tool_result,
            },
        ] = &request.chat_history[..]
        else {
            panic!("unexpected chat_history shape: {:?}", request.chat_history);
        };
        assert_eq!(system, "You are terse.");
        assert!(
            matches!(user.as_slice(), [UserContent::Text(t)] if t.text == "What is the weather?")
        );
        assert_eq!(assistant.len(), 3);
        assert!(
            matches!(&assistant[0], AssistantContent::Reasoning(r) if reasoning_text(r) == "hmm")
        );
        assert!(matches!(
            &assistant[1],
            AssistantContent::ToolCall(call)
                if call.id.as_str() == "t1"
                    && call.function.name == "get_weather"
                    && call.function.arguments == json!({"city": "Tokyo"})
        ));
        assert!(matches!(&assistant[2], AssistantContent::Text(t) if t.text == "Let me check."));
        assert!(matches!(
            tool_result.as_slice(),
            [UserContent::ToolResult(r)] if r.call.as_str() == "t1" && r.name == "get_weather"
        ));
    }

    #[test]
    fn tool_result_error_is_annotated_on_the_wire_only() {
        let err = AgentMessage::ToolResult {
            tool_call_id: "t1".into(),
            name: "echo".into(),
            output: "boom".into(),
            is_error: true,
        };
        let ok = AgentMessage::ToolResult {
            tool_call_id: "t1".into(),
            name: "echo".into(),
            output: "boom".into(),
            is_error: false,
        };

        fn wire_text(m: &AgentMessage) -> String {
            match to_rig_message(m) {
                Some(Message::User { content }) => match &content[..] {
                    [UserContent::ToolResult(r)] => match &r.content[..] {
                        [ToolResultContent::Text(t)] => t.text.clone(),
                        other => panic!("unexpected tool result content: {other:?}"),
                    },
                    other => panic!("unexpected user content: {other:?}"),
                },
                other => panic!("unexpected wire message: {other:?}"),
            }
        }

        // Success passes through byte-for-byte; a failed tool is marked.
        assert_eq!(wire_text(&ok), "boom");
        assert_eq!(wire_text(&err), "ERROR: boom");
        // The stored message itself is untouched — the flag is only consumed
        // at the wire seam.
        match err {
            AgentMessage::ToolResult {
                output, is_error, ..
            } => {
                assert_eq!(output, "boom");
                assert!(is_error);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn endpoint_defaults_to_chat() {
        let opts = LlmOpts {
            model: "m1".to_string(),
            base_url: None,
            api_key: None,
            temperature: None,
            endpoint: LlmEndpoint::Chat,
            effort: None,
            session_id: None,
            retry: RetryPolicy::default(),
        };
        assert_eq!(opts.endpoint, LlmEndpoint::Chat);
        assert_eq!(LlmOpts::default().endpoint, LlmEndpoint::Chat);
    }

    #[test]
    fn no_effort_sends_no_additional_params() {
        let opts = LlmOpts::default();
        let request = build_request(&[], "sys", &[], &opts);
        assert_eq!(request.additional_params, None);
    }

    #[test]
    fn chat_effort_maps_to_reasoning_effort_param() {
        let opts = LlmOpts {
            effort: Some("high".into()),
            endpoint: LlmEndpoint::Chat,
            ..LlmOpts::default()
        };
        let request = build_request(&[], "sys", &[], &opts);
        assert_eq!(
            request.additional_params,
            Some(json!({ "reasoning_effort": "high" }))
        );
    }

    #[test]
    fn responses_effort_maps_to_reasoning_object() {
        let opts = LlmOpts {
            effort: Some("xhigh".into()),
            endpoint: LlmEndpoint::Responses,
            ..LlmOpts::default()
        };
        let request = build_request(&[], "sys", &[], &opts);
        assert_eq!(
            request.additional_params,
            Some(json!({ "reasoning": { "effort": "xhigh", "summary": "auto" } }))
        );
    }

    #[test]
    fn responses_always_requests_a_reasoning_summary() {
        // The case that was invisible before this change: with NO effort the
        // Responses wire must still ask for a reasoning summary — otherwise the
        // model emits an encrypted-only item whose displayable text is empty.
        let opts = LlmOpts {
            endpoint: LlmEndpoint::Responses,
            effort: None,
            ..LlmOpts::default()
        };
        let request = build_request(&[], "sys", &[], &opts);
        assert_eq!(
            request.additional_params,
            Some(json!({ "reasoning": { "summary": "auto" } }))
        );
    }

    #[test]
    fn chat_effort_is_verbatim_passthrough() {
        // Free-style on the Chat wire: any string (including provider
        // dialects) is forwarded verbatim under `reasoning_effort`.
        for level in ["low", "medium", "my-custom-level", ""] {
            let opts = LlmOpts {
                effort: Some(level.into()),
                endpoint: LlmEndpoint::Chat,
                ..LlmOpts::default()
            };
            let request = build_request(&[], "sys", &[], &opts);
            assert_eq!(
                request.additional_params,
                Some(json!({ "reasoning_effort": level })),
                "level {level:?} must pass through verbatim on Chat"
            );
        }
    }

    #[test]
    fn responses_effort_validation() {
        // Known snake_case levels pass; anything else is rejected with a clear
        // message (rig would otherwise fail the request with an opaque error).
        for level in ["none", "minimal", "low", "medium", "high", "xhigh", "max"] {
            let opts = LlmOpts {
                effort: Some(level.into()),
                endpoint: LlmEndpoint::Responses,
                ..LlmOpts::default()
            };
            assert_eq!(invalid_effort(&opts), None, "level {level} must be valid");
        }
        let opts = LlmOpts {
            effort: Some("deep".into()),
            endpoint: LlmEndpoint::Responses,
            ..LlmOpts::default()
        };
        let msg = invalid_effort(&opts).expect("unknown level must be rejected");
        assert!(msg.contains("not valid"), "got: {msg}");
        assert!(
            msg.contains("xhigh"),
            "message must list valid levels: {msg}"
        );
        // Chat stays verbatim even for a value Responses would reject.
        let opts = LlmOpts {
            effort: Some("deep".into()),
            endpoint: LlmEndpoint::Chat,
            ..LlmOpts::default()
        };
        assert_eq!(invalid_effort(&opts), None);
    }

    #[test]
    fn client_sends_session_header_and_user_agent() {
        let opts = LlmOpts {
            api_key: Some("test-key".to_string()),
            session_id: Some("sess-123".to_string()),
            ..LlmOpts::default()
        };
        let headers = openai_client(&opts)
            .expect("client builds")
            .headers()
            .clone();
        assert_eq!(
            headers.get("x-opencode-session").expect("session header"),
            "sess-123"
        );
        assert!(headers.contains_key("user-agent"), "got: {headers:?}");
    }

    #[test]
    fn client_omits_session_header_when_unset() {
        let opts = LlmOpts {
            api_key: Some("test-key".to_string()),
            ..LlmOpts::default()
        };
        let headers = openai_client(&opts)
            .expect("client builds")
            .headers()
            .clone();
        assert!(!headers.contains_key("x-opencode-session"));
        assert!(headers.contains_key("user-agent"), "got: {headers:?}");
    }

    #[test]
    fn localhost_base_url_builds_without_key() {
        // Keyless local endpoints (Ollama, llama.cpp, ...) should work with no
        // configured key via a placeholder bearer token.
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:8080/v1",
            "http://[::1]:8080/v1",
        ] {
            let opts = LlmOpts {
                base_url: Some(url.to_string()),
                api_key: None,
                ..LlmOpts::default()
            };
            assert!(
                openai_client(&opts).is_ok(),
                "keyless {url} must build a client"
            );
        }
    }

    #[test]
    fn remote_base_url_without_key_is_error() {
        let opts = LlmOpts {
            base_url: Some("https://api.example.com/v1".to_string()),
            api_key: None,
            ..LlmOpts::default()
        };
        let err = openai_client(&opts).unwrap_err();
        assert!(err.contains("no API key"), "got: {err}");
    }

    /// Unreachable endpoint: list_models surfaces a string error, no panic.
    #[tokio::test]
    async fn list_models_unreachable_is_error() {
        let opts = LlmOpts {
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            api_key: Some("test-key".to_string()),
            ..LlmOpts::default()
        };
        let err = tokio::time::timeout(std::time::Duration::from_secs(10), list_models(&opts))
            .await
            .expect("list terminates")
            .expect_err("unreachable host must error");
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn list_models_without_key_is_error() {
        let opts = LlmOpts::default();
        let err = list_models(&opts).await.expect_err("no key must error");
        assert!(err.contains("no API key"), "got: {err}");
    }

    #[test]
    fn client_cache_reuses_until_key_changes() {
        let opts = LlmOpts {
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            api_key: Some("k".to_string()),
            session_id: Some("s1".to_string()),
            ..LlmOpts::default()
        };
        let cache = ClientCache::default();
        let builds = || cache.builds.load(std::sync::atomic::Ordering::SeqCst);

        // Identical opts reuse the cached client (the whole point: one pool/TLS).
        cache.get(&opts).expect("builds");
        cache.get(&opts).expect("builds");
        assert_eq!(builds(), 1, "identical opts must reuse the client");

        // model/temperature/effort ride on the request, not the client.
        let model_swap = LlmOpts {
            model: "other".to_string(),
            effort: Some("high".to_string()),
            ..opts.clone()
        };
        cache.get(&model_swap).expect("builds");
        assert_eq!(builds(), 1, "model/effort swap must reuse the client");

        // Endpoint, key, and session header each force a rebuild.
        for changed in [
            LlmOpts {
                base_url: Some("http://127.0.0.1:8/v1".to_string()),
                ..opts.clone()
            },
            LlmOpts {
                api_key: Some("k2".to_string()),
                ..opts.clone()
            },
            LlmOpts {
                session_id: Some("s2".to_string()),
                ..opts.clone()
            },
        ] {
            let before = builds();
            cache.get(&changed).expect("builds");
            assert_eq!(builds(), before + 1, "a key change must rebuild the client");
        }
    }

    #[test]
    fn responses_request_builds_against_default_client_type() {
        use rig::client::CompletionClient;
        let client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://127.0.0.1:9/v1")
            .build()
            .expect("client builds without network");
        let model = client.completion_model("some-model");
        let request = build_request(&[], "sys", &[], &LlmOpts::default());
        let fut = rig::completion::CompletionModel::stream(&model, request);
        drop(fut);
    }

    /// Unreachable endpoint: the adapter surfaces an `Error` event and no
    /// `Done`, at the stream level (initial request failure).
    #[tokio::test]
    async fn adapter_surfaces_error_without_done() {
        let opts = LlmOpts {
            model: "m1".to_string(),
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            api_key: Some("test-key".to_string()),
            temperature: None,
            ..LlmOpts::default()
        };
        let stream_fn = rig_stream_fn();
        let stream = stream_fn(&[], "sys", &[], &opts);
        let events: Vec<LlmStreamEvent> =
            tokio::time::timeout(std::time::Duration::from_secs(10), stream.collect())
                .await
                .expect("stream terminates");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Error { .. })),
            "expected an Error event, got {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Done { .. })),
            "no Done after error, got {events:?}"
        );
    }

    /// Responses path hits the same shared forwarding loop: unreachable
    /// endpoint surfaces `Error` and no `Done`.
    #[tokio::test]
    async fn responses_adapter_surfaces_error_without_done() {
        let opts = LlmOpts {
            model: "m1".to_string(),
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            api_key: Some("test-key".to_string()),
            temperature: None,
            endpoint: LlmEndpoint::Responses,
            effort: None,
            session_id: None,
            retry: RetryPolicy::default(),
        };
        let stream_fn = rig_stream_fn();
        let stream = stream_fn(&[], "sys", &[], &opts);
        let events: Vec<LlmStreamEvent> =
            tokio::time::timeout(std::time::Duration::from_secs(10), stream.collect())
                .await
                .expect("stream terminates");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Error { .. })),
            "expected an Error event, got {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Done { .. })),
            "no Done after error, got {events:?}"
        );
    }
    /// Live smoke: real OpenAI-compatible endpoint, gated on env.
    #[tokio::test]
    #[ignore = "live test; run with WCODE_LIVE=1 WCODE_BASE_URL=... WCODE_API_KEY=... WCODE_MODEL=... cargo test -p wcode-harness --lib -- --ignored"]
    async fn live_rig_stream_fn() {
        let (Ok(base_url), Ok(api_key), Ok(model)) = (
            std::env::var("WCODE_BASE_URL"),
            std::env::var("WCODE_API_KEY"),
            std::env::var("WCODE_MODEL"),
        ) else {
            eprintln!("skipping: WCODE_BASE_URL / WCODE_API_KEY / WCODE_MODEL not set");
            return;
        };
        if std::env::var("WCODE_LIVE").ok().as_deref() != Some("1") {
            eprintln!("skipping: WCODE_LIVE != 1");
            return;
        }
        let opts = LlmOpts {
            model,
            base_url: Some(base_url),
            api_key: Some(api_key),
            temperature: None,
            ..LlmOpts::default()
        };
        let stream_fn = rig_stream_fn();
        let stream = stream_fn(
            &[AgentMessage::user_text("Reply with exactly: pong")],
            "You are a terse echo assistant.",
            &[],
            &opts,
        );
        let events: Vec<LlmStreamEvent> = stream.collect().await;
        let text = events
            .iter()
            .filter_map(|e| match e {
                LlmStreamEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<String>();
        println!("text: {text:?}");
        println!("events: {events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Done { .. }))
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Error { .. }))
        );
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Chunks =
        futures::stream::Iter<std::vec::IntoIter<Result<StreamedAssistantContent, CompletionError>>>;

    /// A transient transport failure as rig surfaces it on the OpenAI path.
    fn transient() -> CompletionError {
        CompletionError::ProviderError(
            "Http client error: error sending request for url (http://x)".into(),
        )
    }

    fn fatal() -> CompletionError {
        CompletionError::ProviderError("invalid_request_error: bad tool schema".into())
    }

    fn empty() -> Chunks {
        futures::stream::iter(Vec::new())
    }

    /// A terminal `Final` item (the stream's `Done`).
    fn done() -> StreamedAssistantContent {
        StreamedAssistantContent::Final(rig::streaming::StreamFinal::new(
            "test",
            rig::completion::Usage::default(),
        ))
    }

    fn first_item_err(e: CompletionError) -> Chunks {
        futures::stream::iter(vec![Err(e)])
    }

    #[test]
    fn transient_statuses_retry_client_errors_do_not() {
        for code in [408, 409, 429, 500, 502, 503, 504] {
            assert!(is_transient_status(code), "{code} should retry");
        }
        for code in [200, 400, 401, 403, 404, 422] {
            assert!(!is_transient_status(code), "{code} must not retry");
        }
    }

    #[test]
    fn retryable_classifies_transport_vs_rejections() {
        assert!(retryable(&CompletionError::HttpError(
            rig::http_client::Error::StreamEnded
        )));
        assert!(retryable(&transient()));
        assert!(!retryable(&fatal()));
        assert!(!retryable(&CompletionError::ResponseError("bad".into())));
    }

    #[test]
    fn backoff_grows_and_never_exceeds_the_cap() {
        let base = Duration::from_millis(100);
        let cap = Duration::from_millis(400);
        let d1 = backoff(1, base, cap);
        assert!(d1 <= base && d1 >= base * 3 / 4, "attempt 1 = {d1:?}");
        for n in 1..12 {
            assert!(backoff(n, base, cap) <= cap, "attempt {n} exceeds cap");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn connect_retries_transient_failures_then_succeeds() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let policy = RetryPolicy {
            max: 3,
            base: Duration::from_millis(10),
            cap: Duration::from_millis(50),
            ..RetryPolicy::default()
        };
        let c = calls.clone();
        let mut state = RetryState::new(policy.max);
        let out = connect_with_retry(
            &policy,
            &mut state,
            move || {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                Box::pin(async move {
                    if n <= 2 {
                        Err(transient())
                    } else {
                        Ok(empty())
                    }
                })
            },
            &tx,
        )
        .await;
        assert!(matches!(out, PreContent::Ready { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(matches!(
            rx.try_recv(),
            Ok(LlmStreamEvent::Retrying { attempt: 1, .. })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(LlmStreamEvent::Retrying { attempt: 2, .. })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn connect_retries_a_first_item_transport_error() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let policy = RetryPolicy {
            max: 2,
            base: Duration::from_millis(5),
            cap: Duration::from_millis(5),
            ..RetryPolicy::default()
        };
        let c = calls.clone();
        let mut state = RetryState::new(policy.max);
        let out = connect_with_retry(
            &policy,
            &mut state,
            move || {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                Box::pin(async move {
                    if n == 1 {
                        Ok(first_item_err(transient()))
                    } else {
                        Ok(empty())
                    }
                })
            },
            &tx,
        )
        .await;
        assert!(matches!(out, PreContent::Ready { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "first-item error retried");
    }

    #[tokio::test(start_paused = true)]
    async fn connect_gives_up_after_max_and_skips_non_retryable() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            max: 1,
            base: Duration::from_millis(1),
            cap: Duration::from_millis(1),
            ..RetryPolicy::default()
        };

        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let mut state = RetryState::new(policy.max);
        let out = connect_with_retry(
            &policy,
            &mut state,
            move || {
                c.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err::<Chunks, CompletionError>(transient()) })
            },
            &tx,
        )
        .await;
        assert!(matches!(out, PreContent::Failed(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "one attempt + one retry");

        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let mut state = RetryState::new(policy.max);
        let out = connect_with_retry(
            &policy,
            &mut state,
            move || {
                c.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err::<Chunks, CompletionError>(fatal()) })
            },
            &tx,
        )
        .await;
        assert!(matches!(out, PreContent::Failed(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "non-retryable returns at once");
    }

    #[test]
    fn fatal_class_flags_hard_errors_but_not_transient_transport() {
        // Transient-class failures stay feedable after their retries are
        // exhausted (the loop feeds the message back to the model); hard
        // client/build failures are fatal (the run stops).
        assert!(!fatal_class(&transient()));
        assert!(!fatal_class(&CompletionError::HttpError(
            rig::http_client::Error::StreamEnded
        )));
        assert!(fatal_class(&fatal()));
        assert!(fatal_class(&CompletionError::ResponseError("bad".into())));
        assert!(fatal_class(&CompletionError::RequestError("bad".into())));
        let json_err = serde_json::from_str::<u8>("not a number").unwrap_err();
        assert!(fatal_class(&CompletionError::JsonError(json_err)));
    }

    /// Live-ish check against a refused port: a transport failure exhausts the
    /// retry budget (Retrying notices) and surfaces as a *non-fatal* stream
    /// error — which is exactly what lets the loop feed it back next turn
    /// instead of dying.
    #[tokio::test]
    async fn refused_port_exhausts_retries_to_non_fatal_error() {
        let opts = LlmOpts {
            model: "m1".to_string(),
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            api_key: Some("test-key".to_string()),
            retry: RetryPolicy {
                max: 2,
                base: Duration::from_millis(10),
                cap: Duration::from_millis(50),
                ..RetryPolicy::default()
            },
            ..LlmOpts::default()
        };
        let stream_fn = rig_stream_fn();
        let stream = stream_fn(&[], "sys", &[], &opts);
        let events: Vec<LlmStreamEvent> =
            tokio::time::timeout(std::time::Duration::from_secs(10), stream.collect())
                .await
                .expect("stream terminates");
        let retries = events
            .iter()
            .filter(|e| matches!(e, LlmStreamEvent::Retrying { .. }))
            .count();
        assert_eq!(retries, 2, "two backoff notices before giving up: {events:?}");
        assert!(
            matches!(events.last(), Some(LlmStreamEvent::Error { fatal: false, .. })),
            "refused port is transient-class, not fatal: {events:?}"
        );
    }

    /// A `CompletionError` carrying a preserved response with a `Retry-After`
    /// header (no HTTP status needed: the header path reads
    /// `provider_response_headers()` regardless of status).
    fn err_with_retry_after(value: &str) -> CompletionError {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_str(value).unwrap());
        CompletionError::ProviderResponse(
            rig::ProviderResponseError::without_status("429")
                .with_headers(Some(Box::new(headers))),
        )
    }

    #[test]
    fn retry_after_parses_integer_seconds() {
        assert_eq!(
            retry_after(&err_with_retry_after("7")),
            Some(Duration::from_secs(7))
        );
    }

    #[test]
    fn retry_after_is_none_when_absent() {
        // A transport failure with no preserved response/headers.
        assert_eq!(retry_after(&transient()), None);
    }

    #[test]
    fn retry_after_is_none_on_non_numeric() {
        // The HTTP-date form and garbage both parse to `None` (dates are out of
        // scope — see `retry_after`).
        assert_eq!(
            retry_after(&err_with_retry_after("Wed, 21 Oct 2015 07:28:00 GMT")),
            None
        );
        assert_eq!(retry_after(&err_with_retry_after("soon")), None);
    }

    #[tokio::test(start_paused = true)]
    async fn retry_wait_honors_retry_after_over_backoff() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // base 10ms gives a ~10ms backoff for attempt 1; `Retry-After: 3` must
        // win, so the paused clock advances ~3s instead.
        let policy = RetryPolicy {
            base: Duration::from_millis(10),
            ..RetryPolicy::default()
        };
        let start = tokio::time::Instant::now();
        assert!(retry_wait(&tx, &policy, 1, &err_with_retry_after("3")).await);
        assert_eq!(start.elapsed(), Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn retry_wait_clamps_retry_after_to_the_cap() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // A hostile/buggy `Retry-After: 100` must not park the turn for 100s:
        // it is clamped to `policy.cap` (8s), not honored verbatim.
        let policy = RetryPolicy {
            cap: Duration::from_secs(8),
            ..RetryPolicy::default()
        };
        let start = tokio::time::Instant::now();
        assert!(retry_wait(&tx, &policy, 1, &err_with_retry_after("100")).await);
        assert_eq!(start.elapsed(), Duration::from_secs(8));
    }
    // ---- #23: stream-stall timeouts ----

    type BoxStream =
        Pin<Box<dyn futures::Stream<Item = Result<StreamedAssistantContent, CompletionError>> + Send>>;

    fn boxed<S>(s: S) -> BoxStream
    where
        S: futures::Stream<Item = Result<StreamedAssistantContent, CompletionError>>
            + Send
            + 'static,
    {
        Box::pin(s)
    }

    fn silent_boxed() -> BoxStream {
        boxed(futures::stream::pending::<Result<StreamedAssistantContent, CompletionError>>())
    }

    /// A fresh silent stream per call — `forward_stream`/`connect_with_retry`'s
    /// re-open seam, never exercised where content already flowed.
    fn open_silent()
    -> Pin<Box<dyn std::future::Future<Output = Result<BoxStream, CompletionError>> + Send>> {
        Box::pin(async { Ok::<_, CompletionError>(silent_boxed()) })
    }

    /// A slow (but successful) connect — used to prove `ttft = ZERO` awaits it
    /// directly instead of racing an immediate `sleep(ZERO)`.
    fn open_slow_ok()
    -> Pin<Box<dyn std::future::Future<Output = Result<BoxStream, CompletionError>> + Send>> {
        Box::pin(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, CompletionError>(boxed(futures::stream::iter(vec![Ok::<_, CompletionError>(
                StreamedAssistantContent::Text(rig::message::Text::new("ok")),
            )])))
        })
    }

    /// A stall is transient-class, so it rides the retry / feed-back paths (B1).
    #[test]
    fn a_stall_error_is_transient_and_not_hard_fatal() {
        let d = Duration::from_secs(1);
        assert!(retryable(&stall_error(d)), "a stall must be transient (B1)");
        assert!(!fatal_class(&stall_error(d)), "a stall must not be hard-fatal");
        assert!(
            stall_error(d).to_string().contains("timed out"),
            "the (timed out) needle drives retryable()"
        );
    }

    /// A silent stream stalls on the `ttft` peek every attempt: each retry emits
    /// a `Retrying` notice, and once the budget is spent the failure is a
    /// transient (non-fatal) `stall_error`.
    #[tokio::test(start_paused = true)]
    async fn a_silent_stream_trips_the_ttft_retries_then_fails() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let policy = RetryPolicy {
            max: 2,
            base: Duration::from_millis(10),
            cap: Duration::from_millis(10),
            ttft: Duration::from_secs(1),
            ..RetryPolicy::default()
        };
        let c = calls.clone();
        let open = move || {
            c.fetch_add(1, Ordering::SeqCst);
            open_silent()
        };
        let mut state = RetryState::new(policy.max);
        let out = connect_with_retry(&policy, &mut state, open, &tx).await;

        let PreContent::Failed(error) = out else {
            panic!("expected Failed from an exhausted stall")
        };
        assert!(retryable(&error), "a stall is transient");
        assert!(!fatal_class(&error), "a stall is not hard-fatal");
        assert_eq!(calls.load(Ordering::SeqCst), 3, "one attempt + two retries");
        let retries = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|e| matches!(e, LlmStreamEvent::Retrying { .. }))
            .count();
        assert_eq!(retries, 2, "a Retrying notice per retry");
    }

    /// A stream that emits a delta and then goes silent trips the idle deadline:
    /// because content already flowed, the failure is a NON-fatal error (the
    /// loop feeds it back) and is not retried (that would duplicate output).
    #[tokio::test(start_paused = true)]
    async fn deltas_then_a_stall_is_a_nonfatal_error() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            max: 3,
            base: Duration::from_millis(10),
            cap: Duration::from_millis(10),
            idle: Duration::from_secs(1),
            ..RetryPolicy::default()
        };
        let stream = boxed(
            futures::stream::iter(vec![Ok::<_, CompletionError>(
                StreamedAssistantContent::Text(rig::message::Text::new("hi")),
            )])
            .chain(futures::stream::pending::<Result<StreamedAssistantContent, CompletionError>>()),
        );
        let state = RetryState::new(policy.max);
        forward_stream(stream, None, policy, open_silent, tx, state).await;

        let events: Vec<LlmStreamEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::TextDelta(t) if t == "hi")),
            "{events:?}"
        );
        let error = events
            .iter()
            .find_map(|e| match e {
                LlmStreamEvent::Error { message, fatal } => Some((message.clone(), *fatal)),
                _ => None,
            })
            .expect("the idle deadline surfaced an error");
        assert!(!error.1, "a post-content stall is non-fatal: {error:?}");
        assert!(
            !events.iter().any(|e| matches!(e, LlmStreamEvent::Retrying { .. })),
            "no retry once content flowed: {events:?}"
        );
    }

    /// A clean stream (deltas then `Done`) is forwarded untouched: no error, no
    /// retry, the idle deadline never fires.
    #[tokio::test(start_paused = true)]
    async fn a_clean_stream_is_unaffected() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            idle: Duration::from_secs(1),
            ..RetryPolicy::default()
        };
        let stream = boxed(futures::stream::iter(vec![
            Ok::<_, CompletionError>(StreamedAssistantContent::Text(rig::message::Text::new("hi"))),
            Ok::<_, CompletionError>(done()),
        ]));
        let state = RetryState::new(policy.max);
        forward_stream(stream, None, policy, open_silent, tx, state).await;

        let events: Vec<LlmStreamEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::TextDelta(t) if t == "hi")),
            "{events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e, LlmStreamEvent::Done { .. })),
            "{events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, LlmStreamEvent::Error { .. })),
            "{events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, LlmStreamEvent::Retrying { .. })),
            "{events:?}"
        );
    }

    /// `ZERO` disables the deadline at BOTH seams: the peek collapses to a plain
    /// `stream.next()` (a silent stream stays pending), and the connect race is
    /// skipped so a slow connect is not spuriously timed out.
    #[tokio::test(start_paused = true)]
    async fn zero_deadlines_disable_the_timeout() {
        // Peek seam: no `sleep(ZERO)`, so a silent stream never yields `Idle`.
        let mut silent =
            futures::stream::pending::<Result<StreamedAssistantContent, CompletionError>>();
        let stalled = tokio::time::timeout(
            Duration::from_secs(1),
            next_or_stall(&mut silent, Duration::ZERO),
        )
        .await;
        assert!(stalled.is_err(), "ZERO must not trip the deadline");

        // Connect seam: `ZERO` awaits the connect directly.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            ttft: Duration::ZERO,
            ..RetryPolicy::default()
        };
        let mut state = RetryState::new(policy.max);
        let out = connect_with_retry(&policy, &mut state, open_slow_ok, &tx).await;
        assert!(
            matches!(out, PreContent::Ready { .. }),
            "a slow connect must not be timed out under ZERO"
        );
    }

    /// The genuinely-stalled *connect* branch: `open()` never resolves, so the
    /// nonzero `ttft` race times out on every attempt. Each retry emits a
    /// `Retrying` notice; once the budget is spent the failure is a transient
    /// (non-fatal) `stall_error` — the same shape as a peek stall, but through
    /// the connect race's `Err(())` arm.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_connect_trips_the_ttft_retries_then_fails() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let policy = RetryPolicy {
            max: 2,
            base: Duration::from_millis(10),
            cap: Duration::from_millis(10),
            ttft: Duration::from_secs(1),
            ..RetryPolicy::default()
        };
        let c = calls.clone();
        let open = move || {
            c.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::pending::<Result<BoxStream, CompletionError>>())
                as Pin<
                    Box<
                        dyn std::future::Future<Output = Result<BoxStream, CompletionError>> + Send,
                    >,
                >
        };
        let mut state = RetryState::new(policy.max);
        let out = connect_with_retry(&policy, &mut state, open, &tx).await;

        let PreContent::Failed(error) = out else {
            panic!("expected Failed from an exhausted connect stall")
        };
        assert!(retryable(&error), "a connect stall is transient");
        assert!(!fatal_class(&error), "a connect stall is not hard-fatal");
        assert_eq!(calls.load(Ordering::SeqCst), 3, "one attempt + two retries");
        let retries = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|e| matches!(e, LlmStreamEvent::Retrying { .. }))
            .count();
        assert_eq!(retries, 2, "a Retrying notice per retry");
    }

    /// The forward loop's *pre-content* idle branch: a stream that emits nothing
    /// and then stalls, with budget remaining, backs off (a `Retrying` notice)
    /// and re-opens — here the re-opened stream succeeds and the turn completes.
    #[tokio::test(start_paused = true)]
    async fn a_pre_content_idle_stall_retries_and_reopens() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            max: 2,
            base: Duration::from_millis(10),
            cap: Duration::from_millis(10),
            idle: Duration::from_secs(1),
            ..RetryPolicy::default()
        };
        // The initial stream never yields → the idle deadline trips pre-content.
        let stream = silent_boxed();
        // The re-open yields a `Final`, so the retried turn completes.
        fn open_done()
        -> Pin<Box<dyn std::future::Future<Output = Result<BoxStream, CompletionError>> + Send>> {
            Box::pin(async {
                Ok::<_, CompletionError>(boxed(futures::stream::iter(vec![Ok::<_, CompletionError>(
                    done(),
                )])))
            })
        }
        let state = RetryState::new(policy.max);
        forward_stream(stream, None, policy, open_done, tx, state).await;

        let events: Vec<LlmStreamEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Retrying { .. })),
            "a pre-content stall backs off: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e, LlmStreamEvent::Done { .. })),
            "the re-opened stream completed: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, LlmStreamEvent::Error { .. })),
            "no error once the retry succeeded: {events:?}"
        );
    }

    /// A re-open that ALSO ends immediately, pre-content: paired with an empty
    /// initial stream, it drives the truncation arm's retries to exhaustion (each
    /// re-open yields no item, so nothing is ever forwarded).
    fn open_empty()
    -> Pin<Box<dyn std::future::Future<Output = Result<BoxStream, CompletionError>> + Send>> {
        Box::pin(async { Ok::<_, CompletionError>(boxed(empty())) })
    }

    /// A delta followed by an early EOF (no `Final`) is a truncation: content
    /// already flowed, so it is NOT retried — a single NON-fatal `Error` ends the
    /// turn (the loop feeds it back), and no `Done` is emitted.
    #[tokio::test(start_paused = true)]
    async fn a_delta_then_a_truncated_end_is_a_nonfatal_error() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            max: 3,
            base: Duration::from_millis(10),
            cap: Duration::from_millis(10),
            idle: Duration::ZERO,
            ..RetryPolicy::default()
        };
        let stream = boxed(futures::stream::iter(vec![Ok::<_, CompletionError>(
            StreamedAssistantContent::Text(rig::message::Text::new("hi")),
        )]));
        let state = RetryState::new(policy.max);
        forward_stream(stream, None, policy, open_silent, tx, state).await;

        let events: Vec<LlmStreamEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::TextDelta(t) if t == "hi")),
            "{events:?}"
        );
        let (message, fatal) = events
            .iter()
            .find_map(|e| match e {
                LlmStreamEvent::Error { message, fatal } => Some((message.clone(), *fatal)),
                _ => None,
            })
            .expect("a truncated end surfaced an error");
        assert!(!fatal, "a post-content truncation is non-fatal: {message}");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Done { .. })),
            "no Done on a truncated stream: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Retrying { .. })),
            "no retry once content flowed: {events:?}"
        );
    }

    /// A `Final`-terminated stream is unchanged: it forwards `Done` and never
    /// trips the truncation guard (which would surface a spurious error).
    #[tokio::test(start_paused = true)]
    async fn a_final_terminated_stream_is_unchanged() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            idle: Duration::ZERO,
            ..RetryPolicy::default()
        };
        let stream = boxed(futures::stream::iter(vec![
            Ok::<_, CompletionError>(StreamedAssistantContent::Text(rig::message::Text::new(
                "hi",
            ))),
            Ok::<_, CompletionError>(done()),
        ]));
        let state = RetryState::new(policy.max);
        forward_stream(stream, None, policy, open_silent, tx, state).await;

        let events: Vec<LlmStreamEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Done { .. })),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Error { .. })),
            "a clean turn surfaces no error: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Retrying { .. })),
            "{events:?}"
        );
    }

    /// A pre-content no-item stream with budget remaining retries on the shared
    /// budget; each re-open also ends empty, so the whole budget is spent before
    /// a NON-fatal error ends the turn — one `Retrying` notice per retry, no
    /// `Done`.
    #[tokio::test(start_paused = true)]
    async fn a_pre_content_no_item_stream_retries_then_errors() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = RetryPolicy {
            max: 2,
            base: Duration::from_millis(10),
            cap: Duration::from_millis(10),
            idle: Duration::ZERO,
            ..RetryPolicy::default()
        };
        let state = RetryState::new(policy.max);
        forward_stream(boxed(empty()), None, policy, open_empty, tx, state).await;

        let events: Vec<LlmStreamEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let retries = events
            .iter()
            .filter(|e| matches!(e, LlmStreamEvent::Retrying { .. }))
            .count();
        assert_eq!(
            retries, policy.max as usize,
            "one Retrying notice per retry: {events:?}"
        );
        let (message, fatal) = events
            .iter()
            .find_map(|e| match e {
                LlmStreamEvent::Error { message, fatal } => Some((message.clone(), *fatal)),
                _ => None,
            })
            .expect("an exhausted truncation budget surfaced an error");
        assert!(!fatal, "an exhausted truncation is non-fatal: {message}");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LlmStreamEvent::Done { .. })),
            "no Done on a truncated stream: {events:?}"
        );
    }
}
