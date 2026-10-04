import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

// These modules must stay pure: no host and no I/O. That is what lets `npm test`
// drive them with no VS Code and no display, and it is what keeps I/O out of the
// view model and the host-side rendering.
// A structural guard is needed because such an import slips through every
// behavioural test — nothing observable changes until the import is used.
//
// `render.ts` may import a relative markdown module (a pure library); it must
// not import `vscode` or a `node:` builtin. `webview.ts` is even stricter: it
// imports only types (its `SessionState` comes in via `import type`, erased at
// runtime).
for (const file of [
  "reducer.ts",
  "render.ts",
  "webview.ts",
  "commands.ts",
  "diff.ts",
  "review.ts",
  "startup.ts",
  "webview/view.ts",
]) {
  test(`${file} imports no host and no I/O`, () => {
    const src = readFileSync(resolve(here, "../src", file), "utf8");

    // A module that imports anything must use `import type` for its types (Node's
    // strip-only mode erases those, but not a value import of a type). A module
    // with NO imports (`startup.ts`) has nothing to check here.
    if (/^\s*import\b/m.test(src)) {
      assert.match(src, /^\s*import\s+type\b/m, "an importing module must use `import type` for types");
    }

    // Whole-file rather than line-by-line, so a wrapped import, a dynamic
    // `import()` and an `export … from` are all caught, not just the tidy form.
    assert.doesNotMatch(
      src,
      /\b(?:from|import|require)\s*\(?\s*["'](?:vscode|node:)/,
      `${file} must import neither the host nor an I/O module`,
    );
  });
}
