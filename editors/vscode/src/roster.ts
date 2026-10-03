/**
 * The team sidebar — a native `TreeView`.
 *
 * The sidebar is small and mostly text (`label · state · action`), so a tree gives
 * native theming, codicon icons, keyboard navigation, accessibility and context
 * menus for free; a `WebviewViewProvider` would re-implement all of that and
 * duplicate the `render.ts` pipeline for no gain. The panel already carries the
 * heavy rendering.
 *
 * ONE view, whose ROOTS are the section nodes ("Team", "Tasks"). Section roots give
 * native collapse and a REMEMBERED open/closed state (keyed by the stable
 * `TreeItem.id`) for free, and keep ONE provider + ONE push path — a second
 * `wcode.tasks` view would double the provider, the change event, `viewsWelcome` and
 * the menus for no gain.
 *
 * **Push, never poll.** `getChildren` returns the cached sections and `set` is called
 * from the extension's event handler (the reducer's output); there is no
 * `setInterval` and no manual refresh command.
 *
 * `roster.ts` is one of the few `vscode` importers (`extension.ts` is the composition
 * root; `panel.ts` and `diffProvider.ts` are the others). The tree's data — the
 * section model, `memberViews`, `memberIconSpec`, `todoIconSpec` — stays PURE in
 * `reducer.ts`, so it is testable without VS Code.
 */
import * as vscode from "vscode";

import {
  memberIconSpec,
  todoIconSpec,
  type MemberNode,
  type RosterItem,
  type SectionNode,
  type SidebarNode,
} from "./reducer.ts";

export type { RosterItem };

/**
 * The sidebar's data provider over the sectioned tree model. Push-only: `set` fires
 * the change event only on a real change (a structural compare).
 */
export class SidebarProvider implements vscode.TreeDataProvider<SidebarNode> {
  private sections: SectionNode[] = [];
  private readonly changed = new vscode.EventEmitter<void>();

  readonly onDidChangeTreeData = this.changed.event;

  /** Replace the sections and repaint — but only when something actually changed. */
  set(sections: SectionNode[]): void {
    if (sameSections(this.sections, sections)) return;
    this.sections = sections;
    this.changed.fire();
  }

  /** The cached member node for a session id (for `reveal`). */
  find(id: string): MemberNode | undefined {
    for (const section of this.sections) {
      for (const child of section.children) {
        if (child.kind === "member" && child.row.id === id) return child;
      }
    }
    return undefined;
  }

  getTreeItem(node: SidebarNode): vscode.TreeItem {
    switch (node.kind) {
      case "section": {
        const item = new vscode.TreeItem(node.label, vscode.TreeItemCollapsibleState.Expanded);
        item.id = node.id; // stable -> VS Code remembers the user's collapse
        item.description = node.description; // the Tasks `☑ done/total`
        item.contextValue = "wcode.section"; // NOT matched by the member menu regex
        return item;
      }
      case "member":
        return memberTreeItem(node.row);
      case "todo": {
        const item = new vscode.TreeItem(node.row.label, vscode.TreeItemCollapsibleState.None);
        item.id = node.id;
        const icon = todoIconSpec(node.row.status);
        item.iconPath = new vscode.ThemeIcon(icon.icon, new vscode.ThemeColor(icon.color));
        item.contextValue = "wcode.todo"; // NOT matched by the member menu regex
        return item;
      }
    }
  }

  getChildren(node?: SidebarNode): SidebarNode[] {
    if (node === undefined) return this.sections;
    return node.kind === "section" ? node.children : []; // members / todos; leaves -> []
  }

  dispose(): void {
    this.changed.dispose();
  }
}

/**
 * One member `TreeItem` (the roster row). A `TreeItem` has ONE description slot and
 * CODICON icons (not the TUI glyphs ●/⠋/✓/○), so the model/root "meta" lives in the
 * TOOLTIP, not on the row — a deliberate native-idiom divergence from the draft.
 */
function memberTreeItem(item: RosterItem): vscode.TreeItem {
  const treeItem = new vscode.TreeItem(item.label, vscode.TreeItemCollapsibleState.None);
  treeItem.id = item.id;
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

/** A cheap structural compare: the sidebar is a handful of small sections/rows. */
function sameSections(a: SectionNode[], b: SectionNode[]): boolean {
  return a.length === b.length && a.every((section, index) => sameSection(section, b[index]));
}

function sameSection(a: SectionNode, b: SectionNode): boolean {
  return (
    a.id === b.id &&
    a.label === b.label &&
    a.description === b.description &&
    a.children.length === b.children.length &&
    a.children.every((child, index) => sameNode(child, b.children[index]))
  );
}

function sameNode(a: SidebarNode, b: SidebarNode): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "member" && b.kind === "member") return sameItem(a.row, b.row);
  if (a.kind === "todo" && b.kind === "todo") return a.row.label === b.row.label && a.row.status === b.row.status;
  return true; // sections never nest here
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
