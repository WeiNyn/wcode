import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { initialState, memberGlyph, reduce, type RosterItem, type SessionMember, type ViewState } from "../src/reducer.ts";
import { renderState, type RenderedBlock, type RenderedState } from "../src/render.ts";
import { parseToWebview, type PanelSessionInfo, type ToWebview, type ViewMode } from "../src/webview.ts";
import {
  classNames,
  bodyKind,
  toolState,
  toolStatus,
  composerControls,
  composeSubmit,
  diffStat,
  emptyKind,
  emptySpec,
  foldOpen,
  formatTokens,
  livelineLabel,
  panelHeader,
  isActivationKey,
  teamCaption,
  selectionRef,
  stateLabel,
  toggleFold,
  turns,
  workingGroup,
  memberInitial,
  memberSwatch,
  MEMBER_SWATCHES,
  type FoldOverride,
  type IdentCell,
  type MeterCell,
} from "../src/webview/view.ts";

const here = dirname(fileURLToPath(import.meta.url));

const idle = (): RenderedState => renderState(initialState());

const session = (over: Partial<PanelSessionInfo> = {}): PanelSessionInfo => ({
  id: "root-1",
  state: "ready",
  stderrTail: "",
  ...over,
});

/* ------------------------------------------------------------------- header */

test("stateLabel names every FSM state", () => {
  assert.equal(stateLabel("ready"), "ready");
  assert.equal(stateLabel("starting"), "starting");
  assert.equal(stateLabel("crashed"), "crashed");
  assert.equal(stateLabel("stopped"), "stopped");
});

/** A ready session rendering ONE root member of `memberState`. */
function headerState(memberState: SessionMember["state"], extra: Partial<ViewState> = {}): RenderedState {
  const state: ViewState = {
    ...initialState(),
    members: [{ id: "root-1", label: "root-1", state: memberState, isRoot: true, model: "sonnet" }],
    targeted: "root-1",
    ...extra,
  };
  return renderState(state, "root-1");
}

/** The identity cell of the ONE-row masthead (asserts cell 1 IS the identity). */
function identOf(state: RenderedState, session: PanelSessionInfo, mode: ViewMode = "focus"): IdentCell {
  const cell = panelHeader(state, session, mode).cells[0];
  assert.equal(cell?.kind, "ident");
  return cell as IdentCell;
}

/** The `.status` (meter) cell of the masthead — it carries the state WORD + the `ctx N`. */
function meterOf(state: RenderedState, session: PanelSessionInfo, mode: ViewMode = "focus"): MeterCell {
  const cell = panelHeader(state, session, mode).cells.find((c) => c.kind === "meter");
  assert.equal(cell?.kind, "meter");
  return cell as MeterCell;
}

test("panelHeader: ONE row — identity, spacer, ctx meter, the mode toggle (no chip/seg/more)", () => {
  const state = headerState("idle", { status: { running: false, planMode: false, contextUsed: 42_000 } });
  assert.deepEqual(
    panelHeader(state, session(), "focus").cells.map((c) => c.kind),
    ["ident", "spacer", "meter", "mode"],
  );
  // No `contextUsed` ⇒ the meter cell is still there, its text empty (the word lives on it).
  const bare = panelHeader(headerState("idle"), session(), "focus").cells;
  assert.deepEqual(
    bare.map((c) => c.kind),
    ["ident", "spacer", "meter", "mode"],
  );
  assert.equal(meterOf(headerState("idle"), session()).text, "", "no count ⇒ no `ctx N` text");
});

test("formatTokens: a k/M suffix, `<1000` verbatim", () => {
  assert.equal(formatTokens(42), "42");
  assert.equal(formatTokens(999), "999");
  assert.equal(formatTokens(1_500), "1.5k");
  assert.equal(formatTokens(42_000), "42k");
  assert.equal(formatTokens(2_000_000), "2M");
  assert.equal(formatTokens(20_000_000), "20M");
});

test("panelHeader: the meter is `ctx 42k` WITHOUT a window — no gauge, no denominator", () => {
  const { cells } = panelHeader(
    headerState("idle", { status: { running: false, planMode: false, contextUsed: 42_000 } }),
    session(),
    "focus",
  );
  const meter = cells.find((c) => c.kind === "meter");
  assert.equal(meter?.kind, "meter", "a meter cell is present");
  assert.equal(meter?.kind === "meter" ? meter.text : "", "ctx 42k");
  for (const cell of cells) {
    const text = cell.kind === "meter" ? cell.text : "";
    assert.ok(!text.includes("▰"), `no gauge glyph in "${text}"`);
    assert.ok(!text.includes("/"), `no denominator in "${text}"`);
  }
});

test("reducer: a `sessions` push threads effort + context_window onto the member", () => {
  const s = reduce(initialState(), {
    type: "sessions",
    sessions: [{ id: "root", model: "m1", effort: "high", context_window: 200_000, state: "idle" }],
  });
  assert.equal(s.members[0].effort, "high");
  assert.equal(s.members[0].contextWindow, 200_000);
  // Absent on the wire ⇒ undefined (older peer / unset).
  const bare = reduce(initialState(), { type: "sessions", sessions: [{ id: "w1", state: "idle" }] });
  assert.equal(bare.members[0].effort, undefined);
  assert.equal(bare.members[0].contextWindow, undefined);
});

