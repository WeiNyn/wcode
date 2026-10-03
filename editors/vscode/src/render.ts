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

/** A tool call, ready to render (collapsed summary + expandable output). */
export interface RenderedTool {
  callId: string;
  name: string;
  /** The one-line summary shown while collapsed. */
  summary: string;
  outputHtml: string;
  outputText: string;
  done: boolean;
  isError: boolean;
  path?: string;
  durationMs?: number;
  /** P2 hook: a diff is available, so a "show diff" control may appear. */
  hasDiff: boolean;
}

/** One transcript block, pre-rendered. */
export interface RenderedBlock {
  kind: Block["kind"];
  html: string;
  live: boolean;
  from?: string;
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
 * Pure. The transcript/todos come from that session's key, so a retarget only
 * changes what is rendered — nothing is lost.
 */
export function renderState(state: ViewState, target: string | null = state.targeted): RenderedState {
  const blocks = transcriptOf(state, target);
  return {
    blocks: blocks.map(renderBlock),
    members: state.members,
    todos: target === null ? [] : state.todos[target] ?? [],
    // `running` is PER-TARGET: the roster's `MemberState` is the liveness fact.
    status: { ...state.status, running: targetRunning(state, target) },
    target: target === null ? null : { id: target, label: targetLabel(state, target) },
  };
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
      return { kind: "assistant", html: renderContent(block.content ?? []), live: block.live ?? false };
    case "tool":
      return { kind: "tool", html: "", live: false, tool: renderTool(block.tool) };
    case "user":
      return { kind: "user", html: escapeHtml(block.text ?? ""), live: false };
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
  if (!tool.done) return "running…";
  if (tool.isError) return "error";
  const firstLine = tool.output.split("\n").find((line) => line.trim() !== "")?.trim() ?? "";
  if (firstLine !== "") return clip(firstLine, 72);
  return tool.path !== undefined && tool.path !== "" ? tool.path : "done";
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

function renderContent(content: ContentBlock[]): string {
  const parts: string[] = [];
  for (const block of content) {
    switch (block.type) {
      case "text":
        parts.push(renderMarkdown(block.text));
        break;
      case "thinking":
        parts.push(
          `<details class="thinking"><summary>thinking</summary>${renderMarkdown(block.text)}</details>`,
        );
        break;
      case "tool_call":
        parts.push(`<div class="tool-call">⚙ ${escapeHtml(block.name)}</div>`);
        break;
    }
  }
  return parts.join("");
}

function renderTool(tool: ToolBlock | undefined): RenderedTool | undefined {
  if (!tool) return undefined;
  return {
    callId: tool.callId,
    name: tool.name,
    summary: toolSummary(tool),
    outputHtml: `<pre class="tool-output">${escapeHtml(tool.output)}</pre>`,
    outputText: tool.output,
    done: tool.done,
    isError: tool.isError,
    path: tool.path,
    durationMs: tool.durationMs,
    hasDiff: typeof tool.diff === "string" && tool.diff !== "",
  };
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
