//! Runtime-synced model catalog: context windows, output ceilings, prices, and
//! (co-located but hand-maintained) protocol facts, sourced from models.dev.
//!
//! W038: replaces the two hand-maintained static tables in `limits.rs` and
//! `pricing.rs` with a single on-disk cached manifest. The boot path never
//! touches the network (W014): [`init`] reads the cache (or the compiled-in
//! [`seed`]) synchronously and kicks a background refresh when stale. A
//! synchronous refresh is available via [`refresh`] (`--refresh-models`).
//!
//! Shape mirrors `session_groups::Manifest`: serde, tolerant read, atomic write.
//! A torn or schema-mismatched cache degrades to the seed and never panics.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::pricing::ModelPrice;

/// Schema version of the on-disk cache file. A mismatched cache is treated as
/// absent (returns the seed), never panics (D-D).
pub const SCHEMA_VERSION: u32 = 1;

/// models.dev endpoint that produces the catalog payload.
pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// Default staleness TTL in seconds. Overridable via `WCODE_MODELS_TTL_SECS`.
pub const DEFAULT_TTL_SECS: u64 = 86_400;

/// Human-readable default cache location (the real path is computed from
/// `$HOME`, see [`default_cache_path`]).
pub const DEFAULT_CACHE_PATH: &str = "~/.local/share/wcode/models.dev.json";

/// Default cache file name.
pub const DEFAULT_CACHE_FILE_NAME: &str = "models.dev.json";

/// Where a catalog came from — for diagnostics / `--dump-catalog`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CatalogSource {
    Seed,
    Cache,
    Network,
}

/// One model's context/output/price entry. `price` is `None` when models.dev
/// carries no `cost` block for the id (rare); `tiers` are stored but ignored
/// in v1 (D028 base-rate-only).
///
/// `context == 0` means the window is UNKNOWN, not zero: models.dev omits the
/// `limit` block for a multimodal/id-less model (160 of them live —
/// `openai/gpt-image-2`, many `nvidia/*`), and `#[serde(default)]` turns that
/// absence into a hard `0`. [`crate::limits::model_limit_in`] therefore treats
/// a zero-context entry as "no limit known" and returns `None`; the entry is
/// still kept so its `price` keeps answering.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelEntry {
    pub context: u64,
    pub output: u64,
    pub price: Option<ModelPrice>,
    pub tiers: Vec<PriceTier>,
}

/// A tiered rate. Stored, ignored in v1 (D028). `size` is the tier threshold
/// in input tokens (Q8 of the brief leaves the exact unit unverified).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PriceTier {
    pub size: u64,
    pub price: ModelPrice,
}

/// One provider's catalog slice.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderEntry {
    pub id: String,
    pub api: String,
    pub aliases: Vec<String>,
    pub models: BTreeMap<String, ModelEntry>,
}

/// The in-memory catalog. Reads clone an `Arc<Catalog>`; a refresh swaps the
/// `Arc` (Q2). `protocol` is the hand-maintained responses-only snapshot,
/// co-located but not sourced from models.dev (Q6).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Catalog {
    pub providers: BTreeMap<String, ProviderEntry>,
    pub protocol: BTreeMap<String, Vec<String>>,
    pub fetched_at: Option<i64>,
    pub source: CatalogSource,
}

/// Errors surfaced by refresh / cache I/O.
#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("invalid schema (expected {expected}, got {got})")]
    Schema { expected: u32, got: u32 },
    #[error("parse: {0}")]
    Parse(String),
}

// ---------------------------------------------------------------------------
// Global state (Q2)
// ---------------------------------------------------------------------------

static CATALOG: OnceLock<RwLock<Arc<Catalog>>> = OnceLock::new();

fn global() -> &'static RwLock<Arc<Catalog>> {
    CATALOG.get_or_init(|| RwLock::new(Arc::new(seed())))
}

static REFRESHING: AtomicBool = AtomicBool::new(false);
static REFRESH_LOCK: Mutex<()> = Mutex::new(());

// Fetcher injection (test-only surface for stubbing HTTP in refresh tests).
pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<String, CatalogError>;
}

/// How long the catalog fetch may take before it is abandoned. Without it a
/// half-open TCP connect blocked `--refresh-models` (and a background refresh)
/// forever — `reqwest` has no default request timeout for a blocking client.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

struct DefaultFetcher;

impl Fetcher for DefaultFetcher {
    fn fetch(&self, url: &str) -> Result<String, CatalogError> {
        let url = url.to_string();
        // The whole request is built AND dropped on a plain thread on purpose:
        // `refresh()` is called from async `main` (`--refresh-models`), and a
        // `reqwest::blocking::Client` owns a private tokio runtime — dropping it
        // inside an async context panics ("Cannot drop a runtime in a context
        // where blocking is not allowed"). The join keeps this synchronous.
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .build()
                .map_err(|e| CatalogError::Http(e.to_string()))?;
            let resp = client
                .get(&url)
                .send()
                .map_err(|e| CatalogError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(CatalogError::Http(format!("status {}", resp.status())));
            }
            resp.text().map_err(|e| CatalogError::Http(e.to_string()))
        })
        .join()
        .map_err(|_| CatalogError::Http("fetch thread panicked".into()))?
    }
}

static FETCHER: Mutex<Option<Arc<dyn Fetcher>>> = Mutex::new(None);

fn current_fetcher() -> Arc<dyn Fetcher> {
    FETCHER
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| Arc::new(DefaultFetcher))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// The live catalog. Never allocates beyond an `Arc` clone.
pub fn current() -> Arc<Catalog> {
    global().read().unwrap().clone()
}

