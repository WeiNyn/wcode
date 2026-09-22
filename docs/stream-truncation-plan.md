# wcode — stream truncation ends a turn silently (plan)

Status: **☑ done.** Two layers, one commit. The adapter
(`crates/wcode-harness/src/streamfn.rs`) and the kernel backstop
(`crates/wcode-harness/src/loop_.rs`) are the only production files touched;
`wcode-harness/tests/loop_tests.rs` gains three tests and fixes one that
under-scripted.

## Problem

A turn ends when the stream yields `None`, **not** when it yields a terminal
record (`Final` → `Done`). When a provider's SSE body ends early — a proxy
idle-close / half-close / EOF, no `[DONE]` and no `finish_reason` — rig emits
**neither a `Final` nor an `Err`**: the normalized stream simply ends with
`None` (`rig-core` `openai_chat_completions_compatible.rs:802`; the spike that
proved it is `crates/wcode-harness/tests/rig_spike.rs:77-79`).

`forward_stream`'s select had exactly one terminal condition, the `None` arm
`None => break`, which treated that early end as a **clean stop**: no `Done`,
no `Error`. The kernel then defaulted the missing `Done` to `StopReason::Stop`.
A truncated turn therefore surfaced as a partial thinking-only assistant, or as
a silent empty turn — the model "answered" and the run moved on.

## Design

Two layers, because the adapter and the kernel answer different questions.

### Layer 1 — the adapter (`streamfn.rs`)

`forward_stream` gains a `done_seen` flag beside `errored` / `content_forwarded`:

- The `Ok(content)` arm computes `is_final = matches!(content, …::Final(_))`
  **before** `map_item` consumes the item, keeps the existing
  `errored && is_final → continue` skip (an `Error` already went out), and sets
  `done_seen |= is_final` **after** `map_item`.
- The `None` arm becomes the real terminal condition:
  - `if done_seen || errored { break; }` — a terminal record (or an `Error`) was
    seen; the trailing `Final` skipped above is covered here.
  - else, pre-content (`!content_forwarded && state.budget > 0`) → **retry** on
    the shared [`RetryState`] budget, exactly as the idle-stall arm does;
  - else → emit a **NON-fatal** `LlmStreamEvent::Error` and break, so the kernel
    feeds it back next turn and (persistently) ends `StopReason::Error`.

`done_seen` is **wire-agnostic**, deliberately: `forward_stream` is generic over
`S: Stream<Item = Result<StreamedAssistantContent, CompletionError>>`, holds no
`StreamingCompletionResponse`, and cannot read `.response` /
`.final_response_yielded` without constraining `S` — which would couple the
seam to rig internals. Deriving it from the same normalized item the kernel
already consumes mirrors the kernel's own `captured` accounting.

Two helpers back this:

- `truncation_error() -> CompletionError` — a synthesized provider error shaped
  like `stall_error`. It need not be `retryable()`: the pre-content branch
  re-opens unconditionally (`connect_with_retry` is never consulted for
  `retryable` there); the message only feeds `retry_wait`'s notice text.
- `pre_content_retry<S, F>(policy, state, open, tx, error) -> PreContent<S>` —
  the one pre-content re-open: spend one attempt + one budget unit, back off
  (abandoning on a dropped consumer), then call `connect_with_retry` with the
  SAME state. The idle-stall arm and the truncation arm both call it (one retry
  path, not two); both match the returned `PreContent<S>` identically. The bound
  is `F: Fn()` (not `FnMut`) so `open: &F` passes through.

**Chosen `Error.message`**: `truncation_error().to_string()` — i.e.
`ProviderError: stream ended before its terminal record (truncated)`, not the
bare design string. It is the same string the `Retrying.reason` carries for the
same error, so the notice and the surfaced error agree.

### Layer 2 — the kernel backstop (`loop_.rs`)

Layer 1 fixes the rig adapter; a custom `StreamFn` (a test double, an embedder's
provider) can still end a turn with no terminal record and reach the loop as a
clean stop. A guard sits between the tool-use-no-call guard and the stop
computation:

