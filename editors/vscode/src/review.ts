/**
 * The in-panel change-review model — PURE (no `vscode`, no I/O), listed in
 * test/purity.test.ts. P4 of the VS Code surface UI/UX rework.
 *
 * It does TWO things, both pure:
 *   1. parse ONE wcode presentation diff into a renderable hunk (`reviewHunk`) —
 *      the generator is the single-hunk spec (`diff.ts` iwLrr; `diff.rs::unified`);
 *   2. model a per-callId VERDICT (`Verdict` / `Verdicts`) for Accept / Reject.
 *
 * The HOST performs the reject WRITE (`diffProvider.ts::revertDiff`); the webview
 * renders from `ToWebview.verdicts`. This module never touches the filesystem.
 *
 * Scope (plan §4 P4): ONE change per tool call — a wcode diff is a SINGLE hunk, so
 * "Accept all / Reject all", the per-file list and the plan-review screen are
 * DEFERRED (they need a multi-change batch that does not exist yet).
 */

/** One rendered line of the hunk (draft `.ln.ctx|add|del`). */
export interface ReviewLine {
  /** `ctx` = context, `add` = inserted, `del` = removed. */
  kind: "ctx" | "add" | "del";
  /** The 1-based gutter number: the NEW file for `ctx`/`add`, the OLD file for `del`. */
  number: number;
  /** The line text WITHOUT the leading ` `/`-`/`+` marker. */
  text: string;
}

/** The single hunk of a wcode diff, ready to render (draft `.hunk`). */
export interface ReviewHunk {
  /** Line 0, the raw `@@ -a,b +c,d @@` header (shown in `.hhead`). */
  header: string;
  lines: ReviewLine[];
  added: number;
  removed: number;
  /** The generator truncated the body (`… (+N more lines)`); the counts are a lower bound. */
  truncated: boolean;
}

/**
 * Parse ONE wcode presentation diff into a renderable hunk. Pure.
 *
 * The generator (`diff.ts` iwLrr) emits EXACTLY one hunk:
 *   line 0     = `@@ -{from},{old} +{from},{new} @@`  (counts always present, SAME `from`)
 *   body lines = ` `/`-`/`+` prefixed
 *   the 40-line cap ends in a `… (+N more lines)` marker (NOT a ` `/`-`/`+` line)
 * Returns `null` for anything that is not exactly one wcode hunk — mirroring
 * `reverseApply`'s strictness (never guess).
 *
 * Line numbering: both sides start at `from`; a `ctx`/`add` line advances the NEW
 * side, a `ctx`/`del` line advances the OLD side. `ctx`/`add` show the new number,
 * `del` the old number.
 */
export function reviewHunk(diff: string): ReviewHunk | null {
  // The generator's diff ends in a newline; drop the one trailing empty field so
  // it is not mistaken for a malformed body line.
  const text = diff.endsWith("\n") ? diff.slice(0, -1) : diff;
  const lines = text.split("\n");
  const header = lines[0] ?? "";
  const match = /^@@ -(\d+),(\d+) \+(\d+),(\d+) @@$/.exec(header);
  if (match === null) return null;

  const oldFrom = Number(match[1]);
  const newFrom = Number(match[3]);
  // Our generator uses the SAME start on both sides; a different one is foreign.
  if (oldFrom !== newFrom) return null;

  const out: ReviewLine[] = [];
  let oldLine = oldFrom;
  let newLine = newFrom;
  let added = 0;
  let removed = 0;
  let truncated = false;

  for (const raw of lines.slice(1)) {
    if (raw.startsWith(" ")) {
      out.push({ kind: "ctx", number: newLine, text: raw.slice(1) });
      oldLine += 1;
      newLine += 1;
    } else if (raw.startsWith("-")) {
      out.push({ kind: "del", number: oldLine, text: raw.slice(1) });
      oldLine += 1;
      removed += 1;
    } else if (raw.startsWith("+")) {
      out.push({ kind: "add", number: newLine, text: raw.slice(1) });
      newLine += 1;
      added += 1;
    } else if (raw.startsWith("…")) {
      // The 40-line cap tail `… (+N more lines)` — a marker, NOT a line.
      truncated = true;
    } else {
      // Anything else (a stray blank, a second `@@`, a foreign patch): not one hunk.
      return null;
    }
  }

  return { header, lines: out, added, removed, truncated };
}

/** A change's review verdict. Client-local; NEVER persisted, not a kernel field. */
export type Verdict = "pending" | "accepted" | "rejected";

/** Verdicts keyed by callId (one change per tool call). A JSON-safe record for the wire. */
export type Verdicts = Readonly<Record<string, Verdict>>;

/** Set one callId's verdict, returning a NEW record (the input is untouched). Pure. */
export function setVerdict(verdicts: Verdicts, callId: string, verdict: Verdict): Record<string, Verdict> {
  return { ...verdicts, [callId]: verdict };
}

/** The verdict for a callId, defaulting to `"pending"`. Pure. */
export function verdictOf(verdicts: Verdicts, callId: string): Verdict {
  return verdicts[callId] ?? "pending";
}

/**
 * The ONE source of the review block's class + settled badge label. Pure.
 *   "pending"  -> { className: "",         label: "" }
 *   "accepted" -> { className: "accepted", label: "accepted" }
 *   "rejected" -> { className: "rejected", label: "reverted" }   // the disk edit was undone
 */
export function verdictUi(verdict: Verdict): { className: string; label: string } {
  switch (verdict) {
    case "pending":
      return { className: "", label: "" };
    case "accepted":
      return { className: "accepted", label: "accepted" };
    case "rejected":
      return { className: "rejected", label: "reverted" };
  }
}
