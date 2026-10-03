/**
 * The stdio transport: `WcodeSession`.
 *
 * Spawns `wcode serve --stdio`, frames NDJSON over **stdout only** (stderr is
 * diagnostics and is NEVER parsed), and owns reply correlation. No `vscode`
 * import — it takes a logger, and the extension host wires it up.
 *
 * Session addressing (hard rule, live-verified): the seeded `sessions` push on
 * connect names the ROOT session's real id, and every subsequent request is
 * addressed to it. Never the `"remote"` placeholder — it resolves only on a
 * single-session server (`server.rs` `live.is_empty()` fallback) and returns
 * `unknown session remote` as soon as a roster exists. Get it wrong and the
 * chat is silently empty.
 *
 * Keep stdin open for the session's life: a client that closes it can lose the
 * replies still in flight.
 */
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { EventEmitter } from "node:events";

import { PROTOCOL_VERSION } from "./protocol.ts";
import type { AgentEvent, RawFrame, Request } from "./protocol.ts";

/** `frame.rs` `MAX_FRAME_BYTES` — the same 16 MiB line cap the server enforces. */
export const MAX_FRAME_BYTES = 16 * 1024 * 1024;

/** The lifecycle FSM. */
export type SessionState = "stopped" | "starting" | "ready" | "crashed";

/** A crash report surfaced to the host (the stderr tail is for the user). */
export interface CrashInfo {
  code: number | null;
  signal: NodeJS.Signals | null;
  stderrTail: string;
}

/** A logger the host supplies; the session itself never touches `vscode`. */
export interface Logger {
  info(message: string): void;
  error(message: string): void;
}

export interface WcodeSessionOptions {
  /** The resolved `wcode` binary (setting → PATH → a clear error). */
  binary: string;
  /** Extra args appended after `serve --stdio`. */
  args?: string[];
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  logger?: Logger;
  /** Crash restarts before giving up. Default 3. */
  maxRestarts?: number;
  /** Backoff (ms) per restart attempt. Default `[500, 1000, 2000]`. */
  backoffMs?: number[];
}

interface Pending {
  resolve: (event: AgentEvent) => void;
  reject: (err: Error) => void;
  timer: NodeJS.Timeout;
}

const NOOP_LOGGER: Logger = { info: () => {}, error: () => {} };

/**
 * The NDJSON splitter — the one line-oriented bug that bites. stdout arrives in
 * arbitrary chunks: a frame can be split across two `data` events, and two
 * frames can ride one chunk.
 *
 * Rules (matching `read_frame`'s framing, `frame.rs`):
 *   - split on `\n`, keep the trailing partial in `rest`;
 *   - trim, skip blank lines;
 *   - drop (never buffer) a line over `MAX_FRAME_BYTES`;
 *   - `JSON.parse` each complete line; a non-object or a throw drops THAT line.
 *
 * Deliberate divergence, owned: `read_frame` *aborts the connection* on an
 * over-cap or unparseable line; the client drops the line and continues, so one
 * bad frame cannot kill a live session. This is NOT "mirroring" the Rust side.
 */
export function feed(carry: string, chunk: string): { frames: RawFrame[]; rest: string } {
  let buffer = carry + chunk;
  const frames: RawFrame[] = [];

  let newline: number;
  while ((newline = buffer.indexOf("\n")) !== -1) {
    const line = buffer.slice(0, newline);
    buffer = buffer.slice(newline + 1);
    pushLine(frames, line);
  }

  // A trailing partial that already exceeds the cap is dropped, not buffered —
  // otherwise a peer that never sends `\n` grows the buffer without bound.
  if (byteLength(buffer) > MAX_FRAME_BYTES) buffer = "";

  return { frames, rest: buffer };
}

function pushLine(frames: RawFrame[], line: string): void {
  const trimmed = line.trim();
  if (trimmed === "") return; // blank: skipped (`frame.rs` blank-line loop)
  if (byteLength(trimmed) > MAX_FRAME_BYTES) return; // over-cap: dropped, not buffered

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return; // unparseable: drop this line, keep the connection
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return;

  const frame = parsed as RawFrame;
  if (typeof frame.type !== "string") return; // a frame needs a discriminant
  frames.push(frame);
}

