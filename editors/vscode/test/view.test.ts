import assert from "node:assert/strict";
import { test } from "node:test";

import { initialState, reduce, type SessionMember, type ViewState } from "../src/reducer.ts";
import { renderState, type RenderedBlock, type RenderedState } from "../src/render.ts";
import { parseToWebview, type PanelSessionInfo, type ToWebview } from "../src/webview.ts";
import {
  classNames,
  composerControls,
  composeSubmit,
  diffStat,
  emptyKind,
  emptySpec,
  panelHeader,
  selectionRef,
  stateLabel,
  toggleExpanded,
  turns,
  workingGroup,
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

test("panelHeader: dot, state, separator, session id", () => {
  const { r1 } = panelHeader(idle(), session());
  assert.deepEqual(
    r1.slice(0, 4).map((c) => c.className),
    ["dot dot-ready", "seg state", "sep", "seg mono"],
  );
  assert.deepEqual(
    r1.slice(0, 4).map((c) => c.text),
    ["", "ready", "·", "root-1"],
  );
});

test("panelHeader: an unknown session id reads as 'no session yet'", () => {
  const { r1 } = panelHeader(idle(), session({ id: null }));
  assert.ok(r1.some((c) => c.className === "seg mono" && c.text === "no session yet"));
});

test("panelHeader: r1 carries target, plan, running and error cells", () => {
  const state: ViewState = {
    ...initialState(),
    // `renderState` overwrites `running` PER-TARGET: the member's `state` is the
    // liveness fact. Input `running: false` below pins that the cell comes from it.
    members: [{ id: "root-1", label: "root-1", state: "running", isRoot: true, model: "sonnet" }],
    targeted: "root-1",
    status: { running: false, planMode: true, contextUsed: 42, lastError: "boom" },
  };
  const { r1 } = panelHeader(renderState(state, "root-1"), session());

  // The target must ALWAYS be visible — an invisible target is the worst
  // failure mode of a re-targeting panel (P3). P3 marks it interactive.
  assert.ok(
    r1.some((c) => c.className === "target" && c.interactive === true && c.text.includes("orchestrator")),
  );
  assert.ok(r1.some((c) => c.className === "pchip plan" && c.text === "plan"));
  assert.ok(r1.some((c) => c.className === "seg running" && c.text === "running…"));
  assert.ok(r1.some((c) => c.className === "seg error" && c.text === "boom"));
});

test("panelHeader: r2 carries the target's model, ctx and the member count", () => {
  const state: ViewState = {
    ...initialState(),
    members: [{ id: "root-1", label: "root-1", state: "idle", isRoot: true, model: "sonnet" }],
    targeted: "root-1",
    status: { running: false, planMode: false, contextUsed: 42 },
  };
  const { r2 } = panelHeader(renderState(state, "root-1"), session());

  // The model comes from the TARGETED member — `ViewStatus.model` is dead.
  assert.ok(r2.some((c) => c.text === "sonnet"));
  assert.ok(r2.some((c) => c.className === "seg ctx" && c.text === "ctx 42"));
  assert.ok(r2.some((c) => c.className === "spacer"));
  assert.ok(r2.some((c) => c.className === "seg" && c.text === "1 members"));
});

test("panelHeader: ctx is the numerator only — no gauge, no denominator", () => {
  // No context window is on the wire, so there is NO `▰▰▰▱▱ N / M` gauge.
  const state = renderState(
    reduce(reduce(initialState(), { type: "agent_start" }), {
      type: "turn_end",
      message: {
        role: "assistant",
        content: [{ type: "text", text: "x" }],
        stop_reason: "stop",
        usage: { input_tokens: 42, output_tokens: 1 },
      },
    }),
  );
  const { r1, r2 } = panelHeader(state, session());
  for (const cell of [...r1, ...r2]) {
    assert.ok(!cell.text.includes("▰"), `no gauge glyph in "${cell.text}"`);
    assert.ok(!cell.text.includes("/"), `no denominator in "${cell.text}"`);
  }
  assert.ok(r2.some((c) => c.className === "seg ctx" && c.text === "ctx 42"));
});

test("panelHeader: the plan chip appears only when plan mode is on", () => {
  const off = renderState(initialState(), null);
  assert.ok(!panelHeader(off, session()).r1.some((c) => c.className === "pchip plan"));
  const on = renderState({ ...initialState(), status: { running: false, planMode: true } }, null);
  assert.ok(panelHeader(on, session()).r1.some((c) => c.className === "pchip plan" && c.text === "plan"));
});

test("panelHeader: running is per-target (the roster's liveness fact)", () => {
  const state: ViewState = {
    ...initialState(),
    members: [
      { id: "root-1", label: "root-1", state: "idle", isRoot: true },
      { id: "agent:w1", label: "w1", state: "running", isRoot: false },
    ],
    targeted: "root-1",
  };
  const root = panelHeader(renderState(state, "root-1"), session());
  const member = panelHeader(renderState(state, "agent:w1"), session());

  assert.ok(!root.r1.some((c) => c.className === "seg running"));
  assert.ok(member.r1.some((c) => c.className === "seg running"));

  // The target cell CHANGED — assert the retarget is visible.
  assert.ok(
    root.r1.some((c) => c.className === "target" && c.interactive === true && c.text.includes("orchestrator")),
  );
  assert.ok(member.r1.some((c) => c.className === "target" && c.interactive === true && c.text.includes("w1")));
  assert.ok(!member.r1.some((c) => c.text.includes("orchestrator")));
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

test("toggleExpanded adds and removes, and never mutates the input set", () => {
  const original = new Set<string>();
  const opened = toggleExpanded(original, "t1");
  assert.deepEqual([...opened], ["t1"]);
  assert.equal(original.size, 0, "the input set is untouched");

  const closed = toggleExpanded(opened, "t1");
  assert.deepEqual([...closed], []);
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
  const message: ToWebview = { kind: "state", state: idle(), session: session(), context: null, verdicts: {} };
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
