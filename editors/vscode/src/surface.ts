import * as vscode from "vscode";

import { renderState } from "./render.ts";
import type { ViewState } from "./reducer.ts";
import type { Verdict } from "./review.ts";
import {
  createThrottle,
  parseFromWebview,
  realScheduler,
  type PanelSessionInfo,
  type SelectionContext,
  type Throttle,
  type ToWebview,
} from "./webview.ts";

/**
 * The shared surface host — V2. ONE controller owns the state, the handlers and the
 * throttle, and renders into any `SurfaceHost`: a sidebar/panel `WebviewView` or an
 * editor `WebviewPanel`. The old `ChatPanel` relied on ONE `WebviewPanel`; this splits
 * its "controller" half (below) from its "host" half (`panel.ts` keeps the panel, a new
 * `webviewView.ts` adds the view).
 */

/** ~30 ms: coalesce a fast token stream into at most ~33 posts/s (was `panel.ts`). */
export const THROTTLE_MS = 30;

/** What the surface calls back into the host for (was `panel.ts::PanelHandlers`). */
export interface SurfaceHandlers {
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
  /** The header target chip / a rail row picked a member: retarget + hydrate. */
  onFocusMember(id: string): void;
  /** The change review: `callId` + intent. The host settles it (Reject writes). */
  onReview(callId: string, verdict: "accept" | "reject"): void;
}

/**
 * Where a surface webview lives. A `WebviewPanel` host wraps it directly; a
 * `WebviewView` host wraps a `vscode.WebviewView` (which adds visibility, but has no
 * `retainContextWhenHidden` and no `reveal`).
 */
export interface SurfaceHost {
  /** Both `WebviewPanel` and `WebviewView` expose `.webview`. */
  readonly webview: vscode.Webview;
  /** Is the host currently shown? (a `WebviewView` may be hidden or not created). */
  readonly visible: boolean;
  /** `WebviewPanel.reveal(V.Column.Beside)`; a `WebviewView` reveals via a command. */
  reveal(): void;
  /** Post a snapshot into this host (the controller serialises; keep postMessage off the host). */
  post(message: ToWebview): void;
  /** The webview's postMessage stream. */
  onMessage(listener: (raw: unknown) => void): vscode.Disposable;
  /** The host went away (panel disposed / view closed) — detach it. */
  onDispose(listener: () => void): vscode.Disposable;
  /** A `WebviewView` only: hide/show. Absent for a `WebviewPanel`. */
  onDidChangeVisibility?(listener: (visible: boolean) => void): vscode.Disposable;
}

/**
 * The panel/view document: strict CSP, no remote resources, one bundled script. Lifted
 * verbatim from `panel.ts::renderHtml` so BOTH hosts share it.
 */
