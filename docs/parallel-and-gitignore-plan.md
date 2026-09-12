# wcode — parallel tool execution & .gitignore awareness

Status: **both phases shipped**. Companion to
[`next-steps.md`](next-steps.md) (item 7).

Two independent harness gaps identified by comparing against pi (`../wi/pi`) and
jcode (`../jcode`): the loop runs tool calls **serially**, and `grep`/`find`
ignore files by a **hardcoded** list rather than the project's `.gitignore`.

| phase | scope | status |
|-------|-------|--------|
| 1 | Parallel tool execution (kernel) | ☑ done |
| 2 | `.gitignore` awareness in grep/find | ☑ done |

The two are independent and land as separate commits.

---

## 1. Scope

**In**

- **Parallel tool execution** — when the model emits several tool calls in one
  message, execute the independent ones concurrently instead of one-at-a-time.
- **`.gitignore` awareness** — `grep` and `find` respect the project's ignore
  rules, so ignored trees are not searched or listed.

**Out**

- Per-tool timeouts, background/long-running processes (jcode's `bg`/`batch`) —
  separate concerns.
- Any change to the mutation semantics: mutating tools stay serialized.
- Image/binary handling (a third, larger gap — not this plan).

## 2. Reference: how pi & jcode do it

**pi** (`packages/agent/src/agent.ts`, `types.ts`, `agent-loop.ts`)

- Parallel tool execution is the **default**: `toolExecution ?? "parallel"`, with
  `"sequential"` as an explicit mode (`ToolExecutionMode`).
- The documented contract: *"preflight tool calls sequentially, then execute
  allowed tools [concurrently]"* — i.e. hooks/validation run in order first, then
  execution fans out.
- Concurrency is **per-tool opt-in**: a tool declares it *"can execute
  concurrently with other tool calls"*.
- Results are re-ordered back to call order (`orderedFinalizedCalls`) before being
  appended to the context.

**pi** (`core/tools/find.ts`, `grep.ts`)

- Find/grep **respect `.gitignore`** (shelling to `fd`/`rg`), with explicit
  `--no-require-git` handling and awareness of nested-repo boundaries.

**jcode**

- Parallel/batch dispatch (`tool/batch.rs`) and ignore-aware search
  (`agentgrep`).

**wcode today**

- `crates/wcode-harness/src/loop_.rs`: `// Tool execution (sequential).`, a
  `while let Some(..) = pending.next()` loop awaiting each `tool.execute`.
- `crates/wcode-cli/src/tools/mod.rs`: `// ponytail: full mutation queue when
  parallel exec lands`, and a `Arc<tokio::sync::Mutex<()>>` **already shared** by
  `edit`/`edits`/`replace`/`write` (reads don't take it).
- `crates/wcode-cli/src/tools/grep.rs`: `const SKIP_DIRS = [".git", "target",
  "node_modules", "build", "dist", …]`; both grep and find walk with
  `walkdir::WalkDir`.

## 3. Design — Phase 1: parallel tool execution

### The seam already exists

- Tool events carry `call_id` (`ToolExecutionStart`/`Update`/`End`), so
  interleaved streams stay pairable by the UI.
- The mutation lock exists and is exactly the primitive needed.
- The kernel already collects the batch (`calls`) before executing.

### Opt-in per tool

Add one method to `TypedTool` (default **false** — conservative, opt-in):

```rust
/// Safe to run concurrently with other calls in the same batch. Read-only
/// tools opt in; anything that mutates state stays a barrier.
fn parallel_safe(&self) -> bool { false }
```

Thread it through `ErasedToolCore`/`Tool` the same way `name`/`description` are.

| tool | `parallel_safe` | why |
|---|---|---|
| `read`, `grep`, `find`, `ast_search` | **true** | read-only |
| `edit`, `edits`, `replace`, `write` | false | mutate (and take the lock) |
| `bash`, `ast_edit` | false | can do anything — barrier by default |

### Execution model: concurrent groups between barriers

Not "run everything at once" — that would let a `read` race a `write` in the same
batch. Instead:

1. **Preflight, in call order** (sequential): `transform_tool_input` →
   `before_tool_call` → dead-sink check. Produces a plan of
   `Run(tool, args)` / `Blocked(reason)` / `Aborted`.
2. **Partition** the plan into groups: a maximal run of adjacent
   `parallel_safe` calls becomes one concurrent group; every other call is its
   own single-call group (a **barrier**).
3. **Execute groups in order**; within a group, fan out with a concurrency cap.
   A later group cannot start until the earlier group has fully completed, so
   each call observes all prior effects and no partial state.
4. **Collect** results into `Vec<Option<ToolOutput>>` indexed by call position.
5. **Append to context in call order** — this is a hard requirement (the API
   pairs each `tool_call` with its result positionally).
6. `synthesize_unexecuted` keeps its job: any call without a result (blocked,
   aborted, dead sink) still gets an error `ToolResult`.

This is strictly a superset of today's behavior: with all tools opting out, every
group is a single call and the order is identical.

### Events

- `ToolExecutionStart` for every call in a group, emitted in call order before
  fanning out.
- `ToolExecutionEnd` — see open questions: emit **as each completes** (live
  feed, needs the UI to pair by `call_id`) or **in call order** (deterministic,
  buffers a fast tool behind a slow one). Context order is unaffected either way.
- `ToolExecutionUpdate` from inside a tool already flows through `ctx.events`;
  two concurrent tools can interleave on the same channel — fine for a
  call_id-keyed UI, worth a note in the REPL printer's mid-line handling.

### Cancellation

Each execution observes `ctx.cancel` (`Tool::execute` returns `"cancelled"`
immediately when set; `bash` kills its process group). On cancel mid-group, the
remaining groups should not start and `synthesize_unexecuted` covers the rest.
Confirm no group can hang past cancel.

### Knob

`[tools] parallel = true|false` (default **true**, matching pi) plus
`--sequential`. Default-on keeps the common case fast; the knob exists for
deterministic debugging and for anyone who prefers today's ordering.

## 4. Design — Phase 2: `.gitignore` awareness

### Approach

Use the **`ignore` crate** (ripgrep's walker — cached, `0.4.26`): pure Rust,
already a sibling of our `globset` dep, and it handles `.gitignore`,
`.ignore`, `.git/info/exclude`, global excludes, and nested repos correctly.
Hand-rolling gitignore semantics is the alternative and is a bad trade.

Replace `walkdir::WalkDir` with `ignore::WalkBuilder` in `grep.rs` and
`find.rs`.

### Decisions inside it

- **Keep `SKIP_DIRS` as a fallback filter.** Belt-and-braces: `target/` and
  `node_modules/` are skipped even if a repo forgot to ignore them. `ignore`
  also skips `.git` itself.
- **`require_git(false)`** so `.gitignore` applies outside a git repo too (pi
  makes the same call) — see open questions.
- **Hidden files**: today's `WalkDir` searches dotfiles; ripgrep's default skips
  them. Keep **including** them (`.hidden(false)`) so this change is *only*
  about gitignore and doesn't silently narrow search.
- **Global/exclude files**: leave `ignore`'s defaults on (respect
  `.git/info/exclude` and the user's global gitignore). That is what "respects
  .gitignore" means to a user.

### Escape hatch

Add a `no_ignore: bool` arg to `grep` and `find` (ripgrep's `--no-ignore`): the
model occasionally needs to search ignored files (a vendor dir, a build output it
must diagnose). Without it, gitignore-awareness becomes a wall.

## 5. As built (target)

- **`crates/wcode-harness/src/tool.rs`** — `parallel_safe()` on `TypedTool`,
  plumbed through `ErasedToolCore`/`Tool`.
- **`crates/wcode-harness/src/loop_.rs`** — preflight → group → fan out →
  order → append; replaces the sequential loop.
- **`crates/wcode-cli/src/tools/mod.rs`** — mark the read-only tools; drop the
  ponytail comment.
- **`crates/wcode-cli/src/tools/grep.rs` / `find.rs`** — `WalkBuilder`,
  `no_ignore` arg.
- **New dep**: `ignore` (cli). **Config**: `[tools] parallel`, `--sequential`.

## 6. Phased tasks

**Phase 1 — parallel tool execution.**
- [x] `parallel_safe()` on `TypedTool` + `Tool`; read/grep/find/ast_search opt in.
- [x] Loop: preflight → groups → fan-out (`buffer_unordered`, cap 8).
- [x] Results appended in call order; Start/End emitted in call order per group.
- [x] `parallel` on `LoopConfig`/`AgentConfig`; `[tools] parallel` + `--sequential`.
- [x] Loop tests: overlap, barriers, call order under out-of-order completion,
      `parallel=false`, plus the dead-sink pairing invariant.

**Phase 2 — `.gitignore` awareness.**
- [x] Add `ignore` (replacing `walkdir`); `walker()` in grep, used by find.
- [x] Keep `SKIP_DIRS` as a filter_entry floor; `no_ignore` arg on both tools.
- [x] Tests: gitignore honored outside a git repo, negation, parent-dir rules,
      the `SKIP_DIRS` floor, and the `no_ignore` escape.

## 7. Testing & verification

- **Unit (parallel)**: a test tool that records `(start, end)` instants — assert
  two `parallel_safe` calls **overlap**; assert a non-safe call never overlaps
  (barrier); assert context order equals call order even when completion order
  differs; assert every `ToolCall` still has exactly one `ToolResult`.
- **Unit (ignore)**: temp repo with `.gitignore` — grep/find skip ignored
  entries; `!keep.txt` negation is honoured; a nested repo's rules don't leak;
  `no_ignore: true` sees the ignored file.
- **Live**: `--dump`-style check is not applicable here, so verify on a real run —
  ask the agent to read three files in one turn and time it before/after (expect
  a clear wall-clock drop), and confirm the session JSONL still pairs calls and
  results in order. For ignore: plant a marker string in an ignored dir in this
  repo, confirm `grep` misses it and `grep no_ignore` finds it.
- `cargo test --workspace` + `cargo clippy --workspace --all-targets` clean.

## 8. Decisions & open questions

**Decided**

- **Opt-in `parallel_safe`**, default `false` — a tool must claim safety; `bash`
  and all mutating tools are barriers.
- **Concurrent groups between barriers** rather than "everything at once" —
  preserves read-after-write semantics inside a batch.
- **Context order is always call order** (API requirement).
- **`ignore` crate**, not hand-rolled gitignore parsing.

**Open**

- **`ToolExecutionEnd` ordering** — as-completed (live, needs call_id pairing in
  the UI) vs in-call-order (deterministic). Recommend as-completed for
  responsiveness, with the REPL printer teaching itself to pair.
- **Concurrency cap** — unbounded (batch size is model-chosen, usually < 8) vs a
  fixed cap (e.g. 8). Recommend a small cap as a runaway guard.
- **`require_git`** — apply `.gitignore` outside a git repo (pi does) or only
  inside one (ripgrep's default)? Recommend `false` (pi's behaviour).
- **Hidden files** — confirm we keep searching dotfiles (`.hidden(false)`), i.e.
  this change is only about ignore rules.
- **`[tools] parallel` default** — on (pi's default, recommended) vs off with
  opt-in. On unless the live timing check shows a regression.

## 9. Progress

- [ ] Settle the open questions above.
- [x] Phase 1 (parallel tool execution) — verified live (see §11).
- [x] Phase 2 (`.gitignore` awareness) — verified end-to-end (see §10).

## 10. Phase 2 result

`walker()` in `grep.rs` builds an `ignore::WalkBuilder`: `.hidden(false)` (hidden
files still searched — only ignore *rules* changed), `.parents(true)`,
`.require_git(false)`, and `no_ignore` turning `git_ignore`/`git_global`/
`git_exclude`/`ignore` off. `SKIP_DIRS` moved from a `walkdir` `filter_entry` to
the same hook on the new walker, so it survives as a floor. `find` reuses
`walker()`; both tools gained `no_ignore: Option<bool>`.

Verified against the real binary by reading the **recorded tool results** out of
the session JSONL (the REPL only previews the first line, so the model's prose is
not evidence):

| run | `grep` output |
|---|---|
| default, `secret/` in `.gitignore`, dir **not** a git repo | `visible.txt` only |
| `no_ignore: true` | `visible.txt` + `secret/hidden.txt` |

`find` behaved identically. The non-git directory is what exercises
`require_git(false)` end-to-end.

**Still open for phase 2:** whether hidden files should keep being searched
(current: yes, to avoid a silent narrowing) — revisit if it ever matters.

## 11. Phase 1 result

`Tool::parallel_safe()` (default `false`) gates concurrency; `read`, `grep`,
`find` and `ast_search` opt in, everything else — including `bash` — stays a
barrier. The loop preflights every call in order (`transform_tool_input`,
`before_tool_call`, tool lookup), partitions into maximal runs of safe calls,
fans each run out with `futures::stream::iter(..).buffer_unordered(8)`, and then
emits End / records / pushes to ctx **in call order**. `parallel` on
`LoopConfig` (set from `AgentConfig::parallel_tools`, from `[tools] parallel`)
forces the old strictly-sequential path.

Two things worth recording:

- **`ToolExecutionStart` now follows preflight, not the block check.** It used to
  be sent per call *before* `before_tool_call`; it is now sent for a whole group
  once preflight has run (pi's contract: preflight sequentially, then fan out).
  The observable difference is timing, not semantics — a blocked call still gets
  Start + an error End. `dead_sink_mid_tool_loop_synthesizes_results` had
  encoded the old timing, so it now asserts the invariant that actually matters:
  every `ToolCall` has exactly one `ToolResult`, in order.
- **End is emitted in call order, not completion order.** The REPL's ✓ line
  carries no tool name, so out-of-order Ends would render as unattributable
  results. Call order keeps the documented `⚙ name ✓ first-line` pairing exact.

Live check (real binary, local model, three `grep` calls in one message):

```
--sequential            default (parallel)
⚙ grep                  ⚙ grep
 ✓ d12/f6.txt…          ⚙ grep
⚙ grep                  ⚙ grep
 ✓ d12/f35.txt…          ✓ d12/f6.txt…
⚙ grep                   ✓ d12/f35.txt…
 ✓ d12/f6.txt…           ✓ d12/f6.txt…
⚙ bash                  ⚙ bash
 ✓ .//                   ✓ .//
```

All three markers before any result = one group; `bash` still its own barrier
afterwards. Wall-clock is not a clean signal here (model latency dominates), so
the event interleaving is the evidence.