```rust
if !aborted && stream_error.is_none() && captured.is_none() {
    let message = "stream ended without a terminal record".to_string();
    let _ = sink.send(AgentEvent::Error { message: message.clone() });
    stream_error = Some(message);
    captured = Some(StopReason::Error);
}
```

It fires only when the select ended with **no terminal event at all** —
`captured` is `None` (no `Done`, no `Error`) and `stream_error` is `None`. It
sets `captured = Some(Error)` so the stop computation yields `Error`, and
`stream_error = Some(..)` so the turn rides the existing feed-back path
(bounded by `DEFAULT_MAX_STREAM_ERROR_TURNS`). Because `stream_error` is **not**
`None`, it deliberately bypasses the hard-fatal early-return
(`captured == Error && stream_error.is_none()`).

## Tasks

| # | task | status |
|---|------|--------|
| 1 | adapter: `done_seen`, the rewritten `None` arm, `Ok`-arm `is_final` | ☑ done |
| 2 | adapter: `truncation_error` + `pre_content_retry`; idle arm switched to it | ☑ done |
| 3 | kernel: the Layer-2 backstop guard | ☑ done |
| 4 | adapter tests: truncation error; clean stream unchanged; pre-content retry-then-error | ☑ done |
| 5 | kernel tests: no-`Done` ⇒ `Error`; nothing-then-end ⇒ `Error`, no empty assistant; `Done` ⇒ unaffected | ☑ done |
| 6 | fix `after_tool_call_patches` (it under-scripted a terminal turn) | ☑ done |

### Tests (assert behavior, not merely pass)

- Adapter (`streamfn.rs::retry_tests`): a delta then **no** `Final` ⇒ a
  NON-fatal `Error` and no `Done`/`Retrying`; a `Final`-terminated stream ⇒
  `Done`, no `Error`; a pre-content no-item stream with budget ⇒ `policy.max`
  `Retrying` notices then a NON-fatal `Error`.
- Kernel (`tests/loop_tests.rs`): a scripted text turn with no `Done` ⇒
  `run` returns `StopReason::Error` (never a silent `Stop`), bounded by the cap,
  with the partial assistant kept and the fed-back notice reaching a later call;
  nothing-then-end ⇒ the same with **no** recorded empty assistant; a
  `Done`-terminated stream ⇒ unaffected.

### Audit — can the backstop false-fire?

No. Every scripted turn in the in-repo suites terminates on a record that sets
`captured`:

- `loop_tests.rs`: 36 `rec.push` turns end with a `Done`; 2 end with a non-fatal
  `Error` (both set `captured`). Raw event counts are higher (38 `Done`, 4
  `Error` lines) because two inline `StreamFn` closures yield their events
  directly rather than via `rec.push`.
- `agent_tests.rs`: all 15 `rec.push` turns end `Done` (16 `Done` lines; 1
  `Error` line in an inline closure).

One pre-existing test — `after_tool_call_patches` — **did** under-script: a
single tool turn with no terminal turn, relying on the old silent default
`Stop`. It now scripts its `Done`-terminated second turn like its siblings.

A note on scope: the select's `_ = tx.closed()` arm is the **adapter's**
(`streamfn.rs`), not the kernel's — the loop's select has only
`cancel.cancelled()`, `stream.next()`, and the idle sleep, so no `aborted = true`
is warranted there (and none was added).

### Out of scope (follow-up)

The milder case — a `Done` with empty **non-tool** content (`streamfn.rs`,
`finish_reason` mapping `ContentFilter`/`Other` → `Stop`) — is left as-is; it is
a completed turn, not a truncation. See `next-steps.md` item 29.

## Status

| # | commit | status |
|---|--------|--------|
| 1 | `harness:` adapter + kernel backstop + tests (+ the plan doc) | ☑ done |

Legend: ☑ done · ◐ in progress · ☐ todo.
