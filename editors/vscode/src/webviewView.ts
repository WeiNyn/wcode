import * as vscode from "vscode";

import type { SurfaceController, SurfaceHost } from "./surface.ts";
import type { ToWebview } from "./webview.ts";

/**
 * The sidebar/panel dock — V2. A `WebviewViewProvider` whose `resolveWebviewView` wraps
 * the `WebviewView` in a `SurfaceHost` and attaches it to the ONE `SurfaceController`.
 * The view is user-movable (primary/secondary sidebar ↔ the panel) for free; VS Code
 * manages where it sits.
 *
 * DIFFERENCES from a `WebviewPanel` host (flagged):
 *   - a `WebviewView` has NO `retainContextWhenHidden` — it keeps DOM state via
 *     `getState`/`setState` (the webview already survives re-renders from a snapshot,
 *     so this is acceptable);
 *   - it has NO `reveal` — revealing needs a command
 *     (`workbench.view.extension.wcode` / the auto-generated `wcode.surface.focus`);
 *   - it fires `onDidChangeVisibility` (a panel does not).
 */
export class SurfaceViewProvider implements vscode.WebviewViewProvider {
  constructor(private readonly controller: SurfaceController) {}

  resolveWebviewView(view: vscode.WebviewView): void {
    // REQUIRED: a WebviewView has NO options ctor (unlike createWebviewPanel). WITHOUT
    // enableScripts + localResourceRoots the bundled script never runs — a BLANK surface.
    view.webview.options = {
      enableScripts: true,
      localResourceRoots: [this.controller.mediaRoot],
    };
    this.controller.attach(new WebviewViewHost(view));
  }
}

/** A `SurfaceHost` over a `vscode.WebviewView`. */
export class WebviewViewHost implements SurfaceHost {
  constructor(private readonly view: vscode.WebviewView) {}

  get webview(): vscode.Webview {
    return this.view.webview;
  }

  get visible(): boolean {
    return this.view.visible;
  }

  reveal(): void {
    void vscode.commands.executeCommand("wcode.surface.focus");
  }

  post(message: ToWebview): void {
    void this.view.webview.postMessage(message);
  }

  onMessage(listener: (raw: unknown) => void): vscode.Disposable {
    return this.view.webview.onDidReceiveMessage(listener);
  }

  onDispose(listener: () => void): vscode.Disposable {
    return this.view.onDidDispose(listener);
  }

  onDidChangeVisibility(listener: (visible: boolean) => void): vscode.Disposable {
    // `WebviewView.onDidChangeVisibility` is an `Event<void>`; read the flag off the view.
    return this.view.onDidChangeVisibility(() => listener(this.view.visible));
  }
}
