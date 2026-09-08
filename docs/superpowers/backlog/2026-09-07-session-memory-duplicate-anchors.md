# Backlog: session-memory for unique duplicate anchors (item C / #7)

Status: **BACKLOG — deferred**, not planned for this phase.
Date moved: 2026-09-07
Source: `docs/superpowers/specs/2026-09-07-read-edit-phase2.md` §C (moved verbatim).

## Why it's on the backlog

The full design below was written when anchors were whitespace-tolerant, which
made every `{`/`}` at any indent share one anchor — a large, painful duplicate
population. **Decision D1 (option B, landed) changed the premises:** anchors now
hash raw content, so nesting level is part of the address and duplicates shrink
to *byte-identical lines at the same indent* (two col-0 `}`s, consecutive blank
lines, repeated `foo();`). `old_string` / `replace_all` already cover those with
plain round-trips. Building the store (checksums, occurrence tables, served
fingerprints, own/external-write split, suffix grammar) for that shrunken case
is complexity we may never need.

## Pull-it-in trigger

- Real sessions show the same-indent duplicate population biting often enough
  that the extra round-trip costs more than the store's complexity, OR
- agents repeatedly fail to disambiguate two col-0 `}`s / repeated statements
  with `old_string`.

If pulled in, keep the hard guarantees: the stateless resolver stays the
authority; memory only shapes *which candidate* an ambiguous `from` prefers and
never authorizes a write on its own; fail closed, never silent.

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