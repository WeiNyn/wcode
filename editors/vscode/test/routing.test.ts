// Gates 2 and 3: the routing the sidebar depends on, LIVE and model-free.
//
// THE EVIDENCE IS THE CONTROL. A request addressed to a bogus member id is
// refused with `unknown session …`, which shows the server validates
// `frame.session` against its live roster; a real id is answered. The successful
// reply's own `session` field is NOT proof — the server reuses the REQUEST's
// `session` on the reply (`reply_to: Some(id), session, …`), so
// `reply.session === member.id` is true by ECHO. Read the pair together.
//
// `Status` and `SetPlanMode` need no model, so this runs against a real
// `wcode serve --stdio`. It also folds the REAL seeded `sessions` push into the
// tree's item model, and grows it with a `Spawned` from a `Define`.
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import type { AgentEvent, RawFrame, SessionInfo } from "../src/protocol.ts";
import { initialState, memberViews, reduce, type ViewState } from "../src/reducer.ts";
import { WcodeSession } from "../src/session.ts";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const binary = process.env.WCODE_BIN ?? resolve(repoRoot, "target", "debug", "wcode");

interface Seen {
  event: AgentEvent;
  frame: RawFrame;
}

test("routing: a member id is served; a bogus one is refused (live)", { skip: !existsSync(binary), timeout: 30_000 }, async () => {
  const session = new WcodeSession({ binary, cwd: repoRoot, logger: { info: () => {}, error: () => {} } });
  const seen: Seen[] = [];
  let state: ViewState = initialState();

  session.on("event", (event: AgentEvent, frame: RawFrame) => {
    seen.push({ event, frame });
    state = reduce(state, event, frame.session);
  });

  await session.start();
  const seeded: SessionInfo[] = state.members.map((m) => ({ id: m.id, model: m.model, state: m.state }));
  const member = seeded.find((s) => s.id.startsWith("agent:")) ?? seeded[0];
  assert.ok(member, "the seeded push named at least one member");

  // 1. Status addressed to the MEMBER — no model call, correlated by reply_to.
  const before = seen.length;
  const status = await session.ask({ type: "status" }, member.id, 5_000);
  const statusFrame = seen.slice(before).find((s) => s.event.type === "status")?.frame;

  // 2. THE CONTROL — the load-bearing evidence: the id is validated against
  //    the live roster, so a bogus one is refused.
  const bogus = await session.ask({ type: "status" }, "agent:does-not-exist", 5_000);

  // 3. SetPlanMode → Ack (infallible), also addressed to the member.
  const plan = await session.ask({ type: "set_plan_mode", on: false }, member.id, 5_000);

  // 4. Grow the roster: `Define` asks the server's factory for a worker.
  const defined = await session.ask({ type: "define", name: "probe" }, member.id, 5_000);

  console.log("routing (live):");
  console.log(`  seeded members   : ${seeded.map((s) => `${s.id}(${s.state})`).join(", ")}`);
  console.log(`  member under test: ${member.id}`);
  console.log(`  status -> member : type=${status.type} reply_to=${statusFrame?.reply_to} session=${statusFrame?.session} (echo, not proof)`);
  console.log(`  status -> bogus  : type=${bogus.type} ${bogus.type === "error" ? JSON.stringify(bogus.message) : ""}  <- THE routing proof`);
  console.log(`  plan   -> member : type=${plan.type}`);
  console.log(`  define -> member : type=${defined.type} ${defined.type === "spawned" ? defined.worker : ""}`);
  console.log("  tree rows        :");
  for (const row of memberViews(state.members)) {
    console.log(`    ${row.label.padEnd(14)} state=${row.state}${row.liveAction ? ` action=${row.liveAction}` : ""} root=${row.isRoot}`);
  }

  await session.stop();

  assert.equal(status.type, "status", "Status is answered (no model)");
  // NOT proof of routing — the server ECHOES the request's `session` back.
  assert.equal(statusFrame?.session, member.id, "the reply echoes the request's session");
  assert.equal(typeof statusFrame?.reply_to, "number", "and is correlated by reply_to");
  // THE proof: the id is validated against the live roster, so a bogus one dies.
  assert.equal(bogus.type, "error", "a bogus member id is refused (the routing proof)");
  assert.match(String(bogus.type === "error" ? bogus.message : ""), /unknown session/);
  assert.equal(plan.type, "ack", "SetPlanMode is infallible and replies Ack");
  assert.ok(state.members.length >= seeded.length, "the roster folded into the tree's model");
});