/// Resolve a runtime `base_url` to a provider id using the live catalog.
/// Returns `None` when no provider matches (unknown endpoint).
///
/// Thin wrapper over [`provider_for_in`] against `current()`: the pure seam is
/// what tests (and any caller holding its own catalog) use, so no test has to
/// write the process-global catalog (B1).
pub fn provider_for(base_url: Option<&str>) -> Option<String> {
    provider_for_in(&current(), base_url)
}

/// [`provider_for`] against an explicit catalog — pure, no globals, no I/O.
pub fn provider_for_in(cat: &Catalog, base_url: Option<&str>) -> Option<String> {
    let key = normalize_url(base_url?);
    for (id, provider) in &cat.providers {
        if normalize_url(&provider.api) == key {
            return Some(id.clone());
        }
        for alias in &provider.aliases {
            if normalize_url(alias) == key {
                return Some(id.clone());
            }
        }
    }
    None
}

/// Normalize a URL to a comparison key (§4.3):
/// lowercase, trim trailing slash, strip a trailing `/v1`, then rewrite the
/// legacy `zen/go` spelling to models.dev's `inference/go/openai`.
pub fn normalize_url(url: &str) -> String {
    let mut s = url.trim().to_ascii_lowercase();
    while s.ends_with('/') {
        s.pop();
    }
    if let Some(stripped) = s.strip_suffix("/v1") {
        s = stripped.to_string();
    }
    s.replace("zen/go", "inference/go/openai")
}

/// Source URL for the catalog fetch. Honors `WCODE_MODELS_URL` so a new
/// provider/source can be pointed at without recompiling; defaults to
/// [`MODELS_DEV_URL`] (models.dev). This is the extension point for fetching
/// from a provider that is not on models.dev: implement [`Fetcher`] for that
/// source and call [`refresh_from`] (or [`refresh_with_url`]).
pub fn catalog_url() -> String {
    std::env::var("WCODE_MODELS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| MODELS_DEV_URL.to_string())
}

/// Blocking refresh from the configured catalog source (see [`catalog_url`]).
/// Used by `--refresh-models`. Single-flight with [`refresh_background`] via
/// `REFRESH_LOCK`.
pub fn refresh() -> Result<(), CatalogError> {
    refresh_with_url(catalog_url(), current_fetcher().as_ref())
}

/// Refresh from an explicit URL using a caller-supplied fetcher (the seam for
/// adding a new provider/source that is not models.dev).
pub fn refresh_from(url: &str, fetcher: &dyn Fetcher) -> Result<(), CatalogError> {
    refresh_with_url(url.to_string(), fetcher)
}

/// Refresh from an explicit URL into an explicit cache path with a
/// caller-supplied [`Fetcher`].
///
/// The seam [`refresh`] delegates to, and the one tests use with a
/// `tempfile::tempdir()` — no test writes `~/.local/share/wcode/models.dev.json`.
/// Fetch + parse happen OUTSIDE `REFRESH_LOCK` (reviewer A7); only the cache
/// write and the `Arc` swap are under it, so a reader always sees a whole
/// catalogue and a failed fetch/write leaves the previous one installed.
pub fn refresh_at(path: &Path, url: &str, fetcher: &dyn Fetcher) -> Result<(), CatalogError> {
    // Fetch + parse OUTSIDE the lock (reviewer A7).
    let body = fetcher.fetch(url)?;
    let mut cat = from_models_dev_json(&body)?;
    if cat.protocol.is_empty() {
        cat.protocol = current().protocol.clone();
    }
    cat.fetched_at = Some(now_unix());
    cat.source = CatalogSource::Network;
    let _guard = REFRESH_LOCK.lock().unwrap();
    // Record the source URL in the cache file for diagnostics.
    write_cache_with_url(path, &cat, url)?;
    *global().write().unwrap() = Arc::new(cat);
    Ok(())
}

/// Refresh from an explicit URL using a caller-supplied fetcher, into the cache
/// path from the environment (or the default).
pub fn refresh_with_url(url: String, fetcher: &dyn Fetcher) -> Result<(), CatalogError> {
    refresh_at(&cache_path_from_env_or_default(), &url, fetcher)
}

/// Refresh using a caller-supplied fetcher against the default URL (test seam).
pub fn refresh_with(fetcher: &dyn Fetcher) -> Result<(), CatalogError> {
    refresh_with_url(MODELS_DEV_URL.to_string(), fetcher)
}
pub fn refresh_background() {
    if REFRESHING.swap(true, Ordering::SeqCst) {
        return; // already in flight
    }
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => {
            REFRESHING.store(false, Ordering::SeqCst);
            return;
        }
    };
    // Reviewer A7: run the blocking HTTP + file I/O on a blocking thread so we
    // never stall the async runtime; take REFRESH_LOCK only around cache write
    // + the Arc swap (see refresh_with).
    handle.spawn_blocking(|| {
        let _ = refresh();
        REFRESHING.store(false, Ordering::SeqCst);
    });
}

/// Read the cache at the configured path into the global, synchronously, and
/// NEVER touch the network: no fetch, no staleness check, no background refresh.
///
/// This is the half a pure one-shot needs (`--dump-config`'s "why did it compact
/// so early?" line, `--dump-catalog`): reading the cache is not the same
/// requirement as refreshing it. When the cache is absent, torn or
/// schema-mismatched the global keeps the compiled-in [`seed`].
pub fn load() {
    let _ = load_into_global(&cache_path_from_env_or_default());
}

