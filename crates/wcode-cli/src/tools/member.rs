//! The `member` tool: inspect or stop a teammate mid-run (§10.1).
//!
//! The kernel answers `Status`/`SideAsk`/`Usage` from a turn-boundary snapshot
//! while a run is in flight, and services `Cancel` immediately — so every op
//! works while the member is thinking or inside a tool call, without waiting for
//! its run to end. `Status`/`Usage` are lean reads, not the whole transcript.

use std::time::{Duration, Instant};

use serde::Deserialize;

use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{MemberState, Request, SessionId};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};
use wcode_protocol::{AskError, Registry};

use crate::agents::{Phonebook, short_name};
use crate::session_groups::{self, SessionGroup};
use crate::tasks::TaskList;
use crate::tools::message::address;

/// Budget for `status` — a lean `Request::Status` read, answered from the
/// member's latest turn snapshot even while it runs. Instant in practice.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
/// Budget for `ask` — the member's own model call.
const ASK_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `cancel` waits for the member's run to actually stop.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);
/// Poll interval while waiting for a cancel to take effect.
const CANCEL_POLL: Duration = Duration::from_millis(100);
/// Budget for `compact` — a peer's own summarization call on its model. Longer
/// than `ASK_TIMEOUT`: the reply is `Compaction` or `Ack` (`NothingToDo`); a
/// *Running* peer defers a `Compact` to its next turn boundary, so this bounds
/// the wait after that run ends. Three minutes — a summarization call on a
/// large context is slow, and a premature timeout only reports "busy".
const COMPACT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Default, Deserialize, schemars::JsonSchema)]
pub struct MemberArgs {
    /// The member's name or address (`w1` or `agent:w1`).
    to: String,
    /// `status` — the member's last completed work, no model call; `ask` — a
    /// tool-free question answered by the member's model from its current
    /// context (needs `text`); `cancel` — stop its in-flight run; `model`/
    /// `effort` — retune it; `usage` — its own tokens, cache hit rate, tool
    /// errors and `$` cost, no model call; `compact` — summarize its older
    /// history now; `retire` — drop it from the roster (and mark it retired).
    op: String,
    /// The question for `op = "ask"`.
    #[serde(default)]
    text: Option<String>,
    /// For `op = "model"`: the new model id → `Request::SetModel { model }`.
    #[serde(default)]
    model: Option<String>,
    /// For `op = "effort"`: the new reasoning effort → `Request::SetEffort`.
    /// `"-"`/`"none"`/`"off"` clears it (`effort: None`); any other value sets
    /// it. Mirrors `WorkerSpec::effort`.
    #[serde(default)]
    effort: Option<String>,
    /// For `op = "compact"`: optional focus instructions for the summary →
    /// `Request::Compact { instructions }`.
    #[serde(default)]
    instructions: Option<String>,
    /// For `op = "retire"`: retire a member that a live task node names
    /// (`Task.owner`) — default false = refuse it (the DAG guard).
    #[serde(default)]
    force: bool,
}

/// Inspects or stops a peer without waiting for its run to end.
pub struct Member {
    registry: Registry,
    me: SessionId,
    phonebook: Phonebook,
    /// The root's shared plan — the retire DAG guard reads `Task.owner`. Passed
    /// as `self.tasks.clone()` from `Orchestrator::tools`.
    tasks: TaskList,
    /// The active session group, so `retire` sets the durable `retired` flag
    /// ([`session_groups::retire_member`]). `None` for a group-less session (a
    /// bare root) → retire removes live only.
    group: Option<SessionGroup>,
}

impl Member {
    pub fn new(
        registry: Registry,
        me: SessionId,
        phonebook: Phonebook,
        tasks: TaskList,
        group: Option<SessionGroup>,
    ) -> Self {
        Self {
            registry,
            me,
            phonebook,
            tasks,
            group,
        }
    }

    /// Resolve `to`: a bare name through the phonebook, else `agent:<name>`.
    fn target(&self, to: &str) -> SessionId {
        self.phonebook.get(to).unwrap_or_else(|| address(to))
    }

