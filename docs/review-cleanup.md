# Review-driven cleanup (item 31)

A whole-repo review (good / bad / ugly) surfaced six concrete, independent
defects; each was fixed as its own commit. Evidence and gates below. Reference
files by function, not line number (they drift).

## What was found

- **Two verified TUI bugs.** The status line pushed the model span twice, so the
  model printed twice; and `Surface` carried its own `model` field that `/model`
  never updated, so the `/surface` picker showed a stale model.
- **Dead code in the kernel.** `StopReason::Deferred` was declared and stringified
  but used nowhere; `cut_for_budget` was `pub` but only its own tests called it.
- **A silent failure.** Auto-compaction's `Err` arm was dropped — no event, no log.
- **Byte-identical duplication.** `grep`'s `include_path` and `find`'s
  `is_included` were identical; the whole-file-CAS stale-digest guard was
  copy-pasted across the mutating tools.
- **A god function.** `main()` was ~876 lines.

## Commits

| commit | area | change |
|--------|------|--------|
| `3b28eb5` | `tui:` | delete the duplicate status-model span; collapse `Surface.model` into `status.model`; two regression tests |
| `563ed4c` | `harness:` | remove dead `StopReason::Deferred`; un-publicize `cut_for_budget` |
| `61ebf66` | `harness:` | add non-fatal `AgentEvent::CompactionSkipped { reason }`; emit it from the loop's `Err` arm; Notice arm in the TUI (never sets `failed`); non-fatal line in the repl |
| `8fa0b03` | `cli:` | hoist one `include_path` into `tools/mod.rs`; grep/find call it |
| `50b2dad` | `cli:` | hoist `stale_digest_guard` into `tools/mod.rs`; all mutators call it |
| `edc1782` | `cli:` | split `main()` into parse_cli / load_config / print_system_prompt / list_models_and_exit / run_socket_client / load_session / build_runtime / build_agent_for / serve / dispatch (pure refactor) |

## Gates

- `cargo test --workspace` green; `cargo clippy --workspace --all-targets` clean.
- Zero `SKETCH` markers in every commit and in the tree.
- Live smoke check on the `main()` split: `--help` → exit 0, an unknown flag →
  exit 2, `--dump-system-prompt` → exit 0, a one-shot against a refused port →
  a clean error (exit 1), never a panic.
- New tests assert behavior, not merely pass: the status line shows the model
  exactly once; `/model` then `/surface` reflects the new model; a summarizer
  failure emits `CompactionSkipped` with **no** `Error` and the run still reaches
  `AgentEnd`; the TUI surfaces a Notice without marking the run failed.

## Deliberately not done

The review also flagged larger, non-blocking debt left for its own effort: the
`app.rs`/`ui.rs` monolith split, the three error idioms, the actor's duplicated
routing arms, and the unbounded channels. Those are refactors, not fixes, and
were out of scope here.
