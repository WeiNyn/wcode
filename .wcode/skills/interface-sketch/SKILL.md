---
name: interface-sketch
description: "Use before implementing a non-trivial change — sketch the interface first, either as a NEW full module or as an IN-PLACE review-only comment block, ground every integration point in file:line + snippet evidence, and hand the filled sketch to a developer. Not for a tiny change whose interface is one obvious edit, and not for the implementation itself."
---

# Interface sketch

A **sketch** is the interface of a change written down before the change is
implemented: types, field lists, function signatures with parameter and return
types, rustdoc contracts, and `#[cfg(test)]` skeletons with `todo!()` bodies. A
sketch may **not** compile — that is the point. It is reviewed in the real code
so the design's open questions settle before implementation writes any logic.

A sketcher writes **no working logic**. If a line would do the work, it is not a
sketch line.

## §0 — Where a sketch may live

Exactly two forms. Pick one per sketch.

### Form A — a NEW file (a full module)

A whole new module, as a real `.rc` file:

- module-level `//!` docs stating what it is and who calls it;
- the public types, with field lists;
- the public and crate-private `fn`s — signature only, with rustdoc contracts;
- `#[cfg(test)]` test skeletons with `todo!()` bodies.

A new file that does not yet compile is fine **only** if it is not yet a module
of the crate; do not add it to `mod` declarations until it is real.

### Form B — an IN-PLACE comment block

The structs and functions to **add to an existing file**, sketched as comment
lines in that file:

```rust
// ==== SKETCH (review-only, not real code) ====
// pub struct Completion {
//     matches: Vec<usize>,
//     selected: usize,
// }
//
// /// Accept the highlighted row; inserts `/<name> ` and closes.
// fn accept(&mut self, name: &str) { todo!() }
// ==== /SKETCH ====
```

An in-place sketch exists so the reviewer can read the proposed shape **in the
file it will land in**. Placement is evidence: put the block where the code
will go.

## §1 — The one rule that matters

**A sketch never modifies a real code line.** It only adds:

- new comments (Form B), or
- a new, un-registered file (Form A).

If a sketch needs the existing code to change to make sense, say so in the
sketch body as a note — do not change it. The sketch may be non-compiling;
as comments it is inert, which is what makes it safe to review.

## §2 — What every sketch must contain

1. **The contract.** For each `fn`: what it takes, what it returns, what it
   promises, and what it does **not** promise (the rustdoc `# Errors` /
   `# Panics` discipline).
2. **The types.** Field lists, not prose. If a field's type is open, mark it
   `todo!()` or `<TBD>` — make the open question visible.
3. **Test skeletons.** One `#[cfg(test)]` block with `fn <behaviour_name>()`
   stubs and `todo!()` bodies. A stub's *name* is the assertion you intend
   ("blocks an un-gated work node"); it is not thrown away by the developer.
4. **Integration points**, each cited `file:line` + a quoted snippet — the real current code it
   plugs into. Anchor, never line number: the anchor hashes the line, so it
   survives edits above it.
5. **Friction.** Where the design fights the existing code, write it down and
   propose a resolution. That paragraph is often the sketch's real value.

## §3 — The hand-off contract

The sketch is **review-only**. The developer's obligation:

1. **Replace every block with real code.** A `// ==== SKETCH … ====` marker and
   its contents are deleted; a new-file sketch becomes a real module.
2. **Honor the approved signatures** — or record why a signature changed.
3. **Honor the reviewer's amendments** from the first-layer review (see the
   `two-layer-review` skill).
4. **Leave ZERO `SKETCH` markers in the diff.** This is a second-layer review
   precondition, not a nicety: a lingering marker means real logic was never
   written.

## §4 — The sketcher's checklist

Run every box before handing off.

- [ ] Form chosen (A new file / B in-place) and justified.
- [ ] No real code line modified — only comments added, or a new un-registered
      file.
- [ ] Every `fn` has a signature **and** a contract.
- [ ] Every open type/field is marked (`todo!()` / `<TBD>`), not guessed.
- [ ] A `#[cfg(test)]` skeleton exists, with behaviour-named stubs.
- [ ] Every integration point cites `file:line` + a quoted snippet.
- [ ] A "friction" paragraph names where the design fights the code.
- [ ] The developer can fill it without redesigning.

## §5 — References

- The locked design doc the sketch implements (cite it in the sketch header).
- The `two-layer-review` skill — the first-layer gate this sketch feeds.
- The `wcode-conventions` skill — the edit/`write` tooling and the citation rule.

## Grounding

- Citing code: `file:line` + a short quoted snippet (D007). The number locates,
  the snippet pins it; an earlier edit or a reformatter can move the line, so
  never rely on the number alone. See `README.md` §"Design: editing by literal text".
- The repo's own sketch convention is stated in `.wcode/team.toml` (the
  orchestrator guidelines comment) and in the sketcher role.

## Editing this file

Keep the frontmatter **quoted** and on one line — an unquoted `: ` is invalid
YAML. `description` ≤ 1024 chars, clipped to 200 in the prompt, so the "Use when
… Not for …" trigger must sit in the first 200. Validation:
`crates/wcode-cli/src/skills.rs:efuUM` / `:Vlnli` / `:ftQCJ`.
