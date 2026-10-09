# Sketch — VS Code extension (P1) — NEW `editors/vscode/`

**Status:** interface sketch, review-only. No working logic. Fillable as-is. A new
TypeScript tree (`editors/vscode/`), a second *client* of the wcode kernel — never a
second kernel (`docs/vscode-extension-plan.md` §1, `:uJL11`).

**Design:** `docs/vscode-extension-plan.md` §3.2 (the client), §3.3 (diffs), §3.5
(team-first) — the lock. The transport is P0's `wcode serve --stdio`
(`docs/sketches/serve-stdio.md`, implemented).

**Grounding (fresh anchors — the wire is the spec, read from attributes):**

- **`Request`** — `#[serde(tag = "type", rename_all = "snake_case")]`
  `crates/wcode-harness/src/protocol.rs:YK8j1`, enum `:Pfw8B`; `#[serde(other)] Unknown`
  `:hMT4F`; aliases `steer`→`Interrupt` `:auNYx`, `follow_up`→`Wake` `:d7q72`.
- **`AgentEvent`** — `#[serde(tag = "type", rename_all = "snake_case")]`
  `crates/wcode-harness/src/event.rs:YK8j1`, enum `:JJqEF`; **no `#[serde(other)]`
  catch-all** (the doc says it "lands with the actor" — `protocol.rs:NoynW`).
- **`Frame<P>`** — `crates/wcode-harness/src/protocol.rs:nDJoL`; body `#[serde(flatten)]`
  `:AcELi`; `reply_to`/`sender` `skip_serializing_if = "Option::is_none"` `:yH2JZ`;
  `SessionId` is `#[serde(transparent)]` (`:PS1bm`) → a bare string.
- **Nested** — `AgentMessage` `#[serde(tag = "role")]` `message.rs:aUoHE`; `ContentBlock`
  `#[serde(tag = "type")]` `:Q06qW`; `StopReason` `:rjiip`; `Usage` `:3Jenx`;
  `TodoItem`/`TodoStatus` `event.rs:9FIi9`/`:kivQt`; `SessionInfo` `protocol.rs:lStv9`;
  `MemberState` `:qqtfh`.
- **The line cap** — `MAX_FRAME_BYTES = 16 * 1024 * 1024` `crates/wcode-protocol/src/frame.rs:gbOLr`;
  one JSON object per line, flush per frame `:EvQuH`; blank lines skipped `:8BPV6`.
- **The TUI is the model to mirror** — `Block` enum `crates/wcode-tui/src/app.rs:k52Iz`,
  `Tool` struct `:CEUJo`, the reducer `App::apply` `:hSHo0` (its
  `MessageUpdate` arm `:8b2AA`; `ToolExecutionUpdate` append `:G4bJc`; `ToolExecutionEnd`
  `:KGPrH`), `seed_history` `:hTSO6`, `MemberState`/`TeamState` `protocol.rs:qqtfh` /
  `crates/wcode-tui/src/lib.rs:y5W8h`.
- **The tree is not swept into Cargo** — `members = [...]` is explicit `Cargo.toml:DKWD1`.
  **`.gitignore` is only `/target`** `.gitignore:zF7FU` → P1 must add `node_modules/`,
  `out/`, `*.vsix`.

---

## 1. The wire — the spec the TS client must match (`crates/wcode-harness`)

**A wrong tag name here is the most expensive P1 bug.** These are copied from the serde
attributes and the round-trip tests, not from memory.

### The envelope

```ts
// Frame<P> — protocol.rs:nDJoL. Body is FLATTENED: {headers, ...body}. One per line.
interface Frame {
  v: number;              // PROTOCOL_VERSION = 1 (protocol.rs:g7GFM)
  id: number;             // u64; a request's id, echoed in reply_to; 0 for pushed events
  reply_to?: number;      // present only on a direct reply (protocol.rs:yH2JZ)
  session: string;        // SessionId — a bare string (transparent, :PS1bm)
  sender?: string;        // present only for A2A (an "agent:<id>" peer)
  type: string;           // the discriminant (below) — flattened into the frame
  [k: string]: unknown;   // the variant's fields
}
```

`id` is `u64` — **TS `number` loses precision above 2^53**. A client that mints ids
monotonically from 1 is safe; a 64-bit counter is not. Keep the client's ids small.

