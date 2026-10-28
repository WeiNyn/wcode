# W038 — Runtime-synced model catalog (context + price) sourced from models.dev — **brief**

- **Status:** landed — P0–P7 complete (catalog seam, cache/refresh, CLI surface, seed helper, docs/decisions)
- **Work item:** W038 · file `docs/work/W038-model-catalog-sync.md` (slug `model-catalog-sync`)
- **Decisions:** none locked yet; this brief proposes resolutions for Q1–Q8 (§5) and names the new decision records it will need (D043-ish for the catalog seam, D044 for the cache/refresh policy)
- **Author:** `agent:brainstormer`
- **Date:** 2026-10 (current session)
- **Relates to:** W014 (rejected network-at-boot), W021/D025 (auto endpoint from the static catalog), W024/D028 (static pricing table + tiers dropped), `session_groups::Manifest` (on-disk JSON template)

---

## 0. Recon — what I verified, and what I did NOT

### 0.1 Verified (each with an anchor)

| Fact | Evidence |
|---|---|
| `limits.rs` exposes three functions gated by `is_opencode_go` | `limits.rs:57` `pub fn is_opencode_go` · `:63` `pub fn model_limit` · `:70` `pub fn model_protocol` · `:86` |
| The static context table has 45 rows | `limits.rs:119` `static OPENCODE_GO: &[(&str, u64, u64)] = &[` … `:165` |
| `pricing.rs` exposes `model_price` + `cost_of`, gated identically | `pricing.rs:115` `pub fn model_price` · `:142` `pub fn cost_of` · `:116` `if crate::limits::is_opencode_go(base_url)` |
| The static price table has 32 rows | `pricing.rs:219` `static OPENCODE_GO: &[(&str, u64, u64, u64, u64)] = &[` … `:252` |
| models.dev's `opencode-go` provider uses a **different** API URL than wcode's gate | live fetch: provider `opencode-go`, `api = https://opencode.ai/inference/go/openai/v1`, 37 models. wcode's gate matches `opencode.ai/zen/go` (`limits.rs:58`). The real OpenCode Go gateway that wcode talks to is `https://opencode.ai/zen/go/v1` (returns 200 on `/models`); the models.dev URL returns 404. **Mismatch confirmed.** |
| The `opencode` (non-Go) provider also exists on models.dev | live fetch: `opencode` → `api = https://opencode.ai/inference/openai/v1` |
| models.dev carries `limit.context`, `limit.output`, `cost{input,output,cache_read,cache_write}`, optional `tiers`/`context_over_200k`, plus `input_audio` | live fetch above; 6 models carry `tiers`; 4 are zero-priced (free) |
| Cost values convert exactly to micro-USD | live fetch: max 6 decimals across all entries |
| W014 explicitly rejected a runtime fetch at boot | `docs/work/W014-sidebar-root-compaction-notices-thinking.md:28` — "A runtime models.dev fetch at startup — network at boot, non-deterministic, contradicts minimalism" (Non-scope) |
| D025 put the protocol snapshot in `limits::model_protocol`, gated identically | `docs/decisions/D025-...:42` |
| D028 dropped tier support and locked base-rate-only pricing | `docs/decisions/D028-...:44` |
| `session_groups::Manifest` is the on-disk JSON template | `session_groups.rs:359` `pub struct Manifest` · `:455` `pub fn load_manifest` · `:462` `pub fn write_manifest` |
| `config_dir()` is `~/.config/wcode`; sessions live under `~/.local/share/wcode/sessions` | `config.rs:1323` · `repl.rs:285` |
| ~15 call sites consume the three functions | grep: `model_limit` at `repl.rs:1489`, `main.rs:625/1073/1790/1989`, `agents.rs:536`, `actor.rs:495/665`, `compaction.rs:150`, `streamfn.rs:59`, `app.rs:2155`, `settings.rs:323/380`; `model_price`/`cost_of` at `actor.rs:352`; `model_protocol` at `streamfn.rs:59` |
| The actor prices usage server-side from `llm.base_url`/`llm.model` | `actor.rs:352` · `agent.rs:488` |

### 0.2 NOT verified (named, per the doctrine)

