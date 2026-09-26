//! End-to-end proof that the **Responses** wire surfaces reasoning, run by
//! default (no API key, no `#[ignore]`): a one-shot `std::net::TcpListener`
//! serves a canned SSE body on `127.0.0.1:0`, `rig_stream_fn()` consumes it
//! through the real rig Responses decoder, and the test asserts BOTH halves of
//! FIX B:
//!
//! (a) the request that went out asked for a reasoning summary
//!     (`"reasoning":{"summary":"auto"}`), and
//! (b) the reasoning actually reaches the kernel as events —
//!     `ThinkingDelta` ×2, then the complete `ThinkingReplace(<summary>)`, then
//!     the answer `TextDelta`, then `Done`.
//!
//! Reverting the request change fails (a); reverting `reasoning_text` /
//! `map_item` fails (b). It has teeth against the real decoder, not a mock.

use std::io::{Read, Write};
use std::net::TcpListener;

use futures::StreamExt as _;
use serde_json::json;
use wcode_harness::event::LlmStreamEvent;
use wcode_harness::message::AgentMessage;
use wcode_harness::streamfn::{LlmEndpoint, LlmOpts, rig_stream_fn};

/// The reasoning summary the canned reasoning item restates.
const SUMMARY: &str = "Let me think";

/// Read one HTTP request off `stream`: parse the headers, then the
/// `Content-Length` bytes of body. Returns the raw request body.
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut chunk).expect("read request headers");
        if n == 0 {
            break buf.len();
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk).expect("read request body");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf[header_end..]).to_string()
}

/// Stand up a one-shot SSE server: it accepts ONE connection, records the
/// request body on a channel, and writes `body` back as a complete
/// `text/event-stream` response before closing.
fn spawn_sse_server(body: String) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept one connection");
        let request_body = read_request(&mut stream);
        let _ = tx.send(request_body);
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/event-stream\r\n\
             Cache-Control: no-cache\r\n\
             Connection: close\r\n\
             Content-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
        stream.flush().expect("flush response");
    });
    (format!("http://{addr}/v1"), rx)
}

/// One SSE `data:` frame carrying a JSON payload.
fn frame(value: serde_json::Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&value).unwrap())
}

/// A minimal but *complete* Responses `CompletionResponse` (every field rig's
/// typed decode requires: id, object, created_at, status, model, output).
fn completion_response(status: &str, output: serde_json::Value) -> serde_json::Value {
    json!({
        "id": "resp_1",
        "object": "response",
        "created_at": 0,
        "status": status,
        "model": "test-model",
        "output": output,
    })
}

/// The canned SSE body: created → two reasoning-summary deltas → the complete
/// reasoning item → the answer text delta → completed → `[DONE]`.
fn canned_body() -> String {
    [
        frame(json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": completion_response("in_progress", json!([])),
        })),
        frame(json!({
            "type": "response.reasoning_summary_text.delta",
            "sequence_number": 1,
            "item_id": "rs_1",
            "output_index": 0,
            "summary_index": 0,
            "delta": "Let me",
        })),
        frame(json!({
            "type": "response.reasoning_summary_text.delta",
            "sequence_number": 2,
            "item_id": "rs_1",
            "output_index": 0,
            "summary_index": 0,
            "delta": " think",
        })),
        frame(json!({
            "type": "response.output_item.done",
            "sequence_number": 3,
            "output_index": 0,
            "item": {
                "type": "reasoning",
                "id": "rs_1",
                "summary": [{"type": "summary_text", "text": SUMMARY}],
                "status": "completed",
            },
        })),
        frame(json!({
            "type": "response.output_text.delta",
            "sequence_number": 4,
            "item_id": "msg_1",
            "output_index": 1,
            "content_index": 0,
            "delta": "pong",
        })),
        frame(json!({
            "type": "response.completed",
            "sequence_number": 5,
            "response": completion_response("completed", json!([])),
        })),
        "data: [DONE]\n\n".to_string(),
    ]
    .concat()
}

#[tokio::test]
async fn responses_wire_requests_and_renders_reasoning() {
    let (base_url, request_rx) = spawn_sse_server(canned_body());
    let opts = LlmOpts {
        model: "test-model".to_string(),
        base_url: Some(base_url),
        api_key: None, // keyless localhost uses the placeholder bearer token
        endpoint: LlmEndpoint::Responses,
        effort: None, // THE case: no effort, the summary must still be requested
        ..LlmOpts::default()
    };

    let stream = rig_stream_fn()(
        &[AgentMessage::user_text("Reply with exactly: pong")],
        "You are terse.",
        &[],
        &opts,
    );
    let events: Vec<LlmStreamEvent> =
        tokio::time::timeout(std::time::Duration::from_secs(10), stream.collect())
            .await
            .expect("stream terminates");

    // (b) — the request that reached the server asked for a reasoning summary.
    let request_body = request_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("server recorded a request");
    assert!(
        request_body.contains("\"summary\":\"auto\""),
        "the Responses request must request a reasoning summary, got: {request_body}"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&request_body).expect("request body is JSON");
    assert_eq!(
        parsed["reasoning"]["summary"], "auto",
        "reasoning.summary must be auto: {request_body}"
    );

    // (a) — the reasoning reached the kernel as events, in order and intact.
    assert_eq!(
        events.len(),
        5,
        "expected exactly 2 deltas + replace + text + done, got: {events:?}"
    );
    assert!(
        matches!(events.first(), Some(LlmStreamEvent::ThinkingDelta(t)) if t == "Let me"),
        "first event must be a reasoning delta, got: {events:?}"
    );
    assert!(
        matches!(events.get(1), Some(LlmStreamEvent::ThinkingDelta(t)) if t == " think"),
        "second event must be a reasoning delta, got: {events:?}"
    );
    assert!(
        matches!(events.get(2), Some(LlmStreamEvent::ThinkingReplace(t)) if t == SUMMARY),
        "third event must be the complete summary (not erased), got: {events:?}"
    );
    assert!(
        matches!(events.get(3), Some(LlmStreamEvent::TextDelta(t)) if t == "pong"),
        "fourth event must be the answer text, got: {events:?}"
    );
    assert!(
        matches!(events.get(4), Some(LlmStreamEvent::Done { .. })),
        "last event must be Done, got: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, LlmStreamEvent::Error { .. })),
        "no error on the reasoning path, got: {events:?}"
    );
}
