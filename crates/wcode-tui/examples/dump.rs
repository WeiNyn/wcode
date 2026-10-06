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
//! running), a near-full context gauge, and plan mode on.

use std::fs;
use std::path::PathBuf;

use wcode_harness::event::AgentEvent;
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

/// The sizes dumped — the width ladder (narrow, base, wide).
const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (48, 20)];

fn main() -> std::io::Result<()> {
    // Mirror the real TUI's `theme::install`: resolve palette B (or the
    // `NO_COLOR` plain theme) through the color-mode ladder. Without this the
    // headless path would use the static seed and ignore `NO_COLOR`.
    wcode_tui::set_theme("default").expect("the `default` preset exists");

    let out = target_dir().join("tui-render");
    fs::create_dir_all(&out)?;

    for (w, h) in SIZES {
        // A fresh fixture per size: the ladder should start from one state.
        let mut app = fixture();
        // Warm the layout so the viewport/scroll bounds are known, then pin the
        // transcript to its TOP with a burst of `PageUp`. A live TUI pins the
        // tail; a design review wants the conversation from its first block.
        let _ = wcode_tui::render_text(&mut app, w, h);
        for _ in 0..128 {
            app.handle(AppEvent::Key(Key::PageUp));
        }

        let ansi_path = out.join(format!("{w}x{h}.ansi"));
        fs::write(&ansi_path, wcode_tui::render_text(&mut app, w, h))?;

        let plain_path = out.join(format!("{w}x{h}.txt"));
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
            label: "root".into(),
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

    // The assistant turn: markdown prose plus two tool calls (their arguments
    // ride the block, so each tool line names its target).
    app.handle(AppEvent::Agent(
        root.clone(),
        AgentEvent::MessageEnd {
            message: AgentMessage::Assistant {
                content: vec![
                    ContentBlock::Text {
                        text: ASSISTANT_MD.into(),
                    },
                    ContentBlock::ToolCall {
                        id: "b1".into(),
                        name: "bash".into(),
                        arguments: json!({ "command": "cargo test --workspace 2>&1 | tail -40" }),
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

/// 30 lines — long enough that the collapsed `bash` preview elides the tail.
fn bash_output() -> String {
    (1..=30)
        .map(|i| format!("test result_{i:02} ... ok    {} passed; 0 failed", i * 7 % 53))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A short file excerpt, shown expanded.
const READ_OUTPUT: &str = "\
  1  pub struct Config {
  2      pub model: String,
  3      pub base_url: Option<String>,
  4  }
  5  impl Config {
  6      pub fn load(path: &Path) -> Result<Self, LoadError> {
  7          let raw = std::fs::read_to_string(path)?;
  8          toml::from_str(&raw).map_err(Into::into)
  9      }
 10  }";
