import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

/**
 * Structural guardrails for the multi-tab refactor (D013 / W010 P2). The
 * `Manager`↔`SessionTab`↔`ChatPanel` wiring imports `vscode` and cannot run under
 * `node --test` (no VS Code host), so these source assertions are the cheap proof
 * that the SINGLETON pattern did not creep back — the exact regression the feature
 * exists to remove. Behavioural proof is the F5 click-path (W010 §7.2).
 */

const here = dirname(fileURLToPath(import.meta.url));
const src = (file: string): string => readFileSync(resolve(here, "../src", file), "utf8");

test("panel.ts is not a singleton any more (D013)", () => {
  assert.doesNotMatch(src("panel.ts"), /static\s+current/, "ChatPanel must keep no `static current`");
  assert.match(src("panel.ts"), /static\s+open\s*\(/, "ChatPanel.open opens a FRESH panel");
});

test("extension.ts holds no per-session module globals (they moved into SessionTab)", () => {
  const extension = src("extension.ts");
  for (const gone of [
    /^\s*let\s+session\b/m,
    /^\s*let\s+viewState\b/m,
    /^\s*let\s+controller\b/m,
    /^\s*let\s+sessionArgs\b/m,
    /^\s*let\s+userStopped\b/m,
    /^\s*let\s+hydrated\b/m,
    /^\s*let\s+reviewVerdicts\b/m,
  ]) {
    assert.doesNotMatch(extension, gone, `extension.ts must not keep ${gone}`);
  }
  assert.match(extension, /new Manager\(/, "extension.ts is the composition root: it builds a Manager");
});

test("the per-tab bundle and its registry exist (W010 P1/P2)", () => {
  assert.match(src("tab.ts"), /export class SessionTab\b/);
  assert.match(src("manager.ts"), /export class Manager\b/);
  assert.match(src("manager.ts"), /implements vscode\.Disposable, ViewBinder\b/);
});

test("package.json contributes the distinct new-session verb (D013 B)", () => {
  const pkg = JSON.parse(readFileSync(resolve(here, "../package.json"), "utf8")) as {
    activationEvents: string[];
    contributes: { commands: Array<{ command: string }> };
  };
  assert.ok(
    pkg.contributes.commands.some((c) => c.command === "wcode.newSession"),
    "wcode.newSession must be a contributed command",
  );
  assert.ok(pkg.activationEvents.includes("onCommand:wcode.newSession"));
});

test("every tab log line is prefixed with the tab label (W010 P4)", () => {
  const tab = src("tab.ts");
  const raw = [...tab.matchAll(/this\.deps\.output\.appendLine\(/g)];
  assert.equal(
    raw.length,
    1,
    "appendLine is called in exactly ONE place — the `log` helper (add `this.log(…)` elsewhere)",
  );
  assert.match(tab, /private log\(line: string\): void \{/, "SessionTab carries a `log` helper");
  assert.match(tab, /`\[\$\{this\.label\}\] \$\{line\}`/, "the helper prefixes the tab's label");
});
