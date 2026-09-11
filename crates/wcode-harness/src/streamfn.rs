//! LLM stream-function seam + default `rig` (OpenAI-compatible) adapter.
//!
//! This module is the only place rig types appear in the kernel. A
//! [`StreamFn`] turns harness conversation context into a [`LlmStream`] of
//! [`LlmStreamEvent`]s; [`rig_stream_fn`] is the default implementation built
//! on rig's Chat Completions API client.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

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
    let stream_fut: std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<StreamingCompletionResponse, CompletionError>>
                + Send,
        >,
    > = match opts.endpoint {
        LlmEndpoint::Chat => {
            let model = client
                .completions_api()
                .completion_model(opts.model.clone());
            Box::pin(async move { model.stream(request).await })
        }
        LlmEndpoint::Responses => {
            let model = client.completion_model(opts.model.clone());
            Box::pin(async move { model.stream(request).await })
        }
    };

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LlmStreamEvent>();
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return error_stream("StreamFn requires a tokio runtime".to_string());
    };
    runtime.spawn(async move {
        let mut stream = match stream_fut.await {
            Ok(stream) => stream,
            Err(e) => {
                let _ = tx.send(LlmStreamEvent::Error {
                    message: e.to_string(),
                });
                return;
            }
        };
        let mut errored = false;
        loop {
            let item = tokio::select! {
                biased;
                _ = tx.closed() => break,
                item = stream.next() => match item {
                    Some(item) => item,
                    None => break,
                },
            };
            let events = match item {
                Ok(content) => {
                    if errored && matches!(content, StreamedAssistantContent::Final(_)) {
                        continue;
                    }
                    map_item(content)
                }
                Err(e) => {
                    errored = true;
                    vec![LlmStreamEvent::Error {
                        message: e.to_string(),
                    }]
                }
            };
            for ev in events {
                if tx.send(ev).is_err() {
                    return;
                }
            }
        }
    });
    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|ev| (ev, rx))
    }))
}

fn error_stream(message: String) -> LlmStream {
    Box::pin(futures::stream::iter(vec![LlmStreamEvent::Error {
        message,
    }]))
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
        additional_params: effort_params(opts),
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

/// Free-style effort passthrough, fanned out per wire shape. None sends
/// nothing (backends that reject unknown fields keep working). Values are
/// forwarded verbatim on Chat (`reasoning_effort`); on Responses the payload
/// is `reasoning: { effort }` after `invalid_effort` has validated the level.
fn effort_params(opts: &LlmOpts) -> Option<serde_json::Value> {
    let effort = opts.effort.as_ref()?;
    match opts.endpoint {
        LlmEndpoint::Chat => Some(serde_json::json!({ "reasoning_effort": effort })),
        LlmEndpoint::Responses => Some(serde_json::json!({ "reasoning": { "effort": effort } })),
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
        // ponytail: kernel v1 is append-only; this complete-block branch is dead
        // on chat-completions wires (deltas only). Restating wire => replacement variant.
        StreamedAssistantContent::Reasoning { reasoning, .. } => {
            // Replacement semantics: the complete block supersedes the
            // accumulated deltas (the loop drops prior thinking on this event).
            vec![LlmStreamEvent::ThinkingReplace(reasoning_text(&reasoning))]
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
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
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
            Some(json!({ "reasoning": { "effort": "xhigh" } }))
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
