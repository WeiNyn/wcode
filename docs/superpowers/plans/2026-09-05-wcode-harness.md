# wcode Agent Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** pi-style minimal agent kernel in Rust (event loop, tools, hooks, JSONL session) + thin CLI with read/bash/edit/write over OpenAI-compatible endpoints.

**Architecture:** Two-crate workspace. `wcode-harness` = kernel (messages, events, loop, tool trait, StreamFn seam with rig adapter, session). `wcode-cli` = binary (config, REPL, built-in tools). Loop is provider-agnostic; rig touches only `streamfn.rs`.

**Tech:** rust 2024, `rig` 0.42 facade (`default-features = false, features = ["reqwest", "rustls"]`), tokio, tokio-util (CancellationToken), async-trait, schemars 1, serde/serde_json, thiserror, uuid, chrono, futures; CLI adds toml, dirs, anyhow.

**Design spec:** `docs/superpowers/specs/2026-09-05-wcode-harness-design.md`

## Global Constraints

- Edition 2024 (workspace root).
- Kernel never panics on model/tool errors — errors are values (pi contract). `run_loop` `Result` only for loop-level breakage (session I/O).
- rig confined to `streamfn.rs`. Kernel types own no rig types.
- Tool errors/validation failures → error `ToolOutput`, loop continues. No max-turns counter.
- Sequential tool execution v1 (parallel + mutation queue deferred).
- OpenAI-compatible wire only (rig openai **Completions** API). Responses API deferred.
- No MCP, no sub-agents, no permissions, no compaction in v1.

---

### Task 0: Scaffold workspace + design docs  ✅ DONE (committed before plan doc)

Root virtual workspace, two crates, stub modules compile, spec + plan committed.

### Task 1: rig spike

Verify pinned dep + streaming API shapes before building the adapter. In `wcode-harness`, write `tests/rig_spike.rs` (`#[ignore]`-gated live part) that:

- constructs the openai client with a custom base URL + api key via the **Completions** API (`completions_api()` path),
- builds a `CompletionRequest` with preamble, chat history, and one `ToolDefinition`,
- calls `CompletionModel::stream()` and asserts (live, `WCODE_LIVE=1`) that items decode as `StreamedAssistantContent` and the terminal carries `StreamFinal { usage, finish_reason }`.

Record the exact constructor names / feature flags that worked in this file's docs. Run: `cargo check --workspace` (must pass) + `cargo test -p wcode-harness --test rig_spike` (ignored without env).

### Task 2: message.rs — message model

**Files:** `crates/wcode-harness/src/message.rs`

Produces (all later tasks consume; exact signatures):

```rust
pub enum Role { User, Assistant, Tool }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Thinking { text: String },
    ToolCall { id: String, name: String, arguments: serde_json::Value },
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason { Stop, Length, ToolUse, Deferred, Aborted, Error }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User { content: Vec<ContentBlock> },
    Assistant {
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    ToolResult {
        tool_call_id: String,
        name: String,
        output: String,
        is_error: bool,
    },
}

impl AgentMessage {
    pub fn user_text(s: impl Into<String>) -> AgentMessage;          // User{content:[Text]}
    pub fn as_text(&self) -> String;                                  // concatenated Text blocks
    pub fn tool_calls(&self) -> Vec<&ContentBlock>;                   // Assistant only
}
```

- Test (TDD): serde roundtrip for each variant + `user_text`/`as_text`; JSON shape: assistant line tags `"role":"assistant"`, blocks tag `"type":"text"` etc.
- Commit: `feat(harness): message model`.

### Task 3: event.rs — event enums

**Files:** `crates/wcode-harness/src/event.rs`

```rust
#[derive(Clone, Debug)]
pub enum LlmStreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCall { id: String, name: String, arguments: serde_json::Value },
    Done { stop_reason: StopReason, usage: Option<Usage> },
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    TurnStart,
    MessageStart { message: AgentMessage },
    MessageUpdate { message: AgentMessage },
    MessageEnd { message: AgentMessage },
    ToolExecutionStart { call_id: String, name: String },
    ToolExecutionUpdate { call_id: String, name: String, partial: String },
    ToolExecutionEnd { call_id: String, name: String, output: String, is_error: bool },
    TurnEnd { message: AgentMessage },
    AgentEnd,
}
```

- Test: serde roundtrip of AgentEvent variants.
- Commit: `feat(harness): event enums`.

