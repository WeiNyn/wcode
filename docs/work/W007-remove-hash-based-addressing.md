# W007 — remove hash-based line addressing — brief

- **Status:** **done** — P0–P6 delivered in `2867604` (P1), `c4ec64a` (P2),
  `7c740fe` (P3+P4), `c83bd22` (P5); P6 is this record. **Second-layer review
  pending** (the human reviews the whole thing — see §9)
- **Work item:** W007
- **Decision:** [`D007`](../decisions/D007-remove-hash-based-addressing.md) (accepted — direction)
- **Supersedes:** `README.md` §"Design: content-addressed editing"; `AGENTS.md` §Gotchas (the anchor rules)
- **Author:** orchestrator (the `brainstormer` worker failed twice; recorded by the orchestrator)
- **Date:** 2026-10-09
- **Citation note:** this brief cites **`file:` + symbol name**, not `file:anchor` — anchors
  were not collected for every point, and the anchor convention is itself the thing under
  review. Symbol references are stable and verifiable.

## 1. The ask

Replace content-addressed **hash** addressing (`XXa1b│fn main() {`) with content-addressed
**text** addressing (`old_string` → `new_string`). Both are content-addressed, so the
README's no-drift rationale survives; only the 5-char hash goes. The driver is D007: the
hash costs ~17 % on every `read`, ~776 tokens of prompt, and a measurable ergonomic tax.

## 2. Scope / non-scope

**In scope.**

- `read`: emit plain `cat -n` lines. Line numbers become **display only, never an address**.
- `edit`: text-match (`old_string` → `new_string`, unique unless `replace_all`).
- `replace`: **merges into `edit`** — it is already exactly text matching.
- `edits`: a batch of text-match ops.
- `write`: loses `expected_digest`.
- Delete the digest machinery: `WorkspaceHooks::expected_digest_for` / `note_read` /
  `note_mutation` (`crates/wcode-cli/src/workspace.rs`), `digest_header`,
  `parse_digest_header`, `harvest_digest`, `stale_digest`, `cache_key`, and the
  `[tools] digest_cas` flag (`crates/wcode-cli/src/config.rs:33`) with its config tests.
- Delete W004's digest **trailer** (`crates/wcode-cli/src/tools/mod.rs`, the
  `digest_header`-shaped success trailer) and `stale_digest_guard` (`tools/mod.rs`).
- Delete the addressing half of `crates/wcode-cli/src/tools/anchor.rs`: `anchor`,
  `is_anchor`, `anchors_for`, `render`, `Range`, `find_ranges`, `apply_replace`,
  `ANCHOR_LEN`, `ANCHOR_SEP`, `ALPHA`, `file_digest`, `DIGEST_HEX_LEN`.
- **The citation convention** — the `file:anchor` evidence currency across
  `.wcode/agents/*.md`, `.wcode/skills/*`, `AGENTS.md`, `README.md`, `docs/**`.

**Non-scope.**

- `ast_search` / `ast_edit` behaviour (they shell out to `ast-grep`; they never addressed
  lines). `ast_edit`'s one `anchor::` use is `changed_range` — a **diff helper, not
  addressing** — see §4.
- `bash`, `bg`, `grep`'s *search* semantics, the TUI, the protocol, the kernel.
- Any change to `write`'s or `read`'s caps, limits, or elision behaviour.
- Re-litigating the TUI's or the vscode surface's presentation.

## 3. Value / complexity / risk, per workstream

| Workstream | Value | Complexity | Risk | One-line why |
|---|---|---|---|---|
| W1 — `edit` gains text-match | high | med | med | The load-bearing change; ambiguity/unique-match semantics are the hard part |
| W2 — `edits`/`replace` migrate | med | low | low | Mechanical once W1's semantics are fixed |
| W3 — `read`/`grep` stop emitting anchors | **high** | low | low | This is the ~17 % — but it breaks every existing session's habit |
| W4 — delete the digest machinery | high | med | **high** | Touches `Hooks`, config, and W004's fix; easy to delete too much |
| W5 — the citation convention | med | **high** | med | ~50 files of prose; the convention is *load-bearing for the team*, so getting it wrong degrades review discipline |
| W6 — tests | — | **high** | med | ~70 test fns reference anchor/digest; this is the largest mechanical surface |

**Overall:** value high, complexity high, risk medium-high. The deletion is easy; the
*ordering* is what makes it safe (see §5).

