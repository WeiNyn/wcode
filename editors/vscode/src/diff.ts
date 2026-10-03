/**
 * Reverse-apply one wcode presentation diff.
 *
 * PURE — no `vscode`, no I/O — so plain node can drive it (a diff is a *text
 * patch*, so a wrong answer is a silently wrong window; the pure half is the
 * half worth testing).
 *
 * **The generator is the spec** (`crates/wcode-cli/src/tools/diff.rs::unified`):
 * exactly ONE hunk, header `@@ -{from},{old_count} +{from},{new_count} @@` —
 * the SAME `from` on both sides, counts always present; body lines prefixed
 * ` `/`-`/`+`; no `\ No newline at end of file` marker (a `---`/`+++` BODY line
 * is legitimate — a removed line whose content starts with `--`); at most
 * 3 context lines each side; a 40-line body cap ending in `… (+N more lines)`;
 * and `None` (no field at all) when the inputs are byte-identical.
 *
 * Every case that cannot be determined returns `null` — never a guess.
 */
import type { ToolBlock, ViewState } from "./reducer.ts";

/** The URI scheme a reconstructed before-image is served under. */
export const DIFF_SCHEME = "wcode-diff";

/** The path a token occupies in a `wcode-diff:` URI. */
export function diffTokenPath(token: string): string {
  return `/${token}`;
}

/** The token inside a `wcode-diff:` URI's path. */
export function tokenFromDiffPath(path: string): string {
  return path.startsWith("/") ? path.slice(1) : path;
}

/**
 * Reconstruct the pre-image: reverse-apply one wcode unified diff to
 * `currentText` (the on-disk post-image). Returns the before-text, or `null`
 * when it cannot be done EXACTLY.
 */
export function reverseApply(currentText: string, unifiedDiff: string): string | null {
  const lines = unifiedDiff.split("\n");
  const header = lines[0];
  // Counts are ALWAYS present in our generator; a count-less `@@ -1 +1 @@` is a
  // foreign patch -> null.
  const match = /^@@ -(\d+),(\d+) \+(\d+),(\d+) @@$/.exec(header);
  if (match === null) return null;
  const from = Number(match[1]);
  const oldCount = Number(match[2]);
  const newFrom = Number(match[3]);
  const newCount = Number(match[4]);
  // Our generator emits the same start on both sides. A different new-side start
  // would splice at the wrong offset, and if the lines happened to match there
  // it would return a plausible but WRONG pre-image.
  if (newFrom !== from) return null;

  const oldLines: string[] = [];
  const newLines: string[] = [];
  for (const line of lines.slice(1)) {
    // A body line must start with ` `, `-` or `+`. Anything else is rejected: the
    // `\ No newline at end of file` marker (never ours), the `… (+N more lines)`
    // truncation marker (ours, but unrecoverable), a stray blank line. (No separate
    // branches for those: they are not ` `/`-`/`+`, so this `else` is the check —
    // a dedicated branch would be unreachable as a distinct rejection.)
    //
    // NOT a `---`/`+++` line. The generator prefixes a REMOVED line with a single
    // `-`, so a removed line whose content is `--` arrives as `---`, and one whose
    // content starts with `-- ` (a SQL comment) arrives as `--- …`. A real
    // `diff -u` patch is already rejected at line 0 (it does not open with `@@`),
    // so a `---`/`+++` line in the BODY is always legitimate generator output.
    if (line.startsWith(" ")) {
      oldLines.push(line.slice(1));
      newLines.push(line.slice(1));
    } else if (line.startsWith("-")) {
      oldLines.push(line.slice(1));
    } else if (line.startsWith("+")) {
      newLines.push(line.slice(1));
    } else {
      return null;
    }
  }
  if (oldLines.length !== oldCount || newLines.length !== newCount) return null;

  const file = splitFile(currentText);
  if (file === null) return null; // a mixed-ending (or lone-CR) file: never guess

  const start = from - 1;
  // A hunk starting before line 1 is foreign — and do not lean on `slice(-1)`,
  // which counts from the end. Past-the-end is caught by the length check below.
  if (start < 0) return null;
  const slice = file.lines.slice(start, start + newCount);
  // The general "the hunk does not fit" guard: a past-the-end hunk yields a short
  // slice. Load-bearing — keep it pinned by `hunk_past_end_of_file`.
  if (slice.length !== newLines.length) return null;
  for (let i = 0; i < slice.length; i += 1) {
    // The context no longer matches: the file moved on since the edit.
    if (slice[i] !== newLines[i]) return null;
  }

  const spliced = [
    ...file.lines.slice(0, start),
    ...oldLines,
    ...file.lines.slice(start + newCount),
  ];
  return joinLines(spliced, file.eol, file.finalEol);
}

