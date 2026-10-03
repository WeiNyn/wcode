#!/usr/bin/env node
/**
 * Prove `test/fixtures/diffs.ts` holds REAL generator output.
 *
 * `crates/wcode-cli/src/tools/diff.rs` has no dependencies, so it compiles
 * standalone: copy it, append a tiny harness that reads hex-encoded `old`/`new`
 * pairs on stdin and prints the generator's output as hex, `rustc` it, and
 * compare against the fixtures. The fixture inputs drive the harness, so the two
 * cannot drift.
 *
 * Requires `rustc`. NOT part of `npm test` — the fixtures are committed, and this
 * is the one-off check that they are not a second guess at the format.
 *
 *   node scripts/verify-diff-fixtures.mjs
 */
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { CAPPED, IDENTITY_FIXTURE, ROUND_TRIP_FIXTURES } from "../test/fixtures/diffs.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..", "..");
const generator = join(repoRoot, "crates/wcode-cli/src/tools/diff.rs");

const HARNESS = `
use std::io::Read;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    for line in input.lines() {
        let mut fields = line.split(' ');
        let old = String::from_utf8(unhex(fields.next().unwrap_or(""))).unwrap();
        let new = String::from_utf8(unhex(fields.next().unwrap_or(""))).unwrap();
        let hex: String = unified(&old, &new)
            .unwrap_or_default()
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        println!("{hex}");
    }
}
`;

const hex = (text) => Buffer.from(text, "utf8").toString("hex");

const dir = mkdtempSync(join(tmpdir(), "wcode-diff-"));
const source = join(dir, "gen.rs");
writeFileSync(source, readFileSync(generator, "utf8") + HARNESS);
const binary = join(dir, "gen");
execFileSync("rustc", ["-O", "-o", binary, source], { stdio: ["ignore", "ignore", "inherit"] });

const fixtures = [...ROUND_TRIP_FIXTURES, IDENTITY_FIXTURE, CAPPED];
const stdin = fixtures.map((f) => `${hex(f.old)} ${hex(f.new)}`).join("\n") + "\n";
const stdout = execFileSync(binary, [], { input: stdin, encoding: "utf8" });
const produced = stdout.trim().split("\n");

let failed = 0;
for (const [index, fixture] of fixtures.entries()) {
  const expected = hex(fixture.diff);
  const actual = produced[index] ?? "";
  const ok = actual === expected;
  if (!ok) failed += 1;
  console.log(`${ok ? "ok  " : "FAIL"} ${fixture.name}`);
  if (!ok) {
    console.log(`     fixture  ${JSON.stringify(fixture.diff)}`);
    console.log(`     produced ${JSON.stringify(Buffer.from(actual, "hex").toString("utf8"))}`);
  }
}

console.log(`\n${fixtures.length - failed}/${fixtures.length} fixtures match the real generator`);
process.exit(failed === 0 ? 0 : 1);
