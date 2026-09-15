# Brainstorm: content-addressed read/edit + grep/find + ast-grep for wcode

Status: **superseded** by `2026-09-07-read-edit-phase2.md` — the content-addressed read/edit + grep/find/ast-grep design landed; see that doc for the current state.
Date: 2026-09-06

## The problem

wcode's default tools were `read` (line numbers), `edit` (`old_string`
replace), `write` (overwrite), `bash`. Three known weaknesses:

1. **Line-number drift corrupts files.** `read` shows positions; the model
   reasons in positions. Insert one line above a target and every number below
   is silently wrong — the classic way agents write to the wrong line.
2. **`edit` makes the model re-type old code.** Output tokens cost ~5–6× input;
   an edit usually echoes the string it replaces.
3. **No structural search.** Grep is `bash`-only; there is nothing for "find
   every *call site* of this function" shape.

Two upstream projects we studied to fix this:

- [`pi-better-edit`](../pi-better-edit) — replaces line numbers with per-line
  content **hash anchors**. `read` prints `ve7│function hello() {`; `edit`
  takes anchor ranges instead of `old_string`. Anchors are content
  addresses, so add/remove lines above never change the anchors below, and
  there's a strict served-range verification (you can't edit what you haven't
  seen).
- [`ast-grep`](../ast-grep) — structural search/replace by AST pattern
  (`if $COND { $BODY }`), instead of regex/text grep.

This document works through how each idea should land in wcode given its
minimal, stateless philosophy.

## Hashline: what we adopt and why

### Core design (landed in the prototype)

`anchor(line) = base62(hash(canon(line)))`, 5 chars over `[A-Za-z0-9]`
(62^5 ≈ 916M addresses), where `canon` strips all ASCII whitespace and `hash`
is FNV-1a 64 + splitmix64 finalizer — deterministic across processes, no
dependencies.

- `read` renders `ANCHOR│line` (no line numbers; `plain:true` restores the
  old `cat -n` output for raw-text needs).
- `edit` takes `{path, from[anchor], to?[anchor], replacement, old_string?,
  replace_all?}`; it resolves the anchors against the *current* file contents,
  replaces the contiguous range, and echoes the region with fresh anchors so
  the next edit chains without a re-read.
- The old string-replace survives as `replace` (unique literal substitutions
  without a read), so nothing is lost.
- `grep` renders results with the same anchors (`path:line  ANCHOR│line  <--`),
  so a hit feeds directly into `edit`.
- `find` is a thin glob walker.

### Why this specific tradeoff

pi-better-edit keeps a **stateful per-file hash store** (SQLite) so duplicated
lines get *unique* per-occurrence anchors and anchors survive diffs via
content matching. That buys the "edit a specific `}`" case. It costs: a hash
store, drift detection, served-range verification, reject-and-serve, ~2k LoC.

We deliberately went **stateless**:

| property | stateless (ours) | stateful (pi-better-edit) |
|---|---|---|
| anchors never move under edits above | ✅ | ✅ |
| survives formatters (whitespace canon) | ✅ | ✅ (canon + analysis) |
| self-healing under external edits | ✅ automatic (recompute per call) | needs checksum/drift machinery |
| no store, no session memory | ✅ | ❌ (SQLite + per-session served state) |
| duplicate lines uniquely addressable | ❌ (shared anchor) | ✅ |
| "verify you've seen the range" guard | partial (old_string) | ✅ full |

The honest cost: identical lines share an anchor, so `edit` on a duplicated
line (`}`, `else {`) is **ambiguous**. We make that a first-class, safe error
(`[E_AMBIGUOUS_ANCHOR]` with candidate line numbers + `old_string`/`replace_all`
escape hatches) rather than silently picking one. That's category `"fail closed,
never wrong"` — the same stance pi-better-edit takes for stale ranges. In
practice the agent either adds the surrounding line to `old_string` or widens
the range with a `to` anchor; one extra round-trip in the rare duplicated-line
case vs. unbounded wrong-line risk.

### Anchor length, honestly

