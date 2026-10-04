/**
 * The wcode wire protocol — the ONE place the serde tags live.
 *
 * Derived from the Rust serde attributes (not from memory):
 *   Frame<P>     crates/wcode-harness/src/protocol.rs   (`#[serde(flatten)]` body)
 *   Request      crates/wcode-harness/src/protocol.rs   (tag = "type", rename_all = "snake_case")
 *   AgentEvent   crates/wcode-harness/src/event.rs       (tag = "type", rename_all = "snake_case")
 *   nested       crates/wcode-harness/src/message.rs
 *
 * The envelope's body is `#[serde(flatten)]`ed, so on the wire a frame is ONE
 * flat JSON object: `{"v":1,"id":7,"session":"s1","type":"cancel"}`.
 *
 * `id`/`reply_to`/`duration_ms` are Rust `u64`; a JS `number` is exact only to
 * 2^53. The client mints small ids from 1, so outbound ids are always safe; the
 * server mints `id: 0` for pushes. An inbound id above 2^53 would lose
 * precision — documented, not guarded.
 */

/** Wire-protocol major version (`protocol.rs` `PROTOCOL_VERSION`). */
export const PROTOCOL_VERSION = 1;

/** The frame envelope, shared by both directions; the body is flattened in. */
export interface Envelope {
  v: number;
  id: number;
  /** Present only on a direct reply; absent on streamed events. */
  reply_to?: number;
  /** A `SessionId` — a bare string (`#[serde(transparent)]`). */
  session: string;
  /** Present only for A2A (an `"agent:<id>"` peer). */
  sender?: string;
}

/** A parsed line off the wire: an envelope plus a not-yet-narrowed body. */
export interface RawFrame extends Envelope {
  type: string;
  [k: string]: unknown;
}

/* ------------------------------------------------------------------ Request */

/**
 * Inbound intent — the 15 request tags plus the `unknown` catch-all
 * (`Request::Unknown`, `#[serde(other)]`). Send the canonical tags; `steer`
 * and `follow_up` also deserialize (aliases) but are not emitted here.
 *
 * `null` is meaningful: `SetEffort.effort` and `Compact.instructions` have no
 * `skip_serializing_if`, so wire `None` is `null`; omitting the key also
 * deserializes to `None`.
 */
export type Request =
  | { type: "submit"; text: string }
  | { type: "notify"; content: string }
  | { type: "interrupt"; content: string }
  | { type: "wake"; content: string }
  | { type: "cancel" }
  | { type: "set_model"; model: string }
  | { type: "set_effort"; effort: string | null }
  | { type: "compact"; instructions: string | null }
  | { type: "set_plan_mode"; on: boolean }
  | { type: "side_ask"; text: string }
  | { type: "get_history" }
  | { type: "status" }
  | { type: "list_sessions" }
  /** Ask the transport for the provider's model catalog (reply: `models`). */
  | { type: "list_models" }
  | {
      type: "define";
      name?: string;
      model?: string;
      role?: string;
      tools?: string[];
      base_url?: string;
      api_key?: string;
      read_only?: boolean;
      effort?: string;
    }
  | { type: "unknown" };

/* --------------------------------------------------------------- AgentEvent */

/** One entry in the `todo` tool's session-local checklist. */
export interface TodoItem {
  content: string;
  status: TodoStatus;
}

export type TodoStatus = "pending" | "in_progress" | "completed";

/**
 * Everything a session streams or replies — **24 tags, no catch-all**
 * (`event.rs`; the serde `other` fallback is deliberately absent). Forward
 * compatibility is the client's job: an unknown `type` is unknown JSON, not a
 * Rust error, so a client must IGNORE it, never throw.
 */
export type AgentEvent =
  | { type: "agent_start" }
  | { type: "turn_start" }
  | { type: "message_start"; message: AgentMessage }
  /** Whole-message REPLACEMENT, not a delta (`event.rs` `MessageUpdate`). */
  | { type: "message_update"; message: AgentMessage }
  | { type: "message_end"; message: AgentMessage }
  | { type: "tool_execution_start"; call_id: string; name: string }
  | { type: "tool_execution_update"; call_id: string; name: string; partial: string }
  | {
      type: "tool_execution_end";
      call_id: string;
      name: string;
      output: string;
      is_error: boolean;
      diff?: string;
      path?: string;
      duration_ms?: number;
    }
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
  /** The provider's model catalog, answering a `list_models` request. */
  | { type: "models"; models: string[] }
  /** The field is `worker`, NOT `id` — the envelope already carries `id`. */
  | { type: "spawned"; worker: string }
  /** The field is `todos`, NOT `id` — the envelope already carries `id`. */
  | { type: "todo"; todos: TodoItem[] }
  | { type: "agent_end" };

/* ----------------------------------------------------------- nested payloads */

/** `message.rs` `ContentBlock` — tag = "type", snake_case. */
export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | { type: "tool_call"; id: string; name: string; arguments: unknown };

/** `message.rs` `AgentMessage` — tag = "role", snake_case. */
export type AgentMessage =
  | { role: "user"; content: ContentBlock[] }
  | {
      role: "assistant";
      content: ContentBlock[];
      stop_reason: StopReason;
      /** `skip_serializing_if = "Option::is_none"` — absent when unset. */
      usage?: Usage;
      model?: string;
    }
  | {
      role: "tool_result";
      tool_call_id: string;
      name: string;
      output: string;
      is_error: boolean;
    };

/** `message.rs` `StopReason` (includes `max_turns`). */
export type StopReason = "stop" | "length" | "tool_use" | "aborted" | "error" | "max_turns";

/** `message.rs` `Usage` — `cache_*` skip when absent. */
export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
}

/** `protocol.rs` `MemberState` — the server-side liveness fact. */
export type MemberState = "idle" | "running" | "done" | "failed";

/** `protocol.rs` `SessionInfo` — `model`/`effort`/`context_window` skip when absent,
 *  `state` defaults idle. */
export interface SessionInfo {
  id: string;
  model?: string;
  /** The session's effective reasoning effort; absent ⇒ unknown. */
  effort?: string;
  /** The context window in tokens — the gauge denominator; absent ⇒ unknown. */
  context_window?: number;
  state: MemberState;
}