1. Whether the OpenCode Go gateway also answers on the models.dev `inference/go/openai/v1` path — it returns 404 today. Unverified whether that's a different gateway or a docs typo.
2. models.dev's update cadence and ETag/`Last-Modified` support — unverified.
3. Whether `cost.input_audio` is billed in addition to `input` — unverified; out of scope.
4. Whether `tiers[].tier.size` is input-tokens or total-context — unverified.
5. Whether a `ModelCatalog` global would break the harness's pure-function tests — untraced.
6. Remote/`--socket` path: whether a mid-sync catalog change reaches an already-running TUI — unverified.
7. Cargo dependency availability for HTTP in the harness — unverified.
8. Size of the full models.dev payload — unrecorded.

---

## 1. The ask

Replace wcode's **hardcoded** model context-window table (`limits.rs`, 45 ids) and price table (`pricing.rs`, 32 ids) with a **runtime-synced, on-disk cached manifest** sourced from `models.dev`, so that:

- context windows, output ceilings, prices (incl. cache read/write), and the responses-only protocol fact are **data, not code**;
- the catalog **stays current** without a release;
- the **matching problem is fixed**: a wcode session whose base URL is `https://opencode.ai/zen/go/v1` is correctly recognized as the `opencode-go` provider even though models.dev advertises `https://opencode.ai/inference/go/openai/v1`;
- the **W014 doctrine holds**: no network on the boot path; the cache is read synchronously and a refresh happens in the background (or on demand).

---

## 2. Scope / Non-scope

**In scope.**

- A new on-disk **manifest** format (JSON, mirroring `session_groups::Manifest`: serde, tolerant read, atomic write) holding per-provider model entries: `context`, `output`, `cost{input,output,cache_read,cache_write}` (micro-USD/Mtok), optional `tiers`, plus a provider-level `api` URL and an alias list.
- A **provider resolver**: normalize a runtime `base_url` to a provider id via the manifest's `api` URL plus a small built-in alias table (so `zen/go/v1` ⇔ `opencode-go`).
- **Replace the bodies** of `limits::model_limit`, `limits::model_protocol`, and `pricing::model_price` so they read from a cached catalog instead of the two `static` slices, preserving their signatures and `Option`-typed honesty. (Widened: `model_limit`/`model_price` apply to any resolved provider; `model_protocol` stays opencode-go-gated.)
- A **cache** at `~/.local/share/wcode/models.dev.json` (data dir, matching the session store convention) with a staleness policy and atomic write.
- A **background refresh** (never on the boot path) that fetches models.dev, rewrites the cache, and swaps the in-memory catalog.
- A **compiled-in seed** (the current 45/32-row snapshot) so a first run with no cache and no network still has today's data.
- `--refresh-models` (and `WCODE_REFRESH_MODELS=1`) to force a synchronous refresh; `--dump-catalog` to print the resolved provider + counts.
- Tests for: provider resolution, cache round-trip, staleness, float→micro conversion, unknown→`None`, tier deferral, concurrency of the global swap.
- Docs: README note, a `D` record, tracker row, `--dump-config` line naming the catalog source.

**Non-scope.**

- **Network at boot / blocking fetch** — forbidden by W014.
- **Tiered pricing in v1** — D028's base-rate-only stance stands; `tiers` are stored but ignored.
- **The responses-only protocol snapshot** — stays a separate, hand-maintained list; models.dev carries no wire-protocol fact.
- **A pricing/config file** — doctrine: behaviour is code, not config.
- **Per-model `base_url` overrides for pricing** — `cost_of` keeps using the session's launch base URL.
- **Audio pricing** (`input_audio`) — no consumer today.
- **Removing the compiled seed** — the seed is the offline fallback.
- **Changing `DEFAULT_CONTEXT_WINDOW`** — now 256_000 (updated per user request).
- **Rewriting the ~15 call sites' signatures** — they keep calling `model_limit`/`model_price`/`model_protocol`.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | WS-A · manifest schema + provider resolver | high | med | med | Data shape + matching fix; alias resolution wrong ⇒ pricing silently disabled. |
| P1 | WS-B · repoint `limits`/`pricing` (keep signatures) | high | low | low | Mechanical find in in-memory catalog; existing row-pinned tests must still pass via seed. |
| P2 | WS-C · cache + staleness + atomic write | high | med | med | New I/O; must not block boot; torn cache degrades to seed, never panics. |
| P3 | WS-D · background refresh + global swap | med | med | med | Concurrency: refresh must not tear a read; `Arc` swap + last-good invariant. |
| P4 | WS-E · compiled-in seed | low | low | low | Reuse today's 45/32 rows as bootstrap. |
| P5 | WS-F · CLI surface | med | low | low | Thin wrappers over harness API. |
| P6 | WS-G · tests + docs | med | med | low | Rewrite row-pinned tests to read from a test catalog; add resolution/cache tests. |