### Request tags — what the client may send (`protocol.rs:Pfw8B`, `:YK8j1`)

| `type` | body | anchor |
|---|---|---|
| `submit` | `{ text: string }` | `:j7Z9C` |
| `notify` | `{ content: string }` | `:MPerU` |
| `interrupt` | `{ content: string }` (alias `steer`) | `:auNYx` |
| `wake` | `{ content: string }` (alias `follow_up`) | `:d7q72` |
| `cancel` | — | `:HzHCr` |
| `set_model` | `{ model: string }` | `:JfA4s` |
| `set_effort` | `{ effort: string \| null }` | `:e8RnA` |
| `compact` | `{ instructions: string \| null }` | `:Upo2N` |
| `set_plan_mode` | `{ on: boolean }` | `:8tGXF` |
| `side_ask` | `{ text: string }` | `:Le2j8` |
| `get_history` | — | `:Zpi4S` |
| `status` | — | `:UMY8l` |
| `list_sessions` | — | `:2kphM` |
| `define` | `{ name?, model?, role?, tools?: string[], base_url?, api_key?, read_only?, effort? }` | `:j5OKE` |

```ts
// Derived union (each variant's tag cited above). `null` is meaningful: SetEffort.effort
// and Compact.instructions have NO skip_serializing_if, so wire None is null; OMITTING the
// key also deserializes to None (Option defaults) — either way clears it.
type Request =
  | { type: "submit"; text: string }
  | { type: "notify"; content: string }
  | { type: "interrupt"; content: string }        // send "interrupt"; "steer" also accepted
  | { type: "wake"; content: string }             // send "wake"; "follow_up" also accepted
  | { type: "cancel" }
  | { type: "set_model"; model: string }
  | { type: "set_effort"; effort: string | null }
  | { type: "compact"; instructions: string | null }
  | { type: "set_plan_mode"; on: boolean }
  | { type: "side_ask"; text: string }
  | { type: "get_history" }
  | { type: "status" }
  | { type: "list_sessions" }
  | { type: "define"; name?: string; model?: string; role?: string; tools?: string[];
      base_url?: string; api_key?: string; read_only?: boolean; effort?: string };
```

The client sends only the subset the panel uses: `submit`, `cancel`, `interrupt`,
`set_plan_mode`, `get_history`, `list_sessions` — and `define` later (P3). The rest exist.

### AgentEvent tags — what the client must parse (`event.rs:JJqEF`, `:YK8j1`)

| `type` | body | anchor |
|---|---|---|
| `agent_start` | — | `:9IhLD` |
| `turn_start` | — | `:AvuKo` |
| `message_start` | `{ message: AgentMessage }` | `:yyQhp` |
| `message_update` | `{ message: AgentMessage }` (**whole message, replacement**) | `:hGXq5` |
| `message_end` | `{ message: AgentMessage }` | `:Ws12l` |
| `tool_execution_start` | `{ call_id, name }` | `:3HX3O` |
| `tool_execution_update` | `{ call_id, name, partial }` (**append**) | `:Zp4NU` |
| `tool_execution_end` | `{ call_id, name, output, is_error, diff?, path?, duration_ms? }` | `:TaYKf` |
| `turn_end` | `{ message: AgentMessage }` | `:GWLSS` |
| `compaction` | `{ summarized: number, kept: number }` | `:AFtlz` |
| `compaction_skipped` | `{ reason: string }` | `:WSx9t` |
| `retrying` | `{ attempt: number, max: number, reason: string }` | `:97oPA` |
| `message_received` | `{ from: string, content: string }` | `:N2lrx` |
| `error` | `{ message: string }` | `:cKEOt` |
| `ack` | — | `:2huak` |
| `stopped` | `{ stop_reason: StopReason }` | `:Skrtl` |
| `side_answer` | `{ text: string, usage: Usage \| null }` | `:1K8Ls` |
| `history` | `{ messages: AgentMessage[] }` | `:DvdZX` |
| `status` | `{ last_assistant_text: string \| null }` | `:yW95W` |
| `sessions` | `{ sessions: SessionInfo[] }` | `:30IMH` |
| `spawned` | `{ worker: string }` (SessionId — NOT `id`) | `:BIyvu` |
| `todo` | `{ todos: TodoItem[] }` (NOT `id`) | `:43Kiy` |
| `agent_end` | — | `:S7pc1` |

