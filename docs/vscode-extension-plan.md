# VS Code extension — plan

**Status:** design locked; P0 not started.
**Companion docs:** the surface model this slots into is
[`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md) (§surface);
the client it must not drift from is `crates/wcode-tui`.

---

## 1. Problem

A VS Code extension for wcode: a chat panel, native diffs, file selection — the
Claude-Code-shaped surface, for wcode.

What makes it tractable is the split wcode already has: the kernel emits
`AgentEvent`s and accepts `Request`s over a frame protocol, and the TUI is a
*client* of it, not part of it. **The extension is a second client, not a second
kernel.** Tools stay where they are (the CLI process); the extension never
reimplements `bash`, `edit` or `read`.

Two things are missing, and only one of them is Rust:

1. **A transport that works on Windows.** `wcode-protocol` is unix-socket-only.
2. **An editor client.** The chat surface, the diff affordances, the team view.

## 2. What already exists (the seam)

- **The surface model.** `session = server-owned runtime`, `surface = a
  client-side view of a session`, `client = a container of surfaces`
  (`interface-protocol-brainstorm.md:cxNiI`, `:r2kVN`, `:0Taxs`), and the TUI
  seam and the A2A seam are deliberately one protocol (`:ibL0X`). An extension
  is a client holding one surface per session.
- **The transport seam.** `enum Backend { Local(SessionHandle), Remote(Client) }`
  with `send`/`ask`/`subscribe`; "swapping the transport is swapping the
  variant" (`wcode-protocol/src/backend.rs:yss9g`, `:kfWpt`).
- **The wire.** One flat JSON object per line (NDJSON), `{v, id, reply_to,
  session, sender, …body}`; requests correlate by `id`/`reply_to`, streamed
  events leave `reply_to` absent (`wcode-harness/src/protocol.rs:nDJoL`,
  `wcode-protocol/src/frame.rs:4XCCO`). Cap 16 MiB (`:gbOLr`).
- **The request surface** (`protocol.rs:Pfw8B`) is already a chat client's whole
  vocabulary: `Submit`, `Interrupt` (steer), `Wake` (follow-up), `Cancel`,
  `Notify`, `SetModel`, `SetEffort`, `SetPlanMode`, `Compact`, `SideAsk`,
  `GetHistory`, `Status`, `ListSessions`, `Define`.
- **The event surface** (`wcode-harness/src/event.rs:JJqEF`) is already a UI's
  whole model: streaming `MessageStart/Update/End`, the tool timeline
  `ToolExecutionStart/Update/End`, `Error`, `Retrying`, `Compaction`, `Stopped`,
  `Todo`, `MessageReceived{from, content}`, plus `History` to hydrate a
  transcript and `Sessions` as an **unsolicited pushed roster**
  (`wcode-protocol/src/server.rs:BrjIS`).
- **`ToolExecutionEnd` already carries the diff**: `diff: Option<String>` (a
  unified diff) and `path: Option<String>` (`event.rs:ux4ID`, `:E3UKn`) —
  explicitly UI-only, never entering the model's context (`tool.rs:EfAZ4`). Every
  write tool returns it (`tools/edit.rs:APhur`, `edits.rs:323`, `replace.rs`,
  `write.rs`).
- **Multiplexing.** One connection carries many sessions; each outbound frame is
  stamped with its origin `session`, and roster growth is pushed
  (`server.rs:Ebbvf`, `:sOnZ2`). Addressed by `SessionId` (`agent:<id>`, `user` —
  `protocol.rs:UeMFf`), and a client narrows to one member with
  `Client::with_session` (`client.rs:u9PyK`).

## 3. Design

### 3.1 P0 — the transport: `wcode serve --stdio`

**The blocking fact.** Every transport module is `#[cfg(unix)]`
(`wcode-protocol/src/lib.rs:u9XUm`); there is no TCP and no stdio, and non-unix
`serve` exits 2 (`wcode-cli/src/main.rs:mDMEi`). A Windows extension cannot
reach a session today.

**Decision: stdio.** `wcode serve --stdio` serves the same frame loop over the
child's stdin/stdout. The framing needs no change (one object per line, blank
lines skipped), and it deletes the whole problem class: no port, no token, no
stale socket file, no `chmod 0600` (`socket.rs:bzptL`), identical on all three
platforms. The extension spawns the process on activation and owns its lifetime.

