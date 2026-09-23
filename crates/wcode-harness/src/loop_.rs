use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::compaction::{self, CompactOutcome, CompactionPolicy};
use crate::event::{AgentEvent, LlmStreamEvent};
use crate::hooks::{HooksSet, ToolCall as HookToolCall};
use crate::message::{AgentMessage, ContentBlock, StopReason, Usage};
use crate::session::{Session, SessionEntry};
use crate::streamfn::{LlmOpts, StreamFn};
use crate::tool::{Tool, ToolContext, ToolOutput};

/// Default per-run turn cap for [`LoopConfig::max_turns`]. Generous enough for
/// long agentic tasks, low enough that a runaway tool loop can't burn tokens
/// unbounded. Unlike cancellation, hitting it is a clean, resumable stop.
pub const DEFAULT_MAX_TURNS: usize = 100;

/// Cap on *consecutive* turns that ended in a fed-back stream error. A
/// transient-class stream failure no longer kills the run — the loop feeds the
/// error back to the model and retries the turn — but a persistently broken
/// request (or a model that cannot recover) must still terminate. After this
/// many consecutive fed-back error turns the run ends with `StopReason::Error`
/// exactly as it did before. Any turn that completes cleanly resets the count.
pub const DEFAULT_MAX_STREAM_ERROR_TURNS: usize = 3;

/// Cap on tools executing concurrently inside one group. The model chooses the
/// batch size, but a runaway batch should not open unbounded file handles.
const MAX_PARALLEL_TOOLS: usize = 8;

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
    /// Maximum number of LLM turns in a single run. A turn is one model call
    /// plus the tool calls it produces. Caps a runaway tool loop (the model
    /// re-issuing tool calls forever) that would otherwise run until cancelled;
    /// the run ends on a completed turn boundary with `StopReason::MaxTurns`.
    /// Counts every turn in the run, including follow-up-triggered ones.
    pub max_turns: usize,
    /// Run concurrency-safe tool calls in one batch in parallel (see
    /// [`crate::tool::TypedTool::parallel_safe`]). `false` forces the old
    /// strictly-sequential behaviour.
    pub parallel: bool,
    /// When to auto-summarize the older prefix, and how much recent context to
    /// keep (see [`crate::compaction`]).
    pub compaction: CompactionPolicy,
    /// Optional session to persist each produced message to as it is pushed
    /// into context — incremental, so a crash mid-run loses at most the
    /// in-flight message instead of the whole turn.
    pub session: Option<&'a mut Session>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error(transparent)]
    Session(std::io::Error), // session append failure while persisting produced messages
    /// A side-call (`Agent::side_ask`) failure: a stream error, or a stream that
    /// produced no text. Not produced by `run_loop` (which surfaces a stream
    /// failure as `AgentEvent::Error`).
    #[error("{0}")]
    Stream(String),
}

/// Return value of [`run_loop`]: the stop reason plus the two input channels
/// that were lent in, handed back so messages queued but never drained (e.g. a
/// follow-up sent while the loop was on its final turn) survive the run
/// boundary instead of being dropped when the caller re-pairs channels.
#[derive(Debug)]
pub struct RunResult {
    pub stop_reason: StopReason,
    pub steering: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
    pub follow_ups: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
}

/// Wrap a normal run exit together with the lent-in channels.
fn finish(
    stop_reason: StopReason,
    steering: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
    follow_ups: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,
) -> Result<RunResult, LoopError> {
    Ok(RunResult {
        stop_reason,
        steering,
        follow_ups,
    })
}

/// The next message to inject at the outer tail: a queued follow-up first
/// (preserving the original behavior), else a queued steer (soft interrupt,
/// point B). One per outer iteration; `continue 'outer` re-runs the turn-start
/// drain, which picks up any remainder.
fn next_injected(cfg: &mut LoopConfig<'_>) -> Option<AgentMessage> {
    cfg.follow_ups
        .try_recv()
        .ok()
        .or_else(|| cfg.steering.try_recv().ok())
}