test("reducer: a `models` event fills the connection catalog", () => {
  const s = reduce(initialState(), { type: "models", models: ["m1", "m2"] });
  assert.deepEqual(s.models, ["m1", "m2"]);
});

test("panelHeader: the identity glyph — the member's liveness for `ready`, the FSM for terminal states", () => {
  assert.equal(identOf(headerState("idle"), session()).glyph, "○");
  assert.equal(identOf(headerState("running"), session()).glyph, "⠋");
  assert.equal(identOf(headerState("done"), session()).glyph, "✓");
  assert.equal(identOf(headerState("failed"), session()).glyph, "✗");
  // The running glyph spins; a healthy session carries NO word.
  assert.equal(identOf(headerState("running"), session()).glyphClass, "glyph g-run spin");
  assert.equal(meterOf(headerState("idle"), session()).word, "");
});

test("panelHeader: a crashed session reads `✗` + the word 'crashed' (never a spinner)", () => {
  const crashed = identOf(headerState("idle"), session({ state: "crashed" }));
  assert.equal(crashed.glyph, "✗");
  assert.equal(crashed.glyphClass, "glyph g-err");
  assert.equal(crashed.stateTitle, "crashed");
  assert.equal(meterOf(headerState("idle"), session({ state: "crashed" })).word, "crashed");
});

test("panelHeader: a `ready` session's glyph title reflects the MEMBER's liveness", () => {
  assert.equal(identOf(headerState("idle"), session()).stateTitle, "idle");
  assert.equal(identOf(headerState("running"), session()).stateTitle, "running", "idle vs running must differ");
  // The terminal states keep the session vocabulary.
  assert.equal(identOf(headerState("idle"), session({ state: "crashed" })).stateTitle, "crashed");
  assert.equal(identOf(headerState("idle"), session({ state: "stopped" })).stateTitle, "stopped");
});

test("panelHeader: starting spins `⠋`, stopped is `○`; both carry their word", () => {
  const starting = identOf(headerState("idle"), session({ state: "starting" }));
  assert.equal(starting.glyph, "⠋");
  assert.equal(meterOf(headerState("idle"), session({ state: "starting" })).word, "starting");
  const stopped = identOf(headerState("idle"), session({ state: "stopped" }));
  assert.equal(stopped.glyph, "○");
  assert.equal(meterOf(headerState("idle"), session({ state: "stopped" })).word, "stopped");
});

test("panelHeader: Focus reads the TARGET member's glyph + name; All reads the ROOT's", () => {
  const state: ViewState = {
    ...initialState(),
    members: [
      { id: "root-1", label: "root-1", state: "idle", isRoot: true, model: "m-root" },
      { id: "agent:w1", label: "w1", state: "running", isRoot: false, model: "m-w1" },
    ],
    targeted: "agent:w1",
  };
  const rendered = renderState(state, "agent:w1");
  const focus = identOf(rendered, session(), "focus");
  assert.equal(focus.glyph, "⠋", "Focus reads the TARGET's glyph");
  assert.equal(focus.name, "w1", "Focus names the TARGET");
  const all = identOf(rendered, session(), "all");
  assert.equal(all.glyph, "○", "All reads the ROOT's glyph alone");
  assert.equal(all.name, "orchestrator", "All names the root");
});

test("panelHeader: the identity glyph is the TARGET member's (a retarget changes it)", () => {
  const state: ViewState = {
    ...initialState(),
    members: [
      { id: "root-1", label: "root-1", state: "idle", isRoot: true },
      { id: "agent:w1", label: "w1", state: "running", isRoot: false },
    ],
    targeted: "root-1",
  };
  assert.equal(identOf(renderState(state, "root-1"), session()).glyph, "○");
  const member = identOf(renderState(state, "agent:w1"), session());
  assert.equal(member.glyph, "⠋");
  assert.equal(member.name, "w1");
});

test("panelHeader: the mode cell carries the mode (the ONE All/Focus control)", () => {
  const focus = panelHeader(headerState("idle"), session(), "focus").cells.find((c) => c.kind === "mode");
  assert.equal(focus?.kind === "mode" ? focus.mode : null, "focus");
  const all = panelHeader(headerState("idle"), session(), "all").cells.find((c) => c.kind === "mode");
  assert.equal(all?.kind === "mode" ? all.mode : null, "all");
});

/* ------------------------------------------------------------------- turns */

function aTool(isError = false): RenderedBlock {
  return {
    kind: "tool",
    html: "",
    live: false,
    tool: {
      callId: "t1",
      name: "edit",
      summary: "s",
      outputText: "",
      done: true,
      isError,
      hasDiff: false,
    },
  };
}

