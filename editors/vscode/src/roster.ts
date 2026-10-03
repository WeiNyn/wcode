/**
 * The team sidebar — a native `TreeView`.
 *
 * The roster is small and mostly text (`label · state · action`), so a tree gives
 * native theming, codicon icons, keyboard navigation, accessibility and context
 * menus for free; a `WebviewViewProvider` would re-implement all of that and
 * duplicate the `render.ts` pipeline for no gain. The panel already carries the
 * heavy rendering.
 *
 * **Push, never poll.** `getChildren` returns the cached items and `set` is
 * called from the extension's event handler (the reducer's output); there is no
 * `setInterval` and no manual refresh command.
 *
 * `roster.ts` is one of the few `vscode` importers (`extension.ts` is the
 * composition root; `panel.ts` and `diffProvider.ts` are the others). The tree's
 * data — `RosterItem`, `memberViews`, `memberIconSpec` — stays PURE in
 * `reducer.ts`, so it is testable without VS Code.
 */
import * as vscode from "vscode";

import { memberIconSpec, type RosterItem } from "./reducer.ts";

export type { RosterItem };

export class RosterProvider implements vscode.TreeDataProvider<RosterItem> {
  private items: RosterItem[] = [];
  private readonly changed = new vscode.EventEmitter<void>();

  readonly onDidChangeTreeData = this.changed.event;

  /** Replace the rows and repaint — but only when something actually changed. */
  set(items: RosterItem[]): void {
    if (sameItems(this.items, items)) return;
    this.items = items;
    this.changed.fire();
  }

  /** The cached row for a session id (for `reveal`). */
  find(id: string): RosterItem | undefined {
    return this.items.find((item) => item.id === id);
  }

  getTreeItem(item: RosterItem): vscode.TreeItem {
    const treeItem = new vscode.TreeItem(item.label, vscode.TreeItemCollapsibleState.None);
    treeItem.id = item.id;
    // The dim right-hand text: the live action while a run is in flight, else
    // the member's liveness.
    treeItem.description = item.liveAction ?? item.state;
    treeItem.tooltip = item.liveAction === undefined
      ? `${item.model ?? "unknown model"} · ${item.state}`
      : `${item.model ?? "unknown model"} · ${item.state} · ${item.liveAction}`;
    const icon = memberIconSpec(item.state);
    treeItem.iconPath = new vscode.ThemeIcon(icon.icon, new vscode.ThemeColor(icon.color));
    // `wcode.root` / `wcode.member` drive the `view/item/context` menu `when`s.
    treeItem.contextValue = item.isRoot ? "wcode.root" : "wcode.member";
    treeItem.command = { command: "wcode.member.focus", title: "Focus", arguments: [item.id] };
    return treeItem;
  }

  getChildren(item?: RosterItem): RosterItem[] {
    return item === undefined ? this.items : [];
  }

  dispose(): void {
    this.changed.dispose();
  }
}

/** A cheap structural compare: the roster is a handful of small rows. */
function sameItems(a: RosterItem[], b: RosterItem[]): boolean {
  return a.length === b.length && a.every((item, index) => sameItem(item, b[index]));
}

function sameItem(a: RosterItem, b: RosterItem): boolean {
  return (
    a.id === b.id &&
    a.label === b.label &&
    a.model === b.model &&
    a.state === b.state &&
    a.isRoot === b.isRoot &&
    a.liveAction === b.liveAction
  );
}
