import assert from "node:assert/strict";
import { test } from "node:test";

import { initialState, reduce, type ViewState } from "../src/reducer.ts";
import { renderState, type RenderedBlock, type RenderedState } from "../src/render.ts";
import { parseToWebview, type PanelSessionInfo, type ToWebview } from "../src/webview.ts";
import { classNames, emptyKind, stateLabel, statusSegments, toggleExpanded } from "../src/webview/view.ts";

const idle = (): RenderedState => renderState(initialState());

const session = (over: Partial<PanelSessionInfo> = {}): PanelSessionInfo => ({
  id: "root-1",
  state: "ready",
  stderrTail: "",
  ...over,
});

/* ------------------------------------------------------------------- status */

test("stateLabel names every FSM state", () => {
  assert.equal(stateLabel("ready"), "ready");
  assert.equal(stateLabel("starting"), "starting");
  assert.equal(stateLabel("crashed"), "crashed");
  assert.equal(stateLabel("stopped"), "stopped");
});

test("statusSegments: dot, state, separator, session id", () => {
  const segments = statusSegments(idle(), session());
  assert.deepEqual(
    segments.map((s) => s.className),
    ["dot dot-ready", "status-state", "status-sep", "status-session"],
  );
  assert.deepEqual(
    segments.map((s) => s.text),
    ["", "ready", "·", "root-1"],
  );
});

test("statusSegments: an unknown session id reads as 'no session yet'", () => {
  const segments = statusSegments(idle(), session({ id: null }));
  assert.equal(segments[3].text, "no session yet");
});

test("statusSegments appends running, context and the last error", () => {
  const running = renderState(reduce(initialState(), { type: "agent_start" }));
  const withCtx = renderState(
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
  const withError = renderState(
    reduce(reduce(initialState(), { type: "agent_start" }), { type: "error", message: "boom" }),
  );

  assert.ok(statusSegments(running, session()).some((s) => s.className === "status-running"));
  assert.ok(statusSegments(withCtx, session()).some((s) => s.text === "ctx 42"));
  assert.ok(statusSegments(withError, session()).some((s) => s.text === "boom"));
  assert.ok(!statusSegments(idle(), session()).some((s) => s.className === "status-running"));
});

/* -------------------------------------------------------------- the target */

test("the status strip names the target, so a retarget is visible", () => {
  // The worst failure mode of a re-targeting panel is an invisible target: the
  // user cannot tell whose transcript they are reading.
  const state: ViewState = {
    ...initialState(),
    members: [
      { id: "root-1", label: "root-1", state: "idle", isRoot: true },
      { id: "agent:w1", label: "w1", state: "running", isRoot: false, liveAction: "edit src/f.rs" },
    ],
    targeted: "root-1",
  };

  const root = renderState(state, "root-1");
  const member = renderState(state, "agent:w1");
  assert.equal(root.target?.label, "orchestrator", "the root reads as orchestrator");
  assert.equal(member.target?.label, "w1");

  const rootSegments = statusSegments(root, session());
  const memberSegments = statusSegments(member, session());
  assert.ok(rootSegments.some((s) => s.className === "status-target" && s.text === "orchestrator"));
  assert.ok(memberSegments.some((s) => s.className === "status-target" && s.text === "w1"));
  assert.ok(!memberSegments.some((s) => s.text === "orchestrator"), "the target segment CHANGED");

  // `running` is per-target (the roster's MemberState), so the member shows it
  // and the idle root does not.
  assert.ok(memberSegments.some((s) => s.className === "status-running"));
  assert.ok(!rootSegments.some((s) => s.className === "status-running"));

  console.log("status strip (headless):");
  for (const [name, view] of [["root-1", root], ["agent:w1", member]] as const) {
    const text = statusSegments(view, session()).map((s) => s.text).join(" ");
    console.log(`  target ${name.padEnd(9)} -> ${text}`);
  }
});

test("the plan chip appears only when plan mode is on", () => {
  const off = renderState(initialState(), null);
  assert.ok(!statusSegments(off, session()).some((s) => s.className === "status-plan"));
  const on = renderState({ ...initialState(), status: { running: false, planMode: true } }, null);
  assert.ok(statusSegments(on, session()).some((s) => s.className === "status-plan" && s.text === "plan"));
});

/* -------------------------------------------------------------- empty states */

test("emptyKind picks the deliberate empty state", () => {
  assert.equal(emptyKind(idle(), session({ state: "crashed" })), "crashed");
  assert.equal(emptyKind(idle(), session({ state: "starting" })), "starting");
  assert.equal(emptyKind(idle(), session({ state: "stopped" })), "stopped");
  assert.equal(emptyKind(renderState(reduce(initialState(), { type: "agent_start" })), session()), "working");
  assert.equal(emptyKind(idle(), session()), "idle");
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

test("classNames composes the block classes", () => {
  const tool = (isError: boolean): RenderedBlock => ({
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
      hasDiff: true,
    },
  });
  assert.equal(classNames(tool(false)), "block tool");
  assert.equal(classNames(tool(false), true), "block tool expanded");
  assert.equal(classNames(tool(true)), "block tool error");
  assert.equal(classNames({ kind: "assistant", html: "", live: true }), "block assistant live");
  assert.equal(classNames({ kind: "assistant", html: "", live: false }), "block assistant");
  assert.equal(classNames({ kind: "user", html: "", live: false }), "block user");
  assert.equal(classNames({ kind: "notice", html: "", live: false }), "block notice");
});

/* --------------------------------------------------------- parseToWebview */

test("parseToWebview accepts a well-formed snapshot", () => {
  const message: ToWebview = { kind: "state", state: idle(), session: session() };
  const parsed = parseToWebview(message);
  assert.ok(parsed);
  assert.equal(parsed.kind, "state");
  assert.equal(parsed.session.id, "root-1");
  assert.equal(parsed.session.state, "ready");
  assert.deepEqual(parsed.state.blocks, []);
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
