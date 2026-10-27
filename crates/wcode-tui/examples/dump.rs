//! Headless frame dumper: draw the **real** TUI (`ui::draw`, via
//! `wcode_tui::render_text` / `render_plain`) onto text at several sizes, so the
//! design can be reviewed and diffed with no live terminal.
//!
//! `cargo run -p wcode-tui --example dump` writes, under the workspace
//! `target/tui-render/`, a `<W>x<H>.ansi` (per-cell ANSI SGR, `cat`-able on a
//! real terminal) and a `<W>x<H>.txt` (symbols only) for 80×24, 120×40, and
//! 48×20 — the width ladder. Each frame is pinned to the transcript's top (a
//! burst of `PageUp` after the first draw), so a review sees the whole run
//! rather than only its tail.
//!
//! The fixture is built with only the public API (`App::new`, `AppEvent`, `Key`,
//! `SurfaceInfo` plus the harness event/message types), mirroring the in-crate
//! test helpers: a user prompt, a compaction notice, a markdown assistant block,
//! a collapsed `bash` with long output, an expanded `read`, three surfaces (one
//! running), a near-full context gauge, plan mode on, and — W029 — one prompt
//! queued behind the running turn, so the composer foot renders its
//! `⇥ 1 queued` segment.
//!
//! `-- empty` swaps the populated fixture for an EMPTY root surface (no flag =
//! the populated ladder, byte-identical): the D5/D042 banner is only on screen
//! when nothing has been committed, so the populated dump can never show it.
//! The files are then written as `empty-<W>x<H>.{txt,ansi}` alongside the rest.

use std::fs;
use std::path::PathBuf;

use wcode_harness::event::{AgentEvent, TodoItem, TodoStatus};
use wcode_harness::message::{AgentMessage, ContentBlock, StopReason, Usage};
use wcode_harness::protocol::SessionId;
use serde_json::json;
use wcode_tui::{App, AppEvent, Key, Status, SurfaceInfo};

/// The assistant's prose: a heading, bullets, inline code, a fenced block, a link.
const ASSISTANT_MD: &str = "\
# Config-loader refactor

The loader now returns `Result<Config, LoadError>` instead of panicking:

- `LoadError::Io` wraps a filesystem failure
- `LoadError::Parse` carries the offending `line` and `column`
- every caller maps it to a user-facing message

```rust
pub enum LoadError {
    Io(std::io::Error),
    Parse { line: usize, column: usize },
}
```

Rationale and edge cases are in the [design note](https://example.com/config).";

/// The assistant's reasoning — committed collapsed to a `··· thinking` row
/// (D33), so the dump shows the one-liner.
const THINKING: &str = "\
I should confirm how anchors hash indentation, then read edit.rs before \
answering; the rule is that the hash covers the raw line, so reindenting moves it.";

/// The sizes dumped — the width ladder (narrow, base, wide).
const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (48, 20)];

fn main() -> std::io::Result<()> {
    // Mirror the real TUI's `theme::install`: resolve palette B (or the
    // `NO_COLOR` plain theme) through the color-mode ladder. Without this the
    // headless path would use the static seed and ignore `NO_COLOR`.
    wcode_tui::set_theme("default").expect("the `default` preset exists");

    let out = target_dir().join("tui-render");
    fs::create_dir_all(&out)?;

    // `cargo run -p wcode-tui --example dump -- empty` selects the empty state.
    let empty = std::env::args().nth(1).as_deref() == Some("empty");
    let prefix = if empty { "empty-" } else { "" };

    for (w, h) in SIZES {
        // A fresh fixture per size: the ladder should start from one state.
        let mut app = if empty { empty_app() } else { fixture() };
        // Warm the layout so the viewport/scroll bounds are known, then pin the
        // transcript to its TOP with a burst of `PageUp`. A live TUI pins the
        // tail; a design review wants the conversation from its first block.
        // The empty state has nothing above the tail, so it skips the burst.
        let _ = wcode_tui::render_text(&mut app, w, h);
        if !empty {
            for _ in 0..128 {
                app.handle(AppEvent::Key(Key::PageUp));
            }
        }

        let ansi_path = out.join(format!("{prefix}{w}x{h}.ansi"));
        fs::write(&ansi_path, wcode_tui::render_text(&mut app, w, h))?;

        let plain_path = out.join(format!("{prefix}{w}x{h}.txt"));
        fs::write(&plain_path, wcode_tui::render_plain(&mut app, w, h))?;

        println!("{} · {}", ansi_path.display(), plain_path.display());
    }
    Ok(())
}

