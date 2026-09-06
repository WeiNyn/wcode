//! Optional `rtk` (Rust Token Killer) integration for the bash tool.
//!
//! When enabled, every bash tool call is passed through `rtk rewrite` first
//! (rtk's single source of truth for filters). If rtk returns a rewrite the
//! command executes as `rtk ...`, so the model sees compressed output and
//! burns fewer tokens; otherwise the original command runs untouched.
//!
//! rtk stays an optional external binary: any failure — binary missing,
//! rewrite timeout, non-rewrite exit — degrades to the current raw behavior.

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use wcode_harness::hooks::{Hooks, ToolCall};

/// How aggressively to use rtk. `Auto` = use it whenever the binary is
/// reachable (spawn + rewrite + fall back on any error), which is
/// behaviourally equivalent to "on if `rtk` is on PATH".
///
/// Serialized as a string (`"auto"` / `"true"` / `"false"`) so config can
/// round-trip; deserialization also accepts TOML booleans (`rtk = true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RtkPreference {
    #[default]
    Auto,
    On,
    Off,
}

impl RtkPreference {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim) {
            None | Some("") | Some("auto") => Ok(RtkPreference::Auto),
            Some("on" | "true" | "1") => Ok(RtkPreference::On),
            Some("off" | "false" | "0") => Ok(RtkPreference::Off),
            Some(other) => Err(format!(
                "unknown rtk {other:?}: valid values are \"auto\", \"true\", \"false\""
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RtkPreference::Auto => "auto",
            RtkPreference::On => "true",
            RtkPreference::Off => "false",
        }
    }
}

impl std::fmt::Display for RtkPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for RtkPreference {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RtkPreference {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct RtkVisitor;
        impl Visitor<'_> for RtkVisitor {
            type Value = RtkPreference;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "\"auto\", a boolean, or \"true\"/\"false\"")
            }
            fn visit_bool<E: serde::de::Error>(self, b: bool) -> Result<Self::Value, E> {
                Ok(if b { RtkPreference::On } else { RtkPreference::Off })
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Self::Value, E> {
                RtkPreference::parse(Some(s)).map_err(E::custom)
            }
        }
        d.deserialize_any(RtkVisitor)
    }
}

fn rtk_enabled(pref: RtkPreference) -> bool {
    pref != RtkPreference::Off
}

/// Corridor check for `rtk rewrite` (the pi extension uses the same contract):
/// - 0: rewrite found
/// - 1: no rtk equivalent (`rtk rewrite` also hints "run rtk init" on stderr)
/// - 3: advisory rewrite
///
/// Anything else, a killed process, or a spawn failure = pass through, no rewrite.
const REWRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub struct RtkHooks {
    enabled: bool,
}

impl RtkHooks {
    pub fn new(pref: RtkPreference) -> Self {
        Self {
            enabled: rtk_enabled(pref),
        }
    }
}

#[async_trait::async_trait]
impl Hooks for RtkHooks {
    async fn transform_tool_input(&self, call: &mut ToolCall) {
        if !self.enabled || call.name != "bash" {
            return;
        }
        let Some(command) = call.arguments.get("command").and_then(|c| c.as_str()) else {
            return;
        };
        let Some(rewritten) = rewrite(command).await else {
            return;
        };
        if let Some(obj) = call.arguments.as_object_mut() {
            obj.insert("command".to_string(), serde_json::json!(rewritten));
        }
    }
}

