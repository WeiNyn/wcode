/**
 * The editor-tab host of a session's surface — V2, de-singletonized for multi-tab (D013).
 *
 * The CONTROLLER (`surface.ts`) owns a session's state, throttle and message contract;
 * this is only the `WebviewPanel` half: create the tab, wrap it in a `WebviewPanelHost`,
 * attach it. The docked home is a `WebviewView` (`webviewView.ts`).
 *
 * A `ChatPanel` used to be a SINGLETON (a `static` field holding the one panel):
 * `wcode: Open in Editor` revealed rather than opened, and one workspace had one
 * conversation. D013 makes a tab PER session — `ChatPanel.open` always returns a
 * fresh panel, and the `Manager` maps panels to sessions. There is no static state
 * here any more.
 *
 * Column policy (D013, user rule): every panel opens as a tab in the ACTIVE editor
 * group and reveal never moves it — `/new`(`/clear`) `/team` `/resume` stack as
 * tabs, never beside. `ViewColumn.Beside` was the V2 dockable-surface default and
 * split the editor on every open. Fix: viewColumn: vscode.ViewColumn.Active
 * at panel.ts:41, and reveal() / WebviewPanelHost.reveal() drop the column so a
 * reveal can never re-split a group.
 *
 * Security posture: `enableScripts` only, a strict CSP with no remote origins (the
 * shared `surfaceHtml`), `localResourceRoots` limited to `media/`, and
 * `retainContextWhenHidden` so the transcript survives a tab switch.
 */
import * as vscode from "vscode";

import type { SurfaceController, SurfaceHost } from "./surface.ts";
import type { ToWebview } from "./webview.ts";

/** One editor tab hosting a session's surface. NOT a singleton (D013). */
export class ChatPanel {
  private constructor(
    private readonly panel: vscode.WebviewPanel,
    controller: SurfaceController,
    mediaRoot: vscode.Uri,
  ) {
    controller.attach(new WebviewPanelHost(panel, mediaRoot));
  }

  /** Open a NEW editor tab for `controller`, titled `title`. Never reveals an existing one. */
  static open(extensionUri: vscode.Uri, controller: SurfaceController, title: string): ChatPanel {
    const mediaRoot = vscode.Uri.joinPath(extensionUri, "media");
    // ACTIVE (never Beside): a `wcode.surface` tab belongs to the active editor
    // group. Beside split the workspace open on every `/new` `/team` `/resume`.
    const panel = vscode.window.createWebviewPanel(
      "wcode.surface",
      title,
      { viewColumn: vscode.ViewColumn.Active, preserveFocus: true },
      { enableScripts: true, localResourceRoots: [mediaRoot], retainContextWhenHidden: true },
    );
    return new ChatPanel(panel, controller, mediaRoot);
  }

  /** Bring this tab to the front (the `Manager` calls this when it goes active). */
  reveal(): void {
    // No column: reveal the tab where it already is — passing one MOVES the panel
    // (Beside re-split the group on every reveal).
    this.panel.reveal(undefined, true);
  }

  /** Set the tab title (sticky labels, D013 C). A no-op when unchanged, so no flicker. */
  setTitle(title: string): void {
    if (this.panel.title !== title) this.panel.title = title;
  }

  /** The user closed the tab (its ×), or the panel was disposed programmatically. */
  onDidDispose(listener: () => void): vscode.Disposable {
    return this.panel.onDidDispose(listener);
  }

  /** A view-state change: `active` is true when this tab gained focus (→ active session). */
  onDidChangeActive(listener: (active: boolean) => void): vscode.Disposable {
    return this.panel.onDidChangeViewState(() => listener(this.panel.active));
  }

  /** Is this tab the active editor right now (the user is looking at it)? */
  get isActive(): boolean {
    return this.panel.active;
  }

  dispose(): void {
    this.panel.dispose();
  }
}

/** A `SurfaceHost` over a `vscode.WebviewPanel` (the editor-tab home). */
class WebviewPanelHost implements SurfaceHost {
  constructor(
    private readonly panel: vscode.WebviewPanel,
    mediaRoot: vscode.Uri,
  ) {
    this.panel.webview.options = { enableScripts: true, localResourceRoots: [mediaRoot] };
  }

  get webview(): vscode.Webview {
    return this.panel.webview;
  }

  get visible(): boolean {
    return this.panel.visible;
  }

  reveal(): void {
    // No column (see ChatPanel.reveal): a reveal must never re-split the group.
    this.panel.reveal(undefined, true);
  }

  post(message: ToWebview): void {
    void this.panel.webview.postMessage(message);
  }

  onMessage(listener: (raw: unknown) => void): vscode.Disposable {
    return this.panel.webview.onDidReceiveMessage(listener);
  }

  onDispose(listener: () => void): vscode.Disposable {
    return this.panel.onDidDispose(listener);
  }
}
