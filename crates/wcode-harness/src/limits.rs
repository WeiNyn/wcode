//! Per-model context/output limits for recognized providers, so the agent can
//! size context against the real window instead of guessing.
//!
//! The provider's own API does not expose a context window: the OpenCode Go
//! (zen) `/models` endpoint returns only `id/object/created/owned_by`, and rig
//! leaves `Model::context_length` empty for every OpenAI-compatible listing
//! (it is only populated for providers whose listing reports a window, e.g.
//! Groq/Moonshot/OpenRouter). The authoritative catalog is models.dev; wcode
//! syncs it at runtime into `crate::catalog` (W038) and keeps a compiled-in
//! seed (`catalog::seed`) for offline use.
//!
//! The *live* measure of how full the context is comes from the provider's own
//! response usage (`prompt_tokens` → `crate::message::Usage::input_tokens`),
//! not from here; these limits only say *how much* fits.

use crate::streamfn::LlmEndpoint;

/// Context window assumed when neither the model catalog nor config supplies
/// one: a conservative baseline so an unknown model still compacts in time.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 256_000;

/// Context window and output ceiling advertised for a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelLimit {
    /// Total context window (tokens) the model accepts.
    pub context: u64,
    /// Maximum output tokens the provider advertises for one reply.
    pub output: u64,
}

/// Cap on how many tokens are held back for the model's reply. The advertised
/// output ceiling can be huge (OpenCode's `deepseek-v4.1-flash` is 384k), so it
/// is capped to keep most of the window usable for input.
pub const DEFAULT_RESERVE_CAP: u64 = 32_768;

impl ModelLimit {
    /// Tokens reserved for the reply: `min(output, DEFAULT_RESERVE_CAP)`.
    pub fn reserve(&self) -> u64 {
        self.output.min(DEFAULT_RESERVE_CAP)
    }

    /// Input-token count at which the window is considered full: the context
    /// window minus the reply reserve.
    pub fn trigger(&self) -> u64 {
        self.context.saturating_sub(self.reserve())
    }

    /// True when a provider-reported `input_tokens` has reached the trigger —
    /// the next request risks running past the window.
    pub fn is_full(&self, input_tokens: u64) -> bool {
        input_tokens >= self.trigger()
    }
}

/// True for endpoints served by OpenCode's Go (zen) API.
pub fn is_opencode_go(base_url: Option<&str>) -> bool {
    base_url.is_some_and(|u| u.to_ascii_lowercase().contains("opencode.ai/zen/go"))
}

/// Limits for `model` served from `base_url`, when we recognize the provider.
/// `None` = unknown provider/model (the caller falls back to a default budget).
pub fn model_limit(base_url: Option<&str>, model: &str) -> Option<ModelLimit> {
    let provider = crate::catalog::provider_for(base_url)?;
    let cat = crate::catalog::current();
    let entry = cat.providers.get(&provider)?.models.get(model)?;
    Some(ModelLimit {
        context: entry.context,
        output: entry.output,
    })
}

