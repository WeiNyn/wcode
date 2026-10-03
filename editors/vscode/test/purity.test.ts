import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

// The reducer must stay pure: no host and no I/O. That is what lets `npm test` drive
// it with no VS Code and no display, and it is what keeps I/O out of the view model.
// A structural guard is needed because such an import slips through every behavioural
// test — nothing observable changes until the import is actually used.
test("reducer.ts imports no host and no I/O", () => {
  const src = readFileSync(resolve(here, "../src/reducer.ts"), "utf8");

  assert.match(src, /^\s*import\s+type\b/m, "expected at least one `import type`");

  // Whole-file rather than line-by-line, so a wrapped import, a dynamic `import()`
  // and an `export … from` are all caught, not just the tidy one-line form.
  assert.doesNotMatch(
    src,
    /\b(?:from|import|require)\s*\(?\s*["'](?:vscode|node:)/,
    "reducer.ts must import neither the host nor an I/O module",
  );
});
