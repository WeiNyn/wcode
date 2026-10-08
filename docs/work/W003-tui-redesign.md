# TUI redesign — brief

- **Status:** **signed off** 2026-09-05 — decisions 1, 2, 4, 5 accepted as defaults; **#3 refined: thinking renders in-flight (streams live) and collapses when the turn is done**
- **Work item:** W003
- **Decisions:** proposes **D29–D36** (to be added to the D-table); extends locked **D1–D28**; builds on **D1b/D3/D4b/D5/D6a/D7** from the v1 mockups
- **Author:** session `agent:brainstormer`
- **Date:** 2026-09-05

## 1. The ask

The wcode TUI's **core features are fine; the TUI experience is poor.** Seven source-verified complaints: (1) it still reads as a **wall of text** despite ratatui; (2) the sidebar, tool blocks, and thinking are **not interactive** — no click-to-expand/collapse, no copy affordance; (3) tools **do not show their params** and `bash` shows only the **first argument, first line, clipped at 60 chars**; (4) colors are **too dim to read** (`dim` = `Modifier::DIM`, no fg, ≈3:1); (5) the sidebar styling and **agent listing order** are wrong and **`Alt+1..N` does not match the listing**; (6) **Todos** should leave the sidebar for the transcript; (7) **Changes** should be a **tree view**. The two proposal docs (`mockups.md` v1, `redesign-v2.md` v2) already settle the visual language (D29–D36, labeled 80×24 renders, a fixed glyph table) and a real captured frame exists. This brief is the reviewable contract before any of it is coded.

### 1.10 Facts I did NOT verify (name them explicitly)

- **Commit `dcace19`** (the headless dumper) — could not run `git`; the dumper *code* and the `target/tui-render/*` frames both exist, so the artifact is real, but the hash is asserted, not checked.
- **Contrast ratios** (≈3:1 / 7.7:1 / 4.6:1 in v2) are hand-computed in the proposal, not measured on a real terminal.
- **Panel real-estate cost at 80×24** ("3 rows of chrome per tool") is hand-drawn, not measured.
- **crossterm click-vs-drag separation** for the D32 drag guard — v2 flags this as unverified; the brainstormer did not test it either.
- Did **not** run any test or clippy (read-only), so every test name below is read from source, not executed; test *outcomes* are unknown.
- Did **not** audit `TestBackend` snapshots for what D29–D36 would move (there are many `ui.rs`/`app.rs` tests; only the ones read are named).
- The exact wording claims in v2 for `docs/tui-design.md:3Kx5S` were read; not every §-anchor in `docs/tui-design.md` was re-verified.

## 2. Scope / Non-scope

**In scope.**

- **Palette** (`crates/wcode-tui/src/theme.rs`): an explicit **legible grey ramp**; `dim` and `thinking` lose the `DIM` modifier (it survives only under `NO_COLOR`); land the **tier palettes** (Basic-8 / 256 / truecolor) through the existing ladder.
- **Tool blocks** → a **panel** with a keyed **params** block; `bash` shows the **full command** (wraps, never clips) and `cwd`; other tools show `path`/`pattern`/`offset`.
- **Per-block disclosure**: click a header (or `▸`/`▾`) to expand/collapse, click `⧉` to copy, with keyboard parity in browse mode.
- **Thinking** collapsed to `··· thinking · N chars`, expandable.
- **One canonical surface order** (stable, root-first) driving the sidebar, the working strip, and `Alt-1..N`; sidebar rows **numbered**; rounded borders everywhere.
- **Todos** become a transcript block; the sidebar Todos section is removed.
- **Changes** render as a directory **tree** (sidebar and `/changes`).
- **Spec/doc amendments**: `docs/tui-design.md` (§1.2, §1.3, §1.4, §2, §4, §5, §6) and the D-table in `docs/team-and-tui-plan.md` (add D29–D36); the v1 mockups' small items (D1b session fold, D4b state emphasis, D5 seeded hint, D7 rounded).

