/**
 * The extension host entry point.
 *
 * `wcode.start` opens (or reveals) the webview chat panel and starts the
 * `wcode serve --stdio` child; `wcode.stop` stops it; `wcode.restart` restarts.
 * The OutputChannel stays for diagnostics (every frame, every stderr line).
 *
 * P3: the sidebar is a CONTROL SURFACE. The roster is pushed (never polled) and
 * a member's row retargets the panel; every send is addressed to the member's
 * session id, which the server demuxes against its live roster. The verbs are
 * the `member` tool's — peek = `Status`, ask = `SideAsk` (no turn), stop =
 * `Cancel` — plus the composer's `Submit`/`Interrupt` and `SetPlanMode`.
 */
import * as fs from "node:fs";
import * as vscode from "vscode";

import { clearDiffs, openDiff, registerDiffProvider, revertDiff } from "./diffProvider.ts";
import { setVerdict, type Verdict } from "./review.ts";
import { ChatPanel, type PanelHandlers } from "./panel.ts";
import type { SelectionContext } from "./webview.ts";
import {
  appendUser,
  HydratedSet,
  initialState,
  planModePending,
  planModeRevert,
  reduce,
  sidebarTree,
  target as setTarget,
  targetLabel,
  type MemberNode,
  type SidebarNode,
  type ViewState,
} from "./reducer.ts";
import { SidebarProvider } from "./roster.ts";
import { WcodeSession, type CrashInfo, type SessionState } from "./session.ts";
import { isUnsupportedStdio, unsupportedStdioMessage } from "./startup.ts";
import type { AgentEvent, RawFrame, TodoItem } from "./protocol.ts";

