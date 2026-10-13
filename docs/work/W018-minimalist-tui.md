# W018 · P5 "Prose Only, fully bare" (TUI minimalism)

**Status:** draft, awaiting human sign-off.
**Author:** brief (read-only); recorded by the orchestrator.
**Decision record:** proposed `D023`.
**Spec authority:** `docs/tui-design.md`.

---

## 1. The ask

Make the wcode TUI read as **prose only, fully bare**: the transcript is the
assistant's prose at default weight, *everything else* is `dim`, and **red is the
only colour** (errors). Concretely: drop the three full-width `─` rules (header,
composer, foot leader); stop borrowing `Modifier::REVERSED` for the `WCODE`/` YOU`
labels (they keep the text, lose the inverse video); make tools **treeless** flat
rows at the gutter column instead of a `├`/`│`/`└` tree; retire the `▸`/`▾`/`▣`
disclosure-and-copy affix (keys only — `Ctrl-T`, browse `y`) together with its D32
click geometry; retire the `···` thinking marker (thinking becomes a plain dim
row); and replace the `▰▱` context gauge with plain `used/limit` text. The change
is a **restyle + glyph retirement**, not a reflow of the base layout (bands,
measure, gutter columns, browse, sidebar, overlays all stay).

---

## 2. Scope / Non-scope

### In scope

- **Chrome rules removed:** the header `─` rule (`ui.rs:442`), the composer `─`
  rule (`ui.rs:1982`), the composer-foot `─` leader (`ui.rs:2010` + the
  `padded_row` `fill` at `ui.rs:2043`).
- **Reverse-video retired at exactly two call sites:** `WCODE` (`ui.rs:765`) and
  ` YOU` (`ui.rs:712`) stop borrowing `selection_style()` (`ui.rs:651`). The label
  text stays.
- **Treeless tools:** retire `TREE_BRANCH`/`TREE_PIPE`/`TREE_END`
  (`ui.rs:36-40`) and `tree_indent`/`tree_body_prefix`/`tree_end_row`
  (`ui.rs:869-880`); tool head + body become flat rows at the gutter column.
- **Affix retired:** the `▸`/`▾`/`▣` run (`ui.rs:981`, `ui.rs:1181`) and its hit
  geometry — `note_affordance_cols` (`ui.rs:1352`), `AffordanceHit`
  (`app.rs:2199`), `AffordanceKind` (`app.rs:2189`), the `affordances` publish
  (`ui.rs:543-606`), `affordance_at` (`app.rs:3154`), `mouse_affordance`
  (`app.rs:2339`) — and the click tests (`app.rs:6680+`).
- **Thinking marker retired:** `THINK_FIRST` (`ui.rs:27`) and the `···` branch of
  `thinking_header_line` (`ui.rs:1405`); committed thinking → a plain dim
  `thinking` row, live → `thinking <t>▌`.
- **Gauge text:** `token_spans` (`ui.rs:2175`) and `bar` (`ui.rs:2205`) collapse
  to plain `used/limit` text (e.g. `14.2k/272k`) on the composer foot's left; the
  green→yellow→red branch is dropped.
- **Palette collapse (§4):** `colored()` (`theme.rs:65`) and `palette_b()`
  (`theme.rs:597`) make `error` the only chromatic role; `body`/`user` default;
  everything else `dim`.
- **Docs:** amend `docs/tui-design.md` (§1, §2, §4
  Header/Speaker/Thinking/Tools/composer-foot/Layout); add `D023`; add tracker row
  #81 in `docs/next-steps.md`.

### Non-scope (explicit)

- **The layout, bands, measure, gutter columns** — no reflow. The header band and
  composer band keep their rows; only their *fill glyph* changes.
  (`docs/tui-design.md:309-341`.)
- **The drag-select highlight** — `Modifier::REVERSED` is **not** removed
  globally; `selection_style()` (`ui.rs:651`) keeps it for `recolor_range`
  (`ui.rs:528`). Only the two borrowing call sites change. (See §3 risk, §4.2.)