Ordering: P0 → P1 → (P2 ∥ P4) → P3 → P5 → P6.

---

## 4. Interface & integration points

### 4.1 The functions that do NOT change shape (the seam)

```rust
// crates/wcode-harness/src/limits.rs:57-91
pub fn is_opencode_go(base_url: Option<&str>) -> bool;
pub fn model_limit(base_url: Option<&str>, model: &str) -> Option<ModelLimit>;
pub fn model_protocol(base_url: Option<&str>, model: &str) -> Option<LlmEndpoint>;
// crates/wcode-harness/src/pricing.rs:115
pub fn model_price(base_url: Option<&str>, model: &str) -> Option<ModelPrice>;
// crates/wcode-harness/src/pricing.rs:142
pub fn cost_of(messages, base_url, fallback_model) -> Option<Cost>;
```

Call sites (unchanged signatures):

- `crates/wcode-cli/src/repl.rs:1489`
- `crates/wcode-cli/src/main.rs:625, 1073, 1790, 1989`
- `crates/wcode-cli/src/agents.rs:536`
- `crates/wcode-harness/src/actor.rs:495, 665` and `:352`
- `crates/wcode-harness/src/compaction.rs:150`
- `crates/wcode-harness/src/streamfn.rs:59`
- `crates/wcode-tui/src/app.rs:2155`
- `crates/wcode-tui/src/settings.rs:323, 380`

### 4.2 The new catalog seam (proposed)

```rust
// crates/wcode-harness/src/catalog.rs (new)
pub struct ModelEntry {
    pub context: u64,
    pub output: u64,
    pub price: Option<ModelPrice>,
    pub tiers: Vec<PriceTier>,   // stored, ignored in v1 (D028)
}
pub struct ProviderEntry {
    pub id: String,
    pub api: String,
    pub aliases: Vec<String>,
    pub models: BTreeMap<String, ModelEntry>,
}
pub struct Catalog {
    pub providers: BTreeMap<String, ProviderEntry>,
    pub protocol: BTreeMap<String, Vec<String>>,
    pub fetched_at: Option<i64>,
    pub source: CatalogSource,   // Seed | Cache | Network
}
pub fn current() -> Arc<Catalog>;
pub fn refresh() -> Result<(), CatalogError>;       // blocking, for --refresh-models
pub fn refresh_background();                       // spawns a task, no-op if in flight
pub fn provider_for(base_url: Option<&str>) -> Option<String>;
```

`limits::model_limit` reads `catalog::current()` filtered by `provider_for` for any resolved provider (no opencode-go gate).

### 4.3 Normalization for the URL mismatch (Q1)

Normalize a base URL to a comparison key:

```
key(base_url) = lowercase(trim_trailing_slash(base_url))
                .strip_suffix("/v1")
                .replace("zen/go", "inference/go/openai")   // legacy alias
```

Match against each provider's `api` (similarly normalized) plus `aliases`. For `opencode-go` the alias list is seeded with `opencode.ai/zen/go` and `opencode.ai/inference/go/openai`, so both spellings resolve to the same provider. For providers whose models.dev entry has no `api` URL (openai, groq, moonshot, mistral, ...), full-path aliases are seeded (e.g. `https://api.openai.com/v1`, `https://api.groq.com/openai/v1`) so URL matching works. The provider id comes from the manifest's top-level key; the URL is only a lookup handle.

### 4.4 Cache file shape (mirroring `Manifest`)

```json
{
  "schema": 1,
  "fetched_at": 1730000000,
  "source_url": "https://models.dev/api.json",
  "providers": {
    "opencode-go": {
      "api": "https://opencode.ai/inference/go/openai/v1",
      "aliases": ["opencode.ai/zen/go", "opencode.ai/inference/go/openai"],
      "models": {
        "deepseek-v4.1-flash": {
          "context": 1000000,
          "output": 384000,
          "price": { "input": 150000, "output": 600000, "cache_read": 3000, "cache_write": 0 },
          "tiers": []
        }
      }
    }
  },
  "protocol": { "opencode-go": ["grok-4.6", "grok-4.7", "..."] }
}
```