/** The tool block with this `call_id`, if the transcript still has it. */
export function findToolBlock(state: ViewState, callId: string): ToolBlock | undefined {
  for (let i = state.transcript.length - 1; i >= 0; i -= 1) {
    const block = state.transcript[i];
    if (block.kind === "tool" && block.tool?.callId === callId) return block.tool;
  }
  return undefined;
}

/** The most recent tool call that carries a diff (the `wcode.openDiff` command). */
export function lastDiffCallId(state: ViewState): string | undefined {
  for (let i = state.transcript.length - 1; i >= 0; i -= 1) {
    const block = state.transcript[i];
    if (block.kind === "tool" && block.tool && block.tool.diff !== undefined && block.tool.diff !== "") {
      return block.tool.callId;
    }
  }
  return undefined;
}

/* ------------------------------------------------------------------ line split */

interface FileLines {
  lines: string[];
  /** The uniform line ending, used between lines. */
  eol: string;
  /** The ending after the last line — `""` when the file has no final newline. */
  finalEol: string;
}

/**
 * Split `text` into lines + endings, or `null` when the endings are MIXED.
 *
 * Matching happens in LF-space: the generator splits with `str::lines()`, which
 * strips a trailing `\r`, so a CRLF file's diff lines carry no `\r`. We strip
 * the terminators on both sides for the match and re-emit with the ending the
 * file actually uses.
 */
function splitFile(text: string): FileLines | null {
  const parts: Array<{ text: string; eol: string }> = [];
  let current = "";
  let endedWithTerminator = false;
  let i = 0;
  while (i < text.length) {
    if (text[i] === "\r" && text[i + 1] === "\n") {
      parts.push({ text: current, eol: "\r\n" });
      current = "";
      endedWithTerminator = true;
      i += 2;
    } else if (text[i] === "\n") {
      parts.push({ text: current, eol: "\n" });
      current = "";
      endedWithTerminator = true;
      i += 1;
    } else {
      current += text[i];
      endedWithTerminator = false;
      i += 1;
    }
  }
  if (text.length > 0 && !endedWithTerminator) parts.push({ text: current, eol: "" });

  const endings = new Set(parts.map((p) => p.eol).filter((eol) => eol !== ""));
  if (endings.size > 1) return null; // mixed endings: the ending must be uniform

  return {
    lines: parts.map((p) => p.text),
    eol: endings.size === 1 ? [...endings][0] : "\n",
    finalEol: parts.length > 0 ? parts[parts.length - 1].eol : "",
  };
}

function joinLines(lines: string[], eol: string, finalEol: string): string {
  if (lines.length === 0) return "";
  return lines.join(eol) + finalEol;
}

/* -------------------------------------------------------------- before registry */

/**
 * A token → pre-image registry. The URI carries an OPAQUE, SHORT token, never
 * the text (no length/escaping limits, no file content in a URI that lands in
 * titles and the undo stack). In memory only — it MUST NEVER touch disk: the
 * whole point is that no before-blob is persisted.
 *
 * Bounded LRU: an evicted-but-still-open tab resolves to `undefined`, and the
 * provider returns `""` rather than throwing.
 */
export class BeforeRegistry {
  private readonly byToken = new Map<string, string>();
  private readonly capacity: number;
  private counter = 0;

  constructor(capacity = 32) {
    this.capacity = capacity;
  }

  /** Mint a token for `beforeText`; evicts the least-recently-used past capacity. */
  mint(beforeText: string): string {
    this.counter += 1;
    const token = `t${this.counter.toString(36)}`;
    this.byToken.set(token, beforeText);
    this.evict();
    return token;
  }

  /** The text for `token`, or `undefined` when unknown or evicted. Refreshes LRU. */
  resolve(token: string): string | undefined {
    const text = this.byToken.get(token);
    if (text === undefined) return undefined;
    this.byToken.delete(token);
    this.byToken.set(token, text);
    return text;
  }

  /** Drop every token — on session stop / panel dispose, so a restart starts clean. */
  clear(): void {
    this.byToken.clear();
  }

  get size(): number {
    return this.byToken.size;
  }

  private evict(): void {
    while (this.byToken.size > this.capacity) {
      const oldest = this.byToken.keys().next().value;
      if (oldest === undefined) return;
      this.byToken.delete(oldest);
    }
  }
}