Shape: the connection loop becomes transport-agnostic — one function over any
`AsyncRead + AsyncWrite` pair — with the unix path calling it on a `UnixStream`
and the stdio path on `tokio::io::join(stdin, stdout)`. The unix-only pieces
(socket path, `chmod`, `UnixListener`) stay unix-only; the frame loop stops being
gated. (Internals pinned by the P0 sketch; `serve` is `main.rs:1336`.)

**Trade-off, stated up front:** stdio serves **exactly one client by
construction**. A TUI *and* the editor on one session is the socket's job. That
is the right v1 default (the extension owning its own session is simpler and
needs no daemon), but it is a real difference from `--socket`, and it is the
reason the socket path must stay.

### 3.2 The client

**Location:** `editors/vscode/`. The Cargo workspace lists its four members
explicitly (`Cargo.toml`), so a sibling `editors/` tree is not swept into it.
TypeScript, bundled with esbuild, dependencies kept minimal.

**Process model.** The extension spawns `wcode serve --stdio` for the workspace
and owns it: start on first use, kill on deactivate, restart on crash with a
visible state. The binary is found by a `wcode.path` setting, then `PATH`, then a
clear error. The same `~/.config/wcode/config.toml` the CLI uses applies;
`--dump-config` is the way to prove which endpoint a session actually resolved.

**The discipline that keeps the clients honest.** `wcode-tui` is a *pure reducer
over `AgentEvent`*. The extension's model layer must be the same thing — events
in, view-model out, no I/O in the reducer — so the two clients cannot drift and
the reducer is testable without VS Code.

**UI:** a webview chat panel, not VS Code's native chat API. The roster/sidebar
is the point (below), and the native chat surface fights anything custom. The
webview talks to the extension host over `postMessage`; the extension host owns
the process and the protocol.

### 3.3 Diffs

`ToolExecutionEnd` gives one unified patch and a path; `vscode.diff` wants two
documents. Sources, ranked:

1. **Reverse-apply the patch** (chosen). Read the current file, reverse-apply the
   unified diff to reconstruct the pre-image, serve both sides from a
   `wcode-diff:` `TextDocumentContentProvider`, open the native diff editor.
   Precise per tool call, works for every write tool, no disk history, no git.
2. **VS Code's own document history** — keep files wcode touches open as
   background documents and let the host track the deltas. Free and exact, but
   only for files already open.
3. **git** — `git show HEAD:<path>` vs the working tree. Zero protocol work, but
   it shows *all* pending changes rather than the edit you asked about.

`diff`/`path` are `Option` and skipped when absent, so "no diff" is a normal
case, not an error. Diffs open **on request** — never automatically.

### 3.4 Approval

The kernel has no approval flow by design (`AGENTS.md`); `Hooks` is a code-only
trait and no `Request` can inject one (`hooks.rs:iPMeO`). Three tiers, cheapest
first:

- **Post-hoc (P1–P3, no kernel change).** Edits land; the diff is one click away;
  reject = VS Code undo or git.
- **Plan-gate (P1–P3, no kernel change).** `Request::SetPlanMode{on}` already
  exists, and `PlanModeHooks` blocks exactly `edit`/`edits`/`write`/`replace`/
  `ast_edit` by name (`hooks.rs:osi4C`). "Let it explore and plan, I approve,
  now execute" works over the wire today.
- **Per-edit approval (P4, kernel change).** A CLI-side `Hooks` impl that asks
  the client and blocks until answered, plus one Request/event pair. This is the
  *sanctioned* way to build it (`AGENTS.md`: build the variant, don't configure
  one), but it is a protocol addition and a deliberate decision — not a v1
  assumption.

### 3.5 Team-first