**Non-scope.**

- **The kernel** (`wcode-harness`) — every change is presentation-only inside `crates/wcode-tui` + docs; the `AgentEvent` seam is untouched.
- **The VS Code surface** (`docs/vscode-ui-*.md`) — a separate client with its own visual system; a TUI-only restyle does not belong there.
- **Any new palette *role*** — the 17 roles (`theme.rs:ZcIx1`, roster at `theme.rs:uxlr2`) are fixed; this changes *what the existing grey roles emit*, not the role set. A new role is a separate decision.
- **Config for behavior** — no `[ui]`/`[tool-panel]` toggles; per `AGENTS.md`, behavior lives in code via `Hooks`. The only new config surface is none.
- **A `$PAGER` viewer / edge autoscroll / double-click-to-expand** — parked in `docs/tui-browse-plan.md` and `docs/tui-mouse-plan.md`; a separate phase.
- **Mermaid / a narrow side panel** — `docs/tui-markdown-plan.md` stretch, untouched.
- **Moving existing docs into `docs/plans/…`** — tracked as W-numbered item 58 (D004); a separate link-sweep change.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | Palette grey ramp + DIM removal (D29) | high | low | low | the core legibility complaint; a `theme.rs` table + span restyle, gated by `NO_COLOR` |
| P1 | Tier palettes (D30) | medium | medium | low | three palettes × 17 roles; the degrade logic already exists, so it is table data |
| P2 | Tool panels + params + full bash cmd (D31) | high | high | medium | a new panel renderer + param extraction on a per-block render-cache path; 80×24 real estate |
| P3 | Per-block click/copy (D32) | high | medium | medium | per-affordance hit regions must not fight drag-to-select; a new mouse routing branch |
| P4 | Thinking in-flight → collapse on done (D33) | medium | low | low | one `content_lines` arm + an auto-collapse at turn end; presentation-only |
| P5 | Canonical order + Alt-N (D34) | medium | medium | medium | one order fn threaded through **three** divergent sort sites; a learnable but real behavior change |
| P6 | Todos → transcript (D35) | medium | medium | low | a new `Block` variant + live update + a sidebar section removal; `AgentEvent::Todo` already carries the data |
| P7 | Changes tree (D36) | medium | medium | low | a tree builder + two render sites; path strings already collected |
| P8 | Visual chrome: D1b/D4b/D5/D7 + D3 lock | medium | low | low | small span/border restyles, each with a pinned layout test |
| P9 | Spec/doc amendments + glyph table | medium | low | low | prose + a decision table; the D-table is the single index |

## 4. Interface & structure

**Added.**

- `crates/wcode-tui/src/app.rs`: `Block::Todos(Vec<TodoItem>)` (new variant on `Block`, `app.rs:k52Iz`); `Tool.params: Vec<(String, String)>` (or `Tool.args: serde_json::Value`) beside `Tool.target`; `App::canonical_order() -> Vec<usize>`; per-block affordance hit regions on the existing `HitMap` (`app.rs:gHXEF`).
- `crates/wcode-tui/src/ui.rs`: `tool_panel_lines()`, `thinking_collapsed_lines()`, `change_tree()`, `Block::Todos` render arm, round-border helper for overlays.
- `crates/wcode-tui/tests/` and `crates/wcode-tui/examples/dump.rs`: new snapshot fixtures for the panel/thinking/tree states.
- `docs/work/W003-tui-redesign.md` — this brief (orchestrator records).

**Changed.**