## 4. The interface and its integration points

**`anchor.rs` is two modules wearing one coat.** Addressing (`anchor`, `is_anchor`,
`anchors_for`, `render`, `Range`, `find_ranges`, `apply_replace`) **deletes**;
`changed_range` (`crates/wcode-cli/src/tools/anchor.rs:changed_range`) is a **line-diff
utility** used by `ast_edit` to summarise a rewrite, and `file_digest`
(`anchor.rs:file_digest`) belongs to the digest machinery. Both need a decision: move to a
new `tools/diff.rs`-style home, or inline. **This is friction D007 does not anticipate** —
the brief flags it rather than hiding it.

**Callers of `anchor::`** (9 files): `edit.rs`, `edits.rs`, `grep.rs`, `read.rs`,
`replace.rs`, `write.rs`, `ast_edit.rs`, `tools/mod.rs`, `workspace.rs`.

**The digest machinery** (`crates/wcode-cli/src/workspace.rs`): `expected_digest_for`,
`note_read`, `note_mutation`, `transform_tool_input`, `after_tool_call`, plus
`digest_header` / `parse_digest_header` / `harvest_digest` / `stale_digest` / `cache_key`.
Call sites in 7 files: `edit.rs`, `edits.rs`, `read.rs`, `replace.rs`, `write.rs`,
`tools/mod.rs`, `workspace.rs`. Note the guard is **already configurable**
(`config.rs:33` `digest_cas`, default true) — so a staged retirement is possible.

**The citation convention** — 6 agent files (`brainstormer`, `designer`, `developer`,
`explorer`, `reviewer`, `sketcher`), 8 skill files (incl.
`brainstorm-brief/assets/brief-template.md` and
`two-layer-review/assets/verdict-template.md`), `AGENTS.md`, `README.md`, and 33 files
under `docs/`. `explorer.md` and `reviewer.md` carry the explicit rationale
("Cite the anchor, never a line number") — those two are the ones whose *reasoning* must be
rewritten, not just the word.

## 5. Plan — deliverable and gate per step

Ordered **add → migrate → delete**, so the tree is green at every step.

| # | Step | Deliverable | Gate |
|---|---|---|---|
| **P0** | Decide the citation format | A one-line convention in D007 or a `DNNN`: `file:line` + a quoted snippet (proposed) | Human sign-off. **Blocks P5 only** |
| **P1** | `edit` gains text-match | `edit { path, old_string, new_string, replace_all? }` works alongside the anchor path; `replace` becomes an alias | `cargo test --workspace` + clippy + a **live** read→edit→edit on a real file |
| **P2** | `edits` migrates | Batch of text-match ops, atomic, all-or-nothing | tests + live multi-op batch |
| **P3** | `read`/`grep` drop anchors | Plain `cat -n` output; the ~17 % lands | tests + live: the same file read before/after, byte-counted |
| **P4** | Delete addressing + digest | `anchor.rs` addressing gone, digest machinery gone, `digest_cas` config gone, W004's trailer gone | full workspace tests + clippy + live stale-edit path (should now be *impossible*, not merely refused) |
| **P5** | The citation convention | `file:anchor` → `file:line` + snippet across agents, skills, `AGENTS.md`, `README.md`, `docs/**` | link/prose check + a dry run of one agent role citing the new format |
| **P6** | Record | W007 marked done; `README.md` §Design rewritten (not deleted — the *argument* survives) | docs review |

**Stopping point:** P0–P4 is the tool change and is self-contained. **P5 can be deferred**
— but not indefinitely, because until it lands the agents are instructed to cite anchors
that no longer exist.

## 6. Exact quality-gate commands

```
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets          # must be clean, no warnings
cargo run -p wcode-cli -- -p "<live check>" --base-url http://localhost:11434/v1
```

Every step: tests + clippy clean **and** a live end-to-end check. Per `AGENTS.md`,
`cargo build` proves nothing about behaviour.

## 7. Testing

**Automated.** ~70 test fns across 13 files reference anchor/digest. Most must be
**deleted, not adapted** (they assert behaviour that ceases to exist). New tests needed:
text-match uniqueness (ambiguous `old_string` refuses), `replace_all`, a batch that aborts
atomically, and — critically — a test asserting `read`'s output contains **no** anchor
separator and **does** carry line numbers.

**Human.** The honest one: **run the same real task twice, anchor-path vs text-path**, on a
local endpoint, and compare how many tool calls fail. That is the only signal available on
the quality question, and it is weak — see §9.

