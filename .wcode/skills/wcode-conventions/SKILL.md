---
name: wcode-conventions
description: "Use when writing or reviewing any change in the wcode repository — commit shape, the required cargo test/clippy gates, the no-anyhow rule, the kernel/CLI split, content-addressed anchors, and the parallel/barrier tool rules. Not for a downstream repo that vendored wcode, and not for the harness's internal event protocol."
---

# wcode conventions

The rules this repository enforces on every change. They are not preferences;
`AGENTS.md` is the source and a review that misses one is a blocking issue.

## §1 — Commits

- **One logical change per commit.** Do not fold two unrelated edits into one
  commit, and do not split one logical change across commits.
- **Subject: imperative, area-prefixed.** `harness:`, `cli:`, `tui:`, `docs:`
  (extend the set consistently — `protocol:` where it applies).
  - Good: `cli: fence [workflow] to --task`
  - Bad: `fixed stuff`, `updates`, `WIP`
- **Track multi-step work** in `docs/next-steps.md` (the tracker). Larger efforts
  get their own doc under `docs/`; the tracker keeps a pointer + status.

## §2 — Every change: tests + clippy clean

```sh
cargo test --workspace                # all tests pass
cargo clippy --workspace --all-targets # clean, no warnings
```

`cargo build` proves nothing about behaviour. Prefer a **live end-to-end
check** against a real or local endpoint — see the `live-verification` skill.

## §3 — No `anyhow`

The workspace does not use `anyhow`. Errors are concrete typed errors
(`thiserror`-style enums, `Result<T, E>` with named variants) or `String` where
the crate already does so. A `?` on a foreign error must name the conversion.

## §4 — The kernel/CLI split

- **`crates/wcode-harness` is the kernel.** It owns the loop, tool dispatch,
  hooks, events, sessions, compaction. It must stay **free of presentation
  concerns** — no TUI/CLI strings, no terminal assumptions.
- **The CLI (`crates/wcode-cli`) is a thin client.** Arg parsing, config,
  concrete tools, the REPL/TUI wiring. Behaviour that is a policy belongs in a
  `Hooks` impl, not in config.
- **Absent by design** (do not add): MCP, subagents, permission prompts,
  approval flows, config files for behavior. Build the variant you want via
  `Hooks` instead of configuring one.
- The TUI (`crates/wcode-tui`) is a `wcode-protocol` client; the protocol crate
  is the transport seam (`Backend`, NDJSON `Frame`s).

Where a policy must read CLI state (e.g. the task list), it lives in the CLI as
a `Hooks` impl — the `VerifyGateHooks` pattern
(`crates/wcode-cli/src/verify_gate.rs:N10Kg`). The kernel must not depend on it.

## §5 — Editing is by literal text

`read` prints `<line number>\t<content>`; `edit` replaces an exact `old_string`
with `new_string`. (D007 — the hash-anchor scheme is gone.)

- Matching is **byte-exact**: whitespace counts, and a common string (a bare
  `}`) needs surrounding context or `replace_all`.
- `old_string` must occur **exactly once** unless `replace_all` — a miss is
  `E_NO_MATCH`, an ambiguous match `E_AMBIGUOUS_MATCH`. Nothing is written on
  either.
- Because the match *is* the check, there is **no separate staleness guard**:
  text that changed simply no longer matches.
- `edits` applies a batch of the same ops to ONE file, **in order** against the
  accumulating content, all-or-nothing.
- `edit`/`edits`/`write` share a mutation lock and write via a temp file +
  rename; concurrent mutation cannot interleave or truncate.
- `read` truncates a >300-char line with `…(+N)` — such a line is **not** usable
  as an `old_string`; use `grep` for the exact bytes.

See `README.md` §"Design: editing by literal text".

## §6 — Tools: read-only vs barrier

- Independent tool calls in one batch run **concurrently**. Read-only tools opt
  in via `parallel_safe() -> true`: `read`, `grep`, `find`, `ast_search`,
  `webfetch`.
- **`bash` is a barrier and never parallel-safe.** It never runs concurrently
  with another call in the same batch. Mutating tools (`edit`, `write`,
  `replace`, …) are barriers too.
- A new read-only tool **should override `parallel_safe() -> true`**; anything
  that mutates state stays a barrier.
- `--sequential` (or `[tools] parallel = false`) restores strictly
  one-at-a-time execution.

## §7 — Extending (the three seams)

- **Tools** — implement `TypedTool` (typed args + a `schemars` schema), wrap with
  `erased()`, add to `default_tools()`. `grep`/`find` are registered only when
  enabled (`[tools] grep/find = true`); `ast_search`/`ast_edit` only when an
  `ast-grep`/`sg` binary is on `PATH`.
- **Policies** — implement `Hooks`; a reason from `before_tool_call` **blocks**
  the call and is fed back to the model as an error.
- **Providers** — implement `StreamFn`; `rig_stream_fn()` is the built-in
  OpenAI-compatible adapter.

## §8 — Checklist

- [ ] One logical change; imperative, area-prefixed subject.
- [ ] `cargo test --workspace` passes.
- [ ] `cargo clippy --workspace --all-targets` clean.
- [ ] No `anyhow`.
- [ ] Kernel free of presentation concerns; CLI still thin.
- [ ] A policy (not config) where behaviour is being added.
- [ ] Anchors re-read after any formatter; no ambiguous/stale edits.
- [ ] New read-only tool overrides `parallel_safe()`; `bash` stays a barrier.
- [ ] Multi-step work tracked in `docs/next-steps.md`.

## References

`AGENTS.md` · `README.md` · the `live-verification` and `traceability-log`
skills.

## Editing this file

Keep the frontmatter **quoted** and on one line. `description` ≤ 1024 chars,
clipped to 200 in the prompt — the trigger must sit in the first 200. Validation:
`crates/wcode-cli/src/skills.rs:efuUM`, `:Vlnli`, `:ftQCJ`.
