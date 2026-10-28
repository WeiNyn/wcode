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
  swaps the `Arc`. `set_current`/`set_fetcher` are the test seams.
- **Tiers stored, ignored (D028).** `PriceTier` is persisted but `cost_of` uses
  the base rate.

## Consequences

- Adding a model no longer needs a release once the cache is refreshed.
- `DEFAULT_CONTEXT_WINDOW` is the fallback for any unresolved provider/model.
- The compiled-in `seed()` keeps offline/first-run behaviour identical to the
  old static tables.
SCRIPT
echo "written D043"