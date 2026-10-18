//! Per-model prices for recognized providers, so `/usage` can report a `$` cost
//! instead of a bare token count.
//!
//! Twin of [`crate::limits`], from the same source and the same gate: the
//! authoritative catalog is models.dev, whose `opencode-go` entry
//! (`api = https://opencode.ai/zen/go/v1`) carries
//! `cost { input, output, cache_read, cache_write }` in **USD per million
//! tokens**. [`OPENCODE_GO`] is a snapshot of that table, and [`model_price`] is
//! gated by the existing [`crate::limits::is_opencode_go`] — so the two tables
//! can never disagree about which provider they describe.
//!
//! Units are **micro-USD per Mtok**, integers, never floats: a price stays
//! `Eq`/serde-stable, a cost is byte-identical across a round-trip, and the
//! arithmetic is one integer division away from the dollars `summary()` prints.
//!
//! Who calls it: the actor's `Request::Usage` reply (see
//! [`crate::protocol::Request::Usage`]) folds the transcript with
//! [`crate::stats::session_stats`] and then prices it with [`cost_of`]; the
//! front-ends only render the [`Cost`] it produced.
//!
//! NOT in v1 (named, deliberate — brief Q5):
//! - `tiers` / `context_over_200k` — a request past the tier size bills up to
//!   ~5× the base rate, so a tiered model's number is an **undercount**;
//!   [`OPENCODE_GO`] stores the base rate only.
//! - a per-model `base_url` from [`crate::streamfn::LlmOpts::model_profiles`] —
//!   pricing uses the session's launch base_url for every turn.
//! - a config file. A const table mirrors `limits.rs`; behaviour is not
//!   configured (`AGENTS.md` §Conventions).

use crate::message::{AgentMessage, Usage};
use crate::stats::Cost;

/// Micro-USD per Mtok: the divisor that turns `tokens × rate` into micro-USD.
/// (`tokens / 1_000_000` Mtok × a micro-USD/Mtok rate = micro-USD.)
const MICRO_PER_MTOK: u128 = 1_000_000;

/// One model's prices: USD per million tokens, in **micro-USD** (1e-6 USD).
///
/// Integers so a price and a derived [`Cost`] stay `Eq` and serde-stable, and
/// so `summary()`'s `$` is reproducible to the last digit. `Default` is the
/// all-zero price — a *known free* model, which is `Some(ModelPrice::default())`
/// and never `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelPrice {
    /// Price of one million **uncached** prompt tokens.
    pub input: u64,
    /// Price of one million completion tokens.
    pub output: u64,
    /// Price of one million prompt tokens served from the provider's cache.
    ///
    /// A cached read is a **SUBSET of `input_tokens`** on the
    /// chat-completions wire (rig maps `usage.prompt_tokens_details
    /// .cached_tokens`, i.e. tokens already inside `prompt_tokens`), so a cached
    /// token is billed at THIS rate *instead of* `input` — never on top of it.
    pub cache_read: u64,
    /// Price of one million tokens **written** to the cache. `0` for most
    /// providers (models.dev omits the field when the provider does not report
    /// it); it is not part of `input`'s accounting.
    pub cache_write: u64,
}

