# wcode Agent Harness — Design

Date: 2026-09-05
Status: approved

## Goal

Minimal, extensible Rust agent harness, pi-style: a small event-driven kernel plus a thin coding CLI. Bare minimum at the core, extensible through traits.

## Decisions (from brainstorm)

| Question | Decision |
|---|---|
| rig's role | **B: rig as LLM engine.** We own the loop; rig supplies streaming + tool-schema plumbing. Loop stays provider-agnostic via an injected stream-fn closure. |
| v1 scope | Kernel + minimal CLI (read/bash/edit/write, REPL + one-shot mode). |
| Layout | Workspace: `wcode-harness` (lib) + `wcode-cli` (bin `wcode`). |
| Session | Append-only JSONL, linear v1, torn-line tolerant, `parent_id` baked in for future tree/fork. |
| Providers | OpenAI-compatible endpoints only (rig openai **Completions** API, base-url override): opencode proxy, ollama, vLLM, openrouter. |
| Kernel shape | pi port: closed `AgentEvent` enum, pure-fn loop, `Hooks` trait (default no-ops), `TypedTool` trait + erasure, `StreamFn` seam. |
| Cancellation | `tokio_util::sync::CancellationToken`. No hand-rolled interrupt machinery. |
| Tool execution | Sequential v1. Parallel + mutation queue deferred. |

## Architecture

```
wcode/
├── Cargo.toml                  # workspace (resolver 3, edition 2024)
├── crates/wcode-harness/       # kernel lib
│   └── src/{lib,message,event,tool,hooks,streamfn,loop_,agent,session}.rs
└── crates/wcode-cli/           # bin `wcode`
    └── src/{main,config,repl}.rs, tools/{read,bash,edit,write}.rs
```

Dependency direction: cli → harness → rig. rig is confined to `streamfn.rs`.

## Core types

```rust
pub enum AgentMessage {
    User { content: Vec<ContentBlock> },
    Assistant { content: Vec<ContentBlock>, stop_reason: StopReason, usage: Option<Usage>, model: Option<String> },
    ToolResult { tool_call_id: String, name: String, output: String, is_error: bool },
}
pub enum ContentBlock { Text { text }, Thinking { text }, ToolCall { id, name, arguments: Value } }
pub enum StopReason { Stop, Length, ToolUse, Deferred, Aborted, Error }

pub enum AgentEvent {                       // closed enum, pi AgentEvent port
    AgentStart, TurnStart,
    MessageStart/Update/End,                // Update = full partial assistant snapshot
    ToolExecutionStart/Update/End,
    TurnEnd, AgentEnd,
}
```

## Loop (kernel)

`run_loop(ctx: &mut Vec<AgentMessage>, cfg: LoopConfig, sink: UnboundedSender<AgentEvent>) -> Result<StopReason>`

- Inner loop = turn: drain steering → `hooks.transform_context` → stream via `StreamFn` → fold deltas into partial assistant (emit `MessageUpdate` per delta) → on `Done`: push assistant → if stop in {Error, Aborted} return → execute tool calls sequentially → push tool results → `should_stop_after_turn`? → repeat while tool calls.
- Outer loop = drain follow-up queue; empty → `AgentEnd`, return.
- No max-turns counter. Errors/aborts are values in the stream (pi contract), never panics.
- Steering: `Agent::steer()` enqueues; loop injects as User messages at next turn start. Follow-ups: injected when the loop would otherwise stop.

## Extensibility surface

```rust
#[async_trait]
pub trait TypedTool: Send + Sync + 'static {
    type Args: DeserializeOwned + JsonSchema;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput;
}
// erased into Tool { name(), definition(), execute(Value, ToolContext) } — central serde
// validation; bad args → error ToolOutput, loop continues.

#[async_trait]
pub trait Hooks: Send + Sync {
    async fn before_tool_call(&self, call) -> Option<String> /* block reason */;
    async fn after_tool_call(&self, call, out: &mut ToolOutput);
    async fn transform_context(&self, msgs: &mut Vec<AgentMessage>);
    async fn should_stop_after_turn(&self, ctx) -> bool;
}   // all default no-ops

pub type StreamFn = Arc<dyn Fn(&[AgentMessage], &str, &[ToolDefinition], &LlmOpts) -> LlmStream>;
pub fn rig_stream_fn() -> StreamFn;   // default impl, sole rig consumer
```

## Session

JSONL, one entry per line, append-only (crash-safe; loader skips torn final line):

```rust
pub enum SessionEntry {
    Header { version, id, cwd, created },
    Message { id, parent_id: Option<String>, message: AgentMessage },
    ModelChange { id, model },
    Unknown(Value),        // forward compat: unknown types tolerated
}
```

Location: `~/.local/share/wcode/sessions/<ts>_<uuid>.jsonl`. `--no-session` = in-memory. Compaction, tree/fork: deferred (format ready).

## CLI

- REPL: stdin loop, streaming deltas to stdout, `Ctrl-C` cancels current turn, `/exit /new /model /resume /sessions`.
- `wcode -p "prompt"` one-shot.
- Tools: `read` (offset/limit, line numbers), `bash` (timeout, cancel-kill), `edit` (unique-match replace, pi semantics), `write`. Shared `Arc<Mutex<()>>` between edit/write.
- Config: `~/.config/wcode/config.toml` (`base_url`, `api_key`, `model`) + env `WCODE_BASE_URL`, `WCODE_API_KEY` (fallback `OPENAI_API_KEY`). Precedence env > toml > default.

## Errors & testing

- Kernel: errors are values. `run_loop` `Result` only for loop-level breakage (session I/O).
- Loop tested via fake StreamFn (scripted event streams, no network): plain turn, tool roundtrip, bad-args resilience, block hook, patch hook, abort, stop-after-turn, follow-up, steering.
- Adapter `map_item` unit-tested with real rig item types; live streaming test behind `WCODE_LIVE=1`.
- CLI tools tested against tmpdir; config precedence tested.

## Skipped (YAGNI, add when needed)

Parallel tool exec + mutation queue; compaction; tree/fork sessions (format ready); TUI; permissions; MCP; sub-agents; streaming tool-arg delta rendering.
