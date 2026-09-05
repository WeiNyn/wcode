//! Compile spike + live smoke for `rig` 0.42 against an OpenAI-compatible
//! endpoint over the **Chat Completions** API (not the default Responses API).
//!
//! # Verified working API paths (rig 0.42, crate `rig` facade over `rig-core`)
//!
//! Cargo pin that suffices (root `[workspace.dependencies]`):
//! `rig = { version = "0.42", default-features = false, features = ["reqwest", "rustls"] }`
//! (`reqwest`/`rustls` forward to `rig-core`; no other feature is needed for
//! the OpenAI provider — providers are not feature-gated. Omitting
//! `default-features` drops the `agent` runtime, which is fine for direct
//! `CompletionModel` use.)
//!
//! Constructor chain:
//!
//! ```text
//! use rig::client::CompletionClient;
//! use rig::providers::openai;
//!
//! // `Client` = Responses API client (the default). `builder()` exists;
//! // `api_key` accepts `&str`/`String` (BearerAuth), `base_url` any AsRef<str>
//! // and should include the `/v1` prefix for OpenAI-compatible gateways
//! // (default is `https://api.openai.com/v1`). `build()` returns
//! // `http_client::Result<openai::Client>`.
//! let client = openai::Client::builder()
//!     .api_key("sk-...")
//!     .base_url("https://gateway.example.com/v1")
//!     .build()?;
//!
//! // Switch to the Chat Completions client (type alias
//! // `openai::CompletionsClient`). This is the REQUIRED hop for
//! // OpenAI-compatible providers that lack `/responses`.
//! let completions = client.completions_api();
//!
//! // From `CompletionClient` trait: `fn completion_model(&self, impl Into<String>)`.
//! let model = completions.completion_model("some-model");
//! // model: openai::completion::CompletionModel (= GenericCompletionModel<OpenAICompletionsExt, reqwest::Client>), Clone
//! ```
//!
//! Request + stream:
//!
//! ```text
//! use rig::completion::{CompletionModel, CompletionRequestBuilder, ToolDefinition};
//! use rig::message::{AssistantContent, Message};
//!
//! // `CompletionRequestBuilder::new(model, prompt: impl Into<Message>)` takes
//! // the model by value (`completion_request()` on the trait is equivalent but
//! // needs `Self: Sized + Clone`). Builder methods: `.preamble(String)`,
//! // `.message(Message)` / `.messages(iter)`, `.tool(ToolDefinition)` /
//! // `.tools(Vec<_>)`, `.temperature(f64)`, `.max_tokens(u64)`, `.build()`.
//! let request = CompletionRequestBuilder::new(model, "prompt")
//!     .preamble("system instructions".to_string())
//!     .message(Message::Assistant { id: None, content: vec![AssistantContent::text("hi")] })
//!     .tool(ToolDefinition { name, description, parameters: json!({...}) })
//!     .build();
//!
//! // `CompletionModel::stream(&self, CompletionRequest) -> impl Future<Output =
//! // Result<StreamingCompletionResponse, CompletionError>>`. Constructing the
//! // future is lazy; polling it opens the HTTP/SSE connection.
//! let mut stream = model.stream(request).await?;
//! ```
//!
//! Consuming the stream (`futures::StreamExt::next`); item type
//! `Result<StreamedAssistantContent, CompletionError>`:
//!
//! - `StreamedAssistantContent::Text(rig::message::Text)` — field `.text: String`
//! - `ToolCall { tool_call: rig::message::ToolCall, internal_call_id: String }`
//! - `ToolCallDelta { internal_call_id: String, content: ToolCallDeltaContent }`
//!   (`ToolCallDeltaContent::Name(String) | Delta(String)`)
//! - `Reasoning { reasoning: Reasoning, id: String }`
//! - `ReasoningDelta { id: String, provider_id: Option<String>, reasoning: String }`
//! - `Final(StreamFinal)` — the normalized terminal record, yielded as an item
//!   exactly once before the stream ends
//! - `Unknown(UnknownPayload)` — unmodeled provider items, verbatim
//!
//! Terminal state (after draining to `None`):
//!
//! - `stream.response: Option<StreamFinal>` (pub field) — `None` means the
//!   stream was truncated or terminated by a transport error, never
//!   "completed with zero usage".
//! - `StreamFinal` fields: `usage: rig::completion::Usage` (pub fields
//!   `input_tokens`/`output_tokens`/`total_tokens`/…), `finish_reason:
//!   Option<rig::completion::FinishReason>`, `provider: String`, `model:
//!   Option<String>`, plus `message_id`/`response_id`/`provider_request_id`
//!   and `raw: serde_json::Value` (the provider's own terminal payload).
//! - Convenience: `stream.usage() -> Usage`, `stream.choice:
//!   Vec<rig::message::AssistantContent>` (aggregated turn content),
//!   `stream.into() -> CompletionResponse`.
//!
//! Gotchas:
//!
//! - The stream **must** be drained to `None`. An `Err` item is not terminal
//!   (malformed frames yield `Err` and the stream continues); breaking at the
//!   first `Err` can miss the `Final` record.
//! - `builder().preamble(..)` is funneled into `chat_history` as a leading
//!   `Message::System` at `.build()` time — the built `CompletionRequest`
//!   always has `preamble: None`.
//! - OpenAI-compatible gateways frequently 404 on the Responses API; always
//!   route through `.completions_api()` when targeting generic gateways.