impl ModelPrice {
    /// One turn's cost numerator: `Σ tokens × rate`, i.e. micro-USD × 1e6.
    ///
    /// Contract:
    /// - `cache_read_tokens` is billed at [`ModelPrice::cache_read`], the
    ///   remainder of `input_tokens` at [`ModelPrice::input`] (the subset rule).
    ///   `cache_read_tokens` is clamped to `input_tokens` so a provider that
    ///   over-reports cannot underflow the remainder.
    /// - `cache_write_tokens` is billed at [`ModelPrice::cache_write`] on top of
    ///   that; a turn reporting no cache field pays the full `input` rate.
    /// - the value is the un-truncated numerator so a caller folding many turns
    ///   sums numerators and divides ONCE ([`cost_of`]); one turn's answer is
    ///   [`ModelPrice::micro_usd`].
    ///
    /// Does NOT promise: tier rates, or that the provider reported usage at all
    /// (a `Usage::default()` turn costs 0 — the caller decides whether a turn
    /// counted).
    pub fn cost_numerator(&self, usage: &Usage) -> u128 {
        // Cached read is a SUBSET of `input_tokens`: bill it at the cache rate
        // INSTEAD of the input rate, outside the cache rate never twice. Clamp
        // so a provider that over-reports cannot underflow the remainder.
        let cached = usage.cache_read_tokens.unwrap_or(0).min(usage.input_tokens);
        let uncached = usage.input_tokens - cached;
        let written = usage.cache_write_tokens.unwrap_or(0);
        uncached as u128 * self.input as u128
            + cached as u128 * self.cache_read as u128
            + written as u128 * self.cache_write as u128
            + usage.output_tokens as u128 * self.output as u128
    }

    /// One turn's cost in whole micro-USD, truncated.
    ///
    /// `micro_usd(u) == (cost_numerator(u) / 1_000_000) as u64` — a sub-micro
    /// fraction is dropped, never rounded, so the printed `$` agrees with the
    /// truncated micro-USD the brief's §7.2 check computes.
    pub fn micro_usd(&self, usage: &Usage) -> u64 {
        (self.cost_numerator(usage) / MICRO_PER_MTOK) as u64
    }
}

/// Prices for `model` served from `base_url`, when we recognize the provider.
///
/// `None` = unknown provider/model — the caller OMITS the `$` (an unknown price
/// is not `$0`; a known-free model is `Some(ModelPrice::default())`).
///
/// Gated by [`crate::limits::is_opencode_go`] EXACTLY like
/// [`crate::limits::model_limit`] (`limits.rs:63` `pub fn model_limit(base_url:
/// Option<&str>, model: &str) -> Option<ModelLimit> {`): a known id behind a
/// non-zen base is NOT assumed — mirrors `limits::unknown_model_or_provider_is_none`.
///
/// Does NOT promise: id coverage. [`OPENCODE_GO`] holds the 32 ids models.dev
/// prices; `limits::OPENCODE_GO` holds 45, so a model can have a context window
/// and NO price (see the table doc).
pub fn model_price(base_url: Option<&str>, model: &str) -> Option<ModelPrice> {
    if crate::limits::is_opencode_go(base_url) {
        return opencode_go(model);
    }
    None
}

/// The session's `$` cost: fold every `Assistant { usage: Some(u), model }` at
/// the price of `model` — the message's own id when set, else
/// `fallback_model` — under `base_url`, summing micro-USD.
///
/// Returns `None` when NOT ONE turn could be priced (unknown model/provider ⇒
/// absent, never `$0`). `Cost::unpriced_turns` counts the turns that reported
/// usage but had no price, so a mid-run model switch reads
/// `~$0.0087 (3 turns unpriced)` instead of silently under-reporting.
///
/// Contract:
/// - one numerator is accumulated across priced turns and divided ONCE, so a
///   long session does not lose a micro-USD per turn to truncation;
/// - a turn with `usage: None` is neither priced nor counted unpriced (it
///   reported nothing);
/// - `messages` is the same slice [`crate::stats::session_stats`] folded, so
///   `stats.turns == priced + unpriced` whenever `cost` is `Some`.
///
/// Does NOT promise: tier rates; a per-model `base_url` (a
/// [`crate::streamfn::ModelProfiles`] override is ignored — the launch base is
/// used for every turn); compaction does not rewrite the messages priced here.
pub fn cost_of(
    messages: &[AgentMessage],
    base_url: Option<&str>,
    fallback_model: &str,
) -> Option<Cost> {
    let mut numerator: u128 = 0;
    let mut unpriced_turns: u64 = 0;
    let mut priced = false;
    for m in messages {
        let AgentMessage::Assistant { usage, model, .. } = m else {
            continue;
        };
        // A turn that reported nothing is neither priced nor counted unpriced.
        let Some(u) = usage else {
            continue;
        };
        let id = model.as_deref().unwrap_or(fallback_model);
        match model_price(base_url, id) {
            Some(price) => {
                numerator += price.cost_numerator(u);
                priced = true;
            }
            None => unpriced_turns += 1,
        }
    }
    // Not one turn could be priced ⇒ absence, never `$0`.
    if !priced {
        return None;
    }
    // Divide ONCE so a long session does not lose a micro-USD per turn.
    Some(Cost {
        micro_usd: (numerator / MICRO_PER_MTOK) as u64,
        unpriced_turns,
    })
}

