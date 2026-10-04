/**
 * The webview panel.
 *
 * Typed, dependency-free (esbuild wraps it as an IIFE). The host renders
 * markdown and posts pre-rendered HTML (`ToWebview`); this is a pure renderer of
 * that snapshot — it never parses markdown, never loads a remote resource, and
 * only talks to the host over `postMessage`.
 *
 * The PURE half lives in `./view.ts` (`stateLabel`, `panelHeader`, `turns`,
 * `diffStat`, `emptySpec`, `emptyKind`, `foldOpen`, `toggleFold`, `classNames`) and
 * `../webview.ts`
 * (`parseToWebview`), both of which plain node can drive. What remains here is
 * DOM-bound: element construction, scroll glue, the click/keydown handlers, and
 * `acquireVsCodeApi()`.
 */
import { filterCommands, parseSlash, type SlashCommand } from "../commands.ts";
import { memberGlyph, sidebarRails, todoGlyph, type RosterItem, type SidebarRails } from "../reducer.ts";
import type { RenderedBlock, RenderedState, RenderedTool } from "../render.ts";
import { reviewHunk, verdictOf, verdictUi, type ReviewHunk, type Verdict } from "../review.ts";
import { parseToWebview, type FromWebview, type PanelSessionInfo, type SelectionContext, type ToWebview, type ViewMode } from "../webview.ts";
import {
  classNames,
  composerControls,
  composeSubmit,
  diffStat,
  emptyKind,
  emptySpec,
  foldOpen,
  MEMBER_SWATCHES,
  memberInitial,
  memberSwatch,
  panelHeader,
  selectionRef,
  toggleFold,
  turns,
  workingGroup,
  type FoldOverrides,
  type HeaderCell,
  type MemberSwatch,
  type Turn,
  type TurnMember,
  type WorkingGroup,
} from "./view.ts";

/** The webview global; not in `@types/vscode` (it is injected by the host). */
interface VsCodeApi {
  /** Typed outbound — `FromWebview` is exhaustive, so a typo is a compile error. */
  postMessage(message: FromWebview): void;
  getState(): unknown;
  setState(state: unknown): void;
}
declare function acquireVsCodeApi(): VsCodeApi;

const vscode = acquireVsCodeApi();

/* ------------------------------------------------------------------ skeleton */

const app = requireEl("app");
app.innerHTML = [
  '<header id="phead" class="phead">',
  '  <div class="r1"></div>',
  '  <div class="r2"></div>',
  "</header>",
  '<div id="modes" class="modes" role="group" aria-label="View mode">',
  '  <button id="mAll" type="button" aria-pressed="true">All</button>',
  '  <button id="mFocus" type="button" aria-pressed="false">Focus</button>',
  "</div>",
  '<div id="ribbon" class="focusbar" aria-live="polite"></div>',
  '<div class="content">',
  '  <aside id="side" class="side" aria-label="Team and tasks"></aside>',
  '  <section class="main">',
  '    <main id="transcript" class="transcript" role="log" aria-label="Transcript"></main>',
  '    <footer id="composer" class="composer">',
  '      <div id="ctx-chips" class="ctx-chips" aria-label="Attached context"></div>',
  '      <div id="cmdmenu" class="cmdmenu" role="listbox" aria-label="Commands" hidden></div>',
  '      <textarea id="input" rows="1" spellcheck="false"',
  '        placeholder="Message wcode…  (Enter to send, Shift+Enter for newline, Esc to cancel)"></textarea>',
  '      <div class="ctoolbar">',
  '        <button id="cattach" class="tool-btn" type="button" title="Attach the current editor selection">@ selection</button>',
  '        <button id="cmode" class="tool-btn mode" type="button"></button>',
  '        <span id="cmodel" class="tool-btn model"></span>',
  '        <span class="spacer"></span>',
  '        <span id="hint" class="hint">Enter to send</span>',
  '        <button id="send" class="btn primary">Send</button>',
  '        <button id="cancel" class="btn" title="Cancel the in-flight run (Esc)">Stop</button>',
  '      </div>',
  '    </footer>',
  '  </section>',
  "</div>",
].join("\n");
const pheadEl = requireEl("phead");
const mAllEl = requireEl("mAll");
const mFocusEl = requireEl("mFocus");
const ribbonEl = requireEl("ribbon");

const transcriptEl = requireEl("transcript");
const sideEl = requireEl("side");
const inputEl = requireEl("input") as HTMLTextAreaElement;
const sendBtn = requireEl("send");
const cancelBtn = requireEl("cancel");
const cmodeEl = requireEl("cmode");
const cmodelEl = requireEl("cmodel");
const cattachEl = requireEl("cattach");
const ctxChipsEl = requireEl("ctx-chips");
const menuEl = requireEl("cmdmenu");