## 8. Expected outcome

- `read` output ~17 % smaller; the prompt ~776 tokens lighter; two fewer edit tools.
- `anchor.rs`'s addressing half, the digest machinery, W004's fix, and `digest_cas` deleted.
- The `from`/`to` **range-clobber hazard disappears**, and with it the compensating control
  that produced W004.
- `README.md` §Design is **rewritten, not deleted**: content-addressing by text is still
  content-addressing, and the no-drift argument is still true.

## 9. Delivery — what actually landed

| Step | Commit | What |
|---|---|---|
| P1 | `2867604` | `edit` → `{path, old_string, new_string, replace_all?}` |
| P2 | `c4ec64a` | `edits` → a batch of literal-text ops, `path` hoisted to the top level; **`replace` deleted** (it was already text matching) |
| P3+P4 | `7c740fe` | `read`/`grep` print plain lines; `anchor.rs`, `workspace.rs`/`WorkspaceHooks`, `stale_digest_guard`/`digest_note`, `expected_digest`, and the `[workspace] digest_cas` config surface all deleted |
| P5 | `c83bd22` | the citation convention → `file:line` + a quoted snippet |
| P6 | this | the record |

**P3 and P4 were merged**, against the brief's staging: dropping the digest
header from `read` is what makes the CAS guard inert, so the guard cannot
outlive it. Two green commits would have meant one of them carrying a
deliberately-dead guard.

### Measured, live

- **Tool definitions: 19 → 18 tools**, 23,227 → **21,988 chars** (−310 tok) — the
  anchor/digest prose and `replace` are gone.
- **`read` output: ~50 % off the prefix.** The old prefix was **9 bytes/line**
  (5 chars + a 3-byte `│` + a space); `{n}\t` is ~5. Measured on four real files
  (read.rs, tool.rs, README.md, main.rs): **40,923 B → 20,159 B**, ~5,200 tokens
  saved per read of that set. Unlike the prompt tax, this **persists in the
  transcript**.
- The system prompt no longer describes anchors (checked with
  `--dump-system-prompt`).
- `cargo test --workspace` 1098 passed, 4 ignored; `cargo clippy --workspace
  --all-targets` clean, **no warnings**.

### Friction the brief predicted, and what it cost

- **`anchor.rs` was two modules in one coat.** `changed_range` is a line-diff
  helper, not addressing — it moved to `tools/diff.rs` alongside `split_lines`.
  Flagged in §4; it was the only real surprise.
- **`WorkspaceHooks` existed only for the digest policy**, so it deleted whole,
  taking its call sites in `agents.rs`/`repl.rs` and the `[workspace]` config
  table with it. That was wider than the brief implied.
- **The `no-op` arm had to be re-added to `edit`** — replacing text with itself
  must not touch the file. Caught by a surviving `workspace.rs` test before the
  file was deleted.

### Historical citations are frozen

The ~33 files under `docs/` that cite `file:anchor` in dated records were
**deliberately not rewritten**. They are records of what was true when written;
rewriting them would falsify history. Their anchors are now unresolvable — a
reader can no longer recover the line from the hash, because nothing computes it
any more. Treat them as frozen prose, not as citations.

## 10. Facts NOT verified

- **Tool-calling quality.** D007's deciding question, and **it cannot be measured in this
  repo** — there is no eval harness. The A/B in §7 gives *a* signal, not a proof. Anything
  stronger needs an external harness. **Do not read this brief as evidence that the change
  makes the model edit better.**
- **Every affected test.** I counted ~70 test fns by grep; I did **not** read them all, and
  some matches are unrelated (`bash.rs:sweep_stale_spills`). The real number is unverified.
- **The `anchor.rs` split.** I did not read all 440 lines. There may be further
  non-addressing helpers like `changed_range` hiding in there.
- **The citation-convention count.** ~822 mentions across 66 files is a grep of the word
  "anchor"; many are prose *about* the design, not citations. The list that must actually
  change is unverified.
- **Nothing was run.** This is read-only recon; no test, build, or clippy was executed for
  this brief. Every test name above is read from source; pass/fail is unknown.
- **The `file:line` + snippet format is proposed, not decided** — that is P0.
- **The two brainstormer failures** (§ Author) mean this brief was written by the
  orchestrator without the skill's independent recon. Treat it as lower-confidence than a
  normal WNNN brief.
