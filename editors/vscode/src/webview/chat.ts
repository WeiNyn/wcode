/**
 * The webview panel.
 *
 * Typed, dependency-free (esbuild wraps it as an IIFE). The host renders
 * markdown and posts pre-rendered HTML (`ToWebview`); this is a pure renderer of
 * that snapshot — it never parses markdown, never loads a remote resource, and
 * only talks to the host over `postMessage`.
 *
 * The PURE half lives in `./view.ts` (`stateLabel`, `panelHeader`, `turns`,
 * `diffStat`, `emptySpec`, `emptyKind`, `toggleExpanded`, `classNames`) and
 * `../webview.ts`
 * (`parseToWebview`), both of which plain node can drive. What remains here is
 * DOM-bound: element construction, scroll glue, the click/keydown handlers, and
 * `acquireVsCodeApi()`.
 */
import type { RenderedBlock, RenderedState, RenderedTool } from "../render.ts";
import { parseToWebview, type FromWebview, type PanelSessionInfo, type SelectionContext, type ToWebview } from "../webview.ts";
import {
  classNames,
  composerControls,
  composeSubmit,
  diffStat,
  emptyKind,
  emptySpec,
  panelHeader,
  selectionRef,
  toggleExpanded,
  turns,
  type Turn,
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
  '<main id="transcript" class="transcript" role="log" aria-label="Transcript"></main>',
  '<footer id="composer" class="composer">',
  '  <div id="ctx-chips" class="ctx-chips" aria-label="Attached context"></div>',
  '  <textarea id="input" rows="1" spellcheck="false"',
  '    placeholder="Message wcode…  (Enter to send, Shift+Enter for newline, Esc to cancel)"></textarea>',
  '  <div class="ctoolbar">',
  '    <button id="cattach" class="tool-btn" type="button" title="Attach the current editor selection">@ selection</button>',
  '    <button id="cmode" class="tool-btn mode" type="button"></button>',
  '    <span id="cmodel" class="tool-btn model"></span>',
  '    <span class="spacer"></span>',
  '    <span id="hint" class="hint">Enter to send</span>',
  '    <button id="send" class="btn primary">Send</button>',
  '    <button id="cancel" class="btn" title="Cancel the in-flight run (Esc)">Stop</button>',
  "  </div>",
  "</footer>",
].join("\n");
const pheadEl = requireEl("phead");

const transcriptEl = requireEl("transcript");
const inputEl = requireEl("input") as HTMLTextAreaElement;
const sendBtn = requireEl("send");
const cancelBtn = requireEl("cancel");
const cmodeEl = requireEl("cmode");
const cmodelEl = requireEl("cmodel");
const cattachEl = requireEl("cattach");
const ctxChipsEl = requireEl("ctx-chips");

/** callIds whose tool output is expanded (survives a re-render). */
let expanded: ReadonlySet<string> = new Set();
/** The last snapshot, so expand/collapse can re-render without the host. */
let lastState: ToWebview | null = null;

/** Does a context chip ride into the next submit? (webview-local, like `expanded`). */
let attached = true;
/** The selection from the LAST snapshot (for `composeSubmit` at submit time). */
let lastContext: SelectionContext | null = null;
/** The last rendered selection ref, so a NEW selection re-arms the chip. */
let lastRef: string | null = null;

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
  const { r1, r2 } = panelHeader(state, session);
  const row1 = el("div", "r1");
  const row2 = el("div", "r2");
  // Every cell is a plain span: the target chip is INERT here (retarget is P3).
  for (const cell of r1) row1.appendChild(el("span", cell.className, cell.text));
  for (const cell of r2) row2.appendChild(el("span", cell.className, cell.text));
  pheadEl.textContent = "";
  pheadEl.append(row1, row2);
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
  // line either — the `.who` line is per-TURN now.
  const node = el("div", classNames(block));
  node.innerHTML = block.html; // host-rendered; markdown-it `html: false` escaped it
  return node;
}

function renderToolFold(block: RenderedBlock): HTMLElement {
  const tool = block.tool;
  if (tool === undefined) return el("details", "fold tool");
  const isOpen = expanded.has(tool.callId);

  const details = el("details", classNames(block, isOpen));
  if (isOpen) details.setAttribute("open", "");

  const summary = el("summary", null);
  summary.appendChild(el("span", "chev"));
  summary.appendChild(el("span", "tname", `⚙ ${tool.name}`));
  summary.appendChild(el("span", "tsum", tool.summary));
  const meta = toolMeta(tool);
  if (meta !== null) summary.appendChild(meta);
  // We drive the fold from the `expanded` set (so it survives a re-render):
  // suppress the native toggle, flip the set, repaint.
  summary.addEventListener("click", (event) => {
    event.preventDefault();
    expanded = toggleExpanded(expanded, tool.callId);
    rerender();
  });
  details.appendChild(summary);

  const inner = el("div", "inner");
  inner.innerHTML = tool.outputHtml; // P4: the diff `.code` preview
  details.appendChild(inner);
  return details;
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
  if (typeof tool.durationMs === "number") {
    if (hasStat) meta.appendChild(document.createTextNode(" · "));
    meta.appendChild(document.createTextNode(`${tool.durationMs}ms`));
  }
  return meta.childNodes.length > 0 ? meta : null;
}

function renderTurn(turn: Turn): HTMLElement {
  const wrap = el("div", `turn ${turn.role}`);
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

function render(snapshot: ToWebview): void {
  const { state, session } = snapshot;
  const stick = nearBottom();
  renderHeader(state, session);
  renderComposer(state, snapshot.context);
  transcriptEl.textContent = "";
  if (state.blocks.length === 0) {
    transcriptEl.appendChild(renderStateCard(state, session));
  } else {
    for (const turn of turns(state.blocks)) transcriptEl.appendChild(renderTurn(turn));
  }
  if (stick) transcriptEl.scrollTop = transcriptEl.scrollHeight;
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
  post({ kind: "submit", text: composeSubmit(text, attached ? lastContext : null) });
  inputEl.value = "";
  autoGrow();
  inputEl.focus();
}

inputEl.addEventListener("input", autoGrow);
inputEl.addEventListener("keydown", (event: KeyboardEvent) => {
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