/** Per-callId tool-fold overrides (the user's manual toggle, keyed to its phase). */
let foldOverrides: FoldOverrides = new Map();
/** The last snapshot, so expand/collapse can re-render without the host. */
let lastState: ToWebview | null = null;
/** The last snapshot's verdicts, so a tool fold finds its verdict without a snapshot. */
let lastVerdicts: Record<string, Verdict> = {};

/** Does a context chip ride into the next submit? (webview-local, like `expanded`). */
let attached = true;
/** The selection from the LAST snapshot (for `composeSubmit` at submit time). */
let lastContext: SelectionContext | null = null;
/** The last rendered selection ref, so a NEW selection re-arms the chip. */
let lastRef: string | null = null;

/** Is the header target menu open? (webview-local view state). */
let menuOpen = false;
/** The roving-tabindex item of the open menu. */
let activeIndex = 0;
/** The header's cell+menu signature; a matching snapshot skips the rebuild (B1). */
let headerKey: string | null = null;
/** Is the team rail collapsed to its avatar strip? (webview-local view state). */
let railCollapsed = false;
/** The rail's team+tasks+target signature; a matching snapshot skips the rebuild. */
let railsKey: string | null = null;

/* ----------------------------------------------------------------- utilities */

function requireEl(id: string): HTMLElement {
  const node = document.getElementById(id);
  if (node === null) throw new Error(`wcode webview: #${id} is missing`);
  return node;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string | null,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className !== null && className !== "") node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function post(message: FromWebview): void {
  vscode.postMessage(message);
}

function nearBottom(): boolean {
  return transcriptEl.scrollHeight - transcriptEl.scrollTop - transcriptEl.clientHeight < 48;
}

function rerender(): void {
  if (lastState !== null) render(lastState);
}

/* ------------------------------------------------------------------ header */

function renderHeader(state: RenderedState, session: PanelSessionInfo): void {
  const key = headerSignature(state, session);
  if (key === headerKey) return; // same cells AND menu state: keep the SAME DOM (focus survives)
  const hadFocus = pheadEl.contains(document.activeElement);
  headerKey = key;
  const { r1, r2 } = panelHeader(state, session);
  const row1 = el("div", "r1");
  const row2 = el("div", "r2");
  for (const cell of r1) {
    row1.appendChild(cell.interactive ? targetChip(cell, state) : el("span", cell.className, cell.text));
  }
  for (const cell of r2) row2.appendChild(el("span", cell.className, cell.text));
  pheadEl.textContent = "";
  pheadEl.append(row1, row2);
  // Focus restore: the old chip is gone after a rebuild; re-find the new one.
  if (menuOpen) focusItem(activeIndex); // the menu reopened -> restore the roving item
  else if (hadFocus) chipButton()?.focus();
}

/**
 * The header's cell+menu signature. `renderHeader` rebuilds ONLY when it changes,
 * so a token stream (which leaves the cells and the menu unchanged) keeps the SAME
 * chip DOM — the open menu and its focus survive a streamed snapshot (B1).
 */
function headerSignature(state: RenderedState, session: PanelSessionInfo): string {
  const { r1, r2 } = panelHeader(state, session);
  const cells = [...r1, ...r2]
    .map((cell) => `${cell.className}\u0001${cell.text}\u0001${cell.interactive ?? false}`)
    .join("\u0002");
  // The MENU state is part of the gate, or open/close would change nothing.
  return `${cells}\u0002${menuOpen}\u0002${activeIndex}`;
}

/** The `.target` cell as a `<button>` + (when open) the member menu. */
function targetChip(cell: HeaderCell, state: RenderedState): HTMLElement {
  const wrap = el("span", "target-wrap");
  const button = el("button", cell.className, cell.text); // class "target"
  button.setAttribute("type", "button");
  button.setAttribute("aria-haspopup", "menu");
  button.setAttribute("aria-expanded", menuOpen ? "true" : "false");
  button.addEventListener("click", () => (menuOpen ? closeMenu() : openMenu()));
  wrap.appendChild(button);
  if (menuOpen) wrap.appendChild(memberMenu(state));
  return wrap;
}

