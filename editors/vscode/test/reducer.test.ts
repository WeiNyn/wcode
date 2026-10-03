import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent } from "../src/protocol.ts";
import { initialState, reduce, seedFromHistory, type ViewState } from "../src/reducer.ts";

const fixturesDir = resolve(dirname(fileURLToPath(import.meta.url)), "fixtures");

function load(name: string): AgentEvent[] {
  return readFileSync(resolve(fixturesDir, name), "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => JSON.parse(line) as AgentEvent);
}

function drive(name: string): ViewState {
  let state = initialState();
  for (const event of load(name)) state = reduce(state, event);
  return state;
}

test("submit round-trips to a committed assistant block", () => {
  const state = drive("submit_stream.handwritten.ndjson");
  assert.equal(state.transcript.length, 1);
  const block = state.transcript[0];
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
  const block = state.transcript[0];
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
  assert.equal(state.transcript.length, 0);
});

test("tool_execution_update appends; end replaces a non-empty output", () => {
  const state = drive("tool_execution.handwritten.ndjson");
  const tools = state.transcript.filter((b) => b.kind === "tool");
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
  const before = initialState();
  let state = before;
  assert.doesNotThrow(() => {
    for (const event of load("unknown_event.handwritten.ndjson")) state = reduce(state, event);
  });
  // The unknown frame changed nothing; the following agent_end only flips running.
  assert.equal(state.transcript.length, 0);
  assert.equal(state.status.running, false);
});

test("sessions folds the roster, root first; spawned appends", () => {
  const state = drive("sessions_roster.handwritten.ndjson");
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
  state = seedFromHistory(state, [
    { role: "user", content: [{ type: "text", text: "hello" }] },
    {
      role: "assistant",
      content: [{ type: "text", text: "hi" }],
      stop_reason: "stop",
      usage: { input_tokens: 7, output_tokens: 2 },
    },
  ]);
  assert.equal(state.transcript.length, 2);
  assert.equal(state.transcript[0].kind, "user");
  assert.equal(state.transcript[1].kind, "assistant");
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
  assert.equal(state.transcript.length, 0);
});