- 3 chars (pi's choice) gives 238k addresses — that *requires* the stateful
  allocator to guarantee uniqueness. Stateless, 3 chars would collide
  constantly on real files.
- 5 chars (ours): accidental collisions are negligible (<1% for a 5k-line
  file of distinct lines); only *genuinely identical* lines collide, which is
  the expected, handled case.
- Cost: ~6–7 extra tokens per read line. `offset`/`limit` paging and
  `plain:true` are the anti-token-bloat escape hatches.

### What we consciously defer

- **Served-range verification** (refuse to edit lines `read` never showed).
  It prevents blind overwrites of externally-drifted regions. Stateless can't
  know what the agent has seen; the hooks seam (`after_tool_call` /
  `transform_context`) is the right place to layer it on if a fork wants it.
- **Unique anchors for duplicates** via session-memory (Phase 2 sketch
  below).
- **Batched same-file edits** (pi reports ~-40% envelope). Our `replace_all`
  covers the most common batch shape (`same` → `diff` everywhere); a multi-
  edit array on `edit` is an easy follow-up.

## ast-grep: how it should integrate

Opinion: **do not embed tree-sitter into wcode.** ast-grep is already a great,
installable CLI (`cargo install ast-grep`, `npm -g @ast-grep/cli`, brew). The
project already has the pattern for optional binaries: the rtk hook is
`auto|true|false` in config and only activates when the `rtk` binary is on
PATH.

The prototype does exactly that:

- `ast_search` tool shells to `sg -p <pattern> [--lang ..] [--ignore ..] <target>`
  with `NO_COLOR=1`.
- It is **only registered when `sg` is present on PATH**, so the model never
  sees a tool it can't actually use (no schema noise on clean machines).
- Search-only for now; `--rewrite` is a natural follow-up when someone wants
  structural edits as a first-class `ast_edit`.

`grep` remains the workhorse (regex is cheaper than AST for text matches and
works on any file type); `ast_search` is for shapes regex can't express (every
`await` regardless of operand, every call to a method, conditionals by shape).

## The tool surface after this change

```
default: read · bash · edit · replace · write
optional ([tools] grep/find = true): grep · find
optional (sg on PATH): ast_search
```

Naming: `edit` is now anchor-based (the thing you reach for after a `read`);
`replace` is the old literal string-replace (reached for when you already know
the exact string and don't need to read). Two verbs, one job each, no
mode-switching inside a single tool.

## Phase 2 sketch: session-memory for unique duplicate anchors

If duplicate-line edits turn out to be common in real sessions, add an
in-memory (not SQLite) per-session `path → (checksum, anchors[])` map, fed by
`read`/`edit`, invalidated by checksum on every call, and cap the ambition to
"disambiguate duplicates within one session" — no disk store, no cross-session
guarantee:

1. On `read`: compute anchors; for a run of identical lines, allocate
   `base, base𝄞, base♯`-style suffixes (`9Bx` = first `}`, `9Bx~` = second `}`
   …). Render suffix only when the line has duplicate siblings in the file.
2. On `edit`: if an anchor has siblings in the store, verify the *occurrence
   index* still lines up (compare surrounding lines) before committing;
   otherwise fall back to the stateless path.
3. External edits (checksum mismatch) → recompute and re-suffix naturally.

The stateless core stays the foundation; memory is an optimization on top that
can be switched off. Keep the hard guarantees (never edit the wrong line) in
the *stateless* resolver regardless.

## Open questions for the room

1. **`old_string` semantics** — currently a filter ("appears up to and
   including the target line"; can pin the preceding line). Is the model-facing
   wording clear enough, or should `edit` also accept an explicit `before`/
   `after` context pair?
2. **Anchor length** — 5 is the stateless sweet spot. Do we add a
   config (`anchor_len = 4..6`) or always 5 for predictability?
3. **`grep` output shape** — anchors on every emitted line (incl. context)
   is what makes hit→edit seamless; is the `path:line  ANCHOR│line  <--`
   format token-optimal?
4. **ast-grep depth** — search-only via `sg` shell-out is the v1. Should
   `ast_edit` (structural rewrite) be a follow-up, and should it require
   `--rewrite` or read-back the diff before writing?
5. **Default tools growth** — we went from 4 to 7 (8 with sg). Acceptable for
   "default", or should grep/find hide behind a flag? The rtk-style `auto`
   pattern argues ast_search is fine to auto-register.
   **Resolved: grep/find are off by default** — `bash` can grep/find itself, so
   the native tools register only when `[tools] grep = true` / `[tools] find = true`
   (env `WCODE_GREP`/`WCODE_FIND` override). ast_search stays auto-registered on
   PATH detection (it has no shell equivalent for `sg -p` patterns).

## Prototype inventory

- `crates/wcode-cli/src/tools/anchor.rs` — canon/hash/encode, `split_lines`,
  `find_ranges`, `apply_replace`, `changed_range` (pure, unit-tested).
- `crates/wcode-cli/src/tools/read.rs` — anchor rendering, `plain` mode.
- `crates/wcode-cli/src/tools/edit.rs` — anchor-range edit with
  `[E_BAD_ANCHOR]` / `[E_STALE_ANCHOR]` / `[E_AMBIGUOUS_ANCHOR]` diagnostics.
- `crates/wcode-cli/src/tools/replace.rs` — old literal string-replace.
- `crates/wcode-cli/src/tools/grep.rs`, `find.rs` — native search/locate with
  anchors on results.
- `crates/wcode-cli/src/tools/ast_search.rs` — optional `sg` shell-out,
  registered on PATH detection.
- Design doc: this file.

Tests: 151 pass (`cargo test`), `cargo clippy --all-targets` clean.