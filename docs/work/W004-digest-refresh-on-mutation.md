# W004 — Digest refresh on mutation

- **Status:** done — implemented in `acbb8be`; second-layer review APPROVED
- **Work item:** W004
- **Decisions:** extends locked **D1–D5** of `docs/workspace-concurrency.md` (amended §5.5); **no new DNNN** — a bug fix (traceability-log §1)
- **Tracker:** [next-steps.md](../next-steps.md) item 61
- **Author:** session `agent:brainstormer` (brief); orchestrator + developer + reviewer (delivery)
- **Date:** 2026-09-05

## 1. The ask

`edit` right after `edit` is rejected because the whole-file digest cache is not
recalculated after a mutating tool (`edit`, `edits`, `replace`, `write`) succeeds,
forcing a re-`read` after every edit.

Confirmed against source:
`WorkspaceHooks::after_tool_call` (`crates/wcode-cli/src/workspace.rs:riNjI`)
short-circuits unless `call.name == "read"`, so only `read` harvests a digest
(`note_read`, `workspace.rs:GlRY7`). `transform_tool_input`
(`workspace.rs:yWt7O`) then auto-attaches that cached digest as `expected_digest`
to every `MUTATOR_TOOLS` call (`workspace.rs:CJgAC` = `write`/`edit`/`edits`/
`replace`). After a successful mutation the cache still holds the **pre-edit**
digest, so the *next* mutation is armed stale → `stale_digest_guard`
(`crates/wcode-cli/src/tools/mod.rs:m4rVu`) refuses with `E_STALE_DIGEST` → a
mandatory re-read. This directly contradicts `edit`'s own success text ("chain
further edits without re-reading", `edit.rs:cm8KM`).

The plumbing already exists: `read` emits the digest as its **first** line via
`digest_header` (`workspace.rs:TIQ6H`, emitted at
`crates/wcode-cli/src/tools/read.rs:zc7ZZ`), and `parse_digest_header`
(`workspace.rs:ucAO3`) reads it back. The kernel is untouched and
`after_tool_call` receives no `working_dir` (why the digest rides the output
text — noted at `read.rs:PGssb`), so the fix rides the same channel.

### 1.1 Fix in one sentence

Each mutator's **success** arm appends the post-write digest as a **trailer line**
(reusing `digest_header`/`parse_digest_header`), and `after_tool_call` refreshes
the cache from that trailer for a successful mutator — so a mutation re-arms the
cache with the bytes it just wrote, and read → edit → edit needs no re-read.

### 1.2 Facts NOT verified

- **Test/clippy outcomes.** Not run (read-only recon). Every test name below is
  read from source; pass/fail is unknown.
- **The live check** (§7.2) needs a local endpoint (Ollama on `localhost:11434`);
  not run.
- **The TUI render of the trailer.** Reasoned from `body_after_summary`
  (`crates/wcode-tui/src/ui.rs:QoC5x`) that the appended line shows as a body line
  under a mutation's summary; not rendered on a real `TestBackend`.
- **`after_tool_call` gets the *transformed* call.** Confirmed at the call site
  `crates/wcode-harness/src/loop_.rs:PyEAo` (it passes `hook_call`, which
  `transform_tool_input` mutated at `loop_.rs:KAjcw`), but not exercised.
- **Handed-in anchor `workspace.rs:PGssb`** does not resolve in `workspace.rs`; it
  resolves in `read.rs:PGssb` (a comment about the missing `working_dir`). The
  handed-in label was slightly drifted.
- **No test asserts exact mutator output** was reported by recon; spot-checked the
  mutator tests (they assert `contains(...)`, `path`, `diff`), found none
  byte-exact — but not audited every assertion in the workspace.

## 2. Scope / Non-scope

**In scope.**

- `crates/wcode-cli/src/workspace.rs`: `after_tool_call` (`riNjI`) refreshes the
  cache for a successful `MUTATOR_TOOLS` call; a new `note_mutation` (sharing
  `parse_digest_header` with `note_read`).
- `crates/wcode-cli/src/tools/mod.rs`: a shared `digest_note(path, written)`
  helper producing the trailer.
- The four mutator **success** arms append the trailer: `edit.rs` (`cm8KM` region
  / `Ok(_)` at `PLRYn`), `edits.rs` (`KGOWL` region / `PLRYn`), `replace.rs`
  (`2B9C5`), `write.rs` (`Hisyu`).
