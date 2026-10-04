import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent, RawFrame } from "../src/protocol.ts";
import { WcodeSession } from "../src/session.ts";

// editors/vscode/test -> editors/vscode -> editors -> <repo root>
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const binary = process.env.WCODE_BIN ?? resolve(repoRoot, "target", "debug", "wcode");

/**
 * Wait until `predicate` holds, or give up after `ms`. The live spine needs this:
 * `start()` resolves on the ready HEURISTIC (the first frame OR 250 ms, whichever
 * comes first — `session.ts::spawnOnce`), so on a loaded machine the seeded
 * `sessions` push can still be in flight when `start()` returns. Asserting
 * immediately is a race that fails under parallel test load.
 */
async function until(predicate: () => boolean, ms = 10_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (!predicate() && Date.now() < deadline) {
    await new Promise((settle) => setTimeout(settle, 20));
  }
}

// The spine, live: spawn the real binary and prove the TS client speaks the
// protocol. Skipped (not failed) when the binary has not been built, so
// `npm test` stays green without a Cargo build.
test("spine: seeded push, real root id, correlated history reply", { skip: !existsSync(binary), timeout: 20_000 }, async () => {
  const session = new WcodeSession({
    binary,
    cwd: repoRoot,
    logger: { info: () => {}, error: () => {} },
  });

  const seen: string[] = [];
  let seedRootId: string | null = null;
  let historyReplyTo: number | undefined;

  session.on("event", (event: AgentEvent, frame: RawFrame) => {
    seen.push(event.type);
    if (event.type === "sessions" && seedRootId === null) {
      seedRootId = event.sessions[0]?.id ?? null;
    }
    if (event.type === "history") {
      historyReplyTo = frame.reply_to;
    }
  });

  await session.start();
  try {
    await until(() => session.rootSessionId !== null);
    assert.ok(session.rootSessionId, "the seeded `sessions` push named a root id");
    assert.equal(session.rootSessionId, seedRootId, "the client addresses the seeded root id");

    const reply = await session.ask({ type: "get_history" });

    assert.equal(reply.type, "history");
    assert.equal(typeof historyReplyTo, "number", "the history reply was correlated via reply_to");

    console.log("live spine check:");
    console.log(`  binary            : ${binary}`);
    console.log(`  seeded root id    : ${session.rootSessionId}`);
    console.log(`  frames seen       : ${seen.join(", ")}`);
    console.log(`  get_history reply : type=${reply.type} reply_to=${historyReplyTo}`);
  } finally {
    // ALWAYS stop: a child leaked by a failed assertion keeps the test runner's
    // event loop alive, so `node --test` HANGS instead of reporting the failure.
    await session.stop();
  }
  assert.equal(session.state, "stopped");
});
