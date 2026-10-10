import assert from "node:assert/strict";
import { test } from "node:test";

import { findCommand } from "../src/commands.ts";
import { initialState, type Block, type ViewState } from "../src/reducer.ts";
import {
  commandAction,
  isPlaceholderLabel,
  nextActiveTab,
  placeholderLabel,
  stickyLabel,
  tabLabel,
  tabTitle,
} from "../src/tabs.ts";

/** A minimal state whose ROOT transcript is `blocks`. */
function stateWithRoot(blocks: Block[]): ViewState {
  const base = initialState();
  return {
    ...base,
    targeted: "root-1",
    members: [{ id: "root-1", label: "root", state: "idle", isRoot: true }],
    transcripts: { "root-1": blocks },
  };
}

/* ------------------------------------------------------------------- routing */

test("commandAction: /new and /resume open a tab; /reload reloads this one", () => {
  // D013 E — the routing table.
  assert.equal(commandAction(findCommand("new")!), "new");
  assert.equal(commandAction(findCommand("clear")!), "new", "an alias routes as its canonical name");
  assert.equal(commandAction(findCommand("resume")!), "resume");
  assert.equal(commandAction(findCommand("reload")!), "reload");
  assert.equal(commandAction(findCommand("sessions")!), "members");
  assert.equal(commandAction(findCommand("team")!), "team");
  // Everything else is a plain request to THIS tab's child.
  assert.equal(commandAction(findCommand("model")!), "request");
  assert.equal(commandAction(findCommand("plan")!), "request");
  assert.equal(commandAction(findCommand("nope-not-real")), "request", "an unresolved name degrades to a request");
});

/* -------------------------------------------------------------------- labels */

test("tabLabel: empty transcript falls back to the placeholder (D013 C)", () => {
  assert.equal(tabLabel(initialState(), null, 1), "wcode 1");
  assert.equal(isPlaceholderLabel(placeholderLabel(3)), true);
  assert.equal(isPlaceholderLabel("hello"), false);
});

test("tabLabel: the first user message wins, clipped", () => {
  assert.equal(tabLabel(stateWithRoot([{ kind: "user", text: "hello world" }]), null, 1), "hello world");
  const long = "x".repeat(80);
  const label = tabLabel(stateWithRoot([{ kind: "user", text: long }]), null, 2);
  assert.equal(label.length, 30);
  assert.equal(label.endsWith("…"), true);
  // A peer's inbound message is not the human's words — skipped for a later user block.
  const blocks: Block[] = [
    { kind: "user", text: "[message from agent:bob]\npeer chatter" },
    { kind: "user", text: "the human speaks" },
  ];
  assert.equal(tabLabel(stateWithRoot(blocks), null, 1), "the human speaks");
});

test("tabLabel: a --resume path labels by basename before any message", () => {
  assert.equal(tabLabel(initialState(), "/tmp/sessions/2026_abc.jsonl", 1), "2026_abc.jsonl");
  assert.equal(tabLabel(initialState(), "C:\\sessions\\root.jsonl", 1), "root.jsonl");
  // A message still beats the path once one exists.
  assert.equal(tabLabel(stateWithRoot([{ kind: "user", text: "hi" }]), "/tmp/x.jsonl", 1), "hi");
});

test("stickyLabel: a real label is set once and never re-derived (D013 C)", () => {
  assert.equal(stickyLabel("wcode 1", "first words"), "first words");
  assert.equal(stickyLabel("first words", "later words"), "first words");
  // A placeholder candidate cannot displace a real label; it can replace a placeholder.
  assert.equal(stickyLabel("first words", placeholderLabel(2)), "first words");
  assert.equal(stickyLabel("wcode 1", placeholderLabel(2)), placeholderLabel(2));
});

/* -------------------------------------------------------------- active tab */

test("nextActiveTab: focused, else last-active, else first, else null (D013 A)", () => {
  assert.equal(nextActiveTab(["a", "b"], "b"), "b");
  assert.equal(nextActiveTab(["a", "b"], "closed", "a"), "a", "falls back to the last active");
  assert.equal(nextActiveTab(["a", "b"], "closed", "also-closed"), "a", "then to the first");
  assert.equal(nextActiveTab(["a", "b"], "closed", null), "a");
  assert.equal(nextActiveTab([], "closed"), null, "no tabs -> nothing to show");
});

test("tabTitle: the placeholder verbatim, else the `wcode:` prefix (D013 C)", () => {
  assert.equal(tabTitle(placeholderLabel(1)), "wcode 1", "no dangling `wcode:` on a placeholder");
  assert.equal(tabTitle("hello world"), "wcode: hello world");
});
