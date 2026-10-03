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

  await session.stop();
  assert.equal(session.state, "stopped");
});
