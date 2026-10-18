# W022 — TUI: informational `/`-commands open a popup overlay

- **Status:** ☑ **landed** — `42efa37` (feature) + `08a0298` (N1 fold); first-layer APPROVED (8 amendments), second-layer APPROVED; zero `SKETCH` markers; workspace tests + clippy clean. Live TUI session not run (no reachable endpoint).
- **Work item:** W022 (index row 52 in `docs/next-steps.md`)
- **Decisions:** amends `docs/tui-design.md` D018; recorded as `docs/decisions/D026-tui-info-modals.md`
- **Author:** `agent:brainstormer` (recon by `agent:explorer`)
- **Date:** brief complete; anchors re-verified in-tree (read-only — no build, no test run)

## 1. The ask

The user: *"there's one improvement I need in wcode cli such that an information
command should show a popup similar to F1 (`/help`, `/usage`, `/effort` etc)."*
Today the informational `/`-commands print an inline `Block::Transient` line (the
command's echo + its reply) into the transcript, retired at the next submit. The
user wants those replies to open a **popup overlay like the `F1` keymap modal**.
TUI-only (`crates/wcode-tui`); the line REPL (`crates/wcode-cli/src/repl.rs`) is
untouched.

## 2. Scope / Non-scope

**In scope.**

- A new overlay variant that renders pre-built text in a centered, `Clear`ed,
  rounded-border modal — the visual twin of the `F1` keymap.
- Rewiring exactly **five** `/`-commands to open it instead of pushing a
  transient reply: **`/help`, `/verify`, `/usage`, `/tasks`, `/team roster`**
  (see Open questions, Q1).
- Scroll for the modal (↑/↓, PgUp/PgDn, wheel) with a `— N more —` affordance,
  because `/help` (~30 lines) and `/verify` (unbounded) overflow a 24-row
  terminal.
- The `Esc`-to-close key path and its render.
- The designer-owned spec amendment to `docs/tui-design.md` (§4 D018 + §4 **Keys**).
- Test updates for the five moved commands + new tests for the variant.

**Non-scope.**

- **Bare usage/error hints** (`/effort` no-arg `app.rs:3583`, `/width`
  no-arg/bad `app.rs:3590-3594`, `/btw` no-arg `app.rs:3605`, `/plan` bad-arg
  `app.rs:3614`, `/reload` bad-arg `app.rs:3631`, unknown-command `app.rs:3546`)
  — hints, not reports; a full-screen modal is the wrong weight. They stay lines.
  (An ` info ` modal aggregating model/effort/width/plan is a *separate* design.)
- **Action confirmations** (`theme: <name>` `app.rs:3568`, `width: N cols`
  `app.rs:3588`, `copied N chars`) — they confirm an action, not report state.
- **The `F1` modal itself** (`Overlay::Help`) — kept byte-identical; content is
  `KEYS` (`app.rs:870`), not command text. Giving `F1` scroll is a follow-up.
- **The REPL** (`repl.rs`) and the **VS Code surface** (`editors/vscode`).
- **`/changes`, `/resume`, `/model`, `/theme` bare, `/surface`, `/team` bare** —
  already open **pickers** (`Overlay::Pick`); unchanged.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | Spec amendment (`docs/tui-design.md` §4 D018 + Keys) | high | low | low | The spec **currently specifies** these replies as transient notices (`tui-design.md:208-222`); code must not lead the spec. Designer-owned, docs-only. |
| P1 | `Overlay::Info` variant + open/close/scroll + `draw_info` | high | medium | medium | One enum variant, one key arm, one render fn refactored from `draw_help`; the scroll clamp is the only fiddly part (needs a stored terminal height). |
| P2 | Rewire the 4 sync commands (`run_command`) | high | low | low | Mechanical: replace `self.transient(<text>)` with `self.open_info(title, <text>)`. |
| P3 | Async `/usage` popup (`render_usage` + a pending flag) | medium | medium | medium | `render_usage` fires from an out-of-frame `AgentEvent::History` (`app.rs:1771`); the flag guards a reply the user didn't ask for. |
| P4 | Tests + `KEYS` + live verification | high | medium | low | ~6 new + ~6 updated tests; the F1-help and transient-twin regressions must stay green. |

## 4. Interface & structure

**Added.**

- `Overlay::Info { title: String, text: String, scroll: u16 }` — a new variant of
  `Overlay` (`crates/wcode-tui/src/app.rs:431-436`). `Overlay` derives
  `#[derive(Clone, Debug)]`, so no new bound.
- `App::open_info(&mut self, title: impl Into<String>, text: impl Into<String>)`
  — the twin of `open_help` (`app.rs:2516-2520`).
- `App::info_scroll(&mut self, delta: isize)` — clamp against
  (text lines − visible rows).
- A `terminal_height: u16` field beside `terminal_width` (`app.rs:2244`) — the
  overlay's clamp needs the draw height, not stored today.
- `ui::draw_info(frame, area, title, text, scroll)` — a `Clear`ed, centered,
  rounded-border modal rendering `text.lines()` from `scroll`, with an
  `esc to close` footer and a `— N more —` hint when clipped.
- A `usage_pending: bool` per-surface field (next to `btw_pending`,
  `app.rs:1194`).

**Changed.**

- `Overlay` doc-comment (`app.rs:427-436`).
- `run_command` arms (`app.rs:3552-3650`): `"verify"` (`:3624`), `"team"
  Some("roster")` (`:3636`), `"tasks"` (`:3639`), `"help"` (`:3641`) each become
  `self.open_info("<title>", <same text>)`; `"usage"` (`:3625`) gains
  `usage_pending = true`.
- `render_usage` (`app.rs:1904-1921`) — opens the `usage` modal when
  `usage_pending`, else keeps the transient (defensive). Still sets
  `self.context_used` unconditionally.
- `on_overlay_key` (`app.rs:4217-4246`) — add an `Info` arm (Esc closes;
  ↑/↓/PgUp/PgDn/wheel scroll; everything else swallowed).
- `draw_overlay` dispatch (`ui.rs:225-232`) — add `Some(Overlay::Info{..})`.
- `KEYS` (`app.rs:870-897`) — add the modal-scroll chords; `/help` prints `KEYS`.
- `command()` (`app.rs:3543`) — the echoed `/<cmd>` line; keep per Q4.

**Integration points** (line numbers move if the line changes).

- `app.rs:431-436` — `pub enum Overlay { Pick(Picker), Help }`.
- `app.rs:2236` — `overlay: Option<Overlay>`; accessor `app.rs:4309`.
- `app.rs:2517-2520` — `open_help` (the pattern `open_info` mirrors).
- `app.rs:3356` — `if self.overlay.is_some() { self.on_overlay_key(key); return; }`.
- `app.rs:4223` — the `Overlay::Help` close logic.
- `ui.rs:167` — `draw_overlay(frame, full, app);` drawn **last**.
- `ui.rs:225-232` — the `draw_overlay` match.
- `ui.rs:234-266` — `draw_help` (width `area.width-4 clamp 64`, title `" keys "`).
- `app.rs:1904-1921` — `render_usage` → `push_transient(line)` (`:1920`).
- `app.rs:1771-1774` — `AgentEvent::History` → `render_usage` (only caller).
- `lib.rs:438-446` — `replay_history` seeds via `seed_history` (does **not** call
  `render_usage`) — the startup `GetHistory` cannot misfire the popup.
- `app.rs:3624/3636/3639/3641`, `app.rs:3625` — the five command arms.
- `app.rs:899-916`, `:3808-3846`, `:738-765`, `:720-734` — the body builders.
- `app.rs:2244` — `terminal_width: u16` (the height field to mirror).

## 5. Plan

1. **Spec amendment (designer, first).** `docs/tui-design.md` §4 **Notices** no
   longer lists `/help`, `/usage`, `/verify`, `/tasks`, `/team roster` as
   transient chatter; a new **"Information modals"** bullet describes the
   `Overlay::Info` (title, body, scroll, `Esc` to close); §4 **Keys** gains the
   chords; a `D0NN` record. Gate: human review of the doc diff; **no code lands
   before this**.
2. **`Overlay::Info` + open/close/scroll.** The variant, `open_info`, the
   `on_overlay_key` `Info` arm, `terminal_height`, `draw_info`, the
   `draw_overlay` arm. Gate: `cargo test -p wcode-tui` — `open_info` sets the
   overlay, `Esc` clears it, ↑/↓ scroll clamps; `cargo clippy` clean.
3. **Rewire the 4 sync commands.** `/help`, `/verify`, `/tasks`, `/team roster`
   open the modal, no transient reply. Gate: new `an_info_command_opens_a_popup`;
   updated `help_lists_every_command_and_the_keymap_from_the_tables`.
4. **Async `/usage`.** `usage_pending` + `render_usage` opens the modal. Gate:
   updated `a_history_reply_updates_usage`, `a_history_reply_without_usage_reports_none`,
   the W014 §7.2 script.
5. **`KEYS` + footer + tests + live check.** `KEYS` carries the scroll chords;
   `the_overlays_use_rounded_borders` covers the Info modal; a human runs the TUI.
   Gate: `cargo test --workspace` + `cargo clippy --workspace --all-targets` + the
   live session.

## 6. Quality gates

- `cargo test --workspace` → all pass.
- `cargo clippy --workspace --all-targets` → clean.
- Live: a keyless endpoint, open each modal.
- Regressions kept green: `f1_renders_the_help_overlay_with_the_keymap`
  (`ui.rs:3462`), `f1_opens_the_help_overlay_and_only_esc_or_f1_closes_it`
  (`app.rs:7310`), `a_mouse_click_under_an_overlay_is_a_no_op` (`ui.rs:3794`),
  `a_transient_row_renders_exactly_like_a_notice_row` (`ui.rs:3973`),
  `the_overlays_use_rounded_borders` (`ui.rs:4628`), `slash_commands_emit_actions`
  (`app.rs:5475`).
- Regressions that **must change**: `help_lists_every_command_and_the_keymap_from_the_tables`
  (`app.rs:6951`), `a_command_notice_is_retired_by_the_next_submit` (`app.rs:4759`),
  `a_history_reply_updates_usage` (`app.rs:5567`),
  `a_history_reply_without_usage_reports_none` (`app.rs:5597`), and the
  verify/team/tasks assertions at `app.rs:5111`, `:5133`, `:5147`, `:6211`,
  `:6290`, `:6407`.

## 7. Testing

### 7.1 Automated

- `an_info_command_opens_a_popup` — each of `/help`, `/verify`, `/tasks`,
  `/team roster`: `overlay()` is `Some(Overlay::Info{..})`; the last transcript
  block is not a reply transient.
- `esc_closes_the_info_modal_and_nothing_else_does` — a `Char` is swallowed and
  leaves `input()` empty; `Esc` clears the overlay.
- `the_info_modal_scrolls_and_clamps` — a body longer than the visible rows: ↓
  moves the offset, a run of ↓ clamps at the end, ↑ returns.
- `a_usage_reply_opens_the_usage_modal` — `submit("/usage")` sets `usage_pending`;
  a subsequent `AgentEvent::History` opens `Overlay::Info{"usage"}` and still sets
  `context_used`.
- `a_stray_history_reply_does_not_open_a_modal` — a `History` without a pending
  `/usage` leaves the overlay `None`.
- `info_modals_render_rounded_borders` — extend `the_overlays_use_rounded_borders`.

### 7.2 Human verification

```sh
cargo run -p wcode-cli -- --tui --base-url http://localhost:11434/v1 --model llama3
```

- `/help` ⏎ → a centered ` help ` modal; the transcript is not flooded.
- `/verify` ⏎ → a ` verify ` modal; if long, ↓/PgDn scrolls; a `— N more —` hint.
- `/usage` ⏎ → after the async reply, a ` usage ` modal (not a bare line).
- `/tasks` ⏎ and `/team roster` ⏎ → ` tasks ` / ` team ` modals.
- `Esc` → each closes; the transcript is intact; `↑ N` on the composer unaffected.
- `F1` → the keymap modal is unchanged.
- (No endpoint needed for the modal shapes; `/usage` needs `GetHistory` answered.)

## 8. Expected outcome

- `/help`, `/verify`, `/usage`, `/tasks`, `/team roster` open a centered
  rounded-border popup; none pushes a multi-line reply into the transcript.
- The popup scrolls on overflow (↑/↓, PgUp/PgDn, wheel) with a `— N more —` hint;
  `Esc` closes it; every other key is swallowed.
- `F1` still opens the ` keys ` modal, dismissed only by `Esc`/`F1` (unchanged).
- Bare usage/error hints and action confirmations still print as transient lines.
- `docs/tui-design.md` §4 D018 + Keys describe the modals; a `D0NN` records it.
- `cargo test --workspace` + `cargo clippy --workspace --all-targets` clean.

## 9. Open questions (recommendations)

**Q1 — Which commands?** Recommend `{ /help, /verify, /usage, /tasks, /team roster }`.
`/help` is *the* information command; `/verify`, `/tasks`, `/team roster` are
multi-line reports. **Keep `/effort` bare, `/width` bare, and the plain
usage/error hints as lines** — they are hints, not reports, and the current
effort/width already ride the header/foot
(`model · effort`, §1:21). `/usage` is worth the pending-flag complexity.

**Q2 — Dismiss keys.** Recommend `Esc` only (mirrors F1's rationale,
`app.rs:4217-4220`). Render the affordance as a body footer ` esc to close`, not
a new `KEYS` row — `Esc` is already in `KEYS` (`app.rs:895`). Do **not** add `q`.

**Q3 — Scrolling.** Yes — `draw_help` today silently clips on a short terminal;
`/help` and `/verify` overflow more. `scroll: u16` in the variant; ↑/↓ = 1 line,
PgUp/PgDn = a page, wheel = 3 lines; a stored `terminal_height`; a `— N more —`
footer. Add the scroll chords to `KEYS`. F1 getting scroll is a non-scope follow-up.

**Q4 — The echoed `/<cmd>` line.** Recommend **keep** it — pushed generically in
`command()` (`app.rs:3543`) and retired at the next submit, so it stays clean.
(Dropping it only for the popup set is the alternative.)

**Q5 — Title.** Recommend the lowercase command name, matching `" keys "`:
` help `, ` usage `, ` verify `, ` tasks `, ` team `.

**Q6 — Reuse vs new type.** Recommend a new variant `Overlay::Info{ title, text,
scroll }`, keeping `Overlay::Help` for `F1` (different content sources). Refactor
the *panel* into one helper `draw_info_panel(...)`; let `draw_help` become a thin
caller so F1's bytes stay identical.

**Q7 — Spec amendment.** Confirmed: the designer edits `docs/tui-design.md`
first (D018 at `:208-222` specifies these as transient notices). Minimal change:
(a) drop those five from the §4 Notices chatter list (keep `/copy`, the
`a turn is already running` guard, the usage/error hints); (b) add a §4
**"Information modals"** bullet; (c) add one sentence to §4 **Keys**; §2's glyph
table (line 84) already covers the overlay frame — no glyph change. Record a `D0NN`.

## 10. Facts not verified

- No build, no test run, no live binary — every behavior claim is from source.
- The startup `GetHistory` "cannot fire the popup" — read from `replay_history`
  (`lib.rs:438-446`) + the single `AgentEvent::History` route (`app.rs:1771`),
  not exercised (the `usage_pending` flag is the hedge).
- Exact line counts of `help_text()` / `verify_text()` output (read, not measured).
- Centered-modal geometry on very small terminals (≤10 rows) — not traced.
- The long tail of `transcript().last()` assertions for the moved commands.
- `GetHistory` round-trip timing (`lib.rs:441`).
- The next free `D0NN` id (designer owns it).
