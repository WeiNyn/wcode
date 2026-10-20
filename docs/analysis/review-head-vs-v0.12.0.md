# Review — the current code against the latest release (v0.12.0)

- **Status:** done
- **Date:** 2026-10-20
- **Reviewed:** `1feab76` (`docs: sweep the last unbounded-bound references`) — local `main`, 15 commits past the tag
- **Baseline:** tag `v0.12.0` = `211da89` (`chore: release 0.12.0`), published 2026-10-19
- **Upstream note:** `origin/main` is `381d2b6` (`chore(homebrew): wcode v0.12.0 [skip ci]`), one commit **ahead** of the reviewed tree. §4.2 turns on that commit existing.
- **Verdict:** the delta is **releasable**. No kernel or protocol change, no build/lint/test debt, no `anyhow`. One behavioural defect in the new CLI surface (F-A → **filed as W030**, deliberately not fixed here) and one pair of stale sentences (F-B → **fixed on this branch**). Two of the three release blockers that a first pass reported survive as release-time actions (§4.1, §4.3); the third (§4.2) is corrected in this record.
- **Filed:** [W030](../work/W030-timeout-zero-lost-on-resume.md) — **Fixed:** F-B — **Corrected:** B2

## 1. What the delta is

`git diff --shortstat v0.12.0..1feab76` → `25 files changed, 3390 insertions(+), 112 deletions(-)`, in three groups:

- **W029 — queue a prompt typed while a turn runs** (`bcb1297`, follow-up fold `f3ddef9`). The kernel/protocol seam is untouched: the actor already defers a second `Submit`. The one new contract is a *targeted* action — `crates/wcode-tui/src/lib.rs:590` `Action::SubmitTo(target, text) => {`, and the comment at `:591-593` is the whole reason it exists: "Resolve by id, like `AskTo` below — never the focused backend: `AgentEnd` for a member can fire while the user is looking at the root (W029)".
- **W028 — workflow hardening** (phases: 1 `d251c40` + review `5c8fbd4`, 2a `7b86f80` + review `dd5fe8b`, 2b `b32deee`, the carried cap `d791944`, template `55a97dc`, the bound reversal `47dbad0`, docs `535fde2`/`c7c4b9d`/`1feab76`). A headless `--task` run becomes observable (`--status`, `--quiet`, the stderr event log), cancellable/resumable (the `resume:` line + `run.json`), and bounded by explicit knobs only.
- **Docs/design** (`5984c4e`, `a5a21f6`, `c7c4b9d`).

Where it lands: `crates/wcode-cli` and `crates/wcode-tui` only.
`git diff --stat v0.12.0..1feab76 -- crates/wcode-harness crates/wcode-protocol` is **empty** — no wire frame, no kernel contract, and no presentation logic in the kernel moved. That is the shape the repo asks for, and it is why the review's risk sits in the CLI surface rather than in the loop.

Hygiene, all re-run at this tree:

| gate | result |
|---|---|
| `cargo test --workspace` | **1385 passed, 0 failed, 4 ignored** |
| `cargo clippy --workspace --all-targets` | clean |
| `git diff --check v0.12.0..HEAD` | exit 0 |
| `grep -rn anyhow --include=*.rs crates/ Cargo.toml` | 0 hits |

## 2. Verdict

Releasable as code. The delta is additive in the CLI, the kernel is untouched, and the one behaviour that changed underneath the old docs (an absent bound used to mean 300 s) is now the single thing a user must be *told* — §4.3. The one defect (F-A) is a recovery-path bug: it misbehaves only on a resume, and only when the operator had explicitly disabled the cap. It is real, it is silent, and fixing it here would have mixed a behaviour change into a review-and-docs commit, so it is filed with its mechanism, three fix shapes, and a live-check recipe instead.

## 3. Findings

### 3.1 F-A — an explicit `--timeout 0` is lost across a resume (filed: W030)

