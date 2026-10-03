# Sketch — VS Code extension P2: native diffs (+ the `chat.ts` fold-in)

**Status:** interface sketch, review-only. No working logic. Fillable as-is.

**Design:** `docs/vscode-extension-plan.md` §3.3 — reverse-apply the patch to
reconstruct the pre-image, serve both sides to `vscode.diff`, open **on request**
(`:hzpIw`, `:xW4PJ`); §3.2 (the client) for the webview side. Base tree = P1b,
`editors/vscode` @ `05baf91` (the `chat.js` fold-in and the enable of `open-diff`).

**Grounding (fresh anchors).**

- **The generator is the spec.** `crates/wcode-cli/src/tools/diff.rs` —
  `pub fn unified(old: &str, new: &str) -> Option<String>` `:OCA3f`; `None` on
  byte-identical input `:upZMW`; split via `str::lines()` `:8eGBK`; `CONTEXT = 3`
  `:fVgoV`; `MAX_LINES = 40` `:t1g64`; the single header `:CEtKl`; the truncation
  marker `:2gYuq`; the body `join("\n")`, **no trailing newline** `:IwWF3`. It is a
  *presentation* diff: "one hunk is enough" `:I4pDJ`, "for presentation only"
  `:6FkkJ`.
- **Call sites** (all pass `&original, &updated`): `edit.rs:fvfd2`, `edits.rs:fvfd2`
  (shared anchor — identical line), `replace.rs:yVhjQ`, `write.rs:H9fcz` (old = ""
  for a new file, `write.rs:F6YGS`).
- **The contract that makes it UI-only.** `ToolOutput.diff`/`path` are
  "presentation only … never enters the model's context" `tool.rs:gPWi4`/`:EfAZ4`/
  `:V7Pib`/`:EYf63`; they ride `ToolExecutionEnd` `event.rs:ux4ID`/`:E3UKn`, both
  `#[serde(default, skip_serializing_if = "Option::is_none")]` `:95kss`.
- **The closest prior art — how this project renders a diff.** `wcode-tui` styles a
  diff **line-by-line by its first character** and never parses a hunk numerically:
  `diff_block_lines` `ui.rs:YVuNy` (the `/changes` re-show: a `path · +a −r` header
  then each `diff.lines()` line), `diff_lines` `ui.rs:JeNO7` (the inline tool diff:
  `⚙ name`, the body, `✓ name · +a −r`); the selector is `raw.chars().next()` →
  `added_style()`/`removed_style()`/`dim()` `ui.rs:lVw4X`; the `+a −r` count is
  `diff_counts` `app.rs:194` ("the `@@` header is neither"). Roles `diff_add`/`diff_del`
  are `theme.rs:54`/`:56` (green/red `:81`/`:82`). **The TUI shows the `@@` header as a
  dim line and styles `-`/`+` bodies; it does not reconstruct files** — P2 is the first
  code in the repo that must *parse* a hunk.
- **P1b seams to extend.** `reducer.ts` keeps `ToolBlock.diff?: string` `:28` (set at
  `:145`); `render.ts` computes `hasDiff` `:6otlg` and documents it as the "P2 hook"
  `:81F17`; `webview.ts` already types `{ kind: "open-diff"; callId }` `:k66G6` +
  `parseFromWebview` `:dvu9e`; `panel.ts` already routes it `:uGivk` →
  `handlers.onOpenDiff` `:YiPSm`; `extension.ts` stubs it `:44mJx` and stubs the
  `wcode.openDiff` command `:rPRjx`. The webview button is shipped **disabled**
  `src/webview/chat.js:Lg0II`/`:Kebmr`.
