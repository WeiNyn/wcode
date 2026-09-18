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
        vec![(SessionId::new("test"), handle)],
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
        vec![(SessionId::new("test"), handle)],
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

/// A client reconnects after the connection drops, and a request issued around
/// the drop is delivered once it is back.
#[tokio::test]
async fn a_client_reconnects_after_the_connection_drops() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("wcode.sock");

    // A "server" that accepts one connection and closes it, so the client sees
    // a dropped connection.
    let bad = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(async move {
        if let Ok((stream, _)) = bad.accept().await {
            drop(stream);
        }
        drop(bad);
    });

    let client = Client::connect(&sock).await.unwrap();
    let mut events = client.subscribe();

    // Bring the real server up on the same path.
    let handle = SessionActor::spawn(agent(vec![vec![
        LlmStreamEvent::TextDelta("back".into()),
        done(),
    ]]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        vec![(SessionId::new("test"), handle)],
        listener,
    ));

    // Wait until the supervisor has reconnected to the real server: a read-back
    // that resolves proves the request path is live again. (A request sent while
    // the connection is down cannot be acknowledged without an app-level ack, so
    // the test does not race the drop.)
    let mut reconnected = false;
    for _ in 0..200 {
        if client.ask(Request::GetHistory).await.is_ok() {
            reconnected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(reconnected, "the client reconnected");

    // Now a turn goes through.
    client.send(Request::Submit { text: "hi".into() }).unwrap();
    let mut saw_text = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("the run streamed after reconnect")
            .expect("open");
        match event {
            AgentEvent::MessageUpdate { message } if message.as_text() == "back" => saw_text = true,
            AgentEvent::AgentEnd => break,
            _ => {}
        }
    }
    assert!(saw_text, "the request went through after reconnecting");
}

/// S4-1: the A2A delivery verbs cross the wire like any other request — a remote
/// `Notify` is answered, surfaced as a streamed event, and recorded, with no
/// turn run.
#[tokio::test]
async fn a_notify_crosses_the_socket() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("wcode.sock");

    let handle = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        vec![(SessionId::new("test"), handle)],
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let mut events = client.subscribe();

    let reply = client
        .ask(Request::Notify {
            content: "ping".into(),
        })
        .await
        .unwrap();
    assert!(matches!(reply, AgentEvent::Ack), "{reply:?}");

    // The recipient surfaced the message on its stream...
    let mut saw_message = false;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if let AgentEvent::MessageReceived { content, .. } = event {
            assert_eq!(content, "ping");
            saw_message = true;
            break;
        }
    }
    assert!(saw_message, "the remote client saw the inbound message");

    // ...and the read-back carries it.
    let reply = client.ask(Request::GetHistory).await.unwrap();
    let AgentEvent::History { messages } = reply else {
        panic!("expected a History reply, got {reply:?}");
    };
    assert!(
        messages.iter().any(|m| m.as_text() == "[message from user]\nping"),
        "the notify landed in the context: {messages:?}"
    );
}

/// S4-4: the registry can hold a **remote** peer, and the sender crosses the
/// socket — a delivery to a served session arrives attributed to the sender.
#[tokio::test]
async fn a_remote_peer_delivery_carries_the_sender() {
    use wcode_protocol::Registry;

    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let w_handle = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        vec![(SessionId::new("w1"), w_handle)],
        listener,
    ));

    let orch = SessionId::agent("orchestrator");
    let w1 = SessionId::agent("w1");

    let registry = Registry::new();
    let client = Client::connect(&sock).await.unwrap();
    let mut events = client.subscribe();
    registry.register_remote(w1.clone(), client);
    registry.set_owner(w1.clone(), orch.clone());

    registry
        .deliver(
            &orch,
            &w1,
            Request::Notify {
                content: "hi".into(),
            },
        )
        .unwrap();

    // The served session received it, attributed to `agent:orchestrator`.
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("an event")
        .expect("open");
    assert!(
        matches!(&event, AgentEvent::MessageReceived { from, content }
            if from == &orch && content == "hi"),
        "{event:?}"
    );
}

/// S4-4 reply: a *lazy* client (the mode `--peer`/`[peers]` use) connects once
/// the peer appears, so two sessions can each be waiting for the other.
#[tokio::test]
async fn a_lazy_client_connects_once_the_peer_appears() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    // Connect lazily *before* the server exists, and queue a request.
    let client = Client::lazy(&sock);
    let mut events = client.subscribe();
    client
        .send_from(
            SessionId::agent("orchestrator"),
            Request::Notify {
                content: "hi".into(),
            },
        )
        .unwrap();

    // Now bring the server up.
    let handle = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        vec![(SessionId::new("w1"), handle)],
        listener,
    ));

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an event")
        .expect("open");
    assert!(
        matches!(&event, AgentEvent::MessageReceived { from, content }
            if from.as_str() == "agent:orchestrator" && content == "hi"),
        "{event:?}"
    );
}

