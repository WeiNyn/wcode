# Fixtures

NDJSON: one wire frame per line, exactly as it comes off `wcode serve --stdio`
stdout (envelope + flattened body).

- **`list_sessions.ndjson` — CAPTURED.** Produced by `npm run capture`
  (`scripts/capture.mjs`) against the real binary. It holds the seeded `sessions`
  push plus the model-free `list_sessions` / `get_history` / `status` replies.
  The session id is random per run, so re-capturing churns the file — the tests
  do not depend on the exact id.
- **`stale-wcode-stderr.txt` — CAPTURED.** The REAL stderr of the stale
  `wcode` on `PATH` (0.3.2, Sep 29 — it predates P0) run as `wcode serve --stdio`:
  exit 2, `error: unexpected argument: --stdio`. Drives `test/startup.test.ts`.
- **`other-parse-error-stderr.txt` — CAPTURED.** The REAL stderr of the CURRENT
  binary run with a *different* unexpected argument (`serve --bogus`). Its usage
  block lists `--stdio`, so it is the false-positive case the startup classifier
  must not match.
- **`*.handwritten.ndjson` — HAND-WRITTEN.** Model-dependent events
  (`message_*`, `tool_execution_*`, `turn_*`, `agent_*`, `compaction*`,
  `retrying`, `stopped`) cannot be produced without a model call, so these are
  written from the typed wire shapes in `src/protocol.ts` (which mirror the Rust
  serde attributes in `crates/wcode-harness`). They are NOT captured; do not
  claim otherwise.
