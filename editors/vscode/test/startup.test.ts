import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { WcodeSession, type CrashInfo } from "../src/session.ts";
import { isUnsupportedStdio, unsupportedStdioMessage } from "../src/startup.ts";

const fixturesDir = resolve(dirname(fileURLToPath(import.meta.url)), "fixtures");
const repoRoot = resolve(fixturesDir, "..", "..", "..", ".."); // fixtures -> test -> vscode -> editors -> repo
const binary = process.env.WCODE_BIN ?? resolve(repoRoot, "target", "debug", "wcode");

/**
 * REAL stderr, captured: `wcode 0.3.2` (Sep 29, predates P0) run as
 * `wcode serve --stdio` — exit 2, `error: unexpected argument: --stdio`.
 */
const STALE = readFileSync(resolve(fixturesDir, "stale-wcode-stderr.txt"), "utf8");

/**
 * REAL stderr, captured: the CURRENT binary (`target/debug/wcode`) run with a
 * DIFFERENT unexpected argument. Its usage block lists `--stdio` (line 28), so
 * this is the false-positive case the classifier must not match.
 */
const OTHER_PARSE_ERROR = readFileSync(resolve(fixturesDir, "other-parse-error-stderr.txt"), "utf8");

test("classifies the REAL stale-wcode stderr as 'older than this extension'", () => {
  assert.match(STALE, /unexpected argument: --stdio/, "the fixture is the real text");
  assert.equal(isUnsupportedStdio(2, STALE), true);
});

test("does NOT classify an unrelated parse error, even though its usage lists --stdio", () => {
  assert.match(OTHER_PARSE_ERROR, /unexpected argument: --bogus/, "a different bad argument");
  assert.match(OTHER_PARSE_ERROR, /--stdio/, "…and its usage DOES mention --stdio");
  assert.equal(isUnsupportedStdio(2, OTHER_PARSE_ERROR), false, "not the same line -> not our failure");
});

test("does NOT classify an unrelated early exit", () => {
  assert.equal(isUnsupportedStdio(1, "provider: chat · base_url http://localhost:11434/v1\nerror: connect refused"), false);
  assert.equal(isUnsupportedStdio(2, 'error: model = "..." is required\n'), false);
  assert.equal(isUnsupportedStdio(null, ""), false, "a killed child leaves no evidence");
  assert.equal(isUnsupportedStdio(1, ""), false);
});

test("a clean session is never classified (no false positive)", () => {
  assert.equal(isUnsupportedStdio(0, ""), false);
  assert.equal(isUnsupportedStdio(0, STALE), false, "a clean exit is not a failure, whatever it printed");
});

test("the message names the resolved binary, the fact, and BOTH remedies", () => {
  const message = unsupportedStdioMessage("/Users/wei/.cargo/bin/wcode");
  assert.match(message, /\/Users\/wei\/\.cargo\/bin\/wcode/, "which binary");
  assert.match(message, /older than this extension/, "the fact");
  assert.match(message, /serve --stdio/, "what it lacks");
  assert.match(message, /cargo install --path crates\/wcode-cli/, "remedy 1: upgrade");
  assert.match(message, /wcode\.path/, "remedy 2: point at a newer binary");
});

/**
 * The retention that makes the classifier reachable: the evidence is stderr's
 * FIRST line, and a 72-line usage dump is longer than the 40-line cap. A
 * tail-only buffer drops the evidence and the classifier never fires.
 *
 * `serve --stdio --bogus` gives a REAL >40-line stderr whose first line is the
 * error and whose usage block mentions `--stdio` — so this pins both the
 * retention AND the line-scoping, live.
 */