- **The markdown code-fence `│`** (`markdown.rs:690`, `:704`) — added-then-flattened
  only if the human says so; default is **keep** (Open Q-b). Ditto the blockquote
  `│` (`markdown.rs:479`).
- **Keyboard parity is preserved, not extended** — browse `Enter`/`Space` expands
  and `y` copies (`docs/tui-design.md:294-298`); `Ctrl-T` toggles all tools. We
  **drop** the *mouse* toggle/copy (the D32 glyph cells) — a deliberate trade-off,
  not an oversight.
- **The 7-preset theme catalog** (`theme.rs:377`) — `dark`/`nord`/… stay coloured
  escape hatches; only `default` (= palette B) becomes bare. We do **not** add a
  `bare` preset (Open Q-f / §4.2).
- **NO_COLOR red** — `NO_COLOR` keeps meaning "no colour", so `error` degrades to
  bold, not red. (See §4.2, Open Q.) `AGENTS.md`'s "minimalism is the point" and
  `theme.rs:87` (`plain()` = "no foreground anywhere") govern.
- **Per `AGENTS.md`** — no new config knob for behaviour, no permission/approval
  surface, no MCP; the change is a *restyle*, and any policy would live in `Hooks`,
  which this is not.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| A | **Chrome** (3 rules + 2 reverse-video borrows + foot leader) | Med — the rules and inverse-video are the loudest non-prose marks on screen | Low — three `Paragraph` emits and two `Span` styles | **Med** — the two borrows share `selection_style()` with drag-select; a careless global edit silently kills text selection |
| B | **Glyph retirement** (tree + affix + `···`) | Med — removes the last structural noise around tools/thinking | Med — tree prefix fns, two head-row builders, plus a cross-file hit-geometry removal | Med — removes mouse toggle/copy; touches `app.rs` + `ui.rs` in lockstep |
| C | **Gauge text** (`token_spans`/`bar` → text) | Low — a chrome nicety; the gauge is already small | Low — one fn rewrite + one deleted helper | Low — the foot left cell is isolated; `format_tokens` stays |
| D | **Palette collapse** (`colored`/`palette_b`/`plain`) | High — this *is* "red the only colour"; the identity of the whole surface | Med — two palette fns, an explicit role table | **High** — 17 roles, per-tier values, and the largest theme-test churn; a wrong `dim` value greys the transcript to illegibility |
| E | **Docs** (spec §1/§2/§4, D023, tracker) | Med — the spec is the authority; an unamended spec drifts | Low — prose + a decision record | Low — docs-only |

---

## 4. Interface & structure

### 4.1 The target role table (the palette collapse — workstream D)

`Theme` has 17 roles (`theme.rs:22-59`). Palette B today (`colored()`
`theme.rs:65-85`, `palette_b()` `theme.rs:597-647`) assigns a distinct hue per
role. **P5 collapses every role to the grey ramp except two: `body`/`user` keep
default weight, and `error` keeps red.**

| role | palette B now | P5 bare |
|---|---|---|
| `body` | `Style::new()` | **unchanged** (default weight) |
| `user` | `Cyan + BOLD` | **default weight** — "keep the user prompt at full strength" |
| `error` | `Red` | **`Red`** — *the only colour* |
| `dim` | `Gray` | `Gray` (workhorse, unchanged) |
| `muted` | `DarkGray` | `DarkGray` (grey ramp) |
| `border` | `DarkGray` | `DarkGray` |
| `accent` | `Cyan + BOLD` | `Gray` |
| `success` | `Green` | `Gray` |
| `warn` | `LightYellow` | `Gray` |
| `code` | `Yellow` | `Gray` |
| `heading` | `Magenta + BOLD` | `Gray` |
| `heading_sub` | `Magenta` | `Gray` |
| `link` | `Blue + UNDERLINED` | `Gray` |
| `tool_name` | `Blue + BOLD` | `Gray` |
| `thinking` | `Magenta + ITALIC` | `Gray` |
| `diff_add` | `Green` | `Gray` |
| `diff_del` | `Red` | `Gray` |

