# wcode — repository guidelines

wcode is a minimal, pi-like coding agent for the terminal: a small kernel
(`wcode-harness`) that runs an LLM tool-loop against any OpenAI-compatible
chat-completions endpoint, plus a thin CLI (`wcode-cli`, binary `wcode`).
Presentation code streams and styles `AgentEvent`s; the kernel owns the loop.

## Layout

`crates/wcode-harness` — the kernel (no CLI/presentation concerns):

- `agent.rs` — `Agent`: `run`, `cancel`/`cancel_token`, `messages`, `compact`,
  `steer`/`follow_up`, `set_model`, `set_effort`, `session_path`.
- `loop_.rs` — the tool-calling loop (`run_loop`, `LoopConfig`, `RunResult`)
  that emits `AgentEvent`s.
- `streamfn.rs` — `StreamFn` trait + the `rig` OpenAI-compatible adapter
  (`rig_stream_fn`) with retry/backoff.
- `tool.rs` — `Tool` / `TypedTool` + `erased()`.
- `hooks.rs` — `Hooks`: `transform_tool_input`, `before_tool_call`,
  `after_tool_call`, `transform_context`, `should_stop_after_turn`.
- `event.rs` — `AgentEvent` / `LlmStreamEvent` (the UI seam).
- `compaction.rs`, `session.rs`, `message.rs`, `limits.rs`.

`crates/wcode-protocol` — transport above the kernel: the `Backend` seam
(`Local`/`Remote`), NDJSON `Frame`s, a socket `serve`/`Client`, so a client speaks
`Request` → `AgentEvent` the same way locally or across a socket.

`crates/wcode-tui` — the full-screen TUI (`ratatui`/`crossterm`), a
`wcode-protocol` client: `run(surfaces, options, new_surfaces)`, a pure reducer
over `AgentEvent`, three bands (transcript · input · status), markdown +
`/`-commands, a team sidebar, overlays (pickers + `F1` help), a named-role
palette (`theme.rs`), tool-output expand/collapse, and a transcript browse mode.

`crates/wcode-cli` — the composition root:

- `main.rs` — arg parsing, config load, agent construction; picks one-shot, TUI
  (TTY, `--tui`/`--no-tui`), or the line REPL.
- `repl.rs` — REPL loop, `/`-commands, event printing, system prompt, the
  project-instruction file.
- `config.rs` — `config.toml` + env resolution.
- `tools/` — concrete tools: read, grep, find, bash, ast_search, ast_edit,
  edit, edits, replace, write.
- `rtk.rs` — optional rtk hook (bash-output proxy).

## Commands

- Build: `cargo build` (or `cargo build --bin wcode`)
- Test: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets` — must be clean
- Run: `cargo run -p wcode-cli -- -p "..."`, or `wcode` once installed
- Install: `cargo install --path crates/wcode-cli`

Rust edition 2024, resolver 3.

## Conventions

- **One logical change per commit.** Imperative subject, area prefix
  (`harness:`, `cli:`, `docs:`).
- **Every change: tests + clippy clean.** Prefer a **live end-to-end check**
  (run against a real or local endpoint) over unit tests alone — `cargo build`
  proves nothing about behavior.
- **Track multi-step work in `docs/next-steps.md`.** Larger efforts get their
  own doc (e.g. `docs/tui-plan.md`); the tracker keeps a pointer + status.
- **Minimalism is the point.** Absent by design: MCP, subagents, permission
  prompts, approval flows, config files for behavior. That logic belongs in
  code, via `Hooks` — build the variant you want instead of configuring one.
- Keep the kernel free of presentation concerns; the CLI is a thin client.

## Gotchas

- **Content-addressed anchors.** `read` prints `ANCHOR│line`; `edit` targets
  `from`/`to` anchors. An anchor is a hash of the line's raw content, so
  indentation is part of the address and a reformatter that reindents *moves*
  anchors — re-read after formatting. Identical lines share an anchor; `edit`
  rejects ambiguous targets unless `old_string` pins one. Edits never shift
  lines above them.
- **Instruction ("reference") files** are discovered as a *set* — a global file
  in `~/.config/wcode/` plus the ancestor chain (repo root → working dir,
  nearest last), candidates `AGENTS.override.md`/`AGENTS.md`/`CLAUDE.md` per
  dir — and folded into the **system prompt at startup** (32 KiB per file, 64
  KiB total). `--no-instructions` / `"off"` / `WCODE_INSTRUCTIONS` control it;
  `--dump-system-prompt` prints the composed prompt and exits.
- **Skills** (`SKILL.md`) are discovered project-first (`.wcode/skills`, then the
  shared `.agents/skills`, falling back to `.claude/skills`) and globally; only
  each skill's `name` + `description` enter the prompt — the body loads on demand
  via `read`. `--no-skills` / `[skills]` control it.
- **Verify against a real binary.** For local checks point at a keyless
  endpoint (`--base-url http://localhost:11434/v1`); for error/retry paths a
  refused port (`http://127.0.0.1:9/v1`) with `WCODE_RETRY_MAX=2`.

## Extending

- **Tools** — implement `TypedTool` (typed args + schemars schema), wrap with
  `erased()`, add to `default_tools()`. Read-only tools should also override
  `parallel_safe() -> true`; everything else (including `bash`) stays a barrier
  and never runs concurrently with another call in the same batch.
- **Policies** — implement `Hooks`; a reason from `before_tool_call` blocks the
  call and is fed back to the model as an error.
- **Providers** — implement `StreamFn`; `rig_stream_fn()` is the built-in
  OpenAI-compatible adapter.

## Further reading

- `README.md` — config, commands, tools, and the content-addressed editing design.
- `docs/next-steps.md` — current work tracker.
- `docs/tui-design.md` — TUI visual spec (bands, glyphs, the palette).
- `docs/tui-plan.md` — TUI phases/architecture (P0–P3 shipped).
- `docs/tui-polish-plan.md` — TUI polish: tool output, theme, keys.
- `docs/tui-browse-plan.md` — TUI transcript browse mode.
