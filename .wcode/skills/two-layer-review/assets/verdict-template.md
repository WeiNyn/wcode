# Review verdict — <sketch | diff>

- **Layer:** 1 (sketch) | 2 (diff)
- **Artefact:** `<path>` (sketch) or `<commit-range>` (diff)
- **Task:** <the dispatched task, one line>
- **Reviewer:** <role or session>
- **Date:** YYYY-MM-DD
- **Verdict:** APPROVE | BLOCKING

## Summary

<One paragraph: what was reviewed and the headline verdict.>

## Blocking issues

<None, or a numbered list. Each cites file:line + a quoted snippet.>

1. **<issue>** — `<path>:<anchor>`
   - What is wrong: <the fact, not a feeling>
   - To clear it: <the concrete change>

## Non-blocking notes

<Findings that do not gate the change. Each with file:line + a snippet where it is a
claim about the code.>

- `<path>:<anchor>` — <note>

## Layer 1 — amendments to honor

<The settled open questions, as concrete instructions for the developer.
Omit for a layer-2 verdict (or write "n/a — amendments confirmed landed").>

1. <instruction>

## Layer 2 — checks

- [ ] Diff does exactly the task; nothing unrequested.
- [ ] Every new test would fail if the logic were wrong.
- [ ] Every layer-1 amendment landed.
- [ ] `grep -rn 'SKETCH' <changed-paths>` is empty.
- [ ] `cargo test --workspace` — <result>.
- [ ] `cargo clippy --workspace --all-targets` — <result>.
- [ ] Live check: <command> — <observed output>.

## Evidence

- `<command>` → <observed output>
- `<path>:<anchor>` — <the fact it establishes>