test("the evidence line survives the stderr cap (live)", { skip: !existsSync(binary), timeout: 20_000 }, async () => {
  const session = new WcodeSession({ binary, args: ["--bogus"], logger: { info: () => {}, error: () => {} } });
  const crashed = new Promise<CrashInfo>((resolve) => session.on("crash", resolve));
  session.start().catch(() => {});
  const crash = await crashed;

  const lines = crash.stderrTail.split("\n");
  console.log("stderr retention (live):");
  console.log(`  kept lines      : ${lines.length} (the cap is 40 tail + 10 head)`);
  console.log(`  first line kept : ${JSON.stringify(lines[0])}`);
  console.log(`  classifier      : ${isUnsupportedStdio(crash.code, crash.stderrTail)} (must be false: a different argument)`);

  assert.match(lines[0] ?? "", /unexpected argument: --bogus/, "the HEAD survived a long usage dump");
  assert.ok(crash.stderrTail.includes("…"), "the middle was elided — the cap really engaged");
  assert.ok(lines.length < 81, `kept ${lines.length} of 81 lines, so it is bounded`);
  // The pure test above covers the false positive with the FULL stderr; here the
  // point is that the evidence line is the one that survived.
  assert.equal(isUnsupportedStdio(crash.code, crash.stderrTail), false, "a different argument is not our failure");

  await session.stop();
});

/**
 * The fake reproduces the REAL stale binary's shape: MORE stderr lines than the
 * tail holds, with the classifier's evidence line FIRST — 72 lines, like
 * `stale-wcode-stderr.txt`.
 *
 * **This test must NOT skip, and must NOT move into a file whose tests are
 * `skip: !existsSync(binary)`.** Its whole point is that the head+tail window is
 * exercised with NO Cargo build present; the live test above skips there, so
 * without this one "the evidence survives the cap" would be asserted rather than
 * tested, and the tail-only bug could return unnoticed.
 *
 * Cross-platform: `node` runs a fake `serve` script in a temp cwd, so no real
 * stale binary is needed. (`JSON.stringify` emits the newline escapes, so this
 * file needs no backslash gymnastics.)
 */
test("an unsupported binary is reported ONCE, and its EVIDENCE survives the cap", async () => {
  const NEWLINE = String.fromCharCode(10);
  const dir = mkdtempSync(join(tmpdir(), "wcode-fake-"));
  const fake = [
    'const lines = ["error: unexpected argument: --stdio"];',
    'for (let i = 0; i < 71; i += 1) lines.push("usage line " + i);',
    `process.stderr.write(lines.join(${JSON.stringify(NEWLINE)}) + ${JSON.stringify(NEWLINE)}, () => process.exit(2));`,
    "",
  ].join(NEWLINE);
  writeFileSync(join(dir, "serve"), fake);

  const logged: string[] = [];
  const session = new WcodeSession({
    binary: process.execPath,
    cwd: dir,
    logger: { info: (m) => logged.push(m), error: (m) => logged.push(m) },
    backoffMs: [10, 10, 10],
  });
  const crashed = new Promise<CrashInfo>((resolve) => session.on("crash", resolve));
  session.start().catch(() => {});
  const crash = await crashed;

  const kept = crash.stderrTail.split(NEWLINE);
  console.log("fake unsupported binary (72 stderr lines, evidence first):");
  console.log(`  kept lines   : ${kept.length} of 72 (the window is 10 head + 40 tail)`);
  console.log(`  kept line 1  : ${JSON.stringify(kept[0])}`);
  console.log(`  elided middle: ${crash.stderrTail.includes("…")}`);
  console.log(`  classifier   : ${isUnsupportedStdio(crash.code, crash.stderrTail)} (must be true)`);

  // The HEAD is what makes the classification work — the property the tail-only
  // buffer broke, tested here with no real binary in play.
  assert.equal(isUnsupportedStdio(crash.code, crash.stderrTail), true, "the evidence survived the cap");
  assert.match(kept[0] ?? "", /unexpected argument: --stdio/, "the FIRST line is the evidence");
  assert.ok(crash.stderrTail.includes("…"), "the splice is VISIBLE — never a silent elision");
  assert.ok(kept.length < 72, `bounded: kept ${kept.length} of 72 lines`);

  await new Promise((resolve) => setTimeout(resolve, 150)); // long past three 10ms restarts
  assert.ok(logged.some((l) => /not restarting/.test(l)), "it says WHY it stopped");
  assert.ok(!logged.some((l) => /restart 1\//.test(l)), "and does NOT respawn a binary that cannot work");
  assert.equal(session.state, "crashed");

  await session.stop();
});
