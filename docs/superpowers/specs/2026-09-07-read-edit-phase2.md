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

## C. Session memory for unique duplicate anchors (the big one — #7)

### C1. Problem restated

Stateless anchors are a pure hash of raw line text (decision D1), so only
*byte-identical* lines share an anchor. Today `edit` rejects ambiguous
duplicates with candidates (`[E_AMBIGUOUS_ANCHOR]`) and asks for
`old_string`/`replace_all`. **After D1's raw hashing, the duplicate-anchor
population shrinks dramatically** — nested `}` at different indents are now
distinct anchors — so this item is de-scoped to same-indent identical lines:
two col-0 `}`s, consecutive blank lines, repeated `foo();`. The question this
item answers: *within one session, can the agent address "the 2nd `}`" (at the
same indent) directly, without losing the never-edit-the-wrong-line
guarantee?*

### C2. Architecture

In-memory, per-session, no disk — exactly the spec's Phase-2 cap. New module
`crates/wcode-cli/src/tools/anchor_store.rs`:

```rust
#[derive(Default)]
pub struct AnchorStore {
    paths: HashMap<PathBuf, PathState>,
}

struct PathState {
    checksum: u64,                        // whole-file hash at last view
    occurrences: HashMap<String /*base*/, Vec<usize>>, // every 0-based line index, in file order
    served: HashMap<String /*suffixed*/, ServedOcc>,   // ONLY what this session's reads displayed
}

struct ServedOcc {
    base: String,
    ordinal: usize,                       // 0-based index within occurrences (a hint, NEVER authority)
    prev_canon: Option<String>,           // ┐ neighborhood fingerprint, captured
    next_canon: Option<String>,           // ┘ from the file at the moment read served it
}
```

`prev_canon`/`next_canon` are the canonical (whitespace-stripped) text of the
*immediately preceding / following* line at serve time. The triple
`(prev_canon, <served line's canon>, next_canon)` is an occurrence's
**fingerprint** — its content identity.

Constructed in `default_tools` (same place as the mutation lock) as
`Arc<RwLock<AnchorStore>>`, shared by `read` (serves + records), `edit`
(resolves), and `replace`/`write` (re-set `checksum` + rebuild `occurrences`
after their atomic write). `ToolContext` is unchanged. **A `None` / disabled store
degrades to today's behavior** — the stateless path is still the code path; memory
only decides *which candidate* an ambiguous `from` prefers, and never authorizes a
write on its own.

Diet rule: suffixing activates **only for base anchors that occur ≥ 2 times
anywhere in the current file**. Unique lines (the overwhelming majority) render
the plain 5-char anchor, so the common read costs zero extra tokens.

#### C2.1 Worked example

Example file `src/handlers.rs` (canonical text in `[brackets]`):

```
 1: fn alpha() {          [fnalpha(){]
 2:     ping();           [ping();]   ← duplicate (also lines 5, 8, 11)
 3: }                     [}]          ← duplicate (also lines 6, 9, 12)
 4: fn beta() {           [fnbeta(){]
 5:     ping();           [ping();]
 6: }                     [}]
 7: fn gamma() {          [fngamma(){]
 8:     ping();           [ping();]
 9: }                     [}]
10: fn delta() {          [fndelta(){]
11:     ping();           [ping();]
12: }                     [}]
```

Say `ping();` anchors to `7QwzP` and `}` to `Xv9Kb` (values are illustrative).
`read` outputs (anchored mode), suffixes = **whole-file occurrence ordinal**:

```
 1: fnalpha(){   → plain anchor (unique)
 2: 7QwzP  │    ping();      7QwzP = 1st occurrence → bare base
 3: Xv9Kb  │}                Xv9Kb = 1st occurrence → bare base
 4: fnbeta(){  → plain anchor (unique)
 5: 7QwzP1 │    ping();      7QwzP1 = 2nd occurrence
 6: Xv9Kb1 │}                Xv9Kb1 = 2nd occurrence
 7: fngamma(){  → plain anchor (unique)
 8: 7QwzP2 │    ping();      7QwzP2 = 3rd occurrence
 9: Xv9Kb2 │}                Xv9Kb2 = 3rd occurrence
10: fndelta(){  → plain anchor (unique)
11: 7QwzP3 │    ping();      7QwzP3 = 4th occurrence
12: Xv9Kb3 │}                Xv9Kb3 = 4th occurrence
```

Formed `PathState` (showing the `}` entries fully):

