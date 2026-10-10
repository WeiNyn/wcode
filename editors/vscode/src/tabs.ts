/**
 * The multi-tab decisions, kept PURE (D013) — no `vscode`, no I/O — so plain node
 * drives them under `npm test`. The vscode-importing half is `tab.ts`/`manager.ts`;
 * every choice that can be a `(input) -> output` function lives here instead, so it
 * is testable without a VS Code host (see `purity.test.ts`).
 *
 * Three decisions from D013 are encoded here:
 *   - **routing** (`commandAction`): `/new` and `/resume` open a NEW tab; `/reload`
 *     reloads THIS tab; `/sessions` keeps its member meaning; everything else is a
 *     plain `Request` to this tab's child (D013 E).
 *   - **labels** (`tabLabel` + `stickyLabel`): a tab is labelled ONCE from the root
 *     transcript's first user message; before that, the `--resume` basename, else a
 *     `wcode <n>` placeholder — never re-derived live (D013 C).
 *   - **active tab** (`nextActiveTab`): the docked sidebar follows the focused tab,
 *     falling back to the last active, then the first (D013 A).
 */
import type { SlashCommand } from "./commands.ts";
import type { Block, ViewState } from "./reducer.ts";
import { parseInbound, transcriptOf } from "./reducer.ts";

/* ------------------------------------------------------------------- routing */

/**
 * What a `/` command DOES above the wire. `request` is the ordinary case: the
 * command becomes a `Request` sent to this tab's child. The other four are
 * composition-root actions (they open/reload a tab or open a picker), which the
 * `Manager` performs — a `/` command never talks to the wire itself.
 */
export type TabAction = "new" | "resume" | "reload" | "members" | "team" | "request";

/**
 * Route a resolved `/` command to a tab-level action (D013 E). `command.name` is
 * the CANONICAL name (an alias was already resolved by `findCommand`), so `clear`
 * arrives here as `new`. An unresolved command (`undefined`) degrades to `request`
 * — it is not this table's job to reject; the host refuses an unknown command
 * before it routes.
 */
export function commandAction(command: SlashCommand | undefined): TabAction {
  switch (command?.name) {
    case "new":
      return "new";
    case "resume":
      return "resume";
    case "reload":
      return "reload";
    case "sessions":
      return "members";
    case "team":
      return "team";
    default:
      return "request";
  }
}

/* -------------------------------------------------------------------- labels */

/** The longest a derived label may be before it is clipped (a tab title is narrow). */
export const LABEL_MAX = 30;

/** `wcode <n>` — the placeholder a tab carries until it earns a real label (D013 C). */
export function placeholderLabel(index: number): string {
  return `wcode ${index}`;
}

/**
 * The editor-tab TITLE for a label (D013 C): the placeholder verbatim (`wcode 3`),
 * else `wcode: <label>`. Keeps the placeholder free of a dangling `wcode:` prefix.
 * Pure.
 */
export function tabTitle(label: string): string {
  return isPlaceholderLabel(label) ? label : `wcode: ${label}`;
}

/** Is `label` still the `wcode <n>` placeholder? Pure. */
export function isPlaceholderLabel(label: string): boolean {
  return /^wcode \d+$/.test(label);
}

/**
 * Sticky labelling (D013 C): once a REAL label is set, a later candidate never
 * overrides it; while the current label is still a placeholder, the candidate is
 * adopted (a placeholder candidate replaces a placeholder, harmlessly). Pure.
 */
export function stickyLabel(current: string, candidate: string): string {
  return isPlaceholderLabel(current) ? candidate : current;
}

/**
 * The best label for a tab NOW (D013 C): the root transcript's first USER message
 * (clipped), else the `--resume` path's basename, else the `wcode <n>` placeholder.
 * Pure, and NOT sticky by itself — the caller passes the result through
 * `stickyLabel` so a label set once is never re-derived from a moving transcript.
 */
export function tabLabel(state: ViewState, resumedPath: string | null, index: number): string {
  const first = firstUserText(rootTranscript(state));
  if (first !== null) return clip(first, LABEL_MAX);
  const base = basename(resumedPath);
  if (base !== null) return base;
  return placeholderLabel(index);
}

/**
 * The ROOT session's transcript — the human's own conversation, the one a tab is
 * named after. The root is the member flagged `isRoot` (the roster is root-first);
 * before a roster arrives there is nothing to label from. Pure.
 */
function rootTranscript(state: ViewState): Block[] {
  const root = state.members.find((m) => m.isRoot);
  return root === undefined ? [] : transcriptOf(state, root.id);
}

/**
 * The first USER block's text across a transcript, clipped-ready: tag-stripped and
 * whitespace-collapsed. A peer's inbound message is NOT the human's words and is
 * skipped (the `[message from <peer>]` tag). `null` when no user block speaks.
 * Mirrors `sessions.ts::firstUserText` (the `/resume` picker's label rule).
 */
function firstUserText(blocks: Block[]): string | null {
  for (const block of blocks) {
    if (block.kind !== "user") continue;
    const raw = block.text ?? "";
    const inbound = parseInbound(raw);
    if (inbound !== null && inbound.from !== "user") continue;
    const body = (inbound?.body ?? raw).replace(/\s+/g, " ").trim();
    if (body !== "") return body;
  }
  return null;
}

/** `…`-clip to `max` chars. Pure. */
function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/**
 * The basename of a filesystem path (POSIX `/` or Windows `\`), or `null` for
 * `null`/`""`. Hand-rolled so this module imports no `node:path` (purity). Pure.
 */
function basename(path: string | null): string | null {
  if (path === null) return null;
  const trimmed = path.replace(/[/\\]+$/, "");
  if (trimmed === "") return null;
  const parts = trimmed.split(/[/\\]/);
  return parts[parts.length - 1] || null;
}

/* -------------------------------------------------------------- active tab */

/**
 * Which tab the docked sidebar should show (D013 A). VS Code exposes exactly one
 * `WebviewView` per view id, so the sidebar follows ONE tab: the FOCUSED tab when it
 * is still open, else the LAST-ACTIVE one, else the first, else `null` (no tabs).
 * Pure — the `Manager` feeds it focus/open/close events and re-attaches the view.
 */
export function nextActiveTab(
  tabs: readonly string[],
  focused: string,
  lastActive: string | null = null,
): string | null {
  if (tabs.includes(focused)) return focused;
  if (lastActive !== null && tabs.includes(lastActive)) return lastActive;
  return tabs[0] ?? null;
}