**`spawned` uses `worker` and `todo` uses `todos`, not `id`** — the envelope already
carries a top-level `id`, and a flattened duplicate would not deserialize (`:www93`,
`:twS1K`). Getting these two wrong is the second-most-expensive bug.

```ts
// Derived union (tags cited above). NOT exhaustive in the client's `switch`: AgentEvent has
// no serde `other` catch-all (event.rs:JJqEF), so a future tag is an UNKNOWN OBJECT, not a
// Rust error — the client must IGNORE unknown `type`s, never throw. (See §8.)
type AgentEvent =
  | { type: "agent_start" } | { type: "turn_start" }
  | { type: "message_start" | "message_update" | "message_end"; message: AgentMessage }
  | { type: "tool_execution_start"; call_id: string; name: string }
  | { type: "tool_execution_update"; call_id: string; name: string; partial: string }
  | { type: "tool_execution_end"; call_id: string; name: string; output: string;
      is_error: boolean; diff?: string; path?: string; duration_ms?: number }
  | { type: "turn_end"; message: AgentMessage }
  | { type: "compaction"; summarized: number; kept: number }
  | { type: "compaction_skipped"; reason: string }
  | { type: "retrying"; attempt: number; max: number; reason: string }
  | { type: "message_received"; from: string; content: string }
  | { type: "error"; message: string }
  | { type: "ack" }
  | { type: "stopped"; stop_reason: StopReason }
  | { type: "side_answer"; text: string; usage: Usage | null }
  | { type: "history"; messages: AgentMessage[] }
  | { type: "status"; last_assistant_text: string | null }
  | { type: "sessions"; sessions: SessionInfo[] }
  | { type: "spawned"; worker: string }
  | { type: "todo"; todos: TodoItem[] }
  | { type: "agent_end" };
```

### Nested types (all from `crates/wcode-harness/src/message.rs`)

```ts
// AgentMessage — #[serde(tag = "role", rename_all = "snake_case")]  message.rs:aUoHE
type AgentMessage =
  | { role: "user"; content: ContentBlock[] }
  | { role: "assistant"; content: ContentBlock[]; stop_reason: StopReason;
      usage?: Usage; model?: string }              // usage/model skip when absent (:95kss)
  | { role: "tool_result"; tool_call_id: string; name: string; output: string; is_error: boolean };

// ContentBlock — #[serde(tag = "type", rename_all = "snake_case")]  message.rs:Q06qW
type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | { type: "tool_call"; id: string; name: string; arguments: unknown };

// StopReason — message.rs:rjiip  (max_turns included)
type StopReason = "stop" | "length" | "tool_use" | "aborted" | "error" | "max_turns";

// Usage — message.rs:3Jenx  (cache_* skip when absent)
interface Usage { input_tokens: number; output_tokens: number;
                  cache_read_tokens?: number; cache_write_tokens?: number; }

// TodoItem / TodoStatus — event.rs:9FIi9 / :kivQt
interface TodoItem { content: string; status: TodoStatus }
type TodoStatus = "pending" | "in_progress" | "completed";

// SessionInfo / MemberState — protocol.rs:lStv9 / :qqtfh
interface SessionInfo { id: string; model?: string; state: MemberState }
type MemberState = "idle" | "running" | "done" | "failed";
```

**The cap rule the splitter must enforce:** drop any line longer than 16 MiB
(`frame.rs:gbOLr`) rather than buffering it (the Rust side errors `InvalidData` at
`:r1BBw`); a client that buffers an over-cap line is a memory bug.

## 2. The transport — `WcodeSession` (`src/session.ts`)

**Session addressing — a hard rule (live-verified).** Read the **seeded `sessions` push** on
connect and address every subsequent request to the **root session's real id**. Do not use the
`"remote"` placeholder: it happens to resolve on a *single-session* server (the `live.is_empty()`
fallback, `server.rs:JVP1s`) but returns `unknown session remote` as soon as a roster exists
(a team, or `--agents`) — a silent failure that shows up as an empty chat. **Also keep stdin
open for the session's life**: a client that closes it can lose the replies still in flight
(observed: 3 of 4 dropped in one piped run).

