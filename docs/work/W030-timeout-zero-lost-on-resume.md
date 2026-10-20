# W030 — an explicit `--timeout 0` is lost across a workflow resume

- **Status:** open (filed, not started)
- **Decisions:** relates to [D033](../decisions/D033-workflow-hardening.md) (the bound model W028 introduced and then amended)
- **Tracker:** [next-steps.md](../next-steps.md) item 90
- **Found by:** the HEAD-vs-v0.12.0 review ([review-head-vs-v0.12.0.md](../analysis/review-head-vs-v0.12.0.md), finding F-A)
- **Author:** session `agent:developer`
- **Date:** 2026-10-20

## 1. The ask

`--timeout 0` is documented as "NO cap, overriding it" (`main.rs:80`
`0 = NO cap, overriding it`). The launch honours that. The `resume:` line the
run prints does not, and a resume therefore re-imposes a cap the operator had
explicitly switched off:

1. **Launch.** `main.rs:1930-1932` `let timeout = crate::scheduler::workflow_bound_opt(args.timeout.or(cfg.workflow…timeout_secs));`
   runs the flag through `scheduler.rs:53-54` `secs.filter(|&s| s > 0).map(Duration::from_secs)`,
   so `Some(0)` collapses to `None`.
2. **Record.** `main.rs:2589` `let cap = timeout.map(|d| d.as_secs());` stores
   `None` at `main.rs:2598` `timeout_secs: cap,` — the record cannot tell "the
   operator said no cap" from "no flag and no config".
3. **Resume line.** `repl.rs:675` `if let Some(secs) = timeout_secs.filter(|&s| s > 0) {`
   omits the pair for `None`, so the line carries no `--timeout`.
4. **Resume.** Nothing reads `run.json` back — `load_run` (`session_groups.rs:491`)
   is test-only — so the resumed process re-derives the cap from
   `args.timeout.or(cfg.workflow…)` (`main.rs:1930-1932`) and lands on
   `[workflow] timeout_secs`. In this repo that is
   `.wcode/workflow.toml:155` `timeout_secs = 21600` — six hours, on a run whose
   launch said "unbounded".

Nothing warns. The cap silently fires hours later, on the recovery of a run the
operator had already had to recover once.

`repl.rs:661-663` states the current intent — `None` "and `Some(0)`, a sub-second
cap that rounds to zero — mean 'no cap', so the flag is OMITTED rather than
resurrected as a 0-second bound" — which is right about the literal flag and
wrong about the world it lands in: an omitted flag is not "no cap", it is "the
config default".

## 2. Scope / Non-scope

**In scope.**
- The launch record keeps the operator's **raw** `--timeout` choice, `0` included, so an explicit "no cap" is distinguishable from "no flag".
- The `resume:` line re-emits it, so a resume reproduces the launch's cap decision.
- Regression tests on the record, the renderer, and their agreement.
- The three doc sites that describe the omission (`repl.rs:661-663`, `main.rs:80`, `README.md:223-225`).

