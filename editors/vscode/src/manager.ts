/**
 * The `Manager` — the multi-tab composition root (D013/W010 P2).
 *
 * It replaces the eight module singletons `extension.ts` used to hold with a
 * `Map<tabId, SessionTab>` plus the SHARED resources a workspace has once: the
 * `OutputChannel`, the status bar, the editor-selection source, and the ONE docked
 * `WebviewView` (re-bound to the active tab — VS Code exposes exactly one view per
 * contributed id, D013 A).
 *
 * Responsibilities:
 *   - open/close tabs (`newSession`, `/new`, `/resume`, `wcode.openInEditor`);
 *   - track the ACTIVE tab (panel focus → active) and route the `wcode.*` commands
 *     to it;
 *   - keep the status bar and every panel title in sync as tabs come and go.
 *
 * The diff provider registry (`diffProvider.ts`) is registered ONCE in `extension.ts`
 * and is deliberately NOT cleared per tab — tokens are unique and before-images
 * immutable (D013, §Q5).
 */
import * as vscode from "vscode";

import { ChatPanel } from "./panel.ts";
import { displayLabel } from "./reducer.ts";
import { listSessions, sessionDir } from "./sessions.ts";
import { listTeams } from "./teams.ts";
import type { SurfaceHost } from "./surface.ts";
import { SessionTab, type SessionTabDeps } from "./tab.ts";
import { nextActiveTab, tabTitle } from "./tabs.ts";
import type { ViewBinder } from "./webviewView.ts";
import { type FoldDefault, type SelectionContext, type ViewPrefs } from "./webview.ts";

export class Manager implements vscode.Disposable, ViewBinder {
  /** The ONE diagnostic channel (shared; D013 Q5). A log channel: levels + timestamps. */
  readonly output: vscode.LogOutputChannel;
  /** The status bar reflects the ACTIVE tab's session (D013 Q5). */
  private readonly status: vscode.StatusBarItem;

  private readonly extensionUri: vscode.Uri;
  private readonly tabs = new Map<string, SessionTab>();
  /** Open order — the fallback order for `nextActiveTab` and the label index. */
  private readonly order: string[] = [];
  /** Editor panels, keyed by tab id (a tab has 0 or 1 panel). */
  private readonly panels = new Map<string, ChatPanel>();
  private readonly disposables: vscode.Disposable[] = [];

  private activeId: string | null = null;
  private lastActiveId: string | null = null;
  private viewHost: SurfaceHost | undefined;
  private context: SelectionContext | null = null;
  private counter = 0;
  private nextIndex = 1;

  constructor(extensionUri: vscode.Uri) {
    this.extensionUri = extensionUri;
    this.output = vscode.window.createOutputChannel("wcode", { log: true });
    this.status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    this.status.command = "wcode.restart";
    this.status.tooltip = "wcode session: click to restart";
    this.disposables.push(this.output, this.status);

    // The selection is STICKY: a `undefined` editor (focus moved to the webview)
    // KEEPS the last selection. Seed once; the listeners keep it in sync after.
    const editor = vscode.window.activeTextEditor;
    if (editor !== undefined) this.context = selectionOf(editor);
    this.disposables.push(
      vscode.window.onDidChangeTextEditorSelection((event) => this.setContext(selectionOf(event.textEditor))),
      vscode.window.onDidChangeActiveTextEditor((editor) => {
        if (editor === undefined) return;
        this.setContext(selectionOf(editor));
      }),
      // A settings change (D015) repaints every tab's folds; `mode` is the default for
      // a NEW session, so existing tabs keep the mode they are already showing.
      vscode.workspace.onDidChangeConfiguration((event) => {
        if (!event.affectsConfiguration("wcode")) return;
        const prefs = this.prefs();
        for (const tab of this.tabs.values()) tab.controller.setPrefs(prefs);
      }),
    );
  }

