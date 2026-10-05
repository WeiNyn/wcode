---
name: brainstorm-brief
description: "Use when a vague request must become a structured, reviewable brief before any code is written — scope, non-scope, value/complexity/risk estimates, the interface, a numbered plan, quality gates, an expected outcome, and a test plan. Not for implementing the change, and not for a one-line task whose scope is already obvious."
---

# Brainstorm → brief

Turns an underspecified request into a **brief** a developer and a reviewer can
act on. The brief is the contract: it states what will change, what will not,
what it is worth against what it costs, and how it will be proven. It ends with
a brief, never with code.

A brief is written **before** implementation. If the answer to a step is
already known and obvious, write it down anyway — the value is the written
record, not the discovery.

## §0 — Recon before you write

The brief rests on facts. Before writing anything:

1. **Read the code** the request touches. Name the functions and cite
   `file:anchor` for every integration point.
2. **Read the docs** the request relates to — `README.md`, `AGENTS.md`,
   `docs/next-steps.md`, the matching `docs/*-plan.md`.
3. **Say what you did not check.** A brief that asserts an unverified fact is
   worse than one that admits the gap.

If the request is genuinely ambiguous on a point that changes the plan, ask
**exactly one** question — never a battery. Otherwise proceed.

## §1 — The eight sections

Write the brief with these sections, in this order. Use
[`assets/brief-template.md`](assets/brief-template.md) verbatim as the skeleton.

| § | section | the question it answers |
|---|---------|-------------------------|
| 1 | **The ask** | what was requested, in one paragraph |
| 2 | **Scope / Non-scope** | two explicit lists — what is in, what is deliberately out |
| 3 | **Estimates** | value / complexity / risk per workstream, each with a one-line why |
| 4 | **Interface & structure** | what is added or changed, with `file:anchor` integration points |
| 5 | **Plan** | numbered steps, each with its deliverable and its gate |
| 6 | **Quality gates** | the exact commands and the exact expected output |
| 7 | **Testing** | 7.1 automated; 7.2 how a human verifies it |
| 8 | **Expected outcome** | what the world looks like when this is done |

## §2 — Scope and non-scope

Two lists, both explicit. The **non-scope** list is the important one: it is
where you foreclose scope creep before it starts. A non-scope entry names a
thing a reasonable reader might assume is in scope and says why it is not.

- Good non-scope: "Moving existing docs — a separate change with its own link
  check."
- Bad non-scope: "Other work." (Names nothing.)

If the repo's own rules put something out of scope (e.g. `AGENTS.md`'s absent-
by-design list), cite them.

## §3 — Estimates

One row per workstream, three judgments, each with a **one-line why** — not a
bare label. Judge against
[`references/estimate-rubric.md`](references/estimate-rubric.md).

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|

Value answers "does this make the product better"; complexity answers "how much
code/logic"; risk answers "what can break if this goes wrong".

## §4 — Interface & structure

State what changes, and **ground every integration point in the real code**:

- New files by path; changed files by function name.
- For each touch point, cite `file:anchor` — the anchor is a hash of the line,
  so it survives edits above it and fails loudly if the code moved.
- Separate "added" from "changed" so a reviewer can see the shape of the diff.

Never cite a line number (it drifts). Never describe a function from memory.

## §5 — The plan

Number the steps. Each step is:

```
<N>. **<name>.** Deliverable: <what exists after this step>. Gate: <how it is proven>.
```

A step with no deliverable is not a step. A step with no gate is not verifiable
— give it one (a command, a test name, an inspected output).

## §6 — Quality gates

The exact commands and the exact expected output. For this repo:

- `cargo test --workspace` — all tests pass.
- `cargo clippy --workspace --all-targets` — clean.
- A live check against a real binary (see the `live-verification` skill) — a
  build alone proves nothing.

Name the regression tests the change must keep green, by test name.

## §7 — Testing

### 7.1 Automated

List the tests that will exist: unit, seam, and the named regression pair. State
what each **asserts about behaviour**, not merely that it runs.

### 7.2 How a human verifies it

Copy-pasteable commands and what to look for in the output. This is the section
a reviewer actually runs. If a step needs a running endpoint, say so.

## §8 — Expected outcome

A short list of end states. Each is checkable. "Better" is not an outcome;
"`--dump-system-prompt` lists all nine skills" is.

## §9 — References

Link the docs the brief builds on, and the sibling artefacts (the sketch, the
work item). A brief with no references is a brief built on nothing.

## Editing this file

Keep the frontmatter **quoted** and on one line: an unquoted `: ` is invalid
YAML and the loader drops the whole skill with only a stderr warning.
`description` is capped at 1024 chars and clipped to 200 in the prompt, so the
"We use when… Not for…" trigger must sit in the first 200 characters.
Validation: `crates/wcode-cli/src/skills.rs:efuUM` (name), `:Vlnli`
(description cap), `:ftQCJ` (prompt clip 200).
