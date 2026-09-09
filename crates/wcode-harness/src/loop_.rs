use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::event::{AgentEvent, LlmStreamEvent};
use crate::hooks::{HooksSet, ToolCall as HookToolCall};
use crate::message::{AgentMessage, ContentBlock, StopReason, Usage};
use crate::session::{Session, SessionEntry};
use crate::streamfn::{LlmOpts, StreamFn};
use crate::tool::{Tool, ToolContext, ToolOutput};

pub struct LoopConfig<'a> {
    pub system: String,
    pub tools: Vec<Tool>,
    pub llm: LlmOpts,
    pub stream_fn: StreamFn,
    pub hooks: HooksSet,
    pub steering: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
    pub follow_ups: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
    pub cancel: CancellationToken,
    /// Working directory for tool execution (bash cwd, file resolution).
    /// Plumbed explicitly so embeddings can run against a directory other
    /// than the process cwd; an empty path falls back to `current_dir()`.
    pub working_dir: std::path::PathBuf,
    /// Optional session to persist each produced message to as it is pushed
    /// into context — incremental, so a crash mid-run loses at most the
    /// in-flight message instead of the whole turn.
    pub session: Option<&'a mut Session>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error(transparent)]
    Session(std::io::Error), // session append failure while persisting produced messages
}

