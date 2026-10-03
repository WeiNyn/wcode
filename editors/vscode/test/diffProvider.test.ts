import assert from "node:assert/strict";
import { test } from "node:test";

import {
  BeforeRegistry,
  DIFF_SCHEME,
  diffTokenPath,
  findToolBlock,
  lastDiffCallId,
  tokenFromDiffPath,
} from "../src/diff.ts";
import { initialState, reduce, type ViewState } from "../src/reducer.ts";

/* --------------------------------------------------------------- the registry */

test("mint() then resolve() round-trips; an unknown token resolves to undefined", () => {
  const registry = new BeforeRegistry();
  const token = registry.mint("the before text");
  assert.equal(registry.resolve(token), "the before text");
  assert.equal(registry.resolve("never-minted"), undefined);
});

test("a wcode-diff: uri carries only the opaque token (never file text)", () => {
  const registry = new BeforeRegistry();
  const before = "secret file contents\n";
  const token = registry.mint(before);
  const uriPath = diffTokenPath(token);

  assert.equal(uriPath, `/${token}`);
  assert.ok(uriPath.length < 12, `token should be short, got ${uriPath}`);
  assert.ok(!uriPath.includes("secret"), "the text must never reach the URI");
  assert.ok(!uriPath.includes("\n"));
  assert.equal(tokenFromDiffPath(uriPath), token, "the path round-trips to the token");
  assert.equal(DIFF_SCHEME, "wcode-diff");
  // The registry is the only place the text lives.
  assert.equal(registry.resolve(tokenFromDiffPath(uriPath)), before);
});

test("the registry evicts the least-recently-used past its capacity", () => {
  const registry = new BeforeRegistry(3);
  const a = registry.mint("A");
  const b = registry.mint("B");
  const c = registry.mint("C");
  assert.equal(registry.size, 3);

  assert.equal(registry.resolve(a), "A", "resolving refreshes A's recency");

  const d = registry.mint("D"); // capacity 3 -> evicts the LRU, now B
  assert.equal(registry.size, 3);
  assert.equal(registry.resolve(a), "A");
  assert.equal(registry.resolve(b), undefined, "B was the least recently used");
  assert.equal(registry.resolve(c), "C");
  assert.equal(registry.resolve(d), "D");
});

test("clear() drops every token (a restart starts clean)", () => {
  const registry = new BeforeRegistry();
  const token = registry.mint("before");
  assert.equal(registry.size, 1);
  registry.clear();
  assert.equal(registry.size, 0);
  assert.equal(registry.resolve(token), undefined);
});

/* --------------------------------------------------------- finding the block */

function stateWithTools(): ViewState {
  let state = initialState();
  state = reduce(state, { type: "tool_execution_start", call_id: "t1", name: "bash" });
  state = reduce(state, {
    type: "tool_execution_end",
    call_id: "t1",
    name: "bash",
    output: "hi",
    is_error: false,
  });
  state = reduce(state, { type: "tool_execution_start", call_id: "t2", name: "edit" });
  state = reduce(state, {
    type: "tool_execution_end",
    call_id: "t2",
    name: "edit",
    output: "",
    is_error: false,
    diff: "@@ -1,1 +1,1 @@\n-a\n+b",
    path: "src/f.rs",
  });
  return state;
}

test("findToolBlock finds the call by call_id, and nothing for an unknown id", () => {
  const state = stateWithTools();
  assert.equal(findToolBlock(state, "t2")?.path, "src/f.rs");
  assert.equal(findToolBlock(state, "t1")?.diff, undefined);
  assert.equal(findToolBlock(state, "nope"), undefined);
});

test("lastDiffCallId is the most recent call that has a diff", () => {
  assert.equal(lastDiffCallId(stateWithTools()), "t2");

  // A `path` with NO diff (a write that changed no line) is not a diff.
  let state = stateWithTools();
  state = reduce(state, { type: "tool_execution_start", call_id: "t3", name: "write" });
  state = reduce(state, {
    type: "tool_execution_end",
    call_id: "t3",
    name: "write",
    output: "wrote it",
    is_error: false,
    path: "src/g.rs",
  });
  assert.equal(lastDiffCallId(state), "t2", "the diff-less write does not shadow t2");

  assert.equal(lastDiffCallId(initialState()), undefined, "no tools -> no diff");
});
