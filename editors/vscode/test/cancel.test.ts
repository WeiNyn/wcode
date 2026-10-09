import assert from "node:assert/strict";
import { test } from "node:test";

import { cancelTargets } from "../src/cancel.ts";
import type { SessionMember } from "../src/reducer.ts";

const member = (id: string, state: SessionMember["state"], isRoot = false): SessionMember => ({
  id,
  label: id,
  state,
  isRoot,
});

test("cancelTargets: Focus mode cancels the focused member ALONE", () => {
  const members = [member("root-1", "running", true), member("agent:w1", "running")];
  assert.deepEqual(cancelTargets("agent:w1", members, "root-1"), ["agent:w1"]);
  assert.deepEqual(cancelTargets("root-1", members, "root-1"), ["root-1"]);
});

test("cancelTargets: All mode cancels the root AND every running member", () => {
  const members = [
    member("root-1", "idle", true), // the root: idle, still cancelled (All = the session)
    member("agent:w1", "running"),
    member("agent:w2", "idle"), // idle non-root: NOT cancelled
    member("agent:w3", "running"),
  ];
  const ids = cancelTargets(null, members, "root-1");
  assert.ok(ids.includes("root-1"), "the root is cancelled");
  assert.ok(ids.includes("agent:w1"), "a running member is cancelled");
  assert.ok(ids.includes("agent:w3"), "a second running member is cancelled");
  assert.ok(!ids.includes("agent:w2"), "an idle member is NOT cancelled");
  assert.equal(ids.length, 3, "exactly the root + the two running members");
});

test("cancelTargets: All mode with no root still reaches each running member", () => {
  const ids = cancelTargets(null, [member("agent:w1", "running")], null);
  assert.deepEqual(ids, ["agent:w1"]);
});
