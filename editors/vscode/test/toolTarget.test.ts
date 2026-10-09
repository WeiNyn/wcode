/**
 * FIX A — the tool row's TARGET; FIX B — the `thinking` fold rename.
 *
 * WHY A NEW FIXTURE: the existing `tool_execution.handwritten.ndjson` has NO
 * assistant `tool_call` before its `tool_execution_start`, so `callTarget` finds
 * nothing and the row can never show a target. `tool_target.handwritten.ndjson`
 * is hand-written so an assistant `message_end` carrying a `tool_call` PRECEDES the
 * `tool_execution_start` — the exact ordering the reducer's target stamp relies on
 * (the sibling `tool_call` was committed by the `message_end` arm before the
 * `tool_execution_start` arm runs).
 *
 * The note's target rule (target wins, else the summary, and NEVER an empty `.to`
 * span) is exercised below at the RENDER seam the webview reads —
 * `RenderedTool.target` / `.summary`. The webview DOM (`chat.ts` `fnNote`)
 * is not headless-testable (it calls `acquireVsCodeApi()` and touches `document`),
 * so it is NOT faked here: only the values it consumes are asserted.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent } from "../src/protocol.ts";
import { initialState, reduce, seedFromHistory, transcriptOf, type ViewState } from "../src/reducer.ts";
import { renderBlock, renderState, toolSummary } from "../src/render.ts";

const fixturesDir = resolve(dirname(fileURLToPath(import.meta.url)), "fixtures");

function drive(name: string, session = "root", state: ViewState = initialState()): ViewState {
  const events = readFileSync(resolve(fixturesDir, name), "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => JSON.parse(line) as AgentEvent);
  let next = state;
  for (const event of events) next = reduce(next, event, session);
  return next;
}

test("FIX A: the reducer stamps a tool block's target from the preceding tool_call", () => {
  const blocks = transcriptOf(drive("tool_target.handwritten.ndjson"), "root");
  const tool = blocks.find((b) => b.kind === "tool")?.tool;
  assert.equal(tool?.target, "src/panel.ts", "the call's `path` argument is the row's target");

  // The target came from the assistant's committed `tool_call`, which stays put.
  const assistant = blocks.find((b) => b.kind === "assistant");
  assert.ok(
    assistant?.content?.some((c) => c.type === "tool_call" && c.id === "t1"),
    "the assistant block still holds the committed tool_call",
  );
});

test("FIX A: renderTool carries the target for the note head's `.to`", () => {
  const tool = renderState(drive("tool_target.handwritten.ndjson"), "root").blocks.find(
    (b) => b.kind === "tool",
  )?.tool;
  assert.equal(tool?.target, "src/panel.ts");
  assert.equal(tool?.summary, "src/panel.ts", "no output -> the path fallback");
});

test("FIX A: toolSummary returns '' while running with no content (one running indicator)", () => {
  assert.equal(toolSummary({ callId: "c", name: "n", output: "", done: false, isError: false }), "");
  // ...and the ONE indicator is the `.tmeta .run` status (V13, `⠋ running…`), not a target span.
});

test("FIX A: renderBlock drops a tool_call content block (no bare `⚙ name` div)", () => {
  const block = renderBlock({
    kind: "assistant",
    content: [{ type: "tool_call", id: "t1", name: "edit", arguments: { path: "src/panel.ts" } }],
  });
  assert.equal(block.html, "", "a tool_call renders as nothing (the fold row carries it)");
  assert.doesNotMatch(block.html, /tool-call/);
});

test("FIX B: the thinking fold label/class is `thinking`, not `thought`", () => {
  const block = renderBlock({ kind: "assistant", content: [{ type: "thinking", text: "x" }] });
  assert.match(block.html, /<details class="fold thinking">/);
  assert.match(block.html, /<span class="tname">thinking<\/span>/);
  assert.doesNotMatch(block.html, /thought/);
});

test("FIX A: a history tool_result has no target, so the target falls back to the summary", () => {
  const state = seedFromHistory(
    initialState(),
    [{ role: "tool_result", tool_call_id: "t1", name: "edit", output: "wrote src/panel.ts", is_error: false }],
    "root",
  );
  const tool = renderState(state, "root").blocks.find((b) => b.kind === "tool")?.tool;

  assert.equal(tool?.target, undefined, "a tool_result carries no arguments, so there is no target");
  assert.equal(tool?.summary, "wrote src/panel.ts", "the summary is the fallback text");
  assert.equal(
    tool?.target ?? tool?.summary,
    "wrote src/panel.ts",
    "the webview's `note.target = tool.target ?? tool.summary` shows the summary for a target-less row",
  );

  // And no empty span: a running call with no target and no output makes the target
  // resolve to "", which the webview's `if (note.target !== "")` guard omits.
  const running = renderState(
    reduce(initialState(), { type: "tool_execution_start", call_id: "c", name: "edit" }, "root"),
    "root",
  ).blocks.find((b) => b.kind === "tool")?.tool;
  assert.equal(running?.target, undefined, "no sibling tool_call -> no target");
  assert.equal(running?.summary, "");
  assert.equal(running?.target ?? running?.summary, "", "an empty `.to` is never emitted as a span");
});