/// Boot wiring: [`load`] the cache, then kick a background refresh when it is
/// absent or older than the TTL. The cache/seed stays usable in the meantime
/// (stale-while-revalidate).
///
/// The ONLY catalog entry point that can reach the network, and only through
/// [`refresh_background`] — which is a no-op outside a tokio runtime and
/// single-flighted while one refresh is in flight.
pub fn init() {
    let cached = load_into_global(&cache_path_from_env_or_default());
    let stale = cached
        .as_ref()
        .map(|cat| is_stale(cat, ttl_secs(), now_unix()))
        .unwrap_or(true);
    if stale {
        refresh_background();
    }
}

/// Read `path` into the global and return what was installed. `None` = no
/// usable cache (the global then keeps whatever it had, i.e. the seed).
fn load_into_global(path: &Path) -> Option<Catalog> {
    let cat = load_cache(path)?;
    *global().write().unwrap() = Arc::new(cat.clone());
    Some(cat)
}

/// True when `cat` was fetched longer than `ttl` seconds before `now`, or never
/// (a seed has no `fetched_at`): the pure staleness rule [`init`] applies, so
/// the TTL boundary is testable without reading or writing the global catalog
/// (B1).
fn is_stale(cat: &Catalog, ttl: u64, now: i64) -> bool {
    cat.fetched_at
        .map(|t| now.saturating_sub(t) > ttl as i64)
        .unwrap_or(true)
}

/// Load a cache file. `None` on absence, IO error, parse error, or schema
/// mismatch — never panics.
pub fn load_cache(path: &Path) -> Option<Catalog> {
    let raw = std::fs::read_to_string(path).ok()?;
    let cache: CacheFile = serde_json::from_str(&raw).ok()?;
    if cache.schema != SCHEMA_VERSION {
        return None;
    }
    Some(Catalog {
        providers: cache.providers,
        protocol: cache.protocol,
        fetched_at: Some(cache.fetched_at),
        source: CatalogSource::Cache,
    })
}

/// Atomic write via temp file + rename. A concurrent reader sees either the
/// old file or the new one, never a torn write.
pub fn write_cache(path: &Path, catalog: &Catalog) -> Result<(), CatalogError> {
    write_cache_with_url(path, catalog, MODELS_DEV_URL)
}

/// Atomic write with an explicit source URL recorded in the cache file.
pub fn write_cache_with_url(
    path: &Path,
    catalog: &Catalog,
    source_url: &str,
) -> Result<(), CatalogError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cache = CacheFile {
        schema: SCHEMA_VERSION,
        fetched_at: catalog.fetched_at.unwrap_or_else(now_unix),
        source_url: source_url.to_string(),
        providers: catalog.providers.clone(),
        protocol: catalog.protocol.clone(),
    };
    let tmp = path.with_extension("tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(file, &cache)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Compiled-in snapshot: a **superset** of the two retired static tables (the
/// old 45-row limit and 32-row price literals) plus everything the live
/// models.dev `opencode-go` listing carried when it was last refreshed — 54
/// limit rows and 37 price rows today, of which 17 are ids the catalogue no
/// longer serves (`glm-5`, `grok-4.6`, `kimi-k2.5`, …).
///
/// Offline fallback and seed for a first run; moved here from `limits.rs` /
/// `pricing.rs` (W038). It is a superset on purpose: offline/first-run
/// behaviour must match the pre-W038 tables exactly, so an id the live payload
/// no longer lists is KEPT (see `scripts/refresh-seed.sh`, which merges by
/// default and needs `--prune` to delete). Where both name an id the live value
/// wins (`kimi-k3`'s `cache_write`).
pub fn seed() -> Catalog {
    use std::collections::BTreeMap;
    let mut models: BTreeMap<String, ModelEntry> = BTreeMap::new();
    for (id, context, output) in SEED_LIMITS {
        models.insert(
            (*id).to_string(),
            ModelEntry {
                context: *context,
                output: *output,
                price: None,
                tiers: Vec::new(),
            },
        );
    }
    for (id, input, output, cache_read, cache_write) in SEED_PRICES {
        let price = ModelPrice {
            input: *input,
            output: *output,
            cache_read: *cache_read,
            cache_write: *cache_write,
        };
        models
            .entry((*id).to_string())
            .and_modify(|e| {
                e.price = Some(price);
            })
            .or_insert(ModelEntry {
                context: 0,
                output: 0,
                price: Some(price),
                tiers: Vec::new(),
            });
    }
    let mut providers = BTreeMap::new();
    providers.insert(
        "opencode-go".to_string(),
        ProviderEntry {
            id: "opencode-go".to_string(),
            api: "https://opencode.ai/inference/go/openai/v1".to_string(),
            aliases: vec![
                "https://opencode.ai/zen/go".to_string(),
                "https://opencode.ai/inference/go/openai".to_string(),
            ],
            models,
        },
    );
    let mut protocol = BTreeMap::new();
    protocol.insert(
        "opencode-go".to_string(),
        SEED_RESPONSES_ONLY.iter().map(|s| (*s).to_string()).collect(),
    );
    Catalog {
        providers,
        protocol,
        fetched_at: None,
        source: CatalogSource::Seed,
    }
}

/// Default cache path, honoring `$HOME` (and `$WCODE_MODELS_CACHE` at call
/// sites via [`cache_path_from_env_or_default`]).
pub fn default_cache_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/share/wcode").join(DEFAULT_CACHE_FILE_NAME)
}