pub async fn run_loop(
    ctx: &mut Vec<AgentMessage>,
    mut cfg: LoopConfig<'_>,
    sink: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
) -> Result<RunResult, LoopError> {
    let tool_defs: Vec<rig::completion::ToolDefinition> =
        cfg.tools.iter().map(|t| t.definition()).collect();
    // The model producing this run's assistant messages, stamped on each for
    // provenance. The session records it; the wire converter ignores it.
    let model = cfg.llm.model.clone();

    if sink.send(AgentEvent::AgentStart).is_err() {
        // Consumer gone before the run started: nothing to finalize.
        return finish(StopReason::Aborted, cfg.steering, cfg.follow_ups);
    }

    // Turns used so far, checked against `cfg.max_turns` at each turn start.
    let mut turns: usize = 0;
    // Consecutive fed-back stream-error turns; see
    // [`DEFAULT_MAX_STREAM_ERROR_TURNS`]. Reset by any cleanly-completed turn.
    let mut consecutive_stream_errors: usize = 0;
    'outer: loop {
        loop {
            // Turn budget: caps a runaway tool loop (the model re-issuing tool
            // calls forever). Checked before a turn starts, so the run ends on a
            // completed turn boundary — the final turn's ToolResults are already
            // in ctx and no ToolCall is left unpaired. Counts every turn in the
            // run, including follow-up-triggered ones.
            if turns >= cfg.max_turns {
                let _ = sink.send(AgentEvent::AgentEnd);
                return finish(StopReason::MaxTurns, cfg.steering, cfg.follow_ups);
            }
            turns += 1;
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

            // Auto-compaction: when the last provider-reported context size is
            // near the ceiling, summarize the older prefix before this turn's
            // request. Best-effort — a summarizer failure must not abort a run
            // that can still proceed (the request itself surfaces real errors).
            if let Some(used) = last_input_tokens(ctx) {
                let window = cfg
                    .compaction
                    .context_window(cfg.llm.base_url.as_deref(), &cfg.llm.model);
                if cfg.compaction.should_compact(window, used) {
                    let outcome = compaction::compact_ctx(
                        ctx,
                        &cfg.compaction,
                        &cfg.stream_fn,
                        &cfg.llm,
                        None,
                        cfg.session.as_deref_mut(),
                    )
                    .await;
                    match outcome {
                        Ok(CompactOutcome::Done { summarized, kept, .. }) => {
                            let _ = sink.send(AgentEvent::Compaction { summarized, kept });
                        }
                        // Nothing to summarize is the quiet happy path — no event.
                        Ok(CompactOutcome::NothingToDo) => {}
                        // Best-effort: report the miss and keep the run going.
                        // Deliberately NOT `AgentEvent::Error` — that would trip
                        // the TUI `failed` state and the repl run-error line for a
                        // run that can still proceed.
                        Err(reason) => {
                            let _ = sink.send(AgentEvent::CompactionSkipped { reason });
                        }
                    }
                }
            }

            let mut content: Vec<ContentBlock> = Vec::new();
            let mut captured: Option<StopReason> = None;
            let mut usage: Option<Usage> = None;
            // A final stream failure that is *feedable* (transient class that
            // exhausted its retries, a mid-stream error, or the tool-use-with-
            // no-call finish). When set, the turn's error is written back into
            // ctx so the model can fix it, and the loop continues below.
            let mut stream_error: Option<String> = None;

            if sink
                .send(AgentEvent::MessageStart {
                    message: assistant(&content, StopReason::Stop, None, &model),
                })
                .is_err()
            {
                aborted = true;
            }

            // Skip the LLM call entirely when the sink is already dead, or when a
            // cancel landed between the post-tool check and this turn start —
            // minting the request only to abort it on the first poll of the
            // biased select would waste one LLM round-trip for nothing.
            if !aborted && cfg.cancel.is_cancelled() {
                aborted = true;
            }

            if !aborted {
                let mut stream = (cfg.stream_fn)(ctx.as_slice(), &cfg.system, &tool_defs, &cfg.llm);
                // The stall backstop's deadline, captured once (ZERO disables).
                let idle = cfg.llm.retry.idle;
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
                                                            message: assistant(&content, StopReason::Stop, None, &model),
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
                                                            message: assistant(&content, StopReason::Stop, None, &model),
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
                                                            message: assistant(&content, StopReason::Stop, None, &model),
                                                        })
                                                        .is_err()
                                                    {
                                                        aborted = true;
                                                        break;
                                                    }
                                                }
                                                Some(LlmStreamEvent::ToolCall { id, name, arguments }) => {
                                                    content.push(ContentBlock::ToolCall { id, name, arguments });
                                                    if sink
                                                        .send(AgentEvent::MessageUpdate {
                                                            message: assistant(&content, StopReason::Stop, None, &model),
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
                                                // A stream Error is surfaced first so consumers can show it. A
                                                // fatal-class failure ends the run; a transient-class one that
                                                // exhausted its retries (or landed mid-stream) is fed back below.
                                                Some(LlmStreamEvent::Retrying { attempt, max, reason }) => {
                                                    let _ = sink.send(AgentEvent::Retrying { attempt, max, reason });
                                                }
                                                Some(LlmStreamEvent::Error { message, fatal }) => {
                                                    let _ = sink.send(AgentEvent::Error { message: message.clone() });
                                                    // Hard-fatal (bad request/auth/schema): retrying or re-feeding
                                                    // cannot help — end the run. Transient-class (retries exhausted
                                                    // or mid-stream after content): feed back and continue below.
                                                    if fatal {
                                                        captured = Some(StopReason::Error);
                                                    } else {
                                                        stream_error = Some(message);
                                                        captured = Some(StopReason::Error);
                                                    }
                                                    break;
                                                }
                                            },
                                            // Kernel backstop: a `StreamFn` that never yields must
                                            // not wedge the run. Feed the failure back — the same
                                            // corrective-turn path as a stream error, bounded by
                                            // DEFAULT_MAX_STREAM_ERROR_TURNS. `idle == ZERO` disables.
                                            _ = tokio::time::sleep(idle), if !idle.is_zero() => {
                                                let message = format!("stream stalled {idle:?}");
                                                let _ = sink.send(AgentEvent::Error { message: message.clone() });
                                                stream_error = Some(message);
                                                captured = Some(StopReason::Error);
                                                break;
                                            }
                                        }
                }
            }

            // Finding #3: a tool-use finish that never streamed a tool call
            // means the model's requested action was lost. rig only delivers
            // the reassembled call on ToolInputEnd (deltas are dropped); a
            // provider that ends input without closing it has no call to emit,
            // so the call would vanish silently. Feed it back loudly instead of
            // silently ending: the model sees the failure and can re-issue the
            // call on the next turn.
            if !aborted
                && captured == Some(StopReason::ToolUse)
                && !content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolCall { .. }))
            {
                let message = "model requested tool use but no tool call was streamed".to_string();
                let _ = sink.send(AgentEvent::Error { message: message.clone() });
                stream_error = Some(message);
                captured = Some(StopReason::Error);
            }

            // Layer 2 kernel backstop: a `StreamFn` that ends a turn with no
            // terminal event at all — no `Done`, no `Error` — is a truncated turn.
            // The adapter (Layer 1) already synthesizes an error for the rig wire,
            // but a custom `StreamFn` (a test double, an embedder's provider) can
            // still reach here as a clean stop. Feed it back through the same
            // corrective-turn path as a stream error (bounded by
            // DEFAULT_MAX_STREAM_ERROR_TURNS) instead of silently stopping.
            //
            // Fires only when the select ended with NO terminal event: `captured`
            // is None and `stream_error` is None. Setting `stream_error` to a
            // non-None note deliberately bypasses the hard-fatal early-return below.
            if !aborted && stream_error.is_none() && captured.is_none() {
                let message = "stream ended without a terminal record".to_string();
                let _ = sink.send(AgentEvent::Error { message: message.clone() });
                stream_error = Some(message);
                captured = Some(StopReason::Error);
            }
            // Cancellation finalizes the partial assistant as Aborted.
            let stop = if aborted {
                StopReason::Aborted
            } else {
                captured.unwrap_or(StopReason::Stop)
            };
            let assistant = assistant(&content, stop, usage, &model);
            if sink
                .send(AgentEvent::MessageEnd {
                    message: assistant.clone(),
                })
                .is_err()
            {
                aborted = true;
            }
            // Don't persist an empty assistant. An abort/error that lands
            // before the model produced anything (dead sink, cancel before the
            // first delta, stream error on the first item) leaves nothing worth
            // keeping in ctx or the session; `to_rig_message` drops empty
            // assistant content at the wire anyway. Partial messages (content
            // streamed before the abort) are still recorded. Event sends above
            // stay unconditional so the consumer's message framing stays
            // balanced — MessageStart was already emitted.
            if !content.is_empty() {
                record_session(&mut cfg, &assistant)?;
                ctx.push(assistant.clone());
            }
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

            // Aborts and hard-fatal errors are values: end the run even if
            // tool calls exist. Every unexecuted call first gets a synthesized
            // error ToolResult so ctx never keeps a ToolCall without its
            // ToolResult (invalid history → wire 400s on the next turn,
            // persisted via session).
            if aborted || matches!(captured, Some(StopReason::Aborted)) {
                if let Err(e) = synthesize_unexecuted(&mut cfg, &sink, ctx, calls.into_iter()) {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return Err(e);
                }
                let _ = sink.send(AgentEvent::AgentEnd);
                return finish(StopReason::Aborted, cfg.steering, cfg.follow_ups);
            }
            // Hard-fatal stream error (captured Error with no feed-back note):
            // the request itself cannot succeed, so retrying or re-feeding is
            // pointless — end the run as before.
            if matches!(captured, Some(StopReason::Error)) && stream_error.is_none() {
                if let Err(e) = synthesize_unexecuted(&mut cfg, &sink, ctx, calls.into_iter()) {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return Err(e);
                }
                let _ = sink.send(AgentEvent::AgentEnd);
                return finish(StopReason::Error, cfg.steering, cfg.follow_ups);
            }
            // A feedable stream failure is written back into ctx — the model
            // sees "your previous response failed: <msg>" and can fix its
            // output, exactly like a malformed tool result. Consecutive such
            // turns are capped ([`DEFAULT_MAX_STREAM_ERROR_TURNS`]) so a broken
            // request or a model that cannot recover still terminates with
            // StopReason::Error as it always did.
            if let Some(err_msg) = stream_error {
                consecutive_stream_errors += 1;
                if let Err(e) = synthesize_unexecuted(&mut cfg, &sink, ctx, calls.into_iter()) {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return Err(e);
                }
                if consecutive_stream_errors > DEFAULT_MAX_STREAM_ERROR_TURNS {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return finish(StopReason::Error, cfg.steering, cfg.follow_ups);
                }
                let notice = AgentMessage::user_text(format!("your previous response failed: {err_msg}"));
                record_session(&mut cfg, &notice)?;
                ctx.push(notice);
                continue;
            }
            consecutive_stream_errors = 0;
            if calls.is_empty() {
                // Turn with no tool calls: the turn's tool work (none) is
                // done, so the stop hook — documented to run after a turn's
                // tool execution — fires here too. Otherwise its contract
                // silently skipped every tool-call-free final answer.
                if cfg.hooks.should_stop_after_turn(ctx).await {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return finish(StopReason::Stop, cfg.steering, cfg.follow_ups);
                }
                break; // outer loop handles follow-ups
            }

            // Tool execution. Preflight every call in order (transform, block,
            // tool lookup), then run each *group* of concurrency-safe calls in
            // parallel. A call that is not `parallel_safe` is a barrier — it
            // runs alone, so no read can race a mutation inside one batch.
            // Results land in ctx in call order whatever the completion order.
            let all_calls = calls.clone();
            let mut plan: Vec<Planned> = Vec::with_capacity(calls.len());
            for (id, name, mut arguments) in calls {
                let mut hook_call = HookToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                };
                // Rewrites/transforms run before everything else so what
                // executes, blocks and is logged is the final command.
                cfg.hooks.transform_tool_input(&mut hook_call).await;
                arguments = hook_call.arguments.clone();

                let planned = match cfg.hooks.before_tool_call(&hook_call).await {
                    Some(reason) => Planned::fail(id, name, format!("blocked: {reason}")),
                    None => match cfg.tools.iter().find(|t| t.name() == name).cloned() {
                        Some(tool) => {
                            let parallel_safe = cfg.parallel && tool.parallel_safe();
                            Planned {
                                id,
                                name,
                                run: Some(PlannedRun { tool, arguments }),
                                fail: None,
                                parallel_safe,
                            }
                        }
                        None => {
                            let msg = format!("unknown tool: {name}");
                            Planned::fail(id, name, msg)
                        }
                    },
                };
                plan.push(planned);
            }

            // Partition into groups: a maximal run of concurrency-safe calls
            // fans out; anything else is a single-call barrier.
            let mut groups: Vec<Vec<Planned>> = Vec::new();
            let mut run: Vec<Planned> = Vec::new();
            for p in plan {
                if p.parallel_safe {
                    run.push(p);
                } else {
                    if !run.is_empty() {
                        groups.push(std::mem::take(&mut run));
                    }
                    groups.push(vec![p]);
                }
            }
            if !run.is_empty() {
                groups.push(run);
            }

            let mut processed = 0usize;
            for group in groups {
                let width = group.len();
                let mut started = vec![false; width];
                let mut results: Vec<Option<ToolOutput>> = (0..width).map(|_| None).collect();
                let mut durations: Vec<Option<u64>> = (0..width).map(|_| None).collect();

                // Start events first, in call order, so the display pairs each
                // ✓ with the ⚙ marker above it.
                for (i, c) in group.iter().enumerate() {
                    let sent = sink
                        .send(AgentEvent::ToolExecutionStart {
                            call_id: c.id.clone(),
                            name: c.name.clone(),
                        })
                        .is_ok();
                    started[i] = sent;
                    if !sent {
                        // Dead sink: the tool never runs. Synthesize its error
                        // output so the ToolResult still lands in ctx.
                        results[i] = Some(ToolOutput {
                            output: "aborted before execution".to_string(),
                            is_error: true,
                            diff: None,
                            path: None,
                        });
                    }
                }

                // Fan out the runnable calls of this group. `after_tool_call`
                // runs inside the task so a tool's hooks stay with its result.
                let mut running = Vec::new();
                // D4 — the live session path, threaded into the tool ctx so
                // `session_search` can read this run's transcript. `cfg.session`
                // is `Option<&mut Session>`; `Session::path()` is `Option<&Path>`.
                let session_path = cfg
                    .session
                    .as_deref()
                    .and_then(|s| s.path())
                    .map(std::path::Path::to_path_buf);
                for (i, c) in group.iter().enumerate() {
                    if !started[i] {
                        continue;
                    }
                    let Some(run) = &c.run else {
                        results[i] = c.fail.clone();
                        continue;
                    };
                    let tool = run.tool.clone();
                    let arguments = run.arguments.clone();
                    let hook_call = HookToolCall {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        arguments: arguments.clone(),
                    };
                    let tctx = ToolContext {
                        call_id: c.id.clone(),
                        name: c.name.clone(),
                        working_dir: cfg.working_dir.clone(),
                        cancel: cfg.cancel.clone(),
                        events: sink.clone(),
                        session_path: session_path.clone(),
                    };
                    let hooks = cfg.hooks.clone();
                    running.push(async move {
                        let started = std::time::Instant::now();
                        let mut out = tool.execute(arguments, tctx).await;
                        hooks.after_tool_call(&hook_call, &mut out).await;
                        let duration_ms = started.elapsed().as_millis() as u64;
                        (i, out, duration_ms)
                    });
                }
                for (i, out, duration_ms) in futures::stream::iter(running)
                    .buffer_unordered(MAX_PARALLEL_TOOLS)
                    .collect::<Vec<_>>()
                    .await
                {
                    results[i] = Some(out);
                    durations[i] = Some(duration_ms);
                }

                // End events, session records and ctx pushes in call order.
                for (i, c) in group.iter().enumerate() {
                    let out = results[i].take().unwrap_or_else(|| ToolOutput {
                        output: "aborted before execution".to_string(),
                        is_error: true,
                        diff: None,
                        path: None,
                    });
                    let sent = started[i]
                        && sink
                            .send(AgentEvent::ToolExecutionEnd {
                                call_id: c.id.clone(),
                                name: c.name.clone(),
                                output: out.output.clone(),
                                is_error: out.is_error,
                                diff: out.diff.clone(),
                                path: out.path.clone(),
                                duration_ms: durations[i],
                            })
                            .is_ok();
                    // Push the result before honoring a dead sink so ctx never
                    // keeps a ToolCall without its ToolResult.
                    let result = AgentMessage::ToolResult {
                        tool_call_id: c.id.clone(),
                        name: c.name.clone(),
                        output: out.output,
                        is_error: out.is_error,
                    };
                    record_session(&mut cfg, &result)?;
                    ctx.push(result);
                    if !sent {
                        aborted = true;
                    }
                }

                processed += width;
                if aborted {
                    // Sink died mid-tool-loop: synthesize an error ToolResult
                    // for every unexecuted call (event sends are best-effort;
                    // the abort path already returns Aborted).
                    let rest = all_calls.iter().skip(processed).cloned();
                    if let Err(e) = synthesize_unexecuted(&mut cfg, &sink, ctx, rest) {
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
                return finish(StopReason::Aborted, cfg.steering, cfg.follow_ups);
            }
            if cfg.hooks.should_stop_after_turn(ctx).await {
                let _ = sink.send(AgentEvent::AgentEnd);
                return finish(StopReason::Stop, cfg.steering, cfg.follow_ups);
            }
        }

        // Soft-interrupt point B: drain a follow-up OR a steer at this shared
        // boundary. A `steer` that arrived during a tool-free turn used to be
        // deferred to a future run; now it injects here and starts the next turn.
        // The stop hook already ran on the way in, so `should_stop_after_turn`
        // stays authoritative.
        match next_injected(&mut cfg) {
            Some(m) => {
                if sink
                    .send(AgentEvent::MessageStart { message: m.clone() })
                    .is_err()
                {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return finish(StopReason::Aborted, cfg.steering, cfg.follow_ups);
                }
                record_session(&mut cfg, &m)?;
                ctx.push(m.clone());
                if sink.send(AgentEvent::MessageEnd { message: m }).is_err() {
                    let _ = sink.send(AgentEvent::AgentEnd);
                    return finish(StopReason::Aborted, cfg.steering, cfg.follow_ups);
                }
                continue 'outer;
            }
            None => {
                let _ = sink.send(AgentEvent::AgentEnd);
                return finish(StopReason::Stop, cfg.steering, cfg.follow_ups);
            }
        }
    }
}

