//! The `peers` tool: list the peers this session can message (§13.15).

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::agents::Phonebook;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct PeersArgs {}

/// Lists the phonebook — the peers reachable by name (§13.15).
pub struct Peers {
    phonebook: Phonebook,
}

impl Peers {
    pub fn new(phonebook: Phonebook) -> Self {
        Self { phonebook }
    }
}

#[async_trait::async_trait]
impl TypedTool for Peers {
    type Args = PeersArgs;

    fn name(&self) -> &str {
        "peers"
    }

    fn description(&self) -> &str {
        "List the peers you can message, as `name -> address`."
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
            .map(|(name, address)| format!("{name} -> {address}"))
            .collect::<Vec<_>>()
            .join("\n");
        ToolOutput {
            output: listing,
            ..ToolOutput::default()
        }
    }
}
