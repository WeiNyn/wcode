/**
 * The extension host entry point.
 *
 * `wcode.start` opens (or reveals) the webview chat panel and starts the
 * `wcode serve --stdio` child; `wcode.stop` stops it; `wcode.restart` restarts.
 * The OutputChannel stays for diagnostics (every frame, every stderr line), and
 * the panel carries the user-facing status.
 *
 * P1b: the panel is the click-path. The rendering is done here (the host) —
 * `render.ts` turns a `ViewState` into HTML — and posted to a dependency-free
 * webview.
 */
import * as fs from "node:fs";
import * as vscode from "vscode";

import { clearDiffs, openDiff, registerDiffProvider } from "./diffProvider.ts";
import { ChatPanel, type PanelHandlers } from "./panel.ts";
import { appendUser, initialState, reduce, type ViewState } from "./reducer.ts";
import { WcodeSession, type SessionState } from "./session.ts";
import type { AgentEvent, RawFrame } from "./protocol.ts";

let session: WcodeSession | undefined;
let viewState: ViewState = initialState();
let output: vscode.OutputChannel | undefined;
let status: vscode.StatusBarItem | undefined;
let extensionUri: vscode.Uri | undefined;

export function activate(context: vscode.ExtensionContext): void {
  extensionUri = context.extensionUri;
  output = vscode.window.createOutputChannel("wcode");
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  status.command = "wcode.restart";
  status.tooltip = "wcode session — click to restart";
  context.subscriptions.push(output, status);
  registerDiffProvider(context, logLine);

  context.subscriptions.push(
    vscode.commands.registerCommand("wcode.start", () => void startSession()),
    vscode.commands.registerCommand("wcode.stop", () => void stopSession()),
    vscode.commands.registerCommand("wcode.restart", () => void restartSession()),
    vscode.commands.registerCommand("wcode.openDiff", () =>
      void openDiff(viewState, workspaceRoot(), undefined, logLine),
    ),
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
  panel.setSession({ id: null, state: next.state, stderrTail: "" });

  next.on("event", (event: AgentEvent, frame: RawFrame) => {
    const correlation = typeof frame.reply_to === "number" ? ` (reply_to ${frame.reply_to})` : "";
    channel.appendLine(`← ${frame.type}${correlation}`);
    viewState = reduce(viewState, event);
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
  next.on("crash", (info: { stderrTail: string }) => {
    channel.appendLine("session crashed");
    if (info.stderrTail !== "") channel.appendLine(info.stderrTail);
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
  // Seed the transcript through the same channel the reducer folds.
  next.send({ type: "get_history" });
}

async function stopSession(): Promise<void> {
  const current = session;
  session = undefined;
  if (current) {
    await current.stop();
    ensureOutput().appendLine("stopped");
  }
  // A restart spawns a fresh child; the before-images belong to the old one.
  clearDiffs();
  ChatPanel.currentPanel()?.setSession({ state: "stopped" });
}

async function restartSession(): Promise<void> {
  await stopSession();
  // A restart spawns a *fresh* child with empty history, so the transcript must
  // start empty too — otherwise the panel shows a conversation the new session
  // knows nothing about.
  viewState = initialState();
  ChatPanel.currentPanel()?.update(viewState, true);
  await startSession();
}

/* ------------------------------------------------------------------ panel io */

function panelHandlers(): PanelHandlers {
  return {
    onSubmit: (text: string) => {
      const channel = ensureOutput();
      if (!session) {
        channel.appendLine("submit ignored: no session is running");
        return;
      }
      try {
        // The session streams only the assistant's reply; echo the user locally.
        viewState = appendUser(viewState, text);
        ChatPanel.currentPanel()?.update(viewState, true);
        session.send({ type: "submit", text });
      } catch (err) {
        channel.appendLine(`submit failed: ${errMessage(err)}`);
      }
    },
    onCancel: () => {
      try {
        session?.send({ type: "cancel" });
      } catch (err) {
        ensureOutput().appendLine(`cancel failed: ${errMessage(err)}`);
      }
    },
    onSteer: (text: string) => {
      try {
        session?.send({ type: "interrupt", content: text });
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
  };
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

function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
