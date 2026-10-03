/**
 * The webview ⇄ host message contract, and the small pure helpers around it.
 *
 * **Pure** — no `vscode` import — so plain node can test the union parsing and
 * the throttle. `panel.ts` is the only place these types meet `vscode`; the
 * webview narrows the inbound side with `parseToWebview`.
 *
 * Divergence from the sketch, owned: the sketch's `ToWebview.state` carried the
 * raw `ViewState`; we render markdown in the host (`render.ts`) and carry the
 * pre-rendered `RenderedState` instead, so the webview stays dependency-free.
 */
import type { RenderedState } from "./render.ts";
import type { Verdict } from "./review.ts";
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

/**
 * The active editor's selection. HOST-derived: the selection is NOT a kernel
 * field and never crosses the protocol — it is a host-local view input.
 */
export interface SelectionContext {
  /** Workspace-relative path (fs path when outside a workspace folder). */
  path: string;
  /** 1-based inclusive first line. */
  startLine: number;
  /** 1-based inclusive last line. */
  endLine: number;
}

/** The client-local view mode: `all` = the merged transcript; `focus` = the target. */
export type ViewMode = "all" | "focus";

/** host → webview. Full snapshots only — there is no append/diff fast path. */
export type ToWebview = {
  kind: "state";
  state: RenderedState;
  session: PanelSessionInfo;
  /** The editor selection the composer may attach, or null. Host-local. */
  context: SelectionContext | null;
  /** Per-callId review verdicts. Host-local (like `context`); JSON-safe. */
  verdicts: Record<string, Verdict>;
  /** The client-local view mode (all = the merged transcript; focus = the target). */
  mode: ViewMode;
};

/** webview → host. */
export type FromWebview =
  | { kind: "submit"; text: string }
  | { kind: "cancel" }
  | { kind: "steer"; text: string }
  | { kind: "open-diff"; callId: string }
  | { kind: "reveal-file"; path: string; line?: number }
  | { kind: "toggle-plan" }
  | { kind: "focus-member"; id: string }
  | { kind: "review"; callId: string; verdict: "accept" | "reject" }
  | { kind: "set-mode"; mode: ViewMode }
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
    case "toggle-plan":
      return { kind: "toggle-plan" };
    case "focus-member":
      return typeof message.id === "string" ? { kind: "focus-member", id: message.id } : null;
    case "set-mode":
      return message.mode === "all" || message.mode === "focus"
        ? { kind: "set-mode", mode: message.mode }
        : null;
    case "review":
      return typeof message.callId === "string" && (message.verdict === "accept" || message.verdict === "reject")
        ? { kind: "review", callId: message.callId, verdict: message.verdict }
        : null;
    case "reveal-file":
      return typeof message.path === "string"
        ? { kind: "reveal-file", path: message.path, line: typeof message.line === "number" ? message.line : undefined }
        : null;
    default:
      return null;
  }
}

/**
 * Narrow an untrusted `postMessage` payload to a `ToWebview`, or `null`. Pure —
 * the webview trusts the host no more than the host trusts the webview (it
 * indexes `state.blocks`).
 */
export function parseToWebview(raw: unknown): ToWebview | null {
  if (raw === null || typeof raw !== "object") return null;
  const message = raw as Record<string, unknown>;
  if (message.kind !== "state") return null;

  const state = message.state;
  if (state === null || typeof state !== "object") return null;
  const rendered = state as Record<string, unknown>;
  if (!Array.isArray(rendered.blocks)) return null;
  if (rendered.status === null || typeof rendered.status !== "object") return null;

  const session = message.session;
  if (session === null || typeof session !== "object") return null;
  const info = session as Record<string, unknown>;
  if (typeof info.state !== "string" || !isSessionState(info.state)) return null;
  if (!(info.id === null || typeof info.id === "string")) return null;
  if (typeof info.stderrTail !== "string") return null;

  const context = parseContext(message.context);
  const verdicts = parseVerdicts(message.verdicts);
  const mode = message.mode === "all" ? "all" : "focus";

  return {
    kind: "state",
    state: state as RenderedState,
    session: { id: info.id as string | null, state: info.state, stderrTail: info.stderrTail },
    context,
    verdicts,
    mode,
  };
}

function isSessionState(value: string): value is SessionState {
  return value === "stopped" || value === "starting" || value === "ready" || value === "crashed";
}

/**
 * Narrow an untrusted `context` to a `SelectionContext`, or `null`. LENIENT: the
 * selection is NOT load-bearing, so an absent / null / wrong-typed / partial value
 * degrades to `null` and the WHOLE snapshot is kept — a reject here would drop the
 * transcript for a decoration, the wrong trade.
 */
function parseContext(raw: unknown): SelectionContext | null {
  if (raw === null || typeof raw !== "object") return null;
  const context = raw as Record<string, unknown>;
  if (typeof context.path !== "string") return null;
  if (typeof context.startLine !== "number" || typeof context.endLine !== "number") return null;
  return { path: context.path, startLine: context.startLine, endLine: context.endLine };
}

/**
 * Narrow untrusted `verdicts` to a `Record<string, Verdict>`. LENIENT like `context`:
 * it is not load-bearing, so anything other than a plain object whose EVERY value is
 * a known verdict degrades to `{}` and the WHOLE snapshot is kept.
 */
function parseVerdicts(raw: unknown): Record<string, Verdict> {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) return {};
  const out: Record<string, Verdict> = {};
  for (const [callId, value] of Object.entries(raw)) {
    if (value !== "pending" && value !== "accepted" && value !== "rejected") return {};
    out[callId] = value;
  }
  return out;
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
