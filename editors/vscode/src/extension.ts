/**
 * The extension host entry point (P1 — the spine).
 *
 * Minimal and runnable: it starts/stops/restarts a `WcodeSession` and logs
 * every frame to an OutputChannel, so an F5 run shows something real before the
 * webview panel (P1b) exists. No webview here.
 */
import * as fs from "node:fs";
import * as vscode from "vscode";

import { initialState, reduce, type ViewState } from "./reducer.ts";
import { WcodeSession } from "./session.ts";
import type { AgentEvent, RawFrame } from "./protocol.ts";

let session: WcodeSession | undefined;
let viewState: ViewState = initialState();
let output: vscode.OutputChannel | undefined;
let status: vscode.StatusBarItem | undefined;

export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel("wcode");
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  status.command = "wcode.restart";
  status.tooltip = "wcode session — click to restart";
  context.subscriptions.push(output, status);

  context.subscriptions.push(
    vscode.commands.registerCommand("wcode.start", () => void startSession()),
    vscode.commands.registerCommand("wcode.stop", () => void stopSession()),
    vscode.commands.registerCommand("wcode.restart", () => void restartSession()),
    vscode.commands.registerCommand("wcode.openDiff", () =>
      void vscode.window.showInformationMessage("wcode: diffs land in P2."),
    ),
  );
}

export function deactivate(): Thenable<void> | undefined {
  return session?.stop();
}

/* ----------------------------------------------------------------- commands */

async function startSession(): Promise<void> {
  const channel = ensureOutput();
  if (session && (session.state === "ready" || session.state === "starting")) {
    channel.appendLine("session already running");
    channel.show(true);
    return;
  }

  let binary: string;
  try {
    binary = resolveBinary();
  } catch (err) {
    const message = errMessage(err);
    channel.appendLine(`error: ${message}`);
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

  next.on("event", (event: AgentEvent, frame: RawFrame) => {
    const correlation = typeof frame.reply_to === "number" ? ` (reply_to ${frame.reply_to})` : "";
    channel.appendLine(`← ${frame.type}${correlation}`);
    viewState = reduce(viewState, event);
  });
  next.on("stderr", (line: string) => channel.appendLine(`stderr: ${line}`));
  next.on("state", (state: string) => {
    if (status) {
      status.text = `wcode: ${state}`;
      status.show();
    }
  });
  next.on("crash", (info: { stderrTail: string }) => {
    channel.appendLine("session crashed");
    if (info.stderrTail !== "") channel.appendLine(info.stderrTail);
  });

  try {
    await next.start();
  } catch (err) {
    const message = errMessage(err);
    channel.appendLine(`failed to start: ${message}`);
    void vscode.window.showErrorMessage(
      `wcode failed to start (${message}). Set "wcode.path" or put wcode on PATH.`,
    );
    return;
  }

  channel.appendLine(`ready — root session ${next.rootSessionId}`);
  // Seed the transcript through the same channel the reducer folds.
  next.send({ type: "get_history" });
  channel.show(true);
}

async function stopSession(): Promise<void> {
  const current = session;
  session = undefined;
  if (current) {
    await current.stop();
    ensureOutput().appendLine("stopped");
  }
}

async function restartSession(): Promise<void> {
  await stopSession();
  await startSession();
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

function ensureOutput(): vscode.OutputChannel {
  output ??= vscode.window.createOutputChannel("wcode");
  return output;
}

function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
