import assert from "node:assert/strict";
import { test } from "node:test";

import { highlight, languageForPath, languageForTool } from "../src/highlight.ts";

test("highlight: a loaded language emits Prism token spans", () => {
  const html = highlight("fn main() {}", "rust");
  assert.ok(html !== null, "rust is loaded");
  assert.match(html, /class="token /, "Prism token spans");
  assert.match(html, /keyword/, "the `fn` keyword token");
});

test("highlight: a fence ALIAS resolves (ts -> typescript, sh -> bash)", () => {
  assert.match(highlight("const x: number = 1;", "ts") ?? "", /class="token /);
  assert.match(highlight("echo hi", "sh") ?? "", /class="token /);
});

test("highlight: an unknown / empty language is null (the caller falls back to escaping)", () => {
  assert.equal(highlight("x", ""), null);
  assert.equal(highlight("x", "nope"), null);
});

test("highlight: Prism escapes its own output — a <script> never survives as raw HTML", () => {
  const html = highlight("<script>alert(1)</script>", "markup");
  assert.ok(html !== null);
  assert.ok(!html.includes("<script>"), "no raw <script> tag");
  assert.ok(html.includes("&lt;"), "the angle brackets are escaped");
});

test("languageForPath: an extension maps to a loaded grammar; unknown / no ext is null", () => {
  assert.equal(languageForPath("src/a.rs"), "rust");
  assert.equal(languageForPath("a/b/c.ts"), "typescript");
  assert.equal(languageForPath("x.json"), "json");
  assert.equal(languageForPath("x.unknownext"), null);
  assert.equal(languageForPath("noext"), null);
});

test("languageForTool: bash/background run bash; a path-bearing tool by its ext; else plain", () => {
  assert.equal(languageForTool("bash", undefined), "bash");
  assert.equal(languageForTool("background", undefined), "bash");
  assert.equal(languageForTool("read", "src/a.rs"), "rust");
  assert.equal(languageForTool("edit", "a/b.py"), "python");
  assert.equal(languageForTool("grep", undefined), null, "no path -> plain");
  assert.equal(languageForTool("read", "a/notes.unknownext"), null);
});
