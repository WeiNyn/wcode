/**
 * Diff fixtures — the EXACT strings `crates/wcode-cli/src/tools/diff.rs::unified`
 * produces. Transcribed from that function's own test expectations and its
 * construction, NOT from prose: a fixture written from a description is a second
 * guess at the format, and a wrong-format fixture would make a wrong parser pass.
 *
 * `source` names the Rust test each one is pinned by. The generator's tests
 * assert *substrings* (a header prefix, `\n-l3`, `more lines`), so where a test
 * pins only part of the string, the rest is the generator's construction read
 * from `diff.rs` itself — said so, per fixture. Anything I could not source this
 * way is labelled `hand-written`.
 */

export interface DiffFixture {
  name: string;
  /** Which `diff.rs` test pins this, or `hand-written`. */
  source: string;
  /** The generator's `old` argument (the pre-image we must recover). */
  old: string;
  /** The generator's `new` argument (the on-disk post-image). */
  new: string;
  /** The generator's output, byte for byte. */
  diff: string;
}

const lines = (prefix: string, count: number): string =>
  Array.from({ length: count }, (_, i) => `${prefix}${i}`).join("\n") + "\n";

/** A fixture whose diff must NOT reverse-apply. */
export interface NullDiffFixture {
  name: string;
  source: string;
  diff: string;
  /** The current text to apply against; defaults to the changed-line post-image. */
  current?: string;
}

/** The post-image of `changedLine` — the default `current` for the null cases. */
export const CHANGED_LINE_CURRENT = "l1\nl2\nX\nl4\nl5\n";

/** Round-trips: `reverseApply(fixture.new, fixture.diff) === fixture.old`. */
export const ROUND_TRIP_FIXTURES: DiffFixture[] = [
  {
    // `a_changed_line_shows_context_minus_and_plus` pins the header
    // (`starts_with("@@ -1,5 +1,5 @@")`), `\n-l3`, `\n+X` and `\n l2`; the body
    // order is the generator's construction (pre-context, `-`, `+`, post-context).
    name: "changed_line",
    source: "diff.rs::tests::a_changed_line_shows_context_minus_and_plus",
    old: "l1\nl2\nl3\nl4\nl5\n",
    new: "l1\nl2\nX\nl4\nl5\n",
    diff: "@@ -1,5 +1,5 @@\n l1\n l2\n-l3\n+X\n l4\n l5",
  },
  {
    // `a_new_file_is_all_additions` pins `starts_with("@@ -1,0 +1,2 @@")` and
    // `+a`/`+b`; `write` passes `old = ""` for a new file (`write.rs`).
    name: "new_file",
    source: "diff.rs::tests::a_new_file_is_all_additions",
    old: "",
    new: "a\nb\n",
    diff: "@@ -1,0 +1,2 @@\n+a\n+b",
  },
  {
    // `an_insertion_is_add_only` pins `\n+c` and `!contains("\n-")`; the header
    // follows from the construction (start = 2, no removals).
    name: "insertion",
    source: "diff.rs::tests::an_insertion_is_add_only",
    old: "a\nb\n",
    new: "a\nb\nc\n",
    diff: "@@ -1,2 +1,3 @@\n a\n b\n+c",
  },
  {
    // CRLF: `str::lines()` strips the trailing `\r`, so the diff body carries no
    // `\r`. No Rust test covers it -> hand-written to the generator's shape.
    name: "crlf",
    source: "hand-written (generator construction; no Rust test covers it)",
    old: "l1\r\nl2\r\nl3\r\n",
    new: "l1\r\nl2\r\nX\r\n",
    diff: "@@ -1,3 +1,3 @@\n l1\n l2\n-l3\n+X",
  },
  {
    // A removed line whose CONTENT starts with `--` arrives as `---x` (the
    // generator prefixes a single `-`). The `---`/`+++` header check must not
    // swallow it — only the `diff -u` header SHAPES (`--- `, `+++ `) are foreign.
    name: "removed_line_starting_with_dashes",
    source: "hand-written (generator construction: format!(\"-{line}\"))",
    old: "a\n--x\n",
    new: "a\ny\n",
    diff: "@@ -1,2 +1,2 @@\n a\n---x\n+y",
  },
  {
    // A removed line whose CONTENT is exactly `--` arrives as `---` (no space).
    // Legitimate generator output, not a `diff -u` header.
    name: "removed_line_content_is_dashes",
    source: "hand-written (generator construction: format!(\"-{line}\"))",
    old: "a\n--\n",
    new: "a\ny\n",
    diff: "@@ -1,2 +1,2 @@\n a\n---\n+y",
  },
  {
    // A removed line whose content starts with `-- ` (a SQL comment) arrives as
    // `--- a SQL comment` — indistinguishable from a `diff -u` header by prefix
    // alone, which is exactly why the prefix check was wrong.
    name: "removed_line_content_starts_with_dashes",
    source: "hand-written (generator construction: format!(\"-{line}\"))",
    old: "a\n-- a SQL comment\n",
    new: "a\ny\n",
    diff: "@@ -1,2 +1,2 @@\n a\n--- a SQL comment\n+y",
  },
  {
    // An ADDED line whose content starts with `++ ` arrives as `+++ …`.
    name: "added_line_content_starts_with_plus_plus",
    source: "hand-written (generator construction: format!(\"+{line}\"))",
    old: "a\n",
    new: "a\n++ b/f.txt\n",
    diff: "@@ -1,1 +1,2 @@\n a\n+++ b/f.txt",
  },
];

