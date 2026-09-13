# wcode TUI — interface redesign plan

Status: **planning**. Companion to [`next-steps.md`](next-steps.md) (item 3,
which this replaces). This is a project, not a task — the detail lives here so
it can be picked up as its own workstream.

> **See also:** [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)
> — the TUI is one client of a protocol shared with agent-to-agent communication.
> That doc reframes this plan: the TUI should be a client of `Request`/`Event`
> over a backend, never of the `Agent` directly (§12 there). The phases below
> still stand; only the seam is redrawn.
>
> **Visual spec:** [`tui-design.md`](tui-design.md) — bands, glyphs, colors,
> and the mockups each phase should hit.

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
multi-session/swarm, remote handoff, inline images, mermaid, and an iOS client.
The TUI still *consumes* the `Backend` seam (§3), so the client/server split is
not a rewrite later — but multi-surface and swarm are jcode-scale; revisit only
if wanted.

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

A **new crate `wcode-tui`** depending on **`wcode-protocol`** (not
`wcode-harness` directly). The TUI is a client of the `Backend` seam —
`Local(SessionHandle)` or `Remote(Client)` — so local vs socket is a transport
swap it never branches on. The kernel is untouched; `wcode-cli` stays the
composition root.

### The seam it consumes

The TUI talks to one thing: a `Backend`
(`../crates/wcode-protocol/src/backend.rs`), whose three verbs mirror the
REPL's use of the session.

| verb | maps to | the TUI uses it to |
|------|---------|--------------------|
| `subscribe()` → `AgentEvent` stream | the actor's outbox | drive every UI update |
| `send(Request)` / `ask(Request)` | the actor's inbox | `Submit`, `Cancel`, `SetModel`, `SetEffort`, `Compact`, `GetHistory` |
| `ask(GetHistory)` → reply | `Agent::messages()` | transcript truth / attach-replay |

Request variants are defined once in `wcode-harness/src/protocol.rs`; the CLI's
`Command` enum maps onto them. The TUI never names `Agent` — it is just another
peer on the same wire.

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
- A tick drives the spinner/status refresh **only while a run is in flight**;
  when idle there is no timer in the `select!`, so the task parks on input and
  never burns frames. Draw only when the reducer reports a visible change.
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
    lib.rs         // pub use, `run(backend, …)`
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

`wcode-cli` gains a dependency on `wcode-tui` and calls
`wcode_tui::run(backend, …)` when the TTY branch is chosen; the existing
`repl.rs` remains for pipes. The look (bands, glyphs, mockups) is specified in
[`tui-design.md`](tui-design.md).

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

## 9. Decisions

Resolved (visual ones in [`tui-design.md`](tui-design.md) §5):

- **Versions** — pin to jcode's: `ratatui 0.30` + `crossterm 0.29` (both exist).
- **Alt-screen** — yes: full-screen with a custom scrollback (P1).
- **Crate** — a new `wcode-tui` depending on `wcode-protocol`.
- **Data source** — the `Backend` seam, never `Agent` (§3). This *is* the
  client/server split the brainstorm §12 asked for; the TUI is a peer.
- **Markdown** — defer to P2 (hand-rolled minimal; decide then).
- **Clipboard** — defer to P2; OSC-52 before pulling `arboard`.

Still open:

- **Thinking** — inline vs collapsed (lean inline P0, collapsible P1).
- **Header/title bar**, **timestamps**, block separation — `tui-design.md` §6.
- **Do we retire the line REPL?** — no; keep it for pipes/`-p`, the TUI
  replaces only the interactive TTY path.

## 10. Progress

- [x] Decide open questions (versions, alt-screen, crate split, seam).
- [x] Visual spec drafted: [`tui-design.md`](tui-design.md).
- [x] Scaffold `wcode-tui`; `ratatui`+`crossterm`; terminal enter/restore guard.
- [x] `App` + reducers; headless reducer + render-snapshot tests. (P0a/P0b.)
- [x] P0c: subscribe a `Backend`, `Submit` a turn, stream into the transcript,
      `Cancel` on Esc/Ctrl-C, wire the status line. (`run(backend, status)`.)
- [x] P0 wired into `wcode-cli`: the TUI on a TTY (or `--tui`), the REPL for
      `--no-tui`/pipes/`-p`.
- [x] Status line: `model · effort · tokens` (used/limit) `· session · state`,
      with narrow-width field dropping.
- [x] P1: scrollback (PgUp/PgDn, pinned view), `/`-commands (`/exit /model
      /effort /compact /usage /help`), prompt history (Up/Down, persisted),
      multiline (Shift-Enter).
- [ ] P2 markdown + status + resize + copy.
- [ ] P3 panels + pickers.
- [ ] P4 stretch.
