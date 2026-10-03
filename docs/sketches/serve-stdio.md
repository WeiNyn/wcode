# Sketch — `wcode serve --stdio` (P0) — NEW `crates/wcode-protocol` fn + `wcode-cli` flag

**Status:** interface sketch, review-only. No working logic. Fillable as-is.

**Design:** `docs/vscode-extension-plan.md` §3.1 (the lock): the connection loop becomes
transport-agnostic — one function over any `AsyncRead + AsyncWrite` pair — the unix path
calling it on a `UnixStream`, the stdio path on `tokio::io::join(stdin, stdout)`; the unix
socket pieces stay gated, the frame loop stops being gated; `--stdio` serves **exactly one
client by construction** (`:B1hRD`, `:fCwej`, `:9yGSX`).

**Grounding (current code, fresh anchors):**

- **The frame loop, today typed to `UnixStream`.** `server.rs:server::connection` is
  `aFpZP`, taking `stream: UnixStream` (`:moywH`) and immediately `stream.into_split()`
  (`:CVI9d`). The rest of its body already touches only transport-free types:
  `Registry`, `Roster`, `Root = (SessionId, SessionHandle)`, `DefineHandler`,
  `Frame<AgentEvent>` / `Frame<Request>`, `read_frame`/`write_frame` (`frame.rs:8BPV6` /
  `:EvQuH`). Its helpers are **already generic/transport-free**: `fan` `server.rs:Ebbvf`,
  `write_loop` is `W: AsyncWrite + Unpin` `:KveYm`, `roster_infos` `:zyU1C`.
- **The accept loop is the only unix-bound part.** `server::serve` `:RNqco` takes
  `listener: UnixListener` (`:Jasoy`); `server::serve_at` `:8ikXJ` binds via
  `socket::bind` `socket.rs:KTS9h`. `socket::bind` is where the `chmod 0600` lives
  (`:bzptL`); `socket::connect` `:9iTda`.
- **The gates.** `lib.rs` gates the whole modules: `client` `:Zn1Gd`, `server` `:DSePh`,
  `socket` `:oSjFy`, each behind `#[cfg(unix)]` `:u9XUm`; re-exports at `:qG4pY`
  (client), `:EdQkq` (server), `:eYegn` (socket).
- **The CLI `serve`.** `main.rs:serve` `:Zok4j`, whole body `#[cfg(unix)]` `:1Jkhp`;
  `#[cfg(not(unix))]` → `eprintln!` `:d8BXd` + `exit(2)` `:Lj7vp`. It prints
  **stdout** diagnostics: `println!("serving session on …")` `:FBmaS` and
  `println!("serving {} session(s) …")` `:yabj4`, then flushes stdout `:603nY` before
  `serve_at(...)` `:hwbfy`. The `serve` flag `:UYZjl`, `--socket` `:53bG5`, USAGE
  lines `serve` `:gO4mx` / `--socket` `:ewMUy`; `default_socket_path` `:ML6wV`; the
  non-unix `--socket` guard `:mWkcF`. Dispatch `if args.serve { serve(…) }` `:Eu4nu`,
  `:NqeIF`; `choose_tui` excludes `serve` `:GulD6`.
- **Logging discipline precedent.** `print_provider_diagnostic`'s own doc: *"one provider
  line to STDERR (never stdout — it would corrupt `-p` output and the TUI alt-screen)"*
  `main.rs:cBOtn`. The rule P0 must extend to `serve --stdio`.

---

## 1. The transport-agnostic seam (`crates/wcode-protocol/src/server.rs`)

**Current shape (generalising from):**

```rust
// server.rs today — unix-typed at the boundary, transport-free in the body.
pub async fn serve(registry: Registry, roster: Roster, root: Root,
                   define: Option<DefineHandler>, listener: UnixListener) -> io::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;      // RNqco — unix listener
        tokio::spawn(connection(registry.clone(), roster.clone(),
                                 root.clone(), define.clone(), stream));
    }
}
async fn connection(/* … */ stream: UnixStream) {            // aFpZP / moywH
    let (read, write) = stream.into_split();                 // CVI9d — the ONLY unix call
    // … fan tasks, roster push, read_frame demux — all transport-free …
}
```

**Refactor — one function over any byte stream.** Rename `connection` → `serve_stream`,
make it `pub`, generic, and `#[cfg]`-free; keep the body byte-for-byte (only the split
changes):