/// The wire protocol `model` speaks on OpenCode Go, when we recognize it.
/// `None` = unknown / both-work / not served by `chat`-or-`responses`, or a
/// non-zen provider — the caller (`EndpointPref::resolve`) falls back to `Chat`.
///
/// Gated by `is_opencode_go` (note: `model_limit` is NOT gated since W038 —
/// it resolves any provider via the catalog). A known id behind a non-OpenCode
/// base is NOT assumed (mirrors `unknown_model_or_provider_is_none`).
/// Only the responses-only ids are enumerated; every other id is `None`
/// (both-works, or speaks a protocol wcode does not implement).
///
/// # Returns
/// `Some(LlmEndpoint::Responses)` for an id in the seed's responses-only list under a
/// zen base; `None` otherwise (never `Some(Chat)` — absence means "let the caller
/// default to chat").
///
/// Does NOT promise: it does not verify the id exists in the live catalog, and it is
/// a snapshot of the OpenCode Go listing (seeded from `SEED_RESPONSES_ONLY`).
pub fn model_protocol(base_url: Option<&str>, model: &str) -> Option<LlmEndpoint> {
    if !is_opencode_go(base_url) {
        return None;
    }
    let cat = crate::catalog::current();
    let ids = cat.protocol.get("opencode-go")?;
    if ids.iter().any(|id| id == model) {
        Some(LlmEndpoint::Responses)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction::CompactionPolicy;

    const ZEN: Option<&str> = Some("https://opencode.ai/zen/go/v1");

    #[test]
    fn recognizes_opencode_go_endpoint() {
        assert!(is_opencode_go(ZEN));
        assert!(is_opencode_go(Some("https://opencode.ai/zen/go/v1/")));
        assert!(is_opencode_go(Some("HTTPS://OPENCODE.AI/ZEN/GO/V1")));
        assert!(!is_opencode_go(Some("https://api.openai.com/v1")));
        assert!(!is_opencode_go(Some("https://opencode.ai/zen/v1")));
        assert!(!is_opencode_go(None));
    }

    #[test]
    fn looks_up_configured_model() {
        let l = model_limit(ZEN, "deepseek-v4.1-flash").expect("known opencode model");
        assert_eq!(l.context, 1_000_000);
        assert_eq!(l.output, 384_000);
    }

    #[test]
    fn unknown_model_or_provider_is_none() {
        assert_eq!(model_limit(ZEN, "no-such-model"), None);
        // A known OpenCode id behind a non-opencode endpoint is not assumed.
        assert_eq!(model_limit(Some("https://api.openai.com/v1"), "kimi-k3"), None);
        assert_eq!(model_limit(None, "kimi-k3"), None);
    }

    #[test]
    fn step_5_preview_free_is_a_1m_window() {
        // Verified against the models.dev `opencode-go` catalog
        // (api = https://opencode.ai/zen/go/v1). Missing here, the model fell
        // back to DEFAULT_CONTEXT_WINDOW and compacted at ~11% of its window.
        assert_eq!(
            model_limit(ZEN, "step-5-preview-free"),
            Some(ModelLimit {
                context: 1_000_000,
                output: 65_536,
            })
        );
    }

    #[test]
    fn catalog_window_beats_the_default_in_compaction() {
        // The trigger is the window less the reply reserve, so the catalog row
        // is what moves it from the 128k default's 111,616 to 983,616.
        let p = CompactionPolicy::default();
        let window = p.context_window(ZEN, "step-5-preview-free");
        assert_eq!(window, 1_000_000);
        assert!(
            !p.should_compact(window, 111_616),
            "the old 128k trigger must not fire on a 1M model"
        );
        assert!(
            p.should_compact(window, 983_616),
            "1M window - 16_384 reserved = 983,616"
        );
    }

    #[test]
    fn configured_window_still_wins_only_when_the_catalog_misses() {
        let p = CompactionPolicy {
            window: Some(200_000),
            ..Default::default()
        };
        // Catalog miss: the configured override is the window.
        assert_eq!(p.context_window(ZEN, "not-a-real-model"), 200_000);
        // Catalog hit: it wins, and the override is ignored.
        assert_eq!(p.context_window(ZEN, "step-5-preview-free"), 1_000_000);
    }

    #[test]
    fn reserve_is_capped_and_trigger_and_is_full_agree() {
        // Huge advertised output (384k) is capped to the reserve cap.
        let big = ModelLimit { context: 1_000_000, output: 384_000 };
        assert_eq!(big.reserve(), DEFAULT_RESERVE_CAP);
        assert_eq!(big.trigger(), 1_000_000 - DEFAULT_RESERVE_CAP);
        assert!(!big.is_full(big.trigger() - 1));
        assert!(big.is_full(big.trigger()));

        // Small advertised output is used verbatim, not the cap.
        let small = ModelLimit { context: 8_192, output: 4_096 };
        assert_eq!(small.reserve(), 4_096);
        assert_eq!(small.trigger(), 4_096);
        assert!(small.is_full(4_096));
    }

    #[test]
    fn model_protocol_matrix() {
        // Every responses-only id under a zen base ⇒ Responses.
        for id in [
            "grok-4.6",
            "grok-4.7",
            "gpt-5.6-luna",
            "gpt-6-luna",
            "muse-spark-1.2-contributor",
            "muse-spark-1.3-contributor",
        ] {
            assert_eq!(model_protocol(ZEN, id), Some(LlmEndpoint::Responses), "{id}");
        }
    }

    #[test]
    fn model_protocol_both_works_and_unknown_are_none() {
        // A both-works id (chat), an unknown id: no protocol fact.
        assert_eq!(model_protocol(ZEN, "deepseek-v4-flash"), None);
        assert_eq!(model_protocol(ZEN, "no-such-model"), None);
    }

    #[test]
    fn model_protocol_needs_a_zen_base() {
        // A known responses-only id behind a NON-zen base is NOT assumed
        // (mirrors `unknown_model_or_provider_is_none`).
        assert_eq!(
            model_protocol(Some("https://api.openai.com/v1"), "grok-4.7"),
            None
        );
        assert_eq!(model_protocol(None, "grok-4.7"), None);
    }
}
