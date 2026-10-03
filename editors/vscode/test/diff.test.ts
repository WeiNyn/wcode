import assert from "node:assert/strict";
import { test } from "node:test";

import { reverseApply } from "../src/diff.ts";
import {
  CAPPED,
  CHANGED_LINE_CURRENT,
  IDENTITY_FIXTURE,
  NULL_FIXTURES,
  ROUND_TRIP_FIXTURES,
} from "./fixtures/diffs.ts";

function roundTrip(name: string): (typeof ROUND_TRIP_FIXTURES)[number] {
  const fixture = ROUND_TRIP_FIXTURES.find((f) => f.name === name);
  assert.ok(fixture, `fixture \`${name}\` is missing`);
  return fixture;
}

for (const fixture of ROUND_TRIP_FIXTURES) {
  test(`round-trips ${fixture.name} to its pre-image [${fixture.source}]`, () => {
    assert.equal(reverseApply(fixture.new, fixture.diff), fixture.old);
  });
}

test("the changed-line fixture matches the generator's own assertions", () => {
  // Mirror `diff.rs::tests::a_changed_line_shows_context_minus_and_plus`.
  const { diff } = roundTrip("changed_line");
  assert.ok(diff.startsWith("@@ -1,5 +1,5 @@"), diff);
  assert.ok(diff.includes("\n-l3"), diff);
  assert.ok(diff.includes("\n+X"), diff);
  assert.ok(diff.includes("\n l2"), diff);
});

test("a new file (@@ -1,0 +1,N @@) reverse-applies to an empty pre-image", () => {
  const { diff } = roundTrip("new_file");
  assert.ok(diff.startsWith("@@ -1,0 +1,2 @@"), diff);
  assert.equal(reverseApply("a\nb\n", diff), "");
});

test("a context-only (trailing-newline) hunk is the IDENTITY, not null", () => {
  const current = IDENTITY_FIXTURE.new;
  assert.equal(
    reverseApply(current, IDENTITY_FIXTURE.diff),
    current,
    "a text patch cannot express a trailing-newline change: nothing to undo",
  );
});

test("a CRLF file round-trips to a CRLF pre-image", () => {
  const fixture = roundTrip("crlf");
  const before = reverseApply(fixture.new, fixture.diff);
  assert.equal(before, fixture.old);
  assert.ok(before?.includes("\r\n"), "the file's ending is preserved, not normalized to LF");
  assert.ok(!before?.includes("\n\n"), "no doubled terminators");
});

test("an LF file stays LF (the ending is detected, not assumed)", () => {
  const fixture = roundTrip("changed_line");
  const before = reverseApply(fixture.new, fixture.diff);
  assert.ok(before !== null && !before.includes("\r"), "no CRLF introduced into an LF file");
});

for (const fixture of NULL_FIXTURES) {
  test(`returns null for ${fixture.name}`, () => {
    assert.equal(reverseApply(fixture.current ?? CHANGED_LINE_CURRENT, fixture.diff), null);
  });
}

test("the capped fixture is exactly the generator's truncation, and is null", () => {
  assert.ok(CAPPED.diff.startsWith("@@ -1,100 +1,100 @@"), CAPPED.diff);
  assert.ok(CAPPED.diff.includes("\n… (+160 more lines)"), CAPPED.diff);
  assert.equal(reverseApply(CAPPED.new, CAPPED.diff), null);
});
