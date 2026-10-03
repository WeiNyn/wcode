/**
 * The view-model reducer — pure: no I/O, no `vscode` import, a plain
 * `(state, event) -> state`. A Node test drives it with captured JSON. It
 * mirrors `wcode-tui`'s `App::apply` (`crates/wcode-tui/src/app.rs`).
 *
 * The one wire subtlety it MUST assume: `message_update` carries a WHOLE
 * `AgentMessage`, not a delta — it REPLACES the live block's content. Never
 * diff-append.
 */
import type {
  AgentEvent,
  AgentMessage,
  ContentBlock,
  MemberState,
  StopReason,
  TodoItem,
} from "./protocol.ts";

export type BlockKind = "user" | "assistant" | "notice" | "error" | "btw" | "tool";

/** One tool invocation, keyed by `call_id` (better than the TUI's last-block). */
export interface ToolBlock {
  callId: string;
  name: string;
  output: string;
  done: boolean;
  isError: boolean;
  diff?: string;
  path?: string;
  durationMs?: number;
}

/** A transcript entry. `live` marks the one streaming assistant block. */
export interface Block {
  kind: BlockKind;
  /** user / notice / error / btw */
  text?: string;
  /** an inter-agent `message_received` sender */
  from?: string;
  /** assistant — text + thinking interleaved */
  content?: ContentBlock[];
  /** tool */
  tool?: ToolBlock;
  /** the streaming assistant block (replaced, never appended) */
  live?: boolean;
}

/** A roster member (root first), from `SessionInfo`. */
export interface SessionMember {
  id: string;
  label: string;
  model?: string;
  state: MemberState;
  isRoot: boolean;
}

export interface ViewStatus {
  running: boolean;
  /** Sticky until the next `agent_start` (a run failure, not an idle reply error). */
  lastError?: string;
  stopReason?: StopReason;
  planMode: boolean;
  /** `input_tokens` of the latest finished turn. */
  contextUsed?: number;
  model?: string;
}

/** The whole rendered surface. */
export interface ViewState {
  transcript: Block[];
  members: SessionMember[];
  todos: TodoItem[];
  status: ViewStatus;
}

export function initialState(): ViewState {
  return {
    transcript: [],
    members: [],
    todos: [],
    status: { running: false, planMode: false },
  };
}

/** The one surface: reduce a single `AgentEvent` into a new state. */
export function reduce(state: ViewState, event: AgentEvent): ViewState {
  switch (event.type) {
    case "agent_start":
      return withStatus(state, { running: true, lastError: undefined });

    case "agent_end":
      return withStatus(state, { running: false });

    case "message_start":
      // Only an assistant opens a live block; a user echo is ignored (app.rs).
      if (event.message.role !== "assistant") return state;
      return {
        ...state,
        transcript: [...state.transcript, { kind: "assistant", content: event.message.content, live: true }],
      };

    case "message_update": {
      if (event.message.role !== "assistant") return state;
      const idx = lastLiveIndex(state.transcript);
      if (idx < 0) return state;
      const transcript = state.transcript.slice();
      transcript[idx] = { ...transcript[idx], content: event.message.content, live: true };
      return { ...state, transcript };
    }

    case "message_end": {
      const idx = lastLiveIndex(state.transcript);
      if (idx < 0) return state;
      const transcript = state.transcript.slice();
      const content = event.message.role === "assistant" ? event.message.content : [];
      if (content.length === 0) {
        transcript.splice(idx, 1); // skip an empty assistant
      } else {
        transcript[idx] = { ...transcript[idx], content, live: false };
      }
      return { ...state, transcript };
    }

    case "tool_execution_start":
      return {
        ...state,
        transcript: [
          ...state.transcript,
          {
            kind: "tool",
            tool: { callId: event.call_id, name: event.name, output: "", done: false, isError: false },
          },
        ],
      };

    case "tool_execution_update":
      return mapTool(state, event.call_id, (tool) => ({ ...tool, output: tool.output + event.partial }));

    case "tool_execution_end":
      return mapTool(state, event.call_id, (tool) => ({
        ...tool,
        output: event.output !== "" ? event.output : tool.output,
        done: true,
        isError: event.is_error,
        diff: event.diff,
        path: event.path,
        durationMs: event.duration_ms,
      }));

    case "error": {
      const transcript = [...state.transcript, { kind: "error" as const, text: event.message }];
      // A run failure marks the run; an idle reply error does not (app.rs).
      const status = state.status.running ? { ...state.status, lastError: event.message } : state.status;
      return { ...state, transcript, status };
    }

    case "stopped":
      return withStatus(state, { stopReason: event.stop_reason });

    case "todo":
      return { ...state, todos: event.todos };

    case "sessions":
      return { ...state, members: event.sessions.map((s, i) => toMember(s.id, s.model, s.state, i === 0)) };

    case "spawned": {
      if (state.members.some((m) => m.id === event.worker)) return state;
      return { ...state, members: [...state.members, toMember(event.worker, undefined, "idle", false)] };
    }

    case "message_received":
      return {
        ...state,
        transcript: [...state.transcript, { kind: "notice", from: event.from, text: event.content }],
      };

    case "history":
      return seedFromHistory(state, event.messages);

    case "turn_end": {
      const usage = event.message.role === "assistant" ? event.message.usage : undefined;
      if (!usage) return state;
      return withStatus(state, { contextUsed: usage.input_tokens });
    }

    case "side_answer":
      return { ...state, transcript: [...state.transcript, { kind: "btw", text: event.text }] };

    case "compaction":
      return notice(state, `⋯ compacted ${event.summarized} messages, kept ${event.kept}`);

    case "compaction_skipped":
      return notice(state, `⋯ compaction skipped: ${event.reason}`);

    case "retrying":
      return notice(state, `⋯ retrying (${event.attempt}/${event.max}): ${event.reason}`);

    // turn_start / ack / status: no per-view state.
    default:
      return state;
  }
}