```ts
// Spawns `wcode serve --stdio` (plan §3.2), frames NDJSON over STDOUT, and owns the
// reply correlation. NO `vscode` import — it takes a logger; the extension host wires it.
interface WcodeSession {
  readonly state: SessionState;                       // the FSM below
  send(request: Request): number;                     // mints id, writes one line, returns it
  ask(request: Request, timeoutMs?: number): Promise<AgentEvent>; // correlates via reply_to
  on(ev: "event", cb: (e: AgentEvent, frame: Frame) => void): Disposable;
  on(ev: "state", cb: (s: SessionState) => void): Disposable;
  on(ev: "stderr", cb: (line: string) => void): Disposable;   // diagnostics — NEVER parsed
  stop(): Promise<void>;                              // kill the child; resolve on exit
}

type SessionState = "stopped" | "starting" | "ready" | "crashed";
```

**Lifecycle FSM + restart policy** (plan §3.2, `:TdR4M`: "start on first use, kill on
deactivate, restart on crash with a visible state"):

```
stopped ──start()──▶ starting ──spawn ok + first frame or 250ms──▶ ready
   ▲                    │                                            │
   │ stop() / deactivate│ spawn error / exit≠0                        │ exit / stream error
   └────────────────────┴──────────── crashed ◀──────────────────────┘
```

- `crashed` → the extension shows a status item carrying the **stderr tail**, and retries at
  most **3** times with backoff **500ms → 1s → 2s**, then stops and requires a manual
  `Wcode: Restart`. Never a silent respawn loop.
- `stop()` on `deactivate`: `child.kill()` then await `'exit'` (plan `:HaRIx`).

**Binary location** (plan §3.2, `:Nyvwk`): setting `wcode.path` → `PATH` (`which wcode`)
→ a clear error surfaced to the user. Not silent.

**The NDJSON splitter — the one line-oriented bug that bites.** stdout arrives in arbitrary
chunks: a frame can be split across two `data` events, and two frames can ride one chunk.
The contract:

```ts
// STDOUT ONLY. stderr is diagnostics and is NEVER fed to the splitter (see §8).
// state: a carry buffer of the bytes since the last '\n'.
function feed(buf: string, chunk: string): { frames: Frame[]; rest: string };
```

Rules, matching `read_frame`'s framing (`frame.rs:8BPV6`): split on `\n`; keep the trailing
partial in `rest`; `JSON.parse` each complete **non-blank, trimmed** line (`:t40WB`/`:pnZYp`);
skip blanks (`:hT749`); drop (do not buffer) a line over `MAX_FRAME_BYTES` (`frame.rs:gbOLr`).
One frame is one JSON object — a line that parses to a non-object, or throws, is a protocol
error **for that line**.
**Deliberate divergence, owned:** `read_frame` *aborts the connection* on an over-cap
(`:r1BBw`) or unparseable (`:gRxYo`) line; the client **drops the line and continues**, so one
bad frame cannot kill a live session. Do not describe this as "mirroring".

**Correlation.** `send` mints `id` (per-connection, monotonic, from 1 — keep it small, `id`
is `u64`); a `Map<number, {resolve, reject, timer}>` keyed by `id` resolves on an inbound
frame whose `reply_to === id` (`protocol.rs:yH2JZ`). A streamed event has `reply_to` absent
and is emitted on the `event` channel.

**Logging discipline.** The child's **stderr** carries `provider: …`, warnings, and P0's
moved `serving …`/`team: …` lines (see `docs/sketches/serve-stdio.md` §5). The client routes
stderr to an output channel; it must **never** `JSON.parse` stderr. stdout is the frame
stream — the same rule the Rust side encodes (`main.rs:cBOtn`).

## 3. The reducer — the discipline (plan §3.2, `:Xsoso`/`:VmiFd`/`:INTQK`)

**Pure:** no I/O, **no `vscode` import**, a plain `(state, event) -> state`. A Node test
drives it with captured JSON (§7). It mirrors `wcode-tui`'s `App::apply` (`app.rs:hSHo0`).

```ts
// src/reducer.ts — no imports outside ./messages (the wire types) and ./types.
interface ViewState {
  transcript: Block[];            // committed blocks + one live assistant block (below)
  members: SessionMember[];       // the roster, root first (from `sessions`)
  todos: TodoItem[];              // from `todo`
  status: { running: boolean; lastError?: string; stopReason?: StopReason;
            planMode: boolean; contextUsed?: number; model?: string };
}

interface Block {                 // mirrors app.rs:k52Iz + the live block
  kind: "user" | "assistant" | "notice" | "error" | "btw" | "tool";
  text?: string;                  // user/notice/error/btw
  content?: ContentBlock[];       // assistant (text + thinking interleaved, app.rs:G1Q7F)
  tool?: ToolBlock;               // tool
  live?: boolean;                 // the streaming assistant block (replaced, not appended)
}

interface ToolBlock {             // mirrors app.rs:CEUJo
  callId: string;                 // the reducer keys by call_id (better than the TUI's last-block)
  name: string; output: string; done: boolean; isError: boolean;
  diff?: string; path?: string; durationMs?: number;
}

interface SessionMember { id: string; label: string; model?: string; state: MemberState; isRoot: boolean }

function reduce(state: ViewState, event: AgentEvent): ViewState;   // the whole surface
```

**Arms, mirroring `apply` (`app.rs:hSHo0`) — with the differences called out:**

- `message_start` → open a `live` assistant block **only** for `role: "assistant"`
  (the TUI ignores a user echo, `app.rs:akFx3`).
- `message_update` → **replace** the live block's `content` with `event.message` (whole
  message; see §8) — do NOT append.