```
PathState { checksum: 0x9f2c1a7…,
  occurrences: {
    "7QwzP" -> [1, 4, 7, 10],   // 0-based line indices of `ping();`
    "Xv9Kb" -> [2, 5, 8, 11],   // 0-based line indices of `}`
  },
  served: {
    "Xv9Kb"  -> ServedOcc { base: "Xv9Kb", ordinal: 0, prev: Some("ping();"), next: Some("fnbeta(){") },
    "Xv9Kb1" -> ServedOcc { base: "Xv9Kb", ordinal: 1, prev: Some("ping();"), next: Some("fngamma(){") },
    "Xv9Kb2" -> ServedOcc { base: "Xv9Kb", ordinal: 2, prev: Some("ping();"), next: Some("fndelta(){") },
    "Xv9Kb3" -> ServedOcc { base: "Xv9Kb", ordinal: 3, prev: Some("ping();"), next: None /* EOF */ },
    //            … `7QwzP…` entries symmetric
  },
}
```

Note two things the example makes concrete:

1. **Every `}` here has a distinct fingerprint** — even though all four are the
   *same canonical line*, their surroundings differ (next line is `fnbeta(){` /
   `fngamma(){` / `fndelta(){` / EOF). So a suffix edit resolves uniquely by
   fingerprint; the ordinal suffix is only what the model *types*, never what
   authorizes.
2. **The duplicates are all non-adjacent** — `ping();` and `}` are separated by
   the `fn …` lines. This is the *common* real-world case (identical closers of
   separate functions), so the numbering must be by whole-file occurrence, not
   by contiguous run. (See C6 — this revises an earlier adjacent-run-only
   proposal.)

### C3. On `read`

1. Compute the file checksum; if it differs from `PathState.checksum`, discard
   that path's state and rebuild `occurrences` from current content (the
   "re-suffix naturally" step).
2. For every base with `occurrences.len() ≥ 2`, render occurrences in file
   order as `base` (1st), `base1` (2nd), `base2` (3rd) … Suffixes are the
   **whole-file ordinal, stable across paging**: a `limit`ed read of lines 5–8
   shows the *same* suffix for that `}` as a full read would.
3. Record a `ServedOcc` fingerprint for every suffixed anchor **actually
   displayed this call** — only what the model could have seen. Unseen
   occurrences are never claimable by a suffix.
4. Unchanged: ambiguity notes still list occurrence line numbers when a plain
   base anchor is used ambiguously.

### C4. On `edit`

1. `from` resolves to a base with siblings but **no** `ServedOcc` entry (store
   disabled, or that occurrence not served this session) → today's
   `[E_AMBIGUOUS_ANCHOR]` with candidates. No behavior change.
2. `from` is a suffixed anchor with a `ServedOcc` entry. **Mutation origin
   decides the resolution path.** `PathState.checksum` is updated after every
   *own* write (`edit`/`replace`/`write`, atomically under the mutation lock,
   which know the exact new content), so a **checksum mismatch at edit time
   means an external actor changed the file** since we last saw it:
   a. **No external change** (checksum matches) → resolve by fingerprint over
      current content, never by position:
      - **Exactly one** line matches `(prev_canon, line_canon, next_canon)` →
        that's the target. Edits *elsewhere* (our own prior edit, a formatter
        that didn't fire between calls) that kept the neighborhood intact still
        work — content-addressed.
      - **Zero** → `[E_STALE_ANCHOR]` (`file changed since read`), rebuild and
        serve fresh anchors.
      - **Several** (two byte-identical environments — the irreducible hole) →
        **do not** guess with the stored ordinal; require a unique `old_string`
        (or a re-read) before writing, and verify it exactly as the stateless
        path already does.
   b. **External change** (checksum mismatch) and the base is a **duplicate**
      (`occurrences.len() ≥ 2`) → **refuse** the suffixed-anchor edit with
      `[E_STALE_ANCHOR]` and a "re-read" hint. After an external edit a rebuild
      can re-match a served fingerprint to a *different* line — a copy-pasted
      block can leave a same-neighborhood line in a new segment, where both the
      ordinal and the fingerprint agree on the *wrong* line. Content identity
      cannot decide; a unique fingerprint match there is not proof. One re-read
      costs a round-trip; a wrong write costs a corrupted file. Fail closed.
   c. External change and the base is **unique** → the stateless path, which
      self-heals exactly as today (a unique line cannot be re-derived to a
      different line).
3. Never write before the relevant check passes; every failure is fail-closed
   with candidates, and the stateless resolver remains the backstop for any
   call that bypasses the store.

### C5. External-edit handling (the design's load-bearing piece)

| mutation since last view (per path) | target | verdict |
|---|---|---|
| none (`checksum` still matches — our own write, or unchanged since served) | unique line | stateless anchor, exactly as today |
| none | duplicate | fingerprint resolves; ≥2 identity-equal matches → require `old_string` or re-read |
| external tool changed the file | unique line | self-heals via stateless anchor, as today |
| external tool changed the file | **duplicate** | **refuse suffixed-anchor edit → re-read** (the "different segment" risk) |

