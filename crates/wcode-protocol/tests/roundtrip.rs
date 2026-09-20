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

/// Spawn a server with a **frozen** roster: the first session is the root, the
/// rest are already-present extras. The common case — the live-roster tests
/// (`a_late_registered_session_reaches_the_client`,
/// `list_sessions_reflects_a_late_registration`) build the `watch` themselves.
fn serve_static(
    sessions: Vec<(SessionId, wcode_harness::actor::SessionHandle)>,
    listener: tokio::net::UnixListener,
) {
    let mut sessions = sessions.into_iter();
    let root = sessions.next().expect("at least one session");
    let rest: Vec<_> = sessions.collect();
    tokio::spawn(wcode_protocol::serve(
        wcode_protocol::Registry::new(),
        tokio::sync::watch::channel(rest).1,
        root,
        None,
        listener,
    ));
}

/// The ids of a roster reply, in order — `Sessions` now carries `SessionInfo`s.
fn ids_of(sessions: &[wcode_harness::protocol::SessionInfo]) -> Vec<SessionId> {
    sessions.iter().map(|s| s.id.clone()).collect()
}

/// Drain `rx` until a `MessageReceived` arrives (skipping a leading roster
/// push), returning `(from, content)`. Panics on timeout.
async fn next_message(
    rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>,
) -> (SessionId, String) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an event")
            .expect("open");
        if let AgentEvent::MessageReceived { from, content } = event {
            return (from, content);
        }
    }
}

#[tokio::test]
async fn bind_sets_owner_only_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("wcode.sock");
    let _listener = wcode_protocol::bind(&sock).await.unwrap();
    // The socket file is the session's only credential; a default umask would
    // leave it group/world-reachable.
    let mode = std::fs::metadata(&sock).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "socket must be owner-only");
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
    serve_static(
        vec![(SessionId::new("test"), handle)],
        listener,
    );

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
    serve_static(
        vec![(SessionId::new("test"), handle)],
        listener,
    );

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
    serve_static(
        vec![(SessionId::new("test"), handle)],
        listener,
    );

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
    serve_static(
        vec![(SessionId::new("test"), handle)],
        listener,
    );

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
    serve_static(
        vec![(SessionId::new("w1"), w_handle)],
        listener,
    );

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
    let (from, content) = next_message(&mut events).await;
    assert!(
        from == orch && content == "hi",
        "from={from:?} content={content:?}"
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
    serve_static(
        vec![(SessionId::new("w1"), handle)],
        listener,
    );

    let (from, content) = next_message(&mut events).await;
    assert!(
        from.as_str() == "agent:orchestrator" && content == "hi",
        "from={from:?} content={content:?}"
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
    serve_static(
        vec![(sa.clone(), a), (sb.clone(), b)],
        listener,
    );

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
    serve_static(
        vec![
            (SessionId::agent("a"), SessionActor::spawn(agent(vec![]))),
            (SessionId::agent("b"), SessionActor::spawn(agent(vec![]))),
        ],
        listener,
    );

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
    serve_static(
        vec![(SessionId::new("test"), handle)],
        listener,
    );

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

/// T2: the server answers `ListSessions` with its roster, in serve order (root
/// first), before any per-session demux.
#[tokio::test]
async fn list_sessions_returns_the_roster() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let root = SessionId::new("root");
    let a = SessionId::agent("a");
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    serve_static(
        vec![
            (root.clone(), SessionActor::spawn(agent(vec![]))),
            (a.clone(), SessionActor::spawn(agent(vec![]))),
        ],
        listener,
    );

    // The legacy client addresses "remote"; the roster is answered regardless.
    let client = Client::connect(&sock).await.unwrap();
    let reply = client.ask(Request::ListSessions).await.unwrap();
    let AgentEvent::Sessions { sessions } = reply else {
        panic!("expected a Sessions reply, got {reply:?}");
    };
    assert_eq!(ids_of(&sessions), vec![root, a]);
}

/// A session registered **before** the server's first `subscribe()` — the CLI's
/// `[team]` startup order (members start ahead of `serve`) — must still be served:
/// the registry seeds the watch with the roster even with no subscriber yet.
/// (`watch::Sender::send` would drop it; `send_replace` keeps it.)
#[tokio::test]
async fn a_session_registered_before_the_server_is_served() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let registry = wcode_protocol::Registry::new();
    let root_id = SessionId::new("root");
    let w1 = SessionId::agent("w1");
    // The member is registered with no subscriber in existence yet.
    registry.register(w1.clone(), SessionActor::spawn(agent(vec![])));

    let root = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        registry.clone(),
        registry.subscribe(),
        (root_id.clone(), root),
        None,
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let reply = client.ask(Request::ListSessions).await.unwrap();
    let AgentEvent::Sessions { sessions } = reply else {
        panic!("expected a Sessions reply, got {reply:?}");
    };
    assert_eq!(ids_of(&sessions), vec![root_id, w1]);
}

