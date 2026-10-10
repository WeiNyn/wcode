import * as vscode from "vscode";

import type { SurfaceHost } from "./surface.ts";
import type { ToWebview } from "./webview.ts";

/**
 * What the `Manager` implements so the ONE docked view can be re-bound to the
 * ACTIVE session (D013 A): VS Code exposes exactly one `WebviewView` per view id,
 * so the sidebar follows the focused tab rather than being N-up.
 */
export interface ViewBinder {
  /** The view resolved: bind it to whatever session is active (the `Manager` decides). */
  bindView(host: SurfaceHost): void;
}

/**
 * The sidebar/panel dock — V2, multi-tab. A `WebviewViewProvider` whose
 * `resolveWebviewView` wraps the `WebviewView` in a `SurfaceHost` and hands it to the
 * `Manager`, which attaches it to the ACTIVE session's controller (and re-attaches on
 * tab changes). The view is user-movable (primary/secondary sidebar ↔ the panel) for
 * free; VS Code manages where it sits.
 *
 * DIFFERENCES from a `WebviewPanel` host (flagged):
 *   - a `WebviewView` has NO `retainContextWhenHidden` — it keeps DOM state via
 *     `getState`/`setState` (the webview already survives re-renders from a snapshot,
 *     so this is acceptable);
 *   - it has NO `reveal` — revealing needs a command
 *     (`workbench.view.extension.wcode` / the auto-generated `wcode.surface.focus`);
 *   - it fires `onDidChangeVisibility` (a panel does not);
 *   - exactly ONE instance exists, re-bound to the active session.
 */
export class SurfaceViewProvider implements vscode.WebviewViewProvider {
  constructor(
    private readonly mediaRoot: vscode.Uri,
    private readonly binder: ViewBinder,
  ) {}

  resolveWebviewView(view: vscode.WebviewView): void {
    // REQUIRED: a WebviewView has NO options ctor (unlike createWebviewPanel). WITHOUT
    // enableScripts + localResourceRoots the bundled script never runs — a BLANK surface.
    view.webview.options = {
      enableScripts: true,
      localResourceRoots: [this.mediaRoot],
    };
    this.binder.bindView(new WebviewViewHost(view));
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
