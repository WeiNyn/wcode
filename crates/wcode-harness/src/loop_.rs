use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::event::{AgentEvent, LlmStreamEvent};
use crate::hooks::{Hooks, ToolCall as HookToolCall};
use crate::message::{AgentMessage, ContentBlock, StopReason, Usage};
use crate::streamfn::{LlmOpts, StreamFn};
use crate::tool::{Tool, ToolContext, ToolOutput};

pub struct LoopConfig {
    pub system: String,
    pub tools: Vec<Tool>,
    pub llm: LlmOpts,
    pub stream_fn: StreamFn,
    pub hooks: Arc<dyn Hooks>,
    pub steering: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
    pub follow_ups: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
    pub cancel: CancellationToken,
}

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error(transparent)]
    Session(std::io::Error), // reserved; the loop itself never produces it in v1
}

pub async fn run_loop(
    ctx: &mut Vec<AgentMessage>,
    mut cfg: LoopConfig,
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

            let _ = sink.send(AgentEvent::MessageStart {
                message: assistant(&content, StopReason::Stop, None),
            });

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
                            Some(LlmStreamEvent::Error { .. }) => {
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
            ctx.push(assistant.clone());
            if sink
                .send(AgentEvent::TurnEnd {
                    message: assistant.clone(),
                })
                .is_err()
            {
                aborted = true;
            }

            // Errors/aborts are values: end the run even if tool calls exist.
            if aborted {
                let _ = sink.send(AgentEvent::AgentEnd);
                return Ok(StopReason::Aborted);
            }
            match captured {
                Some(StopReason::Error) | Some(StopReason::Aborted) => {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return Ok(captured.unwrap());
                }
                _ => {}
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
            if calls.is_empty() {
                break; // turn with no tool calls: outer loop handles follow-ups
            }

            // Tool execution (sequential).
            for (id, name, arguments) in calls {
                let hook_call = HookToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                };
                if sink
                    .send(AgentEvent::ToolExecutionStart {
                        call_id: id.clone(),
                        name: name.clone(),
                    })
                    .is_err()
                {
                    aborted = true;
                    break;
                }
                let out = if let Some(reason) = cfg.hooks.before_tool_call(&hook_call).await {
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
                                // ponytail: env cwd; plumb a LoopConfig.working_dir if a caller needs to override
                                working_dir: std::env::current_dir().unwrap_or_default(),
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
                ctx.push(AgentMessage::ToolResult {
                    tool_call_id: id,
                    name,
                    output: out.output,
                    is_error: out.is_error,
                });
                if !sent {
                    aborted = true;
                    break;
                }
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