fn cache_path_from_env_or_default() -> PathBuf {
    std::env::var_os("WCODE_MODELS_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(default_cache_path)
}

/// Staleness TTL in seconds: `WCODE_MODELS_TTL_SECS`, else
/// [`DEFAULT_TTL_SECS`]. A junk value falls back rather than failing boot.
fn ttl_secs() -> u64 {
    ttl_secs_from(std::env::var("WCODE_MODELS_TTL_SECS").ok().as_deref())
}

/// [`ttl_secs`] without the env read — pure, so the env handling is testable
/// without mutating the process environment.
fn ttl_secs_from(raw: Option<&str>) -> u64 {
    raw.and_then(|s| s.trim().parse().ok())
        .unwrap_or(DEFAULT_TTL_SECS)
}

/// Parse a models.dev `/api.json` payload into a [`Catalog`]. Provider-generic:
/// stores every provider models.dev carries. Limits/prices are applied for ANY
/// provider that `provider_for` resolves (no opencode-go gate); `model_protocol`
/// keeps its own opencode-go gate for the responses-only snapshot.
pub fn from_models_dev_json(input: &str) -> Result<Catalog, CatalogError> {
    let raw: BTreeMap<String, RawProvider> =
        serde_json::from_str(input).map_err(CatalogError::Json)?;
    let mut providers = BTreeMap::new();
    for (id, rp) in raw {
        // Alias seeding for the URL mismatch (Q1): opencode-go is advertised
        // under inference/go/openai but wcode talks to zen/go.
        let aliases = match id.as_str() {
            "opencode-go" => vec![
                "https://opencode.ai/zen/go".to_string(),
                "https://opencode.ai/inference/go/openai".to_string(),
            ],
            "openai" => vec!["https://api.openai.com/v1".to_string()],
            "openrouter" => vec!["https://openrouter.ai/api/v1".to_string()],
            "groq" => vec!["https://api.groq.com/openai/v1".to_string()],
            "moonshot" => vec!["https://api.moonshot.cn/v1".to_string()],
            "mistral" => vec!["https://api.mistral.ai/v1".to_string()],
            _ => Vec::new(),
        };
        let mut models = BTreeMap::new();
        for (mid, rm) in rp.models {
            models.insert(
                mid,
                ModelEntry {
                    context: rm.limit.context,
                    output: rm.limit.output,
                    price: rm.cost.as_ref().map(RawCost::to_model_price),
                    tiers: rm
                        .cost
                        .as_ref()
                        .and_then(|c| c.tiers.as_ref())
                        .map(|ts| ts.iter().map(|t| t.clone().into_price_tier()).collect())
                        .unwrap_or_default(),
                },
            );
        }
        providers.insert(
            id.clone(),
            ProviderEntry {
                id: id.clone(),
                api: rp.api,
                aliases,
                models,
            },
        );
    }
    Ok(Catalog {
        providers,
        protocol: BTreeMap::new(),
        fetched_at: None,
        source: CatalogSource::Network,
    })
}

// ---------------------------------------------------------------------------
// Raw models.dev shapes (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawProvider {
    #[serde(default)]
    api: String,
    #[serde(default)]
    models: BTreeMap<String, RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    #[serde(default)]
    limit: RawLimit,
    #[serde(default)]
    cost: Option<RawCost>,
}

#[derive(Deserialize, Default)]
struct RawLimit {
    /// A genuinely absent `limit` block deserializes to `0` — which the rest of
    /// the kernel reads as UNKNOWN (see [`ModelEntry`]), never as a zero window.
    #[serde(default)]
    context: u64,
    #[serde(default)]
    output: u64,
}

#[derive(Deserialize)]
struct RawCost {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_read: f64,
    #[serde(default)]
    cache_write: f64,
    #[serde(default)]
    tiers: Option<Vec<RawTier>>,
    // context_over_200k is intentionally ignored (D028 base-rate only).
    #[serde(default)]
    #[allow(dead_code)]
    context_over_200k: Option<serde_json::Value>,
}

#[derive(Clone, Deserialize)]
struct RawTier {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_read: f64,
    #[serde(default)]
    cache_write: f64,
    #[serde(default)]
    tier: RawTierSpec,
}

impl RawTier {
    fn into_price_tier(self) -> PriceTier {
        PriceTier {
            size: self.tier.size,
            price: ModelPrice {
                input: (self.input * 1_000_000.0).round() as u64,
                output: (self.output * 1_000_000.0).round() as u64,
                cache_read: (self.cache_read * 1_000_000.0).round() as u64,
                cache_write: (self.cache_write * 1_000_000.0).round() as u64,
            },
        }
    }
}

#[derive(Deserialize, Clone, Default)]
struct RawTierSpec {
    #[serde(rename = "type")]
    #[serde(default)]
    #[allow(dead_code)]
    kind: String,
    #[serde(default)]
    size: u64,
}

impl RawCost {
    fn to_model_price(&self) -> ModelPrice {
        // Float -> micro-USD: USD/Mtok * 1_000_000, rounded to nearest int.
        // Max 6 decimals in the current catalog, so no precision loss (Q8).
        ModelPrice {
            input: (self.input * 1_000_000.0).round() as u64,
            output: (self.output * 1_000_000.0).round() as u64,
            cache_read: (self.cache_read * 1_000_000.0).round() as u64,
            cache_write: (self.cache_write * 1_000_000.0).round() as u64,
        }
    }
}