- `theme.rs::Theme::colored` (`theme.rs:4u4b7`, `dim` at `theme.rs:7YBTt`) — `dim` gets an explicit grey fg, no `DIM`; `thinking` drops `DIM` (`theme.rs:TnBkh`/`zfPJX`); `Theme::from_palette` (`theme.rs:UuLks`) maps the new grey steps; `CATALOG` (`theme.rs:Vrl2O`) gains the tier palettes.
- `ui.rs::target_span` (`ui.rs:rvJud`) — the 60-char/first-line clip is replaced by the params block; `ui.rs::tool_lines` (`ui.rs:ogadO`) grows the panel frame + `▸`/`⧉`.
- `ui.rs::content_lines` Thinking arm (`ui.rs:HXD65`) — inline wrap becomes a collapsed affordance row.
- `ui.rs::draw_sidebar` (`ui.rs:OVnem`) — three sections → **two** (`agents`, `changes`); numbered rows; Changes as a tree.
- `ui.rs::draw_help` (`ui.rs:pYj7j`) / `ui.rs::draw_picker` (`ui.rs:uT9Be`) — add `Borders::ALL` + **rounded** (today square; only `ui.rs:6Xzw7` uses Rounded).
- `app.rs::ACTION_KEYS` (`app.rs:Lot1u`, add `"cwd"`) and `call_target` (`app.rs:5pRjx`) — capture more than the first arg.
- `app.rs::member_rows_indexed` (`app.rs:mqoN0`, sort `:7wDvL`) and `working_team_rows` (`app.rs:cix9e`, sort `:FQAXh`) — both delegate to `canonical_order()`.
- `app.rs::focus_digit` (`app.rs:kf8T6`) → `set_focus` (`app.rs:wOITP`) — `Alt-N` maps to the Nth canonical row, not `surfaces[N-1]`.
- `app.rs::toggle_selected` (`app.rs:Gt0pc`) — extend from Tool-only to Thinking.
- `app.rs::on_mouse_down` (`app.rs:wDms1`) — route a header/`▸`/`⧉` click before the drag-select anchor.
- `app.rs::KEYS` (`app.rs:Spmdk`, `Alt-1..9` entry `app.rs:jT3On`) — reword to "focus sidebar row N".
- `ui.rs::corner_titles` (`ui.rs:iKPYw`) and `ui.rs::draw_session_line` (`ui.rs:xv4Zb`) — D1b fold + D4b restyle.

**Integration points.**