/** The in-panel member list (draft: the chip's dropdown). */
function memberMenu(state: RenderedState): HTMLElement {
  const menu = el("div", "menu");
  menu.setAttribute("role", "menu");
  menu.setAttribute("aria-label", "Switch target member");
  state.members.forEach((member, index) => {
    const item = el("button", "item", member.isRoot ? "orchestrator" : member.label);
    item.setAttribute("type", "button");
    item.setAttribute("role", "menuitem");
    item.tabIndex = index === activeIndex ? 0 : -1; // ROVING tabindex: Tab LEAVES the menu
    if (member.id === state.target?.id) item.setAttribute("aria-current", "true");
    item.addEventListener("click", () => selectMember(member.id));
    item.addEventListener("keydown", (event) => onMenuKey(event, index, state.members.length));
    menu.appendChild(item);
  });
  return menu;
}

/** The chip button currently in the DOM, if any (re-found after a rebuild). */
function chipButton(): HTMLElement | null {
  return pheadEl.querySelector<HTMLElement>("button.target");
}

/** Focus the menu item at `index`. */
function focusItem(index: number): void {
  pheadEl.querySelectorAll<HTMLElement>(".menu .item")[index]?.focus();
}

/**
 * Move the roving-tabindex position to `index` (so Tab / Shift+Tab behave) and
 * focus that item. Keeping `activeIndex` and the DOM tabindex in step means the
 * next snapshot's rebuild re-focuses the SAME item — `renderHeader` restores
 * `focusItem(activeIndex)`, so an arrowed position survives a streamed snapshot.
 */
function moveTo(index: number): void {
  activeIndex = index;
  const items = pheadEl.querySelectorAll<HTMLElement>(".menu .item");
  items.forEach((item, i) => {
    item.tabIndex = i === index ? 0 : -1;
  });
  items[index]?.focus();
}

function openMenu(): void {
  menuOpen = true;
  activeIndex = 0;
  rerender();
  focusItem(0); // focus the first item
}

function closeMenu(): void {
  menuOpen = false;
  rerender();
  chipButton()?.focus(); // return focus to the chip
}

function selectMember(id: string): void {
  post({ kind: "focus-member", id });
  closeMenu();
}

/** The APG Menu-Button keyboard pattern (mirrors the WAI-ARIA example). */
function onMenuKey(event: KeyboardEvent, index: number, count: number): void {
  switch (event.key) {
    case "ArrowDown":
      moveTo((index + 1) % count);
      event.preventDefault();
      break;
    case "ArrowUp":
      moveTo((index - 1 + count) % count);
      event.preventDefault();
      break;
    case "Home":
      moveTo(0);
      event.preventDefault();
      break;
    case "End":
      moveTo(count - 1);
      event.preventDefault();
      break;
    case "Tab":
      closeMenu(); // Tab LEAVES the menu (roving tabindex)
      break;
    case "Escape":
      closeMenu(); // -> focus the chip
      event.preventDefault();
      break;
  }
}

// Outside click closes the menu (a click INSIDE `.target-wrap` — the chip or an
// item — is left to their own handlers).
document.addEventListener("click", (event) => {
  const target = event.target as Element | null;
  if (menuOpen && target !== null && !target.closest(".target-wrap")) closeMenu();
});

/* -------------------------------------------------------------- transcript */

function renderStateCard(state: RenderedState, session: PanelSessionInfo): HTMLElement {
  const spec = emptySpec(emptyKind(state, session));
  const card = el("div", "statecard");
  const body = el("div", "sc-body");

  const title = el("p", "empty-title");
  if (spec.spin) {
    const spin = el("span", "spin", "⠋");
    spin.setAttribute("aria-hidden", "true");
    title.appendChild(spin);
    title.appendChild(document.createTextNode(" "));
  }
  title.appendChild(document.createTextNode(spec.title));
  body.appendChild(title);
  body.appendChild(el("p", "empty-sub", spec.sub));
  if (spec.stderr && session.stderrTail !== "") {
    body.appendChild(el("pre", "stderr", session.stderrTail));
  }

  card.appendChild(body);
  return card;
}

function blockShell(block: RenderedBlock): HTMLElement {
  // ONE element: `classNames` already names the content element ("body" /
  // "body notice" / …), so there is no outer wrap + nested `.body`. No `.role`
  // line either — the `.who` line is per-TURN now.
  const node = el("div", classNames(block));
  node.innerHTML = block.html; // host-rendered; markdown-it `html: false` escaped it
  return node;
}