Two consequences (see Open Questions): `accent()` (`ui.rs:2526`) is the style of
`❯`, the live cursor `▌`, the spinner `⠹`, and `⏻ plan`/`▤ browse` — **collapsing
the `accent` role alone** turns all of them dim without touching a single `ui.rs`
call site (minimal blast radius). And **markdown structure** (`heading`, `link`,
`code`) becomes grey — a `body`/`dim` "wall". The literal reading of the target
("everything else dim") yields this; an alternative keeps the *non-colour*
modifiers (BOLD/UNDERLINED) while dropping every `fg`. This is the single biggest
judgment call (Open Q-c).

### 4.2 The two structural decisions

**(1) Fixed default vs. selectable preset.** The bare look is **not** expressible
as a `Palette`/preset: `from_palette` (`theme.rs:538`) starts from
`Theme::colored()` and *forces a per-role `fg`*, deriving `error` from the
palette's `red` (`set(&mut theme.error, p.red)` `theme.rs:553`). A `bare` preset
would need a new "monochrome" concept on `Palette` (`theme.rs:236`) plus a branch
in `from_palette`, a `CATALOG` entry (`theme.rs:377`), and CLI/picker plumbing.

**Recommendation: make it the new fixed default.** Edit `colored()`
(`theme.rs:65`) and `palette_b()` (`theme.rs:597`) so palette B *is* the bare
palette; edit `plain()` (`theme.rs:88`) to the same shape. Consequences:
- **No new symbol.** `default` (palette B) *is* the bare look. It surfaces through
  the **existing** `[theme] name` (`config.rs:344`), `--theme <name>`
  (`main.rs:140`), `/theme` picker, and `--list-themes` — all keyed off `CATALOG`
  (`theme.rs:377`). `default` keeps its name; `--list-themes` still lists it.
- **Presets stay coloured.** `from_palette` (`theme.rs:538`) still starts from the
  (now-bare) `colored()` skeleton and overwrites the chromatic roles per role — so
  `dark`/`nord`/… remain full-colour escape hatches.
- **`[theme]` role overrides still sit on top** (`config.rs:790`,
  `ThemeSpec::into_theme` `theme.rs:767`).
- **Blast radius (theme.rs):** the tier tests at `theme.rs:1017-1059`
  (`named.accent.fg == Some(Color::Cyan)` etc.) are rewritten; `theme.rs:4324`-style
  assertions that `user != body` (`ui.rs:4324-4327`) are inverted; `plain()` and
  `no_color_wins_*` (`theme.rs:1228`, `:1306`) adjusted.

**(2) `NO_COLOR`.** `Theme::plain()` (`theme.rs:88-110`) sets **no `fg` at all**,
and `resolve` (`theme.rs:917-919`) makes it win unconditionally, so under
`NO_COLOR` "red is the only colour" is *unreachable*. **Recommendation: degrade to
bold-only and document it** — `body`/`user` plain, `error` **bold** (red
unavailable), everything else `dim`. This honours `NO_COLOR`'s contract ("no
colour") and keeps the two palettes structurally identical modulo the single
red→bold step. State the exception in `docs/tui-design.md` §1.3 the way D29
already notes "`dim` falls back to the `DIM` modifier" (`docs/tui-design.md:43`).

### 4.3 Changed draw functions (workstreams A–C)

**Chrome (A).** All three removals are of the form
`Paragraph::new(Line::from(Span::styled("─".repeat(..), dim())))`:
- Header rule — `ui.rs:440-450`: inside `if area.height >= 2`. Deliverable: band 0
  is one row (`session_head_row`), no rule; the header band's height budget drops by
  one (`HEADER_H` `ui.rs:69`).
