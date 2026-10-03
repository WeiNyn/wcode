/**
 * The webview's PURE half — no `vscode`, no DOM, so plain node can drive it.
 *
 * `chat.ts` owns the DOM (createElement, scroll glue, the click handlers);
 * everything here is a plain function over the snapshot, mirroring the
 * `render.ts`-pure / `panel.ts`-impure split on the host side.
 */
import type { RenderedBlock, RenderedState } from "../render.ts";
import type { SessionState } from "../session.ts";
import type { PanelSessionInfo } from "../webview.ts";

/** One inline span of the status strip. */
export interface StatusSegment {
  className: string;
  text: string;
}

/** The FSM state as the user reads it. */
export function stateLabel(state: SessionState): string {
  switch (state) {
    case "ready":
      return "ready";
    case "starting":
      return "starting";
    case "crashed":
      return "crashed";
    default:
      return "stopped";
  }
}

/** The status strip, in order. The stderr tail is a `<details>`, not a segment. */
export function statusSegments(state: RenderedState, session: PanelSessionInfo): StatusSegment[] {
  const segments: StatusSegment[] = [
    { className: `dot dot-${session.state}`, text: "" },
    { className: "status-state", text: stateLabel(session.state) },
    { className: "status-sep", text: "·" },
    { className: "status-session", text: session.id ?? "no session yet" },
  ];
  // WHOSE transcript this is — the target must always be visible (P3).
  if (state.target !== null) {
    segments.push(
      { className: "status-sep", text: "·" },
      { className: "status-target", text: state.target.label },
    );
  }
  if (state.status.planMode) {
    segments.push({ className: "status-sep", text: "·" }, { className: "status-plan", text: "plan" });
  }
  if (state.status.running) {
    segments.push({ className: "status-sep", text: "·" }, { className: "status-running", text: "running…" });
  }
  if (typeof state.status.contextUsed === "number") {
    segments.push(
      { className: "status-sep", text: "·" },
      { className: "status-ctx", text: `ctx ${state.status.contextUsed}` },
    );
  }
  if (state.status.lastError) {
    segments.push(
      { className: "status-sep", text: "·" },
      { className: "status-error", text: state.status.lastError },
    );
  }
  return segments;
}

/** Which empty state to show (the transcript is empty). */
export type EmptyKind = "crashed" | "starting" | "stopped" | "working" | "idle";

export function emptyKind(state: RenderedState, session: PanelSessionInfo): EmptyKind {
  if (session.state === "crashed") return "crashed";
  if (session.state === "starting") return "starting";
  if (session.state === "stopped") return "stopped";
  return state.status.running ? "working" : "idle";
}

/** The tool call's expansion toggle (a new set — the DOM keeps it immutable). */
export function toggleExpanded(expanded: ReadonlySet<string>, callId: string): Set<string> {
  const next = new Set(expanded);
  if (next.has(callId)) next.delete(callId);
  else next.add(callId);
  return next;
}

/** The block's class list. `expanded` is only meaningful for a tool block. */
export function classNames(block: RenderedBlock, expanded = false): string {
  switch (block.kind) {
    case "tool":
      return `block tool${block.tool?.isError ? " error" : ""}${expanded ? " expanded" : ""}`;
    case "assistant":
      return `block assistant${block.live ? " live" : ""}`;
    default:
      return `block ${block.kind}`;
  }
}