function renderToolFold(block: RenderedBlock): HTMLElement {
  const tool = block.tool;
  if (tool === undefined) return el("details", "fold tool");
  const open = foldOpen(foldOverrides, tool.callId, tool.done); // override wins; else open while running
  const details = el("details", classNames(block, open));
  if (open) details.setAttribute("open", "");

  const summary = el("summary", null);
  summary.appendChild(el("span", "chev"));
  summary.appendChild(el("span", "tname", `⚙ ${tool.name}`));
  // `.tsum` is the TARGET (the draft's `⚙ edit` + `src/panel.ts`), falling back to
  // the output summary (`tool.summary`) for a row with no target — a history row, or
  // a call with no recognizable arg. Omit the span when both are empty (exact TUI
  // parity — never an empty `<span class="tsum">`).
  const tsum = tool.target ?? tool.summary;
  if (tsum !== "") summary.appendChild(el("span", "tsum", tsum));
  const meta = toolMeta(tool);
  if (meta !== null) summary.appendChild(meta);
  // We drive the fold from the override map (so it survives a stream re-render):
  // suppress the native toggle, flip the override for this phase, repaint.
  summary.addEventListener("click", (event) => {
    event.preventDefault();
    foldOverrides = toggleFold(foldOverrides, tool.callId, tool.done);
    rerender();
  });
  details.appendChild(summary);

  const inner = el("div", "inner");
  const hunk = typeof tool.diff === "string" && tool.diff !== "" ? reviewHunk(tool.diff) : null;
  inner.appendChild(hunk !== null ? reviewBlock(tool, hunk, verdictOf(lastVerdicts, tool.callId)) : rawOutput(tool));
  details.appendChild(inner);
  return details;
}

/** The plain output `<pre>` for a tool with no diff (the draft's `.fold .out`). */
function rawOutput(tool: RenderedTool): HTMLElement {
  return el("pre", "tool-output", tool.outputText);
}

/** The in-panel change review for ONE diff-bearing tool (draft Change review). */
function reviewBlock(tool: RenderedTool, hunk: ReviewHunk, verdict: Verdict): HTMLElement {
  const ui = verdictUi(verdict); // ONE source of truth: class + badge label
  const settled = verdict !== "pending";
  const wrap = el("div", ui.className === "" ? "review" : `review ${ui.className}`);

  // The bar comes BEFORE the diff (the draft's order); the badge only when settled.
  const bar = el("div", "reviewbar");
  const stat = el("span", "stat");
  stat.appendChild(el("b", null, tool.path ?? ""));
  stat.appendChild(document.createTextNode(" · 1 change"));
  bar.appendChild(stat);
  bar.appendChild(el("span", "spacer"));
  if (ui.label !== "") bar.appendChild(el("span", "badge", ui.label));
  const open = el("button", "btn link", "Open native diff");
  open.setAttribute("type", "button");
  open.addEventListener("click", () => post({ kind: "open-diff", callId: tool.callId }));
  bar.appendChild(open);
  wrap.appendChild(bar);

  const hunkEl = el("div", "hunk");
  const head = el("div", "hhead");
  head.appendChild(el("span", null, hunk.header));
  head.appendChild(el("span", "spacer"));
  head.appendChild(el("span", null, hunk.truncated ? "truncated" : ""));
  hunkEl.appendChild(head);

  const lines = el("div", "lines");
  for (const line of hunk.lines) {
    const row = el("div", `ln ${line.kind}`);
    row.appendChild(el("span", "gutter", String(line.number)));
    row.appendChild(el("span", "txt", line.text));
    lines.appendChild(row);
  }
  hunkEl.appendChild(lines);

  // The webview does NOT flip the verdict locally: the HOST owns it and the next
  // snapshot settles it, so the buttons settle disabled only on a known verdict.
  const actions = el("div", "hactions");
  const accept = el("button", "btn primary", "Accept change");
  accept.setAttribute("type", "button");
  accept.disabled = settled;
  accept.addEventListener("click", () => {
    post({ kind: "review", callId: tool.callId, verdict: "accept" });
    rerender();
  });
  const reject = el("button", "btn", "Reject change");
  reject.setAttribute("type", "button");
  reject.disabled = settled;
  reject.addEventListener("click", () => {
    post({ kind: "review", callId: tool.callId, verdict: "reject" });
    rerender();
  });
  actions.appendChild(accept);
  actions.appendChild(reject);
  hunkEl.appendChild(actions);

  wrap.appendChild(hunkEl);
  return wrap;
}

