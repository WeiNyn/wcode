# Phase 2 design: read/edit refinements — remaining steps

Status: **draft for design review** (no code landed for these items)
Date: 2026-09-07
Supersedes the Phase-2 sketch in `2026-09-06-hashline-grep-integration.md`.

This doc specifies the remaining Phase-2 improvements to the content-addressed
`read`/`edit` tools, in dependency order. Landed already (Phase 2a, in working
tree, reviewed one-by-one):

- **#1** overlapping `replace_all` ranges → hard `[E_OVERLAPPING_RANGES]` rejection (fail closed).
- **#4** `read { from: <anchor>, context? }` — read a region from a content anchor without a line number.

Everything below is the remaining queue (Phase 2b), grouped: **correctness
hardening**, **ergonomics**, **session memory** (the big one, with the full
external-edit story), **anchor-scheme changes** (breaking — must be decided
early), and **stretch**.

---

## 0. Invariants every item must preserve

1. **Never edit the wrong line, and never silently.** Old code is never re-typed
   (`from`/`to` anchors + replacement); stale/ambiguous targets reject with
   diagnostics and write nothing; the only silent paths are no-ops.
2. **Stateless is the foundation.** Everything recomputes anchors from current
   file content on every call; external edits are always picked up. Session
   memory (items #7/#8) is an *optimization on top that can be switched off* and
   — critically — is **never the authority that authorizes a write**.
3. **Fail closed, round-trip cheap.** Ambiguity costs one extra `read`/`old_string`
   round-trip, never a wrong write.
4. **Token diet.** 5-char anchors cost ~6–7 tokens/line. Sufficient and
   plain-mode/page-anchor escapes must stay; new surface must not add per-line
   cost for the common case (unique lines).

---

## A. Correctness hardening (small, independent)

### A1. Unify the degenerate empty-file states `""` and `"\n"` (from #2)

**Problem.** `read` of `""` shows `<empty file — insert at this anchor>`, but
`read` of `"\n"` shows one lonely empty anchor line (`ANCHOR│`), even though
both are "zero logical lines + separator" and `edit` treats `""` specially.

**Design.** Define a single predicate used by `read` and `edit`:

```rust
// anchor.rs
/// True when the file has no logical lines: byte-empty or only a separator.
pub fn is_degenerate_empty(content: &str) -> bool { content.is_empty() || content == "\n" }
```

- `read`: degenerate-empty content renders the single insertion anchor (both
  states), never a phantom empty line.
- `edit`: the existing `content.is_empty()` special-case becomes
  `is_degenerate_empty`; both states populate identically and keep the
  trailing-newline rule. `"\n\n"` stays a *real* blank line — untouched.

**Tests.** `read`/`edit` each on `""` and `"\n"` produce identical output;
round-trip edit of both yields the same file.

### A2. Read size guard + long-line truncation (from #3/#10)

**Problem.** Default `limit` is `u64::MAX` and lines render whole, so one
`read` of a minified bundle can blow the context with no resistance.

**Design.** Two independent, opt-out knobs on `read`:

- **Line truncation (anchored mode only):** display at most `MAX_LINE_CHARS`
  (proposal **300**) chars of each line, then `…(+N)` where N = remaining char
  count. `plain:true` is untouched (raw text means raw). The *anchor still
  hashes the full line*, so a truncated line remains a valid, unambiguous
  `edit` target.
- **Page guard (both modes):** `read` with no `limit` caps at
  `MAX_READ_LINES` (proposal **1000**; ~4–6k tokens worst case) and appends
  `… (N more lines; re-read with offset/limit to continue)`. `limit` > cap is
  honored (explicit wins). Parameterize both constants for tuning; no config
  surface.

**Tests.** Long-line truncation preserves hash-anchor equality with the real
line; `${MAX_READ_LINES+1}`-line file without `limit` returns cap + continuation
note; explicit `limit` above cap returns the full window.

---

## B. Ergonomics

### B1. `ast_edit` — structural rewrite via ast-grep (from #11)

**Design.** Companion to the existing `ast_search` (shells to `sg`). Same
optional-tool registration rule (`sg` on PATH). Two flavors of one tool:

- `ast_edit { path, pattern, rewrite, lang?, commit: Option<bool> }`:
  - `commit: true`/unset → `sg` rewrite via the existing atomic
    temp-file+rename path (same mutation lock), then return a **unified diff of
    the hunks it changed** so the agent sees exactly what landed.
  - `commit: false` (dry-run) → `sg --rewrite` output only, nothing written,
    diff shown.
- Applies only to files `sg` can parse (skips unsupported file types with a
  clear message). Structural edits bypass anchors by design (whole-file AST
  transforms), but the echoed hunks carry line numbers for later
  anchor-based follow-ups.

**Decision RESOLVED: commit-on with diff-echo**; the diff acts as the
verification step (`commit:false` stays as the explicit dry-run escape hatch).

**Landed notes.** Tool `ast_edit` (`tools/ast_edit.rs`), registered when an
`ast-grep` *or* `sg` binary is on PATH (prefers `ast-grep` to skip the `sg`
deprecation banner), sharing the mutation lock. Mechanics: a `-r` run without
`-U` prints ast-grep's own diff (exit 0) and never writes — that is both the
dry-run and the echo; `-U` applies. Apply is atomic: rewrite a same-directory
temp copy that **keeps the file's extension** (so language auto-detect
survives), then temp-write + rename over the original. Dry-run with no matches
→ success, "nothing to rewrite"; commit echoes the diff plus the changed-line
span (`changed_range`) and a re-read hint for anchor-based follow-ups.

