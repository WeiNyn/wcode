# wcode TUI — interface redesign plan

Status: **planning**. Companion to [`next-steps.md`](next-steps.md) (item 3,
which this replaces). This is a project, not a task — the detail lives here so
it can be picked up as its own workstream.

> **See also:** [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)
> — the TUI is one client of a protocol shared with agent-to-agent communication.
> That doc reframes this plan: the TUI should be a client of `Request`/`Event`
> over a backend, never of the `Agent` directly (§12 there). The phases below
> still stand; only the seam is redrawn.

| phase | scope | status |
|-------|-------|--------|
| P0 | skeleton: alt-screen, input box, stream, status line, Ctrl-C, restore | ☐ todo |
| P1 | transcript: scrollback, thinking/tool blocks, `/`-commands, history | ☐ todo |
| P2 | polish: markdown, usage status, resize, copy | ☐ todo |
| P3 | extras: side panel, diff pane, session/model pickers | ☐ todo |
| P4 | stretch: images, mermaid, theming | ☐ todo |

---

## 1. Why

Today `wcode-cli/src/repl.rs` is a **stdin line loop**: print `❯ `, read a line,
run the agent, stream `AgentEvent`s to stdout with ANSI styling. It works, but it
hands the terminal to the OS line discipline and the native scrollback — no
full-screen layout, no controlled scrollback, no status line, no panels, no
input editing, and streaming text can interleave badly with the prompt.

The goal is a **complete TUI**: a full-screen transcript with streaming, an
input box, a status line, and scrollback — driven by the *same* `AgentEvent`
stream the REPL already consumes. The kernel (`wcode-harness`) does not change.