Atomic write via temp-file + rename. A torn or schema-mismatched cache is treated as absent (returns the seed), never panics.

### 4.5 Global state (Q2)

Back the unchanged signatures with a process-global `Arc<Catalog>` behind a `RwLock` (or `arc_swap`). Reads never allocate beyond an `Arc` clone; a refresh swaps the `Arc`. Tests seed the global with a temp catalog via a `#[cfg(test)]` setter.

---

## 5. Decisions to settle (open questions from recon)

- **Q1 — Matching:** Normalized URL + alias list (§4.3).
- **Q2 — Global state vs threading:** Global `Arc<Catalog>` (§4.5).
- **Q3 — Cache location + staleness:** `~/.local/share/wcode/models.dev.json`. TTL 24 h (env `WCODE_MODELS_TTL_SECS`, default 86400). Serve stale-while-revalidate: read cache synchronously at boot regardless of age; if older than TTL (or absent), kick `refresh_background()` and use whatever is available (cache → seed). Failed refresh keeps previous catalog.
- **Q4 — Generalize beyond opencode-go:** Generic now (widened per user request). `model_limit`/`model_price` apply the catalog result for ANY provider `provider_for` resolves — no opencode-go gate. Manifest already stores all 232 providers. `model_protocol` keeps its opencode-go gate (responses-only is opencode-specific). `is_opencode_go` stays for protocol/diagnostics. Providers without a models.dev `api` (e.g. `openai`) need alias seeding to be reachable by URL. W039 folded into W038.
- **Q5 — Tier support:** Defer (D028). Store `tiers` but `cost_of` uses base rate.
- **Q6 — Protocol snapshot separate:** Yes. Moved into manifest's `protocol` section for co-location, but source of truth remains the live probe.
- **Q7 — Refresh vs W014 "no network at boot":** Reconciled by construction: boot reads cache/seed only; `refresh_background()` spawned after load; `--refresh-models` is the only synchronous network path.
- **Q9 — Generic fetch (new):** The fetch is extensible: `catalog_url()` honors `WCODE_MODELS_URL`; `refresh_from(url, fetcher)` / `refresh_with_url(url, fetcher)` allow a new provider/source; `Fetcher` trait is the HTTP seam; `set_current` lets a caller install a hand-built catalog for a provider not on models.dev. Default source remains models.dev.

Decisions still needed from the human:

- **D-A:** TTL default — 24 h vs. 7 days (recommend 24 h).
- **D-B:** Whether `--refresh-models` also updates the protocol snapshot (recommend no).
- **D-C:** Seed regeneration — hand-maintained vs. build script (recommend hand-maintained + `scripts/refresh-seed.sh`).
- **D-D:** Cache invalidation on schema bump (recommend `schema` field; mismatched ⇒ absent).

---

## 6. Numbered plan (deliverable + gate per step)

1. **P0 — Catalog module + provider resolver.** Deliverable: `crates/wcode-harness/src/catalog.rs` with `Catalog`, `ModelEntry`, `ProviderEntry`, `provider_for`, `current()` (seed-backed), unit tests for `provider_for`. Gate: `cargo test -p wcode-harness catalog::` green.
2. **P1 — Repoint `limits`/`pricing`.** Deliverable: `model_limit`, `model_protocol`, `model_price` read from `catalog::current()`; the two `static OPENCODE_GO` slices become the seed loader (`seed()`). Gate: `cargo test --workspace` green; existing tests pass against seed.
3. **P2 — Cache read/write + staleness.** Deliverable: `catalog::load_cache`, `write_cache`, TTL check, atomic write, tolerant read. Gate: unit tests for round-trip, torn file → `None`, schema mismatch → `None`, TTL boundary.
4. **P3 — Boot wiring (no network).** Deliverable: `catalog::init()` called from `main.rs`/lib that loads cache (or seed) into the global; `refresh_background()` spawned after. Gate: `--dump-config` still prints `context: … (source: model catalog)` for a zen model with no network.
5. **P4 — Background refresh + global swap.** Deliverable: `refresh()` (blocking) and `refresh_background()`; atomic `Arc` swap; single-flight guard. Gate: test that a refresh with a stubbed HTTP source replaces the catalog and a failed refresh leaves the previous one.
6. **P5 — CLI surface.** Deliverable: `--refresh-models`, `--dump-catalog`, env `WCODE_MODELS_TTL_SECS` and `WCODE_MODELS_CACHE`. Gate: live run against keyless endpoint + refused port.
7. **P6 — Seed regeneration helper.** Deliverable: `scripts/refresh-seed.sh` (or example binary) that fetches models.dev and rewrites seed literals. Gate: manual run produces a diff that compiles.
8. **P7 — Docs + decision records.** Deliverable: `docs/decisions/D043-model-catalog-seam.md`, `D044-model-catalog-cache.md`, README note, tracker row. Gate: anchors resolve.