### B2. Batch same-file edits (from #5)

**Problem.** N discontiguous edits = N read/edit round-trips; pi-better-edit
reports ~-40% envelope savings for batching.

**Design.** A separate tool `edits` (keeps "one verb, one job", no mode-switch
inside `edit`):

```rust
// tools/edits.rs
struct EditOp { path, from, to?, replacement, old_string?, replace_all? }
EditsArgs { edits: Vec<EditOp> }
```

**Transaction semantics (proposal — fail closed, atomic):**

1. One lock, one read of the file.
2. **Resolve every op against the single original snapshot** — anchors are
   validated there, so an op can't be derailed by an earlier op.
3. All-or-nothing: any op that is stale, ambiguous (without `replace_all`), or
   **overlaps** another op's resolved range (reuse the #1 detector) → nothing
   written, `[E_AMBIGUOUS_ANCHOR]`-style report naming the failing op.
4. On success, apply resolved (disjoint) ranges bottom-up to the accumulating
   content — identical to today's multi-apply loop — then temp-write + rename.

**Decision RESOLVED: all-or-nothing** — the batch is a transaction; partial
application makes the file state depend on op order and hides an op that "sort
of worked".

**Landed notes.** Tool `edits` (`tools/edits.rs`), registered in `default_tools`
with the shared mutation lock. Added over the sketch: (a) **single file per
call** is enforced — `[E_MIXED_FILES]` if ops target different paths; (b) the
fresh-anchor echo covers the combined `changed_range` only when it spans
`≤ MAX_ECHO_LINES` (120), otherwise replies with a re-read hint, so a batch
that touches both ends of a big file doesn't emit a huge echo; (c) empty batch
→ `[E_EMPTY_BATCH]`; each error names the offending op (`op1`/`op2`).

**Tests.** Disjoint batch applies atomically; one stale op aborts the whole
batch untouched; overlapping ops in a batch rejected; mixed `replace_all`
single-line + multi-line batch.

---

## C. Session memory for unique duplicate anchors (item #7) — MOVED TO BACKLOG

The full design (architecture, worked example, read/edit resolution,
external-edit handling, suffix format) was written before **decision D1
(option B)** de-scoped it: anchors now hash raw content, so duplicates shrink to
*byte-identical lines at the same indent*. The complete spec — including the
C4/C5 own-vs-external-write split and the irreducible-hole policy — lives in
`docs/superpowers/backlog/2026-09-07-session-memory-duplicate-anchors.md`,
along with its pull-in trigger. Guarantees that must survive any pull-in: the
stateless resolver stays the authority; memory never authorizes a write on its
own; fail closed, never silent.

## D. Anchor-scheme changes (breaking — decide order first)

### D1. Anchors hash raw content — `canon` removed (**DECIDED: option B**)

**Problem with whitespace tolerance (the deeper of the two):** collapsing
whitespace made indentation *invisible to the address*, so every `{`/`}` at any
indent shared one anchor — nesting level was the #1 real-world ambiguity, and
the whole #7 store existed mainly to fight it. Also whitespace-tolerant canon
still collided `foo bar` with `foobar`.

