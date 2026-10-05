---
name: two-layer-review
description: "Use to review a change in two gates — a first-layer review of a sketch against the locked design (settle open questions before implementation), then a second-layer review of the diff against the task (prove the tests assert behaviour, confirm the amendments landed, confirm zero SKETCH markers). Not for authoring the change, and not for rubber-stamping a diff."
---

# Two-layer review

A change is reviewed **twice**, at two different artefacts:

- **Layer 1 — the sketch.** Before any implementation, review the interface
  against the **locked design**. Settle the design's open questions. A sketch
  that reaches a developer without a first-layer verdict is unreviewed.
- **Layer 2 — the diff.** After implementation, review the change against the
  **task**. Confirm the tests assert behaviour, the first layer's amendments
  landed, and every `SKETCH` marker is gone.

Both layers end in the same verdict shape:
[`assets/verdict-template.md`](assets/verdict-template.md).

A reviewer does not write the change and does not rubber-stamp it. "Looks fine"
is not a verdict.

## §1 — Layer 1 — the sketch vs the locked design

The artefact is a sketch (see the `interface-sketch` skill): a NEW file, or an
IN-PLACE `// ==== SKETCH … ====` comment block.

Check, in order:

1. **Fidelity to the design.** Does the sketched interface implement the locked
   design doc? Cite `file:anchor` for each claim about the real code.
2. **Open questions.** Every open question the design left must be **settled**
   here, in the verdict — not deferred to implementation. A settled question is
   recorded as an amendment.
3. **Integration points.** Each cited `file:anchor` must still name the fact it
   claims; a stale or wrong anchor is a blocking issue.
4. **Friction.** Where the design fights the code, does the sketch resolve it
   honestly, or paper over it?
5. **Test intent.** Do the `todo!()` stubs name the behaviours that will prove
   the change? A stub named `test_works` is a blocking issue.

**Amendments** are the layer-1 output a developer must honor. Write each as a
concrete instruction ("add a `&Path` parameter to `X`; it currently assumes the
cwd"), not a suggestion.

## §2 — Layer 2 — the diff vs the task

The artefact is the working diff.

1. **Read against the task.** Does the diff do the task — no more, no less?
   Unrequested work is a blocking issue (it is scope creep).
2. **Do the tests assert behaviour?** A test that passes without the change is
   worthless. For each new test, ask: *would it fail if the logic were wrong?*
   A test that merely exercises a code path without pinning its result is a
   blocking issue.
3. **Did the amendments land?** Every layer-1 amendment must be present in the
   diff, or explicitly renegotiated in the static record.
4. **Zero `SKETCH` markers.** `grep -rn 'SKETCH'` over the diff must return
   nothing in changed files. A marker left behind means logic is missing.
5. **The gates ran.** `cargo test --workspace` and
   `cargo clippy --workspace --all-targets` are clean; a live check was done
   where behaviour (not just compilation) is at stake. Re-run them.
6. **The repo's conventions hold** (see the `wcode-conventions` skill): one
   logical change per commit, imperative area-prefixed subject, no `anyhow`,
   the kernel free of presentation concerns.

## §3 — The verdict

Emit exactly one of:

- **APPROVE** — no blocking issues. Non-blocking notes are listed separately and
  do not gate the change.
- **BLOCKING** — a numbered list of issues, each with:
  - the `file:anchor` evidence,
  - what is wrong,
  - the concrete change that would clear it.

Rules:

- A blocking issue cites `file:anchor`. An issue with no anchor is an opinion.
- Do not mix notes and blockers in one list; label them.
- Do not approve a diff with a `SKETCH` marker, a hollow test, or a moved open
  question.

## §4 — Reviewer's checklist

**Layer 1:**

- [ ] The sketch is a NEW file or an IN-PLACE `SKETCH` block, and no real code
      line was modified.
- [ ] Fidelity to the locked design checked, with `file:anchor` evidence.
- [ ] Every open question settled and written as an amendment.
- [ ] Integration anchors still name the facts they claim.
- [ ] Test stubs name behaviours.

**Layer 2:**

- [ ] Diff does exactly the task; nothing unrequested.
- [ ] Every new test would fail if the logic were wrong.
- [ ] Every layer-1 amendment landed.
- [ ] `grep -rn 'SKETCH'` is clean over the diff.
- [ ] `cargo test --workspace` + `cargo clippy --workspace --all-targets` clean.
- [ ] A live check was done (or the reason it cannot be is recorded).

## §5 — References

- The `interface-sketch` skill — what layer 1 reviews.
- The `wcode-conventions` skill — the conventions layer 2 enforces.
- The `live-verification` skill — how to prove behaviour, not just compilation.
- `docs/swarm-comparison-plan.md` §5 / `crates/wcode-cli/src/verify_gate.rs` —
  the R′ gate rule: a gate is **shape, not enforcement**
  (`crates/wcode-cli/src/verify_gate.rs:9Vm1l`). Never present a gate as proof;
  the reviewer's verdict is the correctness signal.

## Editing this file

Keep the frontmatter **quoted** and on one line. `description` ≤ 1024 chars and
clipped to 200 in the prompt — the "Use to … Not for …" trigger must sit inside
the first 200. Validation: `crates/wcode-cli/src/skills.rs:efuUM`, `:Vlnli`,
`:ftQCJ`.