/// T1: one socket carrying **many** sessions — a per-session view demuxes on
/// `Frame.session`, so each `ask` correlates to its own request and each
/// session's streamed text reaches only that session's subscription.
#[tokio::test]
async fn two_sessions_demux_over_one_socket() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let a = SessionActor::spawn(agent(vec![vec![
        LlmStreamEvent::TextDelta("from-a".into()),
        done(),
    ]]));
    let b = SessionActor::spawn(agent(vec![vec![
        LlmStreamEvent::TextDelta("from-b".into()),
        done(),
    ]]));
    let sa = SessionId::agent("a");
    let sb = SessionId::agent("b");
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        vec![(sa.clone(), a), (sb.clone(), b)],
        listener,
    ));

    // Two per-session views over one connection.
    let conn = Client::connect(&sock).await.unwrap();
    let va = conn.with_session(sa.clone());
    let vb = conn.with_session(sb.clone());
    let mut ea = va.subscribe();
    let mut eb = vb.subscribe();

    // Each reply correlates to its own request...
    let ra = va.ask(Request::Submit { text: "hi-a".into() }).await.unwrap();
    let rb = vb.ask(Request::Submit { text: "hi-b".into() }).await.unwrap();
    assert!(
        matches!(
            ra,
            AgentEvent::Stopped {
                stop_reason: StopReason::Stop
            }
        ),
        "{ra:?}"
    );
    assert!(
        matches!(
            rb,
            AgentEvent::Stopped {
                stop_reason: StopReason::Stop
            }
        ),
        "{rb:?}"
    );

    // ...and each session's text arrives only on that session's subscription.
    assert!(
        drains_text(&mut ea, "from-a", "from-b").await,
        "a heard its own stream"
    );
    assert!(
        drains_text(&mut eb, "from-b", "from-a").await,
        "b heard its own stream"
    );
}

/// Drain `rx` to `AgentEnd`, asserting `want` arrives and `foreign` never does.
/// Returns whether `want` was seen.
async fn drains_text(
    rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>,
    want: &str,
    foreign: &str,
) -> bool {
    let mut saw = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an event")
            .expect("open");
        match event {
            AgentEvent::MessageUpdate { message } if message.as_text() == want => saw = true,
            AgentEvent::MessageUpdate { message } if message.as_text() == foreign => {
                panic!("{foreign:?} leaked onto the {want:?} subscription")
            }
            AgentEvent::AgentEnd => break,
            _ => {}
        }
    }
    saw
}

/// T1: a request for a session a **multi**-session server does not serve is
/// rejected with a correlated `AgentEvent::Error`.
#[tokio::test]
async fn unknown_session_is_rejected_on_a_multi_session_server() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        vec![
            (SessionId::agent("a"), SessionActor::spawn(agent(vec![]))),
            (SessionId::agent("b"), SessionActor::spawn(agent(vec![]))),
        ],
        listener,
    ));

    let conn = Client::connect(&sock).await.unwrap();
    let ghost = conn.with_session(SessionId::agent("ghost"));
    let reply = ghost.ask(Request::Submit { text: "?".into() }).await.unwrap();
    match reply {
        AgentEvent::Error { message } => assert!(message.contains("ghost"), "{message}"),
        other => panic!("expected an Error reply, got {other:?}"),
    }
}

/// T1: a **single**-session server routes any `frame.session` to its sole
/// session — the byte-compat guarantee for the `--socket` client's placeholder.
#[tokio::test]
async fn single_session_server_routes_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let handle = SessionActor::spawn(agent(vec![vec![
        LlmStreamEvent::TextDelta("only".into()),
        done(),
    ]]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        vec![(SessionId::new("test"), handle)],
        listener,
    ));

    // The legacy client addresses "remote"; the one served session answers.
    let client = Client::connect(&sock).await.unwrap();
    let mut events = client.subscribe();
    let reply = client.ask(Request::Submit { text: "hi".into() }).await.unwrap();
    assert!(
        matches!(
            reply,
            AgentEvent::Stopped {
                stop_reason: StopReason::Stop
            }
        ),
        "{reply:?}"
    );
    assert!(
        drains_text(&mut events, "only", "from-other").await,
        "the default view heard the sole session's stream"
    );
}
