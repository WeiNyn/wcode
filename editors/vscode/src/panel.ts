/**
 * The editor-tab host of the shared surface — V2.
 *
 * The CONTROLLER (`surface.ts`) owns the state, the throttle and the message contract;
 * this is only the `WebviewPanel` half: create the tab, wrap it in a `WebviewPanelHost`,
 * attach it. The docked home is a `WebviewView` (`webviewView.ts`). The editor tab is a
 * singleton host.
 *
 * Security posture: `enableScripts` only, a strict CSP with no remote origins (the
 * shared `surfaceHtml`), `localResourceRoots` limited to `media/`, and
 * `retainContextWhenHidden` so the transcript survives a tab switch.
 */
import * as vscode from "vscode";

import type { SurfaceController, SurfaceHost } from "./surface.ts";
import type { ToWebview } from "./webview.ts";

export class ChatPanel {
  private static current: ChatPanel | undefined;

  private readonly panel: vscode.WebviewPanel;

  private constructor(panel: vscode.WebviewPanel) {
    this.panel = panel;
  }

  /** Open (or reveal) the surface as an editor tab, attached to the shared controller. */
  static createOrShow(extensionUri: vscode.Uri, controller: SurfaceController): ChatPanel {
    if (ChatPanel.current) {
      ChatPanel.current.reveal();
      return ChatPanel.current;
    }
    const mediaRoot = vscode.Uri.joinPath(extensionUri, "media");
    const panel = vscode.window.createWebviewPanel(
      "wcode.surface",
      "wcode",
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      { enableScripts: true, localResourceRoots: [mediaRoot], retainContextWhenHidden: true },
    );
    const host = new WebviewPanelHost(panel, mediaRoot);
    controller.attach(host);
    const created = new ChatPanel(panel);
    // The host detach is wired by `attach`; this only clears the singleton.
    panel.onDidDispose(() => {
      if (ChatPanel.current === created) ChatPanel.current = undefined;
    });
    ChatPanel.current = created;
    return created;
  }

  reveal(): void {
    this.panel.reveal(vscode.ViewColumn.Beside, true);
  }

  dispose(): void {
    if (ChatPanel.current === this) ChatPanel.current = undefined;
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
    this.panel.reveal(vscode.ViewColumn.Beside, true);
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
