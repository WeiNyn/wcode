# D043 — Runtime-synced model catalog seam (context + price)

- **Status:** accepted
- **Date:** this session
- **Relates to:** `crates/wcode-harness/src/catalog.rs` (new), `limits.rs`,
  `pricing.rs`; W038 (`work/W038-model-catalog-sync.md`); D025 (auto endpoint
  from the static catalog), D028 (base-rate-only pricing)
- **Work item:** `work/W038-model-catalog-sync.md`

## Context

`limits::model_limit` and `pricing::model_price` used to read two hand-maintained
static slices (`OPENCODE_GO`) gated by `is_opencode_go`. That meant new models
required a release, the context and price tables could drift apart, and a
base URL spelled `https://opencode.ai/zen/go/v1` (wcode's gateway) never matched
models.dev's advertised `https://opencode.ai/inference/go/openai/v1`.

## Decision

- **One catalog, not two tables.** A single `Catalog` (`catalog.rs`) holds
  per-provider `ModelEntry { context, output, price, tiers }`. `limits` and
  `pricing` both read `catalog::current()` filtered by `catalog::provider_for(base_url)`,
  so they can never disagree about which provider they describe.
- **Provider resolution, not id guessing.** `provider_for` normalizes a base URL
  (lowercase, trim trailing slash, strip `/v1`, rewrite `zen/go` →
  `inference/go/openai`) and matches it against each provider's `api` plus an
  `aliases` list. The alias list is seeded for providers whose models.dev `api`
  is empty (`openai`, `groq`, …) so URL matching works offline after a refresh.
- **Generic, not opencode-only.** `model_limit`/`model_price` apply the catalog
  result for *any* resolved provider. Only `model_protocol` keeps the
  `is_opencode_go` gate (responses-only is an opencode-specific fact).
- **No network on the boot path (W014).** `init()` reads the cache (or the
  compiled-in `seed()`) synchronously and kicks `refresh_background()` only when
  stale. `refresh()` is the sole synchronous network path (`--refresh-models`).
- **Global `Arc<Catalog>` behind an `RwLock`.** Reads clone an `Arc`; a refresh
  swaps the `Arc`. No TEST writes it: `provider_for_in` / `model_limit_in` /
  `model_price_in` take an explicit `&Catalog` (the pure seams), so a test edits
  a catalog it owns and the parallel readers (`limits`, `pricing`, `actor`)
  cannot be perturbed. `load_into_global` (reachable as `load()`/`init()`, both
  production boot wiring) is the only writer.
- **Tiers stored, ignored (D028).** `PriceTier` is persisted but `cost_of` uses
  the base rate.
- **`DEFAULT_CONTEXT_WINDOW` is 256_000** (raised from 128_000 at the user's
  explicit request during W038 — locked, not a default to be re-tuned here). It
  is the window used when the catalog knows NOTHING about the model, and the
  raise is a deliberate trade in the blind spot:
  - a model the catalog knows keeps its real window — `CompactionPolicy::context_window`
    asks the catalog first, so 256k never overrides a real 202k (`glm-5`) or a
    real 1M (`deepseek-v4.1-flash`);
  - an UNKNOWN model therefore compacts later than it did at 128k (at
    256k − the reply reserve instead of 128k − the reserve), and a genuinely
    128k model that is missing from the catalog can overflow before wcode
    compacts. The catalog — not the fallback — is what keeps that set small; the
    fallback is the number that must at least not be absurd for a modern model.

## Consequences

- Adding a model no longer needs a release once the cache is refreshed.
- `DEFAULT_CONTEXT_WINDOW` is the fallback for any unresolved provider/model.
- The compiled-in `seed()` is a SUPERset of the old static tables, not a copy of
  the live payload: 54 limit rows and 37 price rows, 17 of which models.dev no
  longer serves (`glm-5`, `grok-4.6`, …). So offline/first-run behaviour matches
  the pre-W038 tables exactly (`scripts/refresh-seed.sh` merges by default and
  needs `--prune` to delete), and a model the catalog cannot resolve falls back
  to `DEFAULT_CONTEXT_WINDOW` — never to a zero window (`model_limit` returns
  `None` when an entry's `context` is `0`).
- `--dump-config` describes the on-disk catalog, not the seed: `catalog::load()`
  reads the cache with no network, and `catalog::init()` adds the background
  refresh on top. A pure one-shot therefore never opens a socket, and a session
  boot still refreshes a stale cache.
