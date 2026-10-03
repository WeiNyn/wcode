/**
 * The view-model reducer — pure: no I/O, no `vscode` import, a plain
 * `(state, event, session) -> state`. A Node test drives it with captured JSON.
 * It mirrors `wcode-tui`'s `App::apply` (`crates/wcode-tui/src/app.rs`).
 *
 * P3: the surface is **per-session**. One connection carries many sessions, and
 * every event is stamped with its origin `frame.session` (`server.rs` `fan`), so
 * the caller passes that id and each arm writes that member's transcript/todos.
 * `targeted` is the member whose surface the panel is showing — a retarget only
 * changes *which* key is rendered, so no in-flight transcript is lost.
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
  TodoStatus,
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

/** A roster member (root first), from `SessionInfo` plus derived event state. */
export interface SessionMember {
  id: string;
  /** `shortLabel(id)` — the raw short name, not the display label. */
  label: string;
  model?: string;
  state: MemberState;
  isRoot: boolean;
  /**
   * `"{tool} {target}"` for the member's CURRENT run, derived from the live
   * event stream (`tool_execution_start`) and CLEARED on `agent_end` — never
   * stale. The roster itself carries no action field (`MemberState` is liveness
   * only).
   */
  liveAction?: string;
}

/** A roster row, ready for the tree: the id is the routing key, the label is display text. */
export interface RosterItem {
  id: string;
  label: string;
  model?: string;
  state: MemberState;
  isRoot: boolean;
  liveAction?: string;
}

export interface ViewStatus {
  /** Any run in flight on this connection (the rendered `running` is per-target). */
  running: boolean;
  /** Sticky until the next `agent_start` (a run failure, not an idle reply error). */
  lastError?: string;
  stopReason?: StopReason;
  /** The displayed plan mode (optimistic; settled by `planModeAck`). */
  planMode: boolean;
  /** The value `planMode` held before an in-flight toggle; `undefined` = settled. */
  planModePrev?: boolean;
  /** `input_tokens` of the latest finished turn. */
  contextUsed?: number;
  model?: string;
}

/** The whole view model. */
export interface ViewState {
  /** Blocks PER session id (the frame's `session`). */
  transcripts: Record<string, Block[]>;
  /** The member whose surface is shown; `null` until a roster arrives. */
  targeted: string | null;
  members: SessionMember[];
  /** Todos PER session id — the sidebar is authoritative (no transcript notice). */
  todos: Record<string, TodoItem[]>;
  status: ViewStatus;
}

export function initialState(): ViewState {
  return {
    transcripts: {},
    targeted: null,
    members: [],
    todos: {},
    status: { running: false, planMode: false },
  };
}

/**
 * Reduce one event into the session it belongs to — the frame's `session`.
 * The default `""` is for tests that do not care which session a fixture used.
 */
