// Gate 3: the host-side pipeline, end to end and headless —
//   protocol frames → reducer → rendered HTML.
//
// This proves the links the host owns. It does NOT prove the two links it
// cannot reach without a webview: HTML → DOM, and the webview ⇄ host
// `postMessage` hop. Those are exercised only by F5 (see README).
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent, RawFrame } from "../src/protocol.ts";
import { initialState, reduce, type ViewState } from "../src/reducer.ts";
import { renderState, type RenderedState } from "../src/render.ts";
import { WcodeSession } from "../src/session.ts";

const here = dirname(fileURLToPath(import.meta.url));
const fixturesDir = resolve(here, "fixtures");
const repoRoot = resolve(here, "..", "..", "..");
const binary = process.env.WCODE_BIN ?? resolve(repoRoot, "target", "debug", "wcode");

function blocksToHtml(rendered: RenderedState): string {
  return rendered.blocks
    .map((block) => {
      if (block.kind === "tool" && block.tool) {
        return `<div class="tool">⚙ ${block.tool.name} — ${block.tool.summary}</div>\n${block.tool.outputHtml}`;
      }
      return block.html.trimEnd();
    })
    .join("\n");
}

test("pipeline: real frames → reducer → HTML", { skip: !existsSync(binary), timeout: 20_000 }, async () => {
  const session = new WcodeSession({
    binary,
    cwd: repoRoot,
    logger: { info: () => {}, error: () => {} },
  });
  let state: ViewState = initialState();
  const seen: string[] = [];

  session.on("event", (event: AgentEvent, frame: RawFrame) => {
    seen.push(event.type);
    state = reduce(state, event, frame.session);
  });

  await session.start();
  const reply = await session.ask({ type: "get_history" });
  const rendered = renderState(state, state.targeted);

  console.log("pipeline (live):");
  console.log(`  binary         : ${binary}`);
  console.log(`  seeded root id : ${session.rootSessionId}`);
  console.log(`  frames seen    : ${seen.join(", ")}`);
  console.log(`  get_history    : ${reply.type}`);
  console.log(`  rendered members: ${rendered.members.map((m) => m.id).join(", ") || "(none)"}`);
  console.log(`  rendered blocks : ${rendered.blocks.length}`);
  console.log(`  html            : ${blocksToHtml(rendered) || "(empty transcript → the panel's empty state)"}`);

  await session.stop();

  assert.equal(reply.type, "history");
  assert.ok(session.rootSessionId, "the seed named the root session");
  assert.ok(rendered.members.length >= 1, "the roster rendered into the view state");
});

test("pipeline: fixtures → reducer → HTML (printed)", () => {
  let state: ViewState = initialState();
  for (const name of [
    "sessions_roster.handwritten.ndjson",
    "submit_stream.handwritten.ndjson",
    "tool_execution.handwritten.ndjson",
  ]) {
    const events = readFileSync(resolve(fixturesDir, name), "utf8")
      .split("\n")
      .filter((line) => line.trim() !== "")
      .map((line) => JSON.parse(line) as AgentEvent);
    for (const event of events) state = reduce(state, event, "root");
  }

  const rendered = renderState(state, "root");
  const html = blocksToHtml(rendered);
  console.log("pipeline (fixtures) — rendered HTML:");
  console.log(html);

  assert.match(html, /<p>Hello there<\/p>/);
  assert.match(html, /class="tool"/);
});
