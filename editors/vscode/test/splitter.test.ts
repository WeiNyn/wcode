import assert from "node:assert/strict";
import { test } from "node:test";

import { feed, MAX_FRAME_BYTES } from "../src/session.ts";

test("a frame split across two chunks reassembles", () => {
  const first = feed("", '{"v":1,"id":1,"session":"s","type":"can');
  assert.equal(first.frames.length, 0, "a partial line is not a frame yet");
  assert.equal(first.rest, '{"v":1,"id":1,"session":"s","type":"can');

  const second = feed(first.rest, 'cel"}\n');
  assert.equal(second.frames.length, 1);
  assert.equal(second.frames[0].type, "cancel");
  assert.equal(second.rest, "");
});

test("two frames in one chunk split into two", () => {
  const chunk = '{"v":1,"id":1,"session":"s","type":"cancel"}\n{"v":1,"id":2,"session":"s","type":"ack"}\n';
  const { frames, rest } = feed("", chunk);
  assert.equal(frames.length, 2);
  assert.equal(frames[0].type, "cancel");
  assert.equal(frames[1].type, "ack");
  assert.equal(rest, "");
});

test("a blank line is skipped", () => {
  const { frames } = feed("", '\n   \n{"v":1,"id":1,"session":"s","type":"ack"}\n');
  assert.equal(frames.length, 1);
  assert.equal(frames[0].type, "ack");
});

test("an unparseable line is dropped and the next frame survives", () => {
  const { frames } = feed("", 'not json\n{"v":1,"id":1,"session":"s","type":"ack"}\n');
  assert.equal(frames.length, 1);
  assert.equal(frames[0].type, "ack");
});

test("a line over MAX_FRAME_BYTES is dropped, not buffered", () => {
  const overCap = "a".repeat(MAX_FRAME_BYTES + 1);
  const partial = feed("", overCap); // no newline yet
  assert.equal(partial.frames.length, 0);
  assert.equal(partial.rest, "", "an over-cap partial must not be buffered");

  // A complete over-cap line is dropped; a following valid frame still parses.
  const both = feed("", `${overCap}\n{"v":1,"id":1,"session":"s","type":"ack"}\n`);
  assert.equal(both.frames.length, 1);
  assert.equal(both.frames[0].type, "ack");
});
