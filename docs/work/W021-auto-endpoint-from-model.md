# W021 — Auto-select the wire endpoint from the model catalog

**Slug:** `W021-auto-endpoint-from-model`
**ADR:** `docs/decisions/D025-auto-endpoint-from-model-catalog.md`
**Status:** awaiting human sign-off on the brief (no code written)
**Date:** recon complete; all anchors below re-verified in-tree.

## 1. The ask

Today the wire protocol (`chat` = `/chat/completions` vs `responses` = `/responses`)
is a *pinned* value: `LlmEndpoint` on `LlmOpts`, defaulting to `Chat` whenever
config says nothing — `parse_endpoint(None) => Ok(LlmEndpoint::Chat)`
(`crates/wcode-cli/src/config.rs:976`, `None | Some("") => Ok(LlmEndpoint::Chat)`).
When a user points wcode at an OpenCode Go (zen) base and picks a
**responses-only** model (e.g. `grok-4.7`), the chat wire is wrong and the server
answers `{"error":{"type":"ModelProtocolUnsupported"}}` — a failure the user
cannot fix except by hand-pinning `endpoint = "responses"` (and then un-pinning
when they switch to a chat-only model). The request: when the endpoint is **not
fixed** in config, wcode should **derive** it from the model id via the model
catalog (`limits.rs`) rather than defaulting to `chat`. This brief settles *how*
"not fixed" is represented, *where* the mapping and policy live, and how a
mid-session model switch stays correct.

## 2. Scope / Non-scope

**In scope**

- A per-model **protocol** fact in the kernel catalog (`limits.rs`), gated by
  `is_opencode_go` exactly like `model_limit`.
- An **"unpinned"** representation threaded from config → `LlmOpts`, so "absent"
  is distinguishable from "explicitly chat" (they are conflated today at
  `config.rs:976`).
- Resolution of an unpinned endpoint at the **single** place the endpoint is
  settled (`ModelProfiles::settle`, `crates/wcode-harness/src/streamfn.rs:162`),
  so boot, a `/model` switch, a remote `Request::SetModel`, and a worker's own
  model all resolve correctly.
- `--endpoint auto` as an explicit opt-in value; `endpoint` absent ⇒ auto.
- `/reload` re-exec forwards the *unresolved* auto intent (not a pinned
  `chat|responses`).
- `--dump-config` / `provider_summary` name the resolved endpoint and that it was
  auto.
- Docs (`README.md`, `--help`, `AGENTS.md` note) + ADR `D025`.

**Non-scope** (a reasonable reader might assume these are in)

- **A new `Request::SetEndpoint` or any `protocol.rs` wire change.** The switch
  is already correct once `settle` resolves auto (see §4, Q4); adding protocol
  surface would be the wrong layer.
- **Non-OpenCode-Go providers.** `model_protocol` is `None` off
  `opencode.ai/zen/go`, so auto ⇒ `chat`; nothing changes for OpenAI/vLLM/Ollama
  (see Q6). Widening the catalog is already tracked (`docs/next-steps.md:779`
  "Widen the context catalog — `limits::model_limit` keys off one provider only";
  `:995` item 6).
- **Serving the "neither" models** (`claude-haiku-5-5`, `minimax-m2.7` → Anthropic
  `/messages`; `gemini-4-argon-contributor` → Google `:generateContent`). Auto
  cannot reach a protocol wcode does not speak; these fall through to `chat` =
  today's behavior (see Q5).
- **Catalog id/window drift** — 9 stale ids (`docs/next-steps.md:101`), the drift
  tracker (`:779`, `:995`). Orthogonal; do not fold in.
