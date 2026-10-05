---
name: traceability-log
description: "Use to record a decision or open a tracked work item in wcode — the DNNN decision record and the WNNN work item formats, when to open which, and how docs/next-steps.md stays the single index. Not for ephemeral notes, and not for a change small enough that a commit message is the whole record."
---

# Traceability log

Two dated artefacts make multi-step work traceable:

- a **DNNN decision record** (an ADR) — a settled choice, with its context, its
  consequences, and what it forecloses;
- a **WNNN work item** — a brief → plan → gates → outcome for a body of work.

`docs/next-steps.md` stays the **single index**: every work item gets one row
there. The buckets live under `docs/{decisions,work,plans,designs,analysis}/`
(see D004); existing flat docs are not moved as part of adding one.

## §1 — When to open a decision record

Open a **DNNN** when a choice is **locked** and future work must not silently
re-litigate it. Signals:

- Two reasonable options existed and one was chosen *for a reason*.
- The choice changes user-visible behaviour, a default, or a documented
  contract.
- A later reader would otherwise ask "why is it like this?".

Do **not** open one for a reversible implementation detail, a style choice, or a
bug fix. Those live in the commit message.

## §2 — The decision-record format

`docs/decisions/DNNN-<kebab-slug>.md`:

```markdown
# DNNN — <title>

- **Status:** accepted | superseded by DNNN | proposed
- **Date:** YYYY-MM-DD
- **Supersedes / relates to:** <doc names or `—`>

## Context
<why a decision was needed; cite file:anchor for the code facts>

## Decision
<the decision, in the imperative>

## Consequences
<what this enables; what it costs; what it forecloses>
```

Rules:

- Numbers are permanent and never reused. A reversed decision is **superseded**,
  not deleted.
- **Cite `file:anchor` for every fact about the code.** An assertion with no
  anchor is an opinion.
- 30–50 lines. A decision record is a decision, not an essay.
- The Decision section is imperative; the Context is evidence; the Consequences
  name the cost, not only the benefit.

## §3 — When to open a work item

Open a **WNNN** when a body of work spans more than one logical change or more
than one session, and needs a plan and gates a reader can follow. A single
commit does not need one — the commit message is the record.

A work item **references the decisions it implements**, and the tracker row
points at it.

## §4 — The work-item format

`docs/work/WNNN-<kebab-slug>.md`:

```markdown
# WNNN — <title>

- **Status:** in progress | done | parked
- **Decisions:** [DNNN](../decisions/DNNN-….md), …
- **Tracker:** [next-steps.md](../next-steps.md) item <n>

## 1. The ask
## 2. Scope / Non-scope        (two explicit lists)
## 3. Estimates                (value / complexity / risk, one row per workstream + a why)
## 4. Interface & structure    (what is added/changed, with file:anchor integration points)
## 5. Plan                     (numbered steps, each with its deliverable and its gate)
## 6. Quality gates            (exact commands and exact expected output)
## 7. Testing
### 7.1 Automated
### 7.2 How a human verifies it   (copy-pasteable commands + what to look for)
## 8. Expected outcome
## 9. Log                      (dated entries appended as the work progresses)
```

Rules:

- **§9 is append-only.** New entries at the bottom, dated, each saying what
  changed and what is now true.
- Every plan step has a **deliverable** and a **gate**.
- §6 names the exact commands; §7.2 is what a reviewer runs.
- `Status` moves forward (`in progress` → `done`); a parked item says why in the
  Log.

## §5 — The tracker

`docs/next-steps.md` is the index. Every work item/effort gets exactly one row:

```markdown
| <n> | <short title> (see [work/WNNN-….md](work/WNNN-….md)) | ◐ in progress — <one line> |
```

Legend in that file: `☑ done · ◐ in progress · ☐ todo`. Keep the row's status in
sync with the work item's `Status`; the row is a pointer, the work item is the
record.

## §6 — Checklist

- [ ] A **DNNN** exists for each locked decision a WNNN relies on.
- [ ] Each `file:anchor` in a decision record names a real fact.
- [ ] The WNNN `Decisions:` line links each applicable DNNN.
- [ ] The WNNN has all nine sections, with §2 two explicit lists.
- [ ] The tracker (`docs/next-steps.md`) has exactly one row pointing at the
      WNNN.
- [ ] The Log has a dated opening entry.

## References

`docs/decisions/` · `docs/work/` · `docs/next-steps.md` ·
`docs/decisions/D004-docs-taxonomy.md` (the bucket layout).

## Editing this file

Keep the frontmatter **quoted** and on one line. `description` ≤ 1024 chars,
clipped to 200 in the prompt. Validation: `crates/wcode-cli/src/skills.rs:efuUM`,
`:Vlnli`, `:ftQCJ`.