- Tests: per-tool trailer unit tests + the real-seam read→edit→edit chain and its
  peer-rewrite negative (§7.1).
- Docs: amend `docs/workspace-concurrency.md` (§5 settled list + §4 sketch note)
  and add tracker item **61** to `docs/next-steps.md`.

**Non-scope.**

- **`ast_edit`.** A workspace mutator (`ast_edit.rs:7xIXi`, harness
  `MUTATING_TOOLS` `hooks.rs:osi4C`) but with **no** `expected_digest` and **not**
  in the hook's `MUTATOR_TOOLS` (`workspace.rs:CJgAC`) — wholly outside the digest
  policy today. Stays out. **Residual (named):** read `f` → `ast_edit f` (success;
  cache still holds the pre-`ast_edit` digest) → `edit f` (armed stale) →
  `E_STALE_DIGEST` → re-read. Its output already says "re-read … for fresh anchors"
  (`ast_edit.rs:S4dsA`). A separate change could give `ast_edit` a guard *and* a
  refresh.
- **The kernel** (`wcode-harness`): no change. `ToolOutput` (`tool.rs:aFN7f`) gains
  no digest field; the trailer rides `output`.
- **`ToolOutput.diff`/`.path`** presentation fields: untouched.
- **Suppressing the trailer in the UI**: a cosmetic follow-up, not this fix. The
  trailer shows as a body line under an expanded mutation (§4.5).
- **`[workspace] digest_cas = false` still emits the trailer** (the tool cannot read
  the flag); the hook ignores it. Accepted — the digest is informational, not secret.
- **`bash` file writes** (D3): still unseen by the cache. Unchanged.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| 1 | Hook refresh (`after_tool_call` + `note_mutation`) | **high** | **low** | **medium** | Removes the forced re-read (the whole bug); ~15 lines. Risk: the cache is written more often, so a wrong key/parse arms a *wrong* digest — mitigated by hashing the exact bytes written, not a re-read. |
| 2 | Emit sites in the 4 mutators | **high** | **low** | **low** | One `push_str(digest_note(...))` per success arm; append-only, so no `contains`-based test breaks. |
| 3 | Tests (unit trailer + real-seam chain + peer negative) | **high** | **medium** | **low** | The chain test is the only proof the loop's real order re-arms correctly; the peer-rewrite negative is the safety guarantee. |
| 4 | Docs (`workspace-concurrency.md` §4/§5 + tracker 61) | **medium** | **low** | **low** | Prose + one row; the settled-questions list is the single index for this policy. |

## 4. Interface & structure

### 4.0 Locked design decisions (the six the brief must settle)

1. **Mechanism.** Each mutator's *success* arm appends the post-write digest to
   `out.output` (reusing `digest_header`, `workspace.rs:TIQ6H`); `after_tool_call`
   (`riNjI`) updates the cache for a successful mutator by parsing it. Constraint
   met: the kernel gains nothing and `after_tool_call` needs no `working_dir`.
   **Confirmed** over the alternative (hook computes) — see decision 6.