/// The workspace `target/` dir (honors `CARGO_TARGET_DIR`, else `../../target`).
fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target"),
    }
}

/// The other end of the ladder's range: a root surface with NO transcript, so
/// `ui::draw` seeds the empty state (the mark over the hint, D042).
fn empty_app() -> App {
    let mut app = App::new();
    app.set_surfaces(vec![SurfaceInfo {
        id: SessionId::agent("root"),
        label: "wcode".into(),
        model: "zephyr-9".into(),
        is_root: true,
    }]);
    // A root surface seeds `Status::default()` from its `SurfaceInfo` (the model
    // field is a member's), which would print `wcode   wcode` in the head — a
    // fixture artefact, not a frame. Set the status the way the composition root
    // does, so the empty-mode header reads like a real session's.
    app.set_status(Status::new("zephyr-9"));
    app
}

/// A realistic session snapshot, built through the public reducer only.
fn fixture() -> App {
    let mut app = App::new();
    let root = SessionId::agent("root");
    let explorer = SessionId::agent("explorer");
    let reviewer = SessionId::agent("reviewer");

    // Root + two members; `explorer` is mid-run, `reviewer` idles.
    app.set_surfaces(vec![
        SurfaceInfo {
            id: root.clone(),
            label: "wcode".into(),
            model: "zephyr-9".into(),
            is_root: true,
        },
        SurfaceInfo {
            id: explorer.clone(),
            label: "explorer".into(),
            model: "zephyr-9".into(),
            is_root: false,
        },
        SurfaceInfo {
            id: reviewer.clone(),
            label: "reviewer".into(),
            model: "zephyr-9-mini".into(),
            is_root: false,
        },
    ]);

    // A near-full context gauge (186k / 200k) and plan mode on.
    app.set_status(Status {
        model: "zephyr-9".into(),
        effort: Some("high".into()),
        session: Some("9f3c1a7b2e4d".into()),
        context_limit: Some(200_000),
        plan: true,
        // W031: this dump fixture is untitled.
        title: None,
    });
    app.set_cwd(Some("wcode".into()));
    app.set_git(Some("main*".into()));

    app.handle(AppEvent::Agent(
        root.clone(),
        AgentEvent::TurnEnd {
            message: usage_msg(186_000),
        },
    ));
    app.handle(AppEvent::Agent(explorer.clone(), AgentEvent::AgentStart));

    // The user's prompt, typed and submitted through the composer.
    let prompt = "Add a wrapped-error type to the config loader and thread it through.";
    for c in prompt.chars() {
        app.handle(AppEvent::Key(Key::Char(c)));
    }
    app.handle(AppEvent::Key(Key::Enter));
    let _ = app.take_actions();

    // A compaction notice at the turn's start.
    app.handle(AppEvent::Agent(
        root.clone(),
        AgentEvent::Compaction {
            summarized: 12,
            kept: 4,
        },
    ));

    // The live todo checklist (D35): a first-class block in the transcript.
    app.handle(AppEvent::Agent(
        root.clone(),
        AgentEvent::Todo {
            todos: vec![
                TodoItem {
                    content: "map the seam".into(),
                    status: TodoStatus::Pending,
                },
                TodoItem {
                    content: "write the test".into(),
                    status: TodoStatus::Completed,
                },
                TodoItem {
                    content: "run the suite".into(),
                    status: TodoStatus::Pending,
                },
            ],
        },
    ));

    // The assistant turn: markdown prose plus two tool calls (their arguments
    // ride the block, so each tool line names its target).
    app.handle(AppEvent::Agent(
        root.clone(),
        AgentEvent::MessageEnd {
            message: AgentMessage::Assistant {
                content: vec![
                    ContentBlock::Thinking {
                        text: THINKING.into(),
                    },
                    ContentBlock::Text {
                        text: ASSISTANT_MD.into(),
                    },
                    ContentBlock::ToolCall {
                        id: "b1".into(),
                        name: "bash".into(),
                        arguments: json!({ "command": "cargo test --workspace" }),
                    },
                    ContentBlock::ToolCall {
                        id: "r1".into(),
                        name: "read".into(),
                        arguments: json!({ "path": "crates/wcode-cli/src/config.rs" }),
                    },
                ],
                stop_reason: StopReason::ToolUse,
                usage: None,
                model: None,
            },
        },
    ));

    // A collapsed `bash` with long output, then a `read` we expand.
    tool(&mut app, &root, "b1", "bash", &bash_output(), false);
    tool(&mut app, &root, "r1", "read", READ_OUTPUT, false);

    // Expand the last tool via browse (Ctrl-G selects it, Enter toggles, Esc
    // returns to the composer) — the only public path to `expanded`.
    app.handle(AppEvent::Key(Key::Ctrl('g')));
    app.handle(AppEvent::Key(Key::Enter));
    app.handle(AppEvent::Key(Key::Esc));

    // Dock the sidebar (Ctrl-B), so every frame shows the team column — the
    // `wcode` root row plus the badged members. Below `SIDEBAR_MIN_WIDTH` the
    // panel never docks, so the 48x20 frame keeps its base layout.
    app.handle(AppEvent::Key(Key::Ctrl('b')));

    // W029: a prompt typed MID-RUN is queued, not discarded — the root is still
    // running (no `AgentEnd` above), so this second `Enter` enqueues. The frame
    // then proves both halves of the design: the composer foot's `⇥ 1 queued`
    // count, and the `queued · 1 waiting` notice — while the transcript still
    // holds NO block for the queued prompt (it is pushed only when it starts).
    for c in "Also add a regression test for the wrap-around case.".chars() {
        app.handle(AppEvent::Key(Key::Char(c)));
    }
    app.handle(AppEvent::Key(Key::Enter));
    let _ = app.take_actions();

    app
}