- `crates/wcode-tui/src/app.rs:k52Iz` — `pub enum Block` = `User, Assistant, Tool, Notice, Error, Btw, Diff` — **no** `Todos`/`Changes` variant; both are new.
- `crates/wcode-tui/src/app.rs:Lot1u` — `const ACTION_KEYS: [&str; 9]` = `path, file_path, file, pattern, command, cmd, query, url, name`; **no `cwd`**.
- `crates/wcode-tui/src/app.rs:5pRjx` — `call_target` returns the **first** present arg among `ACTION_KEYS` — one string only.
- `crates/wcode-tui/src/ui.rs:rvJud` — `target_span` takes `target.lines().next()` and clips to **60 chars** (`split_at_char(first, 59)`) — the bash complaint.
- `crates/wcode-tui/src/ui.rs:ogadO` — `tool_lines`: the finished tool is a `⚙ name <target>` + `✓ name · note` pair (no params block).
- `crates/wcode-tui/src/ui.rs:HXD65` — `ContentBlock::Thinking { text }` renders inline via `wrap(text, …, THINK_FIRST, THINK_CONT, thinking())`; non-collapsible (`THINK_FIRST` `ui.rs:3vxfD` = `"   ··· "`).
- `crates/wcode-tui/src/theme.rs:7YBTt` — `dim: Style::new().add_modifier(Modifier::DIM)` — **no fg**; the workhorse role (`ui.rs:btua1` `dim()` returns `theme::theme().dim`).
- `crates/wcode-tui/src/theme.rs:TnBkh`/`:zfPJX` — `thinking` = `Magenta + DIM + ITALIC`.
- `crates/wcode-tui/src/theme.rs:7wN6X`/`:QNCsA`/`:vlSit`/`:YJuVt` — the color-mode ladder (`Plain/Named/Indexed/Rgb`), `ColorMode`, `color_mode()`, `resolve()` already exist; tier palettes slot in here.
- `crates/wcode-tui/src/app.rs:mqoN0` (sort `:7wDvL` "running-first, then Reverse(action_at)") vs `crates/wcode-tui/src/app.rs:cix9e` (sort `:FQAXh` "oldest→newest, last-3") vs `crates/wcode-tui/src/app.rs:kf8T6` (`Alt-N` → `set_focus(n-1)` = raw `surfaces` index) — **three divergent orders**.
- `crates/wcode-tui/src/app.rs:wOITP` — `set_focus(idx)` sets `self.focus = idx`; `App.surfaces[0]` is the root (`app.rs:PObkQ` "Index 0 is the root"), and `member_rows_indexed` filters `!s.is_root` (`app.rs:e6dCH`) — so sidebar row 1 is `surfaces[1]`, i.e. `Alt-2`, not `Alt-1`.
- `crates/wcode-tui/src/ui.rs:OVnem` — `draw_sidebar` stacks **Team** (`ui.rs:WRkiC`), **Todos** (`ui.rs:5R7Lv`), **Changes** (`ui.rs:na8uD`) — **three** sections; `ui.rs:IBBPK` pins exactly those three.
- `docs/tui-design.md:4qLNe` (and `:KpqSU`) — the spec claims **four** sections including "Context" — **doc rot, not a layout bug** (no code change; text amend only).
- `crates/wcode-tui/src/app.rs:GYXvc` + `:S13Mv` — `last_todos: Option<Vec<TodoItem>>` cached from `AgentEvent::Todo` (`app.rs:UHptB`), which today dumps a `Block::Notice` via `render_todos` (`app.rs:5XxjU`). The data path already exists.
- `crates/wcode-tui/src/app.rs:v8upK` (changes_by_path) / `:QIo78` (summary) / `:FCnvC` (`changes()`) — the flat `(path, +a, −r)` data a tree view groups.
- `crates/wcode-tui/src/app.rs:gHXEF` (`HitMap`), `:tsjjA` (`SidebarHit{m rect, members}`), `:KdI9G` (`TranscriptHit`) — the per-frame hit geometry; D32 extends it with affordance regions.
- `crates/wcode-tui/src/app.rs:THmWh` (`on_mouse`) → `:wDms1` (`on_mouse_down`), `:hAIlG` (`on_mouse_drag`), `:Rtl4x` (`on_mouse_up`, click-vs-drag split) — the routing an affordance click joins.
- `crates/wcode-tui/src/app.rs:Gt0pc` (`toggle_selected`, Tool-only) and `:WGQnR`/`:ZgPv2` (`copy_selected`/`copy_text`) — the existing expand/copy paths D32 generalizes.
- `crates/wcode-tui/src/app.rs:lshCP` (`set_sidebar_hit`) + `:YDv8n` (`sidebar_member_at`) — how a sidebar click maps to a surface today.
- `crates/wcode-tui/src/ui.rs:HjDd3` (`draw_input_box`) / `:iKPYw` (`corner_titles`) / `:xv4Zb` (`draw_session_line`) — where D1b folds the session row and D4b restyles project↔state.
- `crates/wcode-tui/src/ui.rs:pYj7j`/`:uT9Be` — the help and picker modals render with a default (square) border; only `ui.rs:6Xzw7` sets `BorderType::Rounded`.
- `crates/wcode-tui/src/ui.rs:hQG0Q` (`bar`, `▰▱`) / `:lom2q` (`token_spans`) — the shipped gauge (v1 D3 recommendation: keep `▰▱`, fix §2).
- `crates/wcode-tui/src/app.rs:4f4oV` + `:s6sV5` — `append_block_lines` re-renders a block only when `(rev, width)` changes; a panel/thinking toggle must `bump_rev` (as `toggle_selected` already does).
- `crates/wcode-tui/src/event.rs:qAs7V` — `translate_mouse` maps only the wheel + left Down/Drag/Up; `event.rs:c6J87`/`:yLQM7` maps `Alt+char` to `Key::Alt`.
- `docs/team-and-tui-plan.md:224` (`bQzob`) — the locked **D1–D28** table (the place to add D29–D36).
- `crates/wcode-tui/src/lib.rs:JydeX` (`render_text`) / `:jYkH4` (`render_plain`) + `crates/wcode-tui/examples/dump.rs` — the headless dumper every step's gate uses.
- `target/tui-render/80x24.txt` — a real current frame: strip `● explorer`, rounded box, `⚙ bash  cargo test … | tail -40`, `✓ bash · test result_01 …`.

