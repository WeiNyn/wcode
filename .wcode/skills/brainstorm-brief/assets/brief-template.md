# <title> — brief

- **Status:** draft | in review | approved
- **Work item:** WNNN (if one exists)
- **Decisions:** DNNN (if any are locked)
- **Author:** <role or session>
- **Date:** YYYY-MM-DD

## 1. The ask

<One paragraph: what was requested, in whose words, and what problem it solves.>

## 2. Scope / Non-scope

**In scope.**

- <what will change>
- <what will change>

**Non-scope.**

- <a thing a reasonable reader might assume is in scope> — <why it is not>
- <another> — <why it is not>

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | <name> | high | low | low | <one line> |
| P1 | <name> | | | | |

## 4. Interface & structure

**Added.**

- `<path>` — <what it holds>

**Changed.**

- `<path>::<function>` (`file:line` + a quoted snippet) — <what changes and why>

**Integration points** (verify before coding; a line number moves if the line
itself changes).

- `<path>:<anchor>` — <the fact this brief relies on>

## 5. Plan

1. **<step>.** Deliverable: <what exists>. Gate: <command / test / output>.
2. **<step>.** Deliverable: <what exists>. Gate: <command / test / output>.

## 6. Quality gates

- `cargo test --workspace` → all pass.
- `cargo clippy --workspace --all-targets` → clean.
- <live check command> → <expected output>.
- Regression tests kept green: `<test_name>`, `<test_name>`.

## 7. Testing

### 7.1 Automated

- `<test_name>` — asserts <the behaviour>.
- `<test_name>` — asserts <the behaviour>.

### 7.2 How a human verifies it

```sh
<copy-pasteable command>
```

Look for: <the exact line or state that proves it worked>.

## 8. Expected outcome

- <a checkable end state>
- <a checkable end state>

## 9. References

- `<path>` — <why it matters to this brief>