`--timeout 0` means "no cap" (`main.rs:80` `0 = NO cap, overriding it`). The launch honours it; the `resume:` line does not; the resume re-imposes `[workflow] timeout_secs` (`.wcode/workflow.toml:155` `timeout_secs = 21600`).

- `scheduler.rs:53-54` `pub(crate) fn workflow_bound_opt(secs: Option<u64>) -> Option<Duration> { secs.filter(|&s| s > 0)…` — `Some(0)` → `None`.
- `main.rs:2589` `let cap = timeout.map(|d| d.as_secs());` → the record stores `None` at `main.rs:2598` `timeout_secs: cap,`, so "no cap" and "no flag" are the same value.
- `repl.rs:675` `if let Some(secs) = timeout_secs.filter(|&s| s > 0) {` → no `--timeout` in the line.
- Nothing reads `run.json` back (`load_run`, `session_groups.rs:491`, is test-only), so the resumed process re-derives the cap from args+config and finds 6 h.

This is a defect of the **new** surface (W028 shipped in this delta), not a pre-existing one, and it fails silently hours later. [W030](../work/W030-timeout-zero-lost-on-resume.md) records the mechanism, the three fix shapes, and why the recommended one carries the **raw** flag instead of overloading `timeout_secs` (whose `Some(0)` already means a sub-second cap: `main.rs:3207`).

### 3.2 F-B — two sentences written for the flag-only world (fixed on this branch)

Both described a world where the flag was the only source of a value, so "absent" and "no cap"/"always 0" were the same thing. One of them was invalidated by W028's config default; the other was never true.

- `README.md:223-225` said the `--timeout` cap applies "(`0`/absent = no cap)". An absent flag is **not** no cap — it is `[workflow] timeout_secs`, and unbounded only when that is unset too.
- `main.rs:83-86` (help) and `main.rs:147-148` (the `Args` doc) said `--status` exits 0. It exits 1 whenever there is nothing to print: no sessions (`main.rs:2818`), no such group (`:2832`), no plan in the group (`:2851`) — and `status_latest_picks_the_newest_group` (`:3228`) pins the asymmetry (a plan-less newest group exits 1 while an older one exits 0).

Live proof of the fixed claim, against an empty `HOME`:

```
$ HOME=/tmp/w030home wcode --status
error: no sessions in /tmp/w030home/.local/share/wcode/sessions
$ echo $?
1
```

### 3.3 F-C — `--quiet` without `--task` is accepted and does nothing (observed, not filed)

`main.rs:253` `"--quiet" => a.quiet = true,` has no counterpart in `check_task_args`, while `--timeout` has one (`main.rs:1409-1410` `if args.timeout.is_some() && args.task.is_none() { return Err("--timeout requires --task".into()); }`). So `--quiet` rides along in an interactive run and is ignored. Recorded here, not filed: an ignored flag produces no wrong answer, and adding a refusal would break invocations that are harmless today — a style question, not a defect.

### 3.4 F1 (W025's AND-fold) — still unreachable from the shipped template (already tracked)

The edge AND-fold stops at the first `judge` edge (`scheduler.rs:467` `for dep in &deps {` … `:544` `return EdgeGate::Pending;`, with `:428` `EdgeGate::Pending => {}`), so a later conditional edge on the same target is not re-evaluated. It needs a target with ≥2 incoming conditional edges to matter; the shipped `.wcode/workflow.toml` has **7** `[[workflow.node]]` and **8** `[[workflow.edge]]` blocks and `grep -c '^condition' .wcode/workflow.toml` → **0**, and no `.wcode/teams/*.toml` / `.wcode/team.toml` carries a `[workflow]`. Tracker row 83 already carries this as an open follow-up with both fix options — nothing new to file, nothing reachable in this configuration.

## 4. Release readiness

### 4.1 B1 — the workspace version still equals the shipped tag (**open**; release-time)

