/**
 * Host-side syntax highlighting (W009) — pure: no `vscode`, no I/O, so plain node drives it.
 *
 * `highlight` returns Prism's HTML (`class="token …"`; PRISM escapes its own output, so the
 * webview may insert it via `innerHTML`) or `null` when the language is unknown / not loaded —
 * the caller then falls back to escaped text. Prism + a CURATED grammar set are bundled into
 * `out/extension.js` (the HOST bundle, beside `markdown-it`); the webview stays
 * dependency-free and only inserts the pre-rendered HTML.
 */
import Prism from "prismjs";
import type { Grammar } from "prismjs";
import "prismjs/components/prism-markup.js";
import "prismjs/components/prism-css.js";
import "prismjs/components/prism-javascript.js";
import "prismjs/components/prism-typescript.js";
import "prismjs/components/prism-json.js";
import "prismjs/components/prism-bash.js";
import "prismjs/components/prism-rust.js";
import "prismjs/components/prism-python.js";
import "prismjs/components/prism-toml.js";
import "prismjs/components/prism-yaml.js";
import "prismjs/components/prism-markdown.js";

/** A fence-info alias → a loaded Prism grammar name. */
const ALIASES: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  yml: "yaml",
  rs: "rust",
  py: "python",
  md: "markdown",
  html: "markup",
  htm: "markup",
  xml: "markup",
};

/** A file extension → a loaded Prism grammar name. */
const EXTENSIONS: Record<string, string> = {
  rs: "rust",
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  json: "json",
  py: "python",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  md: "markdown",
  markdown: "markdown",
  css: "css",
  html: "markup",
  htm: "markup",
};

/**
 * Normalize a fence info string / grammar name to a LOADED Prism language, or `null`. The
 * alias map is the webview-agnostic analogue of the TUI's `fence_syntax`
 * (`crates/wcode-tui/src/markdown.rs:554`).
 */
function resolveLanguage(name: string): string | null {
  const key = name.trim().toLowerCase();
  if (key === "") return null;
  const resolved = ALIASES[key] ?? key;
  const grammar = (Prism.languages as Record<string, Grammar | undefined>)[resolved];
  return grammar === undefined ? null : resolved;
}

/** Highlight `code` as `lang`; Prism's HTML (`class="token …"`) or `null` when unknown. Pure. */
export function highlight(code: string, lang: string): string | null {
  const resolved = resolveLanguage(lang);
  if (resolved === null) return null;
  const grammar = (Prism.languages as Record<string, Grammar>)[resolved];
  return Prism.highlight(code, grammar, resolved);
}

const TOKEN_TAG = /<\/?span[^>]*>/g;

/**
 * Split Prism's HTML into one WELL-FORMED fragment per line: a span left open at a line end is
 * closed there and RE-OPENED on the next line, so a multi-line construct (a block comment, a
 * template literal) keeps its colour on every continuation line and each fragment is a valid
 * HTML insertion. Pure.
 */
export function splitHighlightedLines(html: string): string[] {
  const open: string[] = [];
  return html.split("\n").map((line) => {
    const prefix = open.join("");
    TOKEN_TAG.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = TOKEN_TAG.exec(line)) !== null) {
      if (match[0].startsWith("</")) open.pop();
      else open.push(match[0]);
    }
    return `${prefix}${line}${open.map(() => "</span>").join("")}`;
  });
}

/**
 * Highlight a BODY of lines as one unit and split it back per line (a diff body): the
 * join-then-split preserves multi-line context, and `splitHighlightedLines` re-opens the
 * continuation spans. `undefined` when the language is unknown (the caller shows plain text).
 */
export function highlightLines(texts: string[], lang: string | null): string[] | undefined {
  if (lang === null) return undefined;
  const html = highlight(texts.join("\n"), lang);
  return html === null ? undefined : splitHighlightedLines(html);
}

/** The language for a file PATH, by extension; `null` when unknown. Pure. */
export function languageForPath(path: string): string | null {
  const dot = path.lastIndexOf(".");
  if (dot < 0) return null;
  return EXTENSIONS[path.slice(dot + 1).toLowerCase()] ?? null;
}

/**
 * The language for a tool call (W009's resolution rule): `bash`/`background` run bash; a
 * path-bearing tool is highlighted by its file's extension; everything else falls back to the
 * path — so `grep`/`find` (no path) stay plain. Pure.
 */
export function languageForTool(name: string, path: string | undefined): string | null {
  if (name === "bash" || name === "background") return "bash";
  return path === undefined || path === "" ? null : languageForPath(path);
}