## 5. Plan

1. **Palette grey ramp + DIM removal (D29) + D4b.** Deliverable: `dim` (and `thinking`) carry an explicit grey/`DIM`-free style under color; run state `accent`, project `muted` in `corner_titles`. Gate: `cargo test -p wcode-tui theme` green (updated `the_default_preset_is_palette_b` / new `dim_carries_a_grey_and_no_dim_modifier`); `NO_COLOR=1` frame still has no fg (`the_plain_theme_sets_no_foreground_on_any_role`); `examples/dump` frame diff.
2. **Tier palettes (D30).** Deliverable: Basic-8 / 256 / truecolor palettes wired through `resolve_color_mode_from` + `from_palette`. Gate: new `each_tier_yields_its_own_grey_steps`; `a_truecolor_preset_degrades_under_a_non_rgb_mode` still green.
3. **Tool panels + params + full bash command (D31).** Deliverable: a rounded panel per tool with a keyed params block (`cmd`, `cwd`); the command wraps, never clips; `ACTION_KEYS` gains `cwd`. Gate: new `a_bash_panel_shows_the_full_command_unclipped` (assert the rendered frame contains the whole string); `examples/dump` 120×40 frame.
4. **Per-block click / copy (D32).** Deliverable: a header/`▸` click toggles, `⧉` copies (OSC-52), keyboard parity via browse `Enter`/`Space`/`y`; drag-through still selects text. Gate: new `clicking_a_panel_header_toggles_only_that_block` and `clicking_copy_yields_the_block_text` (mouse Down→Up same cell); the existing drag tests (`app.rs:8iq6L`) stay green.
5. **Thinking in-flight → collapse on done (D33).** Deliverable: thinking **streams in-flight** (expanded while the turn runs), then **auto-collapses** to `··· thinking · N chars ▸ ⧉` when the turn ends; manually expandable afterwards via `▸`; expanded body `thinking`-styled. Gate: new `thinking_renders_one_line_until_expanded` and `thinking_auto_collapses_on_turn_end`; `toggle_selected` covers Thinking.
6. **Canonical order + Alt-N (D34).** Deliverable: one `canonical_order()` (root-first, creation order) driving sidebar + strip + `Alt-N`; sidebar rows numbered `1..N`. Gate: updated `member_rows_order_active_first_then_by_action_recency` (→ stable order) and `working_team_rows_keep_only_running_oldest_first_capped_at_three`; new `alt_n_focuses_the_numbered_sidebar_row`.
7. **Todos → transcript (D35).** Deliverable: `Block::Todos`, rendered live in the transcript; sidebar Todos section removed; `last_todos` kept for `/verify`. Gate: updated `the_open_sidebar_shows_its_section_headers_and_a_member_row` (now `agents`/`changes` only); new `todos_render_in_the_transcript`.
8. **Changes tree (D36).** Deliverable: `change_tree()` grouping by directory with `├─`/`└─`, stats right-aligned; used in the sidebar and `/changes`. Gate: new `the_changes_tree_groups_files_by_directory`; `changes_command_opens_a_picker_and_selection_shows_the_diff` green.
9. **Visual chrome: D1b / D5 / D7 (+ D3 lock).** Deliverable: session id folds into the input-box top-left (drop `draw_session_line` + the `session_h` layout row); seeded dim hint when empty; rounded borders on the picker + `F1` help; gauge locked as `▰▱`. Gate: updated `draws_a_committed_block` / layout tests; a frame with no session renders zero rows for it.
10. **Spec/doc amendments + glyph table.** Deliverable: `docs/tui-design.md` §1.2/§1.3/§1.4/§2/§4/§5/§6 amended (drop the phantom "Context" sidebar section; resolve §6 thinking → collapsed; the single fixed glyph table); **D29–D36** added to `docs/team-and-tui-plan.md:224` (`bQzob`). Gate: the frames in `target/tui-render/` match the amended spec by hand-inspection; `docs/next-steps.md` gets a W003 pointer.