2. **Parse position.** `read` keeps the **first** line (unchanged, published
   contract — the `plain:true` negative at `workspace.rs:fygZo`/`read.rs:fGsZb`
   depends on it). Mutators **append**, so their digest is the **last** parseable
   line; harvest by scanning `out.output.lines().rev()`. Recommendation: **unify**
   into one `harvest_digest(text, from_end)`; `note_read` calls `from_end=false`
   (byte-identical to today's `.lines().next()`), `note_mutation` calls
   `from_end=true`. **Pinned:** the `edit`/`edits` "Region now:" echo is
   anchor-rendered (`anchor::render` `anchor.rs:ER6KN` = `ANCHOR│content`,
   `ANCHOR_SEP` `anchor.rs:77MLC`), and `parse_digest_header` requires a `# `
   prefix — so an echoed file line can never be mistaken for a digest header.
3. **Which arms refresh.** Only arms that actually wrote. `edit` no-op (`edit.rs:0E8vn`,
   `is_error:false`) and `edits` no-op (`edits.rs:n9u6e`) **do not** append → no
   cache write. All `is_error:true` arms append nothing, and `after_tool_call` also
   skips `out.is_error`.
4. **Unguarded mutations refresh too.** A successful mutation with no
   `expected_digest` still wrote known bytes, so refreshing is sound and strictly
   better: the cache's meaning is "last known digest of this path", set from the
   bytes actually written — no case arms a *wrong* value.
5. **`ast_edit` out of scope** (named residual in §2).
6. **Digest source: the tool emits it (uniform).** `write` knows `args.content`;
   `edit`/`edits`/`replace` know `updated`. The hook **cannot** compute it — no
   `working_dir`, and a re-read would be a racy TOCTOU against a concurrent peer. So
   each tool hashes its own exact bytes and emits the trailer. Uniform across all four.

### 4.1 Added

- `crates/wcode-cli/src/tools/mod.rs` — new helper:
  ```rust
  /// The trailer appended to a mutator's SUCCESS output: the post-write whole-file
  /// digest in read's `# <path> digest <hex>` shape, so `after_tool_call` harvests it
  /// with the same parser. Leading `\n` keeps it the LAST line.
  pub(crate) fn digest_note(path: &str, written: &str) -> String {
      format!("\n{}", crate::workspace::digest_header(
          path, &anchor::file_digest(written.as_bytes())))
  }
  ```
- `crates/wcode-cli/src/workspace.rs` — new methods/helper:
  ```rust
  /// Harvest the digest a trailing `# … digest …` line carries. `from_end:false` is
  /// read's contract (line 1 only); `from_end:true` scans the mutation trailer.
  fn harvest_digest(text: &str, from_end: bool) -> Option<String>;
  fn note_mutation(&self, name: &str, args: &serde_json::Value, out: &ToolOutput);
  ```
  `note_mutation` = `target_path(name,args)` (`workspace.rs:3pVt9`) →
  `harvest_digest(out.output, true)` → `last_read.insert(cache_key(&path), digest)`
  (`cache_key` `workspace.rs:SXWsc`).

### 4.2 Changed (with `file:anchor` integration points)

| site | anchor | change |
|------|--------|--------|
| `WorkspaceHooks::after_tool_call` | `workspace.rs:riNjI` | after the `digest_cas`/`is_error` guard: `read` → `note_read`; else if `MUTATOR_TOOLS.contains(name)` → `note_mutation` |
| `WorkspaceHooks::note_read` | `workspace.rs:GlRY7` | route the first-line parse through `harvest_digest(out.output, false)` (behavior identical) |
| `Edit::execute` success arm | `edit.rs:cm8KM` / `Ok(_)` at `edit.rs:PLRYn` | append `digest_note(&args.path, &updated)` |
| `Edits::execute` success arm | `edits.rs:KGOWL` / `Ok(_)` at `edits.rs:PLRYn` | append `digest_note(&args.edits[0].path, &updated)` |
| `Replace::execute` success arm | `replace.rs:2B9C5` | append `digest_note(&args.path, &updated)` |
| `Write::execute` success arm | `write.rs:Hisyu` | append `digest_note(&args.path, &args.content)` |
| `MUTATOR_TOOLS` (reused) | `workspace.rs:CJgAC` | no change; now also consulted by `after_tool_call` |
| `digest_header` / `parse_digest_header` (reused) | `workspace.rs:TIQ6H` / `ucAO3` | no change |
| `stale_digest_guard` (unchanged) | `tools/mod.rs:m4rVu` | no change — the guard still refuses a genuine peer rewrite |

Not touched: `read` emit `read.rs:zc7ZZ`; `ToolOutput` `tool.rs:aFN7f`;
`MUTATING_TOOLS` `hooks.rs:osi4C`; the loop seam `loop_.rs:KAjcw`/`PyEAo`.

### 4.3 Trailer format & parse rule (the pinned contract)

- Trailer = `"\n" + "# {caller_path} digest {12-hex}"` via `digest_header` — the
  **last** line, no `ANCHOR_SEP`, so it is never an anchor line (mirrors the
  existing `header_roundtrips_and_is_not_an_anchor_line` invariant,
  `workspace.rs:8c4AL`).
- `parse_digest_header` trims; `harvest_digest(text, true)` returns the last line
  that parses. Only our trailer parses (summary/echo lines are non-`# ` or
  anchor-prefixed).
- The path in the trailer is the **caller-supplied** string, matching the hook's
  `target_path` and `cache_key` normalization — the cache stays keyed lexically
  (`workspace.rs:SXWsc`), consistent with `note_read`.

### 4.4 Cache write safety

