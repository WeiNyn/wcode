# Tool-mark systems — three vocabularies for the tool panel's "icon"

- **Status:** draft, for a human to react to. **Not authorized** — no code is
  changed and `docs/tui-design.md` is **not** amended. A genuine pick amends that
  spec first (`design-taste` §7.3).
- **Companion to:** [`tui-editorial-directions.md`](tui-editorial-directions.md)
  — every direction there renders this same tool panel; this doc proposes the
  **marks** on it.
- **Refines:** the shipped tool panel (D31) —
  `docs/tui-design.md:192-199` and `crates/wcode-tui/src/ui.rs:702-717`.
- **Date:** 2026-02-14

---

## 0. Surface read

> Reading this as: **a terminal surface (Medium A)**, for **wcode users reading a
> tool-heavy session**, in **the locked wcode spec language**
> (`docs/tui-design.md` + `theme.rs`), with the dominant constraint that **a
> glyph must be single-cell or the panel misaligns** — the mark is partly a
> correctness fix, not only a taste one.

---

## 1. The problem — two issues, not one

**1. Reliability / width.** `⚙` **U+2699 GEAR** renders as a **colour emoji
(often double-width)** on many terminals, and `⧉` **U+29C9** has **patchy font
coverage**. The renderer counts width by `chars()` —
`crates/wcode-tui/src/markdown.rs:778-781`:

```
/// Display width (characters; wide glyphs are treated as one cell for now).
fn disp(text: &str) -> usize {
    text.chars().count()
}
```

so a double-width `⚙` is counted as **1 cell** and the panel's padding / the
right-aligned affordances **shift by a column**. `✓ ✗ ▸ ▾` are safe (common,
single-width). **This half of the change is a bug fix.**

**2. Editorial fit.** `⚙`/`✓`/`✗` are *pictographic icons*; under the editorial
stance (a tool call is *apparatus*) the marks could be **typographic**.

The shipped marks live in exactly two places:

- the **start** mark — the panel head, `crates/wcode-tui/src/ui.rs:708-710`:
  `Some(target) if params.is_empty() => format!("⚙ {}  {target}", tool.name),`
  `_ => format!("⚙ {}", tool.name),`
- the **done/error** mark — the summary row, `crates/wcode-tui/src/ui.rs:874-880`
  (`fn tool_summary_row`): `("✗", error_style())` / `("✓", success())`.

Roles: the name is `tool_name` (blue+bold, `crates/wcode-tui/src/theme.rs:79`
`tool_name: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),`); the
marks are `success`/`error` (`theme.rs:73` `success: Style::new().fg(Color::Green),`,
`:72` `error: Style::new().fg(Color::Red),`); the rest is `dim`.

**The constraint every system must meet:** `NO_COLOR` still yields `Theme::plain`
(no colour, `theme.rs:87-109`), so a *colour-only* status distinction fails
there — each system below says how it survives.

---

## 2. How to read the panels