function byteLength(s: string): number {
  return Buffer.byteLength(s, "utf8");
}

/**
 * One `wcode serve --stdio` child, framed and correlated.
 *
 * Events: `"event"` `(event, frame)`, `"state"` `(state)`, `"stderr"` `(line)`,
 * `"crash"` `(CrashInfo)`.
 */
export class WcodeSession extends EventEmitter {
  private child: ChildProcessWithoutNullStreams | null = null;
  private _state: SessionState = "stopped";
  private carry = "";
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private rootId: string | null = null;
  private restarts = 0;
  private readonly stderrTail: string[] = [];
  private stopping = false;
  private readyTimer: NodeJS.Timeout | null = null;
  private readonly readyWaiters: Array<() => void> = [];
  private startReject: ((err: Error) => void) | null = null;

  private readonly logger: Logger;
  private readonly maxRestarts: number;
  private readonly backoffMs: number[];

  private readonly options: WcodeSessionOptions;

  // No TypeScript parameter properties: Node's strip-only type stripping
  // rejects them, and these tests run under plain `node --test`.
  constructor(options: WcodeSessionOptions) {
    super();
    this.options = options;
    this.logger = options.logger ?? NOOP_LOGGER;
    this.maxRestarts = options.maxRestarts ?? 3;
    this.backoffMs = options.backoffMs ?? [500, 1000, 2000];
  }

  get state(): SessionState {
    return this._state;
  }

  /** The root session id learned from the seeded push (null until it arrives). */
  get rootSessionId(): string | null {
    return this.rootId;
  }

  /** Spawn (or restart) the child; resolves once the FSM reaches `ready`. */
  start(): Promise<void> {
    if (this._state === "ready") return Promise.resolve();
    if (this._state === "starting") {
      return new Promise<void>((resolve) => this.readyWaiters.push(resolve));
    }
    this.stopping = false;
    this.restarts = 0; // an explicit start (incl. Wcode: Restart) resets the budget
    return new Promise<void>((resolve, reject) => {
      this.readyWaiters.push(resolve);
      this.startReject = reject;
      this.spawnOnce();
    });
  }

  /**
   * Mint an id, write one frame to the child's stdin, and return the id. Every
   * request is addressed to the real root id — never `"remote"`.
   */
  send(request: Request): number {
    if (!this.child) throw new Error("wcode session is not running");
    if (!this.rootId) {
      throw new Error("wcode session has no root id yet (the seeded `sessions` push has not arrived)");
    }
    const id = this.nextId++;
    const frame = { v: PROTOCOL_VERSION, id, session: this.rootId, ...request };
    this.child.stdin.write(`${JSON.stringify(frame)}\n`);
    return id;
  }