The trailer digest is hashed from the exact bytes written (`updated`/`args.content`),
not re-read from disk, so there is **no TOCTOU** between the write and the refresh. A
peer write landing *after* our write is detected by the *next* mutation's
`stale_digest_guard` (the safety property of §7.1's negative test).

## 5. Plan

1. **Trailer helper + emit sites.** Deliverable: `tools::digest_note` and the four
   success arms append it. Gate: per-tool unit tests parse the last output line to
   `file_digest(written)` (land 1+2 together).
2. **Hook refresh.** Deliverable: `after_tool_call` refreshes on a successful
   mutator; `harvest_digest` shared with `note_read`. Gate:
   `cargo test -p wcode-cli workspace` — `note_read` tests unchanged and green; new
   `note_mutation` unit test.
3. **Real-seam chain + peer negative.** Deliverable: the two integration tests in
   §7.1 driving the real `Read`/`Edit`/`Write` + the real `WorkspaceHooks`. Gate: both
   green; the chain asserts no `E_STALE_DIGEST`, the negative asserts `E_STALE_DIGEST`
   and a byte-identical peer file.
4. **No-op / error arms pinned.** Deliverable: tests asserting a no-op `edit`/`edits`
   and an erroring mutation leave the cache untouched (and emit no trailer). Gate: green.
5. **Docs.** Deliverable: `docs/workspace-concurrency.md` §4/§5 amended + tracker item
   **61** with status. Gate: link/prose read-through; no build impact.
6. **Full gates + live check.** Deliverable: green workspace tests, clean clippy, one
   live `read`→`edit`→`edit` run. Gate: §6.

## 6. Quality gates

```
cargo test --workspace
cargo clippy --workspace --all-targets        # must be clean (no warnings)
```

Regression tests that must stay green (by name):
`workspace::tests::note_read_parses_the_header_digest`,
`workspace::tests::failed_read_does_not_arm_the_cache`,
`workspace::tests::header_roundtrips_and_is_not_an_anchor_line`,
`workspace::chain::read_arms_the_hook_and_the_real_write_is_cas_guarded` (`workspace.rs:KDjgr`),
`workspace::chain::read_arms_the_hook_and_the_real_edit_is_cas_guarded` (`workspace.rs:MvMxC`),
`workspace::chain::the_real_hook_set_composes_in_order` (`workspace.rs:SwPjF`),
`workspace::chain::a_plain_read_arms_nothing` (`workspace.rs:fygZo`),
plus each mutator's `stale_digest_refuses_the_*` / `matching_digest_allows_the_*`.

Live check (see the `live-verification` skill):
```
cargo run -p wcode-cli -- --base-url http://localhost:11434/v1 -p \
  "create f.txt with one line, then read it, then edit that line twice in a row without re-reading"
```
Expected: the **second** `edit` succeeds (no `E_STALE_DIGEST`). With a refused port
(`http://127.0.0.1:9/v1`, `WCODE_RETRY_MAX=2`) the retry path is unaffected.

## 7. Testing

### 7.1 Automated

- **Per-tool trailer units** (one per mutator): after a *success*,
  `out.output.lines().last()` parses via `parse_digest_header` to
  `anchor::file_digest(<written bytes>)`. e.g. `write_success_appends_the_post_write_digest`,
  `edit_success_appends_the_post_edit_digest`, `edits_…`, `replace_…`.
- **Trailer is not an anchor line** (`edit.rs`): assert the last line contains no
  `anchor::ANCHOR_SEP` and that a mid-output anchor-prefixed line parses to `None`.
- **Hook harvests the trailer**: `note_mutation_arms_the_cache_from_the_last_line`
  (`workspace.rs`) — feed a `ToolOutput` whose last line is `digest_header(...)`,
  assert `last_read[cache_key(path)] == digest`.
- **The key regression (real seam):** `a_second_edit_without_a_reread_succeeds` —
  real `Read.execute` → `after_tool_call` → `transform_tool_input`(edit1) → real
  `Edit.execute` → `after_tool_call`(edit1) → `transform_tool_input`(edit2, armed with
  the **post-edit1** digest) → real `Edit.execute`. Asserts the second edit is
  `!is_error`, the file holds both edits, no `E_STALE_DIGEST`.
- **The safety property must not regress:** `a_peer_rewrite_between_two_edits_is_refused`
  — same chain, a peer `std::fs::write` lands between edit1 and edit2. Asserts edit2 is
  `is_error` with `E_STALE_DIGEST` and the peer's bytes are **untouched**.
- **Non-writing arms do not refresh:** `a_noop_edit_does_not_refresh_the_cache` and
  `a_failed_edit_does_not_refresh_the_cache` — cached digest unchanged, no trailer.
- **Unguarded success refreshes:** `an_unguarded_write_success_arms_the_cache` — a
  `write` with no `expected_digest` (cache empty) → `after_tool_call` → the next
  mutation's `transform_tool_input` attaches the post-write digest.

### 7.2 How a human verifies it

```
# 1. gates
cargo test --workspace && cargo clippy --workspace --all-targets

# 2. live, keyless local model
cargo run -p wcode-cli -- --base-url http://localhost:11434/v1 --no-instructions
#   prompt: read f.txt, then change a = 1 → a = 10, then a = 10 → a = 100,
#           each with a single edit call, no re-read between them.
```

Look for: the second `edit` returns `edited …` (not `[E_STALE_DIGEST]`). To check the
trailer, expand a mutation's output in the TUI — a `# <path> digest <hex>` line is the
last body line. Then, as a safety check, rewrite the file between the two edits (or run
two agents on one file): the second edit must refuse with `E_STALE_DIGEST` and leave the
peer's bytes intact. (If no model is reachable, the in-process chain + negative tests in
§7.1 are the fallback proof.)

