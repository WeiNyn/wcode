# D019 — A tool's params always render; collapse governs the output body

- **Status:** accepted (human sign-off 2026-10-10, this session — W015)
- **Date:** 2026-10-10
- **Relates to:** D031 (the tool panel — params, expand/collapse), D010 (the tool
  tree row; the collapse semantics it inherits), D032 (the `▸`/`▣` affordances),
  D009 (inline tools in call order)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §4 (**Tools**) and the
  §5 keymap line — *"`Ctrl-T` expands/collapses every tool's **output** (params
  stay visible; …)"*
- **Fixes:** a latent defect that shipped with D031 and never matched
  `docs/work/tui-render/redesign-v2.md` §3(a)

## Context

A finished, collapsed, non-error tool panel rendered its head row and its output
preview but **not its input params**. `tool_inline_lines`
(`crates/wcode-tui/src/ui.rs:905`) gated them:

```rust
    let expanded = tool.expanded || tool.is_error;
    let mut body: Vec<Line<'static>> = Vec::new();
    if !tool.done || expanded {
        body.extend(panel_param_lines(tool, width));
    }
```

So a completed `bash` showed a bare `├ bash` — and since the head inlines a
target *only* when `params` is empty (`ui.rs:1030`
`Some(target) if tool.params.is_empty()`), it showed neither the command nor a
target. The one thing a `bash` panel is for was absent on its **most common**
state (finished).

The data was never lost: `call_params` (`app.rs`) already collected the call's
arguments into `Tool.params`; the toggle merely hid them at draw time. Three
facts made this a defect rather than a choice:

1. **The design drew params collapsed.** `docs/work/tui-render/redesign-v2.md`
   §3(a) *"collapsed (80×24)"* literally shows `│  cmd  cargo test -p wcode-cli`,
   and §3(b) requires *"the params row stays visible when expanded"* — visible in
   **both** states.
2. **D031 requires it.** *"The `bash` command is shown in full (it wraps, never
   clips)."* A gate that hides it when finished defeats the requirement.
3. **The gate was name-agnostic** — `read`, `edit`, `write`, `grep`, `find` lost
   their inputs too; `bash` was merely the most visible.

A second defect surfaced while fixing it: `tool_param_keys` (`app.rs:773`) named
keys that real calls never carry — `bash`'s `cmd`/`cwd` (the schema field is
`command`; `cwd` is not a bash arg at all) and, decisively, `find`'s `pattern`,
which `FindArgs` (`crates/wcode-cli/src/tools/find.rs`) **does not have** (it is
`{ path, glob, kind, max, no_ignore }`). A `find` panel therefore had *no*
renderable params even after the gate fix.

## Decision

1. **A tool's params rows always render** — running or done, collapsed or
   expanded. `panel_param_lines` runs unconditionally (`ui.rs:908`).
2. **The disclosure governs only the output body.** The `▸`/`▾` affordance and
   `Ctrl-T` expand/collapse the output/diff preview and the `… +N more` hint;
   they never hide params. Params are the call's *identity*.
3. **The params key is the literal JSON key a real call carries.** No aliases:
   `tool_param_keys` lists `bash` → `command`; `read` → `path`/`offset`/`limit`;
   `edit`/`write` → `path`; `grep` → `pattern`/`path`; `find` →
   `path`/`glob`/`kind`.
4. **No per-tool hide list and no config knob.** Every list is bounded to ≤3
   short scalars (a mutator's payload is the *diff body*, not a param), so there
   is no tool that needs its params hidden; a table would be
   config-for-behaviour.

## Mechanism

- `tool_inline_lines` (`ui.rs:905-916`) — the `if` is gone;
  `body.extend(panel_param_lines(tool, width));` is unconditional.
  `tool_panel_body(tool, width, expanded)` keeps its `expanded` keying, so
  collapse still means "output preview".
- `tool_param_keys` (`app.rs:773-784`) — `bash` → `&["command"]`,
  `find` → `&["path", "glob", "kind"]`.
- The head-target rule (`ui.rs:1030`) is **kept unchanged** as a fallback: it
  fires only when `params` is empty, i.e. a call whose args miss its per-tool
  key list, where `call_target`'s broader `ACTION_KEYS` still resolves a label.
- The added rows are appended *inside* `tool_inline_lines`'s returned `lines`,
  so `turn_inline`'s `off += lines.len()` walk (`ui.rs`) and
  `inline_tool_offsets` agree — no offset drift, and the one-panel-per-tool /
  `¹`-ordinal contracts are untouched.

## Consequences

- A finished, collapsed `bash` reads `├ bash  <stats>  ▸ ▣` over
  `│   command  cargo test -p wcode-cli`; the command wraps in full, never
  clipped. The same holds for `read`/`write`/`edit`/`grep`/`find`.
- `Ctrl-T` still collapses/expands the output; params stay put, so the toggle
  now has exactly one meaning.
- A tool's head shows its name alone when params carry the target — the target
  inlines only in the params-empty fallback.
- **Foreclosed:** a per-tool "hide params while collapsed" list; a config knob
  for param visibility; re-introducing a boxed `╭─ ⚙ bash ─╮` frame (the shipped
  form is the D010 tree; `a_note_is_frameless` bans `╭`).
- Gates live in W015 §8 — five collapsed-path tests (`bash`, head interaction,
  `read`, `write`, `find`), each of which fails if the gate is restored, plus
  `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean.

## Residuals (non-gating)

Two test-hygiene leftovers name the retired `cmd` alias: the helper doc at
`crates/wcode-tui/src/ui.rs:4637` (`push_bash_panel` — "its call's `cmd`/`cwd`"),
and the direct-params fixture `panel_tool("bash", vec![("cmd".into(), …)])` in
`app.rs` (no assertion reads it). The `cwd` an extra `push_bash_call` fixture
still sends is now an unrecognized key — harmless, and implicitly the coverage
that `cwd` no longer renders. Cosmetic; no behaviour at stake.
