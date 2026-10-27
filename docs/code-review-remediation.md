# Code review remediation guide

Companion to the full code review of wcode v0.13.2. Each item is self-contained:
file anchors, the defect, a recommended fix, and the gate to run. Implement in
priority order; each item is one logical commit (`harness:`/`cli:`/`tui:`/`protocol:`).

Quality gate for **every** item:
```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets
```
Plus a live check where noted (see `docs/live-verification` skill / AGENTS.md).

---

## Priority 1 — correctness / data loss

### R1. Replay the head of the outbound queue on reconnect (protocol)
- **File:** `crates/wcode-protocol/src/client.rs:373-378`
- **Defect:** frame is popped from `out` before `write_frame`; on failure it is
  lost while `send` already returned `Ok`.
- **Fix:** on `write_frame` error, push the frame back to the front of `out`
  (or retry the head in the reconnect path) before `break`. Alternatively make
  `send` await flush so callers observe failure.
- **Test:** integration test that drops the socket mid-`send` and asserts the
  frame is delivered after reconnect.
- **Gate:** `cargo test -p wcode-protocol`.

### R2. Mint a fresh cancel token on the early session-append failure path (harness)
- **File:** `crates/wcode-harness/src/agent.rs:310-327` (and `agent.rs:363-365` for the
  normal path, `agent.rs:380-397` for the bottom error path).
- **Defect:** `session.append(Message)` failure returns before
  `self.cancel = CancellationToken::new()`, so a cancelled token survives.
- **Fix:** set `self.cancel = CancellationToken::new()` before returning in that
  arm, mirroring the bottom arm. Also refresh `steer`/`follow_up` senders there
  if they were invalidated.
- **Test:** unit test that cancels, forces an append failure (e.g. read-only
  session dir), then runs again and asserts it is not pre-cancelled.
- **Gate:** `cargo test -p wcode-harness`.

### R3. Catch panics in tool execution (harness)
- **File:** `crates/wcode-harness/src/loop_.rs:660-680` (`buffer_unordered`).
- **Defect:** a panicking tool unwinds `run_loop`, dropping the `Agent`.
- **Fix:** wrap each tool future in `std::panic::catch_unwind` (or
  `tokio::task::spawn` + `JoinHandle`) and convert a panic into a `ToolOutput`
  with `is_error: true` and a diagnostic message, preserving result pairing.
- **Test:** a `TypedTool` test double that panics; assert the loop continues and
  emits an error result.
- **Gate:** `cargo test -p wcode-harness`.

### R4. Cancel-token numbering fix in retry display (harness)
- **File:** `crates/wcode-harness/src/streamfn.rs:982-999` + `streamfn.rs:887`.
- **Defect:** `attempts` incremented twice per pre-content retry.
- **Fix:** increment once; either in `pre_content_retry` or in
  `connect_with_retry`, not both. Keep `budget` accounting unchanged.
- **Test:** existing retry unit tests; add an assertion on the emitted
  `Retrying { attempt }` value.
- **Gate:** `cargo test -p wcode-harness`.

---

## Priority 2 — security / trust boundaries

### R5. Constrain `webfetch` targets (cli)
- **File:** `crates/wcode-cli/src/tools/webfetch.rs:135-137` (+ redirect handling).
- **Defect:** only scheme checked; redirects can reach internal endpoints.
- **Fix options (pick one):**
  - (a) Disable redirects (`redirect::Policy::none()`) and document it, or
  - (b) validate each hop's host against an allowlist/blocklist (block
    loopback, link-local, RFC1918) using `reqwest`'s `redirect::Policy::custom`.
- **Test:** unit test that a redirect to `http://127.0.0.1` is refused.
- **Gate:** `cargo test -p wcode-cli`.

### R6. Path containment in `resolve()` (cli) — optional, design decision
- **File:** `crates/wcode-cli/src/tools/mod.rs:80-87` (chokepoint); callers
  `read.rs`, `edit.rs`, `edits.rs`, `write.rs`; plus `agent_files.rs:141-145`,
  `main.rs:853-857`, `repl.rs:313-320`.
- **Defect:** absolute paths and `..` escape the working directory.
- **Fix (if desired):** canonicalize and require the result to be under
  `working_dir` (or the session root); return an error otherwise. Keep an escape
  hatch for absolute paths only when explicitly allowed. Document the trust
  model in README either way.
- **Note:** because `bash` exists, this is defense-in-depth only; decide before
  implementing to avoid breaking legitimate use (e.g. reading `/etc`).
- **Test:** traversal and absolute-path cases.
- **Gate:** `cargo test -p wcode-cli`.

### R7. Authenticate socket peers / tighten `Define` and bind (protocol)
- **Files:** `crates/wcode-protocol/src/server.rs` (Define arm, sender handling),
  `crates/wcode-protocol/src/socket.rs:14-32`.
- **Defect:** any connected client can set `sender`, address any session, and
  invoke the worker factory; socket is chmod'd after bind.
- **Fix:** (a) bind with mode `0o600` from the start (create then chmod before
  listen, or use `umask`), (b) remove/unlink only if it is a socket, (c) gate
  `Define` behind a capability or at least run the handler off the read loop
  (`tokio::spawn`), (d) document the same-user trust boundary; optionally add a
  token handshake for remote peers.
- **Test:** round-trip tests for Define error path; socket mode assertion.
- **Gate:** `cargo test -p wcode-protocol`.

---

## Priority 3 — robustness / UX

### R8. Bounded spill + memory guard (cli)
- **Files:** `crates/wcode-cli/src/tools/bash.rs:214-223`,
  `crates/wcode-cli/src/tools/background.rs:149-164`.
