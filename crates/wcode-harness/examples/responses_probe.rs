//! Live probe — a human's view of the **Responses**-wire reasoning path.
//!
//! This is the interactive sibling of `tests/responses_reasoning.rs`: that test
//! drives a canned SSE body, this probe drives a real endpoint and prints every
//! mapped [`LlmStreamEvent`] as it arrives, so you can watch reasoning surface
//! against your own provider:
//!
//! - `ThinkingDelta` — streamed reasoning-summary deltas;
//! - `ThinkingReplace` — the complete reasoning item the Responses wire restates
//!   after the deltas (replacement semantics; the kernel drops the prior deltas);
//! - `TextDelta` / `Done` — the visible answer;
//! - and, as the regression guard, that no `ThinkingReplace("")` arrives: an
//!   empty replacement would erase the accumulated thinking downstream.
//!
//! It is opt-in (not part of any test run): point it at an endpoint and a model
//! that can emit a reasoning summary.
//!
//!   WCODE_MODEL=<responses-capable> cargo run -p wcode-harness --example responses_probe
//!   WCODE_EFFORT=high                                   # optional
//!   WCODE_PROMPT="think, then say pong"                 # optional
//!
//! Env: `WCODE_BASE_URL` / `WCODE_API_KEY` / `WCODE_MODEL` / `WCODE_ENDPOINT`
//! (`chat` selects Chat Completions; anything else, including unset, selects the
//! Responses wire this probe exists to exercise).

use futures::StreamExt as _;
use wcode_harness::event::LlmStreamEvent;
use wcode_harness::message::AgentMessage;
use wcode_harness::streamfn::{LlmEndpoint, LlmOpts, rig_stream_fn};

/// The non-empty value of `env`, if set.
fn cfg(env: &str) -> Option<String> {
    std::env::var(env).ok().filter(|s| !s.is_empty())
}

/// The wire to probe: `WCODE_ENDPOINT=chat` opts into Chat Completions; the
/// default (and every other value) is the Responses wire.
fn endpoint() -> LlmEndpoint {
    match cfg("WCODE_ENDPOINT").as_deref() {
        Some("chat" | "completions" | "chat-completions") => LlmEndpoint::Chat,
        _ => LlmEndpoint::Responses,
    }
}

/// Print one mapped event — the reasoning path is the point, so it leads.
fn print_event(ev: &LlmStreamEvent) {
    match ev {
        LlmStreamEvent::ThinkingDelta(text) => println!("  thinking delta : {text:?}"),
        LlmStreamEvent::ThinkingReplace(text) => println!("  thinking block : {text:?}"),
        LlmStreamEvent::TextDelta(text) => println!("  text delta     : {text:?}"),
        LlmStreamEvent::ToolCall { name, .. } => println!("  tool call      : {name}"),
        LlmStreamEvent::Retrying {
            attempt,
            max,
            reason,
        } => {
            println!("  retry {attempt}/{max}   : {reason}")
        }
        LlmStreamEvent::Done { stop_reason, usage } => {
            println!("  done           : {stop_reason:?} usage={usage:?}")
        }
        LlmStreamEvent::Error { message, fatal } => {
            println!("  ERROR (fatal={fatal}): {message}")
        }
    }
}

#[tokio::main]
async fn main() {
    let Some(model) = cfg("WCODE_MODEL") else {
        eprintln!(
            "set WCODE_MODEL (plus WCODE_BASE_URL / WCODE_API_KEY for a real endpoint) to run the probe"
        );
        std::process::exit(1);
    };
    let opts = LlmOpts {
        model,
        base_url: cfg("WCODE_BASE_URL"),
        api_key: cfg("WCODE_API_KEY"),
        endpoint: endpoint(),
        effort: cfg("WCODE_EFFORT"),
        session_id: Some("responses-probe".to_string()),
        ..LlmOpts::default()
    };
    let prompt = cfg("WCODE_PROMPT")
        .unwrap_or_else(|| "Think step by step, then reply with exactly: pong".to_string());

    println!("endpoint : {:?}", opts.endpoint);
    println!(
        "base_url : {}",
        opts.base_url.as_deref().unwrap_or("(default openai)")
    );
    println!("model    : {}", opts.model);
    println!("effort   : {}", opts.effort.as_deref().unwrap_or("(none)"));
    println!();

    let mut stream = rig_stream_fn()(
        &[AgentMessage::user_text(prompt)],
        "You are terse.",
        &[],
        &opts,
    );

    let mut saw_thinking = false;
    let mut saw_empty_replace = false;
    while let Some(ev) = stream.next().await {
        print_event(&ev);
        match &ev {
            LlmStreamEvent::ThinkingDelta(_) => saw_thinking = true,
            LlmStreamEvent::ThinkingReplace(text) => {
                saw_empty_replace |= text.is_empty();
                saw_thinking |= !text.is_empty();
            }
            _ => {}
        }
    }

    println!();
    if saw_empty_replace {
        println!("FAIL: a ThinkingReplace(\"\") arrived — it would erase the streamed deltas");
    } else if saw_thinking {
        println!("PASS: reasoning surfaced (ThinkingDelta / non-empty ThinkingReplace)");
    } else {
        println!("NOTE: no reasoning arrived (the model may not emit a summary on this wire)");
    }
}
