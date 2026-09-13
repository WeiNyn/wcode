//! The `message` tool: send to another session (§10.1).

use serde::Deserialize;

use wcode_harness::protocol::{Request, SessionId};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};
use wcode_protocol::Registry;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MessageArgs {
    /// The recipient's address (`agent:w1`). Omit it to message your
    /// orchestrator — a worker has exactly one place to send.
    #[serde(default)]
    to: Option<String>,
    /// What to send.
    content: String,
    /// Delivery mode: `wake` (**default** — run a turn even if the recipient is
    /// idle, so it processes your message), `notify` (append, no turn),
    /// `interrupt` (soft interrupt at the next turn), or `ask` (like `wake`, but
    /// you expect a report back).
    #[serde(default)]
    mode: Option<String>,
}

/// Sends a message to a peer through the [`Registry`] (which enforces the
/// permitted set, §10.1).
pub struct Message {
    registry: Registry,
    me: SessionId,
    /// The default recipient for a worker (its orchestrator); `None` for the
    /// orchestrator itself, which must always name `to`.
    owner: Option<SessionId>,
}

impl Message {
    pub fn new(registry: Registry, me: SessionId, owner: Option<SessionId>) -> Self {
        Self {
            registry,
            me,
            owner,
        }
    }
}

/// A bare name is a worker address (`w1` → `agent:w1`); anything with a
/// `:`-prefix (`agent:…`, `client:…`) is taken verbatim.
fn address(to: &str) -> SessionId {
    if to.contains(':') {
        SessionId::new(to)
    } else {
        SessionId::agent(to)
    }
}

/// Map a tool `mode` onto a target-side request verb. `ask` rides `wake` (the
/// report comes back asynchronously as an inbound message, §10.1).
fn request(mode: Option<&str>, content: String) -> Result<Request, String> {
    match mode.unwrap_or("wake") {
        "notify" => Ok(Request::Notify { content }),
        "wake" | "ask" => Ok(Request::Wake { content }),
        "interrupt" => Ok(Request::Interrupt { content }),
        other => Err(format!(
            "unknown mode `{other}` (expected notify | wake | interrupt | ask)"
        )),
    }
}

#[async_trait::async_trait]
impl TypedTool for Message {
    type Args = MessageArgs;

    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to another agent session. `to` is the recipient's \
         address; omit it to message your orchestrator. `mode` is `wake` \
         (default; the recipient processes it even if idle), `notify`, \
         `interrupt`, or `ask`."
    }

    async fn execute(&self, args: MessageArgs, _ctx: &ToolContext) -> ToolOutput {
        let recipient = args
            .to
            .as_deref()
            .or(self.owner.as_ref().map(SessionId::as_str));
        let Some(to) = recipient.map(address) else {
            return ToolOutput {
                output: "no recipient: give `to` (there is no orchestrator to default to)"
                    .to_string(),
                is_error: true,
                ..ToolOutput::default()
            };
        };

        let request = match request(args.mode.as_deref(), args.content) {
            Ok(request) => request,
            Err(message) => {
                return ToolOutput {
                    output: message,
                    is_error: true,
                    ..ToolOutput::default()
                };
            }
        };

        match self.registry.deliver(&self.me, &to, request) {
            Ok(()) => ToolOutput {
                output: format!("sent to {to}"),
                ..ToolOutput::default()
            },
            Err(e) => ToolOutput {
                output: format!("could not send to {to}: {e}"),
                is_error: true,
                ..ToolOutput::default()
            },
        }
    }
}