test("turns: a user block opens a you turn; the rest fold into one wcode turn", () => {
  const grouped = turns([
    { kind: "user", html: "hi", live: false },
    { kind: "assistant", html: "…", live: false },
    aTool(),
  ]);
  assert.equal(grouped.length, 2);
  assert.equal(grouped[0].role, "you");
  assert.equal(grouped[0].who.className, "byline you");
  assert.equal(grouped[0].who.glyph, "", "the user's byline carries no glyph");
  assert.equal(grouped[0].who.name, "You");
  assert.deepEqual(grouped[0].blocks.map((b) => b.kind), ["user"]);
  assert.equal(grouped[1].role, "wcode");
  assert.equal(grouped[1].who.className, "byline");
  assert.equal(grouped[1].who.glyph, "❯", "the root's byline is the ❯");
  assert.deepEqual(grouped[1].blocks.map((b) => b.kind), ["assistant", "tool"]);
});

test("turns: a leading non-user block opens a wcode turn", () => {
  const grouped = turns([{ kind: "notice", html: "x", live: false }]);
  assert.equal(grouped.length, 1);
  assert.equal(grouped[0].role, "wcode");
});

test("turns: a turn containing an error block is role 'err'", () => {
  const grouped = turns([{ kind: "error", html: "boom", live: false }]);
  assert.equal(grouped[0].role, "err");
  assert.equal(grouped[0].who.className, "byline", "an err turn still reads as wcode");
});

/* ---------------------------------------------------------------- diffStat */

test("diffStat counts single-hunk body lines and skips the @@ header", () => {
  const diff = "@@ -40,6 +40,9 @@\n ctx\n-old\n+new1\n+new2\n ctx2\n";
  assert.deepEqual(diffStat(diff), { added: 2, removed: 1 });
});

test("diffStat: the truncation tail is not counted, and junk is zero", () => {
  const truncated = "@@ -1,1 +1,1 @@\n-a\n+b\n… (+37 more lines)";
  assert.deepEqual(diffStat(truncated), { added: 1, removed: 1 });
  assert.deepEqual(diffStat(""), { added: 0, removed: 0 });
  assert.deepEqual(diffStat("not a diff"), { added: 0, removed: 0 });
});

/* -------------------------------------------------------------- empty states */

test("emptyKind picks the deliberate empty state", () => {
  assert.equal(emptyKind(idle(), session({ state: "crashed" })), "crashed");
  assert.equal(emptyKind(idle(), session({ state: "starting" })), "starting");
  assert.equal(emptyKind(idle(), session({ state: "stopped" })), "stopped");
  assert.equal(emptyKind(renderState(reduce(initialState(), { type: "agent_start" })), session()), "working");
  assert.equal(emptyKind(idle(), session()), "idle");
});

test("emptyKind: 'no-session' when the child never handed us a root session", () => {
  // The panel starts `{ id: null, state: "stopped" }`; that reads as
  // "No session.", not "Stopped" (a session that had run keeps its id).
  assert.equal(emptyKind(idle(), session({ id: null })), "no-session");
  assert.equal(emptyKind(idle(), session({ id: null, state: "stopped" })), "no-session");
  assert.equal(emptyKind(idle(), session()), "idle");
});

test("emptySpec names copy for all six states", () => {
  assert.equal(emptySpec("crashed").title, "wcode crashed.");
  assert.equal(emptySpec("crashed").stderr, true);
  assert.equal(emptySpec("starting").spin, true);
  assert.equal(emptySpec("idle").spin, false);
  assert.equal(emptySpec("no-session").sub, 'Run "wcode: Start Session".');
  assert.equal(emptySpec("stopped").sub, 'Run "wcode: Start Session" to resume.');
  assert.equal(emptySpec("idle").stderr, false);
});

/* ------------------------------------------------------------ expand / class */

test("foldOpen: auto-open while running, auto-collapse when done", () => {
  const none = new Map<string, FoldOverride>();
  assert.equal(foldOpen(none, "t1", false), true, "running -> open");
  assert.equal(foldOpen(none, "t1", true), false, "done -> collapsed");
});

test("toggleFold records an override that wins in its phase", () => {
  const none = new Map<string, FoldOverride>();
  const closed = toggleFold(none, "t1", false); // the user closes a RUNNING fold
  assert.equal(foldOpen(closed, "t1", false), false, "the override wins while running");
  assert.equal(none.size, 0, "the input map is untouched");

  const flipped = toggleFold(closed, "t1", false);
  assert.equal(foldOpen(flipped, "t1", false), true, "toggling again re-opens it");
});

test("foldOpen: an override EXPIRES at the running -> done transition", () => {
  const closed = toggleFold(new Map<string, FoldOverride>(), "t1", false); // closed while running
  // the tool finishes: the override was made for the RUNNING phase, so it no longer applies
  assert.equal(foldOpen(closed, "t1", true), false, "auto-collapse resumes (the next state change)");

  // a toggle made on a DONE fold sticks (there is no further state change)
  const openedDone = toggleFold(closed, "t1", true);
  assert.equal(foldOpen(openedDone, "t1", true), true);
});

