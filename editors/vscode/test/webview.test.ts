import assert from "node:assert/strict";
import { test } from "node:test";

import { initialState } from "../src/reducer.ts";
import { renderState, type RenderedState } from "../src/render.ts";
import { createThrottle, parseFromWebview, type Scheduler } from "../src/webview.ts";

/* ------------------------------------------------------- message-union helpers */

test("parseFromWebview accepts the well-formed messages", () => {
  assert.deepEqual(parseFromWebview({ kind: "submit", text: "hi" }), { kind: "submit", text: "hi" });
  assert.deepEqual(parseFromWebview({ kind: "steer", text: "wait" }), { kind: "steer", text: "wait" });
  assert.deepEqual(parseFromWebview({ kind: "cancel" }), { kind: "cancel" });
  assert.deepEqual(parseFromWebview({ kind: "ready" }), { kind: "ready" });
  assert.deepEqual(parseFromWebview({ kind: "open-diff", callId: "t1" }), { kind: "open-diff", callId: "t1" });
  assert.deepEqual(parseFromWebview({ kind: "reveal-file", path: "a.rs", line: 3 }), {
    kind: "reveal-file",
    path: "a.rs",
    line: 3,
  });
  assert.deepEqual(parseFromWebview({ kind: "reveal-file", path: "a.rs" }), {
    kind: "reveal-file",
    path: "a.rs",
    line: undefined,
  });
});

test("parseFromWebview rejects junk and empty input", () => {
  assert.equal(parseFromWebview(null), null);
  assert.equal(parseFromWebview(42), null);
  assert.equal(parseFromWebview("submit"), null);
  assert.equal(parseFromWebview({}), null);
  assert.equal(parseFromWebview({ kind: "from-the-future" }), null);
  assert.equal(parseFromWebview({ kind: "submit" }), null, "no text");
  assert.equal(parseFromWebview({ kind: "submit", text: "   " }), null, "blank text");
  assert.equal(parseFromWebview({ kind: "open-diff" }), null, "no callId");
  assert.equal(parseFromWebview({ kind: "reveal-file", line: 2 }), null, "no path");
});

/* ------------------------------------------------------------------- throttle */

interface FakeClock {
  scheduler: Scheduler;
  advance(ms: number): void;
  pending(): number;
}

function fakeClock(start = 1_000): FakeClock {
  let now = start;
  let nextId = 1;
  const timers: Array<{ id: number; at: number; fn: () => void }> = [];
  return {
    scheduler: {
      now: () => now,
      setTimeout: (fn, ms) => {
        const id = nextId++;
        timers.push({ id, at: now + ms, fn });
        return id;
      },
      clearTimeout: (handle) => {
        const index = timers.findIndex((t) => t.id === handle);
        if (index >= 0) timers.splice(index, 1);
      },
    },
    advance(ms) {
      now += ms;
      for (const timer of [...timers].sort((a, b) => a.at - b.at)) {
        if (timer.at > now) continue;
        const index = timers.findIndex((t) => t.id === timer.id);
        if (index >= 0) timers.splice(index, 1);
        timer.fn();
      }
    },
    pending: () => timers.length,
  };
}

function states(): RenderedState[] {
  return [
    renderState(initialState()),
    renderState({ ...initialState(), todos: [{ content: "a", status: "pending" }] }),
    renderState({ ...initialState(), status: { running: true, planMode: false } }),
    renderState({ ...initialState(), status: { running: false, planMode: true } }),
  ];
}

test("the throttle sends the first snapshot immediately", () => {
  const sent: RenderedState[] = [];
  const clock = fakeClock();
  const throttle = createThrottle(30, (s) => sent.push(s), clock.scheduler);
  const [s1] = states();

  throttle.push(s1);
  assert.equal(sent.length, 1, "nothing to coalesce yet");
  assert.equal(sent[0], s1);
  assert.equal(clock.pending(), 0);
});

test("rapid pushes coalesce into one send (the last wins)", () => {
  const sent: RenderedState[] = [];
  const clock = fakeClock();
  const throttle = createThrottle(30, (s) => sent.push(s), clock.scheduler);
  const [s1, s2, s3] = states();

  throttle.push(s1); // immediate
  throttle.push(s2); // schedules
  throttle.push(s3); // replaces the pending
  assert.equal(sent.length, 1, "still coalescing");
  assert.equal(clock.pending(), 1);

  clock.advance(30);
  assert.equal(sent.length, 2);
  assert.equal(sent[1], s3, "the last snapshot wins");
  assert.equal(clock.pending(), 0);
});

test("flush sends immediately and cancels the pending send", () => {
  const sent: RenderedState[] = [];
  const clock = fakeClock();
  const throttle = createThrottle(30, (s) => sent.push(s), clock.scheduler);
  const [s1, s2, s3] = states();

  throttle.push(s1);
  throttle.push(s2);
  throttle.flush(s3);
  assert.equal(sent.length, 2);
  assert.equal(sent[1], s3);
  assert.equal(clock.pending(), 0, "the scheduled send was cancelled");

  clock.advance(1000);
  assert.equal(sent.length, 2, "nothing fires after the flush");
});

test("dispose cancels a pending send", () => {
  const sent: RenderedState[] = [];
  const clock = fakeClock();
  const throttle = createThrottle(30, (s) => sent.push(s), clock.scheduler);
  const [s1, s2] = states();

  throttle.push(s1);
  throttle.push(s2);
  assert.equal(clock.pending(), 1);
  throttle.dispose();
  assert.equal(clock.pending(), 0);

  clock.advance(1000);
  assert.equal(sent.length, 1, "the coalesced send never fired");
});