  /** Send and resolve on the frame whose `reply_to` is this request's id. */
  ask(request: Request, timeoutMs = 30_000): Promise<AgentEvent> {
    const id = this.send(request);
    return new Promise<AgentEvent>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`ask(${request.type}) timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
    });
  }

  /** Kill the child and resolve on its exit. */
  async stop(): Promise<void> {
    this.stopping = true;
    this.clearReadyTimer();
    this.rejectPending(new Error("wcode session stopped"));
    const child = this.child;
    if (!child) {
      this.setState("stopped");
      return;
    }
    await new Promise<void>((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) return;
        settled = true;
        resolve();
      };
      child.once("exit", finish);
      child.kill();
      // A child that ignores SIGTERM must not hang deactivate forever.
      const guard = setTimeout(finish, 2000);
      guard.unref?.();
    });
    this.child = null;
    this.setState("stopped");
  }

  private spawnOnce(): void {
    this.setState("starting");
    this.carry = "";
    this.stderrTail.length = 0;
    this.rootId = null;

    let child: ChildProcessWithoutNullStreams;
    try {
      child = spawn(this.options.binary, ["serve", "--stdio", ...(this.options.args ?? [])], {
        cwd: this.options.cwd,
        env: this.options.env,
        stdio: ["pipe", "pipe", "pipe"],
      });
    } catch (err) {
      this.fail(err instanceof Error ? err : new Error(String(err)));
      return;
    }
    this.child = child;

    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => this.onStdout(chunk));
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => this.onStderr(chunk));
    child.on("error", (err: Error) => this.onSpawnError(err));
    child.on("exit", (code: number | null, signal: NodeJS.Signals | null) => this.onExit(code, signal));

    // `ready` on the first frame (the seeded push) or after 250 ms, whichever
    // comes first — a spawn that produced no error is good enough to proceed.
    this.readyTimer = setTimeout(() => this.markReady(), 250);
  }

  private onStdout(chunk: string): void {
    const { frames, rest } = feed(this.carry, chunk);
    this.carry = rest;
    for (const frame of frames) this.dispatch(frame);
    if (frames.length > 0) this.markReady();
  }

  private onStderr(chunk: string): void {
    for (const line of chunk.split("\n")) {
      const trimmed = line.trimEnd();
      if (trimmed === "") continue;
      this.stderrTail.push(trimmed);
      if (this.stderrTail.length > 40) this.stderrTail.shift();
      this.emit("stderr", trimmed);
    }
  }

  private dispatch(frame: RawFrame): void {
    // 1) The seeded roster push names the real root id — address there.
    if (frame.type === "sessions" && frame.reply_to === undefined && this.rootId === null) {
      const sessions = frame.sessions as Array<{ id?: string }> | undefined;
      const id = sessions?.[0]?.id;
      if (typeof id === "string") this.rootId = id;
    }
    // 2) Correlate a direct reply.
    if (typeof frame.reply_to === "number") {
      const pending = this.pending.get(frame.reply_to);
      if (pending) {
        this.pending.delete(frame.reply_to);
        clearTimeout(pending.timer);
        pending.resolve(frame as unknown as AgentEvent);
      }
    }
    // 3) Every frame is surfaced to the reducer (replies included — `history`
    //    seeds the transcript through the same channel).
    this.emit("event", frame as unknown as AgentEvent, frame);
  }

  private onSpawnError(err: Error): void {
    this.logger.error(`wcode spawn failed: ${err.message}`);
    const reject = this.startReject;
    this.startReject = null;
    this.fail(err);
    reject?.(err);
  }

  private onExit(code: number | null, signal: NodeJS.Signals | null): void {
    this.child = null;
    this.clearReadyTimer();
    this.rejectPending(new Error("wcode session exited"));
    if (this.stopping) {
      this.setState("stopped");
      return;
    }
    // A crash: surface the stderr tail, then restart within budget.
    this.setState("crashed");
    this.emit("crash", {
      code,
      signal,
      stderrTail: this.stderrTail.join("\n"),
    } satisfies CrashInfo);
    if (this.restarts < this.maxRestarts) {
      const delay = this.backoffMs[this.restarts] ?? this.backoffMs[this.backoffMs.length - 1] ?? 2000;
      this.restarts += 1;
      this.logger.error(
        `wcode crashed (${code ?? signal}); restart ${this.restarts}/${this.maxRestarts} in ${delay}ms`,
      );
      const timer = setTimeout(() => {
        if (!this.stopping) this.spawnOnce();
      }, delay);
      timer.unref?.();
    } else {
      this.logger.error(
        `wcode crashed ${this.restarts} times; giving up — run "wcode: Restart" to try again`,
      );
    }
  }

  private markReady(): void {
    this.clearReadyTimer();
    if (this._state !== "ready") {
      this.startReject = null;
      this.setState("ready");
    }
    const waiters = this.readyWaiters.splice(0);
    for (const waiter of waiters) waiter();
  }

  private fail(err: Error): void {
    this.clearReadyTimer();
    this.setState("crashed");
    this.emit("crash", { code: null, signal: null, stderrTail: this.stderrTail.join("\n") } satisfies CrashInfo);
    this.startReject = null;
    void err;
  }

  private clearReadyTimer(): void {
    if (this.readyTimer) {
      clearTimeout(this.readyTimer);
      this.readyTimer = null;
    }
  }

  private rejectPending(err: Error): void {
    for (const [, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.reject(err);
    }
    this.pending.clear();
  }

  private setState(state: SessionState): void {
    if (this._state === state) return;
    this._state = state;
    this.emit("state", state);
  }
}
