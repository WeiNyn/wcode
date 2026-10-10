/**
 * `SessionTab` — ONE workspace conversation (D013). Owns everything the extension
 * used to hold as a module singleton (`extension.ts:42-63`): its own `WcodeSession`
 * child, `SurfaceController`, `ViewState`, `HydratedSet`, `sessionArgs`,
 * `userStopped`, `reviewVerdicts`, and every handler the surface calls back into.
 *
 * The `Manager` (`manager.ts`) is the composition root: it creates tabs, binds the
 * docked view to the active one, and routes commands. A tab never reaches back to
 * VS Code's command registry directly — the two composition-root actions it cannot
 * perform alone (`/new` opens a NEW tab, `/resume` opens one too) go through injected
 * `SessionTabDeps` callbacks.
 *
 * A tab does NOT own an editor panel: the `Manager` maps panels to tabs (`/new`
 * opens a tab, `wcode: Open in Editor` re-homes the active session into one). This
 * keeps the sidebar-only flow (today's default) intact.
 */
import * as fs from "node:fs";
import * as vscode from "vscode";

import { cancelTargets } from "./cancel.ts";
import { openDiff as openDiffInEditor, revertDiff } from "./diffProvider.ts";
import { findCommand } from "./commands.ts";
import {
  appendUser,
  HydratedSet,
  initialState,
  planModePending,
  planModeRevert,
  reduce,
  target as setTarget,
  targetLabel,
  type ViewState,
} from "./reducer.ts";
import { setVerdict, type Verdict } from "./review.ts";
import { WcodeSession, type CrashInfo, type SessionState } from "./session.ts";
import { isUnsupportedStdio, unsupportedStdioMessage } from "./startup.ts";
import { SurfaceController, type SurfaceHandlers } from "./surface.ts";
import { commandAction, stickyLabel, tabLabel } from "./tabs.ts";
import type { AgentEvent, RawFrame } from "./protocol.ts";
import type { ViewPrefs } from "./webview.ts";

/** What a tab needs from its `Manager` (the composition root). */
export interface SessionTabDeps {
  /** The extension's install root (for the surface's `media/`). */
  extensionUri: vscode.Uri;
  /** The first workspace folder, or `undefined` (read live — folders can change). */
  workspaceRoot(): string | undefined;
  /** The ONE OutputChannel (shared; every line carries this tab's label — see `log`). */
  output: vscode.OutputChannel;
  /** The diff feature's diagnostic sink (the same OutputChannel). */
  logLine(message: string): void;
  /** Open a NEW session in a new tab (`/new`). */
  openTab(args: string[]): void;
  /** Open a NEW tab resuming a session (`/resume`; a picker when `arg` is empty). */
  resumeTab(arg: string): void;
  /** Open a NEW tab with a project team (`/team`; a picker when `arg` is empty). */
  teamTab(arg: string): void;
  /** This tab's session FSM state changed (the Manager repaints the status bar). */
  onStateChange(tab: SessionTab): void;
  /** This tab's label changed (the Manager retitles the editor tab). */
  onLabelChange(tab: SessionTab): void;
  /** A run finished (`agent_end`) for `sessionId` — the root or a member. */
  onTurnEnd(tab: SessionTab, sessionId: string): void;
  /** The current presentation preferences (D015) — read live from settings. */
  prefs(): ViewPrefs;
}

/** The effort vocabulary a picker offers (the Responses enum; Chat is free-style). */
export const EFFORT_LEVELS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

/** Options for a new tab. */
export interface SessionTabOptions {
  /** A stable id (the `Manager` mints it). */
  id: string;
  /** 1-based open order — the `wcode <n>` placeholder (D013 C). */
  index: number;
  /** Extra args after `serve --stdio` (e.g. `["--resume", path]`). */
  args?: string[];
  /** The resolved session path this tab resumed, for the pre-message label. */
  resumePath?: string | null;
  /** An explicit pre-message label (a `/team` tab shows the team name). */
  label?: string;
}

export class SessionTab {
  readonly id: string;
  readonly index: number;
  readonly controller: SurfaceController;

  private readonly deps: SessionTabDeps;
  private readonly hydrated = new HydratedSet();
  private readonly resumePath: string | null;