```rust
// server.rs — NEW (ungated; the frame loop stops being unix-only).
use tokio::io::{AsyncRead, AsyncWrite};        // new imports
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};    // narrowed: listener/stream only

/// Drive **one** connection over any byte stream: the frame loop (read → demux →
/// actor), the per-session [`fan`] tasks, the live roster push, and the request
/// replies. Transport-free — the caller supplies the bytes. Returns when the
/// peer's stream reaches a clean EOF (`read_frame` → `Ok(None)`, `frame.rs:8BPV6`)
/// or the reader/writer errors.
///
/// The ONE unix assumption removed: `UnixStream::into_split` (`server.rs:CVI9d`)
/// becomes `tokio::io::split`, which needs only `AsyncRead + AsyncWrite`.
pub async fn serve_stream<S>(
    registry: Registry,
    roster: Roster,
    root: Root,
    define: Option<DefineHandler>,
    stream: S,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
    // ── the current `connection` body, unchanged from here (aFpZP…) ──
    // mpsc out-channel; spawn(write_loop(write, out_rx));
    // fan(root) + fan(each roster session); seed the `Sessions` push (BrjIS);
    // spawn the roster-watch push task (sOnZ2);
    // BufReader::new(read) (kVdvt) → while read_frame::<_, Request> (pKC4V) { … demux … }
}
```

**The unix accept loop — narrower, not deleted.** `serve` (`RNqco`) keeps the
`UnixListener`, still `#[cfg(unix)]`, and now spawns the generic seam:

```rust
#[cfg(unix)]
pub async fn serve(registry: Registry, roster: Roster, root: Root,
                   define: Option<DefineHandler>, listener: UnixListener) -> io::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(serve_stream(registry.clone(), roster.clone(),
                                   root.clone(), define.clone(), stream)); // was `connection`
    }
}
```

**And `serve_at`'s *definition* must be gated too — not just its re-export.** `serve_at`
(`server.rs:8ikXJ`) calls `crate::socket::bind` (`socket.rs:KTS9h`), and `mod socket` is
`#[cfg(unix)]` (`lib.rs:oSjFy`). Gating only the re-export (§2) leaves an ungated `serve_at`
referencing a unix-only module, so the crate **fails to compile on non-unix** — which is
exactly what P0 exists to fix. Its signature is unchanged; only the attribute is added:

```rust
#[cfg(unix)]
pub async fn serve_at(… same signature …) -> io::Result<()> { … }
```

**The stdio entry point — NEW, ungated (all three platforms).**

```rust
/// Serve the frame loop over the process's own stdin/stdout — the `--stdio`
/// transport (`docs/vscode-extension-plan.md` §3.1). **Exactly one client**:
/// there is no listener, so no second connection is even expressible. Returns
/// `Ok(())` when the client's stdin reaches EOF (clean shutdown; the spawning
/// client owns the process lifetime — `:dEEMH`).
pub async fn serve_stdio(
    registry: Registry,
    roster: Roster,
    root: Root,
    define: Option<DefineHandler>,
) -> io::Result<()> {
    let stream = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    serve_stream(registry, roster, root, define, stream).await;
    Ok(())
}
```

> `Join<Stdin, Stdout>: AsyncRead + AsyncWrite` — the pair the plan names (`:9yGSX`).
> No `BufReader` around the whole `Join`; `serve_stream` already wraps its read half.
> **Sketch choice — in-place gating vs a split module.** Primary: keep one `server.rs`,
> gate only its unix fns/imports. Alternative: lift `serve_stream`/`serve_stdio` into a
> new `mod connection`. <!-- SKETCH: TODO(prose) — pick one; the split keeps `server.rs`
> 100% unix but churns `lib.rs` re-exports. -->

## 2. `lib.rs` gate changes — which narrow, which go

```rust
mod backend; mod frame; mod registry;                 // unchanged
pub use frame::{read_frame, write_frame};             // unchanged

mod server;                                            // DSePh: DROP the #[cfg(unix)] gate —
                                                       // Root/Roster/DefineArgs/DefineHandler are
                                                       // transport-free; only its fns are unix.
#[cfg(unix)] mod client;                               // Zn1Gd: unchanged (Client stays unix-only, P0)
#[cfg(unix)] mod socket;                               // oSjFy: unchanged (bind/connect = the socket)

pub use server::{DefineArgs, DefineHandler, Root, Roster, serve_stream, serve_stdio}; // EdQkq: UNGATE
#[cfg(unix)] pub use server::{serve, serve_at};        // narrow: the socket accept loop only
#[cfg(unix)] pub use socket::{bind, connect};          // eYegn: unchanged
#[cfg(unix)] pub use client::{Client, FLUSH_TIMEOUT, flush_all}; // qG4pY: unchanged
```