// ---------------------------------------------------------------------------
// Cache file shape (mirrors session_groups::Manifest)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheFile {
    schema: u32,
    fetched_at: i64,
    source_url: String,
    providers: BTreeMap<String, ProviderEntry>,
    protocol: BTreeMap<String, Vec<String>>,
}

// ---------------------------------------------------------------------------
// Test-only injection
// ---------------------------------------------------------------------------

// `set_fetcher` swaps the HTTP implementation behind `refresh()`. There is
// deliberately no catalog setter: a test that wrote the process-global catalog
// raced every parallel reader (`limits`/`pricing`/`actor`), so tests build
// their own `Catalog` and go through the `*_in` seams instead (B1).
pub fn set_fetcher(f: Arc<dyn Fetcher>) {
    *FETCHER.lock().unwrap() = Some(f);
}

// Fetcher is nameable outside tests so callers can build stubs (reviewer A5).
pub use self::Fetcher as FetcherTrait;

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------------------
// Seed data (compiled-in snapshot of the legacy static tables)
// ---------------------------------------------------------------------------

const SEED_LIMITS: &[(&str, u64, u64)] = &[
    ("beam-free", 1048576, 131072),
    ("cat-jam-free", 1048576, 131072),
    ("claude-haiku-5-5", 1000000, 128000),
    ("deepseek-v4-flash", 1000000, 384000),
    ("deepseek-v4-flash-vision-exp", 1000000, 384000),
    ("deepseek-v4-pro", 1000000, 384000),
    ("deepseek-v4.1-flash", 1000000, 384000),
    ("gemini-4-argon", 1048576, 65536),
    ("gemini-4-argon-contributor", 1048576, 65536),
    ("glm-5", 202752, 32768),
    ("glm-5.1", 202752, 32768),
    ("glm-5.2", 1000000, 131072),
    ("glm-5.3", 1000000, 131072),
    ("glm-5.3-flash", 1000000, 131072),
    ("gpt-5.6-luna", 1050000, 128000),
    ("gpt-6-luna", 1050000, 128000),
    ("grok-4.5", 500000, 500000),
    ("grok-4.6", 500000, 500000),
    ("grok-4.7", 500000, 500000),
    ("hy3", 256000, 128000),
    ("hy4-preview", 1024000, 64000),
    ("iceman-contributor-free", 262144, 131072),
    ("kimi-k2.5", 262144, 65536),
    ("kimi-k2.6", 262144, 65536),
    ("kimi-k2.7-code", 262144, 262144),
    ("kimi-k3", 1048576, 131072),
    ("kimi-k3.1-pro", 1048576, 131072),
    ("longcat-2.0", 1000000, 131072),
    ("longcat-2.5-preview-free", 1000000, 131072),
    ("mimo-v2-omni", 262144, 128000),
    ("mimo-v2-pro", 1048576, 128000),
    ("mimo-v2.5", 1000000, 128000),
    ("mimo-v2.5-pro", 1048576, 128000),
    ("mimo-v2.6-flash", 1048576, 131072),
    ("mimo-v2.6-pro", 1048576, 131072),
    ("minimax-m2.5", 204800, 65536),
    ("minimax-m2.7", 204800, 131072),
    ("minimax-m3", 1000000, 131072),
    ("minimax-m3.1-flash", 1000000, 512000),
    ("minimax-m3.1-flash-free", 1000000, 512000),
    ("muse-spark-1.2-contributor", 1048576, 131072),
    ("muse-spark-1.3-contributor", 1048576, 131072),
    ("omen-alpha", 500000, 128000),
    ("ox-alpha-free", 1000000, 131072),
    ("qwen3.5-plus", 262144, 65536),
    ("qwen3.6-plus", 1000000, 65536),
    ("qwen3.7-max", 1000000, 65536),
    ("qwen3.7-plus", 1000000, 65536),
    ("qwen3.8-flash", 1000000, 131072),
    ("qwen3.8-max", 1000000, 131072),
    ("space-bunny", 1048576, 524288),
    ("space-bunny-free", 1048576, 524288),
    ("step-5-preview", 1000000, 65536),
    ("step-5-preview-free", 1000000, 65536),
];

const SEED_PRICES: &[(&str, u64, u64, u64, u64)] = &[
    ("beam-free", 0, 0, 0, 0),
    ("cat-jam-free", 0, 0, 0, 0),
    ("claude-haiku-5-5", 100000, 500000, 10000, 125000),
    ("deepseek-v4-pro", 660000, 1980000, 22000, 0),
    ("deepseek-v4.1-flash", 150000, 600000, 3000, 0),
    ("gemini-4-argon", 2000000, 10000000, 100000, 0),
    ("gemini-4-argon-contributor", 4000000, 20000000, 200000, 0),
    ("glm-5.2", 1400000, 4400000, 260000, 0),
    ("glm-5.3", 1400000, 4400000, 260000, 0),
    ("glm-5.3-flash", 150000, 500000, 30000, 0),
    ("gpt-5.6-luna", 200000, 1200000, 20000, 250000),
    ("gpt-6-luna", 100000, 500000, 10000, 125000),
    ("grok-4.7", 2000000, 6000000, 500000, 0),
    ("hy3", 140000, 580000, 35000, 0),
    ("hy4-preview", 834000, 2501000, 42000, 0),
    ("iceman-contributor-free", 0, 0, 0, 0),
    ("kimi-k2.7-code", 950000, 4000000, 190000, 0),
    ("kimi-k3", 3000000, 15000000, 300000, 3000000),
    ("kimi-k3.1-pro", 2000000, 10000000, 200000, 2000000),
    ("longcat-2.0", 300000, 1200000, 6000, 0),
    ("longcat-2.5-preview-free", 0, 0, 0, 0),
    ("mimo-v2.5", 140000, 280000, 2800, 0),
    ("mimo-v2.5-pro", 435000, 870000, 3625, 0),
    ("mimo-v2.6-flash", 140000, 280000, 2800, 0),
    ("mimo-v2.6-pro", 435000, 870000, 3625, 0),
    ("minimax-m2.7", 300000, 1200000, 60000, 375000),
    ("minimax-m3", 300000, 1200000, 60000, 0),
    ("minimax-m3.1-flash", 150000, 600000, 15000, 0),
    ("minimax-m3.1-flash-free", 0, 0, 0, 0),
    ("muse-spark-1.2-contributor", 100000, 200000, 2000, 0),
    ("muse-spark-1.3-contributor", 100000, 200000, 2000, 0),
    ("qwen3.7-plus", 400000, 1600000, 40000, 500000),
    ("qwen3.8-flash", 150000, 470000, 16000, 200000),
    ("qwen3.8-max", 2000000, 6000000, 250000, 2500000),
    ("space-bunny", 150000, 600000, 30000, 0),
    ("step-5-preview", 1000000, 2700000, 100000, 0),
    ("step-5-preview-free", 0, 0, 0, 0),
];