- **The capture path.** `scripts/capture.mjs` captures only **model-free** frames
  (`test/fixtures/README.md`); a `tool_execution_end` with a real `diff` needs a model
  call (the README's handwritten/captured split). esbuild's webview entry is
  `src/webview/chat.js` `esbuild.mjs:VE6B6` → `media/chat.js` `:NAoYn`.

---

## 0. What the generator emits — pin this before the parser

`unified(old, new)` returns `None` (no frame field at all) when the bytes are equal;
otherwise exactly one hunk:

```
@@ -{from},{old_count} +{from},{new_count} @@
{pre-context lines, each prefixed with a single space}
{- old-middle lines, each prefixed with "-"}
{+ new-middle lines, each prefixed with "+"}
{post-context lines, each prefixed with a single space}
```

joined by `\n`, **no trailing newline**. Precise properties, all from `diff.rs`:

| property | value | anchor |
|---|---|---|
| hunk count | **exactly one** | `:I4pDJ` |
| `---` / `+++` file headers | **absent** | `:CEtKl` (only `@@`) |
| `\ No newline at end of file` | **absent** | (`.lines()` + `join`) `:IwWF3` |
| header range | both sides start at `from` (**same number**) | `:CEtKl` |
| context | ≤3 lines each side | `:fVgoV` |
| body cap | 40 lines, then `… (+N more lines)` | `:t1g64`, `:2gYuq` |
| line split | `str::lines()` — strips `\n` **and a trailing `\r`** | `:8eGBK` |
| identical input | `None` (field omitted) | `:upZMW`, `:95kss` |

**The parser must therefore NOT be a `diff -u` parser.** It must: accept a missing
`---`/`+++`, assume one hunk, and treat the `… (+N more lines)` line (and any
`\ No newline…` line) as **unparseable** → `null` (§1). The TUI's first-character
styling (`ui.rs:lVw4X`) is consistent with this and is the fallback rendering.

## 1. `reverseApply` — the pre-image from the post-image + the patch

```ts
// editors/vscode/src/diff.ts — PURE: no `vscode`, no I/O. Testable under plain node.
/**
 * Reconstruct the pre-image: reverse-apply one wcode unified diff (diff.rs:OCA3f)
 * to `currentText` (the on-disk post-image). Returns the before-text, or `null`
 * when it cannot be done EXACTLY. A wrong answer is a silently wrong diff, so every
 * doubt is a `null`.
 */
export function reverseApply(currentText: string, unifiedDiff: string): string | null;
```

**Algorithm** (one hunk, `diff.rs`):

1. Split `unifiedDiff` on `\n`. The first line must be a header
   `/^@@ -(\d+),(\d+) \+(\d+),(\d+) @@$/` → `from`, `oldCount`, `newFrom`, `newCount`.
   Missing or malformed → `null`. **`newFrom ≠ from` → `null`**: our generator emits the same
   start on both sides (`diff.rs:CEtKl`), so a foreign patch with a different new-side start
   would splice at the wrong offset and, if the lines happen to match there, return a
   plausible but **wrong** pre-image.
2. The remaining lines are the body. Each must start with ` `, `-`, or `+`; anything
   else (a `\`, the `… (+N more lines)` marker, a stray blank) → `null`. **A
   `---`/`+++` line is NOT a failure**: the generator prefixes a REMOVED line with
   a single `-`, so a removed line whose content is `--` arrives as `---` and
   `-- …` arrives as `--- …`. A real `diff -u` patch is already rejected at step 1
   (it does not open with `@@`), so a `---`/`+++` line in the body is always ours.
3. Build `oldLines` = body lines with prefix ` ` or `-` (their text after the first
   char); `newLines` = body lines with prefix ` ` or `+`. Assert `oldLines.length ===
   oldCount` and `newLines.length === newCount`, else `null`.
4. Split `currentText` into lines (`/[\r\n]+/` with terms remembered, or a manual
   scanner that keeps the line endings — see CRLF below). The hunk occupies post-image
   lines `[from-1, from-1 + newCount)`. Assert that slice equals `newLines` (the context
   + `+` lines are all in the post-image), else `null` for v1 (a bounded ±k search is a
   TODO, §8).
5. Splice: replace that slice with `oldLines`, rejoin with the file's endings, and
   return. Everything outside the hunk is byte-preserved from `currentText`.

**Failure modes → `null` (never a wrong answer).**

- `unifiedDiff` empty / whitespace-only → `null`.
- no `/^@@ … @@$/` header, or non-numeric counts → `null`.
- **the `… (+N more lines)` truncation marker is present** (`diff.rs:2gYuq`) — the
  omitted body lines are unknown, so a reconstruction is impossible → `null`.
- a `\ No newline at end of file` line (we never emit it) → `null` (a foreign patch).
- ~~a `---`/`+++` line — not our format → `null`.~~ **RETRACTED (second-layer
  review, P2 fix).** This can only ever false-reject: the generator prefixes a
  REMOVED line with a single `-`, so `--` arrives as `---` and `-- a SQL comment`
  as `--- a SQL comment`; a SQL `--` comment is a common trigger. Foreign `diff -u`
  patches are already rejected by the line-0 `@@` requirement, so the check added
  no safety. The `\` and `… (+N more lines)` branches went with it: they are not
  ` `/`-`/`+` lines, so the prefix dispatch's `else` is the check — a dedicated
  branch is unreachable as a distinct rejection (breaking it changes nothing, so
  it cannot be pinned by a test).
- `oldLines.length ≠ oldCount` or `newLines.length ≠ newCount` → `null`.
- **`newFrom ≠ from`** — a foreign patch; ours always emits the same start on both sides → `null`.
- the `newLines` slice does not match `currentText` at `from-1` (the **file moved on**
  since the edit — the Friction case) → `null`.
- **CRLF / mixed endings.** `str::lines()` (`diff.rs:8eGBK`) strips a trailing `\r`, so
  a CRLF file's diff lines carry no `\r`; a CRLF `currentText` split naively would keep
  `\r` and mismatch. Decision: **normalize both sides to LF for the match; emit with
  the ending detected in `currentText`** (a CRLF file stays CRLF). If the file is
  *mixed*, return `null` rather than guess. **Settled: the ending must be uniform** —
  all-CRLF → re-emit CRLF, all-LF → re-emit LF, **mixed → `null`**. (Detecting "contains
  `\r\n`" and re-emitting CRLF would rewrite every line of a mixed file, which is exactly
  the wrong pre-image this list exists to prevent.)
- **the degenerate all-context hunk.** A change that is only a trailing newline
  (`diff.rs` compares bytes `:upZMW` but `.lines()` drops the terminator) yields a body
  of context-only lines: `oldLines === newLines`. The reverse-apply is then the
  **identity** — `reverseApply` should return `currentText` unchanged (there is no line
  change to undo), and the caller should treat "before === after" as "nothing to show".
  Flag this in a test (§6). <!-- SKETCH: TODO(prose) — confirm identity, not null. -->
- `write` on a **new** file (`write.rs:F6YGS`, old `""`): header `@@ -1,0 +1,N @@`,
  `oldLines = []` → reverse-apply returns `""` (the pre-image is an empty file). Correct.

## 2. The `wcode-diff:` content provider

```ts
// editors/vscode/src/diff.ts (host half) + src/diffProvider.ts — the ONLY vscode import here.
const SCHEME = "wcode-diff";

/**
 * A token → pre-image registry. The URI carries an OPAQUE, SHORT token, never the
 * text (no length/escaping limits, no file content in a URI that lands in the
 * titles/undo stack). In memory only — it MUST NEVER touch disk (plan §3.3): the
 * whole point is that no before-blob is persisted.
 */
class BeforeRegistry {
  private readonly byToken = new Map<string, string>();
  mint(beforeText: string): string;            // -> token (short, unique)
  resolve(uri: vscode.Uri): string | undefined; // token -> before-text, or undefined
  clear(): void;                                 // on session stop / panel dispose
}

class WcodeDiffProvider implements vscode.TextDocumentContentProvider {
  provideTextDocumentContent(uri: vscode.Uri, _token: vscode.CancellationToken): string;
  // returns the before-text from the registry; `""` for an unknown/evicted token
}
```

- **URI shape:** `vscode.Uri.from({ scheme: SCHEME, path: `/${token}` })` — e.g.
  `wcode-diff:/t7f3a`. Only the token is embedded; the path and side live in the token
  table (a `wcode-diff:` doc is always the **left/before** side; the right side is a
  real file, §3).
- **Registration:** `vscode.workspace.registerTextDocumentContentProvider(SCHEME, provider)`
  pushed onto `context.subscriptions` (`extension.ts:IGpyo` activate / `:gsGsr` subscriptions.push).
- **Lifetime / eviction:** tokens live for the panel's lifetime; a bounded LRU (**32**)
  bounds memory. An **evicted-but-still-open** tab: VS Code re-requests content on
  refresh; an unknown token returns `""` and logs, rather than throwing. `clear()` on
  `stopSession()`/`dispose` (`extension.ts:Puqci`), so a restart starts clean (a restart
  spawns a fresh child with empty history, and the P1b panel already resets its transcript).
- **Never touches disk:** the provider resolves purely from the map; it does not read
  the file. (The *right* side is the real file, opened by `vscode.diff`, not by us.)

## 3. Opening the native diff

Handled in the `onOpenDiff(callId)` handler (`extension.ts:44mJx`, replacing the P1b
`showInformationMessage("diffs land in P2")` stub; the `wcode.openDiff` command stub
`:rPRjx` routes to the same code).

```ts
async function openDiff(callId: string): Promise<void> {
  // 1. Find the tool block by callId in the live ViewState (reducer.ts ToolBlock.diff :28).
  // 2. Resolve the path (ToolOutput.path is "as the caller named it" tool.rs:V7Pib — may be
  //    RELATIVE) against workspaceRoot() (extension.ts:XrAxA). Absolute -> as-is.
  //    A missing / non-file path -> a warning, never a wrong window.
  // 3. before = reverseApply(readFileSync(path, "utf8"), block.diff);
  //    before === null -> a message ("wcode: the before-image is unavailable for this
  //    edit"), and STOP — never open a diff with a guessed left side.
  // 4. left  = vscode.Uri.from({ scheme: "wcode-diff", path: `/${registry.mint(before)}` });
  //    right = vscode.Uri.file(path);            // the on-disk post-image (the file as it is)
  // 5. await vscode.commands.executeCommand("vscode.diff", left, right, title, { preview: true });
}
```

- **left = a virtual `wcode-diff:` document (the reconstructed before); right = the
  real on-disk file.** The right side is the file *as it is now*; for the common case
  (the file has not been touched since the edit) that IS the post-image. If a later edit
  has already changed the file, or the 40-line cap truncated the patch, `reverseApply`
  returns `null` — and rather than an enabled control that opens nothing, the click opens
  the **patch text read-only** in a `wcode-diff:` document titled
  `wcode: ${basename(path)} (patch only)`. The gate stays `hasDiff`; reconstruct-vs-patch-only
  is decided at click time.
- **title:** `` `wcode: ${basename(path)} (before → after)` `` — **no em-dash** (the repo
  bans `—` in user-facing UI strings; `.wcode/skills/design-taste/` §5). One separator,
  parenthesised, so the path and the direction do not read as a metadata strip.
- **Reveal at a line:** **no** auto-reveal/navigate — diffs open on request (`:xW4PJ`);
  jumping to a line is the separate `reveal-file` message, already implemented
  (`extension.ts:revealFile` `:MnhMr`). Opening the diff is `preview: true` so it does
  not pile up tabs.
- A **new** file (`@@ -1,0 +1,N @@`): left = an empty `wcode-diff:` doc, right = the
  file. Correct and natural.

## 4. The webview side — turning on `open-diff`

The message is already typed and parsed end-to-end (`webview.ts:k66G6` → `panel.ts:uGivk`
→ `handlers.onOpenDiff` `:YiPSm`); `panelHandlers().onOpenDiff` (`extension.ts:44mJx`)
is the stub to replace. In the webview (`src/webview/chat.js:Lg0II`):

- **Enable the button** (drop `diff.disabled` / the `"Diffs land in P2"` title,
  `:phgdq`/`:uzSpK`) and keep the click → `post({ kind: "open-diff", callId: tool.callId })`
  (`:ZU5mx`). **The call id is `tool.callId`** — the same key the reducer stores the diff
  under (`reducer.ts:23`/`:145`) and the host looks it up by.
- **Gate on `hasDiff`, not `path`.** P1b shows the button when `tool.hasDiff || tool.path`
  (`:Kebmr`). But `path` can be set with **no diff** (a tool that touched a file yet
  changed no line — `unified` returns `None` `:upZMW` → the field is omitted `:95kss`).
  P2 must show the button iff `hasDiff` (`render.ts:6otlg`), because only a diff can be
  opened. A `path`-only block shows **no** diff button (optionally a "reveal file"
  affordance — out of P2 scope).
- **When `diff` is absent** (`Option` + `skip_serializing_if` — `tool.rs:EfAZ4`,
  `event.rs:95kss`): this is a **normal case, not an error** — most tools touch no file
  (`read`, `grep`, `bash`). The tool row renders exactly as today (summary + output),
  with no diff affordance and no warning. The plan says "no diff is a normal case, not
  an error" (`:5CYvR`).

**Dead `ToWebview` variants — delete them.** `webview.ts` declares `{ kind: "append";
block }` `:bug2Z` (the webview handles it at `chat.js:tUJLQ`, but `panel.ts` **never
sends it** — `post()` only ever sends `{ kind: "state" }` `:1vyy0`) and `{ kind: "diff";
… }` `:vCABu` (the webview no-ops it `:M5s8Q`; the host never sends it). Since P2 opens
**native** diffs (not an inline webview renderer), neither is needed: **reduce
`ToWebview` to `{ kind: "state"; state; session }` alone** and delete the `append`/`diff`
arms in both `webview.ts` and `chat` (the message listener at `:L0ZPf`). *Alternative*:
keep `append` as a real fast path — but then `panel.ts` must actually emit it; not worth
the drift. <!-- SKETCH: TODO(prose) — confirm delete vs wire-up. -->

## 5. Fold-in: `src/webview/chat.js` → `chat.ts`

- **Rename + entry change.** `src/webview/chat.js` → `src/webview/chat.ts`; update
  esbuild's webview `entryPoints` (`esbuild.mjs:VE6B6`) to `src/webview/chat.ts` (output
  `media/chat.js` `:NAoYn` is unchanged). The comment at `esbuild.mjs:bwsyi` follows.
- **Module shape.** Write it as an ES module (imports + top-level statements); esbuild's
  `format: "iife"` wraps it, so the manual `(function () { … })()` around the current
  file (`chat.js:emwvy`) is deleted. The webview source imports the **shared, pure**
  types/helpers from the host modules that are already `vscode`-free:
  `import type { RenderedState } from "../render.ts"` and
  `import { parseToWebview, type ToWebview, type FromWebview } from "../webview.ts"`
  (`webview.ts` is pure by header `:wPITg` — safe to bundle into the webview).
- **`acquireVsCodeApi` declaration.** A local ambient declaration (the real API is a
  webview global, not in `@types/vscode`):

  ```ts
  interface VsCodeApi {
    postMessage(message: FromWebview): void;   // typed outbound — the union is exhaustive
    getState(): unknown;
    setState(state: unknown): void;
  }
  declare function acquireVsCodeApi(): VsCodeApi;
  const vscode = acquireVsCodeApi();            // was `chat.js:1znX0` (untyped)
  ```

- **Typing the DOM handlers.** `post(message: FromWebview)` (was `chat.js:MfsXy`);
  the inbound listener narrows the untrusted payload:
  `window.addEventListener("message", (e: MessageEvent<unknown>) => { const m =
  parseToWebview(e.data); if (m) onHost(m); })` (`chat.js:L0ZPf`). `parseToWebview` is
  **new** and **pure** — mirror `parseFromWebview` (`webview.ts:dvu9e`), test it.
- **Pure vs DOM-bound — extract the pure half** into `src/webview/view.ts` (a
  `vscode`-free, DOM-free module, driven by `test/`), mirroring the
  `render.ts`-pure / `panel.ts-impure` split (`render.ts:3VPnb`):

  | extract (pure, testable) | stays DOM-bound in `chat.ts` |
  |---|---|
  | `parseToWebview(raw): ToWebview \| null` | `createElement`/`blockShell`/`render` |
  | `stateLabel(state): string` (`chat.js:4XG9O`) | scroll glue (`nearBottom`) |
  | `statusSegments(state, session): Segment[]` (from `renderStatus`) | the `expanded` Set toggling on click |
  | `toggleExpanded(set: ReadonlySet<string>, id): Set<string>` | the `input` grow / keydown handlers |
  | `classNames(block): string` (the `"block tool …"` composition, `chat.js:9bBBq`) | `acquireVsCodeApi()` + `postMessage` |

- **`ToWebview`/`FromWebview` typing of the DOM handlers** is the point: every `post`
  call is now checked against `FromWebview` (`webview.ts:bg7q4`) and every inbound
  message narrowed through `parseToWebview`, so a typo in `kind` is a compile error, not
  a silent no-op (as `{ kind: "open-diff", callId }` is today, untyped, at
  `chat.js:ZU5mx`).

## 6. Tests & fixtures

```ts
// editors/vscode/test/diff.test.ts — reverseApply (pure); drives real + handwritten diffs.
it("round-trips a captured edit diff to its pre-image", () => { todo!(); });
it("handles a new file (@@ -1,0 +1,N @@) -> pre-image is empty", () => { todo!(); });
it("returns the input unchanged for a context-only (trailing-newline) hunk", () => { todo!(); });
it("preserves CRLF: a CRLF file round-trips to a CRLF before-image", () => { todo!(); });

it("returns null on a truncated diff (… (+N more lines))", () => { todo!(); });
it("returns null when --- / +++ headers are present (a diff -u patch)", () => { todo!(); });
it("returns null on a \\ No newline at end of file line", () => { todo!(); });
it("returns null when the context no longer matches (the file moved on)", () => { todo!(); });
it("returns null on a count mismatch / a malformed header / an empty patch", () => { todo!(); });

// editors/vscode/test/diffProvider.test.ts — the token/URI mapping (pure side).
it("mint() then resolve() round-trips; an unknown token resolves to undefined", () => { todo!(); });
it("a wcode-diff: uri carries only the opaque token (never file text)", () => { todo!(); });
```

**Where real fixtures come from — say it plainly.** `scripts/capture.mjs` captures
**only model-free** frames (`fixtures/README.md`), and a `tool_execution_end` with a
real `diff` requires the model to *call* an edit tool — there is no `Request` that
invokes a tool directly. So:

- A **real** diff fixture must be captured during the **manual pass** (or a local-
  endpoint run: `wcode serve --stdio --base-url http://localhost:11434/v1`, ask it to
  edit a file, tee stdout). Reuse the capture harness's stdin-open technique
  (`capture.mjs:mG3IR` — a closing client drops in-flight replies).
- **Meanwhile**, hand-write fixtures to the generator's exact bytes (cited `diff.rs`),
  like the existing `*.handwritten.ndjson` (`fixtures/README.md`) — e.g. the generator's
  own test cases: `@@ -1,5 +1,5 @@` (`diff.rs:MNpYt`) and `@@ -1,0 +1,2 @@` (`:9dbPv`). The
  generator **always** emits `,count` on both sides (`diff.rs:CEtKl`) and §1's header regex
  requires them, so the count-less `@@ -1 +1 @@\n-old\n+new` shape that `ui.rs:1862` uses is a
  **foreign** patch: use it as a `→ null` fixture, never as a round-trip one —
  and give that fixture a `current` that the body implies, so only the header rule
  rejects it.
  **Add the `---`/`+++` body cases as ROUND-TRIP fixtures** (the P2 fix): a
  removed line whose content is `--`, one whose content is `-- a SQL comment`,
  the existing `--x`, and an ADDED line whose content is `++ b/f.txt`. All four
  are legitimate generator output (`format!("-{line}")` / `format!("+{line}")`).
  Verify every fixture against the **compiled generator**
  (`scripts/verify-diff-fixtures.mjs`), not by eye — that is what caught the
  `---`/`+++` bug's blast radius.
  Label anything not sourced from a `diff.rs` test `*.handwritten`.

## 7. Friction (do not smooth over)

- **A diff is a *text patch*, not before/after blobs — so a wrong reverse-apply is a
  wrong diff, silently.** There is no blob to diff against as a check. The only defence
  is §1's rule: **every doubt → `null`**, and a `null` opens the patch text (`(patch only)`)
  rather than a plausible wrong window. This is why the hunk must be *verified* against
  the current text, not trusted by line number.
- **No diff at all when the tool touched no file** — normal (`tool.rs:EfAZ4`); the
  webview must gate on `hasDiff`, not `path` (§4). Do not surface it as an error.
- **Large diffs and the 16 MiB cap.** The diff itself is tiny (≤40 body lines,
  `diff.rs:t1g64`), but it rides the same NDJSON line as the tool's `output`; the whole
  frame is capped at 16 MiB (`frame.rs:gbOLr`) and blank-line-split. A `write` of a huge
  file yields a **truncated** diff (`… (+N more lines)`) → **not reverse-appliable** →
  `null` (§1). For `write`, prefer opening the on-disk file or a git diff; reverse-apply
  is a best-effort only for small edits.
- **A file edited twice.** Each diff is (stateᵢ₋₁ → stateᵢ); the on-disk file is the
  latest state. Only the **latest** diff reverse-applies against the on-disk file; an
  earlier one's context no longer matches → `null` (correctly). Do not "fix" this by
  applying diffs in sequence — that is a different (and unbounded) feature.
- **Pre-image from git or from VS Code's own document history — the trade-off.**
  Reverse-apply is exact **per tool call, for every write tool, with no disk history
  and no git** (`plan:hzpIw`). The alternatives lose: **git** (`show HEAD:<path>`) shows
  *all* pending changes, not this edit (`:L63KL`); **VS Code document history** works
  only for files already open (`:ZNaZ8`). The plan rules **reverse-apply**;
  honour it. The one honest gap: a file that has since moved on → no diff (§ above),
  where git/document-history could still answer — a **follow-up**, not v1.
- **CRLF and the trailing-newline degenerate hunk** (§1) — the two formats `.lines()`
  (`diff.rs:8eGBK`) quietly normalizes away; both are specified above.
- **The `open-diff` button was disabled on purpose** (`chat.js:Lg0II`); P2 must also fix
  the `hasDiff || path` gate (`:Kebmr`) or it will offer a button that opens nothing.
- **The em-dash ban reaches the diff title** (§3): a UI string, so `—` is out; `·`/`→`
  in, matching the TUI.

## 8. Not verified / left to the implementer

- **No captured real diff was available to test against** — `capture.mjs` is model-free
  (its header `capture.mjs:c52Ip`), so §6 says hand-write to the generator's bytes now
  and capture during the manual pass. The generator's format above is read from
  `diff.rs`, not from a live frame.
- The CRLF policy and the identity-vs-null choice for a context-only hunk (§1 `TODO`s).
- The LRU size / token lifetime (§2 `TODO`), and the diff title wording (§3 `TODO`).
- Delete-vs-wire-up for `ToWebview.append` (§4 `TODO`).
- I did **not** run `reverseApply` against any diff (review-only); the failure-mode list
  is derived from `diff.rs`'s construction, not observed.
- `path` resolution (relative vs absolute, `tool.rs:V7Pib`) and the file-deleted-by-then
  right-side case (§3) are specified but untested.