- **Deleted:** the `#[cfg(unix)]` on `mod server` (`DSePh`) and on the `serve_stream` /
  `serve_stdio` re-export.
- **Narrowed to the unix fns:** `serve`, `serve_at` move to their own `#[cfg(unix)] pub use`.
- **Untouched:** `socket` module + `bind`/`connect` (`bzptL` stays unix-only), the whole
  `client` module.
- **Confirmed transport-free:** `serve_stream`'s body touches only `tokio::io` traits,
  `Registry`, `Roster`, `Root`, `DefineHandler`, `Frame<P>`, `read_frame`/`write_frame`
  (anchors in Grounding). `write_loop` (`KveYm`) and `fan` (`Ebbvf`) need no change — they
  are already generic / handle-based.

## 3. The CLI flag (`crates/wcode-cli/src/main.rs`)

**Args field** (beside `serve: bool` `:qbHc1` / `socket: Option<String>` `:bjjKg`):

```rust
/// `--stdio`: serve the frame loop over stdin/stdout instead of a unix socket.
/// Implies `serve`; mutually exclusive with `--socket`. Cross-platform.
stdio: bool,
```

**Parser arm** (beside `"serve" => a.serve = true` `:UYZjl` / `"--socket"` `:53bG5`):

```rust
"--stdio" => { a.stdio = true; a.serve = true; },
```

**Conflict rules (pinned):**

- **`--stdio` implies `serve`.** `wcode --stdio` and `wcode serve --stdio` are the same
  thing. **The normalisation site is the parser arm itself** (`"--stdio" => { a.stdio = true;
  a.serve = true; }`) — one site, no dispatch special-case.
- **`--stdio` ⊕ `--socket`** → **hard error**, `exit(2)`: `--stdio has no socket path`.
  Covers both spellings — `serve --stdio --socket X` (bind-path mode) and `--stdio
  --socket X` (connect mode) are both contradictory. Validated right after `parse_args`,
  before `default_socket_path` (`:ML6wV`) is consulted.
- **`serve`** alone → today's unix socket (unchanged).
- **`serve --stdio`** → the stdio path.

**USAGE help** (new line between the `serve` line `:gO4mx` and `--socket` `:ewMUy`):

```
  --stdio            serve over stdin/stdout instead of a socket (implies `serve`; one client, any platform)
```

**Position in the parser:** the `"--stdio"` arm goes immediately after `"serve"` (`:UYZjl`)
so the two serving modes read together.

**The non-unix `exit(2)` branch must go (that is P0).** Today `serve`'s whole body is
`#[cfg(unix)]` (`:1Jkhp`), the outer `#[cfg(not(unix))]` printing + `exit(2)`
(`:d8BXd`/`:Lj7vp`). Restructure so the transport-free setup (session id, `SessionActor`,
registry, roster, `define` — all of which already compile everywhere) runs **before** the
branch, then:

```rust
// serve(...) -> !
// ── build session_id, spawn SessionActor, bind bg, register root, registry, roster,
//    define handler — ALL transport-free (lift out of the #[cfg(unix)] block) ──
if args.stdio {
    if let Err(e) = wcode_protocol::serve_stdio(registry, roster, (session_id, handle), define).await {
        eprintln!("serve --stdio: {e}");
        std::process::exit(1);
    }
    std::process::exit(0);                     // client closed stdin → clean exit
}
#[cfg(unix)]
{ /* today's default_socket_path + eprintln + serve_at call, :hwbfy */ }
#[cfg(not(unix))]
{
    eprintln!("error: `serve` needs a socket, which is unix-only; use `--stdio`");
    std::process::exit(2);                     // narrows: SSE only, not stdio
}
```

The `--socket` *connect* guard stays: `#[cfg(not(unix))] if !args.serve && args.socket.is_some()`
→ `exit(2)` (`:mWkcF`) is still correct (a stdio client never sets `--socket`).