export function surfaceHtml(webview: vscode.Webview, mediaRoot: vscode.Uri): string {
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

/**
 * The one controller. It holds `state` / `session` / `context` / `verdicts` / `target`
 * / the `throttle` / the ready handshake — everything `ChatPanel` held today — and
 * broadcasts to EVERY attached host (a `WebviewView` and an editor `WebviewPanel` can
 * be open at once).
 */
export class SurfaceController {
  /** The view's/diff document's resource root (needed by `attach`). */
  readonly mediaRoot: vscode.Uri;

  private readonly handlers: SurfaceHandlers;
  private readonly throttle: Throttle;
  private readonly hosts = new Set<SurfaceHost>();
  private readonly ready = new Set<SurfaceHost>();
  private readonly disposables = new Map<SurfaceHost, vscode.Disposable[]>();

  private state: ViewState;
  private session: PanelSessionInfo = { id: null, state: "stopped", stderrTail: "" };
  private context: SelectionContext | null = null;
  private verdicts: Record<string, Verdict> = {};
  private target: string | null = null;

  constructor(extensionUri: vscode.Uri, handlers: SurfaceHandlers, state: ViewState) {
    this.mediaRoot = vscode.Uri.joinPath(extensionUri, "media");
    this.handlers = handlers;
    this.state = state;
    this.target = state.targeted;
    this.throttle = createThrottle(THROTTLE_MS, (rendered) => this.broadcast(rendered), realScheduler);
  }

  /**
   * Attach a host: set its `webview.options` (enableScripts + localResourceRoots) and its
   * `webview.html`; wire `onMessage`/`onDispose`; mark it NOT-ready. `onMessage(listener)`
   * carries NO host identity, so `attach` CAPTURES the host in the listener closure (a
   * `ready` from ONE host must NOT ready the OTHERS).
   */
  attach(host: SurfaceHost): void {
    if (this.hosts.has(host)) return;
    this.hosts.add(host);
    host.webview.options = { enableScripts: true, localResourceRoots: [this.mediaRoot] };
    host.webview.html = surfaceHtml(host.webview, this.mediaRoot);
    this.disposables.set(host, [
      host.onMessage((raw) => this.onMessage(host, raw)),
      host.onDispose(() => this.detach(host)),
    ]);
  }

  /** Detach a host (the panel disposed / the view closed). */
  detach(host: SurfaceHost): void {
    if (!this.hosts.delete(host)) return;
    this.ready.delete(host);
    for (const disposable of this.disposables.get(host) ?? []) disposable.dispose();
    this.disposables.delete(host);
  }

  /** Reveal the surface: attached panel hosts to the beside column, and the docked view. */
  reveal(): void {
    for (const host of this.hosts) host.reveal();
    // A docked view is not a host until VS Code first resolves it; this command
    // (auto-generated for the view id) reveals it either way — a no-op when nothing docked.
    void vscode.commands.executeCommand("wcode.surface.focus");
  }

  update(state: ViewState, immediate = false): void {
    this.state = state;
    this.push(immediate);
  }

  setSession(info: Partial<PanelSessionInfo>): void {
    this.session = { ...this.session, ...info };
    this.push(true);
  }

  setContext(context: SelectionContext | null): void {
    this.context = context;
    this.push(true);
  }

  setVerdicts(verdicts: Record<string, Verdict>): void {
    this.verdicts = verdicts;
    this.push(true);
  }

  setTarget(id: string | null): void {
    if (this.target === id) return;
    this.target = id;
    this.handlers.onTarget(id);
    this.push(true);
  }

  get currentTarget(): string | null {
    return this.target;
  }

  dispose(): void {
    this.throttle.dispose();
    for (const host of [...this.hosts]) this.detach(host);
  }

  /* ---------------------------------------------------------------- private */

  /** Send a snapshot (throttled, or immediately). Held until a host reports `ready`. */
  private push(immediate: boolean): void {
    if (this.ready.size === 0) return; // a fresh host gets the state on its `ready`
    const rendered = renderState(this.state, this.target);
    if (immediate) this.throttle.flush(rendered);
    else this.throttle.push(rendered);
  }

  /** The throttle's sink: build the message ONCE and post it to every READY host. */
  private broadcast(state: ReturnType<typeof renderState>): void {
    const message: ToWebview = {
      kind: "state",
      state,
      session: this.session,
      context: this.context,
      verdicts: this.verdicts,
    };
    for (const host of this.hosts) {
      if (this.ready.has(host)) host.post(message);
    }
  }

  private onMessage(host: SurfaceHost, raw: unknown): void {
    const message = parseFromWebview(raw);
    if (message === null) return;
    switch (message.kind) {
      case "ready":
        this.ready.add(host);
        host.post({
          kind: "state",
          state: renderState(this.state, this.target),
          session: this.session,
          context: this.context,
          verdicts: this.verdicts,
        });
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
      case "focus-member":
        this.handlers.onFocusMember(message.id);
        break;
      case "review":
        this.handlers.onReview(message.callId, message.verdict);
        break;
    }
  }
}
