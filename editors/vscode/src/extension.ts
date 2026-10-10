/**
 * The extension host entry point — the COMPOSITION ROOT (D013 / W010 P2).
 *
 * Everything that used to be a module-level singleton here (`session`, `viewState`,
 * `controller`, `sessionArgs`, `userStopped`, `hydrated`, `reviewVerdicts`) now lives
 * on a `SessionTab` (`tab.ts`), and the shared resources + tab registry live on the
 * `Manager` (`manager.ts`). This file only wires them to VS Code:
 *
 *   - ONE `WebviewView` provider (`wcode.surface`, the Activity Bar / panel dock),
 *     re-bound to the ACTIVE session by the `Manager`;
 *   - the `wcode.*` commands, each routed to the active tab;
 *   - the diff provider (registered ONCE, shared across tabs — D013 Q5).
 *
 * A workspace can hold several editor tabs, each an INDEPENDENT wcode session — its
 * own `serve --stdio` child, conversation and team (Option A, D013). No cross-tab
 * visibility: a worker spawned in one tab never reaches another.
 */
import * as vscode from "vscode";

import { clearDiffs, registerDiffProvider } from "./diffProvider.ts";
import { Manager } from "./manager.ts";
import { SurfaceViewProvider } from "./webviewView.ts";

let manager: Manager | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const mgr = new Manager(context.extensionUri);
  manager = mgr;
  registerDiffProvider(context, (message) => mgr.output.appendLine(message));

  context.subscriptions.push(
    // ONE view provider, re-bound to the active session (D013 A).
    vscode.window.registerWebviewViewProvider("wcode.surface", new SurfaceViewProvider(mgr.mediaRoot, mgr)),
    mgr,
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("wcode.start", () => mgr.startSession()),
    vscode.commands.registerCommand("wcode.stop", () => mgr.stopSession()),
    vscode.commands.registerCommand("wcode.restart", () => mgr.reloadSession()),
    // A fresh session in a NEW tab (D013 B): distinct from `openInEditor`.
    vscode.commands.registerCommand("wcode.newSession", () => mgr.newSession()),
    vscode.commands.registerCommand("wcode.openInEditor", () => mgr.openInEditor()),
    vscode.commands.registerCommand("wcode.openDiff", () => mgr.openDiff()),
    vscode.commands.registerCommand("wcode.focusPanel", () => mgr.focusPanel()),
    vscode.commands.registerCommand("wcode.showLog", () => mgr.showLog()),
    vscode.commands.registerCommand("wcode.member.focus", (id?: unknown) => {
      if (typeof id === "string") mgr.focusMember(id);
    }),
    // INERT until a menu supplies an id (a follow-up): peek/ask took a tree element
    // before, so a palette call passes no member — it targets the ACTIVE session's
    // focused member instead. `stop` is NOT inert (it stops the focused member).
    vscode.commands.registerCommand("wcode.member.peek", () => mgr.peekMember()),
    vscode.commands.registerCommand("wcode.member.ask", () => mgr.askMember()),
    vscode.commands.registerCommand("wcode.member.stop", () => mgr.stopMember()),
    vscode.commands.registerCommand("wcode.plan.toggle", () => mgr.togglePlan()),
  );
}

export function deactivate(): void {
  // `dispose` stops EVERY tab's child (the orphan backstop) and disposes the view.
  manager?.dispose();
  clearDiffs();
}