**The panel is frameless.** These systems render the tool panel as a **rule-set
plate** — a head line + two `─` rules, **no `╭ ╮ ╰ ╯` corners and no side `│`**
(the shared move proposed in the directions doc, "The frame is a rule, not a
box"). The shipped renderer still draws the **framed D31** panel
(`crates/wcode-tui/src/ui.rs:898` `fn panel_frame`, `:942`
`Span::styled("╭─ ".to_string(), border())`); these frames show the **proposed**
frameless panel, so the two docs agree. *(This is a change from this doc's first
draft, which rendered the framed panel.)*

A real `bash` panel (params + `✓` summary) and its error variant, at 80 cols. The
head line carries the start mark and the right-aligned affordances `▸`/`⧉`
(`crates/wcode-tui/src/ui.rs:753-760` `header_affordances`); the two `─` rules
are `dim`. Frames are hand-authored (§10).

---

## 3. System A — **Keep-and-fix**

### (a) Stance

> Keep a **pictographic** mark, but make it **single-cell and emoji-proof**:
> retire `⚙` for a plain typographic run mark `»`, keep the safe `✓`/`✗`.

### (b) Rendered panels — 80×24

done:

````
   » bash                                   ▸ ⧉
   ────────────────────────────────────────────────────
   cmd  cargo test -p wcode-cli
   ────────────────────────────────────────────────────
   ✓ 41 passed · 12 lines · 9ms
````

error:

````
   » edit                                   ▸ ⧉
   ────────────────────────────────────────────────────
   path  crates/wcode-cli/src/tools/edit.rs
   ────────────────────────────────────────────────────
   ✗ E_NO_MATCH · no literal match · 3ms
````

### (c) Legend

- the head line (`» bash`): `»` + name `tool_name`, `▸`/`⧉` `dim`; the two `─` rules `dim`.
- `✓` `success`; `✗` `error`; the notes (`41 passed`, `E_NO_MATCH`) `dim`.
- `cmd`/`path` keys `dim`, values `body`.

### (d) Width / reliability verdict

| glyph | codepoint | verdict |
|---|---|---|
| `»` | U+00BB | **single-cell**, Latin-1 Supplement, **universal** coverage, emoji-safe — **new**, replaces `⚙` |
| `✓` | U+2713 | single-cell, common; **safe** (kept) |
| `✗` | U+2717 | single-cell, common; **safe** (kept) |
| `▸` `▾` | U+25B8 / U+25BE | single-cell, common; **safe** (kept) |
| `─` | U+2500 | single-cell, common; **safe** (the two rules) |
| `⧉` | U+29C9 | **at risk** — patchy coverage; **not fixed by A** (see §7) |
| `⚙` | U+2699 | **emoji-risk / often double-width** — **retired** |

### (e) §2 amendment + call site

`docs/tui-design.md:58` — the row
``| tool start | `⚙ name  args` | dim |`` becomes ``| tool start | `» name  args` | dim |``.
**New glyph `»`, retired glyph `⚙`** — one-for-one. *Call site:*
`crates/wcode-tui/src/ui.rs:709-710` (the `name_text` `format!`).

*NO_COLOR:* unchanged — `✓`/`✗` are distinct glyphs, so done/error still read
without colour.

### (f) Cost / risk

**Lowest of the three.** One glyph swap in one `format!`; the panel geometry is
unchanged and stays aligned (and is now *correct*). No behaviour change.

### (g) Open question

Is `»` the right run mark, or `›` (U+203A, narrower coverage) / `*` / `+`? (The
choice is cosmetic; `»` is chosen for the widest font coverage.)

---

## 4. System B — **Typographic apparatus**

### (a) Stance

> Drop the gear entirely. A tool call is a **printed reference**: the **frame is
> the mark**, and status is carried by a **word**, never an icon.

### (b) Rendered panels — 80×24

running (in flight):

````
   bash                                     ▸ ⧉
   ────────────────────────────────────────────────────
   cmd   cargo test -p wcode-cli
   ────────────────────────────────────────────────────
   run   41 tests · 1.2s…
````

done:

````
   bash                                     ▸ ⧉
   ────────────────────────────────────────────────────
   cmd   cargo test -p wcode-cli
   ────────────────────────────────────────────────────
   done  41 passed · 12 lines · 9ms
````

error:

````
   edit                                     ▸ ⧉
   ────────────────────────────────────────────────────
   path  crates/wcode-cli/src/tools/edit.rs
   ────────────────────────────────────────────────────
   fail  E_NO_MATCH · no literal match · 3ms
````

### (c) Legend

- the head line (`bash`): name `tool_name` (no start mark), `▸`/`⧉` `dim`; the two `─` rules `dim`.
- `run` `accent`; `done` `success`; `fail` `error` — the **word** is the mark.
- keys `dim`, values `body`; the running `⠹` spinner (if animated) — `accent`
  (`docs/tui-design.md:65` `| spinner (running) | `⠋⠙⠹⠸…` | accent |`).

### (d) Width / reliability verdict

| mark | glyph | verdict |
|---|---|---|
| run mark | *(none — the head line)* | the head line + `─` rule is the mark: single-cell, `dim` |
| `─` | U+2500 | single-cell, common; **safe** (the two rules) |
| status `run`/`done`/`fail` | ASCII words | **single-cell, universal** — no font risk at all |
| `▸` `▾` `⠹` | U+25B8 / U+25BE / Braille | single-cell, common; **safe** (kept) |
| `⧉` | U+29C9 | **at risk** — see §7 |
| `⚙` `✓` `✗` | U+2699 / U+2713 / U+2717 | **retired** as tool marks |

### (e) §2 amendment + call site

`docs/tui-design.md:58-60` — the three tool rows: the start row loses its mark
(``| tool start | `name  args` | dim |``), and done/error become **status words**
in the summary row, not `✓`/`✗`. **Retires `⚙` (start) and `✓`/`✗` (as tool
marks); adds no glyph.** *Call sites:* `crates/wcode-tui/src/ui.rs:709-710` (drop
the `⚙`) and `:876-880` (the word replaces the `("✗",…)`/`("✓",…)` tuple).

*NO_COLOR:* the **words** (`run`/`done`/`fail`) are text — they distinguish
status with no colour at all. This is B's strength.

### (f) Cost / risk

**Lowest inventory change** — retires three glyphs and adds none (the skill's
"retires … and reuses existing marks is cheapest"). Risk: a word is **more ink**
than a mark and slightly less scannable; and `done`/`fail` in a fixed gutter is a
small new layout cost (the summary row keeps its columns).

### (g) Open question

`done`/`run`/`fail`, or shorter `ok`/`…`/`err`? (Shorter keeps the row tighter;
`fail` is clearer than `err`.)

---

## 5. System C — **Bracketed clause**

### (a) Stance

> The apparatus is a **bracketed clause** — `[bash]` — and status is
> **colour-first** (`success`/`error`), so the reading text stays quiet; under
> `NO_COLOR` it falls back to `✓`/`✗`.

### (b) Rendered panels — 80×24

done:

````
   [bash]                                   ▸ ⧉
   ────────────────────────────────────────────────────
   cmd  cargo test -p wcode-cli
   ────────────────────────────────────────────────────
   41 passed · 12 lines · 9ms
````

error:

````
   [edit]                                   ▸ ⧉
   ────────────────────────────────────────────────────
   path  crates/wcode-cli/src/tools/edit.rs
   ────────────────────────────────────────────────────
   E_NO_MATCH · no literal match · 3ms
````

### (c) Legend

- the head line (`[bash]`): the **brackets** `[` `]` carry the status colour —
  `success` (done) / `error` (failed) / `accent` (running); `bash` `tool_name`.
- the summary row has **no mark** (the name's colour is the only status signal);
  notes `dim`; `▸`/`⧉` `dim`.

### (d) Width / reliability verdict

| mark | glyph | verdict |
|---|---|---|
| `[` `]` | U+005B / U+005D | **ASCII, single-cell, universal** — already used in chrome |
| `▸` `▾` | U+25B8 / U+25BE | single-cell, common; **safe** (kept) |
| `⧉` | U+29C9 | **at risk** — see §7 |
| `⚙` | U+2699 | **retired** (the `✓`/`✗` stay only as the `NO_COLOR` fallback) |

### (e) §2 amendment + call site

`docs/tui-design.md:58-60` — the start row becomes ``| tool start | `[name]` |
tool_name, brackets by status |``; done/error lose their mark (colour-only).
**Adds no glyph** — `[`/`]` are ASCII and already used in shipped chrome
(`docs/tui-design.md:250` "`[⏻ plan] · [▤ browse] · ⏸ idle`"). *Call sites:*
`crates/wcode-tui/src/ui.rs:709-710` (wrap the name) and `:876-880` (drop the
mark).

*NO_COLOR:* colour is gone there, so C **falls back to `✓`/`✗`** in the summary
row — exactly the shipped `Theme::plain` pattern (`theme.rs:87-109`, where colour
roles degrade to bold/DIM). **This is C's cost**: its default is colour-only.

### (f) Cost / risk

**Highest.** A colour-only status is a **WCAG 1.4.1 (use of colour) risk** — it
needs the `✓`/`✗` fallback under `NO_COLOR` *and* ideally under any low-colour /
`ansi16` case, not just `NO_COLOR`. It is the most "editorial" (the reading text
is quietest) and the most fragile.

### (g) Open question

Should the fallback (`✓`/`✗` under `NO_COLOR`) also fire under a **reduced-colour**
or `ansi16` terminal, so the colour-only default never becomes the only signal?

---

## 6. Comparison table + recommendation

| system | run mark | done | error | running | new glyph | retires | `NO_COLOR` survives by | risk |
|---|---|---|---|---|---|---|---|---|
| **A · Keep-and-fix** | `»` | `✓` | `✗` | `⠹` | `»` | `⚙` | glyph (`✓`/`✗`) | **low** |
| **B · Typographic** | *(head line)* | word | word | word | **none** | `⚙ ✓ ✗` | the word | low–med |
| **C · Bracketed** | `[name]` | colour | colour | colour | **none** | `⚙` | falls back to `✓`/`✗` | med–high |

**Recommendation: System A (Keep-and-fix).** In the skill's terms:

- **It is the smallest change that fixes a *correctness* bug.** The task's first
  problem is that `⚙` double-widths and `disp` counts 1 cell
  (`markdown.rs:778-781`) — A retires the one unsound glyph and introduces one
  **universally covered** single-cell mark (`»`), so the panel is now aligned by
  construction. That is a *fix*, not a restyle.
- **It keeps a mark** (scannable at a glance) and touches nothing else: `✓`/`✗`
  are already safe, and A needs **one** §2 row to change.
- **Runner-up: B** is the *cheapest inventory* change (retires three glyphs, adds
  none — the skill's "reuses existing marks is cheapest") **and** the more
  editorial end-state; if the human accepts status **words**, pick B instead of A
  — it fixes the same bug and needs no new glyph.
- **C is the most editorial and the most fragile** — recommend it only with the
  `NO_COLOR`/low-colour fallback made explicit.

---

## 7. The shared `⧉` fix (orthogonal to A/B/C)

`⧉` **U+29C9** is the copy affordance (§2 `docs/tui-design.md:72`
``| copy affordance | `⧉` | dim |``; call site `crates/wcode-tui/src/ui.rs:758`
`copy: Span::styled("⧉".to_string(), dim()),`) and has **patchy font coverage** —
a separate bug from `⚙`. Proposed: retire `⧉` for a widely covered single-width
mark, `▣` (U+25A3, Geometric Shapes), with the same call site. This is a **shared
fix** every system inherits; it is a §2 amendment too (`:72`, one-for-one).
With the frame gone (the directions doc §11) the copy cell rides the **head
line**, not a framed top rule.

---

## 8. Spec-amendment list (`docs/tui-design.md` §2)

Every line names the row and the delta. Amended **first** on a pick; **not**
amended here.

- **A** — `:58` `⚙ name  args` → `» name  args`; **new `»`**, retire `⚙`.
- **B** — `:58-60` — the start row drops its mark; done/error become status
  words; **retire `⚙ ✓ ✗`** as tool marks; add nothing.
- **C** — `:58-60` — the start row becomes `[name]` (brackets carry status);
  done/error lose their mark. **Add no glyph** (`[`/`]` are ASCII and already
  used, `:250`).
- **Frame (shared)** — `:69` is **re-scoped** to the `box / overlay frame` row
  (the frame glyphs **stay declared** — the input box + overlays still draw
  them), and a **new row** `| tool plate rule | `─` | dim |` declares the tool
  plate; **no glyph retired, no glyph added** (the directions doc §11).
- **Shared** — `:72` `⧉` → `▣`; **retire `⧉`**, add `▣` (one-for-one).

*(The `✓ ✗` **team-state** row `:75` is untouched by all three — only the *tool*
rows change.)*

---

## 9. Pre-flight (`design-taste` §8, checked)

| box | status |
|---|---|
| exactly three bands, no title bar | ✓ — the marks live **inside** the tool panel; no band changes |
| every glyph from the locked table (or a §2 amendment) | ✓ — A adds `»`, shared adds `▣`, each with a call site; B/C add none |
| palette roles unchanged, ≤1 accent, new role has a call site | ✓ — **no new role**; `tool_name`/`success`/`error`/`dim` only |
| hex only under `Rgb`; `NO_COLOR` still distinguishes status | ✓ — A/B by glyph/word; C by its `✓`/`✗` fallback (§5e) |
| the tool panel is frameless | ✓ — the plate uses `─` rules; the frame glyphs are **re-scoped** to the box/overlays (no glyph retired, directions §11) |
| legible at 80 cols | ✓ — every panel is 80 cols wide |
| every interactive thing has a key + a keymap entry | ✓ — `▸`/`▾`/`⧉` keep their shipped keys (`Ctrl-T`, browse `Enter`/`y`) |

---

## 10. What I did NOT verify (honest)

- **No terminal was run, and no glyph width was measured.** The claim that `⚙` is
  double-width on "many terminals" is the **premise** (a known emoji-presentation
  behaviour), grounded in the *code* fact that `disp` counts `chars()`
  (`crates/wcode-tui/src/markdown.rs:778-781`) — **not** a measurement I took.
  `⧉`'s coverage is likewise asserted from general font knowledge, not audited.
- **The panels are hand-authored**, not `TestBackend` snapshots; column offsets
  are approximate.
- **The frameless re-render is a change from this doc's first draft.** The
  frames here originally showed the framed D31 panel; they now show the
  **frameless rule-plate** (the directions doc §11), which is **not** the
  shipped renderer.
- **No code was compiled or tested.** The call sites (`ui.rs:709-710`, `:876-880`,
  `:758`) were **read**, not changed.
- **`docs/tui-design.md` was not amended**, and nothing outside `docs/` was
  touched.

---

## 11. References

- [`docs/tui-design.md`](../tui-design.md) — §2 the glyph table (`:50-79`) and §4
  the tool panel (D31).
- [`crates/wcode-tui/src/ui.rs`](../../crates/wcode-tui/src/ui.rs) — the tool
  panel (`:702-717`), the summary row (`:874-892`), the affordances (`:753-760`).
- [`crates/wcode-tui/src/theme.rs`](../../crates/wcode-tui/src/theme.rs) — the
  `tool_name`/`success`/`error`/`dim` roles.
- [`tui-editorial-directions.md`](tui-editorial-directions.md) — the directions
  that render this panel.