Rules behind this:

- **Checksum = the own/external discriminator.** `edit`/`replace`/`write` set
  `PathState.checksum` to the content they wrote (they know it exactly, under
  the mutation lock); every `read` serves and records. A mismatch therefore
  means an *external* actor changed the file between two of our operations.
  One pass per call; cheap, and it never trusts memory across an external edit.
- **Fingerprint (fine) authorizes only inside a live world.** The checksum
  match (no external change) is the precondition; within it, the fingerprint is
  content identity extended from lines to occurrences — our own prior edits and
  unstaged formatter runs that keep the neighborhood intact still resolve.
- **External change + duplicate = refuse, not self-heal.** A rebuild after an
  external edit can re-match a served fingerprint to a *different* line (the
  new close of a copy-pasted block); ordinal and fingerprint then agree on the
  wrong segment. Widening the fingerprint does **not** close that — structurally
  identical surroundings are still content-identical. The only sound choices
  are refuse-and-re-read (taken) or ambiguity-refusal (the no-change hole below).
- **Irreducible hole (no-external-change flow):** two occurrences whose
  immediate surroundings are byte-identical are indistinguishable by content. We
  refuse to guess — `old_string` context beyond the identical neighborhood (or a
  re-read) is required. **Fail loud, never silent.**

### C6. Interaction with the rest of the tool surface

- `replace` (literal, no anchors) — untouched.
- `replace_all` on a duplicate base — untouched (applies to every occurrence in
  *current* content).
- `write` — full-file; re-sets that path's `checksum` and rebuilds
  `occurrences`/`served` from the new content (an own write, so no false
  external-mutation flag on the next edit).
- **Non-adjacent duplicates ARE covered** (revised from the earlier
  adjacent-run-only proposal): the fingerprint mechanism does not care about
  contiguity, and the painful cases in practice are non-adjacent (`}`s closing
  separate functions, repeated call sites). Suffixing is by whole-file
  occurrence ordinal, exactly as the C2.1 example shows.

### C7. Suffix format decision

`base` (5 chars) = 1st occurrence; `base` + one alphanumeric char for subsequent
ones: `9Bx1`..`9Bx9`, then `9BxA`… (62-alphabet). `is_anchor`/edit parsing
accepts a 5- or 6-char anchor; a 6-char anchor is unambiguous about being
suffixed. Explicit alternative — `~`/`𝄞`/`♯` unicode marks (pi-better-edit
style) — is prettier but breaks the bare-alphanumeric anchor grammar; rejected.

---

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
  the #7 store makes "what was served" knowable. Still **off by default**;
  layer it as a `Hooks` (the README's designated place for policy) rather than
  in `edit`, so a fork can opt in without changing tool behavior.
- **#12 grep output shape**: anchors on every result line are what make
  hit→read→edit seamless; a `count`/`-l`-style "files only" mode is a follow-up
  if grep output ever dominates context. Not a Phase-2 blocker.
- **Non-adjacent occurrence numbering** — see C6 decision.
- **Cross-session / on-disk anchor store** — explicitly out of scope forever;
  the whole point is stateless self-healing.

---

## F. Suggested implementation order (dependency-aware)

| step | item | depends on | why |
|---|---|---|---|
| ✅ landed | **D1** raw-hash anchors (option B) | — | breaking; done in working tree |
| ✅ landed | **A1** degenerate empty files | — | small correctness |
| ✅ landed | **A2** read size guard + truncation | — | token safety |
| 4 | **C** session-memory anchors (de-scoped after D1) | D1 | remaining duplicates = same-indent identical lines only |
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
4. **C6**: whole-file occurrence numbering (all duplicates, adjacent or not)
   — the worked example shows the common case is non-adjacent, so the
   earlier adjacent-run-only limit is withdrawn (revised).
5. **C7**: suffix format = one alphanumeric char (`9Bx1`, `9BxA`) (proposed)
   vs pi-style unicode marks.
6. **C5**: external-mutation policy — **duplicate bases refuse the suffixed
   anchor and demand a re-read** (own/external write split via `checksum`; no
   fingerprint widening — structurally identical surroundings stay
   content-identical). Within the no-external-change flow, byte-identical
   environments require `old_string` past the identical neighborhood, else
   refuse.
7. **Sizing RESOLVED**: `MAX_LINE_CHARS=300` / `MAX_READ_LINES=1000` landed
   with A2; constants are documented as tunable-in-code if sessions show
   different economics.