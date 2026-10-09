/**
 * The extension host entry point.
 *
 * `wcode.start` opens (or reveals) the webview chat panel and starts the
 * `wcode serve --stdio` child; `wcode.stop` stops it; `wcode.restart` restarts.
 * The OutputChannel stays for diagnostics (every frame, every stderr line).
 *
 * V2: ONE dockable surface — a `WebviewView` (sidebar/panel) + an editor `WebviewPanel`,
 * both fed by the one `SurfaceController`. Every send is addressed to a member's session
 * id, which the server demuxes against its live roster. The team + tasks render INSIDE
 * the surface; the `member` verbs (peek = `Status`, ask = `SideAsk`, stop = `Cancel`) are
 * INERT until a rail-row menu lands — plus `Submit`/`Interrupt` and `SetPlanMode`.
 */
import * as fs from "node:fs";
import * as vscode from "vscode";

import { findCommand } from "./commands.ts";
import { clearDiffs, openDiff, registerDiffProvider, revertDiff } from "./diffProvider.ts";
import { setVerdict, type Verdict } from "./review.ts";
import { ChatPanel } from "./panel.ts";
import { SurfaceController, type SurfaceHandlers } from "./surface.ts";
import { SurfaceViewProvider } from "./webviewView.ts";
import type { SelectionContext } from "./webview.ts";
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
import { WcodeSession, type CrashInfo, type SessionState } from "./session.ts";
import { listSessions, sessionDir } from "./sessions.ts";
import { isUnsupportedStdio, unsupportedStdioMessage } from "./startup.ts";
import type { AgentEvent, RawFrame } from "./protocol.ts";

let session: WcodeSession | undefined;
let viewState: ViewState = initialState();
let output: vscode.OutputChannel | undefined;
let status: vscode.StatusBarItem | undefined;
let extensionUri: vscode.Uri | undefined;
let controller: SurfaceController | undefined;
/**
 * Extra args appended after `serve --stdio` for the NEXT child. `/resume` sets
 * them; `/new` clears them. They persist so `/reload` re-runs the same session.
 */
let sessionArgs: string[] = [];
/**
 * Set by `wcode: Stop Session`, so an explicit stop is not undone by the next
 * `ready` (a webview reload posts one). Cleared by `startSession`.
 */
let userStopped = false;
/** Session ids whose `GetHistory` has been asked for (once each, per child). */
let hydrated = new HydratedSet();
/** The last non-empty editor selection — STICKY (survives focus moving to the webview). */
let lastSelection: SelectionContext | null = null;
/** Per-callId review verdicts. Reset when a fresh child starts. */
let reviewVerdicts: Record<string, Verdict> = {};