- Composer rule — `ui.rs:1979-1991`: the `if h >= 3 { … "─".repeat(w as usize) … }`
  block. Deliverable: the composer's rule row is gone; the input seats directly
  above the foot. This changes the `h >= 3` / `input_end` arithmetic
  (`ui.rs:1979-2007`).
- Foot leader — `ui.rs:2010`: `padded_row(&bl, &br, "─", dim(), w)`. Change the
  `fill` to a space (`padded_row` already fills `fill.repeat(gap-2)` at
  `ui.rs:2043`), so the foot becomes `gauge … spaces … state`.

**Reverse-video (A).** Replace the `selection_style()` borrow at two sites with a
non-inverting style:
- `ui.rs:765` — `let mut spans = vec![Span::styled("WCODE", selection_style())];`
  → `dim()` (or a dedicated `head()` style). `WCODE` keeps its text and column.
- `ui.rs:712` — `Span::styled(" YOU".to_string(), selection_style()),` → `dim()`.
- **Leave `selection_style()` (`ui.rs:651-653`) and `recolor_range`'s call
  (`ui.rs:528`) untouched** — drag-select keeps `REVERSED`; `lib.rs:1111-1117`
  (the `sgr_params` encoder test) is unaffected and **stays green unchanged**.

**Gauge (C).** `token_spans` (`ui.rs:2175-2201`): keep `format_tokens`
(`ui.rs:2213`), drop the `bar(ratio, 8)` span and the `bar_style` ratio branch
(`ui.rs:2184-2197`); emit a single `dim()` span
`format!("{}/{}", format_tokens(used), format_tokens(limit))` (spacing = Open
Q-a). Delete `bar()` (`ui.rs:2205-2210`) and its test `bar_fills_proportionally`
(`ui.rs:2789-2794`). The call site is unchanged: `token_spans(app).unwrap_or_default()`
at `ui.rs:2114`, the foot-left of `corner_titles`' `bottom_levels` (`ui.rs:2115-2117`).

### 4.4 Retired symbols (workstream B)

| symbol | anchor | disposition |
|---|---|---|
| `TREE_BRANCH` / `TREE_PIPE` / `TREE_END` | `ui.rs:36-40` | delete; call sites `ui.rs:875`, `:880`, `:944`, `:1171` |
| `tree_indent` / `tree_body_prefix` / `tree_end_row` | `ui.rs:869-880` | delete; the flat indent is `"   "` (col 3) |
| `collapsed_head_row` affix `"▸ ▣"` | `ui.rs:981` | delete the right-run affix; head = prefix + name + params + fill + note |
| `tool_head_row` affix `format!("{glyph} ▣")` | `ui.rs:1181` (glyph at `ui.rs:1160-1164`) | delete; the `✗`/`TREE_BRANCH` mark (`ui.rs:1168-1172`) collapses to a flat prefix |
| `note_affordance_cols` | `ui.rs:1352` | delete |
| `AffordanceHit` / `AffordanceKind` | `app.rs:2199` / `app.rs:2189` | delete |
| `TranscriptHit.affordances` + publish | `app.rs:2182`, `ui.rs:542-606` | delete the field + the per-frame publish; `set_transcript_hit` (`app.rs:3117-3127`) loses its `affordances` param |
| `affordance_at` / `mouse_affordance` / click arms | `app.rs:3154`, `:2339`, `:3203-3221` | delete; keep `toggle_block`/`copy_block` (browse still calls them) |
| `THINK_FIRST` + `···` branch | `ui.rs:27`, `ui.rs:1405-1431` | delete; `thinking_header_line` returns a plain dim `thinking`, `live_thinking_line` (`ui.rs:1373`) appends the elapsed + cursor |

Everything keyed only by keyboard (`Ctrl-T`, browse `Enter`/`Space`/`y`) is **kept**.

