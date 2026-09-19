//! Scratch probe: what context-window / usage info does the configured
//! OpenAI-compatible endpoint actually expose through rig?
//!
//!   cargo run -p wcode-harness --example opencode_probe
//!   PROBE_OVERSIZED=1 cargo run -p wcode-harness --example opencode_probe
//!
//! Reads ~/.config/wcode/config.toml; WCODE_BASE_URL / WCODE_API_KEY /
//! WCODE_MODEL override it.

use futures::StreamExt as _;
use rig::client::ModelListingClient;
use rig::providers::openai;
use wcode_harness::event::LlmStreamEvent;
use wcode_harness::message::AgentMessage;
use wcode_harness::streamfn::{LlmEndpoint, LlmOpts, rig_stream_fn};

/// Rudimentary `key = "value"` reader for the flat wcode config.
fn read_config(key: &str) -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let text = std::fs::read_to_string(
        std::path::Path::new(&home).join(".config/wcode/config.toml"),
    )
    .ok()?;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn cfg(env: &str, key: &str) -> Option<String> {
    std::env::var(env)
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| read_config(key))
}

#[tokio::main]
async fn main() {
    let base_url = cfg("WCODE_BASE_URL", "base_url");
    let api_key = cfg("WCODE_API_KEY", "api_key");
    let model = cfg("WCODE_MODEL", "model").expect("no model in config/env");

    let shown_key = api_key
        .as_deref()
        .map(|k| format!("{}… ({} chars)", &k[..k.len().min(4)], k.len()))
        .unwrap_or_else(|| "(none)".into());
    println!("endpoint : {}", base_url.as_deref().unwrap_or("(default openai)"));
    println!("model    : {model}");
    println!("api_key  : {shown_key}");

    // ---- 1. Model listing: does the API report a context window? ----
    println!("\n### 1. GET /models — raw rig Model fields");
    let key = api_key.clone().unwrap_or_default();
    let mut builder = openai::Client::builder().api_key::<rig::client::BearerAuth>(key);
    if let Some(u) = &base_url {
        builder = builder.base_url(u);
    }
    let client: openai::Client = match builder.build() {
        Ok(c) => c,
        Err(e) => {
            println!("build error: {e}");
            return;
        }
    };
    match client.list_models().await {
        Ok(list) => {
            println!("models: {}", list.len());
            for m in list.iter().take(4) {
                println!("  {}", serde_json::to_string(m).unwrap_or_default());
            }
            let with_ctx = list.iter().filter(|m| m.context_length.is_some()).count();
            let with_out = list.iter().filter(|m| m.max_output_tokens.is_some()).count();
            println!("  -> context_length present: {with_ctx}/{}", list.len());
            println!("  -> max_output_tokens present: {with_out}/{}", list.len());
            if let Some(m) = list.iter().find(|m| m.id == model) {
                println!("  -> configured model: {}", serde_json::to_string(m).unwrap());
            }
        }
        Err(e) => println!("list_models error: {e}"),
    }

    // ---- 2. Streaming completion: usage on the wire we actually use ----
    println!("\n### 2. streaming completion — usage on the wire wcode uses");
    let opts = LlmOpts {
        model: model.clone(),
        base_url: base_url.clone(),
        api_key: api_key.clone(),
        endpoint: LlmEndpoint::Chat,
        // opencode routes on this header; wcode sets it per-agent.
        session_id: Some("opencode-probe".to_string()),
        ..LlmOpts::default()
    };
    let stream = rig_stream_fn();
    let events: Vec<LlmStreamEvent> = stream(
        &[AgentMessage::user_text("Reply with exactly: pong")],
        "You are terse.",
        &[],
        &opts,
    )
    .collect()
    .await;
    for ev in &events {
        match ev {
            LlmStreamEvent::Done { stop_reason, usage } => {
                println!("Done: stop_reason={stop_reason:?} usage={usage:?}");
            }
            LlmStreamEvent::Error { message, .. } => println!("Error event: {message}"),
            _ => {}
        }
    }

    // ---- 3. Oversized request: does the error name the limit? ----
    if std::env::var("PROBE_OVERSIZED").ok().as_deref() == Some("1") {
        println!("\n### 3. oversized request — does the error reveal the window?");
        let words: usize = std::env::var("PROBE_WORDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300_000);
        let huge = "word ".repeat(words);
        println!("(sending {words} words ≈ {} chars)", huge.len());
        let events: Vec<LlmStreamEvent> = stream(
            &[AgentMessage::user_text(huge)],
            "You are terse.",
            &[],
            &opts,
        )
        .collect()
        .await;
        let mut seen = false;
        for ev in &events {
            match ev {
                LlmStreamEvent::Error { message, .. } => {
                    seen = true;
                    println!("Error: {}", &message[..message.len().min(1200)]);
                }
                LlmStreamEvent::Done { stop_reason, usage } => {
                    println!("(no error; stop_reason={stop_reason:?} usage={usage:?})")
                }
                _ => {}
            }
        }
        if !seen {
            println!("(no Error event — request was accepted?)");
        }
    } else {
        println!("\n(skipping oversized probe; set PROBE_OVERSIZED=1 to run it)");
    }
}