let session: WcodeSession | undefined;
let viewState: ViewState = initialState();
let output: vscode.OutputChannel | undefined;
let status: vscode.StatusBarItem | undefined;
let extensionUri: vscode.Uri | undefined;
let sidebar: SidebarProvider | undefined;
let treeView: vscode.TreeView<SidebarNode> | undefined;
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
  status.tooltip = "wcode session — click to restart";
  context.subscriptions.push(output, status);
  registerDiffProvider(context, logLine);

  sidebar = new SidebarProvider();
  treeView = vscode.window.createTreeView("wcode.members", { treeDataProvider: sidebar });
  context.subscriptions.push(sidebar, treeView);
  // Keyboard navigation retargets too; `focusMember` is idempotent, so the
  // `TreeItem.command` (a click) and this do not double-work.
  context.subscriptions.push(
    treeView.onDidChangeSelection((event) => {
      const id = memberIdOf(event.selection[0]);
      if (id !== undefined) focusMember(id);
    }),
  );

  // The selection is STICKY: `undefined` (focus moved to the webview/terminal)
  // KEEPS the last selection — clearing there is the bug that makes the chip
  // vanish as you go to type.
  context.subscriptions.push(
    vscode.window.onDidChangeTextEditorSelection((event) => {
      lastSelection = selectionOf(event.textEditor);
      ChatPanel.currentPanel()?.setContext(lastSelection);
    }),
    vscode.window.onDidChangeActiveTextEditor((editor) => {
      if (editor === undefined) return;
      lastSelection = selectionOf(editor);
      ChatPanel.currentPanel()?.setContext(lastSelection);
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("wcode.start", () => void startSession()),
    vscode.commands.registerCommand("wcode.stop", () => void stopSession()),
    vscode.commands.registerCommand("wcode.restart", () => void restartSession()),
    vscode.commands.registerCommand("wcode.openDiff", () =>
      void openDiff(viewState, workspaceRoot(), undefined, logLine),
    ),
    vscode.commands.registerCommand("wcode.member.focus", (id?: unknown) => {
      if (typeof id === "string") focusMember(id);
    }),
    vscode.commands.registerCommand("wcode.member.peek", (node?: MemberNode) => void peekMember(node?.row.id)),
    vscode.commands.registerCommand("wcode.member.ask", (node?: MemberNode) => void askMember(node?.row.id)),
    vscode.commands.registerCommand("wcode.member.stop", (node?: MemberNode) => stopMember(node?.row.id)),
    vscode.commands.registerCommand("wcode.plan.toggle", () => togglePlan()),
  );
}

export function deactivate(): Thenable<void> | undefined {
  ChatPanel.disposeCurrent();
  clearDiffs();
  return session?.stop();
}

/* ----------------------------------------------------------------- commands */

async function startSession(): Promise<void> {
  const channel = ensureOutput();
  const panel = ChatPanel.createOrShow(requireExtensionUri(), panelHandlers(), viewState);
  // Seed the chip ONCE when the panel opens; the listeners keep it in sync after.
  const editor = vscode.window.activeTextEditor;
  if (editor !== undefined) lastSelection = selectionOf(editor);
  panel.setContext(lastSelection);

  if (session && (session.state === "ready" || session.state === "starting")) {
    channel.appendLine("session already running");
    panel.reveal();
    return;
  }

  let binary: string;
  try {
    binary = resolveBinary();
  } catch (err) {
    const message = errMessage(err);
    channel.appendLine(`error: ${message}`);
    panel.setSession({ state: "crashed", stderrTail: message });
    void vscode.window.showErrorMessage(`wcode: ${message}`);
    return;
  }

  channel.appendLine(`starting: ${binary} serve --stdio`);
  const next = new WcodeSession({
    binary,
    cwd: workspaceRoot(),
    logger: { info: (m) => channel.appendLine(m), error: (m) => channel.appendLine(`error: ${m}`) },
  });
  session = next;
  hydrated.reset();
  panel.setSession({ id: null, state: next.state, stderrTail: "" });

  next.on("event", (event: AgentEvent, frame: RawFrame) => {
    const correlation = typeof frame.reply_to === "number" ? ` (reply_to ${frame.reply_to})` : "";
    channel.appendLine(`← ${frame.session} ${frame.type}${correlation}`);
    // The frame's `session` is the origin (a fanned event) or the target (a
    // reply) — the key every per-session arm writes.
    viewState = reduce(viewState, event, frame.session);
    ensureTarget();
    pushSidebar();
    if (event.type === "spawned") revealMember(event.worker);
    // `message_end` (and the other settled events) always send the final state.
    panel.update(viewState, isSettled(event));
  });
  next.on("stderr", (line: string) => channel.appendLine(`stderr: ${line}`));
  next.on("state", (state: SessionState) => {
    if (status) {
      status.text = `wcode: ${state}`;
      status.show();
    }
    panel.setSession({ state });
  });
  next.on("crash", (info: CrashInfo) => {
    channel.appendLine("session crashed");
    if (info.stderrTail !== "") channel.appendLine(info.stderrTail);
    // The first-run case: an older `wcode` on PATH rejects `--stdio`. Say what
    // is wrong, WHICH binary, and both ways out — in the existing crash surface.
    if (isUnsupportedStdio(info.code, info.stderrTail)) {
      const message = unsupportedStdioMessage(binary);
      channel.appendLine(message);
      panel.setSession({ state: "crashed", stderrTail: message });
      return;
    }
    panel.setSession({ state: "crashed", stderrTail: info.stderrTail });
  });

  try {
    await next.start();
  } catch (err) {
    const message = errMessage(err);
    channel.appendLine(`failed to start: ${message}`);
    panel.setSession({ state: "crashed", stderrTail: message });
    void vscode.window.showErrorMessage(
      `wcode failed to start (${message}). Set "wcode.path" or put wcode on PATH.`,
    );
    return;
  }

  channel.appendLine(`ready — root session ${next.rootSessionId}`);
  panel.setSession({ id: next.rootSessionId, state: next.state });
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
  ChatPanel.currentPanel()?.setSession({ state: "stopped" });
}

async function restartSession(): Promise<void> {
  await stopSession();
  // A restart spawns a *fresh* child with empty history, so the state must start
  // empty too — otherwise the panel shows a conversation the new session knows
  // nothing about.
  viewState = initialState();
  reviewVerdicts = {};
  ChatPanel.currentPanel()?.update(viewState, true);
  await startSession();
}

/* ------------------------------------------------------------- member verbs */

/** Retarget the panel at a member (and hydrate that member's transcript). */
function focusMember(id: string): void {
  viewState = setTarget(viewState, id);
  pushSidebar();
  const panel = ChatPanel.currentPanel();
  if (panel && panel.currentTarget !== id) {
    panel.setTarget(id); // fires `onTarget`, which hydrates
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

/** `ask` = `Request::SideAsk`: a side question — no turn, not recorded. */
async function askMember(id: string | undefined): Promise<void> {
  if (id === undefined || !session) return;
  const question = await vscode.window.showInputBox({
    title: `wcode: ask ${targetLabel(viewState, id)}`,
    prompt: "A side question — answered from its context, no turn, not recorded",
    placeHolder: "why did you choose that approach?",
  });
  if (question === undefined || question.trim() === "") return;
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
  ChatPanel.currentPanel()?.update(viewState, true);
  const target = ChatPanel.currentPanel()?.currentTarget ?? undefined;
  try {
    session.send({ type: "set_plan_mode", on }, target);
  } catch (err) {
    viewState = planModeRevert(viewState);
    ChatPanel.currentPanel()?.update(viewState, true);
    ensureOutput().appendLine(`plan toggle failed: ${errMessage(err)}`);
  }
}

/* ------------------------------------------------------------------ roster */

/** Push the sidebar sections to the tree (it dedupes; no polling, no refresh). */
function pushSidebar(): void {
  sidebar?.set(sidebarTree(viewState.members, tasksTodos()));
}

/**
 * The Tasks section shows the ROOT session's plan — the sidebar is durable and does
 * NOT retarget, so it is the OVERALL plan, not the panel's `targeted` member.
 */
function tasksTodos(): TodoItem[] {
  const root = viewState.members.find((m) => m.isRoot) ?? viewState.members[0];
  return root === undefined ? [] : viewState.todos[root.id] ?? [];
}

/** The member id a tree node carries (a section / todo node carries none). */
function memberIdOf(node: SidebarNode | undefined): string | undefined {
  return node !== undefined && node.kind === "member" ? node.row.id : undefined;
}

/** The first roster arrival picks a target, so the panel is never unattached. */
function ensureTarget(): void {
  if (viewState.targeted !== null) return;
  const root = viewState.members.find((m) => m.isRoot) ?? viewState.members[0];
  if (root === undefined) return;
  focusMember(root.id);
}

/** Reveal a newly spawned member — without stealing focus or selection. */
function revealMember(id: string): void {
  const item = sidebar?.find(id);
  if (item === undefined || treeView === undefined) return;
  void treeView.reveal(item, { select: false, focus: false });
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

function panelHandlers(): PanelHandlers {
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
        ChatPanel.currentPanel()?.update(viewState, true);
        session.send({ type: "submit", text }, address);
      } catch (err) {
        channel.appendLine(`submit failed: ${errMessage(err)}`);
      }
    },
    onCancel: (target: string | null) => {
      try {
        session?.send({ type: "cancel" }, target ?? undefined);
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
  const panel = ChatPanel.currentPanel();
  if (verdict === "accept") {
    reviewVerdicts = setVerdict(reviewVerdicts, callId, "accepted");
    panel?.setVerdicts(reviewVerdicts);
    return;
  }
  // Reject = reverse-apply the tool's diff to the CURRENT file and write the before-image.
  const ok = await revertDiff(viewState, workspaceRoot(), callId, logLine);
  if (ok) {
    reviewVerdicts = setVerdict(reviewVerdicts, callId, "rejected");
    panel?.setVerdicts(reviewVerdicts);
  }
  // !ok: `revertDiff` already warned + logged (the file moved on -> reverseApply is
  // null); the verdict stays "pending", so the panel never claims a write that did not.
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