/**
 * The degenerate all-context hunk: `old` and `new` differ ONLY by the final
 * newline, so `.lines()` yields identical vectors and the body is context only.
 * No Rust test covers it -> hand-written to the generator's shape.
 *
 * Its reverse-apply is the IDENTITY (`reverseApply(new, diff) === new`): a text
 * patch cannot express a trailing-newline change, so there is nothing to undo.
 */
export const IDENTITY_FIXTURE: DiffFixture = {
  name: "context_only_trailing_newline",
  source: "hand-written (generator construction; no Rust test covers it)",
  old: "a\nb",
  new: "a\nb\n",
  diff: "@@ -1,2 +1,2 @@\n a\n b",
};

/** The 40-line cap (`the_body_is_capped`), which is NOT reverse-appliable. */
export const CAPPED: DiffFixture = {
  // `the_body_is_capped` pins `lines <= 1 + MAX_LINES + 1` and `contains("more
  // lines")`; the 40 `-` lines and the `… (+160 more lines)` count follow from
  // the construction (100 old + 100 new body lines, truncated to 40).
  name: "capped",
  source: "diff.rs::tests::the_body_is_capped",
  old: lines("l", 100),
  new: lines("x", 100),
  diff: `@@ -1,100 +1,100 @@\n${Array.from({ length: 40 }, (_, i) => `-l${i}`).join("\n")}\n… (+160 more lines)`,
};

/** Shapes that must NOT reverse-apply, each with the reason. */
export const NULL_FIXTURES: NullDiffFixture[] = [
  {
    // Counts are ALWAYS present in our generator. `@@ -1 +1 @@` is the shape
    // `wcode-tui`'s own test uses; it is a FOREIGN patch, never generator output.
    // ISOLATED: `current` is the post-image the body implies, so if the counts
    // were ever inferred instead of required, the rest would succeed and return
    // a plausible but wrong pre-image.
    name: "count_less_header",
    source: "hand-written (foreign shape, as used by ui.rs's test)",
    diff: "@@ -1 +1 @@\n-old\n+new",
    current: "new\n",
  },
  {
    // ISOLATED for the line-0 rule: `current` is the post-image the body implies,
    // so only "line 0 must be `@@`" rejects it (a parser that skipped leading
    // `---`/`+++` lines would return a plausible but wrong pre-image).
    name: "diff_u_file_headers",
    source: "hand-written (a `diff -u` patch: its first line is not our header)",
    diff: "--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-a\n+b",
    current: "b\n",
  },
  {
    // ISOLATED: `current` is the post-image the body implies, so only the
    // foreign-line rejection (the prefix dispatch's `else`) rejects it.
    name: "no_newline_marker",
    source: "hand-written (we never emit `\\ No newline at end of file`)",
    diff: "@@ -1,1 +1,1 @@\n-a\n+b\n\\ No newline at end of file",
    current: "b\n",
  },
  {
    // Our generator emits the SAME start on both sides.
    name: "new_from_mismatch",
    source: "hand-written (foreign: the new side starts elsewhere)",
    diff: "@@ -1,2 +3,2 @@\n a\n b",
    // Chosen so the lines DO match at `from - 1`: without the new-side-start
    // check this would return a plausible but wrong pre-image instead of null.
    current: "a\nb\n",
  },
  {
    // The header claims 3 old lines; the body has 2. `current` is chosen so that
    // ONLY the count check rejects it (the new side matches the body and the
    // slice does match) — otherwise the slice-length check would reject it first
    // and the count check would stay unpinned.
    name: "count_mismatch",
    source: "hand-written (the header claims 3 old lines, the body has 2)",
    diff: "@@ -1,3 +1,2 @@\n a\n-b\n+B",
    current: "a\nB\n",
  },
  {
    // ISOLATED: same reason as `no_newline_marker`.
    name: "stray_blank_body_line",
    source: "hand-written (a blank line is not a body prefix)",
    diff: "@@ -1,2 +1,2 @@\n a\n\n b",
    current: "a\nb\n",
  },
  {
    name: "empty",
    source: "hand-written",
    diff: "",
  },
  {
    name: "whitespace_only",
    source: "hand-written",
    diff: "   \n",
  },
  {
    // The truncation marker, ISOLATED: the counts are hand-set to match the
    // 40-line body. (The REAL capped diff's header says 100, which the count
    // check rejects first — `CAPPED` is covered separately and is not isolating.)
    name: "truncation_marker",
    source: "hand-written (isolating variant of the_body_is_capped)",
    diff: `@@ -1,40 +1,0 @@\n${Array.from({ length: 40 }, (_, i) => `-l${i}`).join("\n")}\n… (+160 more lines)`,
    current: "z\n",
  },
  {
    // The context no longer matches: the file moved on since the edit.
    name: "context_mismatch",
    source: "hand-written (the Friction case: a later edit changed the file)",
    diff: "@@ -1,5 +1,5 @@\n l1\n l2\n-l3\n+X\n l4\n l5",
    current: "l1\nl2\nY\nl4\nl5\n",
  },
  {
    name: "hunk_past_end_of_file",
    source: "hand-written (the hunk does not fit the current text)",
    diff: "@@ -1,5 +1,5 @@\n l1\n l2\n-l3\n+X\n l4\n l5",
    current: "l1\nl2\nX\n",
  },
  {
    name: "mixed_line_endings",
    source: "hand-written (the ending must be uniform)",
    diff: "@@ -1,5 +1,5 @@\n l1\n l2\n-l3\n+X\n l4\n l5",
    current: "l1\r\nl2\nX\r\nl4\nl5\n",
  },
];