### Task 4: tool.rs — tool system

**Files:** `crates/wcode-harness/src/tool.rs`

```rust
pub struct ToolContext {
    pub call_id: String,
    pub name: String,
    pub working_dir: std::path::PathBuf,
    pub cancel: tokio_util::sync::CancellationToken,
    pub events: tokio::sync::mpsc::UnboundedSender<AgentEvent>, // tool may send ToolExecutionUpdate
}

#[derive(Clone, Debug, Default)]
pub struct ToolOutput {
    pub output: String,
    pub is_error: bool,
    pub details: Option<serde_json::Value>,   // session/UI only, never sent to LLM
}

#[async_trait::async_trait]
pub trait TypedTool: Send + Sync + 'static {
    type Args: serde::de::DeserializeOwned + schemars::JsonSchema;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput;
}

#[derive(Clone)]
pub struct Tool(Arc<dyn ErasedToolCore>);   // cloneable handle
pub fn erased<T: TypedTool>(t: T) -> Tool;

impl Tool {
    pub fn name(&self) -> &str;
    pub fn definition(&self) -> rig::completion::ToolDefinition; // {name, description, parameters: schemars::schema_for!(Args)}
    pub async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> ToolOutput;
}
```

Erased wrapper semantics: `execute` → `serde_json::from_value::<T::Args>`; failure → `ToolOutput { is_error: true, output: format!("invalid arguments for tool `{name}`: {e}"), ..Default }`; cancel token already set → `is_error: true, output: "cancelled"` without running.

- Tests: dummy typed tool executes with typed args; garbage args → error output, no panic; `definition()` parameters schema contains the struct's property names.
- Commit: `feat(harness): tool trait + erasure`.

### Task 5: session.rs — JSONL session

**Files:** `crates/wcode-harness/src/session.rs`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Header { version: u32, id: String, cwd: String, created: String }, // RFC3339
    Message { id: String, parent_id: Option<String>, message: AgentMessage },
    ModelChange { id: String, model: String },
    #[serde(other)]
    Unknown,
}

pub struct Session { path: Option<PathBuf>, entries: Vec<SessionEntry> }

impl Session {
    pub fn create(dir: &Path) -> Result<Session>;   // "<ts>_<uuid8>.jsonl", writes Header
    pub fn open(path: &Path) -> Result<Session>;    // skips torn final line silently
    pub fn in_memory() -> Session;                  // --no-session
    pub fn append(&mut self, e: SessionEntry) -> Result<()>; // one line + '\n', flushed
    pub fn entries(&self) -> &[SessionEntry];
    pub fn messages(&self) -> Vec<AgentMessage>;    // clone of message entries
    pub fn build_context(&self) -> Vec<AgentMessage>;
    pub fn path(&self) -> Option<&Path>;
    pub fn model(&self) -> Option<String>;          // latest ModelChange
}
```

- Tests: create→append→open roundtrip; torn final line tolerated (all-but-last loaded); `Unknown` tolerated; `model()` reflects latest ModelChange; in-memory never writes.
- Commit: `feat(harness): JSONL session`.

### Task 6: streamfn.rs — seam + rig adapter

**Files:** `crates/wcode-harness/src/streamfn.rs`

```rust
#[derive(Clone, Debug, Default)]
pub struct LlmOpts {
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub temperature: Option<f64>,
}

pub type LlmStream = std::pin::Pin<Box<dyn futures::Stream<Item = LlmStreamEvent> + Send>>;

pub type StreamFn = std::sync::Arc<
    dyn Fn(&[AgentMessage], &str /*system*/, &[rig::completion::ToolDefinition], &LlmOpts) -> LlmStream
        + Send + Sync,
>;