/** The tool head's `+N −M · 38ms` meta, or null when it has neither. */
function toolMeta(tool: RenderedTool): HTMLElement | null {
  const meta = el("span", "tmeta");
  const stat = typeof tool.diff === "string" && tool.diff !== "" ? diffStat(tool.diff) : null;
  const hasStat = stat !== null && (stat.added > 0 || stat.removed > 0);
  if (stat !== null && stat.added > 0) meta.appendChild(el("span", "add", `+${stat.added}`));
  if (stat !== null && stat.removed > 0) {
    if (stat.added > 0) meta.appendChild(document.createTextNode(" "));
    meta.appendChild(el("span", "del", `−${stat.removed}`));
  }
  if (tool.done) {
    if (typeof tool.durationMs === "number") {
      if (hasStat) meta.appendChild(document.createTextNode(" · "));
      meta.appendChild(document.createTextNode(`${tool.durationMs}ms`));
    }
  } else {
    if (hasStat) meta.appendChild(document.createTextNode(" · "));
    // The tool row's ONE running indicator (right-aligned `.tmeta`, the same slot
    // that carries `+N −M · 38ms` once done); `.tsum` above never repeats it.
    meta.appendChild(el("span", "running-tag", "running…"));
  }
  return meta.childNodes.length > 0 ? meta : null;
}

function renderTurn(turn: Turn): HTMLElement {
  // The swatch rides the WRAPPER, so the rail (a sibling of the content) takes it too.
  const sw = turn.swatch === undefined ? "" : ` sw-${turn.swatch}`;
  const wrap = el("div", `turn ${turn.role}${sw}`);
  const rail = el("div", "rail");
  rail.setAttribute("aria-hidden", "true");
  wrap.appendChild(rail);

  const content = el("div", null);
  const who = el("div", turn.who.className);
  who.appendChild(el("span", "av", turn.who.avatar));
  who.appendChild(el("span", "name", turn.who.name)); // no `.stamp` — no time on the wire
  content.appendChild(who);
  for (const block of turn.blocks) {
    content.appendChild(block.kind === "tool" ? renderToolFold(block) : blockShell(block));
  }

  wrap.appendChild(content);
  return wrap;
}

/** The live subagent rows (draft `.group`): a transcript-tail region. */
function renderGroup(group: WorkingGroup): HTMLElement {
  const box = el("div", "group");
  const head = el("div", "ghead");
  const spin = el("span", "spin", "⠋");
  spin.setAttribute("aria-hidden", "true");
  head.appendChild(spin);
  head.appendChild(el("span", null, `${group.count} member${group.count === 1 ? "" : "s"} working`));
  box.appendChild(head);
  for (const row of group.rows) {
    const line = el("div", "running");
    line.appendChild(el("span", `glyph ${row.running ? "g-run" : "g-idle"}`, "●")); // constant ●
    line.appendChild(el("span", "who2", row.name));
    if (row.action !== "") line.appendChild(el("span", "what", row.action));
    box.appendChild(line);
  }
  return box;
}

function render(snapshot: ToWebview): void {
  const { state, session } = snapshot;
  const stick = nearBottom();
  app.dataset.mode = snapshot.mode; // drives `#app[data-mode]` (the ribbon's visibility)
  renderHeader(state, session);
  // The target chip would falsely claim the merged transcript is one member's: hide the
  // whole wrap in All mode on EVERY render (the header's early-return keeps stale DOM).
  const targetWrap = pheadEl.querySelector<HTMLElement>(".target-wrap");
  if (targetWrap !== null) targetWrap.hidden = snapshot.mode === "all";
  renderRails(state);
  renderComposer(state, snapshot.context);
  updateModes(snapshot.mode);
  renderRibbon(state, snapshot.mode);
  lastVerdicts = snapshot.verdicts;
  transcriptEl.textContent = "";
  if (state.blocks.length === 0) {
    transcriptEl.appendChild(renderStateCard(state, session));
  } else {
    const member = snapshot.mode === "all" ? memberOf(state) : undefined;
    for (const turn of turns(state.blocks, member, targetSwatch(state))) transcriptEl.appendChild(renderTurn(turn));
    // The working group is a transcript-TAIL region, NOT nested in a turn: it is
    // LIVE roster state (from `members`), not per-turn block data.
    const group = workingGroup(state.members, state.target?.id ?? null);
    if (group.count > 0) transcriptEl.appendChild(renderGroup(group));
  }
  if (stick) transcriptEl.scrollTop = transcriptEl.scrollHeight;
}

/* ------------------------------------------------------------------- mode */

function updateModes(mode: ViewMode): void {
  mAllEl.setAttribute("aria-pressed", mode === "all" ? "true" : "false");
  mFocusEl.setAttribute("aria-pressed", mode === "focus" ? "true" : "false");
}