### 4.5 Spec amendments (workstream E)

`docs/tui-design.md`, the authority:
- **§1.1/§1.2/§1.4** (`:14-49`) — restate "no `─` rules; the header is a single
  bare row"; "a tool is a flat row, not a tree".
- **§2 Vocabulary** (`:59-82`) — delete the rows `tool head ├`, `tool body/end │ └`,
  `collapsed/expanded affordance ▸ ▾`, `copy affordance ▣`, `context gauge ▰ ▱`;
  change `thinking` to a plain `thinking` word; note `WCODE`/`YOU` are no longer
  reverse-video. (Keep `│` for tables/quotes/code — Open Q-b.)
- **§4 Header** (`:215-221`) — drop "over a full-width `─` rule"; `WCODE` is plain,
  not reversed.
- **§4 Speaker head** (`:222-228`) — `YOU` is plain, not reversed.
- **§4 Thinking** (`:253-270`) — the marker and the `▸/▾/▣` affordances are retired;
  committed = a dim `thinking` row, live = `thinking <t>▌`.
- **§4 Tools** (`:235-252`) — no tree, no `▸ ▣`; params/body are flat rows.
- **§4 composer foot** (`:326-341`) — the gauge is plain `used/limit`; the `─`
  leader is gone.
- **§4 Layout** (`:309-341`) — the header is one row; the composer loses its rule
  row.
- New `docs/decisions/D023-tui-prose-only-bare.md`; tracker row #81 in
  `docs/next-steps.md`.

### 4.6 Facts **not** verified

- That the terminal renders the bare palette legibly at `truecolor`/`256`/`named-16`
  — only the headless dump can be checked here; real-terminal contrast is a human
  step (§7.2).
- The exact **body/param column** of a flat tool (col 3 vs the shared content col 7)
  — Open Q-e.
- The exact **foot gauge spacing/position** and whether the markdown **code-fence
  `│`** should flatten — Open Q-a/-b.
- Whether any **VS Code** surface shares these glyphs — assumed **no**; not verified.
- The full count of `theme.rs`/`ui.rs` tests that assert a specific `fg` (sampled;
  a mechanical sweep is a plan step).

---

## 5. Plan

Each **phase** is one logical PR/commit (imperative subject, `tui:` prefix). Each
numbered step is a deliverable + a gate within its phase. Ordering rationale: the
cheap, reversible chrome edits first, then glyph removal (which touches a second
file's geometry), then the palette identity (largest test churn, and it must be the
*last* code phase so earlier phases assert structural facts rather than colours),
then docs.

### Phase A — Chrome (rules + reverse-video + gauge text)

1. **Reverse-video borrows.** Deliverable: `WCODE` (`ui.rs:765`) and ` YOU`
   (`ui.rs:712`) render plain; `selection_style()` unchanged. Gate: the drag-select
   tests stay green; a **new** test asserts the `WCODE`/`YOU` cells carry **no**
   `REVERSED` (`reversed_cells`, `ui.rs:2619`) while a drag still produces reversed
   text cells.