pub fn rig_stream_fn() -> StreamFn;   // default impl
```

`rig_stream_fn` internals: build openai client per call — api_key (or env fallback) + base_url override, then `.completions_api().completion_model(model)`; map context: preamble = system, chat_history from AgentMessages (user text / assistant blocks / tool results), tools list; `.stream(request)`; spawn-adapt the rig stream into `LlmStream` via **pure item mapper**:

```rust
fn map_item(item: rig::streaming::StreamedAssistantContent) -> Vec<LlmStreamEvent>;
// Text → [TextDelta]; ReasoningDelta → [ThinkingDelta]; Reasoning → [ThinkingDelta(full)] (replacement semantics);
// ToolCallDelta → [] (ignored v1); ToolCall → [ToolCallStart + ToolCall]; Final(StreamFinal) → [Done{stop_reason: finish_reason.map(...).unwrap_or(Stop), usage}];
// Unknown → []; Err item → [Error{message}] then stream ends.
```

FinishReason mapping: Stop→Stop, Length→Length, ToolCall→ToolUse, other/unknown→Stop. If stream yields `Err` → `Error` event, no `Done`.

- Tests: `map_item` per variant (construct rig types directly: `Text::new`, `ToolCallDelta` with `ToolCallDeltaContent::Name/Delta`, `RawStreamingToolCall::new` → `ToolCall`, `StreamFinal::new("test", Usage::default())`); finish-reason mapping incl. ToolCall→ToolUse. Live test `#[ignore]`d behind `WCODE_LIVE=1` calling a real endpoint if configured.
- Commit: `feat(harness): stream-fn seam + rig adapter`.

### Task 7: hooks.rs + loop_.rs — the kernel

**Files:** `crates/wcode-harness/src/hooks.rs`, `crates/wcode-harness/src/loop_.rs`

```rust
#[async_trait::async_trait]
pub trait Hooks: Send + Sync {
    async fn before_tool_call(&self, _call: &ToolCall) -> Option<String> { None } // Some(reason) = blocked
    async fn after_tool_call(&self, _call: &ToolCall, _out: &mut ToolOutput) {}
    async fn transform_context(&self, _msgs: &mut Vec<AgentMessage>) {}
    async fn should_stop_after_turn(&self, _ctx: &[AgentMessage]) -> bool { false }
}
pub struct DefaultHooks;
impl Hooks for DefaultHooks {}  // pure defaults

pub struct ToolCall { pub id: String, pub name: String, pub arguments: serde_json::Value } // loop-local view of ContentBlock::ToolCall

pub struct LoopConfig {
    pub system: String,
    pub tools: Vec<Tool>,
    pub llm: LlmOpts,
    pub stream_fn: StreamFn,
    pub hooks: Arc<dyn Hooks>,
    pub steering: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>,   // drained at turn start
    pub follow_ups: tokio::sync::mpsc::UnboundedReceiver<AgentMessage>, // outer-loop poll
    pub cancel: tokio_util::sync::CancellationToken,
}

pub async fn run_loop(
    ctx: &mut Vec<AgentMessage>,
    cfg: LoopConfig,
    sink: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
) -> Result<StopReason, LoopError>;
pub enum LoopError { Session(std::io::Error) /* reserved; v1: map io errors writing? loop doesn't write — keep enum for future */ }
```

Loop shape:

```
emit AgentStart
outer: loop {
    inner: loop {
        emit TurnStart
        drain steering: for each msg → push to ctx + emit MessageStart/End (User)
        hooks.transform_context(ctx)
        // stream
        emit MessageStart { partial Assistant { .. } }
        let stream = (cfg.stream_fn)(ctx, system, tool_defs, llm)
        select! { biased;
            _ = cfg.cancel.cancelled() => { finish partial as Aborted; emit MessageEnd/TurnEnd/AgentEnd; return Ok(Aborted) }
            item = stream.next() => fold into partial:
                TextDelta → append Text block (current or new) + emit MessageUpdate
                ThinkingDelta → append Thinking block + emit MessageUpdate
                ToolCallStart → note (partial arg buffer), no event (v1)
                ToolCall → append ContentBlock::ToolCall + emit MessageUpdate
                Done → capture stop_reason + usage
                Error → capture error
        }
        emit MessageEnd { final assistant }; push assistant into ctx
        emit TurnEnd
        match captured:
            Error → emit AgentEnd; return Ok(Error)   // stop_reason::Error
            Aborted → emit AgentEnd; return Ok(Aborted)
            none w/ tool calls → execute tools, continue inner
            no tool calls → break inner
        // tool execution (sequential)
        for call in assistant tool calls:
            hooks.before_tool_call → Some(reason) ⇒ ToolOutput{is_error, output: format!("blocked: {reason}")} (skip execute)
            find tool by name; missing ⇒ error output "unknown tool: {name}"
            emit ToolExecutionStart → tool.execute(args, ctx w/ call_id, name, events=sink.clone()) → hooks.after_tool_call(&mut out)
            emit ToolExecutionEnd; push ToolResult AgentMessage
        if hooks.should_stop_after_turn(ctx).await → emit AgentEnd; return Ok(Stop)
    } // end inner
    // outer: follow-ups
    match follow_ups.try_recv() {
        Ok(m) => { push m to ctx; emit MessageStart/End(User); continue outer }
        Err(Empty | Disconnected) => { emit AgentEnd; return Ok(Stop) }
    }
}
```