function renderRibbon(state: RenderedState, mode: ViewMode): void {
  ribbonEl.textContent = "";
  if (mode !== "focus") return; // `#app[data-mode]` governs the display; stay empty in All
  ribbonEl.appendChild(document.createTextNode("Focus: "));
  ribbonEl.appendChild(el("b", null, state.target?.label ?? "wcode"));
  ribbonEl.appendChild(document.createTextNode(" · showing only this member's activity"));
  const showAll = el("button", "btn link", "Show all");
  showAll.setAttribute("type", "button");
  showAll.addEventListener("click", () => post({ kind: "set-mode", mode: "all" }));
  ribbonEl.appendChild(showAll);
}

/** Resolve a block's `origin` (or a peer's `from`) to the member that labels its turn. */
function memberOf(state: RenderedState): (origin: string | undefined) => TurnMember | undefined {
  return (origin) => {
    if (origin === undefined) return undefined;
    const index = state.members.findIndex((m) => m.id === origin);
    if (index < 0) {
      // Not in the roster (a peer that has since left): name it by its id, uncoloured.
      const name = origin.startsWith("agent:") ? origin.slice("agent:".length) : origin;
      return { name, isRoot: false, swatch: MEMBER_SWATCHES[0] };
    }
    const member = state.members[index];
    return {
      name: member.isRoot ? "orchestrator" : member.label,
      isRoot: member.isRoot,
      swatch: memberSwatch(index, member.isRoot),
    };
  };
}

/** The TARGET's swatch: Focus mode shows ONE member, so every turn takes its colour. */
function targetSwatch(state: RenderedState): MemberSwatch | undefined {
  const id = state.target?.id;
  if (id === undefined) return undefined;
  const index = state.members.findIndex((m) => m.id === id);
  return index < 0 ? undefined : memberSwatch(index, state.members[index].isRoot);
}

/* ---------------------------------------------------------------- composer */

function renderComposer(state: RenderedState, context: SelectionContext | null): void {
  const ref = context === null ? null : selectionRef(context);
  // A NEW selection re-arms the chip: selecting again always re-attaches.
  if (ref !== lastRef) {
    attached = true;
    lastRef = ref;
  }
  lastContext = context;

  ctxChipsEl.textContent = "";
  if (context !== null && attached) ctxChipsEl.appendChild(ctxChip(context));

  const { mode, model } = composerControls(state);
  cmodeEl.textContent = `Mode: ${mode}`;
  cmodeEl.setAttribute("aria-pressed", mode === "Plan" ? "true" : "false");
  // Render NOTHING when there is no model — no em-dash placeholder.
  cmodelEl.textContent = model === null ? "" : `Model: ${model}`;
  cmodelEl.hidden = model === null;
}

function ctxChip(context: SelectionContext): HTMLElement {
  const chip = el("span", "ctx");
  chip.appendChild(el("span", "mono", selectionRef(context)));
  const dismiss = el("button", "x", "×");
  dismiss.setAttribute("type", "button");
  dismiss.setAttribute("aria-label", "Remove the attached selection");
  dismiss.addEventListener("click", () => {
    attached = false;
    rerender();
  });
  chip.appendChild(dismiss);
  return chip;
}

/* ------------------------------------------------------------------- rail */

function renderRails(state: RenderedState): void {
  // The rail is DURABLE: the roster + the ROOT session's plan (the OVERALL plan, NOT the
  // shown member's) — `state.todos` is the root's (render.ts).
  const rails = sidebarRails(state.members, state.todos);
  const targetId = state.target?.id ?? null;
  // The rail changes only when the roster / todos / target change — NOT on every
  // streamed token. The signature keeps the SAME DOM (so a collapse toggle keeps its
  // focus) while a live action still repaints.
  const key = JSON.stringify([rails.team, rails.tasks, targetId]);
  if (key === railsKey) return;
  railsKey = key;
  sideEl.textContent = "";
  sideEl.appendChild(teamSection(rails.team, targetId));
  if (rails.tasks.rows.length > 0) sideEl.appendChild(tasksSection(rails.tasks));
  applyCollapsed();
}

/** The rail's collapse control, in the Team head. Its glyph/label/flags are set by
 *  `applyCollapsed`, which also runs after every rebuild. */
function sideToggle(): HTMLElement {
  const btn = el("button", "side-toggle");
  btn.setAttribute("type", "button");
  btn.addEventListener("click", () => {
    railCollapsed = !railCollapsed;
    applyCollapsed();
  });
  return btn;
}

/** Reflect `railCollapsed` on the rail. The class on `#side` drives the CSS; every
 *  toggle button keeps its glyph/label/`aria-expanded` in step. Idempotent, so it
 *  is safe to call after a rebuild. */