`Cargo.toml:6` `version = "0.12.0"` and the three internal pins `Cargo.toml:30-32` are all `0.12.0` — the value crates.io and the tag already hold. `.github/workflows/publish.yml:52` `::error title=tag-version-mismatch::tag '${tag}' != 'v${version}' — bump workspace.package.version` therefore fires on the next tag until a `chore: release X.Y.Z` commit bumps **all four**.

Not done in this review: the number is a judgement about the delta. The bound reversal (§4.3) is a user-visible default change, which argues `0.13.0`, not `0.12.1` — and the release commit is the maintainer's, not a docs commit's.

### 4.2 B2 — the formula is **not** a release behind (**corrected**)

A first pass read the local tree and concluded "`Formula/wcode.rb` says 0.11.0 and v0.12.0 has no formula commit". Both halves are true of **this clone** and false of the repository:

- Local: `Formula/wcode.rb:1` `# This file was generated by scripts/gen-homebrew-formula.sh for v0.11.0 — do not edit.` and `:5` `version "0.11.0"`.
- `git fetch origin` → `211da89..381d2b6  main -> origin/main`; `git log -1 origin/main` → `381d2b6 chore(homebrew): wcode v0.12.0 [skip ci]`. The local branch was 15 ahead **and 1 behind**; the commit was always there.
- Upstream content of `Formula/wcode.rb` at `ref=main` (GitHub contents API): blob `20916e4c…`, `version "0.12.0"`, carrying the four v0.12.0 sha256s.
- The tag's release workflow concluded `success` (run `39252704020`), and the release lists all eight assets — four `wcode-v0.12.0-<target>.tar.gz` + four `.sha256`, every one `state: uploaded`.

So the formula is correct at `v0.12.0` and needs **no** action: not a regeneration (its header forbids edits and the generator would only re-derive what is already there), not a hand edit. The real hazard is operational — **pull (or rebase) before pushing, never `push --force`**: a force-push of the 15 local commits would revert `381d2b6` and put `brew` back on 0.11.0.

### 4.3 B3 — the retired implicit bound needs a hand-written release note (**open**; release-time)

`gh release create … --generate-notes` (`.github/workflows/release.yml`) synthesizes from commit subjects. It cannot infer this, so paste it by hand:

> **Workflow bounds are now opt-in.** In v0.12.0 every workflow `script` — a `Script` node's command and an edge's `condition.script` — was cut off at a hard-coded 300 s (`crates/wcode-cli/src/scheduler.rs:39` in that tag), with no way to raise it. In this release there is **no implicit bound**: a node or edge is bounded only by its own `timeout_secs`, and the guard that matters is the whole-run cap set at launch (`--timeout <secs>`, else `[workflow] timeout_secs`). If you relied on the 300 s cutoff as a backstop against a wedged command, set `timeout_secs` on those nodes/edges before upgrading — otherwise such a command now waits indefinitely.

Why it will not be inferred: the change is a *removal* of a `const` plus a reversal of a default, spread across `7b86f80` (bounds, `cli:`) and `47dbad0` (`cli: an unset workflow bound means unbounded`). Neither subject names the shipped behaviour that stopped happening.

## 5. What this review did not verify

- No live model turn: there is no endpoint on this machine and the delta's workflow paths need a keyed one. The workflow behaviour claims above rest on the code, the tests, and W028's own recorded live run — not on a run I performed.
- No `brew` install of the formula; the v0.12.0 formula claim is the GitHub blob + the successful release run, not a tap install.
- `cargo test --workspace` (1385 passed / 0 failed / 4 ignored) and `cargo clippy --workspace --all-targets` (no warnings) were run at this tree and gate this branch; they prove the delta compiles and its tests pass, not that a workflow behaves — that needs an endpoint (previous bullet).
- F-A's fix is **not** implemented here (by design: one logical change per commit, and the file mix). Its live check is written down in W030 §7.2 and has not been executed.
