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
import { completionText, filterCommands, parseSlash, type SlashCommand } from "../commands.ts";
import { memberGlyph, memberViews } from "../reducer.ts";
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
  isActivationKey,
  livelineLabel,
  livelineSpec,
  panelHeader,
  toolStatus,
  teamCaption,
  selectionRef,
  toggleFold,
  turnNotes,
  turns,
  workingGroup,
  type FoldOverrides,
  type Turn,
  type TurnMember,
  type TurnNote,
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
  '<header id="masthead" class="masthead" role="banner">',
  '  <div class="inner">',
  '    <div class="row1">',
  '      <span id="ident" class="ident"></span>',
  '      <span class="spacer"></span>',
  '      <span id="status" class="status"></span>',
  '      <span id="folio" class="folio"></span>',
  '    </div>',
  '    <div id="team" class="team" role="list" aria-label="Team"></div>',
  '  </div>',
  '</header>',
  '<section class="main">',
  '  <main id="transcript" class="stream" role="log" aria-label="Transcript"></main>',
  '  <div id="liveline" class="liveline" role="status" hidden></div>',
  '  <footer id="composer" class="composer">',
  '    <div id="ctx-chips" class="ctx-chips" aria-label="Attached context"></div>',
  '    <div id="cmdmenu" class="cmdmenu" role="listbox" aria-label="Commands" hidden></div>',
  '    <div class="inner">',
  '      <div class="inputline">',
  '        <span class="pfx" aria-hidden="true">❯</span>',
  '        <textarea id="input" rows="1" spellcheck="false"',
  '          placeholder="Message wcode…" title="Enter to send · Shift+Enter for a newline · Esc to cancel"></textarea>',
  '      </div>',
  '      <div class="foot">',
  '        <button id="cattach" class="linkbtn" type="button" title="Attach the current editor selection">Attach context</button>',
  '        <span aria-hidden="true">·</span>',
  '        <button id="mAct" class="linkbtn" type="button" aria-pressed="true">Act</button>',
  '        <button id="mPlan" class="linkbtn" type="button" aria-pressed="false">Plan</button>',
  '        <button id="send" class="linkbtn send" type="button">Send</button>',
  '        <button id="cancel" class="linkbtn send" type="button" title="Cancel the in-flight run (Esc)" hidden>Stop</button>',
  '      </div>',
  '    </div>',
  '  </footer>',
  '</section>',
].join("\n");
const identEl = requireEl("ident");
const statusEl = requireEl("status");
const folioEl = requireEl("folio");
const teamEl = requireEl("team");

const transcriptEl = requireEl("transcript");
const livelineEl = requireEl("liveline");
const inputEl = requireEl("input") as HTMLTextAreaElement;
const sendBtn = requireEl("send");
const cancelBtn = requireEl("cancel");
const mActEl = requireEl("mAct");
const mPlanEl = requireEl("mPlan");
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
/** The composer's current mode (the mode toggle's pressed segment), for the click guard. */
let composerMode: "Plan" | "Act" = "Act";

/** The masthead's cells+caption signature; a matching snapshot skips the rebuild. */
let mastheadKey: string | null = null;

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

/* ------------------------------------------------------------------ masthead */

