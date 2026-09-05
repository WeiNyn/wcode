#![allow(dead_code)] // Task 10 wires config + tools into the CLI loop

mod config;
mod tools;

fn main() {
    println!("wcode");
}

#[cfg(test)]
mod tests {
    use crate::config::{EnvLike, FileConfig, merge};

    fn env(wcode_base_url: Option<&str>, wcode_api_key: Option<&str>, openai_api_key: Option<&str>) -> EnvLike {
        EnvLike {
            wcode_base_url: wcode_base_url.map(str::to_string),
            wcode_api_key: wcode_api_key.map(str::to_string),
            openai_api_key: openai_api_key.map(str::to_string),
        }
    }

    fn file(base_url: Option<&str>, api_key: Option<&str>, model: Option<&str>) -> FileConfig {
        FileConfig {
            base_url: base_url.map(str::to_string),
            api_key: api_key.map(str::to_string),
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn env_beats_toml() {
        let cfg = merge(
            env(Some("http://env:1"), Some("env-key"), None),
            file(Some("http://toml:1"), Some("toml-key"), Some("m")),
        )
        .unwrap();
        assert_eq!(cfg.base_url.as_deref(), Some("http://env:1"));
        assert_eq!(cfg.api_key.as_deref(), Some("env-key"));
        assert_eq!(cfg.model, "m");
    }

    #[test]
    fn openai_api_key_falls_back_before_toml() {
        let cfg = merge(
            env(None, None, Some("openai-key")),
            file(None, Some("toml-key"), Some("m")),
        )
        .unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("openai-key"));
    }

    #[test]
    fn toml_used_when_env_absent() {
        let cfg = merge(
            env(Some("http://env:1"), None, None),
            file(Some("http://toml:1"), Some("toml-key"), Some("m")),
        )
        .unwrap();
        assert_eq!(cfg.base_url.as_deref(), Some("http://env:1"));
        assert_eq!(cfg.api_key.as_deref(), Some("toml-key"));
    }

    #[test]
    fn model_is_required() {
        let err = merge(env(None, None, None), file(None, None, None)).unwrap_err();
        assert!(err.contains("model"), "error must point at model: {err}");
    }

    #[test]
    fn file_config_parses_toml() {
        let fc: FileConfig = toml::from_str(
            "base_url = \"http://x\"\napi_key = \"k\"\nmodel = \"m\"\n",
        )
        .unwrap();
        assert_eq!(fc.model.as_deref(), Some("m"));
        let cfg = merge(env(None, None, None), fc).unwrap();
        let opts = cfg.to_llm_opts();
        assert_eq!(opts.model, "m");
        assert_eq!(opts.base_url.as_deref(), Some("http://x"));
        assert_eq!(opts.api_key.as_deref(), Some("k"));
        assert_eq!(opts.temperature, None);
    }
}
