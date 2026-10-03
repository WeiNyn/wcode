/**
 * Diagnose a child that died at startup.
 *
 * **Pure** — no `vscode`, no I/O — so plain node drives it: the classifier is the
 * testable half of the crash path.
 *
 * The `--stdio` transport shipped *with this extension*, so anyone who installed
 * wcode before it has a binary on `PATH` that rejects the flag. We detect **that
 * condition**, not a version string: a `cargo run` build reports the workspace
 * version while a release may report something else, so comparing semver is
 * brittle. The child's own stderr is the evidence.
 */

/**
 * True when the child failed because its `wcode` predates `serve --stdio`.
 *
 * The signal is the **error line** naming `--stdio` as an unexpected argument, on
 * a NON-zero exit. It must be the SAME line: a current `wcode`'s usage block lists
 * `--stdio` too, so a parse error for some *other* argument would otherwise match
 * (that is a false positive, and there is a test for it).
 *
 * An unrelated early exit — a missing model, a bad config, a killed child —
 * names neither.
 */
export function isUnsupportedStdio(exitCode: number | null, stderr: string): boolean {
  if (exitCode === 0) return false; // it did not fail
  return /(?:unexpected|unrecognized|unknown) argument[^\n]*--stdio/i.test(stderr);
}

/**
 * The actionable message for that failure: the **resolved binary**, the fact, and
 * BOTH remedies. Never a silent fallback to another binary — that would mask a
 * real misconfiguration.
 */
export function unsupportedStdioMessage(binary: string): string {
  return [
    `wcode at ${binary} is older than this extension: it does not support \`serve --stdio\`.`,
    'Fix: upgrade wcode (`cargo install --path crates/wcode-cli`, or install a newer release),' +
      ' or set "wcode.path" to a binary that does.',
  ].join("\n");
}
