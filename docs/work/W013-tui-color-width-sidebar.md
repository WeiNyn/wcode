# W013 — TUI: colour, an adjustable measure, and a book-outline sidebar

- **Status:** implemented — 5 commits + D011; second-layer review pending
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §1.3, §4
- **Recorded as:** [D011](../decisions/D011-tui-width-color-sidebar.md)
- **Design source:** the four follow-ups agreed in-session

---

## 1. The ask

Five things the human named after W012:
1. **Tool input/output** should have syntax highlighting.
2. The **user and bot messages shared one colour** — hard to tell apart.
3. The **sidebar** is out of style; make it **a book/paper outline**.
4. An **option to increase/decrease the transcript width**.

## 2. Scope / Non-scope

### 2.1 In scope
- **User prompt in the accent role** (`Theme.user`), assistant in `body`.
- **`/width <cols>` + `Alt-[` / `Alt-]`** — a runtime measure (default 68).
- **256-tier highlighting** — map syntect RGB to xterm-256 (cube / gray ramp).
- **Tool highlighting** — a file tool's body/params by extension, `bash`'s
  `command` as shell.
- **A book-outline sidebar** — ruled `── label ──` section headers.

### 2.2 Non-scope
- The kernel (untouched).
- New glyphs (none added).
- The colour ladder's structure (only the 256 tier's *mapping* is filled in).

## 3. Interface (integration points)

- Measure: `ui::draw_transcript` (`MEASURE_MAX` removed), `App.measure`, the
  `/width` command arm, the `Alt-[`/`Alt-]` key arms, `KEYS`.
- Highlight: `markdown::CodeHighlight` (+ `rgb_to_ansi256`), `ui::body_rows`,
  `ui::param_row`/`panel_param_lines`, `ui::wrap_styled`.
- Sidebar: `ui::section_rule` + `draw_sidebar`.
- Colour: `ui::user()` + `speaker_lines`.

## 4. Plan (as shipped)

1. `e9b0f63` — the user prompt in the accent role.
2. `f43a118` — an adjustable transcript width (`/width` + `Alt-[` / `Alt-]`).
3. `805bd44` — colorize code under the 256 tier.
4. `219cabc` — syntax-highlight a tool's input and output.
5. `f6519ae` — a book-outline sidebar (ruled section headers).

## 5. Quality gates

```
cargo test --workspace
cargo clippy --workspace --all-targets
```
Plus the headless frames (`cargo run -p wcode-tui --example dump`) for columns
and colour.

## 6. Expected outcome

A prompt in the accent; a measure the user can widen (`/width 100`); code
colorized on a 256-colour terminal; a tool's file body and `bash` command in
colour; a sidebar that reads as an outline:

```
── agents ────────────────────
  1 ● explorer   read a.rs
  2 ○ developer
(blank)
── changes  3 files  +10 −2 ──
   crates/wcode-tui/src/
    ├─ ui.rs    +8 −0
    └─ app.rs   +2 −1
```
