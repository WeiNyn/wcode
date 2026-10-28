# D044 — Model catalog cache and refresh policy

- **Status:** accepted
- **Date:** this session
- **Relates to:** `crates/wcode-harness/src/catalog.rs`; W038; D043 (seam),
  W014 (no network at boot)
- **Work item:** `work/W038-model-catalog-sync.md`

## Context

The catalog must stay current without a release, but W014 forbids network at
boot. The cache therefore has to be read synchronously and refreshed in the
background (or on demand).

## Decision

- **Cache location.** `~/.local/share/wcode/models.dev.json` (data dir, matching
  the session store), overridable via `WCODE_MODELS_CACHE`. Schema version 1;
  a mismatched or torn cache degrades to the seed and never panics.
- **Atomic write.** Temp file + rename, so a concurrent reader sees either the
  old file or the new one.
- **Staleness (stale-while-revalidate).** TTL default 24 h
  (`WCODE_MODELS_TTL_SECS`, default 86400). Boot reads the cache regardless of
  age; if older than the TTL (or absent), `refresh_background()` is kicked and
  the available catalog (cache → seed) is used in the meantime. A failed refresh
  keeps the previous catalog.
- **Single-flight refresh.** `REFRESHING` (AtomicBool) + `REFRESH_LOCK` (Mutex);
  fetch + parse happen *outside* the lock, only the cache write + `Arc` swap
  happen under it.
- **Background refresh needs a runtime.** `refresh_background()` uses
  `tokio::runtime::Handle::try_current()`; outside a runtime it is a no-op
  (acceptable for the CLI; library users call `refresh()` directly).
- **Source URL.** `catalog_url()` honors `WCODE_MODELS_URL` so a non-models.dev
  source can be pointed at without recompiling; `Fetcher` is the HTTP seam and
  `refresh_at`/`refresh_with_url` allow adding a provider that is not on
  models.dev.
- **Load is not refresh.** `load()` reads the cache synchronously and can never
  touch the network (no staleness check, no fetch); `init()` = `load()` + a
  background refresh when the cache is absent or older than the TTL. A pure
  one-shot (`--dump-config`, `--dump-catalog`) calls `load()`; a session boot
  (`dispatch`, `serve`) calls `init()`.
- **Compiled-in seed.** The regenerated models.dev snapshot (`scripts/refresh-seed.sh`,
  which merges by default) is the offline fallback and the first-run seed; it is
  a superset of the tables it replaced (D043).
- **CLI surface.** `--refresh-models` (synchronous fetch, print counts +
  `fetched_at`, exit), `--dump-catalog` (resolved provider + counts), and
  `--dump-config` (context line naming the catalog source).
- **Fetch timeout.** The blocking client gets an explicit 30 s timeout: a
  half-open connect would otherwise hang `--refresh-models` forever. The fetch
  runs on a spawned thread because a `reqwest::blocking` client owns a runtime
  that must not be dropped inside the async caller.
- **Protocol snapshot stays separate.** The responses-only id list is
  hand-maintained; `refresh()` preserves it when the fetched catalog has none.
  `--refresh-models` does not update it (D-B).