  /** The `media/` root — the docked view provider needs it at resolve time. */
  get mediaRoot(): vscode.Uri {
    return vscode.Uri.joinPath(this.extensionUri, "media");
  }

  /* ------------------------------------------------------------ commands */

  /** `wcode: Start Session` — start the active tab, or open one if none exists. */
  startSession(): void {
    const active = this.active();
    if (active === undefined) {
      this.openTab([]);
      return;
    }
    void active.start();
  }

  /** `wcode: Stop Session` — an explicit stop survives a webview `ready`. */
  stopSession(): void {
    this.active()?.stopByUser();
  }

  /** `wcode: Restart` — reload THIS tab with the same args. */
  reloadSession(): void {
    this.active()?.reload();
  }

  /** `wcode: New Session` / `/new` — a FRESH session in a NEW tab (D013 B/E). */
  newSession(): void {
    this.openTab([]);
  }

  /** `wcode: Open in Editor` — reveal (or open) an editor tab for the ACTIVE session. */
  openInEditor(): void {
    const active = this.active() ?? this.createSession([], null);
    this.openPanelFor(active).reveal();
  }

  /** `wcode: Open Diff` — the active session's latest (or `callId`'s) change. */
  openDiff(): void {
    const active = this.active();
    if (active === undefined) {
      void vscode.window.showWarningMessage("wcode: no session is open");
      return;
    }
    void active.showDiff(undefined);
  }

  /** `wcode: Focus Panel` — reveal the docked sidebar/panel (the ONE `WebviewView`). */
  focusPanel(): void {
    void vscode.commands.executeCommand("wcode.surface.focus");
  }

  /** `wcode: Show Log` — reveal the shared diagnostic OutputChannel. */
  showLog(): void {
    this.output.show(true);
  }

  /**
   * A run finished (`agent_end`) for `sessionId` — the root or a member. Notify
   * with a toast (click through to the tab) UNLESS the user is currently looking
   * at that session's surface: its editor panel is the active editor, or the
   * docked sidebar is showing it. An UNFOCUSED window (the user is in another app)
   * always notifies — that is the case the alert exists for.
   */
  notifyTurnEnd(tab: SessionTab, sessionId: string): void {
    if (this.sessionInFocus(tab)) return;
    const member = tab.viewState.members.find((m) => m.id === sessionId);
    const who = member === undefined ? "session" : displayLabel(member);
    void vscode.window.showInformationMessage(`wcode: ${who} finished`, "Show").then((choice) => {
      if (choice === "Show") this.revealTab(tab);
    });
  }

  /** Is the user looking at this session right now (window focused + its surface shown)? */
  private sessionInFocus(tab: SessionTab): boolean {
    if (!vscode.window.state.focused) return false; // away — always notify
    if (this.panels.get(tab.id)?.isActive === true) return true; // its editor tab is active
    // The docked sidebar is showing this tab, and it is visible.
    return this.activeId === tab.id && this.viewHost?.visible === true;
  }

  private revealTab(tab: SessionTab): void {
    this.setActive(tab.id);
    const panel = this.panels.get(tab.id);
    if (panel !== undefined) panel.reveal();
    else void vscode.commands.executeCommand("wcode.surface.focus");
  }

  /** `wcode.member.focus` — a menu/tree row names a member in the ACTIVE session. */
  focusMember(id: string): void {
    this.active()?.focusMember(id);
  }

  peekMember(): void {
    void this.active()?.peekMember(undefined);
  }

  askMember(): void {
    void this.active()?.askMember(undefined);
  }

  /** `stop` is NOT inert: it stops the FOCUSED member (the palette has no menu id). */
  stopMember(): void {
    const active = this.active();
    if (active === undefined) return;
    active.stopMember(active.targeted ?? undefined);
  }

  togglePlan(): void {
    this.active()?.togglePlan();
  }