test("classNames composes the content-element classes", () => {
  assert.equal(classNames(aTool(false)), "fold tool");
  assert.equal(classNames(aTool(false), true), "fold tool open");
  assert.equal(classNames(aTool(true)), "fold tool error");
  assert.equal(classNames(aTool(true), true), "fold tool error open");
  assert.equal(classNames({ kind: "assistant", html: "", live: true }), "body live");
  assert.equal(classNames({ kind: "assistant", html: "", live: false }), "body");
  assert.equal(classNames({ kind: "user", html: "", live: false }), "body");
  assert.equal(classNames({ kind: "notice", html: "", live: false }), "body notice");
  assert.equal(classNames({ kind: "error", html: "", live: false }), "body error");
  assert.equal(classNames({ kind: "btw", html: "", live: false }), "body btw");
});

/* -------------------------------------------------- V13: tool status */

test("toolState: a tool row is EXACTLY one of running / error / complete", () => {
  assert.equal(toolState({ done: false, isError: false }), "running");
  assert.equal(toolState({ done: true, isError: true }), "error");
  assert.equal(toolState({ done: true, isError: false }), "complete");
  // `isError` only lands at `tool_execution_end`, so a not-done row is running.
  assert.equal(toolState({ done: false, isError: true }), "running", "running wins while not done");
});

test("toolStatus: the exceptions carry a glyph + word; complete is quiet", () => {
  assert.deepEqual(toolStatus({ done: false, isError: false }), {
    className: "run",
    glyph: "⠋",
    word: "running…",
  });
  assert.deepEqual(toolStatus({ done: true, isError: true }), {
    className: "fail",
    glyph: "✗",
    word: "failed",
  });
  assert.deepEqual(
    toolStatus({ done: true, isError: false }),
    { className: null, glyph: "", word: "" },
    "a COMPLETE tool carries no glyph and no colour",
  );
  // The glyphs are the LOCKED member-state vocabulary, not new art.
  assert.equal(toolStatus({ done: false, isError: false }).glyph, memberGlyph("running").glyph);
  assert.equal(toolStatus({ done: true, isError: true }).glyph, memberGlyph("failed").glyph);
});

test("the three tool-status colours live on the ⚙ mark (complete neutral, running accent, error red)", () => {
  const css = readFileSync(resolve(here, "../media/chat.css"), "utf8");
  assert.match(css, /\.tname \{[^}]*color: var\(--vscode-foreground\)/, "complete: the ⚙ mark is NEUTRAL");
  assert.match(
    css,
    /details\.fold\.tool\[data-live="1"\] \.tname \{[^}]*color: var\(--wc-accent\)/,
    "running: the ⚙ mark is accent (BLUE)",
  );
  assert.match(
    css,
    /details\.fold\.tool\.error \.tname \{[^}]*color: var\(--vscode-errorForeground\)/,
    "error: the ⚙ mark is RED",
  );
});

test("the code block has no border: `pre.quote`/`.code` are fill-only recesses", () => {
  const css = readFileSync(resolve(here, "../media/chat.css"), "utf8");
  assert.match(css, /pre\.quote \{[^}]*background: var\(--vscode-textCodeBlock-background\)/, "the quote is a fill");
  const codeRule = css.slice(css.indexOf(".code {"), css.indexOf("}", css.indexOf(".code {")));
  assert.ok(!/border:/.test(codeRule), "`.code` has no border");
  const quoteRule = css.slice(css.indexOf("pre.quote {"), css.indexOf("}", css.indexOf("pre.quote {")));
  assert.ok(!/border:/.test(quoteRule), "`pre.quote` has no border");
});

test("bodyKind: a diff-bearing tool gets the `.review` card, else the output card", () => {
  // EXACTLY one L2a body per fold (V12): `.review` for a diff, the output card otherwise.
  assert.equal(bodyKind({ hasDiff: true }), "review");
  assert.equal(bodyKind({ hasDiff: false }), "output");
  // The real `RenderedTool`: `hasDiff` is the predicate `render.ts` derives from `diff`.
  const plain = aTool(false).tool ?? { hasDiff: false };
  const diffTool = { ...plain, hasDiff: true, diff: "@@ -1,1 +1,1 @@\n-a\n+b" };
  assert.equal(bodyKind(plain), "output");
  assert.equal(bodyKind(diffTool), "review");
});

/* ------------------------------------------------- V10: the notice row */

test("a run failure renders ONCE — the `error` block (`.body.error`); the header stays clean", () => {
  const state = reduce(
    reduce(initialState(), { type: "agent_start" }, "root-1"),
    { type: "error", message: "boom" },
    "root-1",
  );
  assert.equal(state.status.lastError, "boom", "the reducer records the failure");
  const rendered = renderState(state, "root-1");
  // EXACTLY one error BLOCK: `status.lastError` is the record, the block is the render — ONE
  // fact, not two rows.
  const error = rendered.blocks.find((b) => b.kind === "error");
  assert.notEqual(error, undefined, "the error block is present");
  assert.equal(error === undefined ? "" : classNames(error), "body error", "it paints as `.body.error`");
  assert.equal(rendered.blocks.filter((b) => b.kind === "error").length, 1, "rendered ONCE");
  // …and NO header cell carries the failure (V6 removed the cell; it must not return).
  const cells = panelHeader(rendered, session(), "focus").cells;
  assert.ok(!cells.some((c) => "text" in c && c.text.includes("boom")), "the header holds no error cell");
});