/// Ask rtk whether it can filter `command`. Returns the rewritten command
/// (e.g. `rtk git status`) or `None` for pass-through.
async fn rewrite(command: &str) -> Option<String> {
    let mut child = tokio::process::Command::new("rtk")
        .arg("rewrite")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let stdout = tokio::select! {
        biased;
        _ = tokio::time::sleep(REWRITE_TIMEOUT) => {
            let _ = child.kill().await;
            return None;
        }
        status = child.wait() => {
            match status {
                Ok(s) if matches!(s.code(), Some(0 | 3)) => {
                    let mut out = String::new();
                    use tokio::io::AsyncReadExt as _;
                    // Kill the pipe; a truly stuck child still fits the budget.
                    if let Some(mut p) = child.stdout.take() {
                        let _ = p.read_to_string(&mut out).await;
                    }
                    out
                }
                _ => return None,
            }
        }
    };
    let rewritten = stdout.trim();
    if rewritten.is_empty() {
        None
    } else {
        Some(rewritten.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_defaults_to_auto() {
        assert_eq!(RtkPreference::parse(None).unwrap(), RtkPreference::Auto);
        assert_eq!(
            RtkPreference::parse(Some("auto")).unwrap(),
            RtkPreference::Auto
        );
    }

    #[test]
    fn preference_parses_on_off_with_synonyms() {
        assert_eq!(RtkPreference::parse(Some("true")).unwrap(), RtkPreference::On);
        assert_eq!(RtkPreference::parse(Some("on")).unwrap(), RtkPreference::On);
        assert_eq!(RtkPreference::parse(Some("1")).unwrap(), RtkPreference::On);
        assert_eq!(
            RtkPreference::parse(Some("false")).unwrap(),
            RtkPreference::Off
        );
        assert_eq!(
            RtkPreference::parse(Some("off")).unwrap(),
            RtkPreference::Off
        );
        assert_eq!(RtkPreference::parse(Some("0")).unwrap(), RtkPreference::Off);
    }

    #[test]
    fn preference_rejects_unknown_value() {
        let err = RtkPreference::parse(Some("sometimes")).unwrap_err();
        assert!(err.contains("sometimes"), "names the value: {err}");
    }

    #[test]
    fn preference_round_trips_through_serde() {
        for pref in [RtkPreference::Auto, RtkPreference::On, RtkPreference::Off] {
            let json = serde_json::to_string(&pref).unwrap();
            let back: RtkPreference = serde_json::from_str(&json).unwrap();
            assert_eq!(pref, back, "round-trip of {pref} via {json}");
        }
        assert_eq!(serde_json::to_string(&RtkPreference::Auto).unwrap(), "\"auto\"");
    }

    #[test]
    fn preference_deserializes_boolean() {
        let on: RtkPreference = serde_json::from_str("true").unwrap();
        assert_eq!(on, RtkPreference::On);
        let off: RtkPreference = serde_json::from_str("false").unwrap();
        assert_eq!(off, RtkPreference::Off);
    }

    #[test]
    fn preference_deserializes_string_forms() {
        for raw in ["\"auto\"", "\"true\"", "\"on\"", "\"0\""] {
            let pref: RtkPreference = serde_json::from_str(raw).unwrap();
            assert!(pref != RtkPreference::Off || raw == "\"0\"");
        }
        assert_eq!(
            serde_json::from_str::<RtkPreference>("\"auto\"").unwrap(),
            RtkPreference::Auto
        );
    }

    #[test]
    fn preference_deserialization_rejects_garbage() {
        assert!(serde_json::from_str::<RtkPreference>("\"sometimes\"").is_err());
    }

    fn bash_call(command: &str) -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": command }),
        }
    }

    #[tokio::test]
    async fn transform_off_leaves_command_untouched() {
        let hooks = RtkHooks::new(RtkPreference::Off);
        let mut call = bash_call("git status");
        hooks.transform_tool_input(&mut call).await;
        assert_eq!(call.arguments["command"], "git status");
    }

    #[tokio::test]
    async fn transform_skips_non_bash_tools() {
        let hooks = RtkHooks::new(RtkPreference::On);
        let mut call = ToolCall {
            id: "1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": "Cargo.toml" }),
        };
        hooks.transform_tool_input(&mut call).await;
        assert_eq!(call.arguments["path"], "Cargo.toml");
    }

    #[tokio::test]
    async fn transform_rewrites_when_rtk_present() {
        // Integration-ish check: depends on a real `rtk` on PATH; skipped in
        // environments without it, asserted where it exists.
        if std::process::Command::new("rtk")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let hooks = RtkHooks::new(RtkPreference::On);
        let mut call = bash_call("git status");
        hooks.transform_tool_input(&mut call).await;
        assert!(
            call.arguments["command"]
                .as_str()
                .is_some_and(|c| c.starts_with("rtk ")),
            "git status should rewrite to rtk: {}",
            call.arguments["command"]
        );
    }

    #[tokio::test]
    async fn transform_passes_unknown_commands_through() {
        if std::process::Command::new("rtk")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let hooks = RtkHooks::new(RtkPreference::On);
        let mut call = bash_call("wcode-rtk-does-not-know-this 123");
        hooks.transform_tool_input(&mut call).await;
        assert_eq!(call.arguments["command"], "wcode-rtk-does-not-know-this 123");
    }
}