  private label: string;
  private session: WcodeSession | undefined;
  private state: ViewState;
  private args: string[];
  private userStopped = false;
  private verdicts: Record<string, Verdict> = {};
  private disposed = false;

  constructor(deps: SessionTabDeps, options: SessionTabOptions) {
    this.deps = deps;
    this.id = options.id;
    this.index = options.index;
    this.args = options.args ?? [];
    this.resumePath = options.resumePath ?? null;
    this.state = initialState();
    this.label = options.label ?? tabLabel(this.state, this.resumePath, this.index);
    this.controller = new SurfaceController(deps.extensionUri, this.handlers(), this.state, deps.prefs());
    this.controller.setContext(null);
  }

  /* ------------------------------------------------------------- accessors */

  /** The tab's current label (D013 C). */
  get currentLabel(): string {
    return this.label;
  }

  /** The shared OutputChannel, viewed per-tab: every line carries this tab's
   *  label (W010 P4) so two tabs' interleaved diagnostics stay attributable. */
  private log(line: string): void {
    this.deps.output.appendLine(`[${this.label}] ${line}`);
  }

  /** This session's view model — the `wcode.openDiff` command needs it. */
  get viewState(): ViewState {
    return this.state;
  }

  /** The FSM state of this tab's child (the status bar reads it). */
  get sessionState(): SessionState {
    return this.session?.state ?? "stopped";
  }

  /** The focused member id (the palette's `member.stop` has no menu target). */
  get targeted(): string | null {
    return this.state.targeted;
  }

  /* -------------------------------------------------------- lifecycle */

  /** Start the child IFF it is not already up and the user did not stop it. */
  ensureSession(): void {
    if (this.userStopped) return;
    if (this.session && (this.session.state === "ready" || this.session.state === "starting")) return;
    void this.start();
  }

  /** Start (or reveal, when already running) this tab's `serve --stdio` child. */
  async start(): Promise<void> {
    if (this.disposed) return;
    this.userStopped = false;
    const surface = this.controller;

    if (this.session && (this.session.state === "ready" || this.session.state === "starting")) {
      this.log("session already running");
      surface.reveal();
      return;
    }

    let binary: string;
    try {
      binary = resolveBinary();
    } catch (err) {
      const message = errMessage(err);
      this.log(`error: ${message}`);
      surface.setSession({ state: "crashed", stderrTail: message });
      void vscode.window.showErrorMessage(`wcode: ${message}`);
      return;
    }

    this.log(`starting: ${binary} serve --stdio ${this.args.join(" ")}`.trimEnd());
    const next = new WcodeSession({
      binary,
      args: this.args,
      cwd: this.deps.workspaceRoot(),
      logger: { info: (m) => this.log(m), error: (m) => this.log(`error: ${m}`) },
    });
    this.session = next;
    this.hydrated.reset();
    surface.setSession({ id: null, state: next.state, stderrTail: "" });

    next.on("event", (event: AgentEvent, frame: RawFrame) => {
      const correlation = typeof frame.reply_to === "number" ? ` (reply_to ${frame.reply_to})` : "";
      this.log(`← ${frame.session} ${frame.type}${correlation}`);
      // The frame's `session` is the origin (a fanned event) or the target (a
      // reply) — the key every per-session arm writes.
      this.state = reduce(this.state, event, frame.session);
      this.ensureTarget();
      this.refreshLabel();
      // `message_end` (and the other settled events) always send the final state.
      surface.update(this.state, isSettled(event));
      // A completed run: notify the user (unless they are watching this tab).
      if (event.type === "agent_end") this.deps.onTurnEnd(this, frame.session);
    });
    next.on("stderr", (line: string) => this.log(`stderr: ${line}`));
    next.on("state", (state: SessionState) => {
      surface.setSession({ state });
      this.deps.onStateChange(this);
    });
    next.on("crash", (info: CrashInfo) => {
      this.log("session crashed");
      if (info.stderrTail !== "") this.log(info.stderrTail);
      // The first-run case: an older `wcode` on PATH rejects `--stdio`. Say what
      // is wrong, WHICH binary, and both ways out — in the existing crash surface.
      if (isUnsupportedStdio(info.code, info.stderrTail)) {
        const message = unsupportedStdioMessage(binary);
        this.log(message);
        surface.setSession({ state: "crashed", stderrTail: message });
        return;
      }
      surface.setSession({ state: "crashed", stderrTail: info.stderrTail });
    });

    try {
      await next.start();
    } catch (err) {
      const message = errMessage(err);
      this.log(`failed to start: ${message}`);
      surface.setSession({ state: "crashed", stderrTail: message });
      void vscode.window.showErrorMessage(
        `wcode failed to start (${message}). Set "wcode.path" or put wcode on PATH.`,
      );
      return;
    }

    this.log(`ready — root session ${next.rootSessionId}`);
    surface.setSession({ id: next.rootSessionId, state: next.state });
    // The seeded push has already folded, so the target is the root.
    if (this.state.targeted !== null) this.hydrate(this.state.targeted);
  }