Rules: tool execution sequential; each ToolResult gets `tool_call_id` = call.id; a Done with stop_reason Error/Aborted ends the run even if tool calls exist (pi contract); MessageUpdate carries the **full current partial snapshot** (pi contract), CLI renders deltas by diffing.

- Integration tests (`crates/wcode-harness/tests/loop_tests.rs`) — fake StreamFn scripting from a shared queue (Arc<Mutex<VecDeque<Vec<LlmStreamEvent>>>> popping per call, recording received `(ctx_snapshot, system, tools)` per call):
  1. `plain_text_turn` — one text turn: event sequence exactly AgentStart, TurnStart, MessageStart, ≥1 MessageUpdate, MessageEnd, TurnEnd, AgentEnd; ctx ends [User, Assistant(Stop)]
  2. `tool_call_roundtrip` — stream1 returns ToolCall(dummy tool args), stream2 returns text; recording tool asserts typed args; ctx has ToolResult; second stream call saw the tool result in history
  3. `bad_args_resilient` — stream1 ToolCall with args not matching schema → error ToolResult ("invalid arguments"), loop continues to stream2
  4. `before_tool_call_blocks` — hook returns Some("nope") → ToolExecutionEnd is_error, output contains "nope", tool NOT executed, loop continues
  5. `after_tool_call_patches` — hook mutates output → ToolExecutionEnd + ToolResult carry patched text
  6. `abort_mid_stream` — cancel token set while fake stream hangs → returns Ok(Aborted), partial assistant persisted with stop_reason Aborted
  7. `should_stop_after_turn` — hook returns true after turn 1 → Ok(Stop), only 1 stream call
  8. `follow_up_runs_next_turn` — follow_up sent before run → second outer turn executes (2 stream calls)
- Commit: `feat(harness): agent loop + hooks`.

### Task 8: agent.rs — stateful wrapper

**Files:** `crates/wcode-harness/src/agent.rs`

```rust
pub struct AgentConfig {
    pub system: String,
    pub tools: Vec<Tool>,
    pub llm: LlmOpts,
    pub stream_fn: StreamFn,
    pub hooks: Arc<dyn Hooks>,
    pub session: Option<Session>,   // None = no persistence
}

pub struct Agent {
    // owns config parts, ctx: Vec<AgentMessage>, steer_tx, follow_tx (UnboundedSender), cancel token
}
impl Agent {
    pub fn new(cfg: AgentConfig) -> Agent;
    pub fn steer(&self, m: AgentMessage);       // injected at next turn start
    pub fn follow_up(&self, m: AgentMessage);   // runs after loop would stop
    pub fn cancel(&self);
    pub fn cancel_token(&self) -> CancellationToken; // for ctrl-c wiring
    pub async fn run(&mut self, user_text: &str, sink: UnboundedSender<AgentEvent>) -> Result<StopReason, LoopError>;
    pub fn messages(&self) -> &[AgentMessage];
    pub fn session_path(&self) -> Option<&Path>;
}
```

`run`: push `User` message (+ session append) → build LoopConfig (fresh receivers via `UnboundedReceiverStream`-less trick: keep `steer_rx`/`follow_rx` in Agent, `take()` them into LoopConfig and restore after run) → `run_loop` → on Stop: drain any residual follow-ups? (no: leave queued for next run) → return stop reason. Events: run_loop pushes into `sink`; Agent additionally appends Assistant/ToolResult messages to session on MessageEnd/ToolExecutionEnd? Simpler: Agent appends to session from `self.messages()` after run completes (single pass over new messages). Choose: after run, iterate ctx slice [len_before..] and append `SessionEntry::Message` for each.

- Tests: steer during a blocked (channel-waiting) tool → next turn's stream call sees the steer message before assistant; cancel → Ok(Aborted); session file contains all messages after run.
- Commit: `feat(harness): stateful agent wrapper`.

### Task 9: CLI — config + built-in tools

**Files:** `crates/wcode-cli/src/config.rs`, `crates/wcode-cli/src/tools/{mod,read,bash,edit,write}.rs`

