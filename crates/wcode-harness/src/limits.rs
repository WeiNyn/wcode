//! Per-model context/output limits for recognized providers, so the agent can
//! size context against the real window instead of guessing.
//!
//! The provider's own API does not expose a context window: the OpenCode Go
//! (zen) `/models` endpoint returns only `id/object/created/owned_by`, and rig
//! leaves `Model::context_length` empty for every OpenAI-compatible listing
//! (it is only populated for providers whose listing reports a window, e.g.
//! Groq/Moonshot/OpenRouter). The authoritative catalog is models.dev, whose
//! `opencode-go` entry (`api = https://opencode.ai/zen/go/v1`) carries
//! `limit.context` / `limit.output` per model. [`OPENCODE_GO`] is a snapshot of
//! that table.
//!
//! The *live* measure of how full the context is comes from the provider's own
//! response usage (`prompt_tokens` → `crate::message::Usage::input_tokens`),
//! not from here; these limits only say *how much* fits.

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
    if is_opencode_go(base_url) {
        return opencode_go(model);
    }
    None
}

fn opencode_go(model: &str) -> Option<ModelLimit> {
    OPENCODE_GO
        .iter()
        .find(|(id, ..)| *id == model)
        .map(|(_, context, output)| ModelLimit {
            context: *context,
            output: *output,
        })
}

/// OpenCode Go (zen) model table, snapshotted from the models.dev `opencode-go`
/// catalog: `(model id, context window, max output tokens)`.
static OPENCODE_GO: &[(&str, u64, u64)] = &[
    ("deepseek-v4-flash", 1_000_000, 384_000),
    ("deepseek-v4-flash-vision-exp", 1_000_000, 384_000),
    ("deepseek-v4-pro", 1_000_000, 384_000),
    ("deepseek-v4.1-flash", 1_000_000, 384_000),
    ("glm-5", 202_752, 32_768),
    ("glm-5.1", 202_752, 32_768),
    ("glm-5.2", 1_000_000, 131_072),
    ("glm-5.3", 1_000_000, 131_072),
    ("glm-5.3-flash", 1_000_000, 131_072),
    ("gpt-5.6-luna", 1_050_000, 128_000),
    ("grok-4.5", 500_000, 500_000),
    ("grok-4.6", 500_000, 500_000),
    ("hy3", 256_000, 128_000),
    ("hy4-preview", 1_024_000, 64_000),
    ("kimi-k2.5", 262_144, 65_536),
    ("kimi-k2.6", 262_144, 65_536),
    ("kimi-k2.7-code", 262_144, 262_144),
    ("kimi-k3", 1_048_576, 131_072),
    ("longcat-2.0", 1_000_000, 131_072),
    ("mimo-v2-omni", 262_144, 128_000),
    ("mimo-v2-pro", 1_048_576, 128_000),
    ("mimo-v2.5", 1_000_000, 128_000),
    ("mimo-v2.5-pro", 1_048_576, 128_000),
    ("minimax-m2.5", 204_800, 65_536),
    ("minimax-m2.7", 204_800, 131_072),
    ("minimax-m3", 1_000_000, 131_072),
    ("muse-spark-1.2-contributor", 1_048_576, 131_072),
    ("muse-spark-1.3-contributor", 1_048_576, 131_072),
    ("omen-alpha", 500_000, 128_000),
    ("ox-alpha-free", 1_000_000, 131_072),
    ("qwen3.5-plus", 262_144, 65_536),
    ("qwen3.6-plus", 1_000_000, 65_536),
    ("qwen3.7-max", 1_000_000, 65_536),
    ("qwen3.7-plus", 1_000_000, 65_536),
    ("qwen3.8-flash", 1_000_000, 131_072),
    ("qwen3.8-max", 1_000_000, 131_072),
];

#[cfg(test)]
mod tests {
    use super::*;

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
}