/** Paint the ONE-row masthead + the team caption. Impure (DOM). */
function renderMasthead(state: RenderedState, session: PanelSessionInfo, mode: ViewMode): void {
  const key = mastheadSignature(state, session, mode);
  if (key === mastheadKey) return; // same cells + caption: keep the SAME DOM
  mastheadKey = key;
  const { cells } = panelHeader(state, session, mode);
  identEl.textContent = "";
  statusEl.textContent = "";
  folioEl.textContent = "";
  for (const cell of cells) {
    if (cell.kind === "ident") {
      const glyph = el("span", cell.glyphClass, cell.glyph);
      glyph.title = cell.stateTitle;
      // `done`/`failed` read oddly as a SESSION state, so label it just `State: …`.
      glyph.setAttribute("aria-label", `State: ${cell.stateTitle}`);
      identEl.appendChild(glyph);
      identEl.appendChild(el("span", "name", cell.name));
    } else if (cell.kind === "meter") {
      if (cell.word !== "") statusEl.appendChild(el("span", "word", cell.word));
      if (cell.text !== "") statusEl.appendChild(el("span", "mono", cell.text));
    } else if (cell.kind === "mode") {
      statusEl.appendChild(modeToggle(cell.mode));
    } else if (cell.kind === "folio") {
      folioEl.textContent = String(cell.steps);
    }
    // a spacer cell needs no DOM: the skeleton's `.spacer` holds the row's gap
  }
  renderCaption(state);
}

/** The masthead's cells + caption signature; a matching snapshot keeps the SAME DOM. */
function mastheadSignature(state: RenderedState, session: PanelSessionInfo, mode: ViewMode): string {
  const { cells } = panelHeader(state, session, mode);
  const head = cells.map((cell) => JSON.stringify(cell)).join("\u0002");
  const caption = JSON.stringify(teamCaption(memberViews(state.members), state.target?.id ?? null));
  return `${head}\u0002${caption}`;
}

/** The quiet All/Focus text toggle (A1) — the one mode control, in the `.status` line. */
function modeToggle(current: ViewMode): HTMLElement {
  const wrap = el("span", "modes");
  wrap.setAttribute("role", "group");
  wrap.setAttribute("aria-label", "View mode");
  for (const mode of ["all", "focus"] as const) {
    const button = el("button", "linkbtn", mode === "all" ? "All" : "Focus");
    button.setAttribute("type", "button");
    button.dataset.el = `mode-${mode}`;
    button.setAttribute("aria-pressed", current === mode ? "true" : "false");
    button.addEventListener("click", () => post({ kind: "set-mode", mode }));
    wrap.appendChild(button);
  }
  return wrap;
}

/** Paint the dim team caption (`#team`). Impure (DOM). */
function renderCaption(state: RenderedState): void {
  const rows = teamCaption(memberViews(state.members), state.target?.id ?? null);
  teamEl.textContent = "";
  for (const row of rows) {
    const node = el("span", row.className);
    node.setAttribute("role", "listitem");
    // The pair is the retarget affordance, so it carries a KEY, not just a click.
    node.tabIndex = 0;
    node.title = `${row.name} · ${row.state}`;
    node.setAttribute("aria-label", `${row.name}, ${row.state}`);
    const glyph = el("span", `glyph ${row.glyphClass}`, row.glyph);
    glyph.setAttribute("aria-hidden", "true");
    node.appendChild(glyph);
    node.appendChild(el("span", "nm", row.name));
    node.addEventListener("click", () => post({ kind: "focus-member", id: row.id }));
    node.addEventListener("keydown", (event) => {
      if (!isActivationKey(event.key)) return;
      event.preventDefault();
      post({ kind: "focus-member", id: row.id });
    });
    teamEl.appendChild(node);
  }
}

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
  // line either — the `.byline` line is per-TURN now.
  const node = el("div", classNames(block));
  node.innerHTML = block.html; // host-rendered; markdown-it `html: false` escaped it
  return node;
}