/// Table lookup, mirroring `limits::opencode_go` (`limits.rs:93`
/// `fn opencode_go(model: &str) -> Option<ModelLimit> {`): first row whose id
/// matches exactly. No prefix/normalization — an id is an id.
fn opencode_go(model: &str) -> Option<ModelPrice> {
    OPENCODE_GO
        .iter()
        .find(|(id, ..)| *id == model)
        .map(|(_, input, output, cache_read, cache_write)| ModelPrice {
            input: *input,
            output: *output,
            cache_read: *cache_read,
            cache_write: *cache_write,
        })
}

/// OpenCode Go (zen) price table, snapshotted from the models.dev `opencode-go`
/// catalog (`https://models.dev/api.json` → `opencode-go` → `models.<id>.cost`,
/// 32 models): `(model id, input, output, cache_read, cache_write)` in
/// **micro-USD per Mtok** (USD/Mtok × 1_000_000 — `0.15` ⇒ `150_000`,
/// `0.003` ⇒ `3_000`).
///
/// Source notes, per the `limits.rs` discipline of citing where the numbers come
/// from:
/// - every value converts EXACTLY to micro-USD for this snapshot (checked: no
///   entry carries more than 6 decimals), so no rounding happens on the way in;
/// - `cache_write` is `0` where models.dev omits it (the provider does not
///   report a separate write price);
/// - four ids are priced at all-zero = genuinely free: `beam-free`,
///   `cat-jam-free`, `longcat-2.5-preview-free`, `step-5-preview-free`;
/// - `tiers` / `context_over_200k` are DROPPED: the base rate is stored, so a
///   tiered model (`grok-4.7`, `gpt-5.6-luna`, `gpt-6-luna`, `minimax-m3`,
///   `claude-haiku-5-5`, `qwen3.7-plus`) is an undercount above its threshold.
///
/// Coverage gap (named, not a bug to hide): 17 ids in `limits::OPENCODE_GO` have
/// no row here — `deepseek-v4-flash`, `deepseek-v4-flash-vision-exp`, `glm-5`,
/// `glm-5.1`, `grok-4.5`, `grok-4.6`, `kimi-k2.5`, `kimi-k2.6`, `mimo-v2-omni`,
/// `mimo-v2-pro`, `minimax-m2.5`, `omen-alpha`, `ox-alpha-free`,
/// `qwen3.5-plus`, `qwen3.6-plus`, `qwen3.7-max`, `space-bunny-free`. They get
/// a context window but NO `$`. Conversely `beam-free`, `cat-jam-free`,
/// `gemini-4-argon-contributor`, `step-5-preview` are priced here and absent
/// from the limits table — harmless: the two tables answer different questions.
static OPENCODE_GO: &[(&str, u64, u64, u64, u64)] = &[
    ("beam-free", 0, 0, 0, 0),
    ("cat-jam-free", 0, 0, 0, 0),
    ("claude-haiku-5-5", 100_000, 500_000, 10_000, 125_000),
    ("deepseek-v4-pro", 660_000, 1_980_000, 22_000, 0),
    ("deepseek-v4.1-flash", 150_000, 600_000, 3_000, 0),
    ("gemini-4-argon-contributor", 4_000_000, 20_000_000, 200_000, 0),
    ("glm-5.2", 1_400_000, 4_400_000, 260_000, 0),
    ("glm-5.3", 1_400_000, 4_400_000, 260_000, 0),
    ("glm-5.3-flash", 150_000, 500_000, 30_000, 0),
    ("gpt-5.6-luna", 200_000, 1_200_000, 20_000, 250_000),
    ("gpt-6-luna", 100_000, 500_000, 10_000, 125_000),
    ("grok-4.7", 2_000_000, 6_000_000, 500_000, 0),
    ("hy3", 140_000, 580_000, 35_000, 0),
    ("hy4-preview", 834_000, 2_501_000, 42_000, 0),
    ("kimi-k2.7-code", 950_000, 4_000_000, 190_000, 0),
    ("kimi-k3", 3_000_000, 15_000_000, 300_000, 0),
    ("longcat-2.0", 300_000, 1_200_000, 6_000, 0),
    ("longcat-2.5-preview-free", 0, 0, 0, 0),
    ("mimo-v2.5", 140_000, 280_000, 2_800, 0),
    ("mimo-v2.5-pro", 435_000, 870_000, 3_625, 0),
    ("mimo-v2.6-flash", 140_000, 280_000, 2_800, 0),
    ("mimo-v2.6-pro", 435_000, 870_000, 3_625, 0),
    ("minimax-m2.7", 300_000, 1_200_000, 60_000, 375_000),
    ("minimax-m3", 300_000, 1_200_000, 60_000, 0),
    ("muse-spark-1.2-contributor", 100_000, 200_000, 2_000, 0),
    ("muse-spark-1.3-contributor", 100_000, 200_000, 2_000, 0),
    ("qwen3.7-plus", 400_000, 1_600_000, 40_000, 500_000),
    ("qwen3.8-flash", 150_000, 470_000, 16_000, 200_000),
    ("qwen3.8-max", 2_000_000, 6_000_000, 250_000, 2_500_000),
    ("space-bunny", 150_000, 600_000, 30_000, 0),
    ("step-5-preview", 1_000_000, 2_700_000, 100_000, 0),
    ("step-5-preview-free", 0, 0, 0, 0),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, StopReason};

    const ZEN: Option<&str> = Some("https://opencode.ai/zen/go/v1");

    fn usage(input: u64, output: u64, read: Option<u64>, write: Option<u64>) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: read,
            cache_write_tokens: write,
        }
    }

    fn assistant(u: Option<Usage>, model: Option<&str>) -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::Text { text: "a".into() }],
            stop_reason: StopReason::Stop,
            usage: u,
            model: model.map(|m| m.to_string()),
        }
    }

    /// models.dev `opencode-go` → `deepseek-v4.1-flash`:
    /// `{"input":0.15,"output":0.6,"cache_read":0.003}` (no `cache_write`) ⇒
    /// `150_000 / 600_000 / 3_000 / 0` micro-USD per Mtok.
    #[test]
    fn deepseek_v4_1_flash_prices_match_models_dev() {
        assert_eq!(
            model_price(ZEN, "deepseek-v4.1-flash"),
            Some(ModelPrice {
                input: 150_000,
                output: 600_000,
                cache_read: 3_000,
                cache_write: 0,
            })
        );
    }

    /// A genuinely free model is `Some`, not `None`: `Some(ModelPrice::default())`
    /// ⇒ `~$0.0000` on screen, while `None` prints nothing. Pick a priced-zero id
    /// (`step-5-preview-free`); `ox-alpha-free` looks free but has NO row, so it
    /// is the None case in the test below.
    #[test]
    fn a_free_model_is_some_zero_not_none() {
        assert_eq!(
            model_price(ZEN, "step-5-preview-free"),
            Some(ModelPrice::default())
        );
    }

    /// The `limits::unknown_model_or_provider_is_none` matrix, for prices:
    /// an unknown id under a zen base, a known id behind a non-zen base, and
    /// `None` as the base all collapse to `None` (never `$0`).
    #[test]
    fn unknown_model_or_provider_is_none() {
        // Unknown id behind a zen base.
        assert_eq!(model_price(ZEN, "no-such-model"), None);
        // A known id behind a non-zen base is NOT assumed.
        assert_eq!(
            model_price(Some("https://api.openai.com/v1"), "deepseek-v4.1-flash"),
            None
        );
        // No base at all.
        assert_eq!(model_price(None, "deepseek-v4.1-flash"), None);
        // `ox-alpha-free` looks free but has NO row in the pricing snapshot.
        assert_eq!(model_price(ZEN, "ox-alpha-free"), None);
    }

    /// The subset rule, in exact micro-USD. For `deepseek-v4.1-flash` with
    /// `input = 1_000`, `cache_read = 400`, `output = 500`:
    /// `(1_000−400)·150_000 + 400·3_000 + 500·600_000 = 391_200_000`, ÷1e6 ⇒
    /// **391 µUSD**. Also: no cache field ⇒ the full input rate
    /// (`(1_000·150_000)/1e6 = 150`), i.e. cached tokens are never charged twice.
    #[test]
    fn cost_charges_the_cached_subset_at_the_cache_rate() {
        let p = model_price(ZEN, "deepseek-v4.1-flash").expect("a priced model");
        let u = usage(1_000, 500, Some(400), None);
        // (1000-400)*150_000 + 400*3_000 + 500*600_000 = 391_200_000, /1e6 = 391.
        assert_eq!(p.cost_numerator(&u), 391_200_000);
        assert_eq!(p.micro_usd(&u), 391);
        // No cache field ⇒ the FULL input rate: (1000*150_000)/1e6 = 150.
        // (If cache_read were charged at the input rate WITHOUT subtracting it
        // from the uncached remainder, this would read 190 — the failure the
        // test guards against.)
        assert_eq!(p.micro_usd(&usage(1_000, 0, None, None)), 150);
        // A cached read is not charged twice: the same turn with cache_read ==
        // input costs the cache rate for all of it.
        assert_eq!(p.micro_usd(&usage(1_000, 0, Some(1_000), None)), 3);
    }

    /// ADDITIVE (sketch recommendation — the brief's §7.1 list stops at four, but
    /// `cost_of` is the fn the actor calls and would otherwise be untested):
    /// two priced turns + one unpriced model ⇒ `Some(Cost { micro_usd, Some(1) })`;
    /// a session with no priced turn ⇒ `None`.
    #[test]
    fn cost_of_counts_the_turns_it_could_not_price() {
        // Two priced turns (deepseek-v4.1-flash) + one unpriced model id.
        let messages = vec![
            // 1_000_000 * 150_000 / 1e6 = 150_000 µUSD.
            assistant(Some(usage(1_000_000, 0, None, None)), Some("deepseek-v4.1-flash")),
            // No per-message model ⇒ the fallback id is used → 150_000 µUSD.
            assistant(Some(usage(1_000_000, 0, None, None)), None),
            // An unknown id: reported usage, no price ⇒ one unpriced turn.
            assistant(Some(usage(1_000, 1_000, None, None)), Some("no-such-model")),
        ];
        let cost = cost_of(&messages, ZEN, "deepseek-v4.1-flash").expect("some priced turn");
        assert_eq!(cost.micro_usd, 300_000);
        assert_eq!(cost.unpriced_turns, 1);

        // Not one priced turn ⇒ None, never $0.
        let unpriced = vec![assistant(Some(usage(10, 10, None, None)), Some("no-such-model"))];
        assert_eq!(cost_of(&unpriced, ZEN, "no-such-model"), None);
        // A session that reported no usage is None too.
        assert_eq!(cost_of(&[], ZEN, "deepseek-v4.1-flash"), None);
    }
}
