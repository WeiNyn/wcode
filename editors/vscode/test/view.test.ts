import assert from "node:assert/strict";
import { test } from "node:test";

import { initialState, reduce, type SessionMember, type ViewState } from "../src/reducer.ts";
import { renderState, type RenderedBlock, type RenderedState } from "../src/render.ts";
import { parseToWebview, type PanelSessionInfo, type ToWebview, type ViewMode } from "../src/webview.ts";
import {
  classNames,
  composerControls,
  composeSubmit,
  diffStat,
  emptyKind,
  emptySpec,
  foldOpen,
  formatTokens,
  panelHeader,
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
} from "../src/webview/view.ts";

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

/** The identity cell of the ONE-row header (asserts cell 1 IS the identity). */
function identOf(state: RenderedState, session: PanelSessionInfo, mode: ViewMode = "focus"): IdentCell {
  const cell = panelHeader(state, session, mode).cells[0];
  assert.equal(cell?.kind, "ident");
  return cell as IdentCell;
}

test("panelHeader: ONE row — identity, All/Focus seg, spacer, ctx meter, ▾ disclosure", () => {
  const state = headerState("idle", { status: { running: false, planMode: false, contextUsed: 42_000 } });
  assert.deepEqual(
    panelHeader(state, session(), "focus").cells.map((c) => c.kind),
    ["ident", "seg", "spacer", "meter", "more"],
  );
  // No `contextUsed` ⇒ no meter cell at all (the slot is conditional).
  assert.deepEqual(
    panelHeader(headerState("idle"), session(), "focus").cells.map((c) => c.kind),
    ["ident", "seg", "spacer", "more"],
  );
});

test("formatTokens: a k/M suffix, `<1000` verbatim", () => {
  assert.equal(formatTokens(42), "42");
  assert.equal(formatTokens(999), "999");
  assert.equal(formatTokens(1_500), "1.5k");
  assert.equal(formatTokens(42_000), "42k");
  assert.equal(formatTokens(2_000_000), "2M");
  assert.equal(formatTokens(20_000_000), "20M");
});

