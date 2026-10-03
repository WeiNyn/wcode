import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent, AgentMessage, TodoItem } from "../src/protocol.ts";
import {
  HydratedSet,
  initialState,
  reduce,
  seedFromHistory,
  sidebarTree,
  todoBadge,
  todoIconSpec,
  todoViews,
  transcriptOf,
  type MemberNode,
  type SessionMember,
  type TodoNode,
  type ViewState,
} from "../src/reducer.ts";

const fixturesDir = resolve(dirname(fileURLToPath(import.meta.url)), "fixtures");

function load(name: string): AgentEvent[] {
  return readFileSync(resolve(fixturesDir, name), "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => JSON.parse(line) as AgentEvent);
}

/** Drive a fixture into ONE session (the frame's `session` — "root" here). */
function drive(name: string, session = "root"): ViewState {
  let state = initialState();
  for (const event of load(name)) state = reduce(state, event, session);
  return state;
}

test("submit round-trips to a committed assistant block", () => {
  const state = drive("submit_stream.handwritten.ndjson");
  const blocks = transcriptOf(state, "root");
  assert.equal(blocks.length, 1);
  const block = blocks[0];
  assert.equal(block.kind, "assistant");
  assert.equal(block.live, false, "message_end commits the block");
  assert.deepEqual(block.content, [{ type: "text", text: "Hello there" }]);
  assert.equal(state.status.running, false);
  assert.equal(state.status.stopReason, "stop");
  assert.equal(state.status.contextUsed, 12, "turn_end reads input_tokens");
});

test("message_update REPLACES the live content, never appends", () => {
  let state = initialState();
  state = reduce(state, {
    type: "message_start",
    message: { role: "assistant", content: [{ type: "text", text: "" }], stop_reason: "stop" },
  });
  state = reduce(state, {
    type: "message_update",
    message: { role: "assistant", content: [{ type: "text", text: "Hello" }], stop_reason: "stop" },
  });
  state = reduce(state, {
    type: "message_update",
    message: { role: "assistant", content: [{ type: "text", text: "Hello there" }], stop_reason: "stop" },
  });
  const block = transcriptOf(state, "")[0];
  assert.equal(block.live, true);
  assert.deepEqual(
    block.content,
    [{ type: "text", text: "Hello there" }],
    "the second update replaced, it did not concatenate",
  );
});

test("message_start ignores a user echo", () => {
  let state = initialState();
  state = reduce(state, {
    type: "message_start",
    message: { role: "user", content: [{ type: "text", text: "hi" }] },
  });
  assert.equal(transcriptOf(state, "").length, 0);
});

test("tool_execution_update appends; end replaces a non-empty output", () => {
  const state = drive("tool_execution.handwritten.ndjson");
  const tools = transcriptOf(state, "root").filter((b) => b.kind === "tool");
  assert.equal(tools.length, 2);

  const bash = tools[0].tool;
  assert.equal(bash?.name, "bash");
  assert.equal(bash?.done, true);
  assert.equal(bash?.output, "line1\nline2\nline3\n", "end's non-empty output replaced the partials");
  assert.equal(bash?.durationMs, 42);

  const edit = tools[1].tool;
  assert.equal(edit?.name, "edit");
  assert.equal(edit?.output, "", "an empty end-output keeps the (empty) streamed partial");
  assert.equal(edit?.diff, "@@ -1 +1 @@\n-a\n+b\n");
  assert.equal(edit?.path, "src/f.rs");
});

test("an unknown event type is ignored, not thrown", () => {
  let state = initialState();
  assert.doesNotThrow(() => {
    for (const event of load("unknown_event.handwritten.ndjson")) state = reduce(state, event, "root");
  });
  // The unknown frame changed nothing; the following agent_end only flips running.
  assert.equal(transcriptOf(state, "root").length, 0);
  assert.equal(state.status.running, false);
});

test("sessions folds the roster, root first; spawned appends", () => {
  const state = drive("sessions_roster.handwritten.ndjson", "root-1");
  assert.equal(state.members.length, 3);
  assert.equal(state.members[0].id, "root-1");
  assert.equal(state.members[0].isRoot, true);
  assert.equal(state.members[0].model, "m1");
  assert.equal(state.members[1].id, "agent:w1");
  assert.equal(state.members[1].label, "w1", "the agent: prefix is stripped for the label");
  assert.equal(state.members[1].isRoot, false);
  assert.equal(state.members[2].id, "agent:w2");
});

test("history seeds the transcript (user + assistant) and the context size", () => {
  let state = initialState();
  state = seedFromHistory(
    state,
    [
      { role: "user", content: [{ type: "text", text: "hello" }] },
      {
        role: "assistant",
        content: [{ type: "text", text: "hi" }],
        stop_reason: "stop",
        usage: { input_tokens: 7, output_tokens: 2 },
      },
    ],
    "root",
  );
  const blocks = transcriptOf(state, "root");
  assert.equal(blocks.length, 2);
  assert.equal(blocks[0].kind, "user");
  assert.equal(blocks[1].kind, "assistant");
  assert.equal(state.status.contextUsed, 7);
});

test("a run failure is sticky until the next agent_start", () => {
  let state = initialState();
  state = reduce(state, { type: "agent_start" });
  state = reduce(state, { type: "error", message: "boom" });
  assert.equal(state.status.running, true);
  assert.equal(state.status.lastError, "boom");
  state = reduce(state, { type: "agent_start" });
  assert.equal(state.status.lastError, undefined, "a new run clears the failure");
});

test("the CAPTURED list_sessions fixture folds the roster, root first", () => {
  const state = drive("list_sessions.ndjson");
  assert.ok(state.members.length >= 1, "the seeded push named at least the root");
  assert.equal(state.members[0].isRoot, true);
  assert.equal(state.members.filter((m) => m.isRoot).length, 1, "exactly one root");
  // The captured history reply was empty, so nothing was seeded.
  assert.equal(transcriptOf(state, state.members[0].id).length, 0);
});

/* ------------------------------------------------------ hydration (P3 fix) */

const ASSISTANT_HI: AgentMessage = {
  role: "assistant",
  content: [{ type: "text", text: "HI" }],
  stop_reason: "stop",
};

test("history REPLACES the committed blocks — a focused member is never duplicated", () => {
  // The sidebar's core use: focus a member that has ALREADY run. Its events were
  // folded by `frame.session` as they streamed, so its block is already here, and
  // the `GetHistory` reply then carries the SAME message. Appending duplicates it.
  let state = initialState();
  state = reduce(state, { type: "message_start", message: ASSISTANT_HI }, "agent:w1");
  state = reduce(state, { type: "message_end", message: ASSISTANT_HI }, "agent:w1");
  assert.equal(transcriptOf(state, "agent:w1").length, 1, "the streamed block is committed");

  state = seedFromHistory(state, [ASSISTANT_HI], "agent:w1");
  const blocks = transcriptOf(state, "agent:w1");
  assert.equal(blocks.length, 1, "the history REPLACED it, it did not append a copy");
  assert.deepEqual(blocks[0].content, [{ type: "text", text: "HI" }]);
});

test("history keeps an in-flight block, and puts it LAST", () => {
  let state = initialState();
  state = reduce(state, { type: "message_start", message: ASSISTANT_HI }, "agent:w1");
  assert.equal(transcriptOf(state, "agent:w1")[0].live, true, "still streaming");

  state = seedFromHistory(state, [{ role: "user", content: [{ type: "text", text: "hello" }] }], "agent:w1");
  const blocks = transcriptOf(state, "agent:w1");
  assert.equal(blocks.length, 2, "history + the live block");
  assert.equal(blocks[0].kind, "user", "history first");
  assert.equal(blocks[1].live, true, "the live block LAST (never before the history)");
});

test("HydratedSet: a session is hydrated ONCE (a second retarget must not re-ask)", () => {
  const hydrated = new HydratedSet();
  assert.equal(hydrated.claim("agent:w1"), true, "the first focus asks");
  assert.equal(hydrated.claim("agent:w1"), false, "a SECOND focus must not ask again");
  assert.equal(hydrated.claim("agent:w2"), true, "a different member asks once");
  assert.equal(hydrated.size, 2);

  hydrated.release("agent:w2");
  assert.equal(hydrated.claim("agent:w2"), true, "a released claim (the send failed) can retry");

  hydrated.reset();
  assert.equal(hydrated.size, 0);
  assert.equal(hydrated.claim("agent:w1"), true, "a restart re-asks");
});
/* ------------------------------------------------------- the sidebar tree */

test("todoViews maps each todo to a {label,status} row", () => {
  const todos: TodoItem[] = [
    { content: "a", status: "completed" },
    { content: "b", status: "in_progress" },
    { content: "c", status: "pending" },
  ];
  assert.deepEqual(todoViews(todos), [
    { label: "a", status: "completed" },
    { label: "b", status: "in_progress" },
    { label: "c", status: "pending" },
  ]);
});

test("todoIconSpec names a codicon + color per status", () => {
  assert.deepEqual(todoIconSpec("completed"), { icon: "check", color: "charts.green" });
  assert.deepEqual(todoIconSpec("in_progress"), { icon: "play", color: "charts.blue" });
  assert.deepEqual(todoIconSpec("pending"), { icon: "circle-outline", color: "descriptionForeground" });
});

test("sidebarTree: a Team root (when members exist) + a Tasks root (only when non-empty)", () => {
  const members: SessionMember[] = [
    { id: "root-1", label: "root-1", state: "running", isRoot: true, model: "opus" },
    { id: "agent:w1", label: "w1", state: "idle", isRoot: false },
  ];

  // B1: NO members and NO todos -> `[]`, so the `viewsWelcome` empty state renders.
  assert.deepEqual(sidebarTree([], []), []);

  const teamOnly = sidebarTree(members, []);
  assert.equal(teamOnly.length, 1, "no Tasks root when there are no todos");
  assert.equal(teamOnly[0].id, "section:team");
  assert.equal(teamOnly[0].label, "Team");
  assert.equal(teamOnly[0].description, undefined, "the Team root has no badge");
  assert.equal(teamOnly[0].children.length, 2, "memberViews are the Team rows");
  const teamChild0 = teamOnly[0].children[0] as MemberNode;
  assert.equal(teamChild0.row.label, "orchestrator", "the root reads as orchestrator");

  const withTodos = sidebarTree(members, [
    { content: "x", status: "completed" },
    { content: "y", status: "pending" },
  ]);
  assert.equal(withTodos.length, 2);
  assert.equal(withTodos[1].id, "section:tasks");
  assert.equal(withTodos[1].label, "Tasks");
  assert.equal(withTodos[1].description, "☑ 1/2", "the Tasks badge");
  assert.equal(withTodos[1].children.length, 2);
  const taskChild0 = withTodos[1].children[0] as TodoNode;
  assert.equal(taskChild0.id, "section:tasks:0");
  assert.equal(taskChild0.row.label, "x");
});

test("sidebarTree: the Tasks root appears alone when there are no members", () => {
  const tasksOnly = sidebarTree([], [{ content: "t", status: "in_progress" }]);
  assert.equal(tasksOnly.length, 1, "no Team root when there are no members");
  assert.equal(tasksOnly[0].id, "section:tasks");
  assert.equal(tasksOnly[0].children.length, 1);
});

test("todoBadge counts completed over total", () => {
  assert.equal(todoBadge([]), "☑ 0/0");
  assert.equal(
    todoBadge([
      { content: "a", status: "completed" },
      { content: "b", status: "pending" },
    ]),
    "☑ 1/2",
  );
});