---

## 7. Quality gates (exact commands)

```sh
cargo build --bin wcode
cargo test --workspace
cargo clippy --workspace --all-targets
WCODE_RETRY_MAX=2 cargo run -p wcode-cli -- --base-url http://127.0.0.1:9/v1 --dump-config
cargo run -p wcode-cli -- --refresh-models
cargo run -p wcode-cli -- -p "say hi"
```

---

## 8. Testing

**Automated.**

- `catalog::provider_for` matrix (zen/go, inference/go/openai, mixed case, trailing slash, non-go, None).
- `catalog::load_cache` round-trip + torn + schema-mismatch.
- `limits::model_limit` returns the seed's window for `step-5-preview-free` even with no cache (offline).
- `limits::model_limit` returns `None` for a base URL that resolves to no provider (e.g. unknown host) even if the id is in the catalog.
- `pricing::model_price` returns `Some(ModelPrice::default())` for `step-5-preview-free`.
- Float→micro conversion exact for `deepseek-v4.1-flash` (150000/600000/3000/0) and `mimo-v2.6-pro` (435000/870000/3625/0).
- Global swap visibility.
- `cost_of` still matches the W024 golden when pointed at the refreshed catalog.

**Human (live).**

1. `cargo run -p wcode-cli -- --refresh-models` → prints counts + `fetched_at`.
2. `cargo run -p wcode-cli -- --dump-config` (zen config) → `context: 1000000 tokens (source: model catalog)` for `deepseek-v4.1-flash`.
3. REPL `/usage` on a zen session → `~$` present.
4. Disconnect network, delete the cache, run → no panic, seed values used.
5. Add a brand-new model id to a temp cache file → `model_limit` returns its window without a recompile.

---

## 9. Expected outcome

- `limits.rs` and `pricing.rs` contain **no** model rows; the rows live in `~/.local/share/wcode/models.dev.json` (and the compiled seed).
- `is_opencode_go` is still used for the protocol snapshot and diagnostics; limits/prices are now generic over any provider the catalog resolves. A zen base URL resolves to `opencode-go` for both `zen/go/v1` and `inference/go/openai/v1` spellings.
- Boot never touches the network; a refresh is opt-in or background, and a failed refresh never downgrades a good cache.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` green.
- `PROTOCOL_VERSION` unchanged; no new config key; no change to the ~15 call sites' signatures.
- Tier support remains a named follow-up; protocol snapshot remains separate.

---

## 10. References

- `crates/wcode-harness/src/limits.rs` (whole file)
- `crates/wcode-harness/src/pricing.rs` (whole file)
- `crates/wcode-harness/src/compaction.rs:149-160`
- `crates/wcode-harness/src/streamfn.rs:40-65`
- `crates/wcode-harness/src/actor.rs:339-355`, `:490-500`, `:660-670`
- `crates/wcode-cli/src/session_groups.rs:338-467`
- `crates/wcode-cli/src/config.rs:1323`, `repl.rs:285`
- `docs/work/W014-sidebar-root-compaction-notices-thinking.md:28`
- `docs/decisions/D025-auto-endpoint-from-model-catalog.md`
- `docs/decisions/D028-usage-telemetry.md`
- `docs/work/W024-usage-telemetry.md`
- External: `https://models.dev/api.json` → `opencode-go` (37 models, `api = https://opencode.ai/inference/go/openai/v1`)

---

**Ends with a brief.** Recorded by the orchestrator; approved by the human before any code.