Claude Code's extension is one chat = one agent. wcode's protocol is built for
the opposite, and that is what this extension is *for*: a sidebar of live members
with per-member surfaces, status, and peek/ask/stop. Everything needed is already
on the wire — the pushed roster (`Sessions`), `with_session` to narrow,
`GetHistory` to hydrate any member, `MessageReceived` for inter-agent traffic,
`Todo`, `Spawned`. A single-agent chat box would be a worse product built on a
better protocol.

## 4. Phases

| phase | what | touches |
|---|---|---|
| **P0** | `serve --stdio`: transport-agnostic connection loop + the flag + cross-platform | `wcode-protocol`, `wcode-cli` (Rust) |
| **P1** | the client: spawn/supervise, NDJSON client, reducer, webview chat — `Submit`/`Cancel`/`Interrupt`, stream `Message*`, hydrate with `GetHistory`, tool timeline | `editors/vscode` |
| **P2** | diffs: reverse-apply → `wcode-diff:` provider → `vscode.diff`, reveal file at line | `editors/vscode` |
| **P3** | team: roster sidebar, per-member surfaces, `MessageReceived`, `Todo`, the plan-mode toggle | `editors/vscode` |
| **P4** | optional: per-edit approval gate — CLI `Hooks` impl + one Request/event pair | `wcode-cli`, `wcode-protocol` |

## 5. Testing & verification

The UX is the deliverable here, so the gates are manual-first:

- **P0 is machine-verifiable**: `cargo test --workspace` + `clippy`, a stdio
  round-trip test over an in-memory duplex, and a live smoke test — pipe a
  `Request::Submit` frame at `wcode serve --stdio` and read `AgentEvent` frames
  back off stdout.
- **P1–P3 are manual**: `F5` → Extension Development Host, open a workspace, run
  the start command, and exercise the chat. Each phase ends with an explicit
  click-path a human can follow, because "it compiles" says nothing about whether
  the panel is usable.
- **The reducer is unit-tested** (events in, view-model out) precisely so the
  manual pass is about *UX*, not about protocol plumbing.

## 6. Decisions & open questions

Locked:

- **Stdio over TCP/named pipes** — smallest change, no ports or tokens, one code
  path for all platforms. Costs the "one session, many clients" property.
- **Webview, not the native chat API** — the team view is the point.
- **Extension spawns and owns the process** — no daemon in v1.
- **Team in v1 (P3)** — it is the differentiator, not a stretch goal.
- **The socket path stays** — stdio does not replace it; a shared session still
  needs it.

Open:

- Does the extension need a `Hello`/`Attach` handshake? Today a fresh client
  gets the served id pushed on connect — `connection` seeds an unsolicited `Sessions`
  frame (`server.rs:BrjIS`), so no request is needed;
  the legacy `"remote"` placeholder means a single-session server accepts any id
  (`server.rs:JVP1s`), so v1 can simply connect and talk. Nice-to-have, not
  blocking.
- On-disk session listing over the wire — `/resume` is client-local (re-exec), so
  a "recent sessions" picker needs either a re-exec or a new `Request`.
- First-class context injection: there is no `Request` carrying a selection or
  open files. v1 inlines the selection into `Submit{text}` and lets the model
  `read` the paths it names.
- Whether P4's gate is wanted at all, once the plan-gate exists.
- **A Windows user cannot build wcode without a C toolchain.** The dependency graph
  pulls `aws-lc-sys`, which needs `windows.h` (MSVC + the Windows SDK); a cross-check
  from macOS fails *there*, not in our `#[cfg]`s. Since the extension needs a `wcode`
  binary, P1 must choose between a prebuilt release binary (with `wcode.path` pointing
  at it) and requiring VS Build Tools. That is a distribution decision, not a code one
  — but it decides how the extension is installed.

## 7. Progress

| phase | status |
|---|---|
| P0 | ◐ implemented — builds, tests green, live smoke test passes; second-layer review pending |
| P1 | ☐ |
| P2 | ☐ |
| P3 | ☐ |
| P4 | ☐ (optional) |