export function activate(context: vscode.ExtensionContext): void {
  extensionUri = context.extensionUri;
  output = vscode.window.createOutputChannel("wcode");
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  status.command = "wcode.restart";
  status.tooltip = "wcode session: click to restart";
  context.subscriptions.push(output, status);
  registerDiffProvider(context, logLine);

  const surface = new SurfaceController(context.extensionUri, surfaceHandlers(), viewState);
  controller = surface;
  // ONE view provider (the dockable sidebar/panel home) + an editor-tab host command.
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider("wcode.surface", new SurfaceViewProvider(surface)),
    surface,
    vscode.commands.registerCommand("wcode.openInEditor", () =>
      ChatPanel.createOrShow(requireExtensionUri(), surface),
    ),
  );

  // The selection is STICKY: `undefined` (focus moved to the webview/terminal)
  // KEEPS the last selection — clearing there is the bug that makes the chip
  // vanish as you go to type.
  context.subscriptions.push(
    vscode.window.onDidChangeTextEditorSelection((event) => {
      lastSelection = selectionOf(event.textEditor);
      controller?.setContext(lastSelection);
    }),
    vscode.window.onDidChangeActiveTextEditor((editor) => {
      if (editor === undefined) return;
      lastSelection = selectionOf(editor);
      controller?.setContext(lastSelection);
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("wcode.start", () => void startSession()),
    vscode.commands.registerCommand("wcode.stop", () => {
      userStopped = true;
      void stopSession();
    }),
    vscode.commands.registerCommand("wcode.restart", () => void restartSession(sessionArgs)),
    vscode.commands.registerCommand("wcode.openDiff", () =>
      void openDiff(viewState, workspaceRoot(), undefined, logLine),
    ),
    vscode.commands.registerCommand("wcode.member.focus", (id?: unknown) => {
      if (typeof id === "string") focusMember(id);
    }),
    // INERT until a rail-row menu lands (a follow-up): they took a tree element before,
    // so with no menu there is no id to pass — a palette call is a silent no-op.
    vscode.commands.registerCommand("wcode.member.peek", () => void peekMember(undefined)),
    vscode.commands.registerCommand("wcode.member.ask", () => void askMember(undefined)),
    vscode.commands.registerCommand("wcode.member.stop", () => stopMember(undefined)),
    vscode.commands.registerCommand("wcode.plan.toggle", () => togglePlan()),
  );
}

export function deactivate(): Thenable<void> | undefined {
  controller?.dispose();
  clearDiffs();
  return session?.stop();
}

/* ----------------------------------------------------------------- commands */

/**
 * Opening the surface IS the intent to work, so the panel starts the session
 * itself — `wcode: Start Session` stays for a manual start after a stop.
 *
 * Refuses when the user explicitly stopped (`userStopped`) or a child is already
 * running: the webview posts `ready` on every reload, and a stop must survive it.
 */
function ensureSession(): void {
  if (userStopped) return;
  if (session && (session.state === "ready" || session.state === "starting")) return;
  void startSession();
}

async function startSession(): Promise<void> {
  userStopped = false;
  const channel = ensureOutput();
  // Seed the chip ONCE when the surface opens; the listeners keep it in sync after.
  const editor = vscode.window.activeTextEditor;
  if (editor !== undefined) lastSelection = selectionOf(editor);
  controller?.setContext(lastSelection);

  if (session && (session.state === "ready" || session.state === "starting")) {
    channel.appendLine("session already running");
    controller?.reveal();
    return;
  }

  let binary: string;
  try {
    binary = resolveBinary();
  } catch (err) {
    const message = errMessage(err);
    channel.appendLine(`error: ${message}`);
    controller?.setSession({ state: "crashed", stderrTail: message });
    void vscode.window.showErrorMessage(`wcode: ${message}`);
    return;
  }

  channel.appendLine(`starting: ${binary} serve --stdio ${sessionArgs.join(" ")}`.trimEnd());
  const next = new WcodeSession({
    binary,
    args: sessionArgs,
    cwd: workspaceRoot(),
    logger: { info: (m) => channel.appendLine(m), error: (m) => channel.appendLine(`error: ${m}`) },
  });
  session = next;
  hydrated.reset();
  controller?.setSession({ id: null, state: next.state, stderrTail: "" });

  next.on("event", (event: AgentEvent, frame: RawFrame) => {
    const correlation = typeof frame.reply_to === "number" ? ` (reply_to ${frame.reply_to})` : "";
    channel.appendLine(`← ${frame.session} ${frame.type}${correlation}`);
    // The frame's `session` is the origin (a fanned event) or the target (a
    // reply) — the key every per-session arm writes.
    viewState = reduce(viewState, event, frame.session);
    ensureTarget();
    // The rail renders INSIDE the surface from the snapshot — no separate sidebar push.
    // `message_end` (and the other settled events) always send the final state.
    controller?.update(viewState, isSettled(event));
  });
  next.on("stderr", (line: string) => channel.appendLine(`stderr: ${line}`));
  next.on("state", (state: SessionState) => {
    if (status) {
      status.text = `wcode: ${state}`;
      status.show();
    }
    controller?.setSession({ state });
  });
  next.on("crash", (info: CrashInfo) => {
    channel.appendLine("session crashed");
    if (info.stderrTail !== "") channel.appendLine(info.stderrTail);
    // The first-run case: an older `wcode` on PATH rejects `--stdio`. Say what
    // is wrong, WHICH binary, and both ways out — in the existing crash surface.
    if (isUnsupportedStdio(info.code, info.stderrTail)) {
      const message = unsupportedStdioMessage(binary);
      channel.appendLine(message);
      controller?.setSession({ state: "crashed", stderrTail: message });
      return;
    }
    controller?.setSession({ state: "crashed", stderrTail: info.stderrTail });
  });

  try {
    await next.start();
  } catch (err) {
    const message = errMessage(err);
    channel.appendLine(`failed to start: ${message}`);
    controller?.setSession({ state: "crashed", stderrTail: message });
    void vscode.window.showErrorMessage(
      `wcode failed to start (${message}). Set "wcode.path" or put wcode on PATH.`,
    );
    return;
  }

  channel.appendLine(`ready — root session ${next.rootSessionId}`);
  controller?.setSession({ id: next.rootSessionId, state: next.state });
  // The seeded push has already folded, so the target is the root.
  if (viewState.targeted !== null) hydrate(viewState.targeted);
}

async function stopSession(): Promise<void> {
  const current = session;
  session = undefined;
  hydrated.reset();
  if (current) {
    await current.stop();
    ensureOutput().appendLine("stopped");
  }
  // A restart spawns a fresh child; the before-images belong to the old one.
  clearDiffs();
  reviewVerdicts = {};
  controller?.setSession({ state: "stopped" });
}

/**
 * Stop, wipe the view state, and start a FRESH child with `args`. The state must
 * start empty: the new session knows nothing of the old conversation.
 */
async function restartSession(args: string[]): Promise<void> {
  await stopSession();
  sessionArgs = args;
  // A restart spawns a *fresh* child with empty history, so the state must start
  // empty too — otherwise the panel shows a conversation the new session knows
  // nothing about.
  viewState = initialState();
  reviewVerdicts = {};
  controller?.update(viewState, true);
  controller?.setMode("all"); // the mode RESETS on a fresh child
  await startSession();
}

/* --------------------------------------------------------------- / commands */

/**
 * The `/` menu's dispatch. The webview names a command; the host decides what it
 * IS: a composition-root action (a fresh child, a picker) or a plain `Request`.
 * An unknown name is refused — the webview's list is the only source, but a
 * hand-crafted `postMessage` is not trusted.
 */
function runCommand(name: string, arg: string): void {
  const channel = ensureOutput();
  const command = findCommand(name);
  if (command === undefined) {
    channel.appendLine(`unknown command: /${name}`);
    return;
  }
  const target = controller?.currentTarget ?? undefined;

  // Composition-root actions: they respawn the child, so no wire request exists.
  switch (command.name) {
    case "new":
      // A FRESH session: a new session file and a new team (the CLI's `/new`).
      void restartSession([]);
      return;
    case "reload":
      // The SAME args, a new child: config re-read, `serve` re-run.
      void restartSession(sessionArgs);
      return;
    case "resume":
      void resumeSession(arg);
      return;
    case "plan":
      togglePlan();
      return;
    case "sessions":
      void pickConnectionSession();
      return;
    case "status":
      void peekMember(target);
      return;
  }

  if (!session) {
    channel.appendLine(`/${command.name} ignored: no session is running`);
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
          void pickModel(target);
          break;
        }
        session.send({ type: "set_model", model: arg }, target);
        break;
      case "effort":
        if (arg === "") {
          void pickEffort(target);
          break;
        }
        // `-`/`off`/`none` clears it, mirroring the CLI's `/effort -`.
        session.send({ type: "set_effort", effort: isClearEffort(arg) ? null : arg }, target);
        break;
      case "btw":
        void sideAsk(target ?? viewState.targeted ?? "", arg);
        break;
      case "compact":
        session.send({ type: "compact", instructions: arg === "" ? null : arg }, target);
        break;
    }
  } catch (err) {
    channel.appendLine(`/${command.name} failed: ${errMessage(err)}`);
  }
}

