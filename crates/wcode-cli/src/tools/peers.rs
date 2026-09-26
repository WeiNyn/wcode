//! The `peers` tool: list the peers this session can message (§13.15).

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::agents::Phonebook;
use wcode_protocol::Registry;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct PeersArgs {}

/// Lists the phonebook — the peers reachable by name (§13.15).
pub struct Peers {
    phonebook: Phonebook,
    registry: Registry,
}

impl Peers {
    pub fn new(phonebook: Phonebook, registry: Registry) -> Self {
        Self {
            phonebook,
            registry,
        }
    }
}

#[async_trait::async_trait]
impl TypedTool for Peers {
    type Args = PeersArgs;

    fn name(&self) -> &str {
        "peers"
    }

    fn description(&self) -> &str {
        "List the peers you can message, as `name -> address [state]`."
    }

    fn parallel_safe(&self) -> bool {
        true
    }

    async fn execute(&self, _args: PeersArgs, _ctx: &ToolContext) -> ToolOutput {
        let entries = self.phonebook.entries();
        if entries.is_empty() {
            return ToolOutput {
                output: "(no peers yet — `spawn` a worker, or register one with `--peer`)"
                    .to_string(),
                ..ToolOutput::default()
            };
        }
        let listing = entries
            .iter()
            .map(|(name, address)| {
                format!(
                    "{name} -> {address} [{}]",
                    self.registry.state_of(address).label()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        ToolOutput {
            output: listing,
            ..ToolOutput::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wcode_harness::protocol::{MemberState, SessionId};

    fn ctx() -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "p1".into(),
            name: "peers".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
            session_path: None,
        }
    }

    #[tokio::test]
    async fn listing_names_each_members_state() {
        let phonebook = Phonebook::default();
        let address = SessionId::agent("w1");
        phonebook.insert("reviewer", address.clone());
        let registry = Registry::new();
        registry.set_state(address, MemberState::Running);

        let out = Peers::new(phonebook, registry)
            .execute(PeersArgs {}, &ctx())
            .await;
        assert!(
            out.output.contains("reviewer -> agent:w1 [running]"),
            "{}",
            out.output
        );
    }
}
