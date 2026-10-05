# Estimate rubric

How to judge **value**, **complexity**, and **risk** for a wcode workstream.
Each judgment carries a one-line *why*; a bare label is not an estimate.

## Value — does this make the product better?

| level | meaning | wcode examples |
|-------|---------|----------------|
| **high** | unlocks new capability or removes a whole class of bug | roles as `.md`; fencing `[workflow]` to `--task` |
| **medium** | improves an existing path measurably | README/`--help` corrections |
| **low** | polish, or speculative | optional node-artefacts-as-files |

Value is about the **user of the tool**, not the elegance of the change. A
refactor that changes no observable behaviour is low-value *as a feature* even
when it is worth doing.

## Complexity — how much code/logic?

| level | meaning |
|-------|---------|
| **low** | prose, or a file move, or one small edit |
| **medium** | a new module, a new field threaded through, or a parser |
| **high** | a new subsystem, or a change that touches many call sites |

Complexity is about the **diff and the reasoning**, not the line count alone. A
one-line change to a boot predicate can be medium-complexity because its
consequences fan out.

## Risk — what can break if this goes wrong?

| level | meaning |
|-------|---------|
| **low** | worst case is a wrong sentence in a doc |
| **medium** | worst case is a boot error or a silently changed behaviour on an edge path |
| **high** | worst case is data loss, a corrupted session, or an unrecoverable state |

Risk rises when the change touches boot ordering, the config fold, session
writes, or anything the user cannot easily undo. A change to a **default** is
riskier than the same change behind a flag.

## Reading the three together

- **high value / low risk** — do it first; it is cheap and it pays.
- **high value / high risk** — must carry named regression tests and a live
  check; gate it behind the plan explicitly.
- **low value / high complexity** — usually defer; say so in the non-scope list
  rather than estimating it as if it were in scope.
- **medium value / medium risk** — the common case; the gate is the mitigation.

## Anti-patterns

- A label with no *why* — unaccountable.
- Estimating the *implementation* of a thing whose *interface* is not yet
  settled — sketch it first (see the `interface-sketch` skill).
- Calling everything "high"/"critical" — that is not an estimate, it is a shrug.