**Decision: raw-byte anchors.** `anchor(line) = base62(hash(line))` over the
line's exact content; only a single trailing `\r` is dropped (CRLF/LF
hygiene). Consequences:

- `{`, `  {`, `\t{` are **different anchors** — nested braces are directly
  addressable; the duplicate-anchor problem shrinks to *byte-identical lines at
  the same indent* (two col-0 `}`s, consecutive blanks, repeated `foo();`),
  which `old_string` already handles. **This largely de-scopes #7** (see C1).
- `foo bar` ≠ `foobar` (raw bytes differ).
- **Cost:** formatter reindent moves indented lines' anchors → the anchors are
  stale after rustfmt/prettier. In an agentic loop the model re-reads the
  formatted file anyway to see what landed, so this is a bounded re-read, not a
  correctness gap. Explicitly traded for the ambiguity win above.

**Consequence: breaking anchor-rule change (2nd this phase).** Anchors are
never persisted (stateless), blast radius is one session at most; do it while
the phase is young. The `ServedOcc` fingerprints in C compare raw lines now
(canon no longer exists). Still order **D1 before C**.

**Tests.** `anchor("{") != anchor("  {")`, `anchor("}") != anchor("\t}")`,
`anchor("foo bar") != anchor("foobar")`, `anchor("  foo   bar ") != anchor("foo bar")`,
`anchor(line) == anchor(line + "\r")`, and exact-match equality.
(Landed in working tree; whole suite green.)

### D2. Anchor length stays 5.

Rejected long ago and re-confirmed: 6 chars (62^6=56B) is overkill; 5 has
measured <~1.4% accidental collision at 5k distinct lines and those are
recoverable via `old_string`. Not revisited in Phase 2.

---

## E. Stretch / deferred-again

- **#8 served-range verification** (refuse to edit lines `read` never showed):
  depends on the #7 store's "what was served" — it rides along on the backlog
  item (see `docs/superpowers/backlog/…session-memory-duplicate-anchors.md`).
  Still **off by default**; layer it as a `Hooks` (the README's designated
  place for policy) rather than in `edit`, so a fork can opt in without
  changing tool behavior.
- **#12 grep output shape**: anchors on every result line are what make
  hit→read→edit seamless; a `count`/`-l`-style "files only" mode is a follow-up
  if grep output ever dominates context. Not a Phase-2 blocker.
- **Non-adjacent occurrence numbering** — carried to backlog with the C design.
- **Cross-session / on-disk anchor store** — explicitly out of scope forever;
  the whole point is stateless self-healing.

---

## F. Suggested implementation order (dependency-aware)

| step | item | depends on | why |
|---|---|---|---|
| ✅ landed | **D1** raw-hash anchors (option B) | — | breaking; done in working tree |
| ✅ landed | **A1** degenerate empty files | — | small correctness |
| ✅ landed | **A2** read size guard + truncation | — | token safety |
| — | **C** session-memory anchors | D1 | **MOVED TO BACKLOG** (D1 de-scoped it to same-indent byte-identical lines) |
| ✅ landed | **B2** `edits` batching tool | — | all-or-nothing single-file batch; only needs edit's own overlap machinery |
| ✅ landed | **B1** `ast_edit` | — | optional-tool pattern; atomic single-file rewrite + diff echo |

Each item is independently revertable; nothing past step 1 changes the model
never writes the wrong line.

## G. Open decisions for review (consolidated)

1. **D1 anchor rule → option B RESOLVED**: anchors hash raw content (canon
   removed; `{` ≠ `  {` ≠ `\t{`), CRLF-stripped. Breaking, landed in working
   tree, whole suite green. Trades formatter-survival for direct nested-brace
   addressing; de-scopes #7.
2. **B1 `ast_edit` RESOLVED**: commit-on with diff-echo (dry-run kept as
   `commit:false`), landed.
3. **B2 `edits` RESOLVED**: all-or-nothing atomic batch, landed (single file
   per call enforced; echo capped; each error names the op).
4. **C6** — carried to backlog with the C design (whole-file occurrence numbering; adjacent-run-only withdrawn).
5. **C7** — carried to backlog with the C design (single alphanumeric suffix char).
6. **C5** — carried to backlog with the C design (external change + duplicate
   → refuse and re-read; own/external write split; no fingerprint widening).
7. **Sizing RESOLVED**: `MAX_LINE_CHARS=300` / `MAX_READ_LINES=1000` landed
   with A2; constants are documented as tunable-in-code if sessions show
   different economics.