  /** Stop this tab's child. The diff registry is SHARED — never cleared here (D013). */
  async stop(): Promise<void> {
    const current = this.session;
    this.session = undefined;
    this.hydrated.reset();
    if (current) {
      await current.stop();
      this.log("stopped");
    }
    this.verdicts = {};
    this.controller.setSession({ state: "stopped" });
    this.deps.onStateChange(this);
  }

  /** `wcode: Stop Session` — an explicit stop survives a webview `ready`. */
  stopByUser(): void {
    this.userStopped = true;
    void this.stop();
  }

  /** Stop, wipe the view state, and start a FRESH child with `args` (`/reload`). */
  async restart(args: string[]): Promise<void> {
    await this.stop();
    this.args = args;
    this.state = initialState();
    this.verdicts = {};
    this.controller.update(this.state, true);
    this.controller.setMode(this.deps.prefs().mode); // the mode RESETS to the configured default
    await this.start();
  }

  /** Reload THIS tab (`/reload`, `wcode: Restart`) — the same args, a new child. */
  reload(): void {
    void this.restart(this.args);
  }

  /** The tab is going away: dispose the surface and stop the child. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.controller.dispose();
    void this.session?.stop();
    this.session = undefined;
  }

  /* --------------------------------------------------------- member verbs */

  /** Retarget the surface at a member (and hydrate that member's transcript). */
  focusMember(id: string): void {
    this.state = setTarget(this.state, id);
    if (this.controller.currentTarget !== id) {
      this.controller.setTarget(id); // fires `onTarget`, which hydrates
      return;
    }
    this.hydrate(id);
  }

  /** `peek` = `Request::Status`: a lean read, no model call, not the transcript. */
  async peekMember(id: string | undefined): Promise<void> {
    const session = this.session;
    if (id === undefined || !session) return;
    try {
      const reply = await session.ask({ type: "status" }, id, 5_000);
      const work = reply.type === "status" ? reply.last_assistant_text ?? "(no work yet)" : `(${reply.type})`;
      const member = this.state.members.find((m) => m.id === id);
      void vscode.window.showInformationMessage(
        `wcode [${member?.state ?? "?"}] ${targetLabel(this.state, id)} · ${clip(work, 140)}`,
      );
    } catch (err) {
      void vscode.window.showWarningMessage(`wcode: peek failed: ${errMessage(err)}`);
    }
  }

  /** `wcode.member.ask`: prompt for the question, then `sideAsk`. */
  async askMember(id: string | undefined): Promise<void> {
    if (id === undefined || !this.session) return;
    const question = await vscode.window.showInputBox({
      title: `wcode: ask ${targetLabel(this.state, id)}`,
      prompt: "A side question: answered from its context, no turn, not recorded",
      placeHolder: "why did you choose that approach?",
    });
    if (question === undefined || question.trim() === "") return;
    await this.sideAsk(id, question);
  }

