# D025 — Auto-select the wire endpoint from the model catalog

- **Status:** accepted (human sign-off on the W021 brief — "ok go")
- **Date:** this session
- **Relates to:** D018/D024 (the W0xx decision log), the `is_opencode_go` catalog
  (`crates/wcode-harness/src/limits.rs`), `ModelProfiles::settle`
  (`crates/wcode-harness/src/streamfn.rs`), `--detect-endpoint` (a *host* probe —
  explicitly a different feature)
- **Work item:** [`work/W021-auto-endpoint-from-model.md`](../work/W021-auto-endpoint-from-model.md)
  · **Tracker:** [`next-steps.md`](../next-steps.md) item 64

## Context

The wire protocol (`chat` = `/chat/completions` vs `responses` = `/responses`)
was a **pinned** value: `LlmEndpoint` on `LlmOpts`, and, when config said
nothing, it defaulted to `Chat` — `parse_endpoint` mapped `None | Some("")` to
`Ok(LlmEndpoint::Chat)` (`crates/wcode-cli/src/config.rs:976`). Two consequences:

1. **"Not fixed in config" and "explicitly `chat`" were the same value.** There
   was no *unpinned* state to derive an endpoint from; the distinction survived
   only in provenance (`Source`), which is diagnostic and never re-consulted.
2. **OpenCode Go serves some models on only one wire.** `grok-4.6`, `grok-4.7`,
   `gpt-5.6-luna`, `gpt-6-luna`, `muse-spark-1.2-contributor`,
   `muse-spark-1.3-contributor` reject chat with
   `{"error":{"type":"ModelProtocolUnsupported"}}`. The Go `/models` listing
   returns only `id/object/created/owned_by` — no protocol — so wcode could not
   know, and a user had to hand-pin `endpoint = "responses"` and un-pin when
   switching back.

## Decision

1. **"Unpinned" becomes a real state, and absence *is* it.** `parse_endpoint`
   returns `Result<Option<LlmEndpoint>, String>`; `Config.endpoint` becomes
   `Option<LlmEndpoint>`. `None` / `""` / `"auto"` ⇒ `None` (derive);
   `"chat"` / `"responses"` ⇒ `Some(_)` (pin). An explicit `endpoint = "chat"`
   keeps pinning chat exactly as before, so **every existing config is
   byte-identical in behaviour** — except that the models that fail today now
   work. `auto` is accepted as an explicit value too, so a user can say "let the
   catalog decide" even when a layered config supplies an `endpoint`.

2. **The mapping is a kernel catalog fact; the policy lives in `settle`.**
   `limits::model_protocol(base_url, model) -> Option<LlmEndpoint>` sits beside
   `model_limit`, gated by `is_opencode_go` exactly like it, and returns
   `Some(Responses)` only for the responses-only snapshot — never `Some(Chat)`
   (absence means "default to chat"). The apply policy lives in
   `ModelProfiles::settle`, *not* the CLI, deliberately: `settle` is the one
   funnel every endpoint decision already passes through (boot `to_llm_opts` →
   `settle_provider`, a mid-session `/model`, a remote `Request::SetModel`, and a
   worker's own model). Putting it anywhere else would have left the mid-session
   switch broken — or forced a new `Request::SetEndpoint` protocol surface, which
   is **out of scope**.

3. **Precedence is explicit-value-wins.** Profile pin ▸ base pin ▸
   `model_protocol` ▸ `Chat`, resolved against the *settled* `base_url`, so a
   per-model profile that overrides only `base_url` re-derives the endpoint
   against that base.

4. **`--endpoint auto` and an absent endpoint both report `(source: auto)`; the
   provenance stays `Flag`/`Default`.** The `--dump-config` endpoint line reports
   the resolved *value/state* (unpinned → `auto`), not the origin — so
   `--endpoint auto` and an absent `endpoint` are **indistinguishable in the
   dump**, and `--endpoint auto` shows `(source: auto)` rather than
   `(source: flag)`. This is a deliberate consequence of decision 1, recorded
   here so it is a decision and not mistaken for a bug later. `Source::Display`
   renders lowercase (`flag`, `config`), so an assertion must not use
   `(source: Flag)`.

## Consequences

- `LlmOpts.endpoint` **stays** a concrete `LlmEndpoint` — it is the *resolved*
  value that `settle`/`to_llm_opts` writes — so `open_stream`,
  `reasoning_params` and `invalid_effort` are untouched.
- Because a concrete `LlmEndpoint` cannot recover `Auto`, `LlmProvider` carries
  an `EndpointPref { Auto, Pinned(LlmEndpoint) }` and its constructor becomes
  `LlmProvider::of(llm: &LlmOpts, endpoint: EndpointPref)` — the preference
  travels out-of-band.
- `EndpointPref::wire()` (→ `"auto"` / `"chat"` / `"responses"`) is what the
  `/reload` re-exec forwards, so auto survives a re-exec rather than being
  silently re-pinned.
- Models served by **neither** wire (`claude-haiku-5-5`, `minimax-m2.7` →
  Anthropic `/messages`; `gemini-4-argon-contributor` → Google
  `:generateContent`) are deliberately absent from the snapshot: they fall
  through to `chat`, i.e. today's server-side `ModelProtocolUnsupported`, rather
  than gaining a new client-side error surface.
- Non-zen bases are a strict no-op (`is_opencode_go` gate), so OpenAI/vLLM/Ollama
  and the implicit `api.openai.com` all resolve to `chat` as before.

## Alternatives rejected

- **`"auto"` as the only spelling, without making absence a state.** Rejected:
  absence is *already* a distinct input (`FileConfig.endpoint: Option<String>`);
  `parse_endpoint` was merely collapsing it. Keeping `chat` as the unset default
  would also make an explicit `chat` meaningless.
- **Policy in the CLI** (mirroring `--detect-endpoint`). Rejected: detection is an
  opt-in *network probe* whose result is a default base URL; this mapping is pure
  and deterministic, and only `settle` can fix a mid-session switch without new
  protocol surface.
- **A client-side "this model speaks neither wire" guard.** Deferred — a new
  failure surface with no upside over the server's own message (`AGENTS.md`
  "Absent by design").

## Verification (live)

Against the real Go gateway, all six responses-only ids re-confirmed on the
shipped build: each returns `ModelProtocolUnsupported` on
`/chat/completions` and answers on `/responses` — **0 mismatches**. `--dump-config`
on the built binary: `grok-4.7`@zen ⇒ `endpoint: responses (source: auto)`;
`deepseek-v4-flash`@zen ⇒ `chat (source: auto)`; `gpt-5.6-luna`@openai ⇒
`chat (source: auto)`; `--endpoint chat` ⇒ `chat (source: flag)`.