/// Push one finished tool invocation (start → end) into `surface`.
fn tool(app: &mut App, surface: &SessionId, call_id: &str, name: &str, output: &str, is_error: bool) {
    app.handle(AppEvent::Agent(
        surface.clone(),
        AgentEvent::ToolExecutionStart {
            call_id: call_id.into(),
            name: name.into(),
        },
    ));
    app.handle(AppEvent::Agent(
        surface.clone(),
        AgentEvent::ToolExecutionEnd {
            call_id: call_id.into(),
            name: name.into(),
            output: output.into(),
            is_error,
            diff: None,
            path: None,
            duration_ms: Some(1_250),
        },
    ));
}

/// A finished turn reporting context usage — the source of the status gauge.
fn usage_msg(input_tokens: u64) -> AgentMessage {
    AgentMessage::Assistant {
        content: Vec::new(),
        stop_reason: StopReason::Stop,
        usage: Some(Usage {
            input_tokens,
            output_tokens: 0,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }),
        model: None,
    }
}

/// 30 lines — long enough that the collapsed `bash` elides the tail; a trailing
/// `exit code: 0` so the collapsed head's outcome note reads `exit 0 · …`.
fn bash_output() -> String {
    let mut out = (1..=30)
        .map(|i| format!("test result_{i:02} ... ok    {} passed; 0 failed", i * 7 % 53))
        .collect::<Vec<_>>()
        .join("\n");
    out.push_str("\nexit code: 0");
    out
}

/// A short file excerpt, shown expanded. The `read` form (`{n}\t{line}`), so the
/// collapsed head's outcome note reads `<N> lines · …`.
const READ_OUTPUT: &str = "\
1\tpub struct Config {
2\t    pub model: String,
3\t    pub base_url: Option<String>,
4\t}
5\timpl Config {
6\t    pub fn load(path: &Path) -> Result<Self, LoadError> {
7\t        let raw = std::fs::read_to_string(path)?;
8\t        toml::from_str(&raw).map_err(Into::into)
9\t    }
10\t}";