## 8. Expected outcome

- A successful `edit`/`edits`/`replace`/`write` leaves the digest cache holding the
  **post-write** digest (from the appended trailer), so read → edit → edit → … chains
  with **no** re-read.
- A genuine peer rewrite between two edits is still **refused** with `E_STALE_DIGEST`,
  nothing written — the D1 safety property is intact.
- No-op and error arms leave the cache unchanged.
- The kernel, `ToolOutput`, and `stale_digest_guard` are unchanged; the only new visible
  artifact is one `# <path> digest <hex>` trailer line in a mutation's expanded body.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` green; the live
  read→edit→edit run shows a passing second edit.
- `docs/workspace-concurrency.md` §5 lists the refresh rule as settled;
  `docs/next-steps.md` tracks it as item **61**.

## 9. References, tracker item, and doc placement

- **Tracker item:** `docs/next-steps.md` tops at **item 60**, so the new row is **item
  61** — "Digest cache not refreshed after a mutation (W004; see
  `work/W004-digest-refresh-on-mutation.md`)".
- **Design doc:** **no new `docs/*.md`.** Amend **`docs/workspace-concurrency.md`**: §4
  (the `WorkspaceHooks` also *refreshes* the cache after a successful mutation) and §5
  (a settled item: "Refresh after a mutation → the tool's success output carries the
  post-write digest as a trailer; the hook harvests it"). Its D1–D5 are this policy's
  home; a second doc would split it. (An ADR would be `docs/decisions/D006-*`, but this
  is a settled detail of the existing D-space, not a new cross-cutting decision.)

## 10. Log

- **2026-09-05 — opening.** Brief authored (recon + structured brief). Scope: refresh the digest cache after a successful mutator so read → edit → edit needs no re-read; the four mutators append a post-write digest trailer; `after_tool_call` harvests it. Non-scope: `ast_edit`, the kernel, `bash` writes. Human sign-off given.
- **2026-09-05 — first layer (sketch).** 10 comment-only blocks placed across 6 files. Reviewer **BLOCKED** on 2 items (a hollow no-op test; a wrong anchor claim); both fixed with sentinel framing and a corrected anchor, amendments A–C folded. Re-review **APPROVED**.
- **2026-09-05 — implementation.** Commit `acbb8be` (`cli: refresh the digest cache after a successful mutation`): `harvest_digest` (free fn), `note_read` routed through it, `note_mutation`, the `after_tool_call` dispatch, `tools::digest_note`, the four success-arm appends, +12 tests, and the two docs. ZERO `SKETCH` markers.
- **2026-09-05 — second layer.** Reviewer **APPROVED**: exact diff, tests have teeth (the real-seam chain fails without the refresh; the peer-rewrite negative asserts `E_STALE_DIGEST` + untouched bytes; the sentinel negatives catch a wrong refresh), amendments landed, `git show acbb8be | grep -c SKETCH` = 0. Gates: `cargo test --workspace` 1170 passed / 4 ignored; `cargo clippy --workspace --all-targets` clean. Live `read→edit→edit` endpoint check not run (no local model reachable); the in-process chain tests stand as the behavioural proof.
- **Not done (named residual).** `ast_edit` remains outside the digest policy: read → `ast_edit` → `edit` still forces a re-read. A separate change could give `ast_edit` a guard and a refresh.
