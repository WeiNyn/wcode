import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent } from "../src/protocol.ts";
import { appendUser, initialState, reduce, type ViewState } from "../src/reducer.ts";
import { escapeHtml, renderBlock, renderState, toolSummary } from "../src/render.ts";

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

test("an assistant message renders to markdown HTML, not raw text", () => {
  const state = drive("submit_stream.handwritten.ndjson");
  const rendered = renderState(state, "root");
  const block = rendered.blocks[0];
  assert.equal(block.kind, "assistant");
  assert.equal(block.live, false);
  assert.match(block.html, /<p>Hello there<\/p>/, "markdown-it wrapped the text");
});

test("a live assistant block is flagged (the webview paints the cursor)", () => {
  let state = initialState();
  state = reduce(state, {
    type: "message_start",
    message: { role: "assistant", content: [{ type: "text", text: "partial" }], stop_reason: "stop" },
  });
  const block = renderState(state, "").blocks[0];
  assert.equal(block.live, true);
});

test("tool blocks render a collapsed summary and flag a diff", () => {
  const state = drive("tool_execution.handwritten.ndjson");
  const tools = renderState(state, "root").blocks.filter((b) => b.kind === "tool");
  assert.equal(tools.length, 2);

  const bash = tools[0].tool;
  assert.equal(bash?.name, "bash");
  assert.equal(bash?.summary, "line1", "the first non-empty output line");
  assert.equal(bash?.hasDiff, false);
  assert.equal(bash?.durationMs, 42);
  assert.match(bash?.outputHtml ?? "", /^<pre class="tool-output">/);

  const edit = tools[1].tool;
  assert.equal(edit?.summary, "src/f.rs", "an empty output falls back to the path");
  assert.equal(edit?.hasDiff, true, "the P2 diff hook sees the diff");
  assert.equal(edit?.diff, "@@ -1 +1 @@\n-a\n+b\n", "the raw diff crosses the wire for P4 + the +N −M count");
});

test("user, error and notice text is escaped, never rendered as HTML", () => {
  const withUser = appendUser(initialState(), "<script>alert(1)</script>", "root");
  const user = renderState(withUser, "root").blocks[0];
  assert.equal(user.kind, "user");
  assert.ok(!user.html.includes("<script"), "the raw tag must not survive");
  assert.match(user.html, /&lt;script&gt;/);

  const withError = reduce(initialState(), { type: "error", message: '<img src=x onerror="x">' }, "root");
  const error = renderState(withError, "root").blocks[0];
  assert.equal(error.kind, "error");
  assert.ok(!error.html.includes("<img"));
});

test("an assistant's raw HTML is escaped by markdown-it (html: false)", () => {
  const block = renderBlock({
    kind: "assistant",
    content: [{ type: "text", text: "<b>bold</b>" }],
  });
  assert.ok(!block.html.includes("<b>bold</b>"), "raw HTML is escaped, not passed through");
  assert.match(block.html, /&lt;b&gt;/);
});

test("toolSummary clips, and names running / error", () => {
  assert.equal(
    toolSummary({ callId: "c", name: "n", output: "", done: false, isError: false }),
    "running…",
  );
  assert.equal(
    toolSummary({ callId: "c", name: "n", output: "boom", done: true, isError: true }),
    "error",
  );
  const long = "x".repeat(200);
  const clipped = toolSummary({ callId: "c", name: "n", output: long, done: true, isError: false });
  assert.equal(clipped.length, 72);
  assert.ok(clipped.endsWith("…"));
});

test("escapeHtml covers the four dangerous characters", () => {
  assert.equal(escapeHtml('&<>"'), "&amp;&lt;&gt;&quot;");
});

test("a thinking block renders as the draft's folded `thought` row", () => {
  const block = renderBlock({
    kind: "assistant",
    content: [{ type: "thinking", text: "weighing it" }],
  });
  assert.match(block.html, /<details class="fold thought">/);
  assert.match(block.html, /<span class="tname">thought<\/span>/);
  assert.match(block.html, /weighing it/);
  assert.doesNotMatch(block.html, /class="thinking"/, "the old fold class is gone");
});

test("a fenced code block renders as a titled `.code` card", () => {
  const block = renderBlock({
    kind: "assistant",
    content: [{ type: "text", text: "```ts\nconst x = 1;\n```" }],
  });
  assert.match(block.html, /<div class="code">/);
  assert.match(block.html, /<div class="chead"><span>ts<\/span><\/div>/);
  assert.match(block.html, /<pre class="pre">const x = 1;\n<\/pre>/);
  assert.ok(!block.html.includes("hljs"), "no highlighting in P1");
});

test("a live assistant block's thinking fold is open; a settled one is not", () => {
  const live = renderBlock({
    kind: "assistant",
    live: true,
    content: [{ type: "thinking", text: "weighing options" }],
  });
  assert.match(live.html, /<details class="fold thought" open>/);

  const done = renderBlock({
    kind: "assistant",
    live: false,
    content: [{ type: "thinking", text: "weighing options" }],
  });
  assert.doesNotMatch(done.html, / open>/);
  assert.match(done.html, /<details class="fold thought">/);
});
