/**
 * The webview chat panel — the only place the message contract meets `vscode`.
 *
 * Security posture (sketch §4): `enableScripts` only, a strict CSP with **no
 * remote origins** (`default-src 'none'`; only the bundled `media/*` via
 * `webview.cspSource`), `localResourceRoots` limited to `media/`, and
 * `retainContextWhenHidden` so the transcript survives a tab switch.
 *
 * The webview is a pure renderer: the host renders markdown (`render.ts`) and
 * posts `RenderedState` snapshots, coalesced by a throttle so a fast stream
 * cannot flood it.
 */
import * as vscode from "vscode";

import { renderState } from "./render.ts";
import type { ViewState } from "./reducer.ts";
import {
  createThrottle,
  parseFromWebview,
  realScheduler,
  type PanelSessionInfo,
  type SelectionContext,
  type Throttle,
  type ToWebview,
} from "./webview.ts";

/** ~30 ms: coalesce a fast token stream into at most ~33 posts/s. */
export const THROTTLE_MS = 30;

/** What the panel calls back into the host for. */
export interface PanelHandlers {
  /** `target` is the member the composer is addressed to (null = the root). */
  onSubmit(text: string, target: string | null): void;
  onCancel(target: string | null): void;
  onSteer(text: string, target: string | null): void;
  onOpenDiff(callId: string): void;
  onRevealFile(path: string, line?: number): void;
  /** The user picked a member: the host hydrates that session's transcript. */
  onTarget(target: string | null): void;
  /** The composer's `Mode:` control — flip plan-mode (extension.ts `togglePlan`). */
  onTogglePlan(): void;
}

export class ChatPanel {
  private static current: ChatPanel | undefined;

  private readonly panel: vscode.WebviewPanel;
  private readonly throttle: Throttle;
  private readonly handlers: PanelHandlers;
  private readonly disposables: vscode.Disposable[] = [];

  private state: ViewState;
  private session: PanelSessionInfo = { id: null, state: "stopped", stderrTail: "" };
  /**
   * The selection the composer may attach — STICKY: the host keeps the last
   * non-empty selection; clears only on `×` (webview) or an empty editor
   * selection. Not per-target, so it survives a retarget.
   */
  private context: SelectionContext | null = null;
  private webviewReady = false;
  private pendingState: ViewState | null = null;
  /** The member whose surface is shown; every send is addressed there. */
  private target: string | null = null;

  private constructor(
    panel: vscode.WebviewPanel,
    mediaRoot: vscode.Uri,
    handlers: PanelHandlers,
    state: ViewState,
  ) {
    this.panel = panel;
    this.handlers = handlers;
    this.state = state;
    // The panel starts attached to whatever the state was already pointed at.
    this.target = state.targeted;
    this.throttle = createThrottle(THROTTLE_MS, (rendered) => this.post(rendered), realScheduler);

    this.panel.webview.html = renderHtml(this.panel.webview, mediaRoot);
    this.disposables.push(
      this.panel.webview.onDidReceiveMessage((raw: unknown) => this.onMessage(raw)),
      this.panel.onDidDispose(() => this.dispose()),
    );
  }

  /** Create the panel, or reveal the existing one. */
  static createOrShow(extensionUri: vscode.Uri, handlers: PanelHandlers, state: ViewState): ChatPanel {
    if (ChatPanel.current) {
      ChatPanel.current.state = state;
      ChatPanel.current.panel.reveal(vscode.ViewColumn.Beside, true);
      ChatPanel.current.flush();
      return ChatPanel.current;
    }
    const mediaRoot = vscode.Uri.joinPath(extensionUri, "media");
    const panel = vscode.window.createWebviewPanel(
      "wcode.chat",
      "wcode",
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      {
        enableScripts: true,
        localResourceRoots: [mediaRoot],
        retainContextWhenHidden: true,
      },
    );
    ChatPanel.current = new ChatPanel(panel, mediaRoot, handlers, state);
    return ChatPanel.current;
  }

  update(state: ViewState, immediate = false): void {
    this.state = state;
    if (!this.webviewReady) {
      this.pendingState = state;
      return;
    }
    if (immediate) this.flush();
    else this.throttle.push(renderState(state, this.target));
  }

  /** Update the status strip (session id / FSM / stderr tail). */
  setSession(info: Partial<PanelSessionInfo>): void {
    this.session = { ...this.session, ...info };
    this.flush();
  }

  /** Record the active editor's selection (host-global; NOT per-target). */
  setContext(context: SelectionContext | null): void {
    this.context = context;
    this.flush();
  }

  /**
   * Point the surface at a member (null = the root) and hydrate it. A retarget
   * only changes WHICH per-session transcript is rendered — nothing is lost.
   */
  setTarget(id: string | null): void {
    if (this.target === id) return;
    this.target = id;
    this.handlers.onTarget(id);
    this.flush();
  }

  get currentTarget(): string | null {
    return this.target;
  }

  reveal(): void {
    this.panel.reveal(vscode.ViewColumn.Beside, true);
  }

  /** The open panel, if any (the extension drives it from its event handlers). */
  static currentPanel(): ChatPanel | undefined {
    return ChatPanel.current;
  }

  /** Dispose the singleton, if any (extension `deactivate`). */
  static disposeCurrent(): void {
    ChatPanel.current?.dispose();
  }

  dispose(): void {
    if (ChatPanel.current === this) ChatPanel.current = undefined;
    this.throttle.dispose();
    for (const disposable of this.disposables.splice(0)) disposable.dispose();
    this.panel.dispose();
  }

  /* ---------------------------------------------------------------- private */

  private flush(): void {
    if (!this.webviewReady) {
      this.pendingState = this.state;
      return;
    }
    this.throttle.flush(renderState(this.state, this.target));
  }

  private post(state: ReturnType<typeof renderState>): void {
    const message: ToWebview = { kind: "state", state, session: this.session, context: this.context };
    void this.panel.webview.postMessage(message);
  }

  private onMessage(raw: unknown): void {
    const message = parseFromWebview(raw);
    if (message === null) return;
    switch (message.kind) {
      case "ready":
        this.webviewReady = true;
        this.state = this.pendingState ?? this.state;
        this.pendingState = null;
        this.flush();
        break;
      case "submit":
        this.handlers.onSubmit(message.text, this.target);
        break;
      case "steer":
        this.handlers.onSteer(message.text, this.target);
        break;
      case "cancel":
        this.handlers.onCancel(this.target);
        break;
      case "open-diff":
        this.handlers.onOpenDiff(message.callId);
        break;
      case "reveal-file":
        this.handlers.onRevealFile(message.path, message.line);
        break;
      case "toggle-plan":
        this.handlers.onTogglePlan();
        break;
    }
  }
}

/** The panel document: strict CSP, no remote resources, one bundled script. */
function renderHtml(webview: vscode.Webview, mediaRoot: vscode.Uri): string {
  const script = webview.asWebviewUri(vscode.Uri.joinPath(mediaRoot, "chat.js"));
  const style = webview.asWebviewUri(vscode.Uri.joinPath(mediaRoot, "chat.css"));
  const csp = [
    "default-src 'none'",
    `img-src ${webview.cspSource}`,
    `style-src ${webview.cspSource}`,
    `script-src ${webview.cspSource}`,
  ].join("; ");
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta http-equiv="Content-Security-Policy" content="${csp}" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <link rel="stylesheet" href="${style}" />
    <title>wcode</title>
  </head>
  <body>
    <div id="app"></div>
    <script src="${script}"></script>
  </body>
</html>`;
}
