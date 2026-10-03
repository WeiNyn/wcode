/**
 * The webview ⇄ host message contract, and the small pure helpers around it.
 *
 * **Pure** — no `vscode` import — so plain node can test the union parsing and
 * the throttle. `panel.ts` is the only place these types meet `vscode`.
 *
 * Divergence from the sketch, owned: the sketch's `ToWebview.state` carried the
 * raw `ViewState`; we render markdown in the host (`render.ts`) and carry the
 * pre-rendered `RenderedState` instead, so the webview stays dependency-free.
 */
import type { RenderedBlock, RenderedState } from "./render.ts";
import type { SessionState } from "./session.ts";

/** What the panel knows about the child, for the status strip. */
export interface PanelSessionInfo {
  /** The root session id the panel is attached to (null until the seed). */
  id: string | null;
  /** The `WcodeSession` FSM state. */
  state: SessionState;
  /** The child's stderr tail — shown only when `state === "crashed"`. */
  stderrTail: string;
}

/** host → webview. */
export type ToWebview =
  | { kind: "state"; state: RenderedState; session: PanelSessionInfo }
  | { kind: "append"; block: RenderedBlock }
  | { kind: "diff"; callId: string; path: string; diff: string };

/** webview → host. */
export type FromWebview =
  | { kind: "submit"; text: string }
  | { kind: "cancel" }
  | { kind: "steer"; text: string }
  | { kind: "open-diff"; callId: string }
  | { kind: "reveal-file"; path: string; line?: number }
  /** The webview's script has run and it is ready to receive a snapshot. */
  | { kind: "ready" };

/**
 * Narrow an untrusted `postMessage` payload to a `FromWebview`, or `null`.
 * Pure — the host never trusts the webview's shape.
 */
export function parseFromWebview(raw: unknown): FromWebview | null {
  if (raw === null || typeof raw !== "object") return null;
  const message = raw as Record<string, unknown>;
  switch (message.kind) {
    case "submit":
      return typeof message.text === "string" && message.text.trim() !== ""
        ? { kind: "submit", text: message.text }
        : null;
    case "steer":
      return typeof message.text === "string" && message.text.trim() !== ""
        ? { kind: "steer", text: message.text }
        : null;
    case "cancel":
      return { kind: "cancel" };
    case "ready":
      return { kind: "ready" };
    case "open-diff":
      return typeof message.callId === "string" ? { kind: "open-diff", callId: message.callId } : null;
    case "reveal-file":
      return typeof message.path === "string"
        ? { kind: "reveal-file", path: message.path, line: typeof message.line === "number" ? message.line : undefined }
        : null;
    default:
      return null;
  }
}

/* ------------------------------------------------------------------ throttle */

/** The bits of `Date`/`setTimeout` the throttle needs — injectable for tests. */
export interface Scheduler {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

/** The real clock. */
export const realScheduler: Scheduler = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (handle) => clearTimeout(handle as NodeJS.Timeout),
};

export interface Throttle {
  /** Record the latest snapshot; send now if the window elapsed, else schedule. */
  push(state: RenderedState): void;
  /** Send the latest snapshot immediately (a settled event: `message_end`). */
  flush(state: RenderedState): void;
  /** Cancel any scheduled send. */
  dispose(): void;
}

/**
 * Coalesce rapid `push`es into at most one `send` per `intervalMs`, so a fast
 * stream cannot flood the webview. `flush` always sends (the final state).
 */
export function createThrottle(
  intervalMs: number,
  send: (state: RenderedState) => void,
  scheduler: Scheduler = realScheduler,
): Throttle {
  let lastSentAt = Number.NEGATIVE_INFINITY;
  let timer: unknown = null;
  let pending: RenderedState | null = null;

  const fire = (): void => {
    timer = null;
    if (pending === null) return;
    const state = pending;
    pending = null;
    lastSentAt = scheduler.now();
    send(state);
  };

  const schedule = (): void => {
    if (timer !== null) return;
    const elapsed = scheduler.now() - lastSentAt;
    const delay = Math.max(0, intervalMs - elapsed);
    timer = scheduler.setTimeout(fire, delay);
  };

  return {
    push(state) {
      pending = state;
      if (timer === null && scheduler.now() - lastSentAt >= intervalMs) {
        fire();
        return;
      }
      schedule();
    },
    flush(state) {
      if (timer !== null) {
        scheduler.clearTimeout(timer);
        timer = null;
      }
      pending = null;
      lastSentAt = scheduler.now();
      send(state);
    },
    dispose() {
      if (timer !== null) {
        scheduler.clearTimeout(timer);
        timer = null;
      }
      pending = null;
    },
  };
}
