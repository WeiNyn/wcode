# D007 — remove hash-based line addressing from the tools

- **Status:** accepted (direction); implementation staged by the W007 brief
- **Date:** 2026-10-09
- **Supersedes / relates to:** `README.md` §"Design: content-addressed editing"
  (`README.md:UQZBx`) and the anchor rules in `AGENTS.md` §Gotchas (`AGENTS.md`). It
  supersedes the **addressing mechanism**, not the no-drift *property*: text matching
  is content-addressed too (see Decision). W006 Phase 2 is held (`git stash`, message
  "W006 Phase 2 (prose trim) — HELD pending the anchor-removal decision") because most
  of what it trims is about to be deleted; its non-anchor trims survive.

## Context

`read`/`edit` address lines by a 5-char **content anchor** (`XXa1b│fn main() {`), a pure
hash of the line's raw content, following the hashline ideas in `pi-better-edit`. The
rationale is in the README: line numbers are *positional*, so inserting one line
silently re-points every number below it, and anchors fix that.

Three costs turned out to be measurable, and one is not:

1. **~17 % on every `read`.** The `XXXXX│ ` prefix is 7 chars/line — measured at 16.9 %
   across five real files (239,230 bytes → 40,418 bytes of prefix, ~10,100 tokens). Read
   output **persists in the transcript**, so unlike a one-off prompt tax this compounds.
2. **~776 tokens of prompt** describing the machinery: the `read`/`edit`/`edits`/
   `replace`/`grep` top-level descriptions plus `from`/`to`/`old_string`/`expected_digest`.
3. **Ergonomics, observed.** In one session (the W006 one) the model made **5 ×
   `E_BAD_ANCHOR`** failures — passing a *line number* where an anchor was required — plus
   `E_STALE_ANCHOR` and `E_STALE_DIGEST` rejections. ~8 failed calls from the addressing
   scheme alone. A model reproduces natural text far more reliably than it transcribes an
   opaque 5-char token.
4. **A hazard with a compensating control.** `from`/`to` verify the **boundaries only**;
   every line between them is clobbered unseen. `expected_digest` (whole-file) exists to
   compensate — which is why W004 (the digest-refresh bug) was possible and why `edit`'s
   own success text contradicted the guard. Text matching has no such hazard: you
   reproduce the whole block, so nothing is clobbered unseen, and **the match is the
   stale check**.

## Decision

**Address lines by their literal text, not a hash of it.** Text matching is *also*
content-addressed — the README's no-drift argument survives intact; only the hash goes.

- `read` prints plain `cat -n` lines. Line numbers remain a **display convenience**, never
  an address.
- `edit` becomes text-match (`old_string` → `new_string`, unique unless `replace_all`).
  `replace` is already exactly that and **merges into `edit`**; `edits` becomes a batch of
  text-match ops.
- `write` loses `expected_digest` (nothing to guard — the write is the whole file).
- **Deletes:** `crates/wcode-cli/src/tools/anchor.rs` (440 lines), the digest machinery in
  `workspace.rs` (`expected_digest`, `stale_digest_guard`, `note_read`,
  `parse_digest_header`, the `WorkspaceHooks` digest cache), the ambiguity/candidate
  machinery, and W004's fix with the bug class it addressed.

**The citation convention changes with it, and this is the real blast radius.**
`file:anchor` is not only an editing mechanism — it is this repo's **evidence currency for
the team**. `explorer.md:20` and `reviewer.md:27` instruct every role to cite it, with an
explicit rationale ("Cite the anchor, never a line number — line numbers drift"). The
anchors come *from* `read`; remove them and the agents have nothing to cite. Replacement:
**`file:line` + a quoted snippet** — the number locates, the snippet pins. Drift matters for
*editing*, not for a citation that is consumed immediately.

Surface: ~822 mentions across 66 files (`.wcode/agents` 6/10, `.wcode/skills` 8/48,
`docs` 32/246, `crates` 18/491, `README.md`+`AGENTS.md` 2/27).

## Consequences

- **The gain:** ~17 % off every read, ~776 tokens off every prompt, two fewer edit tools,
  ~1,450 lines of machinery and a whole bug class deleted, and fewer ways for the model to
  fail. This is "minimalism is the point" applied to the one place the repo was not minimal.
- **The accepted losses:** (a) no cheap **large-range replacement** — a 40-line range must
  be re-emitted (output tokens, whitespace-exact); `write` covers whole-file, the *medium*
  range is the gap; (b) **unique-match ergonomics** — a bare `}` or `return;` needs
  surrounding context or `replace_all`; (c) the **whole-file staleness guard** — text
  matching catches a local change, not "the file changed somewhere else since you read it".
- **What this forecloses:** re-introducing hash addressing, and `expected_digest`, without
  a new decision superseding this one.
- **What is NOT evidenced — and this is the deciding gap.** The cost side above is
  measured. **Whether the model edits *better* is not**: there is no eval harness here, so
  the quality claim rests on the ergonomic evidence in Context §3 and on first principles,
  not on a measurement. The W007 brief must state this plainly and propose whatever cheap
  A/B is actually available rather than implying a proof.
- Implementation follows in the W007 brief; `docs/` moves nothing (D004 item 58).