    async fn status(&self, to: &SessionId) -> ToolOutput {
        let state = self.registry.state_of(to).label();
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        match backend.ask_within(Request::Status, STATUS_TIMEOUT).await {
            Ok(AgentEvent::Status { last_assistant_text }) => {
                let work = last_assistant_text
                    .unwrap_or_else(|| "(no completed work yet)".to_string());
                ToolOutput {
                    output: format!("[{state}] {to} · {work}"),
                    ..ToolOutput::default()
                }
            }
            Ok(other) => ToolOutput {
                output: format!("[{state}] {to} · unexpected reply: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!("[{state}] {to} · no reply within {STATUS_TIMEOUT:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    /// `op = "usage"`: the member's own usage report — `Request::Usage` (WS-C)
    /// → `AgentEvent::Usage { stats }`, rendered by `stats.summary()`.
    ///
    /// Uses [`STATUS_TIMEOUT`], not a new constant: `Request::Usage` is answered
    /// from the turn-boundary snapshot exactly like `Request::Status` (the
    /// actor's mid-run arm), so it is an instant read and a *Running* member
    /// answers at once. It is NOT a model call.
    ///
    /// Output: `"[{state}] {to} · {summary}"`, mirroring [`Self::status`]'s
    /// `"[{state}] {to} · {work}"` — the state label is what tells the caller
    /// whether the numbers are final. The summary is the kernel's one shared
    /// wording (`turns/in/out`, `% cache`, `N tool errors`, `~$0.0012`), so this
    /// tool invents no formatting and a priced member shows its `$` for free.
    async fn usage(&self, to: &SessionId) -> ToolOutput {
        let state = self.registry.state_of(to).label();
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        match backend.ask_within(Request::Usage, STATUS_TIMEOUT).await {
            Ok(AgentEvent::Usage { stats }) => ToolOutput {
                output: format!("[{state}] {to} · {}", stats.summary()),
                ..ToolOutput::default()
            },
            // An older peer catches `usage` as `Request::Unknown` and answers
            // `Ack`: "unavailable", never "zero usage".
            Ok(AgentEvent::Ack) => ToolOutput {
                output: format!(
                    "[{state}] {to} · usage unavailable (peer predates Request::Usage)"
                ),
                ..ToolOutput::default()
            },
            Ok(AgentEvent::Error { message }) => error(format!("[{state}] {to} · {message}")),
            Ok(other) => ToolOutput {
                output: format!("[{state}] {to} · unexpected reply: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!("[{state}] {to} · no reply within {STATUS_TIMEOUT:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    async fn ask(&self, to: &SessionId, text: &str) -> ToolOutput {
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        let request = Request::SideAsk {
            text: text.to_string(),
        };
        match backend.ask_within(request, ASK_TIMEOUT).await {
            Ok(AgentEvent::SideAnswer { text, .. }) => ToolOutput {
                output: text,
                ..ToolOutput::default()
            },
            Ok(other) => ToolOutput {
                output: format!("{to} replied unexpectedly: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!("{to} is busy: no answer within {ASK_TIMEOUT:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    async fn cancel(&self, to: &SessionId) -> ToolOutput {
        if let Err(e) = self.registry.resolve(&self.me, to) {
            return error(format!("cannot reach {to}: {e}"));
        }
        let state = self.registry.state_of(to);
        if state != MemberState::Running {
            return ToolOutput {
                output: format!("{to} is not running ({})", state.label()),
                ..ToolOutput::default()
            };
        }
        if let Err(e) = self.registry.deliver(&self.me, to, Request::Cancel) {
            return error(format!("could not cancel {to}: {e}"));
        }
        // Wait for the run to actually end rather than reporting a blind send.
        let deadline = Instant::now() + CANCEL_TIMEOUT;
        loop {
            let now = self.registry.state_of(to);
            if now != MemberState::Running {
                return ToolOutput {
                    output: format!("cancelled {to} (now {})", now.label()),
                    ..ToolOutput::default()
                };
            }
            if Instant::now() >= deadline {
                return ToolOutput {
                    output: format!("cancel sent to {to}; still running after {CANCEL_TIMEOUT:?}"),
                    ..ToolOutput::default()
                };
            }
            tokio::time::sleep(CANCEL_POLL).await;
        }
    }

    /// Cancel a running member, best-effort and silent — the retire path's first
    /// step. A member that is unreachable or already idle is a no-op; a running
    /// one is cancelled and awaited up to [`CANCEL_TIMEOUT`] (the `cancel` poll
    /// loop, without the report).
    async fn cancel_if_running(&self, to: &SessionId) {
        if self.registry.resolve(&self.me, to).is_err() {
            return;
        }
        if self.registry.state_of(to) != MemberState::Running {
            return;
        }
        let _ = self.registry.deliver(&self.me, to, Request::Cancel);
        let deadline = Instant::now() + CANCEL_TIMEOUT;
        while self.registry.state_of(to) == MemberState::Running && Instant::now() < deadline {
            tokio::time::sleep(CANCEL_POLL).await;
        }
    }

    /// `op = "model"`: `Request::SetModel { model }` on the peer → `Ack`. After
    /// the `Ack`, mirror into the roster with [`Registry::set_model`] — the socket
    /// server already does this for remote peers, so the local path must match or
    /// `peers`/the sidebar would lie.
    async fn set_model(&self, to: &SessionId, model: &str) -> ToolOutput {
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        let request = Request::SetModel {
            model: model.to_string(),
        };
        match backend.ask_within(request, STATUS_TIMEOUT).await {
            Ok(AgentEvent::Ack) => {
                self.registry.set_model(to.clone(), model);
                ToolOutput {
                    output: format!("{to} now runs model `{model}`"),
                    ..ToolOutput::default()
                }
            }
            Ok(AgentEvent::Error { message }) => {
                error(format!("could not set {to}'s model: {message}"))
            }
            Ok(other) => ToolOutput {
                output: format!("{to} replied unexpectedly: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!("{to} is busy: no reply within {STATUS_TIMEOUT:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    /// `op = "effort"`: `Request::SetEffort { effort }` on the peer → `Ack`.
    /// `"-"`/`"none"`/`"off"` → `None` (clears); any other value → `Some(level)`.
    /// Mirror with [`Registry::set_effort`] after the `Ack`, as `set_model` does.
    async fn set_effort(&self, to: &SessionId, effort: &str) -> ToolOutput {
        let value = match effort {
            "-" | "none" | "off" => None,
            level => Some(level.to_string()),
        };
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        let request = Request::SetEffort {
            effort: value.clone(),
        };
        match backend.ask_within(request, STATUS_TIMEOUT).await {
            Ok(AgentEvent::Ack) => {
                self.registry.set_effort(to.clone(), value.clone());
                let output = match value {
                    Some(level) => format!("{to} effort set to `{level}`"),
                    None => format!("{to} effort cleared"),
                };
                ToolOutput {
                    output,
                    ..ToolOutput::default()
                }
            }
            Ok(AgentEvent::Error { message }) => {
                error(format!("could not set {to}'s effort: {message}"))
            }
            Ok(other) => ToolOutput {
                output: format!("{to} replied unexpectedly: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!("{to} is busy: no reply within {STATUS_TIMEOUT:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    /// `op = "compact"`: `Request::Compact { instructions }` on the peer.
    /// Surfaces `AgentEvent::Compaction { summarized, kept }`; `Ack` = nothing to
    /// compact (`CompactOutcome::NothingToDo`). A **Running** peer defers a
    /// `Compact` to its next turn boundary, so this waits up to
    /// [`COMPACT_TIMEOUT`]; on timeout it returns a "busy — its run must end
    /// first" message, not a blind send.
    async fn compact(&self, to: &SessionId, instructions: Option<&str>) -> ToolOutput {
        let backend = match self.registry.resolve(&self.me, to) {
            Ok(backend) => backend,
            Err(e) => return error(format!("cannot reach {to}: {e}")),
        };
        let request = Request::Compact {
            instructions: instructions.map(str::to_string),
        };
        match backend.ask_within(request, COMPACT_TIMEOUT).await {
            Ok(AgentEvent::Compaction { summarized, kept }) => ToolOutput {
                output: format!("{to}: summarized {summarized} messages, kept {kept}"),
                ..ToolOutput::default()
            },
            Ok(AgentEvent::Ack) => ToolOutput {
                output: format!("{to}: nothing to compact"),
                ..ToolOutput::default()
            },
            Ok(AgentEvent::Error { message }) => error(format!("could not compact {to}: {message}")),
            Ok(other) => ToolOutput {
                output: format!("{to} replied unexpectedly: {other:?}"),
                ..ToolOutput::default()
            },
            Err(AskError::Timeout) => ToolOutput {
                output: format!(
                    "{to} is busy: its run must end before it can compact (no reply within {COMPACT_TIMEOUT:?})"
                ),
                ..ToolOutput::default()
            },
            Err(AskError::Closed) => error(format!("{to} has shut down")),
        }
    }

    /// `op = "retire"`: the durable removal, in order:
    ///   1. cancel-if-running (the [`Self::cancel_if_running`] poll loop);
    ///   2. [`Registry::remove`] — drops `peers`, so `contains`→false and a
    ///      `subscribe()`r (the sidebar) drops the row;
    ///   3. [`Phonebook::remove`] — the name stops resolving;
    ///   4. [`session_groups::retire_member`] — sets the manifest `retired` flag
    ///      so `--resume` does NOT respawn it (the `.jsonl` stays on disk as an
    ///      audit trail). A no-op when `group` is `None`.
    ///
    /// DAG guard first: refuse when a live task names the member
    /// (`Task.owner`) unless `force`. The refusal names the task id/title.
    async fn retire(&self, to: &SessionId, force: bool) -> ToolOutput {
        if !force
            && let Some(task) = self
                .tasks
                .snapshot()
                .into_iter()
                .find(|t| t.owner.as_ref() == Some(to))
        {
            return error(format!(
                "{to} is assigned task {} (`{}`); wait for it to finish or pass `force: true`",
                task.id, task.title
            ));
        }
        self.cancel_if_running(to).await;
        self.registry.remove(to);
        let name = short_name(to);
        self.phonebook.remove(&name);
        if let Some(group) = &self.group
            && let Err(e) = session_groups::retire_member(group, &name)
        {
            return error(format!("retired {to} live, but could not record the retirement: {e}"));
        }
        ToolOutput {
            output: format!("retired {to}"),
            ..ToolOutput::default()
        }
    }
}

fn error(message: String) -> ToolOutput {
    ToolOutput {
        output: message,
        is_error: true,
        ..ToolOutput::default()
    }
}

#[async_trait::async_trait]
impl TypedTool for Member {
    type Args = MemberArgs;

    fn name(&self) -> &str {
        "member"
    }

    fn description(&self) -> &str {
        "Inspect or retune a teammate without waiting for its turn to end. \
         `op = \"status\"` returns the member's last completed work (no model \
         call); `op = \"ask\"` puts a tool-free question to the member's own \
         model from its current context (needs `text`); `op = \"cancel\"` stops \
         the member's in-flight run and waits for it to stop; \
         `op = \"model\"`/`op = \"effort\"` retune a peer (the roster is \
         updated); `op = \"usage\"` reports the member's own tokens, cache hit \
         rate, per-tool errors and `$` cost (no model call); `op = \
         \"compact\"` summarizes a peer's older history now; `op = \"retire\"` \
         drops it from the roster AND marks it retired so it does not respawn \
         on --resume."
    }

    async fn execute(&self, args: MemberArgs, _ctx: &ToolContext) -> ToolOutput {
        let to = self.target(&args.to);
        match args.op.as_str() {
            "usage" => self.usage(&to).await,
            "status" => self.status(&to).await,
            "ask" => match args.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(text) => self.ask(&to, text).await,
                None => error("`ask` needs `text` (the question)".to_string()),
            },
            "cancel" => self.cancel(&to).await,
            "model" => match args.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                Some(model) => self.set_model(&to, model).await,
                None => error("`model` needs `model` (the new model id)".to_string()),
            },
            "effort" => match args.effort.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
                Some(effort) => self.set_effort(&to, effort).await,
                None => error("`effort` needs `effort` (`-` clears it)".to_string()),
            },
            "compact" => self.compact(&to, args.instructions.as_deref()).await,
            "retire" => self.retire(&to, args.force).await,
            other => error(format!(
                "unknown op `{other}` (expected status | ask | cancel | model | effort | compact | usage | retire)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use wcode_harness::actor::SessionActor;
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::LlmStreamEvent;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::message::{AgentMessage, StopReason};
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    fn ctx() -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "m1".into(),
            name: "member".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
            session_path: None,
        }
    }

    /// Poll until `id` reaches `want` (the registry mirrors each member's run
    /// state through a watcher task, so it updates on its own).
    async fn wait_until(registry: &Registry, id: &SessionId, want: MemberState) {
        for _ in 0..300 {
            if registry.state_of(id) == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{id} never reached {want:?}");
    }

    /// A registered member with `context` seeded and a caller-supplied stream.
    fn member_session(
        registry: &Registry,
        name: &str,
        context: Vec<AgentMessage>,
        stream_fn: StreamFn,
    ) -> SessionId {
        member_session_with(
            registry,
            name,
            context,
            stream_fn,
            CompactionPolicy::default(),
        )
    }

    /// Like [`member_session`], but with a caller-supplied compaction policy — a
    /// `compact` test needs a policy whose cut is non-empty for a small history.
    fn member_session_with(
        registry: &Registry,
        name: &str,
        context: Vec<AgentMessage>,
        stream_fn: StreamFn,
        compaction: CompactionPolicy,
    ) -> SessionId {
        let agent = Agent::new(AgentConfig {
            system: "sys".into(),
            tools: vec![],
            llm: LlmOpts {
                model: "m1".into(),
                ..LlmOpts::default()
            },
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context,
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction,
            plan_mode: wcode_harness::hooks::PlanModeHandle::new(),
            // W031 test double: the kernel default, like every non-CLI site.
            title_after_turns: wcode_harness::title::DEFAULT_TITLE_AFTER_TURNS,
        });
        let handle = SessionActor::spawn(agent);
        let id = SessionId::agent(name);
        registry.register(id.clone(), handle);
        id
    }

    fn text_stream(text: &'static str) -> StreamFn {
        Arc::new(move |_c, _s, _t, _o| {
            Box::pin(futures::stream::iter(vec![
                LlmStreamEvent::TextDelta(text.into()),
                LlmStreamEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: None,
                },
            ])) as LlmStream
        })
    }

    fn hanging_stream() -> StreamFn {
        Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::pending::<LlmStreamEvent>()) as LlmStream
        })
    }

    /// The orchestrator's side: a registry that owns the member + a `member` tool.
    fn orchestrator(registry: &Registry) -> Member {
        orchestrator_with(registry, Phonebook::default(), TaskList::new(), None)
    }

    /// A `member` tool over caller-supplied phonebook / plan / group — for the
    /// ops that read them (`retire` needs the plan and the group; `model`/`retire`
    /// mirror the phonebook).
    fn orchestrator_with(
        registry: &Registry,
        phonebook: Phonebook,
        tasks: TaskList,
        group: Option<SessionGroup>,
    ) -> Member {
        Member::new(
            registry.clone(),
            SessionId::agent("orchestrator"),
            phonebook,
            tasks,
            group,
        )
    }

    #[tokio::test]
    async fn status_reports_state_and_last_work() {
        let registry = Registry::new();
        let assistant = AgentMessage::Assistant {
            content: vec![wcode_harness::message::ContentBlock::Text {
                text: "running cargo test".into(),
            }],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        };
        let id = member_session(&registry, "w1", vec![assistant], text_stream("unused"));
        // Wire the ownership edge so `resolve` permits it.
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "status".into(),
                    text: None,
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("running cargo test"), "{}", out.output);
    }

    #[tokio::test]
    async fn usage_reports_the_members_tokens_without_a_model_call() {
        let registry = Registry::new();
        let assistant = AgentMessage::Assistant {
            content: vec![wcode_harness::message::ContentBlock::Text {
                text: "did some work".into(),
            }],
            stop_reason: StopReason::Stop,
            usage: Some(wcode_harness::message::Usage {
                input_tokens: 1_000,
                output_tokens: 500,
                cache_read_tokens: Some(400),
                cache_write_tokens: None,
            }),
            model: None,
        };
        let id = member_session(&registry, "w1", vec![assistant], text_stream("unused"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "usage".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("1 turn"), "{}", out.output);
        assert!(out.output.contains("1000 in"), "{}", out.output);
    }

    #[tokio::test]
    async fn usage_on_an_empty_member_reports_no_usage() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("unused"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "usage".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("(no usage reported)"), "{}", out.output);
    }

    #[tokio::test]
    async fn ask_returns_the_members_side_answer() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("I am debugging X"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "ask".into(),
                    text: Some("what are you doing?".into()),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(out.output, "I am debugging X");
    }

    #[tokio::test]
    async fn cancel_stops_a_running_member() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), hanging_stream());
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        // Start a run that hangs, and wait until it is observably running.
        registry
            .resolve(&SessionId::agent("orchestrator"), &id)
            .unwrap()
            .send(Request::Submit { text: "work".into() })
            .unwrap();
        wait_until(&registry, &id, MemberState::Running).await;

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "cancel".into(),
                    text: None,
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("cancelled"), "{}", out.output);
        assert_eq!(registry.state_of(&id), MemberState::Done);
    }

    #[tokio::test]
    async fn ask_without_text_is_an_error() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));
        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "ask".into(),
                    text: None,
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("needs `text`"), "{}", out.output);
    }

    #[tokio::test]
    async fn unknown_op_is_an_error() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));
        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "poke".into(),
                    text: None,
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("unknown op"), "{}", out.output);
    }

    #[tokio::test]
    #[ignore = "live test; run with WCODE_BASE_URL=... (and WCODE_MODEL=...)"]
    async fn live_ask_against_a_real_model() {
        let base = std::env::var("WCODE_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".into());
        let model = std::env::var("WCODE_MODEL").unwrap_or_else(|_| "gemma4:e4b".into());

        let registry = Registry::new();
        let agent = Agent::new(AgentConfig {
            system: "You are a teammate. Answer in one short sentence.".into(),
            tools: vec![],
            llm: LlmOpts {
                model,
                base_url: Some(base),
                ..LlmOpts::default()
            },
            stream_fn: wcode_harness::streamfn::rig_stream_fn(),
            hooks: HooksSet::default(),
            session: None,
            context: vec![AgentMessage::user_text("I am refactoring the parser.")],
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
            plan_mode: wcode_harness::hooks::PlanModeHandle::new(),
            // W031 test double: the kernel default, like every non-CLI site.
            title_after_turns: wcode_harness::title::DEFAULT_TITLE_AFTER_TURNS,
        });
        let id = SessionId::agent("w1");
        registry.register(id.clone(), SessionActor::spawn(agent));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "ask".into(),
                    text: Some("what are you working on?".into()),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        println!("LIVE member ask -> {}", out.output);
        assert!(!out.is_error, "{}", out.output);
        assert!(!out.output.trim().is_empty());
    }

    // ---- new ops (W020 §4.1) ----

    /// A minimal `SessionGroup` under a temp dir, for the durable-retire tests.
    fn group_under(dir: &std::path::Path) -> SessionGroup {
        let gdir = dir.join("100_a");
        std::fs::create_dir_all(gdir.join("members")).unwrap();
        SessionGroup {
            root_path: gdir.join("root.jsonl"),
            members_dir: gdir.join("members"),
            dir: gdir,
        }
    }

    #[tokio::test]
    async fn model_op_acks_and_mirrors_the_roster() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "model".into(),
                    model: Some("gpt-x".into()),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("gpt-x"), "{}", out.output);
        assert_eq!(
            registry.model_of(&id).as_deref(),
            Some("gpt-x"),
            "the roster mirrors the new model"
        );
    }

    #[tokio::test]
    async fn model_op_without_a_model_is_an_error() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));
        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "model".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("needs `model`"), "{}", out.output);
    }

    #[tokio::test]
    async fn effort_op_sets_and_dash_clears_it() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "effort".into(),
                    effort: Some("high".into()),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(registry.effort_of(&id).as_deref(), Some("high"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "effort".into(),
                    effort: Some("-".into()),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(registry.effort_of(&id), None, "`-` clears the effort");
    }

    #[tokio::test]
    async fn compact_op_surfaces_summarized_and_kept() {
        let registry = Registry::new();
        let assistant = |text: &str| AgentMessage::Assistant {
            content: vec![wcode_harness::message::ContentBlock::Text {
                text: text.into(),
            }],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        };
        let context = vec![
            AgentMessage::user_text("first"),
            assistant("a1"),
            AgentMessage::user_text("second"),
            assistant("a2"),
        ];
        // A policy whose cut is non-empty for a two-turn history.
        let policy = CompactionPolicy {
            keep_recent_tokens: 0,
            keep_recent_turns: 1,
            ..CompactionPolicy::default()
        };
        let id = member_session_with(&registry, "w1", context, text_stream("the summary"), policy);
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "compact".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("summarized 2"), "{}", out.output);
        assert!(out.output.contains("kept 2"), "{}", out.output);
    }

    #[tokio::test]
    async fn compact_op_with_nothing_to_do_says_so() {
        let registry = Registry::new();
        // Default policy + a tiny history → the cut is empty (`NothingToDo`).
        let id = member_session(
            &registry,
            "w1",
            vec![AgentMessage::user_text("hi")],
            text_stream("unused"),
        );
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));
        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "compact".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("nothing to compact"), "{}", out.output);
    }

    #[tokio::test]
    async fn retire_op_removes_and_marks_retired() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let dir = tempfile::tempdir().unwrap();
        let group = group_under(dir.path());
        session_groups::record_member(
            &group,
            &crate::agents::WorkerSpec {
                name: Some("w1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let book = Phonebook::default();
        book.insert("w1", id.clone());

        let out = orchestrator_with(&registry, book.clone(), TaskList::new(), Some(group.clone()))
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "retire".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(!registry.contains(&id), "gone from the registry");
        assert_eq!(book.get("w1"), None, "gone from the phonebook");
        let record = session_groups::load_manifest(&group)
            .unwrap()
            .members
            .into_iter()
            .find(|r| r.name == "w1")
            .unwrap();
        assert!(record.retired, "the durable retired flag is set");
    }

    #[tokio::test]
    async fn retire_refuses_a_task_owned_member_unless_forced() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), text_stream("x"));
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        let tasks = TaskList::new();
        let task = tasks.create("ship it", Vec::new(), false).unwrap();
        tasks.assign(task.id, id.clone()).unwrap();

        let tool = orchestrator_with(&registry, Phonebook::default(), tasks, None);
        let out = tool
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "retire".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("force"), "{}", out.output);
        assert!(registry.contains(&id), "refused: still registered");

        let out = tool
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "retire".into(),
                    force: true,
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(!registry.contains(&id), "forced retire succeeds");
    }

    #[tokio::test]
    async fn a_running_member_is_cancelled_before_retire() {
        let registry = Registry::new();
        let id = member_session(&registry, "w1", Vec::new(), hanging_stream());
        registry.set_owner(id.clone(), SessionId::agent("orchestrator"));

        registry
            .resolve(&SessionId::agent("orchestrator"), &id)
            .unwrap()
            .send(Request::Submit { text: "work".into() })
            .unwrap();
        wait_until(&registry, &id, MemberState::Running).await;

        let out = orchestrator(&registry)
            .execute(
                MemberArgs {
                    to: "w1".into(),
                    op: "retire".into(),
                    ..MemberArgs::default()
                },
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(!registry.contains(&id), "removed after cancelling");
        assert_ne!(
            registry.state_of(&id),
            MemberState::Running,
            "the run was cancelled, not left running"
        );
    }
}