  /** `/resume`: an explicit path opens a NEW tab; no argument opens a picker. */
  async resumeSession(arg: string): Promise<void> {
    if (arg !== "") {
      this.output.appendLine(`resuming ${arg}`);
      this.openTab(["--resume", arg], arg);
      return;
    }
    const cwd = this.workspaceRoot();
    const choices = await listSessions(sessionDir(), cwd);
    if (choices.length === 0) {
      void vscode.window.showInformationMessage(`wcode: no sessions recorded for ${cwd ?? "(no workspace)"}`);
      return;
    }
    const picked = await vscode.window.showQuickPick(
      choices.map((choice) => ({
        label: choice.label,
        description: choice.when,
        detail: choice.path,
        path: choice.path,
      })),
      { title: "wcode: resume a session", placeHolder: "Newest first: only this workspace", matchOnDetail: true },
    );
    if (picked === undefined) return;
    this.output.appendLine(`resuming ${picked.path}`);
    this.openTab(["--resume", picked.path], picked.path);
  }

  /** The editor selection changed: push the sticky value to EVERY tab (D013 Q5). */
  setContext(context: SelectionContext | null): void {
    this.context = context;
    for (const tab of this.tabs.values()) tab.controller.setContext(context);
  }

  /**
   * `/team`: an explicit name opens a NEW tab with that project team; no argument
   * opens a picker over `.wcode/teams/*.toml` (D017). The CLI resolves the file
   * via `--team <name>`, so this never builds a path.
   */
  private async teamTab(arg: string): Promise<void> {
    if (arg !== "") {
      this.output.appendLine(`team ${arg}`);
      this.openTab(["--team", arg], null, arg);
      return;
    }
    const root = this.workspaceRoot();
    const teams = root === undefined ? [] : await listTeams(root);
    if (teams.length === 0) {
      void vscode.window.showInformationMessage(
        "wcode: no project teams (add .wcode/teams/<name>.toml)",
      );
      return;
    }
    const picked = await vscode.window.showQuickPick(
      teams.map((name) => ({ label: name, name })),
      { title: "wcode: start a session with a project team", placeHolder: "Teams in .wcode/teams" },
    );
    if (picked === undefined) return;
    this.output.appendLine(`team ${picked.name}`);
    this.openTab(["--team", picked.name], null, picked.name);
  }

  /* -------------------------------------------------------------- the view */

  /** The ONE docked `WebviewView` resolved: bind it to the active session (D013 A). */
  bindView(host: SurfaceHost): void {
    this.viewHost = host;
    this.disposables.push(host.onDispose(() => {
      if (this.viewHost === host) this.viewHost = undefined;
    }));
    // A sidebar with no session is the intent to work — start one (no editor tab).
    if (this.active() === undefined) this.createSession([], null);
    this.rebindView();
  }

  /* --------------------------------------------------------------- tabs */

  /** A NEW session in a NEW editor tab. */
  private openTab(args: string[], resumePath: string | null = null, label?: string): SessionTab {
    const tab = this.createSession(args, resumePath, label);
    this.openPanelFor(tab);
    return tab;
  }

  /** A new session, no panel (the sidebar-only home; `openInEditor` adds one later). */
  private createSession(args: string[], resumePath: string | null, label?: string): SessionTab {
    this.counter += 1;
    const id = `tab-${this.counter}`;
    const tab = new SessionTab(this.deps(), {
      id,
      index: this.nextIndex++,
      args,
      resumePath,
      label,
    });
    this.tabs.set(id, tab);
    this.order.push(id);
    tab.controller.setContext(this.context);
    this.setActive(id);
    void tab.start();
    return tab;
  }

  /** Open (or reveal) the editor tab hosting `tab`. */
  private openPanelFor(tab: SessionTab): ChatPanel {
    const existing = this.panels.get(tab.id);
    if (existing !== undefined) {
      existing.reveal();
      return existing;
    }
    const panel = ChatPanel.open(this.extensionUri, tab.controller, tabTitle(tab.currentLabel));
    this.panels.set(tab.id, panel);
    panel.onDidDispose(() => this.onPanelClosed(tab, panel));
    panel.onDidChangeActive((active) => {
      if (active) this.setActive(tab.id);
    });
    return panel;
  }