/** The bare output recess (fill-only; NO card head, NO `Copy`). Impure (DOM). */
function rawOutput(tool: RenderedTool): HTMLElement {
  const pre = el("pre", "quote");
  // W009: the host pre-highlights; Prism escapes its own output, so it is safe as HTML.
  if (tool.outputHtml !== undefined) pre.innerHTML = tool.outputHtml;
  else pre.textContent = tool.outputText;
  return pre;
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
  for (const [i, line] of hunk.lines.entries()) {
    const row = el("div", `ln ${line.kind}`);
    row.appendChild(el("span", "gutter", String(line.number)));
    const txt = el("span", "txt");
    // W009: the host pre-highlights each line (well-formed spans); else the plain text. NOTE:
    // `el(tag, class, text)` sets `textContent`, so the html MUST go through `innerHTML`.
    const html = tool.diffLinesHtml?.[i];
    if (html !== undefined) txt.innerHTML = html;
    else txt.textContent = line.text;
    row.appendChild(txt);
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

/** The tool head's meta slot: `⠋ running…` / `✗ failed` (V13), or the `+N −M · 38ms`
 *  stat for a COMPLETE tool ("the exception is loud, the norm is quiet"), or null. */
function toolMeta(tool: RenderedTool): HTMLElement | null {
  const meta = el("span", "tmeta");
  const status = toolStatus(tool);
  if (status.className !== null) {
    // RUNNING / ERROR (V13): a status glyph + word in the state's own colour, and NO stat
    // — the colour and the glyph ARE the message, and a failure's numbers would be noise.
    const span = el("span", status.className);
    if (status.className === "run") {
      // The running glyph SPINS (the surface's first animation, reduced-motion gated).
      const glyph = el("span", "spin", status.glyph);
      glyph.setAttribute("aria-hidden", "true");
      span.appendChild(glyph);
      span.appendChild(document.createTextNode(` ${status.word}`));
    } else {
      span.appendChild(document.createTextNode(`${status.glyph} ${status.word}`));
    }
    meta.appendChild(span);
    return meta;
  }
  // COMPLETE: the `+N −M · 38ms` stat, dim, NO glyph (the norm is quiet).
  const stat = typeof tool.diff === "string" && tool.diff !== "" ? diffStat(tool.diff) : null;
  const hasStat = stat !== null && (stat.added > 0 || stat.removed > 0);
  if (stat !== null && stat.added > 0) meta.appendChild(el("span", "add", `+${stat.added}`));
  if (stat !== null && stat.removed > 0) {
    if (stat.added > 0) meta.appendChild(document.createTextNode(" "));
    meta.appendChild(el("span", "del", `−${stat.removed}`));
  }
  if (typeof tool.durationMs === "number") {
    if (hasStat) meta.appendChild(document.createTextNode(" · "));
    meta.appendChild(document.createTextNode(`${tool.durationMs}ms`));
  }
  return meta.childNodes.length > 0 ? meta : null;
}

function renderTurn(turn: Turn): HTMLElement {
  // `.turn.new` marks the turn that OPENS a speaker (the role-aware rhythm).
  const wrap = el("div", `turn ${turn.role}${turn.isNew ? " new" : ""}`);
  const byline = el("div", turn.who.className);
  if (turn.who.glyph !== "") byline.appendChild(el("span", "glyph", turn.who.glyph));
  byline.appendChild(el("span", "name", turn.who.name)); // no `.stamp` — no time on the wire
  wrap.appendChild(byline);
  // The tool blocks LEAVE the flow (A2): each prints a reference at its own boundary and is
  // collected; the apparatus is then set ONCE, at the turn's foot, after ALL blocks.
  const notes = turnNotes(turn.blocks);
  let i = 0;
  for (const block of turn.blocks) {
    if (block.kind === "tool" && i < notes.length) {
      wrap.appendChild(fnmark(notes[i]));
      i += 1;
    } else {
      wrap.appendChild(blockShell(block));
    }
  }
  if (notes.length > 0) wrap.appendChild(renderFootnotes(notes));
  return wrap;
}

/** The printed reference (`.fnmark`) for ONE note — a real button, so Enter/Space are free. */
function fnmark(note: TurnNote): HTMLElement {
  const button = el("button", "fnmark", String(note.n));
  button.setAttribute("type", "button");
  button.id = `fnm-${note.n}`;
  button.setAttribute("aria-controls", `fn-${note.n}`);
  button.setAttribute("aria-expanded", foldOpen(foldOverrides, note.callId, note.done) ? "true" : "false");
  button.title = `Note ${note.n}, ${note.name}`;
  button.addEventListener("click", () => {
    foldOverrides = toggleFold(foldOverrides, note.callId, note.done);
    rerender();
  });
  return button;
}

/** The turn's footnote apparatus (`.footnotes` `<ol>`), one `<li>` per note. */
function renderFootnotes(notes: TurnNote[]): HTMLElement {
  const list = el("ol", "footnotes");
  list.setAttribute("aria-label", "Notes");
  for (const note of notes) list.appendChild(fnNote(note));
  return list;
}

/** ONE footnote `<li>`: the head (reusing `toolMeta`) + the note's `details.fn-out`. */
function fnNote(note: TurnNote): HTMLElement {
  const item = el("li", null);
  item.id = `fn-${note.n}`;
  const head = el("div", "fn-head");
  const n = el("span", "n", String(note.n));
  n.setAttribute("aria-hidden", "true");
  head.appendChild(n);
  head.appendChild(el("span", "mark", `⚙ ${note.name}`));
  if (note.target !== "") head.appendChild(el("span", "to", note.target));
  const meta = toolMeta(note.tool);
  if (meta !== null) head.appendChild(meta);
  item.appendChild(head);

  // The note's `open` is driven by the SHARED override map, so it survives `render`'s
  // `textContent = ""` rebuild (a native `details` open would be lost on every tick).
  const details = el("details", "fn-out");
  details.id = `fn-${note.n}-out`;
  if (foldOpen(foldOverrides, note.callId, note.done)) details.setAttribute("open", "");
  const summary = el("summary", null);
  summary.appendChild(el("span", "chev"));
  summary.appendChild(document.createTextNode(" output"));
  summary.addEventListener("click", (event) => {
    event.preventDefault(); // suppress the native toggle; the override drives it
    foldOverrides = toggleFold(foldOverrides, note.callId, note.done);
    rerender();
  });
  details.appendChild(summary);
  const inner = el("div", "inner");
  const hunk = note.hasDiff ? reviewHunk(note.tool.diff ?? "") : null;
  inner.appendChild(hunk !== null ? reviewBlock(note.tool, hunk, verdictOf(lastVerdicts, note.callId)) : rawOutput(note.tool));
  details.appendChild(inner);
  item.appendChild(details);
  return item;
}

/* ------------------------------------------------------- the one live line */

/** The one live type-line (draft `.liveline`): `{n} working` iff count > 0. Impure (DOM). */
function renderLiveline(state: RenderedState): void {
  const spec = livelineSpec(workingGroup(state.members, state.target?.id ?? null));
  if (spec.kind === "none") {
    livelineEl.hidden = true;
    livelineEl.textContent = "";
    return;
  }
  livelineEl.hidden = false;
  livelineEl.textContent = "";
  // exactly ONE active member: the glyph + its name + what it is doing.
  if (spec.kind === "one") {
    const line = el("span", "row live-one");
    const glyph = el("span", `glyph ${spec.glyphClass}${spec.running ? " spin" : ""}`, spec.glyph);
    glyph.setAttribute("aria-hidden", "true");
    line.appendChild(glyph);
    line.appendChild(el("span", "who", spec.name));
    if (spec.action !== "") {
      const dot = el("span", null, "·");
      dot.setAttribute("aria-hidden", "true");
      line.appendChild(dot);
      line.appendChild(el("span", "what", spec.action));
    }
    livelineEl.appendChild(line);
    livelineEl.setAttribute("aria-label", spec.action === "" ? spec.name : `${spec.name} · ${spec.action}`);
    return;
  }
  // several: the aggregate count.
  const line = el("span", "row live-count");
  const dots = el("span", "dots", "⋯");
  dots.setAttribute("aria-hidden", "true");
  line.appendChild(dots);
  const label = el("span", "lbl");
  label.appendChild(el("b", null, String(spec.count)));
  label.appendChild(document.createTextNode(" working"));
  line.appendChild(label);
  livelineEl.appendChild(line);
  livelineEl.setAttribute("aria-label", livelineLabel(spec.count));
}

function render(snapshot: ToWebview): void {
  const { state, session } = snapshot;
  const stick = nearBottom();
  renderMasthead(state, session, snapshot.mode);
  renderComposer(state, snapshot.context);
  renderLiveline(state);
  lastVerdicts = snapshot.verdicts;
  transcriptEl.textContent = "";
  if (state.blocks.length === 0) {
    transcriptEl.appendChild(renderStateCard(state, session));
  } else {
    const col = el("div", "col");
    const member = snapshot.mode === "all" ? memberOf(state) : undefined;
    for (const turn of turns(state.blocks, member)) col.appendChild(renderTurn(turn));
    transcriptEl.appendChild(col);
  }
  if (stick) transcriptEl.scrollTop = transcriptEl.scrollHeight;
}

/** Resolve a block's `origin` (or a peer's `from`) to the member that labels its turn. */
function memberOf(state: RenderedState): (origin: string | undefined) => TurnMember | undefined {
  return (origin) => {
    if (origin === undefined) return undefined;
    const member = state.members.find((m) => m.id === origin);
    if (member === undefined) {
      // Not in the roster (a peer that has since left): name it by its id, idle glyph.
      const name = origin.startsWith("agent:") ? origin.slice("agent:".length) : origin;
      return { name, isRoot: false, glyph: memberGlyph("idle").glyph };
    }
    return {
      name: member.isRoot ? "orchestrator" : member.label,
      isRoot: member.isRoot,
      glyph: memberGlyph(member.state).glyph,
    };
  };
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

  const controls = composerControls(state);
  composerMode = controls.mode;
  // The mode control is the quiet text toggle (`.linkbtn`), the same in the masthead.
  for (const segment of controls.segments) {
    (segment.mode === "Act" ? mActEl : mPlanEl).setAttribute("aria-pressed", segment.pressed ? "true" : "false");
  }
  // `Stop` is conditional chrome: visible ONLY while a run is in flight (Esc still cancels).
  cancelBtn.hidden = !controls.stop;
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

/** Flip plan-mode to `mode` unless it is already active (the wire op is a bare toggle). */
function selectMode(mode: "Plan" | "Act"): void {
  if (mode !== composerMode) post({ kind: "toggle-plan" });
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
 * Run a command picked from the menu (the ENTER half; Tab is `completeMenuCommand`). A
 * command that NEEDS an argument (`/btw`, the only one left) only arms the input — running
 * it empty would be a round-trip the host must reject.
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

/**
 * Complete the highlighted `/` command into the composer (Tab) — the COMPLEMENT of
 * `runMenuCommand`: Tab ARMS the line and keeps focus; it NEVER dispatches a `post`.
 * `renderCmdMenu()` then hides the menu for `/<name> `, because `cmdQuery()` returns null
 * once a space is typed — the intended behaviour.
 */
function completeMenuCommand(command: SlashCommand): void {
  inputEl.value = completionText(command);
  autoGrow();
  renderCmdMenu();
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
    if (event.key === "Tab") {
      event.preventDefault(); // keep Tab away from focus traversal
      completeMenuCommand(cmdItems[cmdIndex]);
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
// The mode toggle: post and let the HOST flip plan-mode; the next snapshot reflects it
// (no optimistic flip). The wire only carries `toggle-plan`, so a click on the segment
// that is ALREADY active is a no-op — otherwise it would flip the mode the wrong way.
mActEl.addEventListener("click", () => selectMode("Act"));
mPlanEl.addEventListener("click", () => selectMode("Plan"));
// Re-attach the same selection the chip was dismissed from.
cattachEl.addEventListener("click", () => {
  attached = true;
  rerender();
});
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