const SEED_RESPONSES_ONLY: &[&str] = &[
    "grok-4.6",
    "grok-4.7",
    "gpt-5.6-luna",
    "gpt-6-luna",
    "muse-spark-1.2-contributor",
    "muse-spark-1.3-contributor",
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn provider_for_matrix() {
        // A LOCALLY built catalog: no test writes the process-global one (B1).
        let cat = seed();
        let zen = Some("https://opencode.ai/zen/go/v1");
        assert_eq!(provider_for_in(&cat, zen), Some("opencode-go".to_string()));
        let inf = Some("https://opencode.ai/inference/go/openai/v1");
        assert_eq!(provider_for_in(&cat, inf), Some("opencode-go".to_string()));
        // mixed case + trailing slash
        assert_eq!(
            provider_for_in(&cat, Some("HTTPS://OPENCODE.AI/ZEN/GO/V1/")),
            Some("opencode-go".to_string())
        );
        // non-go
        assert_eq!(provider_for_in(&cat, Some("https://api.openai.com/v1")), None);
        assert_eq!(provider_for_in(&cat, None), None);
    }

    #[test]
    fn normalize_url_alias() {
        assert_eq!(normalize_url("https://opencode.ai/zen/go/v1"), "https://opencode.ai/inference/go/openai");
        assert_eq!(normalize_url("https://opencode.ai/zen/go/v1/"), "https://opencode.ai/inference/go/openai");
        assert_eq!(normalize_url("https://opencode.ai/inference/go/openai/v1"), "https://opencode.ai/inference/go/openai");
    }

    #[test]
    fn load_cache_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        let cat = seed();
        write_cache(&path, &cat).unwrap();
        let loaded = load_cache(&path).unwrap();
        assert_eq!(loaded.providers.len(), cat.providers.len());
        assert_eq!(loaded.providers.get("opencode-go").unwrap().models.len(), cat.providers.get("opencode-go").unwrap().models.len());
    }

    #[test]
    fn load_cache_torn_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_cache(&path).is_none());
    }

    /// B3: the seed is a SUPERset of the two retired static tables. These 17 ids
    /// are ones the live models.dev `opencode-go` listing no longer serves; the
    /// seed keeps them so offline/first-run behaviour matches pre-W038 exactly
    /// (`grok-4.6` also has to stay: `SEED_RESPONSES_ONLY` names it, and the two
    /// lists must not disagree about whether a model exists).
    #[test]
    fn the_seed_keeps_the_ids_the_live_payload_dropped() {
        let cat = seed();
        let models = &cat.providers.get("opencode-go").unwrap().models;
        for id in [
            "deepseek-v4-flash",
            "deepseek-v4-flash-vision-exp",
            "glm-5",
            "glm-5.1",
            "grok-4.5",
            "grok-4.6",
            "kimi-k2.5",
            "kimi-k2.6",
            "mimo-v2-omni",
            "mimo-v2-pro",
            "minimax-m2.5",
            "omen-alpha",
            "ox-alpha-free",
            "qwen3.5-plus",
            "qwen3.6-plus",
            "qwen3.7-max",
            "space-bunny-free",
        ] {
            let entry = models.get(id).unwrap_or_else(|| panic!("`{id}` missing from the seed"));
            assert!(entry.context > 0, "`{id}` must keep a real window");
        }
        // The responses-only snapshot names `grok-4.6`, so the limit table must
        // know it too — that inconsistency is what the drop created.
        assert!(cat.protocol["opencode-go"].iter().any(|id| id == "grok-4.6"));
        assert!(models.contains_key("grok-4.6"));
    }

    /// A live value wins where both tables name an id: `kimi-k3`'s `cache_write`
    /// is 3_000_000 in the refreshed payload and 0 in the old static table.
    #[test]
    fn a_live_value_beats_the_retired_table() {
        let cat = seed();
        let price = cat.providers["opencode-go"].models["kimi-k3"].price.unwrap();
        assert_eq!(price.cache_write, 3_000_000);
    }

    /// B5: the SCHEMA path, specifically. The earlier version of this test wrote
    /// `{"schema":999,"providers":{},"protocol":{}}` — which fails to parse at
    /// all (`CacheFile` also requires `fetched_at` + `source_url`), so it exited
    /// through `serde_json::from_str(..).ok()?` and never reached the schema
    /// check it claimed to pin. Every field is now supplied, so the ONLY reason
    /// to reject this file is `schema != SCHEMA_VERSION`; the same file with the
    /// current schema loads, which pins that (a missing-field trap would fail
    /// both halves).
    #[test]
    fn load_cache_schema_mismatch_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        let file = |schema: u32| {
            format!(
                r#"{{"schema":{schema},"fetched_at":1700000000,"source_url":"https://example.invalid/api.json","providers":{{}},"protocol":{{}}}}"#
            )
        };
        std::fs::write(&path, file(SCHEMA_VERSION + 1)).unwrap();
        assert!(
            load_cache(&path).is_none(),
            "a newer schema is treated as absent"
        );
        std::fs::write(&path, file(SCHEMA_VERSION)).unwrap();
        assert!(
            load_cache(&path).is_some(),
            "the same file at the current schema loads — the rejection above was the schema"
        );
    }

    /// The positive refresh path the brief's P4 gate names: a stubbed fetch
    /// replaces the catalogue AND writes the cache. Everything is a tempdir — no
    /// test touches `~/.local/share/wcode/models.dev.json`.
    ///
    /// The stub payload is built FROM the seed (plus a marker id) on purpose:
    /// `compaction`'s precedence tests read `current()`, and this test swaps it,
    /// so the swapped value has to behave exactly like the seed.
    #[test]
    fn a_refresh_replaces_the_catalog_and_writes_the_cache() {
        let _gate = refresh_tests();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.dev.json");
        let payload = seed_payload(&[("w038-probe", 123_456)]);
        let before = current();
        assert!(
            !before.providers["opencode-go"]
                .models
                .contains_key("w038-probe")
        );

        refresh_at(&path, "https://example.invalid/api.json", &Stub(payload)).unwrap();

        let after = current();
        assert!(
            !Arc::ptr_eq(&before, &after),
            "a refresh installs a new Arc"
        );
        assert!(
            after.providers["opencode-go"]
                .models
                .contains_key("w038-probe"),
            "the fetched payload is live"
        );
        // The same catalogue visible to a reader answers through the pure seam.
        assert_eq!(
            crate::limits::model_limit_in(
                &after,
                Some("https://opencode.ai/zen/go/v1"),
                "w038-probe"
            )
            .map(|l| l.context),
            Some(123_456)
        );
        // ... and the cache file round-trips, with schema, timestamp and the
        // source URL recorded.
        let cached = load_cache(&path).expect("the refresh wrote the cache");
        assert!(cached.providers["opencode-go"].models.contains_key("w038-probe"));
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["schema"], SCHEMA_VERSION);
        assert_eq!(raw["source_url"], "https://example.invalid/api.json");
        assert!(raw["fetched_at"].as_i64().unwrap() > 0);
    }

    /// The other half of P4: a failing stub leaves the previous catalogue
    /// installed (same `Arc`) and writes NOTHING — not even a temp file. Both
    /// failure kinds are covered: the FETCH fails, and the fetch succeeds but
    /// the body does not parse.
    #[test]
    fn a_failed_refresh_leaves_the_previous_catalog_and_writes_nothing() {
        let _gate = refresh_tests();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.dev.json");
        let before = current();

        // (a) the fetch itself fails.
        let err = refresh_at(&path, "https://example.invalid/api.json", &FailingFetch)
            .expect_err("a refused fetch is an error");
        assert!(matches!(err, CatalogError::Http(_)), "got {err:?}");
        assert!(
            Arc::ptr_eq(&before, &current()),
            "a failed refresh leaves the previous catalog installed"
        );
        assert!(!path.exists(), "a failed refresh writes nothing");

        // (b) the fetch succeeds, the payload is unparseable.
        let err = refresh_at(&path, "https://example.invalid/api.json", &Stub("{not json".into()))
            .expect_err("an unparseable payload is an error");
        assert!(matches!(err, CatalogError::Json(_)), "got {err:?}");
        assert!(Arc::ptr_eq(&before, &current()));
        assert!(
            !dir.path().join("models.dev.tmp").exists(),
            "no torn temp file either"
        );
    }

    /// Serialises the two tests that DO write the global (via `refresh_at`):
    /// they are the only writers, so a `before` snapshot inside the gate is
    /// stable. Readers never take it — the swapped catalogue is seed-equivalent.
    fn refresh_tests() -> std::sync::MutexGuard<'static, ()> {
        static GATE: Mutex<()> = Mutex::new(());
        GATE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A `Fetcher` that returns a fixed body — the refresh tests' HTTP seam.
    struct Stub(String);

    impl Fetcher for Stub {
        fn fetch(&self, _url: &str) -> Result<String, CatalogError> {
            Ok(self.0.clone())
        }
    }

    /// A `Fetcher` that always fails, like a refused connect.
    struct FailingFetch;

    impl Fetcher for FailingFetch {
        fn fetch(&self, _url: &str) -> Result<String, CatalogError> {
            Err(CatalogError::Http("boom".into()))
        }
    }

    /// Render `seed()`'s `opencode-go` slice back into the models.dev payload
    /// shape, plus `extra` marker models — the input a refresh test feeds the
    /// stub, so the swapped catalogue is behaviourally the seed.
    fn seed_payload(extra: &[(&str, u64)]) -> String {
        let cat = seed();
        let mut models = serde_json::Map::new();
        for (id, entry) in &cat.providers["opencode-go"].models {
            let mut m = serde_json::Map::new();
            if entry.context > 0 {
                m.insert(
                    "limit".into(),
                    serde_json::json!({ "context": entry.context, "output": entry.output }),
                );
            }
            if let Some(p) = entry.price {
                m.insert(
                    "cost".into(),
                    serde_json::json!({
                        "input": p.input as f64 / 1e6,
                        "output": p.output as f64 / 1e6,
                        "cache_read": p.cache_read as f64 / 1e6,
                        "cache_write": p.cache_write as f64 / 1e6,
                    }),
                );
            }
            models.insert(id.clone(), serde_json::Value::Object(m));
        }
        for (id, context) in extra {
            models.insert(
                (*id).to_string(),
                serde_json::json!({ "limit": { "context": context, "output": 4096 } }),
            );
        }
        serde_json::json!({
            "opencode-go": {
                "id": "opencode-go",
                "api": "https://opencode.ai/inference/go/openai/v1",
                "models": models,
            }
        })
        .to_string()
    }

    /// The TTL boundary, on the pure rule: strictly-greater is stale, exactly
    /// the TTL is not (so a 24 h cache is refreshed on the 24 h + 1 s tick), and
    /// a catalogue that was never fetched (the seed) is always stale.
    #[test]
    fn is_stale_at_the_ttl_boundary() {
        let cat_at = |fetched_at: Option<i64>| Catalog {
            fetched_at,
            ..seed()
        };
        // Just under, exactly at, just over a 100 s TTL.
        assert!(!is_stale(&cat_at(Some(1_000)), 100, 1_099));
        assert!(
            !is_stale(&cat_at(Some(1_000)), 100, 1_100),
            "exactly at the TTL is still fresh"
        );
        assert!(is_stale(&cat_at(Some(1_000)), 100, 1_101));
        // Never fetched ⇒ stale, whatever the clock.
        assert!(is_stale(&cat_at(None), 100, 1_000));
        // A ttl of 0 makes any elapsed second stale, and the same second not.
        assert!(!is_stale(&cat_at(Some(1_000)), 0, 1_000));
        assert!(is_stale(&cat_at(Some(1_000)), 0, 1_001));
    }

    /// `WCODE_MODELS_TTL_SECS` wins over the default; a junk/blank value falls
    /// back instead of failing boot.
    #[test]
    fn ttl_secs_honours_the_env_var() {
        assert_eq!(ttl_secs_from(None), DEFAULT_TTL_SECS);
        assert_eq!(ttl_secs_from(Some("60")), 60);
        assert_eq!(ttl_secs_from(Some(" 120 ")), 120);
        assert_eq!(ttl_secs_from(Some("0")), 0);
        assert_eq!(ttl_secs_from(Some("soon")), DEFAULT_TTL_SECS);
        assert_eq!(ttl_secs_from(Some("")), DEFAULT_TTL_SECS);
    }

    #[test]
    fn float_to_micro_conversion_exact() {
        // Build a minimal models.dev JSON and check conversion.
        let json = r#"{
            "opencode-go": {
                "id": "opencode-go",
                "api": "https://opencode.ai/zen/go/v1",
                "models": {
                    "deepseek-v4.1-flash": {
                        "limit": {"context": 1000000, "output": 384000},
                        "cost": {"input": 0.15, "output": 0.6, "cache_read": 0.003}
                    },
                    "mimo-v2.6-pro": {
                        "limit": {"context": 1048576, "output": 131072},
                        "cost": {"input": 0.435, "output": 0.87, "cache_read": 0.003625}
                    }
                }
            }
        }"#;
        let cat = from_models_dev_json(json).unwrap();
        let p = cat.providers.get("opencode-go").unwrap();
        let d = p.models.get("deepseek-v4.1-flash").unwrap();
        assert_eq!(d.context, 1_000_000);
        let price = d.price.unwrap();
        assert_eq!(price.input, 150_000);
        assert_eq!(price.output, 600_000);
        assert_eq!(price.cache_read, 3_000);
        let m = p.models.get("mimo-v2.6-pro").unwrap();
        let price2 = m.price.unwrap();
        assert_eq!(price2.input, 435_000);
        assert_eq!(price2.output, 870_000);
        assert_eq!(price2.cache_read, 3_625);
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert_eq!(
            provider_for_in(&seed(), Some("https://unknown.example.com/v1")),
            None
        );
    }

    /// An edit is visible on the `Catalog` you own and on NOTHING else: the
    /// global is never written by a test (B1). Removing a model from a local
    /// value cannot be observed by `current()`, and a fresh `seed()` still has
    /// it.
    #[test]
    fn a_local_edit_is_invisible_to_the_global() {
        let mut mine = seed();
        mine.providers
            .get_mut("opencode-go")
            .unwrap()
            .models
            .remove("deepseek-v4.1-flash");
        // The local value lost the row ...
        assert!(!mine.providers["opencode-go"].models.contains_key("deepseek-v4.1-flash"));
        assert_eq!(
            crate::limits::model_limit_in(&mine, Some("https://opencode.ai/zen/go/v1"), "deepseek-v4.1-flash"),
            None
        );
        // ... the process-global one is untouched ...
        assert!(
            current().providers["opencode-go"].models.contains_key("deepseek-v4.1-flash"),
            "no test writes the global catalog"
        );
        // ... and so is a fresh seed.
        assert!(seed().providers["opencode-go"].models.contains_key("deepseek-v4.1-flash"));
    }

}