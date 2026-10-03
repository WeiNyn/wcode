#!/usr/bin/env node
/**
 * Capture REAL frames from `wcode serve --stdio` into `test/fixtures/`.
 *
 * Ported from the working `/tmp/drive.py` harness. It:
 *   1. spawns the real binary (WCODE_BIN, else <repo>/target/debug/wcode),
 *   2. HOLDS STDIN OPEN (a closing client can lose in-flight replies — observed:
 *      3 of 4 dropped in one piped run),
 *   3. reads the seeded `sessions` push and addresses the ROOT session's real id,
 *   4. sends model-free requests and writes every frame it sees.
 *
 * Only model-free frames are captured here (`sessions`, `history`, `status`).
 * Model-dependent events (`message_*`, `tool_execution_*`, `turn_*`, `agent_*`,
 * `compaction*`, `retrying`, `stopped`) cannot be produced without a model call,
 * so those fixtures are HAND-WRITTEN from the typed wire shapes and marked as
 * such — see test/fixtures/README.md.
 */
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..", "..");
const binary = process.env.WCODE_BIN ?? resolve(repoRoot, "target", "debug", "wcode");
const fixturesDir = resolve(here, "..", "test", "fixtures");
mkdirSync(fixturesDir, { recursive: true });

const child = spawn(binary, ["serve", "--stdio"], { cwd: repoRoot, stdio: ["pipe", "pipe", "pipe"] });
child.stderr.setEncoding("utf8");
child.stderr.on("data", (chunk) => process.stderr.write(`[stderr] ${chunk}`));

// A watchdog: if a reply is lost, close stdin anyway so the capture cannot hang.
const watchdog = setTimeout(() => child.stdin.end(), 5000);
watchdog.unref?.();

/** @type {Array<Record<string, unknown>>} */
const frames = [];
let carry = "";
let rootId = null;
let nextId = 1;
/** @type {Set<number>} */
const awaiting = new Set();

function send(body) {
  const id = nextId++;
  awaiting.add(id);
  child.stdin.write(`${JSON.stringify({ v: 1, id, session: rootId, ...body })}\n`);
  return id;
}

child.stdout.setEncoding("utf8");
child.stdout.on("data", (chunk) => {
  carry += chunk;
  let nl;
  while ((nl = carry.indexOf("\n")) !== -1) {
    const line = carry.slice(0, nl).trim();
    carry = carry.slice(nl + 1);
    if (line === "") continue;
    const frame = JSON.parse(line);
    frames.push(frame);

    if (frame.type === "sessions" && frame.reply_to === undefined && rootId === null) {
      rootId = frame.sessions?.[0]?.id ?? null;
      if (rootId === null) {
        console.error("the seeded push named no root session");
        child.stdin.end();
        return;
      }
      console.log(`seeded push: type=sessions id=${frame.id} reply_to=${frame.reply_to ?? "-"}`);
      console.log(`root session from the seed: ${rootId}\n`);
      send({ type: "list_sessions" });
      send({ type: "get_history" });
      send({ type: "status" });
      continue;
    }

    if (typeof frame.reply_to === "number") {
      awaiting.delete(frame.reply_to);
      const extra = Object.keys(frame).filter(
        (k) => !["v", "id", "reply_to", "session", "sender", "type"].includes(k),
      );
      console.log(`  -> type=${frame.type} reply_to=${frame.reply_to} keys=${JSON.stringify(extra)}`);
      if (awaiting.size === 0) {
        // Give the detached writer a beat, then close stdin and let it exit.
        setTimeout(() => child.stdin.end(), 150);
      }
    }
  }
});

child.on("error", (err) => {
  console.error(`spawn failed: ${err.message}`);
  process.exitCode = 1;
});

const exitCode = await new Promise((resolveExit) => child.on("exit", resolveExit));
const outPath = resolve(fixturesDir, "list_sessions.ndjson");
writeFileSync(outPath, `${frames.map((f) => JSON.stringify(f)).join("\n")}\n`);
console.log(`\nwrote ${frames.length} frame(s) to ${outPath}`);
console.log(`child exit: ${exitCode}`);
