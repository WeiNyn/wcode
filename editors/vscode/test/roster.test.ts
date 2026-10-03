import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent } from "../src/protocol.ts";
import {
  actionLabel,
  callTarget,
  initialState,
  memberIconSpec,
  memberViews,
  planModePending,
  reduce,
  target,
  targetLabel,
  todoBadge,
  transcriptOf,
  type ViewState,
} from "../src/reducer.ts";

const fixturesDir = resolve(dirname(fileURLToPath(import.meta.url)), "fixtures");

function load(name: string): AgentEvent[] {
  return readFileSync(resolve(fixturesDir, name), "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => JSON.parse(line) as AgentEvent);
}

function drive(name: string, session = ""): ViewState {
  let state = initialState();
  for (const event of load(name)) state = reduce(state, event, session);
  return state;
}

/* ------------------------------------------------------------------ the roster */

test("folds a `sessions` push into members, root first", () => {
  const state = drive("sessions_roster.handwritten.ndjson", "root-1");
  assert.deepEqual(
    state.members.map((m) => m.id),
    ["root-1", "agent:w1", "agent:w2"],
  );
});

test("appends a `spawned` worker without disturbing the roster", () => {
  // The committed fixture already covers growth: a `sessions` push + a `spawned`.
  const state = drive("sessions_roster.handwritten.ndjson", "root-1");
  assert.equal(state.members[1].id, "agent:w1", "the pushed member kept its place");
  assert.equal(state.members[2].id, "agent:w2", "the spawned worker appended");
  assert.equal(state.members[2].state, "idle");
});

test("marks exactly the first member root", () => {
  const state = drive("sessions_roster.handwritten.ndjson", "root-1");
  assert.equal(state.members.filter((m) => m.isRoot).length, 1);
  assert.equal(state.members[0].isRoot, true);
});

test("a `sessions` push preserves the derived liveAction", () => {
  let state = drive("sessions_roster.handwritten.ndjson", "root-1");
  state = reduce(state, { type: "tool_execution_start", call_id: "t1", name: "bash" }, "agent:w1");
  assert.equal(state.members[1].liveAction, "bash");
  // A later roster push must not drop it (the push carries no action field).
  state = reduce(state, { type: "sessions", sessions: [{ id: "root-1", state: "idle" }, { id: "agent:w1", state: "running" }] }, "root-1");
  assert.equal(state.members[1].liveAction, "bash");
  assert.equal(state.members[1].state, "running", "the push still updates liveness");
});

/* --------------------------------------------------------------- liveAction */

test("liveAction is set on tool_execution_start and CLEARED on agent_end", () => {
  let state = drive("sessions_roster.handwritten.ndjson", "root-1");
  const w1 = (): ViewState["members"][number] | undefined => state.members.find((m) => m.id === "agent:w1");

  state = reduce(state, { type: "tool_execution_start", call_id: "t1", name: "bash" }, "agent:w1");
  assert.equal(w1()?.liveAction, "bash");
  assert.equal(
    state.members.find((m) => m.id === "root-1")?.liveAction,
    undefined,
    "only the emitting member gets the action",
  );

  state = reduce(state, { type: "agent_end" }, "agent:w1");
  assert.equal(w1()?.liveAction, undefined, "a finished member never shows a stale action");
});

test("the action label names the call's target argument", () => {
  const blocks = [
    {
      kind: "assistant" as const,
      content: [
        { type: "tool_call" as const, id: "t1", name: "edit", arguments: { path: "src/f.rs" } },
        { type: "tool_call" as const, id: "t2", name: "bash", arguments: { command: "ls -la" } },
        { type: "tool_call" as const, id: "t3", name: "read", arguments: {} },
      ],
    },
  ];
  assert.equal(callTarget(blocks, "t1"), "src/f.rs");
  assert.equal(actionLabel(blocks, "t1", "edit"), "edit src/f.rs");
  assert.equal(actionLabel(blocks, "t2", "bash"), "bash ls -la");
  assert.equal(actionLabel(blocks, "t3", "read"), "read", "no recognizable argument falls back to the name");
  assert.equal(actionLabel(blocks, "missing", "edit"), "edit");
});

/* -------------------------------------------------------------------- todos */

test("`todo` is scoped to the emitting session and adds NO transcript notice", () => {
  let state = drive("sessions_roster.handwritten.ndjson", "root-1");
  state = reduce(state, { type: "todo", todos: [{ content: "a", status: "pending" }] }, "agent:w1");
  state = reduce(state, { type: "todo", todos: [{ content: "b", status: "completed" }] }, "root-1");

  assert.deepEqual(state.todos["agent:w1"], [{ content: "a", status: "pending" }]);
  assert.deepEqual(state.todos["root-1"], [{ content: "b", status: "completed" }]);
  assert.equal(
    transcriptOf(state, "agent:w1").length,
    0,
    "the sidebar is authoritative — no notice per update",
  );
});

test("todoBadge counts the completed entries", () => {
  assert.equal(
    todoBadge([
      { content: "a", status: "completed" },
      { content: "b", status: "in_progress" },
      { content: "c", status: "completed" },
    ]),
    "☑ 2/3",
  );
  assert.equal(todoBadge([]), "☑ 0/0");
});

/* ------------------------------------------------------------ message traffic */

test("a `message_received` lands in the RECEIVING session's transcript, with its sender", () => {
  let state = drive("sessions_roster.handwritten.ndjson", "root-1");
  state = reduce(state, { type: "message_received", from: "agent:w1", content: "ping" }, "agent:w2");

  const received = transcriptOf(state, "agent:w2");
  assert.equal(received.length, 1);
  assert.equal(received[0].kind, "notice");
  assert.equal(received[0].from, "agent:w1", "the sender rides the block (the webview renders it)");
  assert.equal(received[0].text, "ping");
  assert.equal(transcriptOf(state, "agent:w1").length, 0, "not the sender's transcript");
  assert.equal(transcriptOf(state, "root-1").length, 0);
});

test("per-session transcripts do not bleed into each other", () => {
  let state = initialState();
  state = reduce(
    state,
    { type: "message_start", message: { role: "assistant", content: [{ type: "text", text: "a" }], stop_reason: "stop" } },
    "agent:w1",
  );
  state = reduce(
    state,
    { type: "message_start", message: { role: "assistant", content: [{ type: "text", text: "b" }], stop_reason: "stop" } },
    "agent:w2",
  );
  assert.equal(transcriptOf(state, "agent:w1").length, 1);
  assert.equal(transcriptOf(state, "agent:w2").length, 1);
  assert.deepEqual(transcriptOf(state, "agent:w1")[0].content, [{ type: "text", text: "a" }]);
  assert.deepEqual(transcriptOf(state, "agent:w2")[0].content, [{ type: "text", text: "b" }]);
});

/* ---------------------------------------------------------------- plan mode */

test("plan-mode: optimistic set, settle on `ack`, revert on `error`", () => {
  const settled = planModePending(initialState(), true);
  assert.equal(settled.status.planMode, true);
  assert.equal(settled.status.planModePrev, false, "the value to revert to is remembered");

  const acked = reduce(settled, { type: "ack" });
  assert.equal(acked.status.planMode, true, "SetPlanMode is infallible: Ack settles it");
  assert.equal(acked.status.planModePrev, undefined);

  const reverted = reduce(settled, { type: "error", message: "nope" });
  assert.equal(reverted.status.planMode, false, "an errored reply reverts the optimistic flip");
  assert.equal(reverted.status.planModePrev, undefined);
});

test("an `ack` with nothing pending changes nothing", () => {
  const state = initialState();
  assert.equal(reduce(state, { type: "ack" }), state);
});

/* --------------------------------------------------------------- the target */

test("target/targetLabel: the root reads as 'orchestrator'", () => {
  let state = drive("sessions_roster.handwritten.ndjson", "root-1");
  assert.equal(state.targeted, null, "nothing is targeted until the host picks one");

  state = target(state, "agent:w1");
  assert.equal(state.targeted, "agent:w1");
  assert.equal(targetLabel(state, "agent:w1"), "w1");
  assert.equal(targetLabel(state, "root-1"), "orchestrator");
  assert.equal(targetLabel(state, "agent:nope"), "nope", "an unknown id falls back to its short name");

  assert.equal(target(state, "agent:w1"), state, "setting the same target is a no-op");
});

/* ------------------------------------------------------- the tree's pure half */

test("memberViews/memberIconSpec map each MemberState", () => {
  const views = memberViews([
    { id: "root-1", label: "root-1", model: "m1", state: "idle", isRoot: true },
    { id: "agent:w1", label: "w1", state: "running", isRoot: false, liveAction: "edit src/f.rs" },
    { id: "agent:w2", label: "w2", state: "done", isRoot: false },
    { id: "agent:w3", label: "w3", state: "failed", isRoot: false },
  ]);
  assert.equal(views[0].label, "orchestrator", "the root's display label");
  assert.equal(views[1].label, "w1");
  assert.equal(views[1].liveAction, "edit src/f.rs");
  assert.equal(views[0].model, "m1");

  assert.deepEqual(memberIconSpec("running"), { icon: "circle-filled", color: "charts.green" });
  assert.deepEqual(memberIconSpec("idle"), { icon: "circle-outline", color: "descriptionForeground" });
  assert.deepEqual(memberIconSpec("done"), { icon: "check", color: "charts.blue" });
  assert.deepEqual(memberIconSpec("failed"), { icon: "error", color: "charts.red" });
});
