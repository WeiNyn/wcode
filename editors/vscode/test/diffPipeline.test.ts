// Gate 3: the diff pipeline, headless — a REAL generator patch applied to a REAL
// file on disk, through `reverseApply`, to the before-image; plus the patch-only
// fallback for a truncated patch.
//
// The patch bytes come from `test/fixtures/diffs.ts`, which
// `scripts/verify-diff-fixtures.mjs` checks against the compiled generator.
//
// NOT verified here: `vscode.diff`, the `wcode-diff:` provider, and the webview
// button — those need VS Code (see README).
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { reverseApply } from "../src/diff.ts";
import { CAPPED, ROUND_TRIP_FIXTURES } from "./fixtures/diffs.ts";

function fixture(name: string): (typeof ROUND_TRIP_FIXTURES)[number] {
  const found = ROUND_TRIP_FIXTURES.find((f) => f.name === name);
  assert.ok(found, `fixture \`${name}\` is missing`);
  return found;
}

test("pipeline: a real patch reverse-applies to a before-image (printed)", () => {
  const dir = mkdtempSync(join(tmpdir(), "wcode-diff-"));
  const afterPath = join(dir, "f.txt");
  const { new: postImage, diff, old: preImage } = fixture("changed_line");

  // The on-disk file IS the post-image the generator diffed against.
  writeFileSync(afterPath, postImage, "utf8");

  const current = readFileSync(afterPath, "utf8");
  const before = reverseApply(current, diff);
  assert.equal(before, preImage, "the before-image is recovered exactly");

  const beforePath = join(dir, "f.before.txt");
  assert.ok(before !== null);
  writeFileSync(beforePath, before, "utf8");
  assert.equal(readFileSync(beforePath, "utf8"), preImage, "and it is what we serve as the left side");

  console.log("diff pipeline (reconstructed):");
  console.log(`  patch  : ${JSON.stringify(diff)}`);
  console.log(`  before : ${JSON.stringify(readFileSync(beforePath, "utf8"))}`);
  console.log(`  after  : ${JSON.stringify(readFileSync(afterPath, "utf8"))}`);
  console.log(`  title  : wcode: f.txt (before → after)`);
});

test("pipeline: a truncated patch falls back to the patch text (printed)", () => {
  const dir = mkdtempSync(join(tmpdir(), "wcode-diff-"));
  const afterPath = join(dir, "f.txt");
  writeFileSync(afterPath, CAPPED.new, "utf8");

  const current = readFileSync(afterPath, "utf8");
  const before = reverseApply(current, CAPPED.diff);
  assert.equal(before, null, "the 40-line cap makes an exact reconstruction impossible");

  // The host's decision (`diffProvider.openDiff`): null -> open the patch text,
  // read-only, rather than a plausible wrong window.
  console.log("diff pipeline (patch only):");
  console.log(`  reverseApply -> null (truncated: the omitted body lines are unknown)`);
  console.log(`  title  : wcode: f.txt (patch only)`);
  console.log(`  patch  : ${JSON.stringify(CAPPED.diff)}`);
});

test("pipeline: a CRLF file on disk round-trips to a CRLF before-image", () => {
  const dir = mkdtempSync(join(tmpdir(), "wcode-diff-"));
  const afterPath = join(dir, "f.txt");
  const { new: postImage, diff, old: preImage } = fixture("crlf");
  writeFileSync(afterPath, postImage, "utf8");

  const before = reverseApply(readFileSync(afterPath, "utf8"), diff);
  assert.equal(before, preImage);
  assert.ok(before?.includes("\r\n"), "the ending is preserved on disk, not normalized");
});
