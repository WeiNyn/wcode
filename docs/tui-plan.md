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
| P3 | extras: overlays + pickers, diff rendering, run changeset, session picker | ☑ P3 done |
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
    lib.rs         // the event loop; `run(backend, status, history)`
    app.rs         // App state + pure reducers (AgentEvent / key) + Actions
    event.rs       // crossterm events → AppEvent
    terminal.rs    // raw mode / alt screen / restore guard
    ui.rs          // frame composition (the three bands)
    markdown.rs    // message rendering (headings, bullets, code, tables)
    clipboard.rs   // OSC-52 copy (base64)
```

`wcode-cli` gains a dependency on `wcode-tui` and calls
`wcode_tui::run(backend, status, history)` when the TTY branch is chosen; the
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
- `/`-commands: the TUI ships `/exit /model /effort /compact /usage /copy
  /help`; the session-lifecycle ones (`/new /resume /sessions /reload`) stay in
  the REPL (they need the composition root).
- Prompt history (persisted beside sessions); multiline (Shift-Enter — the `\`
  continuation is not implemented).

**P2 — polish.**
- Markdown rendering of messages, live and committed (own minimal renderer).
- Usage/token status; context-window bar.
- Copy: OSC-52 (`/copy`; no mouse selection).
- Theme detection (truecolor via `COLORTERM`), graceful 256-color fallback.

**P3 — extras.**

The TUI is a *client of a `Backend`*: it owns no filesystem, no `LlmOpts`, no
session dir, no model list. Every P3 feature wants a capability the session does
not expose, so the phase is mostly presentation plus **capabilities injected by
the composition root** (the same shape P0–P2 already use for `status`/`history`),
not protocol growth. Two forks, both resolved the conservative way:

- **Inject, don't extend the protocol.** A model list and a session list are
  things the *client* can fetch (`list_models`, `list_sessions`); pushing
  `ListModels`/`ListSessions` into the actor would pull the endpoint and the
  session dir into the kernel — the multi-session scope §1 excludes. Revisit only
  when the socket TUI needs it (the same reasoning that defers the O(n²) wire
  deltas).
- **The session picker is a re-exec handoff.** The TUI cannot rebuild an agent;
  a picker selection emits an Action ("quit and `--resume <path>`") the CLI maps
  through the existing `repl::reload_args`, keeping the TUI a pure client.

Slices, ascending in coupling (each independently shippable — tests + clippy +
**a live check**):

- **P3a — overlay layer + model picker.** A generic modal (`Overlay::Picker`)
  drawn over the three bands (design §1: an overlay never reflows the base;
  draft D). Keys route to the overlay first; Esc closes, ↑/↓ move, printable
  chars filter, Enter selects. `/model` with no arg opens it; a selection emits
  the existing `SetModel`. The model list is injected into `run(...)`.
- **P3b — diff rendering.** `edit`/`edits`/`write`/`replace` attach a compact
  unified diff to their result; the TUI styles `@@`/`+`/`-` lines (dim/green/red)
  under the `⚙` line with a `+a −r` summary. The diff is **UI-only**: it rides a
  new `ToolOutput::diff` → `ToolExecutionEnd.diff` field so the model's context
  stays lean (route 2 in the fork below).
- **P3c — run changeset.** The changed file's path now rides the tool event
  (`ToolExecutionEnd.path`, UI-only, alongside `diff`); the TUI accumulates the
  run's `(path, +a −r, diff)`, `/changes` lists the changed files in the overlay,
  and selecting one re-shows its diff. View-owned and ephemeral — reset on the
  next prompt, never persisted (durable history is git's job).
- **P3d — session picker.** `/resume` opens the overlay seeded from an injected
  session list (`id · age · first user line`, built by the composition root from
  the session dir; an argument pre-fills the filter). A selection does not resume
  in place — the TUI returns `Outcome::Resume(path)` and the CLI re-execs with
  `--resume <path>` through the shared `exec_self`, keeping the TUI a pure client.
  The list is empty over a socket (the server owns the session).

Testing: pure reducer tests (overlay open/filter/select → the right `Action`;
keys swallowed while open; the changeset's accumulate/reset), `TestBackend`
snapshots (overlay box, diff styling, the changeset picker), and a live check
per slice.

Open: dedicated picker chords vs. the `/`-palette; `NO_COLOR`/narrow-width
behaviour for overlays; pickers under `Backend::Remote` (no session dir, no model
list).

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


## 9. Decisions

Resolved (visual ones in [`tui-design.md`](tui-design.md) §5):

- **Versions** — pin to jcode's: `ratatui 0.30` + `crossterm 0.29` (both exist).
- **Alt-screen** — yes: full-screen with a custom scrollback (P1).
- **Crate** — a new `wcode-tui` depending on `wcode-protocol`.
- **Data source** — the `Backend` seam, never `Agent` (§3). This *is* the
  client/server split the brainstorm §12 asked for; the TUI is a peer.
- **Markdown** — landed in P2: a hand-rolled minimal renderer (`markdown.rs`),
  no dependency.
- **Clipboard** — landed in P2: OSC-52 (`/copy`), no `arboard`.
- **Diff is UI-only** — not in the tool `output`, so it never enters the model's
  context; carried by `ToolOutput::diff` / `AgentEvent::ToolExecutionEnd.diff`, so
  it works live but is not persisted (a replayed session shows no diff).
- **The changeset is view-owned** — the run's changes live in the TUI
  (accumulated from `ToolExecutionEnd.path`/`.diff`), reset per prompt and never
  persisted; the path rides the event the same UI-only way the diff does. Durable
  history is git's job, so there is no protocol request and no fs read.
- **P3 capabilities** — injected from the composition root, *not* new protocol
  requests (`ListModels`/`ListSessions`); the session picker is a `--resume`
  re-exec handoff. Revisit when the socket client needs them.

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
- [x] P2: markdown for committed messages; context bar; redraw on resize;
      `/copy` (OSC-52).
- [x] P2 fixes: the wheel scrolls the transcript (mouse capture; 3 lines a
      notch) and startup replays `GetHistory`, so a resumed session opens on its
      earlier turns rather than an empty pane.
- [x] P3a: overlay layer + model picker (`/model` with no arg opens a
      centered modal; the model list is injected; `/model <id>` still sets).
- [x] P3b: diff rendering (UI-only unified diff; styled `@@`/`+`/`-`, `+a −r`).
- [x] P3c: run changeset (`ToolExecutionEnd.path`; `/changes` lists the run's
      changed files and re-shows a diff).
- [x] P3d: session picker (`/resume`; `Outcome::Resume` → CLI re-exec handoff).
- [ ] P4 stretch.