```rust
// config.rs
#[derive(Debug, Deserialize)]
pub struct FileConfig { pub base_url: Option<String>, pub api_key: Option<String>, pub model: Option<String> }
pub struct Config { pub base_url: Option<String>, pub api_key: Option<String>, pub model: String }
impl Config {
    pub fn load() -> Config;   // env WCODE_BASE_URL/WCODE_API_KEY (fallback OPENAI_API_KEY) > toml > defaults (model: "gpt-4o-mini"? no: model REQUIRED; error if missing → prompt in main)
    pub fn default_path() -> Option<PathBuf>; // ~/.config/wcode/config.toml
}
```

Tools (all: `#[derive(Deserialize, JsonSchema)]` Args structs, `impl TypedTool`, shared `Arc<Mutex<()>>` between edit/write; paths resolve relative to `ctx.working_dir`; `// ponytail: full mutation queue when parallel exec lands`):

- `read`: `{ path, offset: Option<u64> /*1-based line*/, limit: Option<u64> }` → `cat -n`-style numbered lines; missing file → error output.
- `bash`: `{ command, timeout_secs: Option<u64> = 30 }` → tokio process, stdout+stderr captured (stderr appended, labeled), non-zero exit → output includes exit code (not is_error? make non-zero = is_error false, exit code visible; pi keeps it in-band. Choose: is_error = false always, output states exit code) — actually is_error=true on non-zero helps model notice; decide: **is_error = (exit != 0)**. Cancel token → kill child.
- `edit`: `{ path, old_string, new_string, replace_all: Option<bool> }` → 0 matches → error "not found"; >1 && !replace_all → error "matches N times, set replace_all"; else replace.
- `write`: `{ path, content }` → mkdir parent + write; returns bytes written.

- Tests (tmpdir): read w/ offset+limit; edit unique / not-found / multi-fail / replace_all; write creates parents; bash echo; bash timeout kills (sleep 5, timeout 1); config precedence env>toml.
- Commit: `feat(cli): config + built-in tools`.

### Task 10: CLI — repl + main + README

**Files:** `crates/wcode-cli/src/repl.rs`, `crates/wcode-cli/src/main.rs`, `README.md`

- `main.rs`: manual arg parse: `wcode [-p "<prompt>"] [--resume [path]] [--no-session] [--model <id>] [--base-url <url>]`; builds Config → LlmOpts → rig_stream_fn → tools → Agent → one-shot or REPL.
- `repl.rs`:
  - stdin line loop; prompt `❯ `; blank line = skip.
  - `/exit`, `/new`, `/model <id>` (session ModelChange + LlmOpts update), `/resume [path]` (list sessions in dir, pick latest if no arg), `/sessions` (list).
  - Event printing (spawned task over sink receiver): `MessageUpdate` → diff vs last printed length: print TextDelta chunks to stdout (flush), Thinking to stderr dimmed (`\x1b[2m`); `ToolExecutionStart` → `\r⚙ {name}` dimmed; `ToolExecutionUpdate` → append partial inline; `ToolExecutionEnd` → print ` ✓`/` ✗` + first line of output (dimmed, truncated 120 chars).
  - Ctrl-C: first press → `agent.cancel()` (turn aborts, stays in REPL); while idle → exit. Install via `tokio::signal::ctrl_c()` task.
  - Session: AgentConfig.session = Session::create(default_dir) unless --no-session.
- Tests: `/command` parse fn (table test); event-printer diff logic as pure fn (given last_len + update → emitted string). REPL interaction itself untested (manual).
- README: what it is, install, config example, tools list, philosophy (pi-like: no MCP/subagents/permissions by design; hooks for that).
- Commit: `feat(cli): repl + main + readme`.

### Task 11: End-to-end smoke + polish

- `cargo clippy --workspace -- -D warnings` clean.
- `cargo test --workspace` green.
- Optional live e2e behind `WCODE_LIVE=1`: `wcode -p "read Cargo.toml and tell me the version"` against configured endpoint (docs only if no endpoint available).
- Commit: `chore: clippy clean + final polish`.

---

## Skipped (YAGNI, add when needed)

- Parallel tool exec + pi-style file mutation queue
- Compaction entry type (format-ready)
- Tree/fork sessions (`parent_id` already in format)
- TUI (ratatui), permissions, MCP, sub-agents
- Streaming tool-arg delta rendering (ToolCallDelta ignored in adapter v1)