function applyCollapsed(): void {
  sideEl.classList.toggle("collapsed", railCollapsed);
  sideEl.querySelectorAll<HTMLElement>(".side-toggle").forEach((btn) => {
    btn.textContent = railCollapsed ? "»" : "«";
    btn.title = railCollapsed ? "Expand the team rail" : "Collapse the team rail";
    btn.setAttribute("aria-label", btn.title);
    btn.setAttribute("aria-expanded", railCollapsed ? "false" : "true");
  });
}

/** A member avatar: the swatch is IDENTITY (`--sw`, the fill), the `g-*` class is
 *  STATE (`--st`, the border), the initial is the glyph. A row avatar is decorative
 *  (`<span>`, aria-hidden); a strip chip is a `<button>`. */
function memberAvatar(row: RosterItem, index: number, interactive: boolean): HTMLElement {
  const cls = ["mav", `sw-${memberSwatch(index, row.isRoot)}`, memberGlyph(row.state).className];
  const initial = memberInitial(row.label, row.isRoot);
  if (!interactive) {
    const span = el("span", cls.join(" "), initial);
    span.setAttribute("aria-hidden", "true");
    return span;
  }
  const btn = el("button", cls.join(" "), initial);
  btn.setAttribute("type", "button");
  btn.title = `${row.label} · ${row.liveAction ?? row.state}`;
  btn.setAttribute("aria-label", `Focus ${btn.title}`);
  return btn;
}

function teamSection(rows: RosterItem[], targetId: string | null): HTMLElement {
  const box = el("div", "sec team-sec");
  const head = el("div", "side-head");
  head.appendChild(el("span", "side-title", "Team"));
  head.appendChild(el("span", "spacer"));
  head.appendChild(sideToggle());
  box.appendChild(head);

  const list = el("ul", "roster");
  rows.forEach((row, index) => {
    const item = el("li", row.id === targetId ? "sel" : null);
    item.appendChild(memberAvatar(row, index, false));
    const who = el("span", "who");
    who.appendChild(el("b", null, row.label));
    who.appendChild(el("span", "meta", row.model ?? (row.isRoot ? "root" : "")));
    item.appendChild(who);
    item.appendChild(el("span", "act-line", row.liveAction ?? row.state));
    // A rail row click RETARGETS — the same P3 path the header chip's menu uses.
    item.addEventListener("click", () => post({ kind: "focus-member", id: row.id }));
    list.appendChild(item);
  });
  box.appendChild(list);

  // The COLLAPSED form: the same members as a strip of avatar chips. The fill is the
  // member's swatch (identity), the ring is the state colour — legible at ~30px wide.
  const strip = el("ul", "avatars");
  rows.forEach((row, index) => {
    const cell = el("li", null);
    const chip = memberAvatar(row, index, true);
    chip.setAttribute("aria-current", row.id === targetId ? "true" : "false");
    chip.addEventListener("click", () => post({ kind: "focus-member", id: row.id }));
    cell.appendChild(chip);
    strip.appendChild(cell);
  });
  box.appendChild(strip);
  return box;
}

function tasksSection(tasks: SidebarRails["tasks"]): HTMLElement {
  const box = el("div", "sec tasks-sec");
  const head = el("div", "side-head");
  head.appendChild(el("span", null, "Tasks"));
  head.appendChild(el("span", "spacer"));
  head.appendChild(el("span", "seg", tasks.badge));
  box.appendChild(head);
  const list = el("ul", "todo");
  for (const row of tasks.rows) {
    const glyph = todoGlyph(row.status);
    const item = el("li", glyph.className === "" ? null : glyph.className);
    item.appendChild(el("span", "box", glyph.glyph));
    item.appendChild(el("span", null, row.label));
    list.appendChild(item);
  }
  box.appendChild(list);
  return box;
}

/* ------------------------------------------------------------- / commands */

/** The `/` menu's current items (webview-local: the INPUT drives them, not the host). */
let cmdItems: SlashCommand[] = [];
/** The highlighted item of the open menu. */
let cmdIndex = 0;

/**
 * The menu's query: the text after `/` while the FIRST word is still being typed,
 * else null. A space means the argument is being typed, so the menu closes and
 * the line submits normally.
 */
function cmdQuery(): string | null {
  const text = inputEl.value;
  if (!text.startsWith("/")) return null;
  const rest = text.slice(1);
  return /\s/.test(rest) ? null : rest;
}

