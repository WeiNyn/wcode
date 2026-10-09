# D006 — editorial (W005) supersedes four shipped v3 decisions

- **Status:** accepted
- **Date:** 2026-10-09
- **Supersedes / relates to:** `docs/vscode-ui-v3-plan.md` — decisions 3 & 5, V12,
  V13r, V14, and the composer `.seg` of §1.4/V9; it also **drops** the V11b gauge and
  the six `charts-*` member swatches. Nothing else in the locked plans is touched, and
  the wire / kernel contract is untouched.

## Context

The human reviewed six directions (`docs/design/vscode-modern-directions.md`) and chose
**A · editorial**; its prototype `docs/design/vscode-modern-editorial-draft.html` is the
new visual spec. v3's plan is the shipped spec (`docs/vscode-ui-v3-plan.md:CRpGr`), and
its own convention is to record an amendment explicitly (`:gBoLf`); `design-taste` §7.3
(`:cgHk2`) forbids changing a locked invariant silently. The redesign is therefore
authorised only by first recording the supersession — W005 E0, docs-only, before any code.

## Decision

In `docs/vscode-ui-v3-plan.md`, **editorial supersedes**:

- **decision 3** (`:8HCPY`, the team's one home + two echoes) — the rail and the working
  pill both go; the team becomes a dim caption, the aggregate a live type-line;
- **decision 5** (`:Jkqsl`, L1 / L2a / L2b) — **narrowed**: the code block and the tool
  output become fill-only recesses, so the change-review card is the only L2a object left;
- **V12** (`:zTBVB`) — the tool output card, its `.chead` and its `Copy` go;
- **V13r** (`:cXMdP`) — a complete tool is neutral, not green;
- **V14** (`:Sc8DY`) — no spine, no head hairline, no `you` fill and no live accent wash;
- **the composer `.seg`** of §1.4 / V9 (`:ywwQg`) — a quiet text toggle instead.

It **drops** the **V11b** context-meter gauge (`:Sw11b`) and the six `charts-*` member
swatches (`:WJJlL`). It **keeps** decision 2 (`:M3ETf`, the mode stays, restyled) and
decision 12 (`:CSVRU`, the text `ctx 42k` default).

Editorial also **narrows V13** (`:nYqsK`, the tool-status colour): the tool row's L1 rule
is dropped — the A prototype draws the tool as a dim sentence
(`docs/design/vscode-modern-editorial-draft.html:yQw0I`) — so the status lives on the `⚙`
mark alone (neutral complete / accent running / red error). V14's removed live accent wash
goes with the rule.

## Consequences

- The **reason**: A is a reading surface — the words are the interface. Every widget v3
  added to dress the prose (a rail, a spine, a hairline head, a `you` fill, a working
  pill, a gauge, a code/output card) competes with the words and is removed; identity
  survives as a caption and air as the composition.
- The shipped spec stays readable: each affected item carries an "Amended by editorial
  (W005)" note rather than being rewritten; decisions 1, 4, 6–11 and V6–V11a stand.
- The cost: the team loses its persistent home and the one-click retarget; the aggregate
  loses its jump affordance; the code/output loses its card border and `Copy`; the
  context meter loses its gauge and the surface its swatches. These are accepted losses.
- **What this forecloses:** re-adding the rail, the working pill, the gauge or the turn
  spine without a new decision that supersedes this one.
- Implementation follows in `docs/plans/vscode-ui-editorial-plan.md` §1 (E1–E7);
  nothing under `docs/` moves (D004 item 58).