  /** `SideAsk`: a side question — no turn, not recorded. The `/btw` command's engine. */
  async sideAsk(id: string, question: string): Promise<void> {
    const session = this.session;
    if (!session || id === "") return;
    try {
      const reply = await session.ask({ type: "side_ask", text: question }, id, 60_000);
      const answer = reply.type === "side_answer" ? reply.text : `(${reply.type})`;
      void vscode.window.showInformationMessage(`wcode: ${clip(answer, 400)}`);
    } catch (err) {
      // `Ask` on a busy member can time out — say "busy", not "failed".
      void vscode.window.showWarningMessage(
        `wcode: ${targetLabel(this.state, id)} did not answer in time (it may be busy): ${errMessage(err)}`,
      );
    }
  }

  /** `stop` = `Request::Cancel`; the row's state settles via the roster push. */
  stopMember(id: string | undefined): void {
    if (id === undefined) return;
    try {
      this.session?.send({ type: "cancel" }, id);
    } catch (err) {
      this.log(`stop failed: ${errMessage(err)}`);
    }
  }

  /** `SetPlanMode` → `Ack` (infallible): optimistic, settled on `Ack`, reverted on `Error`. */
  togglePlan(): void {
    const session = this.session;
    if (!session) return;
    const on = !this.state.status.planMode;
    this.state = planModePending(this.state, on);
    this.controller.update(this.state, true);
    const target = this.controller.currentTarget ?? undefined;
    try {
      session.send({ type: "set_plan_mode", on }, target);
    } catch (err) {
      this.state = planModeRevert(this.state);
      this.controller.update(this.state, true);
      this.log(`plan toggle failed: ${errMessage(err)}`);
    }
  }

  /* --------------------------------------------------------- panel io */

  /** The `wcode.openDiff` command / a transcript row: open this session's diff. */
  showDiff(callId?: string): Promise<void> {
    return openDiffInEditor(this.state, this.deps.workspaceRoot(), callId, this.deps.logLine);
  }

  /* ------------------------------------------------------------ private */

  private handlers(): SurfaceHandlers {
    return {
      onSubmit: (text: string, target: string | null) => this.submit(text, target),
      onCancel: (target: string | null) => this.cancel(target),
      onSteer: (text: string, target: string | null) => this.steer(text, target),
      onOpenDiff: (callId: string) => void this.showDiff(callId),
      onRevealFile: (path: string, line?: number) => void revealFile(path, line),
      onTarget: (target: string | null) => {
        if (target !== null) this.hydrate(target);
      },
      onTogglePlan: () => this.togglePlan(),
      onCommand: (name: string, arg: string) => this.runCommand(name, arg),
      onReady: () => this.ensureSession(),
      onFocusMember: (id: string) => {
        // Defence-in-depth: ignore an id not in the roster (a bogus id would
        // retarget to an empty transcript and waste a `get_history`).
        if (this.state.members.some((m) => m.id === id)) this.focusMember(id);
      },
      onReview: (callId: string, verdict: "accept" | "reject") => void this.reviewChange(callId, verdict),
    };
  }

  private submit(text: string, target: string | null): void {
    const session = this.session;
    if (!session) {
      this.log("submit ignored: no session is running");
      return;
    }
    const address = target ?? this.state.targeted ?? session.rootSessionId;
    if (address === null) {
      this.log("submit ignored: no target member yet");
      return;
    }
    try {
      // The session streams only the assistant's reply; echo the user locally
      // into THAT member's transcript.
      this.state = appendUser(this.state, text, address);
      this.controller.update(this.state, true);
      session.send({ type: "submit", text }, address);
    } catch (err) {
      this.log(`submit failed: ${errMessage(err)}`);
    }
  }

  private cancel(target: string | null): void {
    try {
      // Focus cancels the focused member; All cancels the WHOLE session (root + runners).
      for (const id of cancelTargets(target, this.state.members, this.session?.rootSessionId ?? null)) {
        this.session?.send({ type: "cancel" }, id);
      }
    } catch (err) {
      this.log(`cancel failed: ${errMessage(err)}`);
    }
  }

  private steer(text: string, target: string | null): void {
    try {
      this.session?.send({ type: "interrupt", content: text }, target ?? undefined);
    } catch (err) {
      this.log(`steer failed: ${errMessage(err)}`);
    }
  }

