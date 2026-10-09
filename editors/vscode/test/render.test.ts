import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent } from "../src/protocol.ts";
import { appendUser, initialState, reduce, type SessionMember, type ViewState } from "../src/reducer.ts";
import { escapeHtml, renderBlock, renderMerged, renderState, toolSummary } from "../src/render.ts";

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

test("toolSummary clips output, names error, and is empty while running with no content", () => {
  assert.equal(
    toolSummary({ callId: "c", name: "n", output: "", done: false, isError: false }),
    "",
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

test("a thinking block renders as the draft's folded `thinking` row", () => {
  const block = renderBlock({
    kind: "assistant",
    content: [{ type: "thinking", text: "weighing it" }],
  });
  assert.match(block.html, /<details class="fold thinking">/);
  assert.match(block.html, /<span class="tname">thinking<\/span>/);
  assert.match(block.html, /weighing it/);
  assert.match(block.html, /class="fold thinking"/, "the renamed fold class is present");
});

test("a fenced code block renders as a titled `.code` card with Prism tokens", () => {
  const block = renderBlock({
    kind: "assistant",
    content: [{ type: "text", text: "```ts\nconst x = 1;\n```" }],
  });
  assert.match(block.html, /<div class="code">/);
  assert.match(block.html, /<div class="chead"><span>ts<\/span><\/div>/);
  assert.match(block.html, /class="token /, "the code body carries Prism token spans");
  assert.match(block.html, /keyword[^>]*>const</, "the `const` keyword is a token");
});

test("a live assistant block's thinking fold is open; a settled one is not", () => {
  const live = renderBlock({
    kind: "assistant",
    live: true,
    content: [{ type: "thinking", text: "weighing options" }],
  });
  assert.match(live.html, /<details class="fold thinking" open>/);

  const done = renderBlock({
    kind: "assistant",
    live: false,
    content: [{ type: "thinking", text: "weighing options" }],
  });
  assert.doesNotMatch(done.html, / open>/);
  assert.match(done.html, /<details class="fold thinking">/);
});
test("renderMerged INTERLEAVES members by arrival order, tagged by origin", () => {
  const members: SessionMember[] = [
    { id: "root-1", label: "root-1", state: "running", isRoot: true },
    { id: "agent:w1", label: "w1", state: "idle", isRoot: false },
  ];
  let state: ViewState = { ...initialState(), members };
  // The orchestrator speaks, hands off, the worker answers, the orchestrator resumes —
  // the timeline the panel must show, NOT per-member contiguous runs.
  state = appendUser(state, "hello", "root-1");
  state = appendUser(state, "on it", "agent:w1");
  state = appendUser(state, "another", "root-1");

  assert.deepEqual(
    renderMerged(state).map((b) => b.origin),
    ["root-1", "agent:w1", "root-1"],
    "woven by arrival, so the orchestrator's continuation is a NEW block after the worker's",
  );
});

test("renderMerged falls back to a stable order for blocks with no seq", () => {
  const members: SessionMember[] = [
    { id: "root-1", label: "root-1", state: "running", isRoot: true },
    { id: "agent:w1", label: "w1", state: "idle", isRoot: false },
  ];
  const state: ViewState = {
    ...initialState(),
    members,
    transcripts: {
      "root-1": [{ kind: "user", text: "a" }],
      "agent:w1": [{ kind: "user", text: "b" }],
    },
  };
  assert.deepEqual(renderMerged(state).map((b) => b.origin), ["root-1", "agent:w1"]);
});

test("renderTool carries the highlighted OUTPUT (W009): bash by the tool, a read by its path", () => {
  const bash = renderBlock({
    kind: "tool",
    tool: { callId: "c1", name: "bash", output: "echo hi", done: true, isError: false },
  }).tool;
  assert.match(bash?.outputHtml ?? "", /class="token /, "bash output is highlighted as bash");
  const read = renderBlock({
    kind: "tool",
    tool: { callId: "c2", name: "read", output: "fn main() {}", done: true, isError: false, path: "src/a.rs" },
  }).tool;
  assert.match(read?.outputHtml ?? "", /keyword[^>]*>fn</, "a `.rs` read is highlighted as rust");
  const plain = renderBlock({
    kind: "tool",
    tool: { callId: "c3", name: "grep", output: "some lines", done: true, isError: false },
  }).tool;
  assert.equal(plain?.outputHtml, undefined, "a pathless tool stays plain (textContent fallback)");
});

test("renderTool highlights the diff BODY per line, re-opening a continuation span (W009 P4)", () => {
  // A `.ts` block comment spans lines 1-2: Prism opens the comment span on line 1 and closes it
  // on line 2, so a naive per-line split would leave line 1 unclosed and line 2 a bare close
  // (its colour lost). The join-then-split RE-OPENS the span on the continuation line.
  const diff = "@@ -1,3 +1,3 @@\n /* one\n    two */\n const x = 1;\n";
  const tool = renderBlock({
    kind: "tool",
    tool: { callId: "c", name: "edit", output: "", done: true, isError: false, path: "src/a.ts", diff },
  }).tool;
  assert.equal(tool?.hasDiff, true);
  const lines = tool?.diffLinesHtml ?? [];
  assert.equal(lines.length, 3, "one fragment per hunk line");
  lines.forEach((html, i) => {
    const open = (html.match(/<span/g) ?? []).length;
    const close = (html.match(/<\/span>/g) ?? []).length;
    assert.equal(open, close, `line ${i + 1} has well-formed (balanced) spans`);
  });
  assert.match(lines[0], /token comment/, "the comment opens with colour on line 1");
  assert.match(lines[1], /token comment/, "the CONTINUATION line keeps the comment colour (re-opened)");
});