test("panelHeader: the meter is the TEXT `ctx 42k` — no gauge, no denominator", () => {
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

test("panelHeader: the identity glyph — the member's liveness for `ready`, the FSM for terminal states", () => {
  assert.equal(identOf(headerState("idle"), session()).glyph, "○");
  assert.equal(identOf(headerState("running"), session()).glyph, "⠋");
  assert.equal(identOf(headerState("done"), session()).glyph, "✓");
  assert.equal(identOf(headerState("failed"), session()).glyph, "✗");
  // The running glyph spins; a healthy session carries NO word.
  assert.equal(identOf(headerState("running"), session()).glyphClass, "glyph g-run spin");
  assert.equal(identOf(headerState("idle"), session()).word, "");
});

test("panelHeader: a crashed session reads `✗` + the word 'crashed' (never a spinner)", () => {
  const crashed = identOf(headerState("idle"), session({ state: "crashed" }));
  assert.equal(crashed.glyph, "✗");
  assert.equal(crashed.glyphClass, "glyph g-err");
  assert.equal(crashed.word, "crashed");
  assert.equal(crashed.stateTitle, "crashed");
});

test("panelHeader: starting spins `⠋`, stopped is `○`; both carry their word", () => {
  const starting = identOf(headerState("idle"), session({ state: "starting" }));
  assert.equal(starting.glyph, "⠋");
  assert.equal(starting.word, "starting");
  const stopped = identOf(headerState("idle"), session({ state: "stopped" }));
  assert.equal(stopped.glyph, "○");
  assert.equal(stopped.word, "stopped");
});

test("panelHeader: Focus shows the TARGET member; All hides the chip and shows the ROOT glyph", () => {
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
  assert.equal(focus.chip, "❯ w1 ▾");
  assert.equal(focus.chipHidden, false);
  assert.equal(focus.glyph, "⠋", "Focus reads the TARGET's glyph");
  const all = identOf(rendered, session(), "all");
  assert.equal(all.glyph, "○", "All reads the ROOT's glyph alone");
  assert.equal(all.chip, "❯ w1 ▾", "the chip stays in the DOM…");
  assert.equal(all.chipHidden, true, "…but is HIDDEN in All");
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
  assert.equal(member.chip, "❯ w1 ▾");
});

test("panelHeader: the seg cell carries the mode (the ONE All/Focus control)", () => {
  const focus = panelHeader(headerState("idle"), session(), "focus").cells.find((c) => c.kind === "seg");
  assert.equal(focus?.kind === "seg" ? focus.mode : null, "focus");
  const all = panelHeader(headerState("idle"), session(), "all").cells.find((c) => c.kind === "seg");
  assert.equal(all?.kind === "seg" ? all.mode : null, "all");
});

test("panelHeader: the ▾ disclosure carries the session id + model (no effort row)", () => {
  const more = panelHeader(headerState("idle"), session(), "focus").cells.find((c) => c.kind === "more");
  assert.equal(more?.kind, "more");
  assert.deepEqual(more?.kind === "more" ? more.rows : [], [
    { key: "session", value: "root-1" },
    { key: "model", value: "sonnet" },
  ]);
  // No session id ⇒ "no session yet"; no member ⇒ no model row at all.
  const bare = panelHeader(renderState(initialState(), null), session({ id: null }), "focus").cells.find(
    (c) => c.kind === "more",
  );
  assert.equal(bare?.kind, "more");
  assert.deepEqual(bare?.kind === "more" ? bare.rows : [], [{ key: "session", value: "no session yet" }]);
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
      outputHtml: "",
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
  assert.equal(grouped[0].who.className, "who you");
  assert.equal(grouped[0].who.avatar, "Y");
  assert.deepEqual(grouped[0].blocks.map((b) => b.kind), ["user"]);
  assert.equal(grouped[1].role, "wcode");
  assert.equal(grouped[1].who.className, "who wcode");
  assert.equal(grouped[1].who.avatar, "❯");
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
  assert.equal(grouped[0].who.className, "who wcode", "an err turn still reads as wcode");
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

test("composerControls: mode reflects planMode, model is the target member's", () => {
  const state: ViewState = {
    ...initialState(),
    members: [{ id: "root-1", label: "root-1", state: "idle", isRoot: true, model: "sonnet" }],
    targeted: "root-1",
    status: { running: false, planMode: true },
  };
  assert.equal(composerControls(renderState(state, "root-1")).mode, "Plan");
  assert.equal(composerControls(renderState(state, "root-1")).model, "sonnet");
  // No target / no model ⇒ Act, and model null (never an em-dash).
  assert.equal(composerControls(renderState(initialState(), null)).mode, "Act");
  assert.equal(composerControls(renderState(initialState(), null)).model, null);
});

test("selectionRef formats @path#Lstart-end (1-based, inclusive)", () => {
  assert.equal(selectionRef({ path: "src/panel.ts", startLine: 88, endLine: 104 }), "@src/panel.ts#L88-104");
  assert.equal(selectionRef({ path: "a.rs", startLine: 12, endLine: 12 }), "@a.rs#L12-12");
});

test("composeSubmit prepends the ref, else passes the text through", () => {
  assert.equal(composeSubmit("hi", null), "hi");
  assert.equal(composeSubmit("hi", { path: "src/p.ts", startLine: 1, endLine: 2 }), "@src/p.ts#L1-2\n\nhi");
});
/* -------------------------------------------------------- working group */

test("workingGroup: other members that are running or carry a liveAction", () => {
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
  // No `glyph` field — a row always renders ●; `running` picks .g-run / .g-idle.
  assert.equal(group.rows[0].running, true);
  assert.equal(group.rows[0].action, "edit src/f.rs");
  assert.equal(group.rows[1].running, false); // -> .g-idle; the glyph is still ●
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
test("turns: WITHOUT a resolver (Focus), labels stay you/wcode", () => {
  const t = turns([
    { kind: "user", html: "hi", live: false },
    { kind: "assistant", html: "", live: false },
  ]);
  assert.equal(t[0].who.name, "you");
  assert.equal(t[1].who.name, "wcode");
  assert.equal(t[1].who.avatar, "❯");
});

test("turns: WITH a resolver (All), a non-user turn is labeled by its origin member", () => {
  const blocks: RenderedBlock[] = [
    { kind: "assistant", html: "", live: false, origin: "agent:w1" },
    { kind: "tool", html: "", live: false, origin: "agent:w1" },
  ];
  const t = turns(blocks, (origin) => (origin === "agent:w1" ? { name: "explorer", isRoot: false, swatch: "green" } : undefined));
  assert.equal(t.length, 1, "one contiguous member run");
  assert.equal(t[0].who.name, "explorer");
  assert.equal(t[0].who.avatar, "E");
  assert.equal(t[0].who.className, "who wcode sw-green", "the turn carries the member swatch");
  assert.equal(t[0].swatch, "green");

  // A1/R1: a ROOT origin keeps the `❯` avatar (the resolver carries `isRoot`).
  const rootTurn = turns(
    [{ kind: "assistant", html: "", live: false, origin: "root-1" }],
    (o) => (o === "root-1" ? { name: "orchestrator", isRoot: true, swatch: "blue" } : undefined),
  );
  assert.equal(rootTurn[0].who.avatar, "❯");
  assert.equal(rootTurn[0].who.name, "orchestrator");
});

test("turns: an ORIGIN change starts a NEW turn (mixed-origin adjacency)", () => {
  const blocks: RenderedBlock[] = [
    { kind: "assistant", html: "", live: false, origin: "agent:w1" },
    { kind: "assistant", html: "", live: false, origin: "agent:w2" }, // NO user text between
  ];
  const members: Record<string, { name: string; isRoot: boolean; swatch: "green" | "orange" }> = {
    "agent:w1": { name: "explorer", isRoot: false, swatch: "green" },
    "agent:w2": { name: "developer", isRoot: false, swatch: "orange" },
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
      ? { name: "orchestrator", isRoot: true, swatch: "blue" as const }
      : id === "agent:explorer"
        ? { name: "explorer", isRoot: false, swatch: "green" as const }
        : undefined;
  const t = turns(blocks, resolve);
  assert.equal(t.length, 3, "the peer message is its own turn");
  assert.equal(t[1].who.name, "explorer", "labeled by the SENDER, never 'you'");
  assert.equal(t[1].who.avatar, "E");
  assert.equal(t[1].who.className, "who wcode sw-green");
  assert.equal(t[1].swatch, "green");
  assert.equal(t[1].blocks[0].kind, "peer");
});

test("turns: the Focus fallback swatch tints a single member's turns", () => {
  const t = turns([{ kind: "assistant", html: "", live: false }], undefined, "orange");
  assert.equal(t[0].who.className, "who wcode sw-orange");
  assert.equal(t[0].swatch, "orange");
  assert.equal(t[0].who.name, "wcode", "Focus still labels the assistant turn 'wcode'");
});
