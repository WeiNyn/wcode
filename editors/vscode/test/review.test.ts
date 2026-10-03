import assert from "node:assert/strict";
import { test } from "node:test";

import { reviewHunk, setVerdict, verdictOf, verdictUi } from "../src/review.ts";

test("reviewHunk parses one wcode hunk into typed, numbered lines", () => {
  const diff = "@@ -40,6 +40,9 @@\n ctx\n-old\n+new1\n+new2\n";
  assert.deepEqual(reviewHunk(diff), {
    header: "@@ -40,6 +40,9 @@",
    lines: [
      { kind: "ctx", number: 40, text: "ctx" }, // new-side number
      { kind: "del", number: 41, text: "old" }, // old-side number
      { kind: "add", number: 41, text: "new1" }, // new-side number
      { kind: "add", number: 42, text: "new2" },
    ],
    added: 2,
    removed: 1,
    truncated: false,
  });
});

test("reviewHunk: ctx/add take the new number, del takes the old", () => {
  // A `del` at `from` advances ONLY the old side; the next line keeps the new number.
  const hunk = reviewHunk("@@ -5,3 +5,3 @@\n-a\n b\n+c\n");
  assert.deepEqual(hunk?.lines, [
    { kind: "del", number: 5, text: "a" },
    { kind: "ctx", number: 5, text: "b" },
    { kind: "add", number: 6, text: "c" },
  ]);
  assert.equal(hunk?.added, 1);
  assert.equal(hunk?.removed, 1);
});

test("reviewHunk: a `… (+N more lines)` tail sets truncated and is not a line", () => {
  const truncated = "@@ -1,1 +1,1 @@\n-a\n+b\n… (+37 more lines)";
  assert.equal(reviewHunk(truncated)?.truncated, true);
  assert.equal(reviewHunk(truncated)?.lines.length, 2);
});

test("reviewHunk returns null for a count-less / multi-hunk / foreign diff", () => {
  assert.equal(reviewHunk(""), null);
  assert.equal(reviewHunk("@@ -1 +1 @@\n-a\n+b"), null); // count-less
  assert.equal(reviewHunk("@@ -1,1 +5,1 @@\n-a\n+b"), null); // a different `from` per side
  assert.equal(reviewHunk("@@ -1,1 +1,1 @@\n-a\n+b\n@@ -9,1 +9,1 @@\n-x\n+y"), null); // multi-hunk
  assert.equal(reviewHunk("not a diff"), null);
});

test("setVerdict returns a NEW record (the input is untouched)", () => {
  const before = { a: "pending" as const };
  const after = setVerdict(before, "b", "accepted");
  assert.deepEqual(after, { a: "pending", b: "accepted" });
  assert.deepEqual(before, { a: "pending" });
});

test("verdictOf defaults to 'pending'", () => {
  assert.equal(verdictOf({}, "missing"), "pending");
  assert.equal(verdictOf({ t1: "rejected" }, "t1"), "rejected");
});

test("verdictUi maps a verdict to the block class + badge label", () => {
  assert.deepEqual(verdictUi("pending"), { className: "", label: "" });
  assert.deepEqual(verdictUi("accepted"), { className: "accepted", label: "accepted" });
  assert.deepEqual(verdictUi("rejected"), { className: "rejected", label: "reverted" });
});