/// S2: the served roster names each session's **model**, so a client can label a
/// member by its own model instead of substituting the root's. A session the
/// registry knows no model for is `None` (the client then falls back).
#[tokio::test]
async fn the_roster_carries_each_sessions_model() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let registry = wcode_protocol::Registry::new();
    let root_id = SessionId::new("root");
    let root = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        registry.clone(),
        registry.subscribe(),
        (root_id.clone(), root),
        None,
        listener,
    ));

    // The root and one worker have a known model; a second worker has none.
    let w1 = SessionId::agent("w1");
    let w2 = SessionId::agent("w2");
    registry.set_model(root_id.clone(), "m1");
    registry.register(w1.clone(), SessionActor::spawn(agent(vec![])));
    registry.set_model(w1.clone(), "m2");
    registry.register(w2.clone(), SessionActor::spawn(agent(vec![])));

    let client = Client::connect(&sock).await.unwrap();
    let reply = client.ask(Request::ListSessions).await.unwrap();
    let AgentEvent::Sessions { sessions } = reply else {
        panic!("expected a Sessions reply, got {reply:?}");
    };
    assert_eq!(ids_of(&sessions), vec![root_id.clone(), w1.clone(), w2.clone()]);
    let models: Vec<Option<&str>> = sessions.iter().map(|s| s.model.as_deref()).collect();
    assert_eq!(models, vec![Some("m1"), Some("m2"), None], "{sessions:?}");
}

/// T2: a registration that races the connection's snapshot is still served — the
/// id is folded into the connect-time snapshot, fanned by the initial fan, and
/// named by the seed push. (The **growth** push, for a registration *after* the
/// snapshot, is covered by `a_growth_push_reaches_an_established_client`.)
#[tokio::test]
async fn a_registration_racing_the_connect_is_served() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let registry = wcode_protocol::Registry::new();
    let root_id = SessionId::new("root");
    let root = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        registry.clone(),
        registry.subscribe(),
        (root_id.clone(), root),
        None,
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let mut roster = client.subscribe_roster();

    // A worker appears *after* the connection: register it into the live
    // registry (exactly what a runtime `spawn` does).
    let w1 = SessionId::agent("w1");
    registry.register(w1.clone(), SessionActor::spawn(agent(vec![])));

    // The client learns the new id from the pushed roster.
    tokio::time::timeout(Duration::from_secs(5), roster.changed())
        .await
        .expect("a roster change")
        .expect("open");
    assert!(
        roster.borrow().iter().any(|s| s.id == w1),
        "the pushed roster names the late session: {:?}",
        roster.borrow()
    );

    // ...and the new session's events reach this connection's fan for it.
    let mut events = client.with_session(w1.clone()).subscribe();
    client
        .with_session(w1.clone())
        .send(Request::Notify {
            content: "hi".into(),
        })
        .unwrap();
    let mut saw = false;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if let AgentEvent::MessageReceived { content, .. } = event {
            assert_eq!(content, "hi");
            saw = true;
            break;
        }
    }
    assert!(saw, "an event from the late session reached the client");
}

/// T2: `ListSessions` is answered from the **live** roster, so a session
/// registered after the server started appears in order (root first). This
/// covers the live read; the pushed roster is covered by
/// `a_growth_push_reaches_an_established_client`.
#[tokio::test]
async fn list_sessions_reads_the_live_roster() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let registry = wcode_protocol::Registry::new();
    let root_id = SessionId::new("root");
    let root = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        registry.clone(),
        registry.subscribe(),
        (root_id.clone(), root),
        None,
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let w1 = SessionId::agent("w1");
    registry.register(w1.clone(), SessionActor::spawn(agent(vec![])));

    // Wait until the server has observed the change (its push reaches us)...
    let mut roster = client.subscribe_roster();
    tokio::time::timeout(Duration::from_secs(5), roster.changed())
        .await
        .expect("a roster change")
        .expect("open");

    // ...then `ListSessions` reflects it, root first.
    let reply = client.ask(Request::ListSessions).await.unwrap();
    let AgentEvent::Sessions { sessions } = reply else {
        panic!("expected a Sessions reply, got {reply:?}");
    };
    assert_eq!(ids_of(&sessions), vec![root_id, w1]);
}