/** Rebuild the menu from the input. Cheap: it runs on every keystroke. */
function renderCmdMenu(): void {
  const query = cmdQuery();
  if (query === null) {
    closeCmdMenu();
    return;
  }
  cmdItems = filterCommands(query);
  if (cmdItems.length === 0) {
    closeCmdMenu();
    return;
  }
  if (cmdIndex >= cmdItems.length) cmdIndex = 0;
  menuEl.textContent = "";
  cmdItems.forEach((command, index) => {
    const item = el("div", index === cmdIndex ? "cmd sel" : "cmd");
    item.setAttribute("role", "option");
    item.setAttribute("aria-selected", index === cmdIndex ? "true" : "false");
    item.appendChild(el("span", "cmd-name", `/${command.name}`));
    if (command.arg !== undefined) item.appendChild(el("span", "cmd-arg", command.arg));
    item.appendChild(el("span", "cmd-help", command.help));
    // `mousedown`, not `click`: preventDefault keeps focus in the textarea, so the
    // input's blur cannot tear the menu down before the handler runs.
    item.addEventListener("mousedown", (event: MouseEvent) => {
      event.preventDefault();
      runMenuCommand(command);
    });
    menuEl.appendChild(item);
  });
  menuEl.hidden = false;
}

function closeCmdMenu(): void {
  cmdItems = [];
  cmdIndex = 0;
  menuEl.hidden = true;
  menuEl.textContent = "";
}

/**
 * Run a command picked from the menu. One that NEEDS an argument only arms the
 * input (`/model `) — running it empty would be a round-trip the host must reject.
 */
function runMenuCommand(command: SlashCommand): void {
  if (command.requiresArg === true) {
    inputEl.value = `/${command.name} `;
    closeCmdMenu();
    autoGrow();
    inputEl.focus();
    return;
  }
  inputEl.value = "";
  autoGrow();
  closeCmdMenu();
  post({ kind: "command", name: command.name, arg: "" });
  inputEl.focus();
}

/* -------------------------------------------------------------------- input */

// Grow via the `rows` attribute, not an inline style: the panel's CSP is
// `style-src ${webview.cspSource}` with no 'unsafe-inline'.
function autoGrow(): void {
  const lines = inputEl.value.split("\n").length;
  inputEl.rows = Math.max(1, Math.min(lines, 8));
}

function submit(): void {
  const text = inputEl.value;
  if (text.trim() === "") return;
  // A KNOWN `/name` line is a command; anything else (including an unknown `/…`,
  // the CLI's rule) is prompt text for the model.
  const slash = parseSlash(text);
  inputEl.value = "";
  autoGrow();
  closeCmdMenu();
  if (slash !== null) post({ kind: "command", name: slash.command.name, arg: slash.arg });
  else post({ kind: "submit", text: composeSubmit(text, attached ? lastContext : null) });
  inputEl.focus();
}

inputEl.addEventListener("input", () => {
  autoGrow();
  renderCmdMenu();
});
inputEl.addEventListener("keydown", (event: KeyboardEvent) => {
  if (menuEl.hidden === false && cmdItems.length > 0) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      cmdIndex = (cmdIndex + 1) % cmdItems.length;
      renderCmdMenu();
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      cmdIndex = (cmdIndex - 1 + cmdItems.length) % cmdItems.length;
      renderCmdMenu();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      closeCmdMenu();
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      // A BARE `/` only opens the list — running the top item there would start a
      // fresh session off one keystroke. The query must name something first.
      if (cmdQuery() !== null && cmdQuery() !== "") runMenuCommand(cmdItems[cmdIndex]);
      return;
    }
  }
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    submit();
  } else if (event.key === "Escape") {
    event.preventDefault();
    post({ kind: "cancel" });
  }
});
sendBtn.addEventListener("click", submit);
// The mode control: post and let the HOST flip plan-mode; the next snapshot
// reflects it (no optimistic flip here).
cmodeEl.addEventListener("click", () => post({ kind: "toggle-plan" }));
// Re-attach the same selection the chip was dismissed from.
cattachEl.addEventListener("click", () => {
  attached = true;
  rerender();
});
mAllEl.addEventListener("click", () => post({ kind: "set-mode", mode: "all" }));
mFocusEl.addEventListener("click", () => post({ kind: "set-mode", mode: "focus" }));
cancelBtn.addEventListener("click", () => {
  post({ kind: "cancel" });
});

/* ------------------------------------------------------------------ host io */

window.addEventListener("message", (event: MessageEvent<unknown>) => {
  const message = parseToWebview(event.data);
  if (message === null) return;
  lastState = message;
  render(message);
});

// The handshake: the host holds its snapshot until we say we are listening.
post({ kind: "ready" });
inputEl.focus();
