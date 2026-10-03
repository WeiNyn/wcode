# Fixtures

NDJSON: one wire frame per line, exactly as it comes off `wcode serve --stdio`
stdout (envelope + flattened body).

- **`list_sessions.ndjson` — CAPTURED.** Produced by `npm run capture`
  (`scripts/capture.mjs`) against the real binary. It holds the seeded `sessions`
  push plus the model-free `list_sessions` / `get_history` / `status` replies.
  The session id is random per run, so re-capturing churns the file — the tests
  do not depend on the exact id.
- **`*.handwritten.ndjson` — HAND-WRITTEN.** Model-dependent events
  (`message_*`, `tool_execution_*`, `turn_*`, `agent_*`, `compaction*`,
  `retrying`, `stopped`) cannot be produced without a model call, so these are
  written from the typed wire shapes in `src/protocol.ts` (which mirror the Rust
  serde attributes in `crates/wcode-harness`). They are NOT captured; do not
  claim otherwise.