## 6. Quality gates

- `cargo test --workspace` → **all pass.**
- `cargo clippy --workspace --all-targets` → **clean** (workspace rule; `AGENTS.md`).
- `cargo run -p wcode-tui --example dump` → writes `target/tui-render/{80x24,120x40,48x20}.{txt,ansi}`; `cat target/tui-render/120x40.txt` shows the **full** bash command and the params block.
- `cargo build --bin wcode` → builds (proves nothing about behavior; the **live check** below is the real gate).
- A **live check** against a keyless local endpoint (per the `live-verification` skill): run the TUI with `--base-url http://localhost:11434/v1`, exercise a `bash` tool, `Ctrl-B`, `Alt-1..N`, a panel click, thinking collapse.
- Regression tests kept green (by name): `the_plain_theme_sets_no_foreground_on_any_role`, `no_color_wins_over_a_spec`, `resolve_color_mode_follows_the_ladder`, `the_semantic_palette_keeps_roles_distinct`, `a_truecolor_preset_degrades_under_a_non_rgb_mode`, `changes_command_opens_a_picker_and_selection_shows_the_diff`, and the drag-select mouse tests (`app.rs:8iq6L`).

## 7. Testing

### 7.1 Automated

- `theme`: `dim_carries_an_explicit_grey_and_no_dim_modifier` — asserts `dim.fg.is_some()` and `!dim.add_modifier.contains(DIM)` under color, and `Some(DIM)` under `plain`. `each_tier_yields_its_own_grey_steps` — asserts Basic-8/256/Rgb give distinct grey steps. (Updates `the_default_preset_is_palette_b`, which today asserts `dim.fg == None`.)
- `ui`: `a_bash_panel_shows_the_full_command_unclipped` — asserts the rendered frame contains the **entire** command (fails today at 60 chars). `a_tool_panel_shows_its_params` — asserts the `cmd`/`cwd` key rows appear. `thinking_renders_one_line_until_expanded` — asserts one affordance row collapsed, body rows expanded. `the_changes_tree_groups_files_by_directory` — asserts a directory header + `├─`/`└─` leaves. `the_open_sidebar_shows_two_sections_with_numbered_rows` — asserts `agents`/`changes` (not `Todos`) and a `1`/`2`/`3` badge.
- `app`: `clicking_a_panel_header_toggles_only_that_block` — Down→Up same cell toggles one block; `clicking_copy_yields_the_block_text` — a `⧉` click pushes `Action::Copy` with the full output; `alt_n_focuses_the_numbered_sidebar_row` — `Alt-1` focuses the first **member**, not `surfaces[0]`; `canonical_order_is_stable_root_first` — the order does not depend on activity.
- Seam/regression pair: the block-render cache — `append_block_lines` must re-render on a toggle (`bump_rev`); the drag-select pair (`in a drag, copy; in a click, select`) must both hold after affordance routing.
- Snapshot: regenerate `target/tui-render/*.{txt,ansi}` via `examples/dump` and diff against the amended spec.

### 7.2 How a human verifies it