2. **Rules + foot leader.** Deliverable: no full-width `─` rule in the header,
   composer, or foot; the composer's rule row is gone and its height math updated.
   Gate: `cargo run -p wcode-tui --example dump` → no pure-`─` line (a line
   composed only of `─`; the head row's own `─` leader is kept) in
   `target/tui-render/80x24.txt`; the composer/header tests (`ui.rs:4496`, `:4562`,
   `:4810`) rewritten to assert the *absence* of the rule and the surviving rows.
3. **Gauge text.** Deliverable: `token_spans` emits plain `used/limit` `dim` text;
   `bar()` deleted. Gate: `bar_fills_proportionally` (`ui.rs:2789`) deleted; a new
   unit test asserts `token_spans` text (e.g. `14.2k/272k`) and no `▰`/`▱`; the dump
   foot shows the text.

### Phase B — Glyphs (treeless tools + affix + thinking)

4. **Treeless tools.** Deliverable: `TREE_*` + `tree_*` deleted;
   `collapsed_head_row`/`tool_head_row` emit a flat prefix. Gate: tests `ui.rs:2975`,
   `:4595`, `:5326`, `:5339`, `:5520` rewritten (`├`/`│`/`└` absent, the tool name
   still present); dump shows flat tool rows.
5. **Retire the affix + D32 geometry.** Deliverable: `▸/▾/▣`,
   `note_affordance_cols`, `AffordanceHit`, `AffordanceKind`, `affordance_at`,
   `mouse_affordance`, the publish block, and `set_transcript_hit`'s `affordances`
   param deleted; browse `Enter`/`Space`/`y` and `Ctrl-T` unchanged. Gate:
   `app.rs:6680+` click tests **deleted**; a **new** browse test proves `Enter`
   expands and `y` copies a tool without any published affinity cell; `cargo clippy`
   clean (dead-code removed, not `#[allow]`ed).
6. **Retire the `···` marker.** Deliverable: committed thinking = a dim `thinking`
   row; live = `thinking <t>▌`; `THINK_FIRST` and the `···` branch deleted. Gate:
   tests `ui.rs:5696`, `:5718`, `:5747`, `:5801` rewritten (no `···`, no `▸`).

### Phase C — Palette identity

7. **Collapse `colored()`/`palette_b()`/`plain()`.** Deliverable: the §4.1 role
   table is the shipped palette B; `error` is the only `fg` colour. Gate: the tier
   tests (`theme.rs:1017-1059`), `ui.rs:4324` (`user != body`), and the `plain()`
   tests rewritten; the dump's **`.ansi`** (`cat target/tui-render/80x24.ansi`) shows
   exactly one colour (red) and only on error text; a **new** test asserts
   `theme().accent.fg == theme().error.fg` is `false` and every non-`error`
   chromatic role shares the `dim` ramp.

### Phase D — Spec, decision, tracker

8. **Docs.** Deliverable: `docs/tui-design.md` amended per §4.5;
   `docs/decisions/D023-tui-prose-only-bare.md` written; tracker row #81 added.
   Gate: `--dump-system-prompt` unaffected (no prompt change); the spec's §2 table
   has no retired glyph; a grep of `docs/tui-design.md` for `▸|▾|▣|▰|▱|···` returns
   nothing the surviving §4 does not explicitly keep.
9. **Final live verification.** Deliverable: one end-to-end read of the dump at
   three widths plus a real-terminal eyeball. Gate: §6, §7.2.

---

## 6. Quality gates

Exact commands (run from the repo root; each must pass before the commit that
closes its phase):

```
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p wcode-tui --example dump        # writes target/tui-render/{80x24,120x40,48x20}.{txt,ansi}
```

Expected output:
- `cargo test --workspace` — **all tests pass** (rewritten tests included; §7.1).
- `cargo clippy --workspace --all-targets` — **clean** (deleted symbols *deleted*,
  not `#[allow(dead_code)]`).
- `target/tui-render/80x24.txt` — **no** pure-`─` line (a line composed only of
  `─`; a head-row leader between runs is kept); **no** `├`,
  `│` (tool region), `└`, `▸`, `▾`, `▣`, `···`, `▰`, `▱`.
- `cat target/tui-render/80x24.ansi` — the only colour is **red** (SGR `31`/`38;…`)
  on error text; the `14.2k/272k` foot text is dim.

Regression tests that must **stay green** (prove the non-scope held):
- `paint_bar_replaces_the_first_visible_glyph_without_shifting` (`ui.rs:4011`).
- the drag-select tests (`ui.rs:3796`/`:3850`; `reversed_cells`, `ui.rs:2619`).
- `sgr_params_orders_modifiers_then_fg_then_bg` (`lib.rs:1105`).
- the sidebar tests (`ui.rs:4345`, `:4461`).

---

## 7. Testing

### 7.1 Automated

| existing test | anchor | action | after |
|---|---|---|---|
| `bar_fills_proportionally` | `ui.rs:2789` | **delete** | replaced by a `token_spans` text test |
| composer/header rule tests | `ui.rs:4496`, `:4562`, `:4810` | rewrite | no full-width `─`; input + foot rows survive |
| tree tests | `ui.rs:2975`, `:4595`, `:5326`, `:5339`, `:5520` | rewrite | flat tool row: no `├`/`│`/`└`, name at col 3 |
| affix/click tests | `ui.rs:5520`, `:5553`, `:5578`; `app.rs:6680+` | **delete** | replaced by a browse `Enter`/`y` test |
| thinking tests | `ui.rs:5696`, `:5718`, `:5747`, `:5801` | rewrite | plain `thinking`; live `thinking <t>▌`; no `···`/`▸`/`▣` |
| `WCODE` tests | `ui.rs:4297`, `:4554`, `:5606` | rewrite | `WCODE` present, **not** reversed |
| theme tier tests | `theme.rs:1017-1059` | rewrite | bare palette: only `error` chromatic |
| `user != body` | `ui.rs:4324-4327` | **invert** | `user == body` |

**NEW pins (one per removal):**
1. `wcode_and_you_are_not_reversed_but_selection_is`.
2. `no_full_width_rule_is_drawn`.
3. `the_gauge_is_plain_text`.
4. `a_collapsed_tool_has_no_tree_and_no_affix`.
5. `browse_enter_expands_and_y_copies_a_tool_without_affordances`.
6. `thinking_rows_are_plain_dim_lines`.
7. `only_error_carries_a_foreground_colour`.

### 7.2 How a human verifies it

```
cargo run -p wcode-tui --example dump
sed -n '1,40p' target/tui-render/80x24.txt        # no ─ ─run, ├ │ └ ▸ ▾ ▣ ··· ▰ ▱
cat        target/tui-render/80x24.ansi           # red only, on errors
```

Then a **real terminal** (`wcode --tui`): (i) the transcript reads as prose with
quiet grey chrome, (ii) a failed tool's `✗` is the only red, (iii) `14.2k/272k`
sits on the composer foot's left, (iv) `Ctrl-G` browse still nudges a `▌` bar and
`Enter` expands a tool. For `NO_COLOR`: `NO_COLOR=1 wcode --tui` — `✗` is **bold**,
not red (documented degradation).

---

## 8. Expected outcome

- `target/tui-render/*.txt` contains **no** pure-`─` line, and **no** `├`, `└`,
  `▸`, `▾`, `▣`, `···`, `▰`, `▱` in the tool/thinking/gauge regions.
- `target/tui-render/*.ansi` emits exactly **one** foreground colour (red), only on
  error text.
- `WCODE`/`YOU` render at default/dim weight with **no** `REVERSED`; a drag-select
  still reverses its cells.
- Tools render as flat rows; browse `Enter`/`Space`/`y` and `Ctrl-T` still
  expand/copy.
- `--list-themes` still lists `default`; `[theme] name = "nord"` still yields a
  fully coloured surface; `NO_COLOR=1` yields bold-not-red errors.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean;
  `docs/tui-design.md` §1/§2/§4 and `D023` match the shipped pixels.

---

## 9. References

- `docs/tui-design.md` §1 (`:14-49`), §2 (`:59-82`), §4 (`:215-341`).
- `AGENTS.md`; `README.md`; `docs/next-steps.md` (add row #81).
- Precedents P5 reverses: `D010`, `D012`, `D018`, `D019`, `D020`, `D021`.
- `docs/design/tui-editorial-directions.md` §9.
- Work item: this file. Decision: `D023`.

---

## Open questions — RESOLVED (human sign-off)

All resolved per the brief's recommendations; each is now a binding interface
decision:

1. **Foot gauge spacing/position — flush-left `14.2k/272k`.** The foot's left cell
   (`ui.rs:2114`); no spaces around the slash.
2. **Markdown code-fence `│` — keep** (`markdown.rs:690`/`:704`, blockquote `:479`).
   It is the only thing separating a code plate from prose.
3. **Markdown structure roles — drop the `fg`, keep the non-colour modifiers.**
   `heading`/`heading_sub` keep BOLD; `link` keeps UNDERLINED; `code` keeps its
   weight — but no colour.
4. **Diffs — dim both** `diff_add`/`diff_del` (the `+`/`−` glyphs suffice; needed so
   red stays errors-only).
5. **Flat tool body column — col 3.** The whole tool cluster (head + params + body)
   sits at the gutter column (`"   "`), aligned under the tool name.
6. **Fixed default** (§4.2) — edit `colored()`/`palette_b()`/`plain()`; keep
   `default` as the name; presets stay coloured escape hatches. No `bare` preset.
7. **Mouse toggle/copy loss — intended.** Retiring the affix drops mouse
   click-to-toggle/copy; keyboard parity (`Ctrl-T`, browse `Enter`/`Space`/`y`)
   is preserved.

---

## Open questions — original list (for the record)

1. **(user) foot gauge spacing/position.** Foot's **left** cell (`ui.rs:2114`).
   Candidates: **flush-left** `14.2k/272k` · **spaced** `14.2k / 272k` ·
   **right-aligned** beside the state chip.
2. **(user) markdown code-fence `│`.** `markdown.rs:690`/`:704` (blockquote `:479`).
3. **Markdown structure roles (`heading`/`link`/`code`).**
4. **Diffs (`diff_add`/`diff_del`).**
5. **Flat tool body column** (col 3 vs. col 7).
6. **Preset vs. default.**
7. **Mouse copy/toggle loss.**

---

## Outcome — shipped

Four phases, each first-layer (sketch) + second-layer (diff) reviewed and
**APPROVED**:

- **Phase A — chrome** (`312a807`): no `─` band rules (header, composer, foot
  leader); `WCODE`/` YOU` are dim, not reversed (drag-select keeps `REVERSED`);
  the gauge is plain `used/limit` text. `ui.rs` only.
- **Phase B — glyphs** (`0981bda`): treeless tools (flat col 3, stats
  right-aligned); the `▸`/`▾`/`▣` affix + its D32 mouse geometry retired
  (keyboard parity: `Ctrl-T`, browse `Enter`/`Space`/`y`); the `···` marker
  retired (committed = a dim `thinking` row, live = `thinking <t>▌`, body col 3).
- **Phase C — palette** (`b62fd0e`): the bare `default` is the grey ramp;
  **`error` red is the only colour**; `body`/`user` default weight; markdown
  keeps only non-colour modifiers (`heading` BOLD, `heading_sub`/`link`
  UNDERLINED, `code` plain); the code fence is monochrome
  (`markdown::grey_syntect_theme`); presets stay coloured escape hatches;
  `NO_COLOR` degrades `error` to bold.
- **Phase D — docs** (`eeb7042`, `14de3a8`, `497a614`): `docs/tui-design.md`
  §1/§2/§4 amended; `docs/decisions/D023-tui-prose-only-bare.md`; tracker #81.

Plus the N1 `CONTENT_COL` doc nit (`f7c13ec`).

**Gates:** `cargo test --workspace` green (un-isolated), `cargo clippy
--workspace --all-targets` clean, and the live `target/tui-render/*.ansi` carries
**no hue** (only the grey ramp).

**Related bug found en route:** W019 (`5a3ba33`) — the omarchy source now resolves
the ANSI schema too (see [W019](W019-omarchy-ansi-schema.md)).

**Released as `v0.7.0`.**

---

*End of brief — shipped.*