/// T2: a session registered **after** the connection's snapshot is fanned and
/// pushed to the client — the growth path. The `ListSessions` round-trip below
/// forces the connection task to take its snapshot (root only) first, so the
/// late id can only arrive through the roster watch (unlike
/// `a_registration_racing_the_connect_is_served`).
#[tokio::test]
async fn a_growth_push_reaches_an_established_client() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let registry = wcode_protocol::Registry::new();
    let root_id = SessionId::new("root");
    let root = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        registry.clone(),
        registry.subscribe(),
        (root_id.clone(), root),
        None,
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    // Force the server's connection task to take its snapshot (root only) before
    // the late registration: a round-trip needs the read loop, which the
    // connection reaches only after the snapshot.
    client.ask(Request::ListSessions).await.unwrap();

    let mut roster = client.subscribe_roster();
    let w1 = SessionId::agent("w1");
    registry.register(w1.clone(), SessionActor::spawn(agent(vec![])));

    // The growth push carries the grown roster (awaited past any seed push).
    let sessions = loop {
        let sessions = roster.borrow_and_update().clone();
        if sessions.iter().any(|s| s.id == w1) {
            break sessions;
        }
        tokio::time::timeout(Duration::from_secs(5), roster.changed())
            .await
            .expect("a roster change")
            .expect("open");
    };
    assert_eq!(ids_of(&sessions), vec![root_id.clone(), w1.clone()]);

    // ...and the late session's events reach this connection's fan for it.
    let mut events = client.with_session(w1.clone()).subscribe();
    client
        .with_session(w1.clone())
        .send(Request::Notify {
            content: "hi".into(),
        })
        .unwrap();
    let mut saw = false;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if let AgentEvent::MessageReceived { content, .. } = event {
            assert_eq!(content, "hi");
            saw = true;
            break;
        }
    }
    assert!(saw, "an event from the grown session reached the client");
}

/// R1: `Define` is intercepted by the socket server and answered by its injected
/// handler, which builds a worker and registers it — so the new id is pushed to
/// the client's roster like any other late session.
#[tokio::test]
async fn a_define_request_spawns_on_a_served_peer() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let registry = wcode_protocol::Registry::new();
    let root_id = SessionId::new("root");
    let root = SessionActor::spawn(agent(vec![]));
    // The injected handler registers a worker into the served registry (exactly
    // what the CLI's `Orchestrator` factory does), so it is served too.
    let spawn_registry = registry.clone();
    let handler: wcode_protocol::DefineHandler = Arc::new(move |args| {
        let name = args.name.unwrap_or_else(|| "w1".into());
        let id = SessionId::agent(name);
        spawn_registry.register(id.clone(), SessionActor::spawn(agent(vec![])));
        Ok(id)
    });
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        registry.clone(),
        registry.subscribe(),
        (root_id.clone(), root),
        Some(handler),
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let mut roster = client.subscribe_roster();
    // Round-trip first, so the connection's snapshot is root-only: the new id
    // must arrive through growth.
    client.ask(Request::ListSessions).await.unwrap();

    let reply = client
        .ask(Request::Define {
            name: Some("w1".into()),
            model: Some("m".into()),
            role: None,
            tools: None,
            base_url: None,
            api_key: None,
        })
        .await
        .unwrap();
    let AgentEvent::Spawned { worker: id } = reply else {
        panic!("expected a Spawned reply, got {reply:?}");
    };
    assert_eq!(id, SessionId::agent("w1"));

    // The defined worker is served: it appears in the pushed roster.
    let sessions = loop {
        let sessions = roster.borrow_and_update().clone();
        if sessions.iter().any(|s| s.id == id) {
            break sessions;
        }
        tokio::time::timeout(Duration::from_secs(5), roster.changed())
            .await
            .expect("a roster change")
            .expect("open");
    };
    assert_eq!(ids_of(&sessions), vec![root_id.clone(), id.clone()]);
}