- **Defect:** on spill failure `buf` grows unbounded.
- **Fix:** on `File::create` failure, stop buffering (drop incoming bytes) or
  return an error result; at minimum clear `buf` periodically. Add a hard cap
  (e.g. 64 MiB) with truncation notice.
- **Gate:** `cargo test -p wcode-cli`.

### R9. Process-global syntax theme (tui)
- **Files:** `crates/wcode-tui/src/markdown.rs:518-521`,
  `crates/wcode-tui/src/theme.rs:174-188`.
- **Defect:** `SYNTAX_THEME` is a global `RwLock` swapped at runtime; tests
  race.
- **Fix:** either (a) make rendering read the theme through a context passed
  down (remove global), or (b) serialize theme swaps with rendering and make
  tests use a scoped override. At minimum, make `install_syntax_theme` idempotent
  per-test via a test helper that restores on drop.
- **Gate:** `cargo test -p wcode-tui` (must be green in parallel, 10 runs).

### R10. Terminal restore on early failure (tui)
- **File:** `crates/wcode-tui/src/terminal.rs:27-37`.
- **Defect:** if the setup `execute!` fails, no `restore()`.
- **Fix:** call `restore()` (or rely on the panic hook) in the error branch
  before returning `Err`. Make `restore()` safe to call twice (already
  idempotent).
- **Gate:** `cargo test -p wcode-tui`.

### R11. Broadcast lag handling (tui)
- **File:** `crates/wcode-tui/src/lib.rs` `spawn_forwarder`.
- **Defect:** `Lagged` skips `AgentEnd`, leaving `running == true`.
- **Fix:** on `Lagged`, set a dirty/resync flag and re-fetch state, or at least
  force `running = false`? Better: increase `EVENT_BUFFER` and log; on lag,
  trigger a full state refresh from the backend.
- **Gate:** `cargo test -p wcode-tui`.

### R12. Replay roster push before subscriber exists (protocol)
- **File:** `crates/wcode-protocol/src/client.rs:425`.
- **Fix:** use `send_replace` like `Registry::refresh_roster`.
- **Gate:** `cargo test -p wcode-protocol`.

### R13. Bounded outbound queues (protocol)
- **Files:** `client.rs` `out`, server `out`.
- **Fix:** cap queue depth (e.g. 1024) and apply backpressure (await or
  return `Busy`) instead of unbounded growth.
- **Gate:** `cargo test -p wcode-protocol`.

### R14. `create_named` name validation (harness)
- **File:** `crates/wcode-harness/src/session.rs:103-111`.
- **Fix:** reject names containing path separators or `..` (mirror
  `validate_member_name`).
- **Gate:** `cargo test -p wcode-harness`.

### R15. Normalize `LoopConfig.working_dir` (harness)
- **Files:** `crates/wcode-harness/src/loop_.rs`, `agent.rs:104-109`.
- **Fix:** apply the same empty→cwd normalization in `run_loop` before building
  `ToolContext`.
- **Gate:** `cargo test -p wcode-harness`.

### R16. `notify` republishes context snapshot (harness)
- **File:** `crates/wcode-harness/src/agent.rs:409-420`.
- **Fix:** call `self.ctx_snapshot.send_replace(...)` after push + persist.
- **Gate:** `cargo test -p wcode-harness`.

### R17. Minor items
- `agent.rs:380-397`: document that deferred `Tell`s can be lost on error; or
  re-queue them onto the new channels.
- `loop_.rs:738-749`: check cancel between groups to stop earlier.
- `stats.rs:282-288`: align `unpriced_turns` with the documented contract.
- `terminal.rs`: idempotent restore (already).
- `app.rs:1934-2003`: log dropped out-of-order tool events.
- `theme.rs:687`: replace `.expect` with a returned error or a fallback.
- `tools/mod.rs:111-114`: handle missing file name.
- `session_groups.rs:203-216`: allow `..` only if not a path component.
- `config.rs`: document array-replace merge semantics.
- `background.rs:283-430`: document pid-reuse residual risk; add regression
  test for `bg kill` lock ordering (`background.rs:876-904`).
- `event.rs:253`: consider a catch-all variant for forward compat, or document
  the reconnect-loop consequence.
- `server.rs:88-93`: don't terminate `serve` on a transient `accept` error.

---

## Maintainability

### R18. Split monolithic modules
- `crates/wcode-tui/src/app.rs` (~509k), `ui.rs` (~294k), `main.rs` (~206k),
  `config.rs` (~129k), `repl.rs` (~124k), `streamfn.rs` (~119k).
- **Action:** extract submodules by feature (input handling, rendering, command
  dispatch) without changing behavior; keep public API stable. Do this last,
  one module at a time, with tests green after each.

---

## Suggested commit order
1. R2, R4, R14, R15, R16 (harness, small)  
2. R1, R12, R13 (protocol)  
3. R3 (harness, bigger)  
4. R5, R8 (cli)  
5. R9, R10, R11 (tui)  
6. R6, R7 (trust boundaries, design-gated)  
7. R17 (cleanup)  
8. R18 (refactor, optional)

## Verification checklist (run after each item)
```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets
# plus, for protocol/cli changes:
cargo test -p wcode-protocol
cargo test -p wcode-cli
# live check for retry/error paths:
WCODE_RETRY_MAX=2 wcode -p "hi" --base-url http://127.0.0.1:9/v1
```

## Not covered by this guide
- VS Code extension (`editors/vscode/`) TypeScript build/test.
- Live end-to-end against a real model endpoint.
- Concurrency stress tests for the actor and scheduler.
- Dependency CVE audit (`cargo audit` unavailable here).