fn assistant(
    content: &[ContentBlock],
    stop_reason: StopReason,
    usage: Option<Usage>,
    model: &str,
) -> AgentMessage {
    AgentMessage::Assistant {
        content: content.to_vec(),
        stop_reason,
        usage,
        // Provenance only: the wire converter ignores `model`; the session keeps it.
        model: (!model.is_empty()).then(|| model.to_string()),
    }
}

/// Synthesizes an error ToolResult for every unexecuted tool call so ctx
/// never keeps a ToolCall without its ToolResult. Event sends are
/// best-effort (the sink may already be dead). Pushes are persisted to the
/// session like every other produced message.
/// One tool call after preflight: either ready to run, or already failed
/// (blocked by a hook, unknown tool, dead sink).
struct Planned {
    id: String,
    name: String,
    /// `Some` = ready to execute.
    run: Option<PlannedRun>,
    /// `Some` = the result to record without executing.
    fail: Option<ToolOutput>,
    /// May run concurrently with its group peers.
    parallel_safe: bool,
}

impl Planned {
    fn fail(id: String, name: String, output: String) -> Self {
        Planned {
            id,
            name,
            run: None,
            fail: Some(ToolOutput {
                output,
                is_error: true,
                diff: None,
                path: None,
            }),
            parallel_safe: false,
        }
    }
}

/// A prepared execution: the resolved tool plus its hook-transformed input.
struct PlannedRun {
    tool: Tool,
    arguments: serde_json::Value,
}

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
            diff: None,
            path: None,
        };
        let _ = sink.send(AgentEvent::ToolExecutionEnd {
            call_id: rid.clone(),
            name: rname.clone(),
            output: out.output.clone(),
            is_error: out.is_error,
            diff: out.diff.clone(),
            path: out.path.clone(),
            duration_ms: None,
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

/// Provider-reported input tokens of the most recent assistant message: the
/// size of the last request, used to decide whether to compact.
fn last_input_tokens(ctx: &[AgentMessage]) -> Option<u64> {
    ctx.iter().rev().find_map(|m| match m {
        AgentMessage::Assistant { usage: Some(u), .. } => Some(u.input_tokens),
        _ => None,
    })
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
