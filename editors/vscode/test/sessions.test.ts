import assert from "node:assert/strict";
import { test } from "node:test";

import { firstUserText, formatWhen, parseHeader } from "../src/sessions.ts";

const HEADER = '{"type":"header","version":1,"id":"abc","cwd":"/repo","created":"2026-09-20T05:12:03Z"}';
const user = (text: string) =>
  JSON.stringify({ type: "message", id: "m", parent_id: null, message: { role: "user", content: [{ type: "text", text }] } });

test("parseHeader reads cwd + created, and rejects anything else", () => {
  assert.deepEqual(parseHeader(HEADER), { cwd: "/repo", created: "2026-09-20T05:12:03Z" });
  assert.equal(parseHeader(user("hi")), null);
  assert.equal(parseHeader("not json"), null);
  assert.equal(parseHeader(""), null);
});

test("firstUserText skips assistant turns and returns the first USER message", () => {
  const assistant = JSON.stringify({
    type: "message",
    message: { role: "assistant", content: [{ type: "text", text: "hello there" }] },
  });
  assert.equal(firstUserText([HEADER, assistant, user("do the thing")]), "do the thing");
});

test("firstUserText strips the harness's inbound tag (the human's own too)", () => {
  assert.equal(firstUserText([user("[message from user]\nreview the diff")]), "review the diff");
  // A peer's message is NOT the human's — it is skipped in favour of a real user turn.
  assert.equal(firstUserText([user("[message from agent:w1]\nfound issues"), user("fix it")]), "fix it");
});

test("firstUserText clips a long message and collapses whitespace", () => {
  const long = firstUserText([user("a".repeat(200))], 20);
  assert.equal(long.length, 20);
  assert.ok(long.endsWith("…"));
  assert.equal(firstUserText([user("line one\nline two")]), "line one line two");
});

test("firstUserText returns '' when the head holds no user message", () => {
  assert.equal(firstUserText([HEADER]), "");
  assert.equal(firstUserText([]), "");
});

test("formatWhen renders a local timestamp and '' for junk", () => {
  assert.match(formatWhen("2026-09-20T05:12:03Z"), /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
  assert.equal(formatWhen(""), "");
  assert.equal(formatWhen("nonsense"), "");
});