- **A client-side "this model speaks neither" diagnostic.** A nicer error than
  the server's `ModelProtocolUnsupported`, but a new failure surface; listed as a
  follow-up, not here (per the repo's minimalism rule, `AGENTS.md` "Absent by
  design").
- **Re-detecting a HOST.** `--detect-endpoint` (`main.rs:409` `detect_endpoint`,
  `config.rs:465` `detect_candidates`) probes *base URLs* via `probe_endpoint`
  (`streamfn.rs:336`); it never inspects a protocol and is untouched.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| 1 | Catalog protocol fact (`limits.rs`) | med | low | low | one `&[&str]` + one pure fn mirroring `model_limit` (`limits.rs:61`); a wrong id just misses (falls back to chat). |
| 2 | `EndpointPref` + `settle` resolution (`streamfn.rs`) | high | med | med | fixes the ask *and* auto-fixes every runtime switch; but it touches the one provider funnel — a bug misroutes every request, and `base()`/`of()` are re-read across reload. |
| 3 | CLI representation (`parse_endpoint`/`Config`/dump) | med | med | med | `Config.endpoint` type change ripples to `to_llm_opts` (`config.rs:1123`), `provider_summary` (`:1094`), `config_dump` (`main.rs:446`), tests — signposted, but real surface. |
| 4 | Runtime-switch correctness | high | low | low | falls out of #2 for free — no protocol change; it is the reason #2 is placed in `settle`. |
| 5 | `/reload` + worker forwarding | med | low | med | a re-exec that forwards a *resolved* `--endpoint` would silently re-pin and regress auto (`repl.rs:438`); small diff, silent-failure mode. |
| 6 | Docs + ADR | low | low | low | record the value + the "unset ≠ chat" decision so the next reader does not re-open it. |

## 4. Interface & structure

### 4.0 The six design questions (recommendation + why)

**Q1 — How "unset" is represented → make it `Option<LlmEndpoint>`, `None` = auto;
also accept `"auto"`.**
Add a third accepted value `"auto"` and, crucially, keep *absence* distinct:
`parse_endpoint` becomes `Result<Option<LlmEndpoint>, String>` —
`None | Some("") | Some("auto") => Ok(None)`, `Some("chat") => Ok(Some(Chat))`,
`Some("responses") => Ok(Some(Responses))`. `Config.endpoint` becomes
`Option<LlmEndpoint>`. This is option **(a)+(c)** combined. Rationale: `None` is
*already* the distinct state on the input side — `FileConfig.endpoint: Option<String>`
(`config.rs:313` `pub endpoint: Option<String>,`) and `ModelProfile.endpoint:
Option<String>` (`config.rs:363`) — and `parse_endpoint` today *collapses* it
(`config.rs:976`); the fix is to stop collapsing. Back-compat: `endpoint = "chat"`
present ⇒ `Some(Chat)` ⇒ pinned chat (unchanged); `endpoint` absent ⇒ `None` ⇒
auto ⇒ resolves to `Chat` for every non-zen base and for every zen model that is
not responses-only ⇒ **byte-identical behavior today**, *except* it now fixes the
responses-only models that currently fail. `--dump-config`'s source label changes
only in wording (`Default` → rendered `auto`). Rejected: option (b) alone (`None`
= auto but no `"auto"` literal) removes the explicit escape hatch a user may want
when they *do* mean "let the catalog decide" with an endpoint present in a layered
config; and reusing `chat` to also mean auto would make an explicit value
meaningless.

**Q2 — Where the mapping lives → fact in `limits.rs` (kernel); policy in
`ModelProfiles::settle`.**
Add `pub fn model_protocol(base_url: Option<&str>, model: &str) ->
Option<LlmEndpoint>` beside `model_limit` (`limits.rs:61`), gated by
`is_opencode_go` (`limits.rs:55`). The **apply-only-when-unpinned policy lives in
the kernel `settle`**, not the CLI. This *deviates* from the recon's "policy
belongs in the CLI, mirroring `probe_endpoint`/`main.rs`" suggestion, deliberately:
the detection analogy is about a *network probe* (I/O, opt-in) whose result is a
default base URL; here the mapping is pure and deterministic, and — decisively —
the CLI cannot fix a mid-session switch (Q4) without a new protocol request.
`settle` is the one funnel every endpoint decision already passes through (boot
`to_llm_opts` → `settle_provider`, `agent.rs:157`; workers `agents.rs:421`; remote
`SetModel` `actor.rs:397`). Type shape:

```rust
// crates/wcode-harness/src/streamfn.rs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EndpointPref { #[default] Auto, Pinned(LlmEndpoint) }

impl EndpointPref {
    pub fn resolve(self, base_url: Option<&str>, model: &str) -> LlmEndpoint {
        match self {
            Self::Pinned(e) => e,
            Self::Auto => crate::limits::model_protocol(base_url, model)
                .unwrap_or(LlmEndpoint::Chat),
        }
    }
}
```

`LlmOpts.endpoint: LlmEndpoint` (`streamfn.rs:43`) **stays concrete** — it is the
*resolved* value that `settle`/`to_llm_opts` writes — so `open_stream`
(`streamfn.rs:731`, `LlmEndpoint::Chat => client.completions_api()…`) is
untouched.

**Q3 — Per-model override interaction → explicit still wins; `None` inherits
*auto*.**
`LlmProfile.endpoint: Option<LlmEndpoint>` (`streamfn.rs:73`) is unchanged; its
`None` = inherit (`streamfn.rs:88-91` `if let Some(endpoint) = self.endpoint {
llm.endpoint = endpoint; }`). Precedence, highest first:

1. `[models.<id>].endpoint = Some(e)` — pins (profile).
2. launch `LlmProvider.endpoint = Pinned(e)` — pins (flag/env/config global).
3. `model_protocol(llm.base_url, llm.model)` — auto.
4. `LlmEndpoint::Chat` — default.

In the auto case the profile's `None` now inherits **auto**, and resolution runs
*after* the profile's `base_url` overlay, so a profile that sets only `base_url`
re-resolves the endpoint against *that* base — matching the existing "a profile
may override only `base_url` without resetting the endpoint" contract
(`streamfn.rs:70-72`).

**Q4 — Runtime model switching → in scope; fix in `settle`, no protocol change.**
`Agent::set_model` (`agent.rs:145`) calls `self.llm.settle_provider()`
(`agent.rs:157`), and the protocol handler routes `Request::SetModel { model }`
(`protocol.rs:257`) → `agent.set_model(model)` (`actor.rs:397`). So resolving auto
inside `settle` makes a mid-session switch (`deepseek-v4.1-flash` → `grok-4.7`), a
remote/TUI `/model`, and a `member op=model` (`member.rs:218` `Request::SetModel`)
all correct with **zero** protocol changes. **Explicitly out of scope:**
`protocol.rs`/`Request::SetEndpoint`, and any change to `Agent::set_model`'s
signature.

**Q5 — Models the catalog cannot serve → fall through to today's `chat`; no new
error class.**
`claude-haiku-5-5`, `minimax-m2.7`, `gemini-4-argon-contributor` speak neither chat
nor responses. They are not in the responses-only set, so `model_protocol` ⇒ `None`
⇒ auto ⇒ `chat` ⇒ the server's `ModelProtocolUnsupported` remains the surface
— **unchanged from today**. Rationale: we cannot "auto" to a protocol wcode does
not implement, and adding a client-side guard is a new failure surface with no
upside over the server's own message; minimalism (`AGENTS.md` "Absent by design").
*Optional follow-up (non-scope):* a client-side "model X is served by neither
`chat` nor `responses`" startup diagnostic.

**Q6 — Non-opencode-go providers → auto is a no-op.**
`model_protocol` returns `None` when `!is_opencode_go(base_url)` (`limits.rs:55`),
so auto ⇒ `Chat`. Confirmed intent: an absent endpoint on a vanilla OpenAI base,
an unset base (`api.openai.com` implicit), a local vLLM/Ollama base — all resolve
to `chat`, **identical to today**. `--dump-config`/`provider_summary` render the
resolved `chat`.

### 4.1 Added

- `crates/wcode-harness/src/limits.rs` — `pub fn model_protocol(...)` (after
  `model_limit`, `limits.rs:61`) + `static OPENCODE_GO_RESPONSES_ONLY: &[&str]`
  beside `OPENCODE_GO` (`limits.rs:80`):
  ```rust
  static OPENCODE_GO_RESPONSES_ONLY: &[&str] = &[
      "grok-4.6", "grok-4.7", "gpt-5.6-luna", "gpt-6-luna",
      "muse-spark-1.2-contributor", "muse-spark-1.3-contributor",
  ];
  ```
- `crates/wcode-harness/src/streamfn.rs` — `pub enum EndpointPref { Auto,
  Pinned(LlmEndpoint) }` + `resolve`.

### 4.2 Changed (each with `file:line` + snippet)

- `crates/wcode-harness/src/streamfn.rs:105` — `pub endpoint: LlmEndpoint,` →
  `pub endpoint: EndpointPref,` on `LlmProvider`.
- `crates/wcode-harness/src/streamfn.rs:113` — `pub fn of(llm: &LlmOpts) -> Self`
  reads `llm.endpoint`; it must convey the pref (either take the pref as a param or
  be built directly by the CLI). Its one caller is `config.rs:1137` `let mut
  profiles = ModelProfiles::new(LlmProvider::of(&opts));`.
- `crates/wcode-harness/src/streamfn.rs:122` — `LlmProvider::apply`
  (`llm.endpoint = self.endpoint;`) — now resolves via the pref.
- `crates/wcode-harness/src/streamfn.rs:162` — `pub fn settle(&self, llm: &mut
  LlmOpts)` — **the core**: apply base_url/api_key, overlay the profile, then
  compute `llm.endpoint` = profile pin ▸ base pin ▸ `model_protocol(llm.base_url,
  llm.model)` ▸ `Chat`.
- `crates/wcode-harness/src/streamfn.rs:181` — `pub fn base(&self) -> Option<&LlmProvider>`
  — consumers must handle `EndpointPref` (below).
- `crates/wcode-cli/src/config.rs:974` — `pub fn parse_endpoint(value: Option<&str>)
  -> Result<Option<LlmEndpoint>, String>`; add `"auto"`; stop collapsing `None`.
- `crates/wcode-cli/src/config.rs:483` — `pub endpoint: LlmEndpoint,` →
  `pub endpoint: Option<LlmEndpoint>,`.
- `crates/wcode-cli/src/config.rs:656` — `let endpoint = parse_endpoint(env.wcode_endpoint.as_deref().or(file.endpoint.as_deref()))`
  — now yields the `Option`.
- `crates/wcode-cli/src/config.rs:661` — provenance: unchanged enum; when the value
  is `None` the dump renders `auto` (see `:446`).
- `crates/wcode-cli/src/config.rs:808-814` — `[models.<id>]` parse: `Some("auto")`
  ⇒ `None` (inherit), preserving today's `None | Some("") => None`.
- `crates/wcode-cli/src/config.rs:1094` — `pub fn provider_summary(&self)` — resolve
  `self.endpoint` (`endpoint_label(self.endpoint)` at `:1101`) against
  `self.effective_base_url(&self.model)` before labelling.
- `crates/wcode-cli/src/config.rs:1123` — `pub fn to_llm_opts(&self) -> LlmOpts`
  — seed `LlmProvider` with `EndpointPref` from `self.endpoint`; set
  `opts.endpoint` to the resolved launch endpoint (`endpoint: self.endpoint` at
  `:1129`).
- `crates/wcode-cli/src/main.rs:652-662` — `--endpoint` apply arm
  (`cfg.endpoint = endpoint;`) — now `Option`; `--endpoint auto` accepted.
- `crates/wcode-cli/src/main.rs:446-449` — `config_dump` endpoint line
  (`"endpoint: {} (source: {})"`) — render the resolved label and `(source: auto)`
  when `cfg.endpoint.is_none()`.
- `crates/wcode-cli/src/repl.rs:438-453` — `reload_args` re-exec: forwards
  `--endpoint chat|responses` from `base.endpoint`. **When the base pref is `Auto`,
  forward `--endpoint auto` (or omit `--endpoint`), never a resolved pin** —
  otherwise the re-exec silently re-pins auto. (The test
  `reload_args_forwards_the_launch_base_not_the_settled_provider`, `repl.rs:1813`,
  must be updated.)
- `crates/wcode-cli/src/agents.rs:414-421` — worker `llm.settle_provider()` — no
  code change; verify the worker (a clone of `t.llm` then `llm.model = model`)
  re-resolves auto for the *worker's* model id.

### 4.3 Tests to extend / keep green

- `crates/wcode-cli/src/config.rs:1180` `endpoint_parses_chat_and_responses` (now
  `Option`); `:1190` `endpoint_rejects_unknown_value` (add `auto` as valid); `:1196`
  `endpoint_env_beats_toml`.
- `crates/wcode-cli/src/config.rs:2336` `mod endpoint_detect_tests` (host detection;
  prove untouched).
- `crates/wcode-cli/src/main.rs:3339` `parse_endpoint_flag`; `:2922`
  `config_dump_names_the_context_window_source`; `:3001`
  `config_dump_names_a_pinned_model_base_not_openai`.
- `crates/wcode-harness/src/streamfn.rs` settle/cache tests (~`:1781-1827`, incl.
  the `ep-swap` endpoint-swap + client-cache test).
- `crates/wcode-cli/src/config.rs` profile-wins test (~`:1290-1320`).
- `crates/wcode-cli/src/repl.rs:1813` reload-forward test.
- `crates/wcode-harness/src/limits.rs` `mod tests` (`:128`).

## 5. Plan

1. **Kernel catalog fact.** Deliverable: `limits::model_protocol(ZEN, "grok-4.7")
   == Some(Responses)`; `… "deepseek-v4-flash" == None`; `… "kimi-k3"` behind a
   non-zen base `== None`; `is_opencode_go` still gates it. Gate: new unit tests in
   `crates/wcode-harness/src/limits.rs` `mod tests`; `cargo test -p wcode-harness
   limits`.
2. **`EndpointPref` type.** Deliverable: `EndpointPref::Auto.resolve(Some(ZEN),
   "grok-4.7") == Responses`; `… "deepseek-v4-flash" == Chat`;
   `EndpointPref::Pinned(Responses).resolve(any, any) == Responses`. Gate: unit test
   in `streamfn.rs`.
3. **`settle` resolves auto.** Change `LlmProvider.endpoint` → `EndpointPref`
   (`:105`), `of`/`apply` (`:113/:122`), `settle` (`:162`). Deliverable: for an
   unpinned base, `settled("grok-4.7").endpoint == Responses` and
   `settled("deepseek-v4-flash").endpoint == Chat`; a profile pin still wins; a
   profile `base_url`-only re-resolves. Gate: extend the `streamfn.rs` settle tests
   + a new test: `set_model("grok-4.7")` after booting chat ⇒ `Responses`.
4. **CLI representation.** `parse_endpoint` → `Option` (`config.rs:974`),
   `Config.endpoint: Option<LlmEndpoint>` (`:483`), `--endpoint auto` (`main.rs:652`),
   `[models]` auto=inherit (`config.rs:808`), dump renders `auto` (`main.rs:446`),
   `provider_summary` resolves (`config.rs:1094`). Deliverable: `--dump-config`
   prints `endpoint: responses (source: auto)` for zen+`grok-4.7` and `endpoint:
   chat (source: auto)` for an OpenAI base. Gate: `parse_endpoint_flag`, a new
   `--dump-config` test, existing config tests updated.
5. **`to_llm_opts` feeds the pref.** `config.rs:1123` — build
   `LlmProvider{ endpoint: EndpointPref::… }` from `self.endpoint`. Deliverable:
   `cfg.to_llm_opts().endpoint` and
   `cfg.to_llm_opts().model_profiles.base().unwrap().endpoint` are auto-consistent.
   Gate: config unit test; `main.rs:3029`
   `detection_lands_before_to_llm_opts_and_settle_keeps_it` stays green.
6. **Reload + worker forwarding.** `repl.rs:438` forwards the *pref*; workers rely
   on `settle` (`agents.rs:421`). Deliverable: a `/reload` of an auto session
   re-execs with `--endpoint auto` (not a pin); a worker on a responses-only model
   gets `Responses`. Gate: `repl.rs:1813` test updated + a new assertion; a worker
   test in `agents.rs`.
7. **Docs + ADR.** Deliverable: `README.md` endpoint value list gains `auto`
   (default when absent); `main.rs` `--help` (`:58`, `:89`) says
   `chat|responses|auto`; `AGENTS.md`/`docs/next-steps.md` point at `D025`;
   `docs/decisions/D025-auto-endpoint-from-model-catalog.md` records "unset ≠ chat".
   Gate: doc review (`--dump-system-prompt` unaffected; inspect `--dump-config`).
8. **Live verification.** The §7.2 commands. Gate: observed output.

## 6. Quality gates

- `cargo test --workspace` — all green (the §4.3 named tests are the regression
  set).
- `cargo clippy --workspace --all-targets` — **clean** (no new warnings; the
  `Option<LlmEndpoint>` change will surface every `endpoint_label(cfg.endpoint)`
  call site — fix each).
- A **live** check against a real binary (a build proves nothing about behavior) —
  §7.2.
- Keep green: `endpoint_parses_chat_and_responses`, `endpoint_rejects_unknown_value`,
  `endpoint_env_beats_toml`, `parse_endpoint_flag`,
  `config_dump_names_the_context_window_source`,
  `config_dump_names_a_pinned_model_base_not_openai`, the `streamfn` settle/cache
  tests, `reload_args_forwards_the_launch_base_not_the_settled_provider`,
  `detection_lands_before_to_llm_opts_and_settle_keeps_it`.

## 7. Testing

### 7.1 Automated

- **Kernel fact (`limits.rs`)** — `model_protocol` returns `Responses` for each of
  the six ids under a zen base, `None` for a both-works id (`deepseek-v4-flash`),
  `None` for an unknown id, `None` for a known id behind a non-zen base (mirrors
  `unknown_model_or_provider_is_none`, `limits.rs:153`), `None` for `base_url =
  None`.
- **Pref (`streamfn.rs`)** — `EndpointPref::resolve` matrix: pinned wins over
  catalog; auto picks catalog; auto off-zen ⇒ chat.
- **Settle (seam)** — with a `ModelProfiles` whose base is `Auto`, `settled(m)`
  gives `Responses` for a responses-only zen id and `Chat` otherwise; a profile
  `endpoint: Some(..)` overrides; a profile `base_url`-only re-resolves against the
  new base; `set_model` mid-session flips chat→responses and back.
- **CLI** — `parse_endpoint(Some("auto")) == Ok(None)`, `parse_endpoint(None) ==
  Ok(None)`, `Some("chat") == Some(Chat)`; `[models.x].endpoint = "auto"` ⇒ inherit;
  `config_dump` shows `(source: auto)` and the resolved label; `provider_summary`
  matches.
- **Reload** — `reload_args` emits `--endpoint auto` when the base pref is `Auto`,
  and still emits the pinned label when `Pinned`.

### 7.2 How a human verifies it

Keyless (no network needed to inspect resolution):

```sh
# zen + a responses-only model, endpoint absent ⇒ responses, auto
cargo run -p wcode-cli -- --model grok-4.7 \
  --base-url https://opencode.ai/zen/go/v1 --dump-config
#   expect: endpoint: responses (source: auto)

# a both-works model ⇒ chat (unchanged)
cargo run -p wcode-cli -- --model deepseek-v4-flash \
  --base-url https://opencode.ai/zen/go/v1 --dump-config
#   expect: endpoint: chat (source: auto)

# non-zen base ⇒ chat (no-op)
cargo run -p wcode-cli -- --model gpt-5.6-luna \
  --base-url https://api.openai.com/v1 --dump-config
#   expect: endpoint: chat (source: auto)

# an explicit pin still wins
cargo run -p wcode-cli -- --model grok-4.7 \
  --base-url https://opencode.ai/zen/go/v1 --endpoint chat --dump-config
#   expect: endpoint: chat (source: Flag)
```

Live against a keyless local endpoint (resolution is observable with a refused
port for the error/retry path):

```sh
WCODE_RETRY_MAX=2 cargo run -p wcode-cli -- --model grok-4.7 \
  --base-url http://127.0.0.1:9/v1 --endpoint auto -p "hi"
```

- Expect the connect/retry failure against the refused port, **not**
  `ModelProtocolUnsupported` — proving the endpoint was derived, not forced. A real
  zen key (if present) verifies the end-to-end: the same model that today 400s on
  chat now answers on responses without any `--endpoint`.

Human `/model` check (TUI or REPL, against a real zen key): boot on
`deepseek-v4-flash` (chat), `/model grok-4.7`, send a prompt — it must succeed
without touching config; `/model deepseek-v4-flash` again — chat again.

## 8. Expected outcome

- `endpoint` absent ⇒ wcode derives `chat`/`responses` from the model id against
  the catalog; an OpenCode Go responses-only model works with no `--endpoint`.
- `endpoint = "auto"` is an accepted explicit value; `endpoint =
  "chat"|"responses"` still pins and always wins over auto.
- A mid-session `/model`, a remote `Request::SetModel`, and a worker's own model all
  re-derive the endpoint with no `--endpoint` and no protocol change.
- Non-OpenCode-Go bases, unset bases, and both-works/unknown zen models behave
  **exactly** as today (chat) — zero regression.
- `--dump-config` shows the resolved endpoint and `(source: auto)`; `/reload`
  preserves auto across re-exec.
- The six responses-only ids are enumerated once, in `limits.rs`, beside the context
  table.

## 9. References

- `crates/wcode-harness/src/limits.rs` — the one catalog (`OPENCODE_GO` `:80`,
  `model_limit` `:61`, `is_opencode_go` `:55`).
- `crates/wcode-harness/src/streamfn.rs` — `LlmEndpoint` `:29`, `LlmOpts.endpoint`
  `:43`, `LlmProfile.endpoint` `:73`, `LlmProvider` `:105`, `settle` `:162`,
  `open_stream` `:725`.
- `crates/wcode-cli/src/config.rs` — `parse_endpoint` `:974`, `FileConfig.endpoint`
  `:313`, `ModelProfile` `:363`, `Config.endpoint` `:483`, `merge` endpoint
  resolution `:656`, `provider_summary` `:1094`, `to_llm_opts` `:1123`.
- `crates/wcode-cli/src/main.rs` — `--endpoint` apply `:652`, `config_dump` `:440`,
  `detect_endpoint` `:409`, detection opt-in `:386`.
- `crates/wcode-cli/src/repl.rs:438` — reload re-exec forwarding;
  `crates/wcode-cli/src/agents.rs:414` — worker settle;
  `crates/wcode-harness/src/actor.rs:397` — `Request::SetModel` →
  `agent.set_model`.
- `README.md` (endpoint value list), `AGENTS.md` "Absent by design",
  `docs/next-steps.md:779`/`:995` (widen-the-catalog, deferred).
- Sibling: ADR `docs/decisions/D025-auto-endpoint-from-model-catalog.md`; work item
  `docs/work/W021-auto-endpoint-from-model.md`.

## 10. Facts the brief did NOT verify

- **The responses-only set itself.** The brainstormer reused the orchestrator's
  empirical probe of the 40 live zen ids (`grok-4.6/4.7`, `gpt-5.6-luna`,
  `gpt-6-luna`, `muse-spark-1.2/1.3-contributor` responses-only) and ran **no**
  network probe (read-only worker). Confirm the six ids once live before shipping;
  the map is the only source and it is a snapshot.
- **`GET /zen/go/v1/models` exposing no protocol** — taken on trust from recon (the
  four-field listing).
- **`/reload` re-exec actually fires on a TUI/`--resume`** and that `--endpoint
  auto` round-trips through `parse_args` — read `reload_args` (`repl.rs:438`) and
  `parse_endpoint_flag` (`main.rs:3339`) but no live resume was traced.
- **`LlmProvider::of`'s other consumers** (only `config.rs:1137` found) and whether
  `base()`'s pref survives every `/reload`/resume path verbatim — the
  serialization/forwarding of an `EndpointPref` across the process boundary is
  asserted from comments, not exercised.
- **`provider_summary`'s consumers** (startup diagnostic, REPL banner, TUI) all
  tolerating a resolved-in-place label — the two call-site docs (`config.rs:1092`,
  `main.rs:422`) were read but the TUI banner was not audited.

## 11. Outcome

**Shipped (uncommitted at the time of writing).** All 15 sketch blocks were filled
and the markers deleted; the change spans `crates/wcode-harness/src/{limits,streamfn}.rs`,
`crates/wcode-harness/tests/agent_tests.rs`, `crates/wcode-cli/src/{config,main,repl,agents}.rs`,
`README.md`, plus new `docs/decisions/D025-auto-endpoint-from-model-catalog.md`
and `docs/next-steps.md` item 64.

**Gates.** `cargo test --workspace` — 1233 passed, 0 real failures (16 new tests);
`cargo clippy --workspace --all-targets` — clean; zero `SKETCH` markers.

**Live.** All six responses-only ids re-confirmed against the real Go gateway (each
rejects `/chat/completions` with `ModelProtocolUnsupported`, answers on
`/responses`) — **0 mismatches**, closing §10's biggest unknown. `--dump-config`
on the built binary: `grok-4.7`@zen ⇒ `responses (source: auto)`;
`deepseek-v4-flash`@zen ⇒ `chat (source: auto)`; `gpt-5.6-luna`@openai ⇒
`chat (source: auto)`; `--endpoint chat` ⇒ `chat (source: flag)`.

**Review.** Layer 1 APPROVE (one blocker: the missing `--dump-config` test
skeleton — cleared). Layer 2 APPROVE, in two passes: the first pass also caught
two test-quality defects, both fixed — `settle_auto_derives_from_the_catalog`
seeded its own computation (now a stale `LlmEndpoint::Chat`, **mutation-proven** to
fail if `settle` step 3 is deleted), and the brief §5 step 6 worker gate was
missing (now `an_auto_worker_derives_the_endpoint_from_its_own_model`).

**Deviation from the brief's file list.** Nothing beyond the sketch's own
enumeration; the sketcher found ~13 additional break sites in the brief's §4.3 gap
(every `LlmProvider { endpoint: … }` literal and `== base.endpoint` comparison),
and the review confirmed the list complete.

**Operator note.** The repo's own `~/.config/wcode/config.toml` pins
`endpoint = "chat"`, so auto is **inert** until that line is removed or set to
`"auto"` — a pin always wins by design.

**Non-gating residuals** (logged at `docs/next-steps.md`, "Open (W021
residuals)"): the pre-existing `wcode-tui` syntax-theme test race; no end-to-end
TUI `/reload` round-trip of `EndpointPref`; the six-id snapshot is time-sensitive;
and the still-deferred catalog widening. Not done: nothing was committed — the
tree is left dirty for human review.