  /** A tab's editor panel closed (its ×). Stop its child unless it is the sole session. */
  private onPanelClosed(tab: SessionTab, panel: ChatPanel): void {
    if (this.panels.get(tab.id) === panel) this.panels.delete(tab.id);
    // A tab with no other panel is gone — stop its child and forget it. The ONE
    // session case is kept: the docked sidebar is still its home (today's flow).
    if (this.panels.has(tab.id)) return;
    if (this.tabs.size <= 1) return;
    this.closeSession(tab);
  }

  private closeSession(tab: SessionTab): void {
    if (!this.tabs.delete(tab.id)) return;
    const at = this.order.indexOf(tab.id);
    if (at >= 0) this.order.splice(at, 1);
    tab.dispose(); // stops the child and disposes the controller (detaching any host)
    this.output.appendLine(`closed ${tab.id}`);
    if (this.activeId === tab.id) {
      this.activeId = nextActiveTab(this.order, "", this.lastActiveId);
    }
    this.rebindView();
    this.updateStatus();
  }

  /** The active tab: the focused one, else the last active, else none. */
  private active(): SessionTab | undefined {
    return this.activeId === null ? undefined : this.tabs.get(this.activeId);
  }

  private setActive(id: string): void {
    if (this.activeId === id || !this.tabs.has(id)) return;
    this.lastActiveId = this.activeId;
    this.activeId = id;
    this.rebindView();
    this.updateStatus();
  }

  /** Re-point the ONE docked view at the active session's controller. */
  private rebindView(): void {
    const host = this.viewHost;
    if (host === undefined) return;
    for (const tab of this.tabs.values()) tab.controller.detach(host);
    this.active()?.controller.attach(host);
  }

  /* ------------------------------------------------------------ lifecycle */

  dispose(): void {
    for (const panel of this.panels.values()) panel.dispose();
    this.panels.clear();
    for (const tab of this.tabs.values()) tab.dispose();
    this.tabs.clear();
    this.order.length = 0;
    this.activeId = null;
    for (const disposable of this.disposables) disposable.dispose();
  }

  /* -------------------------------------------------------------- private */

  private deps(): SessionTabDeps {
    return {
      extensionUri: this.extensionUri,
      workspaceRoot: () => this.workspaceRoot(),
      output: this.output,
      logLine: (message) => this.output.appendLine(message),
      openTab: (args) => {
        this.openTab(args);
      },
      resumeTab: (arg) => {
        void this.resumeSession(arg);
      },
      teamTab: (arg) => {
        void this.teamTab(arg);
      },
      onStateChange: () => this.updateStatus(),
      onLabelChange: (tab) => {
        const panel = this.panels.get(tab.id);
        panel?.setTitle(tabTitle(tab.currentLabel));
      },
      onTurnEnd: (tab, sessionId) => this.notifyTurnEnd(tab, sessionId),
      prefs: () => this.prefs(),
    };
  }

  private updateStatus(): void {
    const active = this.active();
    if (active === undefined) {
      this.status.hide();
      return;
    }
    this.status.text = `wcode: ${active.sessionState}`;
    this.status.show();
  }

  private workspaceRoot(): string | undefined {
    return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  }

  /** The presentation preferences (D015), read live from the `wcode.*` settings. */
  private prefs(): ViewPrefs {
    const cfg = vscode.workspace.getConfiguration("wcode");
    return {
      mode: cfg.get<string>("view.mode") === "focus" ? "focus" : "all",
      thinking: foldDefault(cfg.get<string>("view.thinking")),
      tools: foldDefault(cfg.get<string>("view.tools")),
    };
  }
}

/** One `FoldDefault` from a setting, or `"auto"` for anything unexpected. */
function foldDefault(raw: unknown): FoldDefault {
  return raw === "collapsed" || raw === "expanded" ? raw : "auto";
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