/// R1: `Define` with no handler installed is a correlated error, not a silent
/// drop — an older or non-orchestrator server must say so.
#[tokio::test]
async fn define_without_a_handler_is_a_correlated_error() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");

    let root = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        wcode_protocol::Registry::new(),
        tokio::sync::watch::channel(Vec::new()).1,
        (SessionId::new("root"), root),
        None,
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let reply = client
        .ask(Request::Define {
            name: Some("w1".into()),
            model: None,
            role: None,
            tools: None,
            base_url: None,
            api_key: None,
        })
        .await
        .unwrap();
    match reply {
        AgentEvent::Error { message } => assert!(message.contains("not enabled"), "{message}"),
        other => panic!("expected an Error reply, got {other:?}"),
    }
}

/// Item 11 (smoke): with a connected peer, `flush` returns promptly — it does not
/// wait out its timeout — and the request ordered behind it is delivered. The
/// barrier's *ordering* guarantee is proven by `a_flush_waits_behind_an_unwritten_frame`
/// and its bound by `flush_is_bounded_when_the_peer_is_unreachable`.
#[tokio::test]
async fn a_flush_returns_promptly_when_connected() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");
    let handle = SessionActor::spawn(agent(vec![]));
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    tokio::spawn(wcode_protocol::serve(
        wcode_protocol::Registry::new(),
        tokio::sync::watch::channel(Vec::new()).1,
        (SessionId::new("test"), handle),
        None,
        listener,
    ));

    let client = Client::connect(&sock).await.unwrap();
    let mut events = client.subscribe();

    // Fire-and-forget, then the barrier: the Notify is written before `flush`
    // returns, so the server has observed it.
    client
        .send(Request::Notify {
            content: "hi".into(),
        })
        .unwrap();
    let start = std::time::Instant::now();
    client.flush(Duration::from_secs(2)).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "flush returned promptly when connected: {elapsed:?}"
    );

    // The server observed it: the Notify is surfaced on the stream (after the
    // connect-time roster seed).
    let (_, content) = next_message(&mut events).await;
    assert_eq!(content, "hi");
}

/// Item 11: `flush` against a peer that is not listening returns within the
/// timeout — it must never hang a one-shot exit. It *waits* because the barrier
/// is queued behind a supervisor stuck reconnecting, which also proves the
/// barrier is actually enqueued and awaited (a no-op flush would return at once).
#[tokio::test]
async fn flush_is_bounded_when_the_peer_is_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("nobody.sock");
    // Lazy: never fails; the supervisor keeps retrying the dead path and never
    // reaches the queued barrier.
    let client = Client::lazy(&sock);
    let timeout = Duration::from_millis(300);
    let start = std::time::Instant::now();
    client.flush(timeout).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed >= timeout / 2,
        "flush waited for the barrier: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "flush gave up within the timeout rather than hanging: {elapsed:?}"
    );
}

/// Item 11 (load-bearing): `flush` does not complete while an earlier frame is
/// still unwritten. A peer that accepts but never reads jams the supervisor's
/// write of a frame larger than any socket buffer, so a `flush` queued behind it
/// must **wait** — a no-op flush would return at once and this assertion fails.
#[tokio::test]
async fn a_flush_waits_behind_an_unwritten_frame() {
    use tokio::io::AsyncReadExt as _;

    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("w.sock");
    let listener = wcode_protocol::bind(&sock).await.unwrap();
    // Accept the connection but do not read until released, so the client's write
    // of an oversized frame blocks in the supervisor.
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = release_rx.await;
        // Drain so the client's blocked write completes.
        let mut buf = [0u8; 8192];
        while matches!(stream.read(&mut buf).await, Ok(n) if n > 0) {}
    });

    let client = Client::connect(&sock).await.unwrap();
    // Far larger than any unix-socket buffer: the write cannot complete yet.
    client
        .send(Request::Notify {
            content: "x".repeat(8 << 20),
        })
        .unwrap();

    let timeout = Duration::from_millis(150);
    let start = std::time::Instant::now();
    client.flush(timeout).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed >= timeout / 2,
        "flush waited behind the unwritten frame: {elapsed:?}"
    );

    // Release the reader: the frame drains and a fresh flush completes promptly.
    let _ = release_tx.send(());
    let start = std::time::Instant::now();
    client.flush(Duration::from_secs(2)).await;
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "flush completed once the reader drained"
    );
}