pub async fn run_loop(
    ctx: &mut Vec<AgentMessage>,
    mut cfg: LoopConfig<'_>,
    sink: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
) -> Result<StopReason, LoopError> {
    let tool_defs: Vec<rig::completion::ToolDefinition> =
        cfg.tools.iter().map(|t| t.definition()).collect();

    if sink.send(AgentEvent::AgentStart).is_err() {
        // Consumer gone before the run started: nothing to finalize.
        return Ok(StopReason::Aborted);
    }

    'outer: loop {
        loop {
            let mut aborted = false;
            if sink.send(AgentEvent::TurnStart).is_err() {
                aborted = true;
            }

            while let Ok(msg) = cfg.steering.try_recv() {
                if sink
                    .send(AgentEvent::MessageStart {
                        message: msg.clone(),
                    })
                    .is_err()
                {
                    aborted = true;
                    break;
                }
                record_session(&mut cfg, &msg)?;
                ctx.push(msg.clone());
                if sink.send(AgentEvent::MessageEnd { message: msg }).is_err() {
                    aborted = true;
                    break;
                }
            }
            cfg.hooks.transform_context(ctx).await;

            let mut content: Vec<ContentBlock> = Vec::new();
            let mut captured: Option<StopReason> = None;
            let mut usage: Option<Usage> = None;

            if sink
                .send(AgentEvent::MessageStart {
                    message: assistant(&content, StopReason::Stop, None),
                })
                .is_err()
            {
                aborted = true;
            }

            // Skip the LLM call entirely when the sink is already dead.
            if !aborted {
                let mut stream = (cfg.stream_fn)(ctx.as_slice(), &cfg.system, &tool_defs, &cfg.llm);
                loop {
                    tokio::select! {
                                            biased;
                                            _ = cfg.cancel.cancelled() => {
                                                aborted = true;
                                                break;
                                            }
                                            item = stream.next() => match item {
                                                None => break,
                                                Some(LlmStreamEvent::TextDelta(delta)) => {
                                                    append_text(&mut content, &delta);
                                                    if sink
                                                        .send(AgentEvent::MessageUpdate {
                                                            message: assistant(&content, StopReason::Stop, None),
                                                        })
                                                        .is_err()
                                                    {
                                                        aborted = true;
                                                        break;
                                                    }
                                                }
                    Some(LlmStreamEvent::ThinkingDelta(delta)) => {
                                                    append_thinking(&mut content, &delta);
                                                    if sink
                                                        .send(AgentEvent::MessageUpdate {
                                                            message: assistant(&content, StopReason::Stop, None),
                                                        })
                                                        .is_err()
                                                    {
                                                        aborted = true;
                                                        break;
                                                    }
                                                }
                                                Some(LlmStreamEvent::ThinkingReplace(delta)) => {
                                                    replace_thinking(&mut content, &delta);
                                                    if sink
                                                        .send(AgentEvent::MessageUpdate {
                                                            message: assistant(&content, StopReason::Stop, None),
                                                        })
                                                        .is_err()
                                                    {
                                                        aborted = true;
                                                        break;
                                                    }
                                                }
                                                // v1: no event for ToolCallStart.
                                                Some(LlmStreamEvent::ToolCallStart { .. }) => {}
                                                Some(LlmStreamEvent::ToolCall { id, name, arguments }) => {
                                                    content.push(ContentBlock::ToolCall { id, name, arguments });
                                                    if sink
                                                        .send(AgentEvent::MessageUpdate {
                                                            message: assistant(&content, StopReason::Stop, None),
                                                        })
                                                        .is_err()
                                                    {
                                                        aborted = true;
                                                        break;
                                                    }
                                                }
                                                Some(LlmStreamEvent::Done { stop_reason, usage: u }) => {
                                                    captured = Some(stop_reason);
                                                    usage = u;
                                                    // Done ends the fold: a stalled stream must not
                                                    // hang the run or overwrite the captured reason.
                                                    break;
                                                }
                                                // A stream Error behaves like Done{Error}: capture, end run.
                                                // The message is surfaced first so consumers can show it.
                                                Some(LlmStreamEvent::Error { message }) => {
                                                    let _ = sink.send(AgentEvent::Error { message });
                                                    captured = Some(StopReason::Error);
                                                    break;
                                                }
                                            }
                                        }
                }
            }

            // Cancellation finalizes the partial assistant as Aborted.
            let stop = if aborted {
                StopReason::Aborted
            } else {
                captured.unwrap_or(StopReason::Stop)
            };
            let assistant = assistant(&content, stop, usage);
            if sink
                .send(AgentEvent::MessageEnd {
                    message: assistant.clone(),
                })
                .is_err()
            {
                aborted = true;
            }
            record_session(&mut cfg, &assistant)?;
            ctx.push(assistant.clone());
            if sink
                .send(AgentEvent::TurnEnd {
                    message: assistant.clone(),
                })
                .is_err()
            {
                aborted = true;
            }

            let calls: Vec<(String, String, serde_json::Value)> = assistant
                .tool_calls()
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolCall {
                        id,
                        name,
                        arguments,
                    } => Some((id.clone(), name.clone(), arguments.clone())),
                    _ => None,
                })
                .collect();

            // Errors/aborts are values: end the run even if tool calls exist.
            // Every unexecuted call first gets a synthesized error ToolResult
            // so ctx never keeps a ToolCall without its ToolResult (invalid
            // history → wire 400s on the next turn, persisted via session).
            if aborted || matches!(captured, Some(StopReason::Error | StopReason::Aborted)) {
                if let Err(e) = synthesize_unexecuted(&mut cfg, &sink, ctx, calls.into_iter()) {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return Err(e);
                }
                let _ = sink.send(AgentEvent::AgentEnd);
                return Ok(if aborted {
                    StopReason::Aborted
                } else {
                    captured.unwrap()
                });
            }
            if calls.is_empty() {
                break; // turn with no tool calls: outer loop handles follow-ups
            }

            // Tool execution (sequential).
            let mut pending = calls.into_iter();
            while let Some((id, name, mut arguments)) = pending.next() {
                let mut hook_call = HookToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                };
                // Rewrites/transforms run before everything else so what
                // executes, blocks and is logged is the final command.
                cfg.hooks.transform_tool_input(&mut hook_call).await;
                arguments = hook_call.arguments.clone();
                let start_sent = sink
                    .send(AgentEvent::ToolExecutionStart {
                        call_id: id.clone(),
                        name: name.clone(),
                    })
                    .is_ok();
                if !start_sent {
                    aborted = true;
                }
                let out = if !start_sent {
                    // Dead sink: the tool never runs. Synthesize its error
                    // output so the ToolResult still lands in ctx.
                    ToolOutput {
                        output: "aborted before execution".to_string(),
                        is_error: true,
                        ..ToolOutput::default()
                    }
                } else if let Some(reason) = cfg.hooks.before_tool_call(&hook_call).await {
                    ToolOutput {
                        output: format!("blocked: {reason}"),
                        is_error: true,
                        ..ToolOutput::default()
                    }
                } else {
                    match cfg.tools.iter().find(|t| t.name() == name) {
                        Some(tool) => {
                            let tctx = ToolContext {
                                call_id: id.clone(),
                                name: name.clone(),
                                working_dir: cfg.working_dir.clone(),
                                cancel: cfg.cancel.clone(),
                                events: sink.clone(),
                            };
                            let mut out = tool.execute(arguments, tctx).await;
                            cfg.hooks.after_tool_call(&hook_call, &mut out).await;
                            out
                        }
                        None => ToolOutput {
                            output: format!("unknown tool: {name}"),
                            is_error: true,
                            ..ToolOutput::default()
                        },
                    }
                };
                let sent = sink
                    .send(AgentEvent::ToolExecutionEnd {
                        call_id: id.clone(),
                        name: name.clone(),
                        output: out.output.clone(),
                        is_error: out.is_error,
                    })
                    .is_ok();
                // Push the result before honoring a dead sink so ctx never
                // keeps a ToolCall without its ToolResult.
                let result = AgentMessage::ToolResult {
                    tool_call_id: id,
                    name,
                    output: out.output,
                    is_error: out.is_error,
                };
                record_session(&mut cfg, &result)?;
                ctx.push(result);
                if !sent {
                    aborted = true;
                }
                if aborted {
                    // Sink died mid-tool-loop: synthesize an error ToolResult
                    // for every unexecuted call (event sends are best-effort;
                    // the abort path already returns Aborted).
                    if let Err(e) = synthesize_unexecuted(&mut cfg, &sink, ctx, pending) {
                        let _ = sink.send(AgentEvent::AgentEnd);
                        return Err(e);
                    }
                    break;
                }
            }

            if cfg.cancel.is_cancelled() {
                // A cancel that fired mid-tool-loop ends the run right here
                // instead of rolling into another LLM turn: tools already saw
                // the token via ToolContext.cancel and pushed their results, so
                // the history is complete (no orphan ToolCalls to synthesize).
                aborted = true;
            }
            if aborted {
                let _ = sink.send(AgentEvent::AgentEnd);
                return Ok(StopReason::Aborted);
            }
            if cfg.hooks.should_stop_after_turn(ctx).await {
                let _ = sink.send(AgentEvent::AgentEnd);
                return Ok(StopReason::Stop);
            }
        }

        match cfg.follow_ups.try_recv() {
            Ok(m) => {
                if sink
                    .send(AgentEvent::MessageStart { message: m.clone() })
                    .is_err()
                {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return Ok(StopReason::Aborted);
                }
                record_session(&mut cfg, &m)?;
                ctx.push(m.clone());
                if sink.send(AgentEvent::MessageEnd { message: m }).is_err() {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return Ok(StopReason::Aborted);
                }
                continue 'outer;
            }
            Err(
                tokio::sync::mpsc::error::TryRecvError::Empty
                | tokio::sync::mpsc::error::TryRecvError::Disconnected,
            ) => {
                let _ = sink.send(AgentEvent::AgentEnd);
                return Ok(StopReason::Stop);
            }
        }
    }
}