**Non-goals for v1** (kept deliberately out, matching wcode's "minimal" stance):
the client/server split, multi-session/swarm, remote handoff, inline images,
mermaid, and an iOS client. Those are jcode-scale; revisit only if wanted.

## 2. Reference: how jcode does it

jcode is the mature reference (`../jcode`). Facts worth carrying over:

- **Stack**: `ratatui 0.30` + `crossterm 0.29` (`event-stream`). Immediate-mode
  rendering; a **custom scrollback** (not the terminal's native one).
- **Scale**: `jcode-tui` alone is ~205K LOC; whole repo ~633K across ~80 crates.
  The TUI is their largest component — a TUI is expensive even done well.
- **Client/server split**: their TUI is a *client* over a newline-framed JSON
  protocol (`crates/jcode-tui/src/tui/backend.rs`), which is what unlocks
  multi-session/remote. Not needed for wcode's in-process kernel — our
  `AgentEvent` channel is the same seam, minus the socket.
- **Presentation crate re-exports the app core** (`jcode-tui/src/lib.rs` is
  `pub use jcode_app_core::*; pub mod tui;`) so the TUI compiles as a separate
  rustc unit while `crate::…` paths keep resolving. **Compile isolation is
  explicit** (`docs/COMPILE_TIME_ISOLATION_REFACTOR.md`,
  `docs/MODULAR_ARCHITECTURE_RFC.md`).
- **`TuiState` trait** — a 114-method data-access interface with a `TestState`
  impl, consumed as `&dyn TuiState` by ~28 render modules, for deterministic
  tests (`docs/TUISTATE_TRAIT_DECOMPOSITION.md`). We'll aim *smaller*.
- **Redraw scheduling** (`redraw_schedule.rs`): 250 ms idle → 5 s deep-idle → up
  to an FPS cap, with animation-only partial repaints.
- **Terminal quirks catalog**: `docs/TERMINAL_CAPABILITIES.md`.

## 3. Architecture for wcode

A **new crate `wcode-tui`** depending on `wcode-harness`. The kernel is
untouched; `wcode-cli` stays the composition root.

### Seams already present in the kernel

| seam | today | the TUI uses it to |
|------|-------|--------------------|
| `AgentEvent` stream (typed, `serde`) | `agent.run(prompt, tx)`; REPL drains it | drive every UI update |
| `Agent::run(prompt, tx)` | one-shot / REPL | start a turn |
| `Agent::cancel_token()` | Ctrl-C task | cancel an in-flight run |
| `Agent::messages()` / `compact()` | `/usage`, `/compact` | transcript truth, manual compaction |
| `Session` (JSONL) | `/resume`, `/sessions` | history + session picker |

This is the whole point: **the TUI is just another client of the event stream.**

### Binary / mode selection

`wcode` chooses the front-end once at startup:

- `--tui` / `--no-tui` force it.
- default: TUI when `stdout().is_terminal()` and not `-p`; otherwise the current
  REPL (piped/CI) or one-shot stay exactly as they are. **The non-TTY path is
  non-negotiable** — piping and tests depend on it.

### Event loop

```
crossterm EventStream  ─┐
AgentEvent channel     ─┼─► tokio::select! ─► reduce into App state ─► redraw?
tick interval          ─┘
```

- `crossterm::event::EventStream` (async) gives keys/mouse/resize/paste.
- The agent's `AgentEvent` receiver gives stream deltas, tool lifecycle, turns.
- A tick drives animation/status refresh; separate "real change" from
  "animation only" so idle doesn't burn frames (jcode's `redraw_schedule`).
- Render is immediate-mode: compose a `Frame`, run `terminal.draw` only when the
  state (or tick policy) says so.

### State model

One `App` struct — small and flat: transcript (a `Vec` of rendered blocks +
a live streaming block), scroll offset, input buffer + cursor, status fields
(model, effort, tokens, session path), and a `running` flag. Updates are pure
reducer functions over `AgentEvent`/key events (testable without a terminal).
Extract submodules only if it grows — **do not** build a 100-method god-trait.

### Terminal lifecycle

Enter on start, restore on **every** exit path (normal, error, panic, signal):
raw mode, alternate screen, bracketed paste, mouse capture, cursor visibility,
and an SGR reset. Use a `Drop` guard plus a panic hook (`docs/TERMINAL_
CAPABILITIES.md` §recommendations).

## 4. Crate & module layout

```
crates/wcode-tui/
  src/
    lib.rs         // pub use, `run(llm, hooks, tools, compaction, …)`
    app.rs         // App state + reducers over AgentEvent / key events
    event.rs       // crossterm events + agent events → AppEvent
    terminal.rs    // raw mode / alt screen / restore guard
    ui/
      mod.rs       // frame composition (layout)
      transcript.rs
      input.rs
      status.rs
    markdown.rs    // P2 — message rendering
```

`wcode-cli` gains a dependency on `wcode-tui` and calls `wcode_tui::run(...)`
when the TTY branch is chosen; the existing `repl.rs` remains for pipes.

## 5. Feature phasing

**P0 — skeleton (the "does it feel alive" gate).**
- Alt-screen + raw mode + bracketed paste + restore guard.
- Input box; Enter submits a prompt; agent runs; assistant text streams into a
  live transcript block; tool calls show a dim one-line status.
- Status line: model, effort, run state.
- Ctrl-C cancels the run; Ctrl-C when idle exits. Resize doesn't corrupt.

**P1 — transcript & commands.**
- Scrollback (offset into the rendered transcript), follow-tail while streaming.
- Thinking + tool blocks styled and collapsible.
- `/`-command palette mirroring the REPL (`/exit /new /model /models /effort
  /resume /sessions /usage /compact /reload`) with completion.
- Prompt history (persisted beside sessions); multiline (Shift-Enter / `\`).

**P2 — polish.**
- Markdown rendering of completed messages (own minimal renderer, see open qs).
- Usage/token status; context-window bar.
- Copy: mouse selection + OSC-52 or clipboard.
- Theme detection (truecolor via `COLORTERM`), graceful 256-color fallback.

**P3 — extras.**
- Side panel (file viewer; later diff viewer).
- Diff pane for edits.
- Session picker and model picker overlays.

**P4 — stretch.** Inline images (kitty/iTerm), mermaid, configurable theming.

## 6. Terminal handling checklist

Carried straight from jcode's `TERMINAL_CAPABILITIES.md`:

- Set the background SGR **before** any erase (`\e[K`/`\e[J`/`\e[2J`).
- Detect truecolor via `COLORTERM` (`truecolor`/`24bit`), not terminfo.
- Treat emoji/double-width defensively (Unicode 15.1+ width tables); pad or
  avoid in grid-aligned areas.
- **Full redraw on `SIGWINCH`** — don't try to patch incrementally.
- Always restore terminal state on exit, including SIGTERM/SIGINT.
- Test under `tmux`; degrade for Terminal.app / `screen` (256-color, ASCII).

## 7. Testing

- **Pure reducers**: feed `AgentEvent`/key sequences into `App`, assert state.
- **Render snapshots**: ratatui's `TestBackend` buffer diff for layout stability.
- **Headless loop**: drive the event loop with synthetic events, no real term.
- **Manual TTY smoke** per phase on macOS Terminal/iTerm + tmux.

## 8. Dependencies

- `ratatui` (0.30) and `crossterm` (0.29, `event-stream`) — the entire point of
  the crate; keep them **confined to `wcode-tui`** so the kernel and the piped
  REPL stay lean.
- Possibly a clipboard crate (`arboard`, as jcode uses) or OSC-52 to avoid one —
  decide at P2.

## 9. Open questions

- **ratatui/crossterm versions** — pin to jcode's (`0.30`/`0.29`) or the latest?
- **Alt-screen vs inline** — full-screen alt-screen (jcode) or an inline
  renderer that preserves normal scrollback? Full-screen is simpler and matches
  the goal.
- **Markdown** — hand-rolled minimal vs `pulldown-cmark` + our styling vs a
  ready `tui-markdown`-style crate.
- **Crate** — new `wcode-tui` (recommended, mirrors jcode's isolation) vs a
  module inside `wcode-cli`.
- **Clipboard** — `arboard` dep vs OSC-52 escape.
- **Client/server seam** — out of scope now; note only that the `AgentEvent`
  channel keeps the door open.
- **Do we retire the line REPL?** Keep it for pipes/`-p`; the TUI replaces only
  the interactive TTY path.

## 10. Progress

- [ ] Decide open questions (versions, alt-screen, markdown, crate split).
- [ ] Scaffold `wcode-tui`; `ratatui`+`crossterm`; terminal enter/restore guard.
- [ ] `App` + reducers over `AgentEvent`; headless tests.
- [ ] P0 skeleton wired into `wcode-cli` behind the TTY branch.
- [ ] P1 transcript + commands + history.
- [ ] P2 markdown + status + resize + copy.
- [ ] P3 panels + pickers.
- [ ] P4 stretch.