/**
 * Seed the transcript from a `GetHistory` reply (or a resumed session) — the
 * second entry point, mirroring `App::seed_history`. The TUI seeds at connect,
 * not in `apply`; here it folds through the `history` arm.
 */
export function seedFromHistory(state: ViewState, messages: AgentMessage[]): ViewState {
  if (messages.length === 0) return state;
  const transcript = state.transcript.slice();
  let contextUsed = state.status.contextUsed;
  for (const message of messages) {
    switch (message.role) {
      case "user": {
        const text = textOf(message.content);
        if (text.trim() !== "") transcript.push({ kind: "user", text });
        break;
      }
      case "assistant": {
        if (message.content.length > 0) transcript.push({ kind: "assistant", content: message.content });
        if (message.usage) contextUsed = message.usage.input_tokens;
        break;
      }
      case "tool_result":
        transcript.push({
          kind: "tool",
          tool: {
            callId: message.tool_call_id,
            name: message.name,
            output: message.output,
            done: true,
            isError: message.is_error,
          },
        });
        break;
    }
  }
  return { ...state, transcript, status: { ...state.status, contextUsed } };
}

/* ------------------------------------------------------------------ helpers */

function withStatus(state: ViewState, patch: Partial<ViewStatus>): ViewState {
  return { ...state, status: { ...state.status, ...patch } };
}

function notice(state: ViewState, text: string): ViewState {
  return { ...state, transcript: [...state.transcript, { kind: "notice", text }] };
}

function lastLiveIndex(transcript: Block[]): number {
  for (let i = transcript.length - 1; i >= 0; i -= 1) {
    if (transcript[i].live) return i;
  }
  return -1;
}

function mapTool(state: ViewState, callId: string, update: (tool: ToolBlock) => ToolBlock): ViewState {
  const idx = state.transcript.findIndex((b) => b.kind === "tool" && b.tool?.callId === callId);
  if (idx < 0) return state;
  const transcript = state.transcript.slice();
  const block = transcript[idx];
  if (!block.tool) return state;
  transcript[idx] = { ...block, tool: update(block.tool) };
  return { ...state, transcript };
}

function toMember(id: string, model: string | undefined, state: MemberState, isRoot: boolean): SessionMember {
  return { id, label: shortLabel(id), model, state, isRoot };
}

function shortLabel(id: string): string {
  return id.startsWith("agent:") ? id.slice("agent:".length) : id;
}

function textOf(content: ContentBlock[]): string {
  return content
    .filter((b): b is { type: "text"; text: string } => b.type === "text")
    .map((b) => b.text)
    .join("");
}