/** The effort vocabulary a picker offers (the Responses enum; Chat is free-style). */
const EFFORT_LEVELS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

/**
 * `/model` with NO argument: ask the transport for the catalog ON DEMAND (no startup
 * caching, no race), then a native QuickPick marks the current model and sets the pick.
 * The reply is the `AgentEvent::Models { models }` a `ListModels` is answered with; it
 * ALSO flows through the frame handler into the `models` reducer arm (a harmless side
 * effect of the on-demand ask).
 */
async function pickModel(target: string | undefined): Promise<void> {
  if (!session) return;
  const reply = await session.ask({ type: "list_models" }).catch(() => undefined);
  const models = reply?.type === "models" ? reply.models : [];
  if (models.length === 0) {
    void vscode.window.showInformationMessage("wcode: no model catalog from this provider");
    return;
  }
  const current = viewState.members.find((m) => m.id === (target ?? viewState.targeted))?.model;
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

/**
 * `/effort` with NO argument: a native QuickPick over the levels, plus a `clear` entry
 * (`-`, sending nothing). A TYPED `/effort <level>` still posts whatever the user wrote
 * (Chat is free-style) — see the non-empty arm in `runCommand`.
 */
async function pickEffort(target: string | undefined): Promise<void> {
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

/** The spellings of `/effort` that CLEAR it (the CLI's `/effort -`). Pure. */
function isClearEffort(arg: string): boolean {
  const value = arg.trim().toLowerCase();
  return value === "-" || value === "off" || value === "none";
}

/**
 * `/resume`: an explicit path respawns straight into it; no argument opens a
 * picker over the session dir, filtered to THIS workspace (another project's
 * conversations are noise). Newest first, labeled by the first user message.
 */
async function resumeSession(arg: string): Promise<void> {
  if (arg !== "") {
    ensureOutput().appendLine(`resuming ${arg}`);
    await restartSession(["--resume", arg]);
    return;
  }
  const cwd = workspaceRoot();
  const choices = await listSessions(sessionDir(), cwd);
  if (choices.length === 0) {
    void vscode.window.showInformationMessage(`wcode: no sessions recorded for ${cwd}`);
    return;
  }
  const picked = await vscode.window.showQuickPick(
    choices.map((choice) => ({
      label: choice.label,
      description: choice.when,
      detail: choice.path,
      path: choice.path,
    })),
    { title: "wcode: resume a session", placeHolder: "Newest first: only this workspace", matchOnDetail: true },
  );
  if (picked === undefined) return;
  ensureOutput().appendLine(`resuming ${picked.path}`);
  await restartSession(["--resume", picked.path]);
}

/** `/sessions`: the LIVE sessions in this connection; picking one retargets. */
async function pickConnectionSession(): Promise<void> {
  const picked = await vscode.window.showQuickPick(
    viewState.members.map((member) => ({
      label: targetLabel(viewState, member.id),
      description: `${member.state}${member.model === undefined ? "" : ` · ${member.model}`}`,
      detail: member.id,
      id: member.id,
    })),
    { title: "wcode: sessions in this connection" },
  );
  if (picked !== undefined) focusMember(picked.id);
}

/* ------------------------------------------------------------- member verbs */

/** Retarget the panel at a member (and hydrate that member's transcript). */
function focusMember(id: string): void {
  viewState = setTarget(viewState, id);
  const surface = controller;
  if (surface && surface.currentTarget !== id) {
    surface.setTarget(id); // fires `onTarget`, which hydrates
    return;
  }
  hydrate(id);
}

/** `peek` = `Request::Status`: a lean read, no model call, not the transcript. */
async function peekMember(id: string | undefined): Promise<void> {
  if (id === undefined || !session) return;
  try {
    const reply = await session.ask({ type: "status" }, id, 5_000);
    const work = reply.type === "status" ? reply.last_assistant_text ?? "(no work yet)" : `(${reply.type})`;
    const member = viewState.members.find((m) => m.id === id);
    void vscode.window.showInformationMessage(
      `wcode [${member?.state ?? "?"}] ${targetLabel(viewState, id)} · ${clip(work, 140)}`,
    );
  } catch (err) {
    void vscode.window.showWarningMessage(`wcode: peek failed: ${errMessage(err)}`);
  }
}

/** `SideAsk`: a side question — no turn, not recorded. The `/btw` command's engine. */
async function sideAsk(id: string, question: string): Promise<void> {
  if (!session) return;
  try {
    const reply = await session.ask({ type: "side_ask", text: question }, id, 60_000);
    const answer = reply.type === "side_answer" ? reply.text : `(${reply.type})`;
    void vscode.window.showInformationMessage(`wcode: ${clip(answer, 400)}`);
  } catch (err) {
    // `Ask` on a busy member can time out — say "busy", not "failed".
    void vscode.window.showWarningMessage(
      `wcode: ${targetLabel(viewState, id)} did not answer in time (it may be busy): ${errMessage(err)}`,
    );
  }
}

/** `wcode.member.ask`: prompt for the question, then `sideAsk`. */
async function askMember(id: string | undefined): Promise<void> {
  if (id === undefined || !session) return;
  const question = await vscode.window.showInputBox({
    title: `wcode: ask ${targetLabel(viewState, id)}`,
    prompt: "A side question: answered from its context, no turn, not recorded",
    placeHolder: "why did you choose that approach?",
  });
  if (question === undefined || question.trim() === "") return;
  await sideAsk(id, question);
}

/** `stop` = `Request::Cancel`; the row's state settles via the roster push. */
function stopMember(id: string | undefined): void {
  if (id === undefined) return;
  try {
    session?.send({ type: "cancel" }, id);
  } catch (err) {
    ensureOutput().appendLine(`stop failed: ${errMessage(err)}`);
  }
}

/** `SetPlanMode` → `Ack` (infallible): optimistic, settled on `Ack`, reverted on `Error`. */
function togglePlan(): void {
  if (!session) return;
  const on = !viewState.status.planMode;
  viewState = planModePending(viewState, on);
  controller?.update(viewState, true);
  const target = controller?.currentTarget ?? undefined;
  try {
    session.send({ type: "set_plan_mode", on }, target);
  } catch (err) {
    viewState = planModeRevert(viewState);
    controller?.update(viewState, true);
    ensureOutput().appendLine(`plan toggle failed: ${errMessage(err)}`);
  }
}

/* ------------------------------------------------------------------ roster */

/** The first roster arrival picks a target, so the surface is never unattached. */
function ensureTarget(): void {
  if (viewState.targeted !== null) return;
  const root = viewState.members.find((m) => m.isRoot) ?? viewState.members[0];
  if (root === undefined) return;
  focusMember(root.id);
}

/** Ask a member for its history — ONCE per session, addressed to ITS id. */
function hydrate(id: string): void {
  // The guard is `reducer.ts`'s pure `HydratedSet` (so it is tested); a second
  // retarget of the same session must not re-ask (that would duplicate it).
  if (!session || !hydrated.claim(id)) return;
  try {
    session.send({ type: "get_history" }, id);
  } catch (err) {
    hydrated.release(id);
    ensureOutput().appendLine(`hydrate ${id} failed: ${errMessage(err)}`);
  }
}

/* ------------------------------------------------------------------ panel io */

function surfaceHandlers(): SurfaceHandlers {
  return {
    onSubmit: (text: string, target: string | null) => {
      const channel = ensureOutput();
      if (!session) {
        channel.appendLine("submit ignored: no session is running");
        return;
      }
      const address = target ?? viewState.targeted ?? session.rootSessionId;
      if (address === null) {
        channel.appendLine("submit ignored: no target member yet");
        return;
      }
      try {
        // The session streams only the assistant's reply; echo the user locally
        // into THAT member's transcript.
        viewState = appendUser(viewState, text, address);
        controller?.update(viewState, true);
        session.send({ type: "submit", text }, address);
      } catch (err) {
        channel.appendLine(`submit failed: ${errMessage(err)}`);
      }
    },
    onCancel: (target: string | null) => {
      try {
        // Stop cancels the WHOLE team: every RUNNING agent, not only the target.
        const running = viewState.members.filter((member) => member.state === "running").map((member) => member.id);
        const ids = new Set(
          running.length > 0 ? running : [target ?? viewState.targeted ?? session?.rootSessionId ?? ""],
        );
        for (const id of ids) if (id !== "") session?.send({ type: "cancel" }, id);
      } catch (err) {
        ensureOutput().appendLine(`cancel failed: ${errMessage(err)}`);
      }
    },
    onSteer: (text: string, target: string | null) => {
      try {
        session?.send({ type: "interrupt", content: text }, target ?? undefined);
      } catch (err) {
        ensureOutput().appendLine(`steer failed: ${errMessage(err)}`);
      }
    },
    onOpenDiff: (callId: string) => {
      void openDiff(viewState, workspaceRoot(), callId, logLine);
    },
    onRevealFile: (path: string, line?: number) => {
      void revealFile(path, line);
    },
    onTarget: (target: string | null) => {
      if (target !== null) hydrate(target);
    },
    onTogglePlan: () => togglePlan(),
    onCommand: (name: string, arg: string) => runCommand(name, arg),
    onReady: () => ensureSession(),
    onFocusMember: (id: string) => {
      // Defence-in-depth: ignore an id not in the roster (a bogus id would
      // retarget to an empty transcript and waste a `get_history`).
      if (viewState.members.some((m) => m.id === id)) focusMember(id);
    },
    onReview: (callId: string, verdict: "accept" | "reject") => void reviewChange(callId, verdict),
  };
}

/**
 * Settle a change review. Accept is a NO-OP on disk (the edit already landed);
 * Reject reverse-applies the diff and WRITES the before-image. The verdict is set
 * only when a write SUCCEEDED — the panel never claims a write that did not happen.
 */
async function reviewChange(callId: string, verdict: "accept" | "reject"): Promise<void> {
  const surface = controller;
  if (verdict === "accept") {
    reviewVerdicts = setVerdict(reviewVerdicts, callId, "accepted");
    surface?.setVerdicts(reviewVerdicts);
    return;
  }
  // Reject = reverse-apply the tool's diff to the CURRENT file and write the before-image.
  const ok = await revertDiff(viewState, workspaceRoot(), callId, logLine);
  if (ok) {
    reviewVerdicts = setVerdict(reviewVerdicts, callId, "rejected");
    surface?.setVerdicts(reviewVerdicts);
  }
  // !ok: `revertDiff` already warned + logged (the file moved on -> reverseApply is
  // null); the verdict stays "pending", so the surface never claims a write that did not.
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

/* ------------------------------------------------------------------ helpers */

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

function workspaceRoot(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

/** The active editor's selection as a `SelectionContext`, or null when empty. */
function selectionOf(editor: vscode.TextEditor): SelectionContext | null {
  if (editor.selection.isEmpty) return null;
  const { start, end } = editor.selection;
  const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
  const path = folder
    ? vscode.workspace.asRelativePath(editor.document.uri, false)
    : editor.document.uri.fsPath;
  // VS Code's `end` is EXCLUSIVE: a selection ending at column 0 does NOT cover
  // that line, so the last covered line is `end.line` (not `end.line + 1`).
  const lastLine = end.character === 0 && end.line > start.line ? end.line : end.line + 1;
  return { path, startLine: start.line + 1, endLine: lastLine }; // 1-based inclusive
}

function requireExtensionUri(): vscode.Uri {
  if (!extensionUri) throw new Error("wcode activated without an extension URI");
  return extensionUri;
}

function ensureOutput(): vscode.OutputChannel {
  output ??= vscode.window.createOutputChannel("wcode");
  return output;
}

/** The diff feature's diagnostic sink (the same OutputChannel). */
function logLine(message: string): void {
  ensureOutput().appendLine(message);
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