  /**
   * The `/` menu's dispatch. The webview names a command; the host decides what
   * it IS: a composition-root action (a fresh tab, a picker) or a plain `Request`.
   * An unknown name is refused — the webview's list is the only source, but a
   * hand-crafted `postMessage` is not trusted.
   */
  private runCommand(name: string, arg: string): void {
    const command = findCommand(name);
    if (command === undefined) {
      this.log(`unknown command: /${name}`);
      return;
    }
    const target = this.controller.currentTarget ?? undefined;

    // Composition-root / tab-level actions (they open a tab or a picker). `plan`
    // and `status` route as "request" but are handled locally, below.
    switch (commandAction(command)) {
      case "new":
        // A FRESH session in a NEW tab (D013 E) — the CLI's `/new`.
        this.deps.openTab([]);
        return;
      case "resume":
        this.deps.resumeTab(arg);
        return;
      case "reload":
        // The SAME args, a new child, THIS tab.
        this.reload();
        return;
      case "members":
        void this.pickConnectionSession();
        return;
      case "team":
        this.deps.teamTab(arg);
        return;
      case "request":
        break;
    }

    // Local (non-wire) commands that take the "request" route.
    switch (command.name) {
      case "plan":
        this.togglePlan();
        return;
      case "status":
        void this.peekMember(target);
        return;
    }

    const session = this.session;
    if (!session) {
      this.log(`/${command.name} ignored: no session is running`);
      return;
    }
    if (command.requiresArg === true && arg === "") {
      void vscode.window.showWarningMessage(`wcode: /${command.name} needs an argument`);
      return;
    }
    try {
      switch (command.name) {
        case "model":
          if (arg === "") {
            void this.pickModel(target);
            break;
          }
          session.send({ type: "set_model", model: arg }, target);
          break;
        case "effort":
          if (arg === "") {
            void this.pickEffort(target);
            break;
          }
          // `-`/`off`/`none` clears it, mirroring the CLI's `/effort -`.
          session.send({ type: "set_effort", effort: isClearEffort(arg) ? null : arg }, target);
          break;
        case "btw":
          void this.sideAsk(target ?? this.state.targeted ?? "", arg);
          break;
        case "compact":
          session.send({ type: "compact", instructions: arg === "" ? null : arg }, target);
          break;
      }
    } catch (err) {
      this.log(`/${command.name} failed: ${errMessage(err)}`);
    }
  }

  /**
   * `/model` with NO argument: ask the transport for the catalog ON DEMAND (no startup
   * caching, no race), then a native QuickPick marks the current model and sets the pick.
   */
  private async pickModel(target: string | undefined): Promise<void> {
    const session = this.session;
    if (!session) return;
    const reply = await session.ask({ type: "list_models" }).catch(() => undefined);
    const models = reply?.type === "models" ? reply.models : [];
    if (models.length === 0) {
      void vscode.window.showInformationMessage("wcode: no model catalog from this provider");
      return;
    }
    const current = this.state.members.find((m) => m.id === (target ?? this.state.targeted))?.model;
    const items: Array<vscode.QuickPickItem & { id: string }> = models.map((id) => ({
      label: id,
      description: id === current ? "current" : undefined,
      id,
    }));
    const picked = await vscode.window.showQuickPick(items, {
      title: "wcode: switch model",
      placeHolder: current ?? "pick a model",
    });
    if (picked !== undefined) session.send({ type: "set_model", model: picked.id }, target);
  }

  /** `/effort` with NO argument: a native QuickPick over the levels, plus a `clear` entry. */
  private async pickEffort(target: string | undefined): Promise<void> {
    const session = this.session;
    if (!session) return;
    const items: Array<vscode.QuickPickItem & { id: string; clear: boolean }> = [
      ...EFFORT_LEVELS.map((level) => ({ label: level as string, id: level as string, clear: false })),
      { label: "clear", description: "send nothing", id: "-", clear: true },
    ];
    const picked = await vscode.window.showQuickPick(items, { title: "wcode: reasoning effort" });
    if (picked !== undefined) {
      session.send({ type: "set_effort", effort: picked.clear ? null : picked.id }, target);
    }
  }