fn assistant(
    content: &[ContentBlock],
    stop_reason: StopReason,
    usage: Option<Usage>,
) -> AgentMessage {
    AgentMessage::Assistant {
        content: content.to_vec(),
        stop_reason,
        usage,
        model: None,
    }
}

/// Synthesizes an error ToolResult for every unexecuted tool call so ctx
/// never keeps a ToolCall without its ToolResult. Event sends are
/// best-effort (the sink may already be dead). Pushes are persisted to the
/// session like every other produced message.
fn synthesize_unexecuted(
    cfg: &mut LoopConfig<'_>,
    sink: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    ctx: &mut Vec<AgentMessage>,
    calls: impl Iterator<Item = (String, String, serde_json::Value)>,
) -> Result<(), LoopError> {
    for (rid, rname, _) in calls {
        let out = ToolOutput {
            output: "aborted before execution".to_string(),
            is_error: true,
            ..ToolOutput::default()
        };
        let _ = sink.send(AgentEvent::ToolExecutionEnd {
            call_id: rid.clone(),
            name: rname.clone(),
            output: out.output.clone(),
            is_error: out.is_error,
        });
        let result = AgentMessage::ToolResult {
            tool_call_id: rid,
            name: rname,
            output: out.output,
            is_error: out.is_error,
        };
        record_session(cfg, &result)?;
        ctx.push(result);
    }
    Ok(())
}

/// Persist a just-produced message to the configured session, if any. Runs at
/// every `ctx.push` so a crash mid-run loses only the in-flight message
/// instead of the whole turn.
fn record_session(cfg: &mut LoopConfig<'_>, msg: &AgentMessage) -> Result<(), LoopError> {
    let Some(session) = cfg.session.as_deref_mut() else {
        return Ok(());
    };
    session
        .append(SessionEntry::Message {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            message: msg.clone(),
        })
        .map_err(LoopError::Session)
}

fn append_text(content: &mut Vec<ContentBlock>, delta: &str) {
    match content.last_mut() {
        Some(ContentBlock::Text { text }) => text.push_str(delta),
        _ => content.push(ContentBlock::Text {
            text: delta.to_string(),
        }),
    }
}

fn append_thinking(content: &mut Vec<ContentBlock>, delta: &str) {
    match content.last_mut() {
        Some(ContentBlock::Thinking { text }) => text.push_str(delta),
        _ => content.push(ContentBlock::Thinking {
            text: delta.to_string(),
        }),
    }
}

/// Replacement semantics for a complete Thinking block: any thinking
/// accumulated from deltas is dropped and the block holds just `text`.
fn replace_thinking(content: &mut Vec<ContentBlock>, text: &str) {
    content.retain(|b| !matches!(b, ContentBlock::Thinking { .. }));
    content.push(ContentBlock::Thinking {
        text: text.to_string(),
    });
}