export function reduce(state: ViewState, event: AgentEvent, session = ""): ViewState {
  switch (event.type) {
    case "agent_start":
      return withStatus(state, { running: true, lastError: undefined });

    case "agent_end":
      // The run is over: the member must never keep a stale action.
      return setLiveAction(withStatus(state, { running: false }), session, undefined);

    case "message_start":
      // Only an assistant opens a live block; a user echo is ignored (app.rs).
      if (event.message.role !== "assistant") return state;
      return setBlocks(state, session, [
        ...blocksOf(state, session),
        { kind: "assistant", content: event.message.content, live: true },
      ]);

    case "message_update": {
      if (event.message.role !== "assistant") return state;
      const blocks = blocksOf(state, session);
      const idx = lastLiveIndex(blocks);
      if (idx < 0) return state;
      const next = blocks.slice();
      next[idx] = { ...next[idx], content: event.message.content, live: true };
      return setBlocks(state, session, next);
    }

    case "message_end": {
      const blocks = blocksOf(state, session);
      const idx = lastLiveIndex(blocks);
      if (idx < 0) return state;
      const next = blocks.slice();
      const content = event.message.role === "assistant" ? event.message.content : [];
      if (content.length === 0) {
        next.splice(idx, 1); // skip an empty assistant
      } else {
        next[idx] = { ...next[idx], content, live: false };
      }
      return setBlocks(state, session, next);
    }

    case "tool_execution_start": {
      const blocks = blocksOf(state, session);
      const started = setBlocks(state, session, [
        ...blocks,
        { kind: "tool", tool: { callId: event.call_id, name: event.name, output: "", done: false, isError: false } },
      ]);
      // The roster row's dim right-hand text, from the live stream — cleared on
      // `agent_end`, so a finished member falls back to its `MemberState`.
      return setLiveAction(started, session, actionLabel(blocks, event.call_id, event.name));
    }

    case "tool_execution_update":
      return mapTool(state, session, event.call_id, (tool) => ({
        ...tool,
        output: tool.output + event.partial,
      }));

    case "tool_execution_end":
      return mapTool(state, session, event.call_id, (tool) => ({
        ...tool,
        output: event.output !== "" ? event.output : tool.output,
        done: true,
        isError: event.is_error,
        diff: event.diff,
        path: event.path,
        durationMs: event.duration_ms,
      }));

    case "error": {
      const withError = setBlocks(state, session, [
        ...blocksOf(state, session),
        { kind: "error", text: event.message },
      ]);
      // A run failure marks the run; an idle reply error does not (app.rs). A
      // pending optimistic plan toggle whose reply errored reverts.
      const status = withError.status.running
        ? { ...withError.status, lastError: event.message }
        : withError.status;
      return planModeRevert({ ...withError, status });
    }

    case "stopped":
      return withStatus(state, { stopReason: event.stop_reason });

    case "ack":
      // `SetPlanMode` is infallible and replies `Ack`: settle the optimistic chip.
      return planModeAck(state);

    case "todo":
      return { ...state, todos: { ...state.todos, [session]: event.todos } };

    case "sessions":
      return { ...state, members: mergeMembers(state.members, event.sessions) };

    case "spawned": {
      if (state.members.some((m) => m.id === event.worker)) return state;
      return { ...state, members: [...state.members, toMember(event.worker, undefined, "idle", false)] };
    }

    case "message_received":
      // The frame is stamped with the RECEIVING session, so inter-agent traffic
      // lands in that member's transcript (the root's own in the root's).
      return setBlocks(state, session, [
        ...blocksOf(state, session),
        { kind: "notice", from: event.from, text: event.content },
      ]);

    case "history":
      return seedFromHistory(state, event.messages, session);

    case "turn_end": {
      const usage = event.message.role === "assistant" ? event.message.usage : undefined;
      if (!usage) return state;
      return withStatus(state, { contextUsed: usage.input_tokens });
    }

    case "side_answer":
      return setBlocks(state, session, [...blocksOf(state, session), { kind: "btw", text: event.text }]);

    case "compaction":
      return notice(state, session, `⋯ compacted ${event.summarized} messages, kept ${event.kept}`);

    case "compaction_skipped":
      return notice(state, session, `⋯ compaction skipped: ${event.reason}`);

    case "retrying":
      return notice(state, session, `⋯ retrying (${event.attempt}/${event.max}): ${event.reason}`);

    // turn_start / status: no per-view state.
    default:
      return state;
  }
}

/**
 * Seed a session's transcript from a `GetHistory` reply (or a resumed session) —
 * the second entry point, mirroring `App::seed_history`. The TUI seeds at
 * connect, not in `apply`; here it folds through the `history` arm.
 */