  /** `/sessions`: the LIVE sessions in THIS tab's connection; picking one retargets. */
  private async pickConnectionSession(): Promise<void> {
    const picked = await vscode.window.showQuickPick(
      this.state.members.map((member) => ({
        label: targetLabel(this.state, member.id),
        description: `${member.state}${member.model === undefined ? "" : ` · ${member.model}`}`,
        detail: member.id,
        id: member.id,
      })),
      { title: "wcode: sessions in this connection" },
    );
    if (picked !== undefined) this.focusMember(picked.id);
  }

  /** The first roster arrival picks a target, so the surface is never unattached. */
  private ensureTarget(): void {
    if (this.state.targeted !== null) return;
    const root = this.state.members.find((m) => m.isRoot) ?? this.state.members[0];
    if (root === undefined) return;
    this.focusMember(root.id);
  }

  /** Ask a member for its history — ONCE per session, addressed to ITS id. */
  private hydrate(id: string): void {
    const session = this.session;
    // The guard is `reducer.ts`'s pure `HydratedSet` (so it is tested); a second
    // retarget of the same session must not re-ask (that would duplicate it).
    if (!session || !this.hydrated.claim(id)) return;
    try {
      session.send({ type: "get_history" }, id);
    } catch (err) {
      this.hydrated.release(id);
      this.log(`hydrate ${id} failed: ${errMessage(err)}`);
    }
  }

  /** Re-derive the sticky label; notify the Manager only when it actually changed. */
  private refreshLabel(): void {
    const candidate = tabLabel(this.state, this.resumePath, this.index);
    const next = stickyLabel(this.label, candidate);
    if (next === this.label) return;
    this.label = next;
    this.deps.onLabelChange(this);
  }

  /**
   * Settle a change review. Accept is a NO-OP on disk (the edit already landed);
   * Reject reverse-applies the diff and WRITES the before-image. The verdict is set
   * only when a write SUCCEEDED — the panel never claims a write that did not happen.
   */
  private async reviewChange(callId: string, verdict: "accept" | "reject"): Promise<void> {
    if (verdict === "accept") {
      this.verdicts = setVerdict(this.verdicts, callId, "accepted");
      this.controller.setVerdicts(this.verdicts);
      return;
    }
    // Reject = reverse-apply the tool's diff to the CURRENT file and write the before-image.
    const ok = await revertDiff(this.state, this.deps.workspaceRoot(), callId, this.deps.logLine);
    if (ok) {
      this.verdicts = setVerdict(this.verdicts, callId, "rejected");
      this.controller.setVerdicts(this.verdicts);
    }
    // !ok: `revertDiff` already warned + logged (the file moved on -> reverseApply is
    // null); the verdict stays "pending", so the surface never claims a write that did not.
  }
}

/* ------------------------------------------------------------------ helpers */

/** Events whose state must be posted immediately (not coalesced). */
function isSettled(event: AgentEvent): boolean {
  switch (event.type) {
    case "message_end":
    case "agent_end":
    case "error":
    case "history":
      return true;
    default:
      return false;
  }
}

/** The spellings of `/effort` that CLEAR it (the CLI's `/effort -`). Pure. */
function isClearEffort(arg: string): boolean {
  const value = arg.trim().toLowerCase();
  return value === "-" || value === "off" || value === "none";
}

async function revealFile(path: string, line?: number): Promise<void> {
  try {
    const document = await vscode.workspace.openTextDocument(path);
    const editor = await vscode.window.showTextDocument(document, { preview: true });
    if (typeof line === "number" && line > 0) {
      const position = new vscode.Position(line - 1, 0);
      editor.selection = new vscode.Selection(position, position);
      editor.revealRange(new vscode.Range(position, position));
    }
  } catch (err) {
    void vscode.window.showWarningMessage(`wcode: could not open ${path}: ${errMessage(err)}`);
  }
}

/**
 * Resolve the binary: the `wcode.path` setting, then `PATH`. A configured path
 * that does not exist is a loud error, never a silent fallback.
 */
function resolveBinary(): string {
  const configured = vscode.workspace.getConfiguration("wcode").get<string>("path")?.trim() ?? "";
  if (configured === "") return "wcode"; // resolved from PATH; a miss surfaces as a spawn error
  if (!fs.existsSync(configured)) {
    throw new Error(`wcode.path points at a file that does not exist: ${configured}`);
  }
  return configured;
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
