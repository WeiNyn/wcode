/**
 * The webview panel.
 *
 * Typed, dependency-free (esbuild wraps it as an IIFE). The host renders
 * markdown and posts pre-rendered HTML (`ToWebview`); this is a pure renderer of
 * that snapshot — it never parses markdown, never loads a remote resource, and
 * only talks to the host over `postMessage`.
 *
 * The PURE half lives in `./view.ts` (`stateLabel`, `statusSegments`,
 * `emptyKind`, `toggleExpanded`, `classNames`) and `../webview.ts`
 * (`parseToWebview`), both of which plain node can drive. What remains here is
 * DOM-bound: element construction, scroll glue, the click/keydown handlers, and
 * `acquireVsCodeApi()`.
 */
import type { RenderedBlock, RenderedState } from "../render.ts";
import { parseToWebview, type FromWebview, type PanelSessionInfo, type ToWebview } from "../webview.ts";
import { classNames, emptyKind, statusSegments, toggleExpanded } from "./view.ts";

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
  '<header id="status" class="status"></header>',
  '<main id="transcript" class="transcript"></main>',
  '<footer id="composer" class="composer">',
  '  <textarea id="input" rows="1" spellcheck="false"',
  '    placeholder="Message wcode…  (Enter to send, Shift+Enter for newline, Esc to cancel)"></textarea>',
  '  <div class="composer-actions">',
  '    <button id="send" class="btn primary">Send</button>',
  '    <button id="cancel" class="btn" title="Cancel the in-flight run (Esc)">Cancel</button>',
  '    <span id="hint" class="hint">Enter to send · Shift+Enter newline · Esc cancel</span>',
  "  </div>",
  "</footer>",
].join("\n");

const statusEl = requireEl("status");
const transcriptEl = requireEl("transcript");
const inputEl = requireEl("input") as HTMLTextAreaElement;
const sendBtn = requireEl("send");
const cancelBtn = requireEl("cancel");

/** callIds whose tool output is expanded (survives a re-render). */
let expanded: ReadonlySet<string> = new Set();
/** The last snapshot, so expand/collapse can re-render without the host. */
let lastState: ToWebview | null = null;

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

/* ------------------------------------------------------------------- status */

function renderStatus(state: RenderedState, session: PanelSessionInfo): void {
  statusEl.textContent = "";
  for (const segment of statusSegments(state, session)) {
    statusEl.appendChild(el("span", segment.className, segment.text));
  }
  if (session.state === "crashed" && session.stderrTail !== "") {
    const details = el("details", "stderr-block");
    details.appendChild(el("summary", null, "wcode stderr (tail)"));
    details.appendChild(el("pre", "stderr", session.stderrTail));
    statusEl.appendChild(details);
  }
}

/* ------------------------------------------------------------------- blocks */

function renderEmpty(state: RenderedState, session: PanelSessionInfo): HTMLElement {
  const wrap = el("div", "empty");
  switch (emptyKind(state, session)) {
    case "crashed":
      wrap.appendChild(el("p", "empty-title", "wcode crashed."));
      wrap.appendChild(el("p", "empty-sub", "Run “wcode: Restart” to try again."));
      break;
    case "starting":
      wrap.appendChild(el("p", "empty-title", "Starting wcode…"));
      break;
    case "stopped":
      wrap.appendChild(el("p", "empty-title", "wcode is stopped."));
      wrap.appendChild(el("p", "empty-sub", "Run “wcode: Start Session”."));
      break;
    case "working":
      wrap.appendChild(el("p", "empty-title", "Working…"));
      break;
    default:
      wrap.appendChild(el("p", "empty-title", "No messages yet."));
      wrap.appendChild(el("p", "empty-sub", "Type below and press Enter."));
  }
  return wrap;
}

function blockShell(block: RenderedBlock, role: string | null): HTMLElement {
  const wrap = el("div", classNames(block));
  if (role !== null) wrap.appendChild(el("div", "role", role));
  const body = el("div", "body");
  body.innerHTML = block.html; // host-rendered; markdown-it `html: false` escaped it
  wrap.appendChild(body);
  return wrap;
}

function renderTool(block: RenderedBlock): HTMLElement {
  const tool = block.tool;
  if (tool === undefined) return el("div", "block tool");
  const isOpen = expanded.has(tool.callId);
  const wrap = el("div", classNames(block, isOpen));

  const head = el("div", "tool-head");
  const toggle = el("button", "tool-toggle");
  toggle.appendChild(el("span", "chev", isOpen ? "▾" : "▸"));
  toggle.appendChild(el("span", "tool-name", `⚙ ${tool.name}`));
  toggle.appendChild(el("span", "tool-summary", tool.summary));
  if (typeof tool.durationMs === "number") {
    toggle.appendChild(el("span", "tool-dur", `${tool.durationMs}ms`));
  }
  toggle.addEventListener("click", () => {
    expanded = toggleExpanded(expanded, tool.callId);
    rerender();
  });
  head.appendChild(toggle);

  // P2: enabled iff there is a diff. A `path` with NO diff (a write that changed
  // no line) must not offer a button that opens nothing.
  if (tool.hasDiff) {
    const diff = el("button", "tool-diff", "show diff");
    diff.addEventListener("click", () => {
      post({ kind: "open-diff", callId: tool.callId });
    });
    head.appendChild(diff);
  }
  wrap.appendChild(head);

  if (isOpen) {
    const body = el("div", "tool-body");
    body.innerHTML = tool.outputHtml;
    wrap.appendChild(body);
  }
  return wrap;
}

function renderBlock(block: RenderedBlock): HTMLElement {
  switch (block.kind) {
    case "user":
      return blockShell(block, "you");
    case "assistant":
      return blockShell(block, "wcode");
    case "error":
      return blockShell(block, "error");
    case "btw":
      return blockShell(block, "btw");
    case "tool":
      return renderTool(block);
    default:
      // A notice carries its sender when it is inter-agent traffic
      // (`message_received`): without this the block has no role line and the
      // reader cannot tell who spoke.
      return blockShell(block, block.from ?? null);
  }
}

function render(snapshot: ToWebview): void {
  const { state, session } = snapshot;
  const stick = nearBottom();
  renderStatus(state, session);
  transcriptEl.textContent = "";
  if (state.blocks.length === 0) {
    transcriptEl.appendChild(renderEmpty(state, session));
  } else {
    for (const block of state.blocks) transcriptEl.appendChild(renderBlock(block));
  }
  if (stick) transcriptEl.scrollTop = transcriptEl.scrollHeight;
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
  post({ kind: "submit", text });
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