export function seedFromHistory(state: ViewState, messages: AgentMessage[], session = ""): ViewState {
  if (messages.length === 0) return state;
  const hydrated: Block[] = [];
  let contextUsed = state.status.contextUsed;
  for (const message of messages) {
    switch (message.role) {
      case "user": {
        const text = textOf(message.content);
        if (text.trim() !== "") hydrated.push({ kind: "user", text });
        break;
      }
      case "assistant": {
        if (message.content.length > 0) hydrated.push({ kind: "assistant", content: message.content });
        if (message.usage) contextUsed = message.usage.input_tokens;
        break;
      }
      case "tool_result":
        hydrated.push({
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
  // The history is authoritative for the session's PAST: it REPLACES the
  // committed blocks. A member's events are folded by `frame.session` as they
  // stream, so its committed blocks are already here — appending would duplicate
  // them (and put a live block before the history it belongs to). Only an
  // in-flight (`live`) block survives, and it goes LAST.
  const live = blocksOf(state, session).filter((block) => block.live === true);
  return {
    ...setBlocks(state, session, [...hydrated, ...live]),
    status: { ...state.status, contextUsed },
  };
}

/**
 * Optimistic echo of the user's own submitted text into `session`. The session
 * streams only the assistant's reply — a `Submit`'s text never comes back as an
 * event (the TUI echoes locally too) — so without this the user's message would
 * vanish the moment they press Enter.
 */
export function appendUser(state: ViewState, text: string, session = ""): ViewState {
  if (text.trim() === "") return state;
  return setBlocks(state, session, [...blocksOf(state, session), { kind: "user", text }]);
}

/** The blocks for one session (never undefined). Pure. */
export function transcriptOf(state: ViewState, id: string | null): Block[] {
  if (id === null) return [];
  return state.transcripts[id] ?? [];
}

/** Point the surface at a member. Pure. */
export function target(state: ViewState, id: string | null): ViewState {
  if (state.targeted === id) return state;
  return { ...state, targeted: id };
}

/** The display name for a member: the root reads as "orchestrator". Pure. */
export function displayLabel(member: SessionMember): string {
  return member.isRoot ? "orchestrator" : member.label;
}

/** The display name for a session id, resolved against the roster. Pure. */
export function targetLabel(state: ViewState, id: string): string {
  const member = state.members.find((m) => m.id === id);
  return member ? displayLabel(member) : shortLabel(id);
}

/** The tree's rows: the roster, root first, with display labels. Pure. */
export function memberViews(members: SessionMember[]): RosterItem[] {
  return members.map((member) => ({
    id: member.id,
    label: displayLabel(member),
    model: member.model,
    state: member.state,
    isRoot: member.isRoot,
    liveAction: member.liveAction,
  }));
}

/** The `☑ done/total` badge for a todo list. Pure. */
export function todoBadge(todos: TodoItem[]): string {
  const done = todos.filter((todo) => todo.status === "completed").length;
  return `☑ ${done}/${todos.length}`;
}

/** Flip the plan chip optimistically, remembering the value to revert to. Pure. */
export function planModePending(state: ViewState, on: boolean): ViewState {
  return withStatus(state, { planMode: on, planModePrev: state.status.planMode });
}

/** Settle the plan chip: `SetPlanMode` is infallible and replies `Ack`. Pure. */
export function planModeAck(state: ViewState): ViewState {
  if (state.status.planModePrev === undefined) return state;
  return withStatus(state, { planModePrev: undefined });
}

/** Revert a pending plan toggle (its reply errored). Pure. */
export function planModeRevert(state: ViewState): ViewState {
  const prev = state.status.planModePrev;
  if (prev === undefined) return state;
  return withStatus(state, { planMode: prev, planModePrev: undefined });
}

/** The argument keys a tool call may name in its action label, most specific first. */
const ACTION_KEYS = ["path", "file_path", "file", "pattern", "command", "cmd", "query", "url", "name"];

/**
 * The team row's action label for a tool call: `"{name} {target}"`, where the
 * target is the first present string argument among `ACTION_KEYS` on the
 * committed assistant block whose `ToolCall` id matches. Falls back to `name`.
 * Mirrors `app.rs::action_label`. Pure.
 */
export function actionLabel(blocks: Block[], callId: string, name: string): string {
  const target = callTarget(blocks, callId);
  return target === undefined ? name : `${name} ${target}`;
}

/** The call's recognizable target argument, if any. Mirrors `app.rs::call_target`. Pure. */
export function callTarget(blocks: Block[], callId: string): string | undefined {
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const content = blocks[i].content;
    if (content === undefined) continue;
    for (const block of content) {
      if (block.type !== "tool_call" || block.id !== callId) continue;
      const args = block.arguments;
      if (args === null || typeof args !== "object") continue;
      const record = args as Record<string, unknown>;
      for (const key of ACTION_KEYS) {
        const value = record[key];
        if (typeof value === "string" && value !== "") return clip(value, 40);
      }
    }
  }
  return undefined;
}

/**
 * The once-per-session hydration guard: a session's `GetHistory` is asked for at
 * most once per child, because a second hydration would duplicate its transcript.
 *
 * Pure, so the "re-hydrate on every retarget" defect is testable — a guard no
 * test can reach is a guard nobody can trust.
 */
export class HydratedSet {
  private readonly ids = new Set<string>();

  /** True the FIRST time this id is claimed, false on every later claim. */
  claim(id: string): boolean {
    if (this.ids.has(id)) return false;
    this.ids.add(id);
    return true;
  }

  /** Give a claim back (the request was never sent). */
  release(id: string): void {
    this.ids.delete(id);
  }

  /** Forget every claim — a restart spawns a fresh child with empty history. */
  reset(): void {
    this.ids.clear();
  }

  get size(): number {
    return this.ids.size;
  }
}

/* ------------------------------------------------------------------ helpers */

function blocksOf(state: ViewState, session: string): Block[] {
  return state.transcripts[session] ?? [];
}

function setBlocks(state: ViewState, session: string, blocks: Block[]): ViewState {
  return { ...state, transcripts: { ...state.transcripts, [session]: blocks } };
}

function withStatus(state: ViewState, patch: Partial<ViewStatus>): ViewState {
  return { ...state, status: { ...state.status, ...patch } };
}

function notice(state: ViewState, session: string, text: string): ViewState {
  return setBlocks(state, session, [...blocksOf(state, session), { kind: "notice", text }]);
}

function lastLiveIndex(transcript: Block[]): number {
  for (let i = transcript.length - 1; i >= 0; i -= 1) {
    if (transcript[i].live) return i;
  }
  return -1;
}

function mapTool(
  state: ViewState,
  session: string,
  callId: string,
  update: (tool: ToolBlock) => ToolBlock,
): ViewState {
  const blocks = blocksOf(state, session);
  const idx = blocks.findIndex((b) => b.kind === "tool" && b.tool?.callId === callId);
  if (idx < 0) return state;
  const block = blocks[idx];
  if (!block.tool) return state;
  const next = blocks.slice();
  next[idx] = { ...block, tool: update(block.tool) };
  return setBlocks(state, session, next);
}

/** Set (or clear) the `liveAction` on the member with this session id. */
function setLiveAction(state: ViewState, session: string, action: string | undefined): ViewState {
  const idx = state.members.findIndex((m) => m.id === session);
  if (idx < 0) return state;
  const member = state.members[idx];
  if (member.liveAction === action) return state;
  const members = state.members.slice();
  members[idx] =
    action === undefined
      ? {
          id: member.id,
          label: member.label,
          model: member.model,
          state: member.state,
          isRoot: member.isRoot,
        }
      : { ...member, liveAction: action };
  return { ...state, members };
}

/**
 * Replace the roster from a `sessions` push, PRESERVING the derived `liveAction`
 * for members that persist (the push carries no action field — it is event-stream
 * state, not roster state).
 */
function mergeMembers(
  previous: SessionMember[],
  incoming: Array<{ id: string; model?: string; state: MemberState }>,
): SessionMember[] {
  return incoming.map((session, index) => {
    const member = toMember(session.id, session.model, session.state, index === 0);
    const before = previous.find((m) => m.id === session.id);
    return before?.liveAction === undefined ? member : { ...member, liveAction: before.liveAction };
  });
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

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
/** One Tasks row, ready for the rail. Pure. */
export interface TodoView {
  /** The item text (the draft spans the row). */
  label: string;
  status: TodoStatus;
}

/**
 * Map the todo list to rows (draft `.todo`). Pure — the box glyph comes from
 * `todoGlyph(status)` at render time, not a per-row field.
 */
export function todoViews(todos: TodoItem[]): TodoView[] {
  return todos.map((todo) => ({ label: todo.content, status: todo.status }));
}

/** The webview's team+tasks rail (draft `.side`). Pure. */
export interface SidebarRails {
  /** The Team rows (the same shape as `memberViews`). */
  team: RosterItem[];
  /** The Tasks section: the `☑ done/total` badge + rows. */
  tasks: { badge: string; rows: TodoView[] };
}

/**
 * Build the rail model from the roster + ONE session's todos. Pure.
 *   team  = memberViews(members)
 *   tasks = { badge: todoBadge(todos), rows: todoViews(todos) }
 * The webview renders it DIRECTLY (no native tree, no `SidebarNode`).
 */
export function sidebarRails(members: SessionMember[], todos: TodoItem[]): SidebarRails {
  return {
    team: memberViews(members),
    tasks: { badge: todoBadge(todos), rows: todoViews(todos) },
  };
}

/** The draft's `.todo .box` glyph + row class per `TodoStatus`. Pure. */
export function todoGlyph(status: TodoStatus): { glyph: string; className: string } {
  switch (status) {
    case "completed":
      return { glyph: "☑", className: "done" };
    case "in_progress":
      return { glyph: "▸", className: "doing" };
    default:
      return { glyph: "☐", className: "" };
  }
}

/**
 * The draft's `.roster .glyph` glyph + class per `MemberState` (the TUI vocabulary).
 * The webview CANNOT use a codicon id, so the rail renders text. Pure.
 */
export function memberGlyph(state: MemberState): { glyph: string; className: string } {
  switch (state) {
    case "running":
      return { glyph: "⠋", className: "g-run" };
    case "done":
      return { glyph: "✓", className: "g-done" };
    case "failed":
      return { glyph: "✗", className: "g-err" };
    default:
      return { glyph: "○", className: "g-idle" };
  }
}
