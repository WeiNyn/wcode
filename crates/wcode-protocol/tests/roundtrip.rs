//! End-to-end: a session served over a Unix socket, driven by a remote client.
//!
//! Proves the S2 claim — the local in-process backend and the remote socket
//! backend speak the same protocol: a `Client` `ask`s, streams events, and
//! reads state back exactly like a `SessionHandle` would.

#![cfg(unix)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wcode_harness::actor::SessionActor;
use wcode_harness::agent::{Agent, AgentConfig};
use wcode_harness::event::{AgentEvent, LlmStreamEvent};
use wcode_harness::message::StopReason;
use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
use wcode_protocol::Client;

fn agent(script: Vec<Vec<LlmStreamEvent>>) -> Agent {
    let script = Arc::new(Mutex::new(VecDeque::from(script)));
    let stream_fn: StreamFn = Arc::new(move |_ctx, _sys, _tools, _opts| {
        let events = script.lock().unwrap().pop_front().unwrap_or_default();
        Box::pin(futures::stream::iter(events)) as LlmStream
    });
    Agent::new(AgentConfig {
        system: "sys".into(),
        tools: Vec::new(),
        llm: LlmOpts {
            model: "m1".into(),
            ..LlmOpts::default()
        },
        stream_fn,
        hooks: wcode_harness::hooks::HooksSet::default(),
        session: None,
        context: Vec::new(),
        working_dir: std::path::PathBuf::new(),
        max_turns: wcode_harness::loop_::DEFAULT_MAX_TURNS,
        parallel_tools: true,
        compaction: wcode_harness::compaction::CompactionPolicy::default(),
    })
}

fn done() -> LlmStreamEvent {
    LlmStreamEvent::Done {
        stop_reason: StopReason::Stop,
        usage: None,
    }
}

#[tokio::test]
async fn a_remote_client_drives_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("wcode.sock");

    let handle = SessionActor::spawn(agent(vec![vec![
        LlmStreamEvent::TextDelta("hello".into()),
        done(),
    ]]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        handle,
        SessionId::new("test"),
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let mut events = client.subscribe();

    // `ask(Submit)` resolves with the run's stop reason...
    let reply = client
        .ask(Request::Submit { text: "hi".into() })
        .await
        .unwrap();
    assert!(
        matches!(
            reply,
            AgentEvent::Stopped {
                stop_reason: StopReason::Stop
            }
        ),
        "{reply:?}"
    );

    // ...while the streamed text arrived on the subscription.
    let mut saw_text = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("an event")
            .expect("open");
        match event {
            AgentEvent::MessageUpdate { message } if message.as_text() == "hello" => {
                saw_text = true;
            }
            AgentEvent::AgentEnd => break,
            _ => {}
        }
    }
    assert!(saw_text, "the streamed text reached the remote client");

    // A read-back also round-trips.
    let reply = client.ask(Request::GetHistory).await.unwrap();
    let AgentEvent::History { messages } = reply else {
        panic!("expected a History reply, got {reply:?}");
    };
    assert!(
        messages.iter().any(|m| m.as_text() == "hi"),
        "the remote read-back carries the user turn: {messages:?}"
    );
}

#[tokio::test]
async fn a_second_client_observes_the_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("wcode.sock");

    let handle = SessionActor::spawn(agent(vec![vec![
        LlmStreamEvent::TextDelta("shared".into()),
        done(),
    ]]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        handle,
        SessionId::new("test"),
        listener,
    ));

    // Two independent connections to one session.
    let a = Client::connect(&sock).await.unwrap();
    let b = Client::connect(&sock).await.unwrap();
    let mut a_events = a.subscribe();
    let mut b_events = b.subscribe();

    a.ask(Request::Submit { text: "go".into() }).await.unwrap();

    // Both subscribers see the run stream and end.
    for (name, rx) in [("a", &mut a_events), ("b", &mut b_events)] {
        let mut saw = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap_or_else(|_| panic!("{name}: an event"))
                .unwrap_or_else(|_| panic!("{name}: open"));
            match event {
                AgentEvent::MessageUpdate { message } if message.as_text() == "shared" => {
                    saw = true
                }
                AgentEvent::AgentEnd => break,
                _ => {}
            }
        }
        assert!(saw, "{name} observed the streamed text");
    }
}