**Setup helpers currently unix-gated that the stdio path also needs.** Lifting the
transport-free setup out of `#[cfg(unix)]` also means ungating what it *calls*:
`live_roster` is `#[cfg(unix)]` (`main.rs:J8U02`) but is transport-free (a `watch` over the
registry's local sessions), and stdio serves the same live roster — **un-gate it**. By
contrast `Client::lazy` / `register_remote` (`:QV1fe`) stay unix-only (A2A socket peers,
§7). `SessionActor::spawn`, `Background::bind`, `register_root`, `registry.set_model` are
harness/registry calls, not transport calls — no gate.

## 4. Exactly one client (enforced structurally)

- **By construction, not by a check.** `serve_stdio` calls `serve_stream` **once** and
  returns; there is no `listener.accept()` (`:RNqco`) on the path, so a "second
  connection" is not expressible — stdin/stdout are a single byte stream. A second client
  would spawn a second `wcode serve --stdio` child.
- **On client EOF:** `read_frame` → `Ok(None)` (`frame.rs:iW5XM`) ends the `while let
  Ok(Some(_))` loop (`:pKC4V`); `serve_stream` returns; `serve_stdio` returns `Ok(())`;
  `serve` exits `0`. The detached tasks (`write_loop`, `fan`, the roster watch) are reaped
  at process exit. **No `AgentEvent` is synthesised on EOF** — the client initiated the
  close, and no `AgentEnd` event exists (run-scoped stop is `Stopped`).
- **Open:** does an EOF **cancel an in-flight run**, or just exit? The extension "kills on
  deactivate" (`:dEEMH`), so exit-only is defensible; a `Cancel` on the root before exit
  is also defensible. <!-- SKETCH: TODO(prose) — decide EOF semantics; do NOT assume. -->

## 5. Logging discipline — **call this out** (a real risk, verified)

**The rule.** stdout is the frame stream under `--stdio`; **stderr is the only diagnostic
sink**. Any stray `println!`/`print!` on the serve path corrupts the NDJSON (`frame.rs:4XCCO`).

**Grep result — the current `serve` path already violates it, in five places.** Two are in
`serve` itself; **three run earlier, in `build_runtime`** (`main.rs:1050`), which executes
before the `if args.serve` dispatch (`:Eu4nu`) and so is on the stdio path too:

- `println!("serving session on {}", path.display())` — `main.rs:FBmaS` (in `serve`, inside
  its `#[cfg(unix)]` block `:1Jkhp`)
- `println!("serving {} session(s): {}", …)` — `main.rs:yabj4` (in `serve`)
- `println!("team: {}", names.join(", "))` — `main.rs:hUvuy` (in `build_runtime`; fires with
  `serve --agents` + a non-empty `[team]`)
- `println!("workflow: {n} nodes")` — `main.rs:cUxK4` (in `build_runtime`; fires with
  `--agents` + a `[workflow]`)
- `println!("team: … (restored, …)")` — `main.rs:LwZS7` guard (in `build_runtime`; reached by
  `serve --resume <group>` — `load_session` sets `resuming_group` at `main.rs:939` and nothing
  excludes `serve` — so it is on the stdio path too)

All five are diagnostics on the setup path; the two `serving …` lines sit *after* the session
is wired and *before* `serve_at` (`:hwbfy`), and the `std::io::stdout().flush()` at `:603nY`
exists to flush them. Under `--stdio` each is emitted as non-JSON bytes on the frame stream.
After the §3 restructure: #1 (it reads the socket-only `path`) stays in the unix branch; #2
(it uses the transport-free `ids`) belongs in the lifted setup, so `--stdio` announces what it
serves too. Both are `eprintln!` either way — the rule is unconditional.
**Fix: `eprintln!` all five.** The `stdout().flush()` then becomes unnecessary on the stdio
path (the frame writer flushes per frame, `frame.rs:5jPxk`) and must not race the writer.

**Not violations (checked).** `print_provider_diagnostic` is `eprintln!` by design (`:cBOtn`).
`--version` (`:7rKqx`), `--help` (`:8Z0DC`), `--dump-config` (`:sZKQ6`),
`--dump-system-prompt` (`:1kAOp`), `--list-models` (`:1Ujc6`, `:3rEur`) and `--list-themes`
(`:SR0ut`) are early-exit paths that never reach `serve`. The TUI-new-session
(`:4xLRH`), headless-task (`:Tnv2R`) and one-shot (`:OZJpu`) prints are downstream of the
`dispatch` branch `serve` never takes, and the `tools/mod.rs` one is `#[cfg(test)]`.

**Rule to encode in the skill/comment:** under `--stdio`, stdout carries *only*
`Frame` JSON; every diagnostic is `eprintln!`.

## 6. Test skeletons (`#[cfg(test)]`, `todo!()`)

```rust
// crates/wcode-protocol/src/server.rs  (or a tests/ module beside it)
#[cfg(test)]
mod tests {
    use super::*;
    use wcode_harness::protocol::{Frame, Request, SessionId};

    // A served root needs a real SessionHandle (SessionActor::spawn over a test
    // Agent). The harness for building one is NOT sketched here — see the actors'
    // own tests. Everything below assumes `fn test_root() -> (Root, Registry)`.
    fn test_root() -> (Root, Registry) { todo!() }

    #[tokio::test]
    async fn stdio_duplex_round_trips_submit_to_events() {
        // tokio::io::duplex(64 * 1024) → (server_side, client_side).
        // spawn serve_stream(registry, empty_roster, root, None, server_side);
        // write_frame(Request::Submit{ text } frame) on client_side;
        // read_frame::<AgentEvent> until MessageStart/MessageEnd;
        // assert every line is exactly one JSON object (one frame per line: split on
        // the trailing b'\n' — frame.rs write_frame pushes it at Mh01K; read_frame
// returns Ok(None) only at clean EOF, Vnq0L).
        todo!()
    }

    #[tokio::test]
    async fn a_malformed_line_ends_the_connection() {
        // write "not json\n" then close the write half.
        // Expect the driver to stop (read_frame → Err(InvalidData) via its json_error,
        // frame.rs:gRxYo) rather than hang or panic.
        todo!()
    }

    #[tokio::test]
    async fn a_clean_eof_shuts_the_driver_down() {
        // write a valid ListSessions frame, read its Sessions reply, then drop the
        // write half (EOF). Expect serve_stream to return Ok(()) — no panic, no hang.
        todo!()
    }
}
```

**Live smoke test (a human runs this — the plan's P0 gate, `:Olmxw`).** No model needed
for the first hop; `ListSessions` is answered directly (`server.rs:b9yJ3`):

```sh
# 1. Round-trip without a model — the server answers ListSessions itself:
printf '%s\n' '{"v":1,"id":1,"session":"remote","type":"list_sessions"}' \
  | wcode serve --stdio
# expect ONE JSON object per line on stdout, e.g. a {"type":"sessions",...} frame;
# the "remote" placeholder reaches the sole session (server.rs:JVP1s).

# 2. A real run (keyless local endpoint, AGENTS.md gotcha):
printf '%s\n' '{"v":1,"id":1,"session":"remote","type":"submit","text":"say hi"}' \
  | wcode serve --stdio --base-url http://localhost:11434/v1 --model <id>
# expect streamed AgentEvent frames: message_start …, tool_execution_* …, message_end.

# 3. Prove stdout is pure frames: stderr may chatter, stdout must parse as NDJSON:
... | wcode serve --stdio 1>out.ndjson 2>err.log ; jq -c . out.ndjson   # every line parses
```

## 7. Friction (where stdio genuinely cannot match the socket)

- **A second client / a shared session.** The socket's whole point; stdio serves one.
  A TUI *and* the editor on one session needs `--socket` (`:B1hRD`). The socket path
  stays.
- **`--socket` attach.** A client attaching to a *running* session needs a socket path to
  `socket::connect` (`:9iTda`). stdio has none; a stdio client owns the child it spawned.
- **Remote A2A peers (`--peer <name>=<socket>`).** `register_remote(…, Client::lazy(path))`
  needs a socket path (`:QV1fe`); `--peer` and the non-unix gate (`:V79DT`) stay
  socket-only. A stdio server cannot be reached *as a peer* by another process.
- **`--owner`** (report back over the socket) presumes a socket; over stdio it has no
  channel. <!-- SKETCH: TODO(prose) — `--owner` + `--stdio` must error, not silently no-op. -->
- **What *does* still work over stdio:** `--agents` and the `Define` gate (`main.rs:VkMfV`)
  are transport-independent — the handler registers a worker into the registry the roster
  watches, and the worker is pushed as `Spawned` / `Sessions` over the *same* stdio
  connection (in-process). So `wcode serve --stdio --agents` is fine for **in-process**
  workers; only *remote* peers are impossible.
- **`default_socket_path`** (`:ML6wV`) is irrelevant to stdio — the stdio branch must not
  consult it (the `serve` restructure in §3 keeps it inside the unix branch).

## 8. Not verified / left to the implementer

- The exact `SessionHandle` test harness for the round-trip test (`test_root()` is a
  `todo!()`): the actors' own tests own that construction.
- Whether `serve --stdio` should `Cancel` an in-flight run on EOF (§4, open).
- The in-place-gating vs split-module choice (§1, `TODO`).
- `--owner` + `--stdio` semantics (§7, `TODO`).
