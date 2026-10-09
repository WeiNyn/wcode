/**
 * Host-side rendering: `ViewState` → blocks + HTML.
 *
 * **Pure** — no `vscode`, no I/O — so plain node can drive it (that is the whole
 * point: the panel is untestable, this is not).
 *
 * Markdown is rendered HERE, in the extension host, with `html: false`
 * (`markdown.ts`), and the resulting HTML is what crosses `postMessage`. The
 * webview stays dependency-free and never ships `markdown-it`; `html: false`
 * escapes raw HTML, so the HTML is safe to insert with `innerHTML`.
 */
import type { ContentBlock, TodoItem } from "./protocol.ts";
import type { Block, SessionMember, ToolBlock, ViewState, ViewStatus } from "./reducer.ts";
import { targetLabel, transcriptOf } from "./reducer.ts";
import { renderMarkdown } from "./markdown.ts";
import { highlight, highlightLines, languageForTool } from "./highlight.ts";
import { reviewHunk } from "./review.ts";

/** A tool call, ready to render (collapsed summary + expandable output). */
export interface RenderedTool {
  callId: string;
  name: string;
  /** The one-line summary shown while collapsed. */
  summary: string;
  outputText: string;
  /** W009: the tool OUTPUT, highlighted host-side (Prism HTML; Prism escapes it). Absent
   *  when the language is unknown / the output is empty, so the webview shows plain text. */
  outputHtml?: string;
  done: boolean;
  isError: boolean;
  path?: string;
  durationMs?: number;
  /** P2 hook: a diff is available, so a "show diff" control may appear. */
  hasDiff: boolean;
  /**
   * The raw wcode presentation diff, so the webview counts `+N −M` itself
   * (`view.ts::diffStat`) and P4's in-panel review has the body. Omitted when
   * there is no diff (mirrors the `hasDiff` gate).
   */
  diff?: string;
  /** W009: the diff BODY highlighted, one well-formed HTML fragment per `reviewHunk` line
   *  (`splitHighlightedLines` re-opens a continuation span). Absent when plain / no hunk. */
  diffLinesHtml?: string[];
  /** `ToolBlock.target` — the row's `⚙ name  <target>`. Absent on a history row
   *  (or a call with no recognizable arg). */
  target?: string;
}

/** One transcript block, pre-rendered. */
export interface RenderedBlock {
  kind: Block["kind"];
  html: string;
  live: boolean;
  from?: string;
  /** The session id this block came from (All mode labels the turn by it). */
  origin?: string;
  /** The block's arrival ordinal, carried through so `renderMerged` can interleave. */
  seq?: number;
  tool?: RenderedTool;
}

/** The member the panel is showing: its id (the routing key) and its display name. */
export interface RenderedTarget {
  id: string;
  label: string;
}

/** The whole surface the webview paints. */
export interface RenderedState {
  blocks: RenderedBlock[];
  members: SessionMember[];
  todos: TodoItem[];
  status: ViewStatus;
  /** Whose transcript this is — always visible in the status strip. */
  target: RenderedTarget | null;
}

/**
 * Render the full view state FOR A TARGET member (default: the state's own).
 * Pure. The transcript comes from that session's key, so a retarget only
 * changes what is rendered — nothing is lost.
 */
export function renderState(state: ViewState, target: string | null = state.targeted): RenderedState {
  const blocks = transcriptOf(state, target);
  return {
    blocks: blocks.map(renderBlock),
    members: state.members,
    // The rail is DURABLE (it does not retarget), so it shows the ROOT session's plan.
    todos: rootTodos(state),
    // `running` is PER-TARGET: the roster's `MemberState` is the liveness fact.
    status: { ...state.status, running: targetRunning(state, target) },
    target: target === null ? null : { id: target, label: targetLabel(state, target) },
  };
}

/** The ROOT session's todos — the rail's durable plan (it does not retarget). Pure. */
function rootTodos(state: ViewState): TodoItem[] {
  const root = state.members.find((member) => member.isRoot) ?? state.members[0];
  return root === undefined ? [] : state.todos[root.id] ?? [];
}