test("the notice/btw fold: distinct classes, ONE L1 rule in the sheet (friction 5)", () => {
  // Distinct in the CONTRACT — the renderer still chooses one.
  assert.equal(classNames({ kind: "notice", html: "", live: false }), "body notice");
  assert.equal(classNames({ kind: "btw", html: "", live: false }), "body btw");
  // ONE treatment in the SHEET: a single rule lists both selectors, and `.body.btw` never
  // opens a rule of its own (the fold is CSS-only).
  const css = readFileSync(resolve(here, "../media/chat.css"), "utf8");
  assert.match(css, /\.body\.notice,\s*\n\.body\.btw\s*\{/, "notice + btw share ONE rule");
  const withoutFold = css.replace(/\.body\.notice,\s*\n\.body\.btw\s*\{/, "/* fold */");
  assert.ok(!/(^|\n)\s*\.body\.btw\s*\{/.test(withoutFold), "`.body.btw` has no rule of its own");
  // …and it is the dim inset LINE (`--vscode-descriptionForeground`, a 2px left rule).
  const rule = css.slice(css.indexOf(".body.notice,"));
  assert.match(rule, /border-left: 2px solid var\(--vscode-descriptionForeground\)/);
  assert.match(rule, /color: var\(--vscode-descriptionForeground\)/);
});

/* --------------------------------------------------------- parseToWebview */

test("parseToWebview accepts a well-formed snapshot", () => {
  const message: ToWebview = {
    kind: "state",
    state: idle(),
    session: session(),
    context: null,
    verdicts: {},
    mode: "focus",
  };
  const parsed = parseToWebview(message);
  assert.ok(parsed);
  assert.equal(parsed.kind, "state");
  assert.equal(parsed.session.id, "root-1");
  assert.equal(parsed.session.state, "ready");
  assert.deepEqual(parsed.state.blocks, []);
  assert.equal(parsed.context, null, "no context ⇒ null");
  assert.deepEqual(parsed.verdicts, {}, "no verdicts ⇒ {}");

  const withCtx = parseToWebview({ ...message, context: { path: "src/a.rs", startLine: 3, endLine: 5 } });
  assert.deepEqual(withCtx?.context, { path: "src/a.rs", startLine: 3, endLine: 5 });

  const withV = parseToWebview({ ...message, verdicts: { t1: "accepted" } });
  assert.deepEqual(withV?.verdicts, { t1: "accepted" });

  // LENIENT (like `context`): a bad verdict degrades to `{}`; the snapshot SURVIVES.
  const bad = parseToWebview({ ...message, verdicts: { t1: "bogus" } });
  assert.ok(bad, "the snapshot survives a bad verdict");
  assert.deepEqual(bad.verdicts, {}, "a bad verdict degrades to {}, not a drop");

  // `mode` is LENIENT (like `context`/`verdicts`): anything but "all" degrades to "focus".
  assert.equal(parseToWebview({ ...message, mode: "all" })?.mode, "all");
  assert.equal(parseToWebview({ ...message, mode: "bogus" })?.mode, "focus");

  // LENIENT (M1): a non-conforming context degrades to null and the snapshot SURVIVES.
  const lenient = parseToWebview({ ...message, context: "x" });
  assert.ok(lenient, "the snapshot survives a wrong-typed context");
  assert.equal(lenient.context, null);
  assert.equal(
    parseToWebview({ ...message, context: { path: "a.rs" } })?.context,
    null,
    "a partial context also degrades to null",
  );
});

test("parseToWebview rejects junk and malformed snapshots", () => {
  assert.equal(parseToWebview(null), null);
  assert.equal(parseToWebview(42), null);
  assert.equal(parseToWebview({}), null);
  assert.equal(parseToWebview({ kind: "append", block: {} }), null, "the append variant is gone");
  assert.equal(parseToWebview({ kind: "diff", callId: "t1" }), null, "the diff variant is gone");
  assert.equal(parseToWebview({ kind: "state" }), null, "no state");
  assert.equal(parseToWebview({ kind: "state", state: {} }), null, "no blocks");
  assert.equal(parseToWebview({ kind: "state", state: { blocks: [] } }), null, "no status");
  assert.equal(
    parseToWebview({ kind: "state", state: { blocks: [], status: {} }, session: {} }),
    null,
    "no session state",
  );
  assert.equal(
    parseToWebview({
      kind: "state",
      state: { blocks: [], status: {} },
      session: { id: "r", state: "bogus", stderrTail: "" },
    }),
    null,
    "an unknown FSM state",
  );
  assert.equal(
    parseToWebview({
      kind: "state",
      state: { blocks: [], status: {} },
      session: { id: 7, state: "ready", stderrTail: "" },
    }),
    null,
    "a non-string id",
  );
  assert.equal(
    parseToWebview({ kind: "state", state: { blocks: [], status: {} }, session: { id: null, state: "ready" } }),
    null,
    "no stderrTail",
  );
});
/* --------------------------------------------------------------- composer */

test("composerControls: mode + the two toggle segments + the `Stop` gate (NO model)", () => {
  const running: ViewState = {
    ...initialState(),
    members: [{ id: "root-1", label: "root-1", state: "running", isRoot: true, model: "sonnet" }],
    targeted: "root-1",
    status: { running: true, planMode: true },
  };
  const controls = composerControls(renderState(running, "root-1"));
  assert.equal(controls.mode, "Plan");
  // The `Model: …` span is gone (the model lives in the header's `▾` disclosure now).
  assert.ok(!("model" in controls), "composerControls carries no `model` field");
  // The mode is the `.seg` FORM: two segments (Act then Plan), exactly one pressed.
  assert.deepEqual(
    controls.segments.map((s) => [s.mode, s.pressed]),
    [
      ["Act", false],
      ["Plan", true],
    ],
  );
  // `Stop` is conditional: present ONLY while a run is in flight.
  assert.equal(controls.stop, true, "a running session shows Stop");
});

test("composerControls: an idle composer is Act, with NO `Stop`", () => {
  const controls = composerControls(renderState(initialState(), null));
  assert.equal(controls.mode, "Act");
  assert.deepEqual(
    controls.segments.map((s) => [s.mode, s.pressed]),
    [
      ["Act", true],
      ["Plan", false],
    ],
  );
  assert.equal(controls.stop, false, "no Stop when nothing is running");
});

test("selectionRef formats @path#Lstart-end (1-based, inclusive)", () => {
  assert.equal(selectionRef({ path: "src/panel.ts", startLine: 88, endLine: 104 }), "@src/panel.ts#L88-104");
  assert.equal(selectionRef({ path: "a.rs", startLine: 12, endLine: 12 }), "@a.rs#L12-12");
});

test("composeSubmit prepends the ref, else passes the text through", () => {
  assert.equal(composeSubmit("hi", null), "hi");
  assert.equal(composeSubmit("hi", { path: "src/p.ts", startLine: 1, endLine: 2 }), "@src/p.ts#L1-2\n\nhi");
});
/* --------------------------------------------------------- the live line */

test("livelineLabel: the live type-line text is count-only (`{n} working`)", () => {
  assert.equal(livelineLabel(0), "0 working");
  assert.equal(livelineLabel(1), "1 working");
  assert.equal(livelineLabel(3), "3 working");
});

test("reduced-motion freezes the liveline dots: the gate names `.liveline .dots`", () => {
  const css = readFileSync(resolve(here, "../media/chat.css"), "utf8");
  const gate = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"));
  assert.match(gate, /\.liveline \.dots\s*\{/, "the liveline dots are reduced-motion gated");
});

test("isActivationKey: Enter and Space activate the caption row; other keys do not", () => {
  assert.equal(isActivationKey("Enter"), true);
  assert.equal(isActivationKey(" "), true, "Space");
  assert.equal(isActivationKey("Spacebar"), true, "the legacy Space name");
  assert.equal(isActivationKey("a"), false);
  assert.equal(isActivationKey("Tab"), false, "Tab must still move focus, not retarget");
});

test("the team caption is keyboard-activatable: a tabindex row whose keydown posts focus-member", () => {
  // There is no DOM harness, so this guards the WIRING: remove the tabindex or the
  // keydown path and the caption row goes mouse-only again — the test fails.
  const src = readFileSync(resolve(here, "../src/webview/chat.ts"), "utf8");
  assert.match(src, /node\.tabIndex = 0/, "the caption row is focusable");
  assert.match(src, /addEventListener\("keydown"[\s\S]{0,240}?focus-member/, "a keydown posts focus-member");
});

/* -------------------------------------------------------- working group */

test("workingGroup: other members that are running or carry a liveAction (the pill's source)", () => {
  const members: SessionMember[] = [
    { id: "root-1", label: "root-1", state: "running", isRoot: true }, // the target: EXCLUDED
    { id: "agent:w1", label: "w1", state: "running", isRoot: false, liveAction: "edit src/f.rs" },
    { id: "agent:w2", label: "w2", state: "done", isRoot: false }, // finished: excluded
    { id: "agent:w3", label: "w3", state: "idle", isRoot: false, liveAction: "grep onOpenDiff" },
  ];
  const group = workingGroup(members, "root-1");
  assert.equal(group.count, 2);
  assert.deepEqual(group.rows.map((r) => r.id), ["agent:w1", "agent:w3"]);
  assert.equal(group.rows[0].name, "w1");
  assert.equal(group.rows[0].running, true);
  assert.equal(group.rows[0].action, "edit src/f.rs");
  assert.equal(group.rows[1].running, false);
  assert.equal(group.rows[1].action, "grep onOpenDiff");
});

test("workingGroup: a running member with no liveAction still appears (empty action)", () => {
  const members: SessionMember[] = [{ id: "agent:w1", label: "w1", state: "running", isRoot: false }];
  const group = workingGroup(members, "root-1");
  assert.equal(group.count, 1);
  assert.equal(group.rows[0].action, "");
});

test("workingGroup is empty when no other member is active", () => {
  const members: SessionMember[] = [
    { id: "root-1", label: "root-1", state: "running", isRoot: true },
    { id: "agent:w2", label: "w2", state: "done", isRoot: false },
  ];
  assert.deepEqual(workingGroup(members, "root-1"), { count: 0, rows: [] });
});

test("workingGroup: the root reads as 'orchestrator' when it is NOT the target", () => {
  const members: SessionMember[] = [
    { id: "root-1", label: "root-1", state: "running", isRoot: true },
    { id: "agent:w1", label: "w1", state: "idle", isRoot: false },
  ];
  const group = workingGroup(members, "agent:w1");
  assert.equal(group.rows[0].name, "orchestrator");
});


test("teamCaption: one row per member; the glyph is memberGlyph(state) for EVERY row (the root too)", () => {
  const rows: RosterItem[] = [
    { id: "root-1", label: "orchestrator", state: "running", isRoot: true },
    { id: "agent:w1", label: "explorer", state: "done", isRoot: false, model: "sonnet", liveAction: "grep onOpenDiff" },
    { id: "agent:w2", label: "developer", state: "failed", isRoot: false },
    { id: "agent:w3", label: "reviewer", state: "idle", isRoot: false },
  ];
  const caption = teamCaption(rows, "agent:w1");
  assert.equal(caption.length, 4);
  // STATE is the glyph's class + char — for every row, INCLUDING the root (never `❯`).
  assert.deepEqual(
    caption.map((r) => [r.glyph, r.glyphClass]),
    [
      ["⠋", "g-run"],
      ["✓", "g-done"],
      ["✗", "g-err"],
      ["○", "g-idle"],
    ],
  );
  assert.equal(caption[0].className, "m", "no swatch edge; not the target");
  assert.equal(caption[1].className, "m sel", "the target carries `sel` (the retarget affordance)");
  assert.equal(caption[0].name, "orchestrator");
  assert.equal(caption[1].action, "grep onOpenDiff", "the liveAction wins");
  assert.equal(caption[3].action, "idle", "the action falls back to the state word");
});

test("panelHeader: no member-count cell remains on the masthead (the team is a caption now)", () => {
  const state = renderState(
    {
      ...initialState(),
      members: [
        { id: "root-1", label: "root-1", state: "idle", isRoot: true },
        { id: "agent:w1", label: "w1", state: "idle", isRoot: false },
      ],
      targeted: "root-1",
    },
    "root-1",
  );
  const texts = panelHeader(state, session(), "focus").cells.flatMap((c) => ("text" in c ? [c.text] : []));
  assert.ok(!texts.some((t) => /\d+ members/.test(t)), "the bar carries no `N members` cell");
});

test("turns: WITHOUT a resolver (Focus), labels stay You/wcode", () => {
  const t = turns([
    { kind: "user", html: "hi", live: false },
    { kind: "assistant", html: "", live: false },
  ]);
  assert.equal(t[0].who.name, "You");
  assert.equal(t[1].who.name, "wcode");
  assert.equal(t[1].who.glyph, "❯");
});

test("turns: WITH a resolver (All), a non-user turn is labeled by its origin member", () => {
  const blocks: RenderedBlock[] = [
    { kind: "assistant", html: "", live: false, origin: "agent:w1" },
    { kind: "tool", html: "", live: false, origin: "agent:w1" },
  ];
  const t = turns(blocks, (origin) => (origin === "agent:w1" ? { name: "explorer", isRoot: false, glyph: "⠋" } : undefined));
  assert.equal(t.length, 1, "one contiguous member run");
  assert.equal(t[0].who.name, "explorer");
  assert.equal(t[0].who.glyph, "⠋", "the byline carries the member's STATE glyph");
  assert.equal(t[0].who.className, "byline", "no swatch class");

  // A ROOT origin keeps the `❯` glyph (the resolver carries `isRoot`).
  const rootTurn = turns(
    [{ kind: "assistant", html: "", live: false, origin: "root-1" }],
    (o) => (o === "root-1" ? { name: "orchestrator", isRoot: true, glyph: "⠋" } : undefined),
  );
  assert.equal(rootTurn[0].who.glyph, "❯");
  assert.equal(rootTurn[0].who.name, "orchestrator");
});

test("turns: an ORIGIN change starts a NEW turn (mixed-origin adjacency)", () => {
  const blocks: RenderedBlock[] = [
    { kind: "assistant", html: "", live: false, origin: "agent:w1" },
    { kind: "assistant", html: "", live: false, origin: "agent:w2" }, // NO user text between
  ];
  const members: Record<string, { name: string; isRoot: boolean; glyph: string }> = {
    "agent:w1": { name: "explorer", isRoot: false, glyph: "⠋" },
    "agent:w2": { name: "developer", isRoot: false, glyph: "⠋" },
  };
  const t = turns(blocks, (origin) => (origin === undefined ? undefined : members[origin]));
  assert.equal(t.length, 2, "TWO turns, one per origin — NOT one collapsed turn");
  assert.equal(t[0].who.name, "explorer");
  assert.equal(t[1].who.name, "developer");
});

test("memberSwatch: the root always takes the first swatch", () => {
  assert.equal(memberSwatch(0, true), MEMBER_SWATCHES[0]);
  assert.equal(memberSwatch(3, true), MEMBER_SWATCHES[0], "the root ignores its index");
});

test("memberSwatch: non-root members cycle the palette by position", () => {
  assert.equal(memberSwatch(0, false), MEMBER_SWATCHES[0]);
  assert.equal(memberSwatch(1, false), MEMBER_SWATCHES[1]);
  assert.equal(memberSwatch(2, false), MEMBER_SWATCHES[2]);
});

test("memberSwatch: adjacent members differ, and the palette wraps", () => {
  const n = MEMBER_SWATCHES.length;
  // Every member in the first lap is a DIFFERENT colour — the whole point of the rail.
  const lap = Array.from({ length: n }, (_, i) => memberSwatch(i, false));
  assert.equal(new Set(lap).size, n, "no repeats within one lap");
  // Past the palette it wraps back to the first colour (a collision only past six members).
  assert.equal(memberSwatch(n, false), MEMBER_SWATCHES[0]);
  assert.equal(memberSwatch(n + 1, false), MEMBER_SWATCHES[1]);
});

test("memberInitial: the root reads ❯, a member its initial (upper-cased)", () => {
  assert.equal(memberInitial("orchestrator", true), "❯");
  assert.equal(memberInitial("scout", false), "S");
  assert.equal(memberInitial("explorer", false), "E");
  assert.equal(memberInitial("Developer", false), "D", "already upper-cased is unchanged");
});

test("turns: a peer block opens its OWN turn, labeled by its sender", () => {
  const blocks: RenderedBlock[] = [
    { kind: "assistant", html: "", live: false, origin: "root-1" },
    { kind: "peer", html: "found 3 issues", live: false, from: "agent:explorer" },
    { kind: "assistant", html: "", live: false, origin: "root-1" },
  ];
  const resolve = (id: string | undefined) =>
    id === "root-1"
      ? { name: "orchestrator", isRoot: true, glyph: "⠋" }
      : id === "agent:explorer"
        ? { name: "explorer", isRoot: false, glyph: "✓" }
        : undefined;
  const t = turns(blocks, resolve);
  assert.equal(t.length, 3, "the peer message is its own turn");
  assert.equal(t[1].who.name, "explorer", "labeled by the SENDER, never 'you'");
  assert.equal(t[1].who.glyph, "✓");
  assert.equal(t[1].who.className, "byline");
  assert.equal(t[1].blocks[0].kind, "peer");
});

test("turns: WITHOUT a resolver (Focus), the assistant turn stays the plain `wcode` byline", () => {
  const t = turns([{ kind: "assistant", html: "", live: false }]);
  assert.equal(t[0].who.className, "byline");
  assert.equal(t[0].who.glyph, "❯");
  assert.equal(t[0].who.name, "wcode", "Focus still labels the assistant turn 'wcode'");
});

/* ------------------------------------------------------- V14: composition */

test("turns: `.new` marks a SPEAKER CHANGE; a repeat speaker is a continuation", () => {
  // Two `user` blocks in a row are the SAME speaker (you -> you), so the second CONTINUES;
  // the assistant flips the speaker, and the third `user` flips it back.
  const t = turns([
    { kind: "user", html: "one", live: false },
    { kind: "user", html: "two", live: false },
    { kind: "assistant", html: "", live: false },
    { kind: "user", html: "three", live: false },
  ]);
  assert.deepEqual(
    t.map((x) => x.isNew),
    [true, false, true, true],
    "a speaker change is new; the repeat `you` is a continuation",
  );
});

test("turns: two peer blocks from the SAME sender are one speaker (the second continues)", () => {
  const t = turns([
    { kind: "peer", html: "", live: false, from: "agent:w1", origin: "agent:w1" },
    { kind: "peer", html: "", live: false, from: "agent:w1", origin: "agent:w1" },
    { kind: "peer", html: "", live: false, from: "agent:w2", origin: "agent:w2" },
  ]);
  assert.deepEqual(t.map((x) => x.isNew), [true, false, true], "the repeated sender continues");
  assert.equal(t[0].isNew, true, "the FIRST turn always opens a speaker");
});

test("the composition is in the sheet: a reading column, a byline, the paragraph beat (no spine)", () => {
  const css = readFileSync(resolve(here, "../media/chat.css"), "utf8");
  // the reading column is capped + centered.
  assert.match(css, /\.col \{[^}]*max-width: var\(--ed-measure\)/, "the column is capped");
  assert.match(css, /\.col \{[^}]*margin: 0 auto/, "the column is centered");
  // the byline: small-caps mono, no avatar box.
  assert.match(css, /\.byline \{[^}]*text-transform: uppercase/, "the byline is small-caps");
  // a speaker change is a paragraph break (--wc-5), not a row.
  assert.match(css, /\.turn\.new \{[^}]*margin-top: var\(--wc-5\)/, "a speaker change gets the beat");
  // NO spine, NO `who` hairline, NO `you` fill.
  assert.ok(!/\.rail\s*\{/.test(css), "no spine");
  assert.ok(!/\.turn \.who/.test(css), "no turn-head hairline");
  assert.ok(!/\.turn\.you\s*\{/.test(css), "no `you` fill");
});


