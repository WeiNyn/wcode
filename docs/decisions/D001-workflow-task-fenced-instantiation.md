# D001 — `[workflow]` is kept but fenced to `--task`

- **Status:** accepted
- **Date:** 2026-10-05
- **Supersedes / relates to:** `docs/workflow-task-injection-plan.md`,
  `docs/project-team-loading.md`

## Context

`should_instantiate_workflow(resuming_group, has_workflow)` returns
`!resuming_group && has_workflow` (`crates/wcode-cli/src/main.rs:3Tsl0`; body
`uluIe`) and is called with `cfg.workflow.is_some()`
(`crates/wcode-cli/src/main.rs:GbCBX`). The predicate never reads `args.task`.
Two shapes therefore diverge:

- A `[workflow]` whose node titles carry **no** `{{task}}` template instantiates
  at boot whether or not a task is present. With no seeded task the scheduler
  drives an **unseeded** DAG — a silent start, not an error.
- A `[workflow]` whose node titles **use** `{{task}}` without `--task` /
  `WCODE_TASK` is a boot error: `check_task_args` returns
  `"[workflow] uses {{task}} but no --task/WCODE_TASK was given"`
  (`crates/wcode-cli/src/main.rs:IAMr0`) and the CLI exits 2.

So the DAG (the scheduler-driven, headless run) and the interactive team path
are mutually exclusive in practice, and one of the two failure modes is silent.

## Decision

A `[workflow]` instantiates **only** when `--task`/`WCODE_TASK` is present.
Otherwise the `[workflow]` table is **inert**: the CLI prints a one-line notice
and starts the interactive team from `[team]` / `.wcode/agents/` instead. No new
stage is ever added to the DAG for the brainstorm step — the interactive team is
driven by the orchestrator prompt, not the scheduler.

## Consequences

- The interactive team and the headless DAG stop competing for the same boot.
- `should_instantiate_workflow`'s predicate must additionally see the task, so
  the silent unseeded-DAG start disappears.
- Docs that present `[workflow]` as optional-for-interactive (e.g. the boot
  guards in `README.md`) must be corrected to the fenced reading.