use futures::StreamExt;
use rig::client::CompletionClient;
use rig::completion::{CompletionModel, CompletionRequestBuilder, ToolDefinition};
use rig::message::{AssistantContent, Message};
use rig::providers::openai;
use rig::streaming::{StreamFinal, StreamedAssistantContent};

/// Compile the full call chain without any network I/O: build client →
/// completions client → model → request with preamble + chat history + one
/// tool, then lazily construct (but never poll) the streaming future.
#[test]
fn completions_stream_call_chain_compiles() {
    let client = openai::Client::builder()
        .api_key("test-key")
        .base_url("http://127.0.0.1:9/v1")
        .build()
        .expect("client builds without network");

    let model = client.completions_api().completion_model("some-model");

    let tool = ToolDefinition {
        name: "get_weather".to_string(),
        description: "Look up the current weather for a city".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "city": { "type": "string", "description": "City name" }
            },
            "required": ["city"]
        }),
    };

    let request = CompletionRequestBuilder::new(
        model.clone(),
        "What is the weather in Tokyo? Use the get_weather tool.",
    )
    .preamble("You are a concise weather assistant.".to_string())
    .message(Message::Assistant {
        id: None,
        content: vec![AssistantContent::text(
            "Understood, I will answer weather questions.",
        )],
    })
    .tool(tool)
    .temperature(0.5)
    .max_tokens(512)
    .build();

    // Preamble is funneled into a leading System message at build time.
    assert!(request.preamble.is_none());
    assert!(request.chat_history.is_empty() == false);
    assert!(
        matches!(request.chat_history.first(), Some(Message::System { .. })),
        "preamble must land as the first System message, got {:?}",
        request.chat_history.first()
    );
    assert_eq!(request.tools.len(), 1);
    assert_eq!(request.tools[0].name, "get_weather");

    // Lazy future construction compiles the whole `CompletionModel::stream`
    // chain; dropping without polling performs no I/O.
    let stream_future = model.stream(request);
    drop(stream_future);
}

/// Live smoke test against a real OpenAI-compatible endpoint.
///
/// Gated on env:
/// - `WCODE_LIVE=1` (must be set or the test no-ops)
/// - `WCODE_BASE_URL` (e.g. `https://gateway.example.com/v1`)
/// - `WCODE_API_KEY`
/// - `WCODE_MODEL`
#[tokio::test]
#[ignore = "live test; run with WCODE_LIVE=1 WCODE_BASE_URL=... WCODE_API_KEY=... WCODE_MODEL=... cargo test -p wcode-harness --test rig_spike -- --ignored"]
async fn live_completions_stream() {
    let (Ok(base_url), Ok(api_key), Ok(model_name)) = (
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

    let client = openai::Client::builder()
        .api_key(api_key)
        .base_url(base_url)
        .build()
        .expect("client builds");
    let model = client.completions_api().completion_model(model_name);

    let request =
        CompletionRequestBuilder::new(model.clone(), "Reply with exactly the word: pong")
            .preamble("You are a terse echo assistant.".to_string())
            .build();

    let mut stream = model
        .stream(request)
        .await
        .expect("stream request accepted");

    let mut variant_names: Vec<&'static str> = Vec::new();
    let mut text = String::new();
    let mut terminal: Option<StreamFinal> = None;
    let mut errors: Vec<String> = Vec::new();

    // Drain to None: an Err item is not terminal; a later Final may still arrive.
    while let Some(item) = stream.next().await {
        match item {
            Ok(content) => {
                variant_names.push(match &content {
                    StreamedAssistantContent::Text(t) => {
                        text.push_str(&t.text);
                        "Text"
                    }
                    StreamedAssistantContent::ToolCall { tool_call, .. } => {
                        eprintln!("tool call: {}", tool_call.function.name);
                        "ToolCall"
                    }
                    StreamedAssistantContent::ToolCallDelta { .. } => "ToolCallDelta",
                    StreamedAssistantContent::Reasoning { .. } => "Reasoning",
                    StreamedAssistantContent::ReasoningDelta { .. } => "ReasoningDelta",
                    StreamedAssistantContent::Final(final_record) => {
                        terminal = Some(final_record.clone());
                        "Final"
                    }
                    StreamedAssistantContent::Unknown(_) => "Unknown",
                });
            }
            Err(err) => errors.push(err.to_string()),
        }
    }

    println!("streamed variants: {variant_names:?}");
    println!("aggregated text: {text:?}");
    println!("aggregated choice: {:?}", stream.choice);

    assert!(
        variant_names.contains(&"Text") || variant_names.contains(&"ToolCall"),
        "expected at least one content item, got {variant_names:?}"
    );
    assert!(
        errors.is_empty(),
        "stream surfaced errors: {errors:?}"
    );

    // Terminal record must be readable both from the yielded Final item and
    // from `stream.response` after the drain.
    let final_record = terminal.or_else(|| stream.response.clone());
    let final_record = final_record.expect("provider must emit a terminal record");
    println!(
        "terminal: usage={} finish_reason={:?} provider={} model={:?}",
        final_record.usage.total_tokens,
        final_record.finish_reason,
        final_record.provider,
        final_record.model,
    );
    assert!(
        final_record.finish_reason.is_some(),
        "finish_reason should be readable on the terminal record"
    );
    assert_eq!(
        stream.usage().total_tokens,
        final_record.usage.total_tokens,
        "stream.usage() mirrors the terminal record"
    );
}
