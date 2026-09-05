use std::path::PathBuf;

use serde::Deserialize;
use wcode_harness::streamfn::LlmOpts;

#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: String,
}

/// Snapshot of the relevant environment variables, so merging is testable.
#[derive(Debug, Default)]
pub struct EnvLike {
    pub wcode_base_url: Option<String>,
    pub wcode_api_key: Option<String>,
    pub openai_api_key: Option<String>,
}

impl EnvLike {
    pub fn from_env() -> Self {
        Self {
            wcode_base_url: std::env::var("WCODE_BASE_URL").ok(),
            wcode_api_key: std::env::var("WCODE_API_KEY").ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
        }
    }
}

/// Precedence: env (WCODE_* with OPENAI_API_KEY fallback) > toml. Model is
/// required and comes from the toml only; error tells main what to prompt for.
pub fn merge(env: EnvLike, file: FileConfig) -> Result<Config, String> {
    let model = file
        .model
        .ok_or("no model configured: set `model = \"...\"` in ~/.config/wcode/config.toml")?;
    Ok(Config {
        base_url: env.wcode_base_url.or(file.base_url),
        api_key: env
            .wcode_api_key
            .or(env.openai_api_key)
            .or(file.api_key),
        model,
    })
}

impl Config {
    pub fn load() -> Result<Config, String> {
        let file = match Self::default_path().map(|p| std::fs::read_to_string(p)) {
            Some(Ok(text)) => Some(
                toml::from_str::<FileConfig>(&text)
                    .map_err(|e| format!("config parse error: {e}"))?,
            ),
            _ => None,
        };
        merge(EnvLike::from_env(), file.unwrap_or_default())
    }

    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("wcode").join("config.toml"))
    }

    pub fn to_llm_opts(&self) -> LlmOpts {
        LlmOpts {
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            temperature: None,
        }
    }
}
