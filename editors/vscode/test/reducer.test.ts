import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent, AgentMessage, ContentBlock, TodoItem } from "../src/protocol.ts";
import {
  HydratedSet,
  appendUser,
  initialState,
  memberGlyph,
  parseInbound,
  reduce,
  seedFromHistory,
  sidebarRails,
  todoBadge,
  todoGlyph,
  todoViews,
  transcriptOf,
  type SessionMember,
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
/* ------------------------------------------------------------------- rail */

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

test("sidebarRails: team rows + the tasks badge/rows", () => {
  const members: SessionMember[] = [
    { id: "root-1", label: "root-1", state: "running", isRoot: true, model: "opus" },
    { id: "agent:w1", label: "w1", state: "idle", isRoot: false },
  ];
  const todos: TodoItem[] = [
    { content: "a", status: "completed" },
    { content: "b", status: "pending" },
  ];
  const rails = sidebarRails(members, todos);
  assert.equal(rails.team.length, 2);
  assert.equal(rails.team[0].label, "orchestrator", "the root reads as orchestrator");
  assert.equal(rails.team[0].model, "opus");
  assert.equal(rails.tasks.badge, "☑ 1/2");
  assert.deepEqual(rails.tasks.rows, [
    { label: "a", status: "completed" },
    { label: "b", status: "pending" },
  ]);
  assert.deepEqual(sidebarRails([], []).tasks, { badge: "☑ 0/0", rows: [] });
});

test("todoGlyph maps each status to the draft's box + row class", () => {
  assert.deepEqual(todoGlyph("completed"), { glyph: "☑", className: "done" });
  assert.deepEqual(todoGlyph("in_progress"), { glyph: "▸", className: "doing" });
  assert.deepEqual(todoGlyph("pending"), { glyph: "☐", className: "" });
});

test("memberGlyph maps each MemberState to the TUI glyph + class", () => {
  assert.deepEqual(memberGlyph("running"), { glyph: "⠋", className: "g-run" });
  assert.deepEqual(memberGlyph("done"), { glyph: "✓", className: "g-done" });
  assert.deepEqual(memberGlyph("failed"), { glyph: "✗", className: "g-err" });
  assert.deepEqual(memberGlyph("idle"), { glyph: "○", className: "g-idle" });
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

/* ------------------------------------------------- inbound tags (history + live) */

test("parseInbound splits the harness tag off an inbound message", () => {
  assert.deepEqual(parseInbound("[message from user]\nhello"), { from: "user", body: "hello" });
  assert.deepEqual(parseInbound("[message from agent:explorer]\nplease review"), {
    from: "agent:explorer",
    body: "please review",
  });
  assert.deepEqual(parseInbound("[message from bg]\nbg1 exited (0)"), { from: "bg", body: "bg1 exited (0)" });
  // A body with newlines keeps everything after the tag's own newline.
  assert.deepEqual(parseInbound("[message from agent:w1]\nline1\nline2"), { from: "agent:w1", body: "line1\nline2" });
});

test("parseInbound returns null when there is no tag (a locally typed message)", () => {
  assert.equal(parseInbound("hello there"), null);
  assert.equal(parseInbound(""), null);
  // A tag with no newline is not the tag — the harness always emits `]\n`.
  assert.equal(parseInbound("[message from user] hello"), null);
});

test("history: a peer's inbound message hydrates as a `peer` block, not the user's", () => {
  let state = initialState();
  state = seedFromHistory(
    state,
    [
      { role: "user", content: [{ type: "text", text: "[message from user]\nreview the diff" }] },
      { role: "user", content: [{ type: "text", text: "[message from agent:explorer]\nfound 3 issues" }] },
      { role: "user", content: [{ type: "text", text: "[message from bg]\nbg1 exited (0)" }] },
    ],
    "root",
  );
  const blocks = transcriptOf(state, "root");
  assert.equal(blocks[0].kind, "user", "the human's own words stay a user block");
  assert.equal(blocks[0].text, "review the diff", "the tag is stripped for display");
  assert.equal(blocks[1].kind, "peer");
  assert.equal(blocks[1].from, "agent:explorer");
  assert.equal(blocks[1].text, "found 3 issues");
  assert.equal(blocks[2].kind, "notice", "a non-peer sender (bg) is a notice");
});

test("history: an untagged user message is untouched", () => {
  let state = initialState();
  state = seedFromHistory(state, [{ role: "user", content: [{ type: "text", text: "plain hello" }] }], "root");
  assert.equal(transcriptOf(state, "root")[0].text, "plain hello");
});

test("seq: appended blocks get a monotonic arrival ordinal across sessions", () => {
  let state = initialState();
  state = appendUser(state, "a", "root");
  state = appendUser(state, "b", "agent:w1");
  state = appendUser(state, "c", "root");
  assert.deepEqual(
    [transcriptOf(state, "root")[0].seq, transcriptOf(state, "agent:w1")[0].seq, transcriptOf(state, "root")[1].seq],
    [0, 1, 2],
  );
});

test("seq: hydrated history sorts BEFORE a retained live block", () => {
  let state = initialState();
  // A live assistant block streams first (seq 0), then the history arrives.
  state = reduce(state, { type: "message_start", message: { role: "assistant", content: [{ type: "text", text: "…" }], stop_reason: "stop" } }, "root");
  state = seedFromHistory(state, [{ role: "user", content: [{ type: "text", text: "old" }] }], "root");
  const blocks = transcriptOf(state, "root");
  assert.equal(blocks[0].kind, "user", "the history leads");
  assert.equal(blocks[1].live, true, "the in-flight block trails");
  assert.ok((blocks[0].seq ?? 0) < (blocks[1].seq ?? 0), "and its seq sorts first");
});

test("seq: EVERY appended block carries one — including a tool call", () => {
  // Regression: `tool_execution_start` once bypassed `pushBlocks`, so its block had no
  // seq and the merged view sorted it to the FRONT (the tool rendered a turn early).
  let state = initialState();
  const assistant = (content: ContentBlock[]): AgentMessage =>
    ({ role: "assistant", content, stop_reason: "stop" });
  state = appendUser(state, "do it", "root");
  state = reduce(state, { type: "message_start", message: assistant([]) }, "root");
  state = reduce(
    state,
    { type: "message_update", message: assistant([{ type: "tool_call", id: "c1", name: "read", arguments: {} }]) },
    "root",
  );
  state = reduce(
    state,
    {
      type: "message_end",
      message: assistant([{ type: "tool_call", id: "c1", name: "read", arguments: {} }]),
    },
    "root",
  );
  state = reduce(state, { type: "tool_execution_start", call_id: "c1", name: "read" }, "root");

  const blocks = transcriptOf(state, "root");
  assert.deepEqual(
    blocks.map((b) => b.kind),
    ["user", "assistant", "tool"],
  );
  assert.deepEqual(
    blocks.map((b) => b.seq),
    [0, 1, 2],
    "a tool block gets its arrival ordinal like any other",
  );
  assert.ok(blocks.every((b) => typeof b.seq === "number"), "no block may be left without one");
});