**Non-scope.**
- Per-node / per-edge bounds (`workflow_bound_opt`'s other four callers) and their own `0` handling — untouched.
- The `[workflow] timeout_secs` default itself, and its precedence over an absent flag — that precedence is the design (D033).
- `--quiet`/`--status` combination guards (the separate nit in the review, finding F-C).
- Making `--status` or any other reader consume `run.json` (option iii below), and any "resume as launched" contract for fields other than the cap.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P1 | `RunRecord.timeout_flag` + its write site | high | low | low | one optional field with `#[serde(default)]`; old records load as `None`, i.e. today's behaviour |
| P2 | `workflow_resume_line` emits the raw flag | high | low | med | the renderer plus its two call sites (`print_workflow_resume` at `main.rs:2626`/`:2639`) and three existing renderer tests (`repl.rs:1819`/`:1840`/`:1846`) |
| P3 | Docs + the test that pins the pair | med | low | low | three prose sites; one test asserting `--timeout 0` survives end to end |

## 4. Interface & structure

### 4.1 Recommended: carry the raw flag (option i)

`session_groups.rs:472-481` — `RunRecord` gains one field, next to
`pub timeout_secs: Option<u64>` (`:480`):

```rust
/// The RAW `--timeout` token, `0` included — the flag's own value, NOT the
/// effective cap (`timeout_secs`), so an explicit "no cap" survives a resume.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub timeout_flag: Option<u64>,
```

- `run_workflow` gains the raw token as a parameter — supplied at its call site
  (`main.rs:1942-1952`, from `args.timeout`, beside `args.config`/`args.quiet`)
  — and the record sets it at `main.rs:2594-2598`, beside the existing
  `timeout_secs: cap`.
- `repl.rs:664` `workflow_resume_line(group, task, config, timeout_secs)` gains a
  `timeout_flag: Option<u64>` parameter — placed immediately before
  `timeout_secs` — and pushes `--timeout <n>` for `Some(n)`, `0` included,
  instead of filtering on `s > 0`. `timeout_secs` keeps its present job (the
  summary and the record's effective cap).
- Backward compatibility is automatic: an old `run.json` deserializes
  `timeout_flag: None`, which renders exactly what it renders today.

**Why not overload `timeout_secs`:** `Some(0)` there already means a sub-second
cap rounded down (`main.rs:3207` `timeout_secs: Some(0), // 150 ms rounds down to 0 s`;
`repl.rs:1844` says the same), so `Some(0)` cannot also mean "explicitly
unbounded".

### 4.2 The two alternatives, and why they lose

- **(ii) A tri-state `Cap::{Unset, Disabled, Secs(u64)}`** recorded instead of
  `timeout_secs`. Carries the same information, but breaks the `run.json` shape
  for an existing reader (`load_run`, and the `a_cancelled_run_persists_run_json`
  equality assertion at `main.rs:3201-3211`) to gain nothing the raw field does
  not. Rejected.
- **(iii) Read `run.json` on resume and treat the recorded cap as binding.** The
  only option that makes a resume ignore the *current* config — fuller, and the
  contract W028 wrote down ("the launch is recorded … so a resume does not
  resume unbounded", `main.rs:2561-2565`). It changes the meaning of a resume for
  every field and needs a rule for records predating the flag. Rejected **for
  this defect**; if "resume exactly as launched" ever becomes the contract it
  deserves its own decision record, and (i) is still its first step.

## 5. Plan

1. **Add `RunRecord.timeout_flag`** (`session_groups.rs:472-481`), thread it
   through `run_workflow` (`main.rs:1942-1952`) and set it at
   `main.rs:2594-2598`. *Deliverable:* the record round-trips `Some(0)`.
   *Gate:* `run_json_round_trips` (`session_groups.rs:511`) extended with a
   `timeout_flag` leg; `cargo test -p wcode-cli`.
2. **Render it** (`repl.rs:664-684`): the parameter, the push, and the doc
   comment above it rewritten to say "the raw flag, so an explicit `0` survives
   a resume whose config sets a cap". *Deliverable:* `--timeout 0` in the line.
   *Gate:* the three renderer tests at `repl.rs:1819`/`:1840`/`:1846` updated;
   a new `resume_line_carries_an_explicit_zero`.
3. **Pin the agreement** — a test that builds a record with
   `{timeout_flag: Some(0), timeout_secs: None}` and asserts the line carries
   `--timeout 0` while the summary cap stays `None`; plus one asserting an
   *absent* flag still omits the pair (so the config default keeps applying when
   the operator never chose). *Deliverable:* both directions pinned.
   *Gate:* the new tests fail if the filter returns (`s > 0`).
4. **Docs** — `main.rs:80`'s help line, `README.md:223-225`, and the
   `repl.rs:661-663` contract comment state the new rule ("`0` = no cap, and the
   resume line carries it"). *Deliverable:* no site says an omitted flag means
   "no cap". *Gate:* `grep -rn "0/absent" README.md crates/` is empty.

## 6. Quality gates

```
cargo test --workspace          # all pass; the 4 ignored stay 4
cargo clippy --workspace --all-targets   # no warnings
git diff --check
```
Plus the live check in §7.2 — the defect is a printed string, so the printed
string is what must be read.

## 7. Testing

### 7.1 Automated

- `run_json_round_trips` — the launch record with `timeout_flag: Some(0)`.
- `resume_line_carries_an_explicit_zero` — `workflow_resume_line(_, _, Some("/tmp/wf.toml"), Some(0) /* flag */, None /* cap */)` yields `--agents --config /tmp/wf.toml --timeout 0 --resume /g --task t`.
- `resume_line_omits_an_absent_flag` — `timeout_flag: None` still yields no pair (the config default must keep working).
- `a_cancelled_run_persists_run_json` (`main.rs:3170`) — its `RunRecord` equality gains the new field.

### 7.2 How a human verifies it

Against a config whose `[workflow] timeout_secs` is set (this repo's
`.wcode/workflow.toml:155` is 21600), launch with the cap explicitly off and a
refused endpoint so the run ends promptly:

```
WCODE_RETRY_MAX=2 wcode --agents --base-url http://127.0.0.1:9/v1 \
  --task "no-op" --timeout 0
```

- **Today:** the `resume:` line printed by the non-zero exit carries `--agents … --resume <groupdir> --task …` and **no** `--timeout`; re-running it imposes the 6 h cap.
- **After the fix:** the same line carries `--timeout 0`, and the resumed run stays unbounded.

The run must NOT be resumed in a way that silently changes its cap; that is the
whole assertion.

## 8. Expected outcome

An explicit `--timeout 0` means unbounded at launch **and** on every resume the
run itself prints. The record distinguishes "the operator disabled the cap" from
"the operator never set one", and the resume line re-emits the former. Old
`run.json` files (and old resume lines) keep their present behaviour.

## 9. Log

- **2026-10-20** — filed from the HEAD-vs-v0.12.0 review (F-A), not fixed there:
  the defect is in the shipped W028 surface and wants its own commit, its own
  test, and a decision on (i) vs (iii). No code changed by the filing.