```sh
cargo test --workspace && cargo clippy --workspace --all-targets
cargo run -p wcode-tui --example dump
cat target/tui-render/120x40.txt      # full bash command + params panel + rounded frame
cat target/tui-render/80x24.ansi      # palette legibility (dim now readable)
NO_COLOR=1 cargo run -p wcode-tui --example dump && cat target/tui-render/80x24.txt
```

Then the live TUI against a local endpoint:

```sh
cargo run -p wcode-cli -- --base-url http://localhost:11434/v1
```

Look for: `dim` tool lines **readable** at normal brightness; a tool **panel** whose `cmd` line is the **whole** command; clicking a header expands it; `Ctrl-B` then `Alt-1..N` focusing the **numbered** sidebar rows in order; a `··· thinking · N chars` line collapsed; todos in the transcript (not the sidebar); a `changes` directory tree.

## 8. Expected outcome

- `dim` text renders at ≥WCAG-AA contrast under color and still carries no fg under `NO_COLOR`.
- A finished `bash` shows its **full command** (no 60-char clip) plus `cwd`, in a bordered panel whose params stay visible when expanded.
- Clicking a panel header toggles it; clicking `⧉` copies it; `Ctrl-B` + `Alt-1..N` focus numbered sidebar rows that **match** the listing.
- Thinking is one collapsed line by default, expandable.
- Todos render in the transcript; the sidebar shows exactly **two** sections (`agents`, `changes`).
- Changes render as a directory tree in the sidebar and `/changes`.
- `docs/tui-design.md` describes the shipped screen (no phantom Context section); `docs/team-and-tui-plan.md` carries D29–D36; `docs/next-steps.md` points at W003.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` are green.

## 9. References

- `docs/work/tui-render/redesign-v2.md` — the v2 proposal: D29–D36, labeled 80×24 renders, the fixed glyph table, the spec sections to amend, the tradeoffs + open questions.
- `docs/work/tui-render/mockups.md` — the v1 selections (D1b, D3, D4b, D5, D6a, D7) this brief folds in.
- `target/tui-render/80x24.txt` (+ `.ansi`, `120x40`, `48x20`) — the real current frame the redesign is measured against.
- `docs/tui-design.md` — the visual spec to amend (§1.2 `zwmWX`, §1.3 `yQCWu`, §1.4 `vHHsu`, §2 `pMZOm`, §4 sidebar `4qLNe`, §4 transcript `a0dAT`/`nZDLN`, §4 keys `yZ4Cg`, §5 `1c4bV`, §6 `QwBGB`).
- `docs/team-and-tui-plan.md:224` (`bQzob`) — the locked D-table to extend.
- `docs/next-steps.md:zft6w` (item 14) — "tier palettes open" (D30 scope).
- `crates/wcode-tui/src/{theme,ui,app,event}.rs` and `crates/wcode-tui/examples/dump.rs` — the integration points above.
- `crates/wcode-tui/src/lib.rs:JydeX`/`:jYkH4` — the headless renderer every gate uses.

## Decisions (signed off 2026-09-05 — 1, 2, 4, 5 as defaults; 3 refined)

1. **Panels (D31):** always-on, or a compact one-line fallback when many tools are visible? *Default — confirm:* **always-on**, flagging the 80×24 cost (~3 rows of chrome per tool).
2. **Grey ramp (D29):** are `#a9b1ba` / `#7d8790` / `#3b4252` the right steps, or a warmer/cooler cast? *Default — confirm:* the v2 hexes.
3. **Thinking (D33):** **SIGNED OFF — refined.** Thinking renders **in-flight** (streams live, expanded) and **auto-collapses when the turn is done** (still manually expandable afterwards).
4. **Order (D34):** stable creation order, or a fixed running-first order that `Alt-N` follows exactly? *Default — confirm:* **stable creation order**.
5. **bash panel (D31):** show `cmd`/`cwd` always, or `cwd` only when it differs from the project root? *Default — confirm:* **always**.
