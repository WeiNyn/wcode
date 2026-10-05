#!/usr/bin/env node
/**
 * A structural CSS guard.
 *
 * `npm run build` bundles only `media/chat.js` and NEVER parses `media/chat.css`, so a
 * structural slip in the sheet ships silently — the V12 review caught an orphaned comment
 * tail (a stray comment-closing token, raw before `.mini`). This checks the sheet's
 * STRUCTURE (not its semantics) and exits non-zero on a break:
 *   1. braces balance;
 *   2. no rogue comment-closing token outside a comment;
 *   3. no text outside a comment/rule: at depth 0 a line opens a rule (ends `{`, or `,`
 *      for a multi-selector list); inside a rule it is a declaration (`;`) or a brace.
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const mediaDir = resolve(here, "..", "media");
const CLOSE = "*/";

let failed = false;
const fail = (file, line, message) => {
  console.error(`${file}${line === 0 ? "" : `:${line}`}: ${message}`);
  failed = true;
};

const sheets = readdirSync(mediaDir).filter((name) => name.endsWith(".css"));
if (sheets.length === 0) fail("media", 0, "no stylesheet found");

for (const name of sheets) {
  const src = readFileSync(join(mediaDir, name), "utf8");

  // 1. Braces balance — a stray brace shifts the meaning of every rule after it.
  const open = (src.match(/\{/g) ?? []).length;
  const close = (src.match(/\}/g) ?? []).length;
  if (open !== close) fail(name, 0, `unbalanced braces: ${open} \`{\` vs ${close} \`}\``);

  // 2 + 3. Blank the comments (keeping the newlines, so line numbers stay true), then walk.
  const stripped = src.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, " "));
  let depth = 0;
  stripped.split("\n").forEach((raw, index) => {
    const line = raw.trim();
    if (line === "") return;
    const atDepth = depth;
    for (const ch of line) {
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
    }
    if (line.includes(CLOSE)) fail(name, index + 1, "rogue comment-closing token outside a comment");
    if (depth < 0) fail(name, index + 1, "a `}` with no opening `{`");
    const ends = line.endsWith("{") || line.endsWith("}") || line.endsWith(";") || line.endsWith(",");
    if (!ends) {
      fail(name, index + 1, atDepth === 0 ? "text outside a rule" : "unterminated declaration");
    }
  });
}

if (failed) process.exit(1);
console.log(`check-css: ok (${sheets.join(", ")})`);