function targetRunning(state: ViewState, target: string | null): boolean {
  if (target === null) return state.status.running;
  const member = state.members.find((m) => m.id === target);
  return member ? member.state === "running" : state.status.running;
}
/** Render one block. Pure. */
export function renderBlock(block: Block): RenderedBlock {
  switch (block.kind) {
    case "assistant":
      return { kind: "assistant", html: renderContent(block.content ?? [], block.live ?? false), live: block.live ?? false };
    case "tool":
      return { kind: "tool", html: "", live: false, tool: renderTool(block.tool) };
    case "user":
      return { kind: "user", html: escapeHtml(block.text ?? ""), live: false };
    case "peer":
      // A peer's message is prose from another agent — markdown, like `btw`.
      return { kind: "peer", html: renderMarkdown(block.text ?? ""), live: false, from: block.from };
    case "notice":
      return { kind: "notice", html: escapeHtml(block.text ?? ""), live: false, from: block.from };
    case "error":
      return { kind: "error", html: escapeHtml(block.text ?? ""), live: false };
    case "btw":
      return { kind: "btw", html: renderMarkdown(block.text ?? ""), live: false };
    default:
      // Forward-compat: an unknown block kind renders as a plain notice.
      return { kind: "notice", html: escapeHtml(block.text ?? ""), live: false };
  }
}

/** The collapsed one-liner for a tool call. Pure. */
export function toolSummary(tool: ToolBlock): string {
  if (tool.isError) return "error";
  const firstLine = tool.output.split("\n").find((line) => line.trim() !== "")?.trim() ?? "";
  if (firstLine !== "") return clip(firstLine, 72);
  if (tool.path !== undefined && tool.path !== "") return tool.path;
  return tool.done ? "done" : "";
}

/** Escape text for safe insertion into HTML. Pure. */
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/* ------------------------------------------------------------------ helpers */

function renderContent(content: ContentBlock[], live: boolean): string {
  const parts: string[] = [];
  for (const block of content) {
    switch (block.type) {
      case "text":
        parts.push(renderMarkdown(block.text));
        break;
      case "thinking":
        parts.push(
          `<details class="fold thinking"${live ? " open" : ""}><summary><span class="chev"></span><span class="tname">thinking</span></summary><div class="inner">${renderMarkdown(block.text)}</div></details>`,
        );
        break;
      // A `tool_call` content block renders as NOTHING (TUI parity:
      // crates/wcode-tui/src/ui.rs:559 `ContentBlock::ToolCall { .. } => {}`); the
      // tool's own `details.fold.tool` row (chat.ts `renderToolFold`) carries the call.
      case "tool_call":
        break;
    }
  }
  return parts.join("");
}

function renderTool(tool: ToolBlock | undefined): RenderedTool | undefined {
  if (!tool) return undefined;
  const hasDiff = typeof tool.diff === "string" && tool.diff !== "";
  const hunk = hasDiff ? reviewHunk(tool.diff ?? "") : null;
  return {
    callId: tool.callId,
    name: tool.name,
    summary: toolSummary(tool),
    outputText: tool.output,
    outputHtml: highlight(tool.output, languageForTool(tool.name, tool.path) ?? "") ?? undefined,
    done: tool.done,
    isError: tool.isError,
    path: tool.path,
    durationMs: tool.durationMs,
    hasDiff,
    diff: tool.diff,
    diffLinesHtml:
      hunk === null ? undefined : highlightLines(hunk.lines.map((line) => line.text), languageForTool(tool.name, tool.path)),
    target: tool.target,
  };
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/**
 * ALL-mode: every session's blocks, INTERLEAVED by arrival order. Pure.
 *
 * The reducer stamps each block with a monotonic `seq` (the order the client saw
 * it), so a member's blocks are contiguous only when nothing else streamed in
 * between — which is exactly the timeline the panel should show. Blocks without a
 * seq (a hand-built state) keep their relative order (a stable sort).
 */
export function renderMerged(state: ViewState): RenderedBlock[] {
  const merged = state.members.flatMap((member) =>
    transcriptOf(state, member.id).map((block) => ({ ...renderBlock(block), origin: member.id, seq: block.seq })),
  );
  return merged.sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0));
}