- `message_end` → commit the block (`live = false`); skip an empty assistant
  (`app.rs:XUoLe`).
- `tool_execution_start` → push a `ToolBlock { callId, name, done: false }`.
- `tool_execution_update` → find by `callId`, **append** `partial` to `output`
  (matches `app.rs:G4bJc`, but keyed by `call_id`, not last-block).
- `tool_execution_end` → find by `callId`: set `output` = `event.output` when non-empty
  (else keep the streamed partial, `app.rs:PMpNp`); `done = true`; `isError`; `diff`,
  `path`, `durationMs`.
- `agent_start` → `status.running = true`; clear `lastError` (a run failure is sticky
  until the next start, `app.rs:YUFcZ`).
- `agent_end` → `status.running = false`.
- `error` → push an `error` block; if `running`, set `lastError`
  (an idle reply error does not mark the run failed, `app.rs:Yq9bw`).
- `stopped` → record `stop_reason` (the run's why).
- `todo` → replace `state.todos`.
- `sessions` → replace `state.members` (root first). **The TUI does NOT do this in `apply`**
  — it builds surfaces from the roster at the driver level (`main.rs:SYTRC`); the extension's
  roster is view state, so the reducer folds it here.
- `spawned` → add the new member (`worker`).
- `message_received` → push a block `from: event.from` (inter-agent traffic, §3.5).
- `history` → **seed** the transcript (a second entry point, `seedFromHistory(state, messages)`
  — mirrors `app.rs:hTSO6`; the TUI seeds at connect, not in `apply`).
- `turn_end` → read `usage` into `status.contextUsed` (`app.rs:gr1ZK`).
- everything else (`turn_start`, `ack`, `compaction*`, `retrying`, `side_answer`, `status`)
  → a `notice` block or no-op, as `apply` does.

## 4. The webview contract (`src/panel.ts` + `media/`)

Host ⇄ webview over `postMessage`; the webview is a **pure renderer of `ViewState`**.

```ts
// host → webview
type ToWebview =
  | { kind: "state"; state: ViewState }              // the primitive: a full snapshot per reduce
  | { kind: "append"; block: Block }                 // optional fast path
  | { kind: "diff"; callId: string; path: string; diff: string }; // a diff became available

// webview → host
type FromWebview =
  | { kind: "submit"; text: string }                 // → Request::Submit
  | { kind: "cancel" }                               // → Request::Cancel
  | { kind: "steer"; text: string }                  // → Request::Interrupt (soft, no cancel)
  | { kind: "open-diff"; callId: string }            // → vscode.diff (plan §3.3)
  | { kind: "reveal-file"; path: string; line?: number };
```

**Security posture** (plan §3.2, `:ZoQAa` — the host owns the process and the protocol):

- `enableScripts: true` (the chat surface needs it) — nothing else.
- CSP: `default-src 'none'; img-src ${webview.cspSource}; style-src ${webview.cspSource};
  script-src ${webview.cspSource};` — **no remote origins**.
- `localResourceRoots: [media/]` — only bundled assets load.
- `retainContextWhenHidden: true` — the transcript survives a tab switch.
- **No remote resources** — the repo's "zero external requests" ethos (`style.css:V3G80`):
  no CDN, no webfont, no remote image. Diffs open on request, never automatically
  (plan §3.3, `:xW4PJ`).

## 5. Layout (`editors/vscode/`)

```
editors/vscode/
  package.json          main: ./out/extension.js
                        engines.vscode: <conservative — see §8>
                        activationEvents: ["onCommand:wcode.start", "onCommand:wcode.stop"]
                        contributes.commands: wcode.start, wcode.stop, wcode.restart, wcode.openDiff
                        contributes.configuration: wcode.path (string, "the wcode binary")
  tsconfig.json         strict, module: commonjs (esbuild bundles), outDir: out/
  esbuild.mjs           bundle src/extension.ts -> out/extension.js (platform: node, cjs)
  src/extension.ts      activate(): create the panel + WcodeSession, wire postMessage
  src/messages.ts       the wire types of §1 (the ONE place tags live)
  src/session.ts        WcodeSession + the NDJSON splitter (§2)
  src/reducer.ts        pure reduce / seedFromHistory (§3)
  src/panel.ts          WebviewPanel, CSP, localResourceRoots (§4)
  src/diff.ts           P2 stub (§6)
  media/chat.css  media/chat.js  media/base.css   (bundled; zero remote assets)
  .vscodeignore
```

**`package.json` — only the manifest bits that matter:** `main`, `engines.vscode`,
`activationEvents`, `contributes.commands`, `contributes.configuration` (`wcode.path`).
`engines.vscode` is a §8 decision (conservative vs the native-chat-API era); the plan
chooses a **webview, not the native chat API** (`:rT6jP`), so the floor need not be the
chat-API release.

**Not swept into Cargo:** the workspace lists its four members explicitly
(`Cargo.toml:DKWD1`), so `editors/` is a sibling tree Cargo ignores.

**`.gitignore` additions** (today only `/target`, `.gitignore:zF7FU`): append
`node_modules/`, `out/`, `*.vsix`.

## 6. P2 / P3 stubs (signatures only — NOT designed here)

```ts
// P2 — diffs (plan §3.3, reverse-apply is the chosen source, :hzpIw):
class WcodeDiffProvider implements vscode.TextDocumentContentProvider {
  // scheme "wcode-diff:"; provideTextDocumentContent(uri): string
  // the uri encodes {path, side: "before"|"after", callId}.
}
/** Reconstruct the pre-image: reverse-apply the unified diff to the CURRENT file text. */
function reverseApply(currentText: string, unifiedDiff: string): string;   // -> beforeText

// P2/P3 — a diff request becomes a `vscode.diff` of two `wcode-diff:` documents:
//   left  = reverseApply(readFile(path), diff)
//   right = readFile(path)         (vscode.diff(leftUri, rightUri, `${base} (wcode)`))

// P3 — the roster item the sidebar renders (from SessionInfo, protocol.rs:lStv9):
interface RosterItem { id: string; label: string; model?: string; state: MemberState;
                       isRoot: boolean; liveAction?: string }
```

## 7. Tests & fixtures

**Reducer tests driven by REAL captured `AgentEvent` JSON** — a plain Node test
(`tsx`/`node --test`), no VS Code:

```ts
// test/reducer.test.ts
it("submit round-trips to a live assistant block", () => {
  // fixtures/*.ndjson captured from `wcode serve --stdio` (§ below).
  todo!();
});
it("message_update REPLACES the live content, never appends", () => { todo!(); });
it("tool_execution_update appends; end replaces a non-empty output", () => { todo!(); });
it("an unknown event type is ignored, not thrown", () => { todo!(); });   // forward-compat
it("sessions folds the roster, root first", () => { todo!(); });

// test/splitter.test.ts — the line-oriented bugs:
it("a frame split across two chunks reassembles", () => { todo!(); });
it("two frames in one chunk split into two", () => { todo!(); });
it("a blank line is skipped", () => { todo!(); });
it("a line over MAX_FRAME_BYTES is dropped, not buffered", () => { todo!(); });
```

**Fixture source — P0's smoke test is the truth** (`docs/sketches/serve-stdio.md` §6). The
model-free first fixture:

```sh
# `printf |` closes stdin at once, and a closing client can lose the correlated reply
# (observed: 3 of 4 replies dropped in one run). Fine for one frame — for anything more,
# hold stdin open (spawn, read the seed, write, read) instead of piping.
printf '%s\n' '{"v":1,"id":1,"session":"remote","type":"list_sessions"}' | wcode serve --stdio
# yields a seeded `sessions` push (id 0, reply_to absent) and, normally, a correlated
# reply (reply_to: 1) — TWO frames, one line each: the perfect first fixture.
```

A fuller capture: run a real `submit` against a local endpoint
(`--base-url http://localhost:11434/v1`) and tee stdout to a file —
`wcode serve --stdio | tee fixtures/submit.ndjson` — which yields `message_start` /
`message_update` / `tool_execution_*` / `message_end` / `agent_end` in order. Commit the
captured files as fixtures so the reducer's tests run with no model and no VS Code.

## 8. Friction (do not smooth over)

- **One client, by construction.** `--stdio` serves exactly one connection
  (`docs/sketches/serve-stdio.md` §4); a TUI **and** the editor cannot share a session. A
  shared session needs `--socket` (plan §3.1, `:B1hRD`). The extension owns its child.
- **Markdown rendering — decided: one bundled dependency.** The premise first written here was
  false: `wcode-tui` does **not** hand-roll its markdown — it uses `pulldown-cmark` (CommonMark +
  GFM tables/strikethrough/tasklists + footnotes) with `syntect` for fenced highlighting
  (`crates/wcode-tui/Cargo.toml:0qScI`, `crates/wcode-tui/src/markdown.rs:RcyRD`). Only the
  *site* is dependency-free (`style.css:V3G80`), and that rule is about **runtime** requests, not
  build deps. (`docs/tui-design.md:232`'s "hand-rolled" line is stale — do not cite it.)
  **Ruling: `markdown-it` with `html: false`**, bundled by esbuild. `html: false` escapes raw
  HTML rather than passing it through, so it keeps the injection safety that motivated
  hand-rolling, while a real parser covers the surface the TUI already renders. **W009 closed the
  `syntect` analogue** (D008): fenced code, tool output and tool input are token-highlighted
  **host-side** by Prism (`editors/vscode/src/highlight.ts`, bundled into `out/extension.js`),
  while the webview stays dependency-free (`media/chat.js` gains only DOM branches). The
  CSP admits only the bundled `media/*` assets.
- **`engines.vscode` floor.** Conservative (support older VS Code) vs the native-chat-API
  era. The plan picks a webview, **not** the native chat API (`:rT6jP`), so the floor need
  not match the chat-API release — stay conservative. **Pinned: `^1.85.0`** — above the 1.74
  auto-activation release, below anything that would require the native chat API.
- **What `AgentEvent` makes awkward:**
  1. **`message_update` is a WHOLE-message replacement, not a delta.** The serde payload is
     a full `AgentMessage` (`event.rs:hGXq5`), and the TUI assigns it (`app.rs:8b2AA`
     `self.live = Some(message)`). So `message_update` is O(message) on the wire per token —
     the `tui-theming-plan`-style "the O(n²) matters only over a socket" note applies. The
     reducer MUST assume replacement (idempotent), and must NOT diff-append. A future
     delta encoding is additive, so the reducer keyed on "replace" stays correct.
  2. **No event `other` catch-all** (`event.rs:JJqEF`; the doc defers it, `protocol.rs:NoynW`),
     so forward-compat is the **client's** job: an unknown `type` is unknown JSON, not a Rust
     error — the client ignores it. (Contrast `Request::Unknown`, `protocol.rs:hMT4F`.)
  3. **`id` is `u64`** — TS `number` is unsafe past 2^53; keep client ids small.
  4. **`spawned.worker` / `todo.todos`, not `id`** — the flatten forbids a top-level `id`
     duplicate (`event.rs:www93`, `:twS1K`).

## 9. Not verified / left to the implementer

- The exact `engines.vscode` version, the restart backoff numbers, and the markdown
  decision (§8 `TODO`s).
- That every `Request`/`AgentEvent` tag above round-trips **through an actual `wcode serve
  --stdio` process** — I read the serde attributes and the round-trip tests (the attributes
  are the spec), but did not run a capture (review-only).
- The fixture file names/format (§7) — sketched, not captured.
- `wcode-tui`'s roster path: the TUI builds surfaces from `ListSessions` at the driver
  (`main.rs:SYTRC`), not in `apply`, so "mirror the TUI" for the roster means mirroring
  `SessionInfo`/`MemberState`, not an `apply` arm — flagged in §3.
