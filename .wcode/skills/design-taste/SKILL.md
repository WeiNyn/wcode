---
name: design-taste
description: "Use when building or restyling any user-facing screen of wcode's two UI surfaces — the terminal TUI (docs/tui-design.md + theme.rs) or the web page (index.html/style.css). Read the brief, set three dials, run the pre-flight. Not for prose docs or the kernel."
---

# Design taste

A design skill for wcode's **two UI surfaces** — the terminal TUI and the static
site. It reads the brief, sets three dials, enforces whatever truth the surface
already has, and runs a pre-flight before a screen is called done. It
**enforces** the locked TUI spec (`docs/tui-design.md` + `crates/wcode-tui/src/theme.rs`)
and the site's existing tokens (`style.css`); it redesigns neither.

## §0 — Surface read (before anything)

Emit **one line**, then proceed:

```
Reading this as: <medium: a terminal surface | a web screen>, for <who>, in the
<locked wcode spec | the site's dark-terminal tokens | <named system>> language,
with <the one dominant constraint — density at 80×24, or zero external requests>.
```

- **Name the surface first.** Medium **A** = terminal (`crates/wcode-tui`);
  **B** = web (`index.html`/`style.css`, or a greenfield page). The two rule sets
  are disjoint; picking the wrong one is the failure mode.
- **Ambiguous brief → ask exactly ONE question**, never a battery, and only when
  the medium genuinely diverges.
- **Anti-default** — reach past the LLM defaults: a centered hero over a mesh,
  three equal feature cards, AI-purple gradients, an em-dash flourish, a new
  accent color. The likeliest default follows the medium — for a terminal it is
  a web-dashboard layout or a borrowed glyph; for a web screen it is the centered
  hero over a mesh gradient with a row of three identical cards. Reach past both.

## §1 — The three dials

`DESIGN_VARIANCE` (1 symmetry → 10 chaos) · `MOTION_INTENSITY` (1 static → 10
cinematic) · `VISUAL_DENSITY` (1 airy → 10 packed). Infer them from §0; never ask
for them. **The TUI reading of each dial is not the web reading.**

| brief signal | VARIANCE | MOTION | DENSITY |
|---|---|---|---|
| TUI — any surface (the spec is the design) | **1, fixed** | **1–2** | 7–8 |
| TUI — a new region (sidebar/overlay) | **1, fixed** | 1–2 | 8–9 |
| web — the wcode site | 5–6 | 3–4 | 4–5 |
| web — greenfield marketing | 6–8 | 4–6 | 3–5 |

- **The TUI dials are pinned**: `VARIANCE` 1 (the spec is the design), `MOTION`
  ≤2 (the spinner `⠋⠙⠹⠸` and the live cursor, both gone under
  `NO_COLOR`/reduced-motion), `DENSITY` high (an 80×24 grid, legible to 48 cols).
  A reading that implies inventing a layout is a misread — the TUI has no layout
  freedom left to spend.

## §2 — Where the truth already lives

Every rule below points at a source. Pick the source before styling; a rule with
no source is a rule not to make.

- **A. TUI → `docs/tui-design.md`.** The spec is agreed and implemented through
  P3 (`docs/tui-design.md:Mgxqe`); its locked-decisions list is `:npxul`. §3
  cites and enforces it; a genuine redesign must **first amend that spec**, never
  the skill.
- **B. The site → `style.css`'s custom properties.** There is **no** design doc
  for the site: `index.html` + `style.css` are the spec (`style.css:FGqrn`,
  `:V3G80`). §4 derives from those tokens; writing them down is this skill's first
  act of authority on the site.
- **C. Greenfield → install a real system, or label the aesthetic honestly.** A
  named system (Primer, GOV.UK, Material, …) means its official package, not a
  re-draw; one system per project.

## §3 — Medium A rules (terminal) — enforcement, not invention

- **Three bands, always** — transcript (flex) → input → status; there is no
  fourth (`docs/tui-design.md:W4SAR`). An overlay never reflows the base.
- **Gutter**: a 1-col margin, a marker column, content at a fixed column; wrapped
  continuation lines align under the content (`:zwmWX`).
- **Glyphs come from the locked table only** (`docs/tui-design.md:34-50`,
  `:pMZOm`): ``❯ ··· ⚙ ✓ ✗ ⋯ ▌ ⠋ ▤ · ` │ ─ ┼ █░``. Inventing a glyph is a fail.
- **Palette = the 17 named roles in `theme.rs`** (`crates/wcode-tui/src/theme.rs:3dRHq`);
  a new role needs a real call site. Named ANSI only (the palette-B default);
  **≤1 accent** (`accent`, cyan).
- The **color-mode ladder is implemented and invariant**
  (`crates/wcode-tui/src/theme.rs:7wN6X`, `:QNCsA`): `NO_COLOR`/`TERM=dumb` →
  `Plain`; truecolor `COLORTERM` → `Rgb`; `256color` → `Indexed`; else `Named`.
  Hex is honored **only** under `Rgb` (`:IWs97`); elsewhere it degrades to that
  role's palette-B default. (Landed as T2 — `docs/next-steps.md:zft6w`.)
- **The open item is the tier palettes, not the ladder**: the concrete
  Basic-8/256/truecolor colors are still open (`docs/next-steps.md:C4lUT`). Cite
  them; never decide their colors. Do **not** cite `docs/tui-design.md:4AL6e` as
  current — that NO_COLOR line's "detection is still open" neighbour (`:Y3ofN`)
  predates the shipped ladder.
- **No title bar** (`docs/tui-design.md:rbQjh`). **Sidebar off by default**,
  byte-identical base while closed, docked only ≥80 cols; below 80 the layout is
  untouched (`:7TMJb`, `:D1aSS`).
- **A blank line between roles** (`:vHHsu`); only prose and the prompt are
  full-strength, the rest dim. **Key-first**: every interactive thing has a key
  and a keymap entry (`docs/tui-design.md:yZ4Cg`, the Keys block, line 197); a
  feature with no key binding is undiscoverable.

## §4 — Medium B rules — the wcode site

These rules are **this site's**, derived from its tokens. A greenfield page is
**not** governed by §4 — it is §2.C (install a system) + §5 (copy). §4 does not
travel.

- **Zero external requests** (`style.css:V3G80`): system font stack only; no
  webfont `<link>`, CDN script/style, remote image, or analytics. A new
  off-origin `<script src=…>`/`<link rel=…>` is a fail.
- **The token set is the palette** (`style.css:FGqrn`): `--bg --panel --panel-2
  --fg --muted --border --accent --prompt --code --warn --glow`. Adding a color
  means adding a prop; **≤1 accent** (`--accent`, `:SWk8U`), used identically
  everywhere a link is (`a`, `:gn1if`); `--prompt`/`--code`/`--warn` are semantic,
  not decorative.
- **Dark terminal, always, one theme** (`style.css:V3G80`, `color-scheme: dark`):
  no light toggle, no section flips mid-page.
- **Typography/layout**: monospace system stack; `main` is capped at
  `max-width: 62rem` (`style.css:AaYeK`; `footer` at `:wd8KM`, same width); the
  `h1`/`h2`/`h3` scale (`:tJQYv`, `:FOd6L`, `:CwAsx`) is the hierarchy — weigh
  weight+color before raw size.
- **IA is preserved**: the section order in `index.html` (`:bO8Jn`…`:Jomcn`) is
  the information architecture; do not reorder or drop a section without asking.
- **States**: hover / `:focus-visible` / disabled for every interactive element;
  landmarks carry `aria-label`, diagrams `role="img"` + `aria-label`.
- **Motion**: exactly one animation today (blink, `style.css:rvKBt`); any
  addition is justified in one sentence and honors `prefers-reduced-motion`.

## §5 — Shared copy rules

These apply to **rendered UI strings** on both surfaces.

- **Plain, functional labels** over performative ones ("Testimonials", not "From
  the field").
- **The em-dash (`—`) ban is scoped to web UI strings only** — headlines,
  eyebrows, labels, buttons, captions, alt text, nav. It does **not** reach repo
  markdown prose (README, `docs/*.md`, `AGENTS.md`), and it does **not** reach
  the TUI: the locked spec deliberately uses `—` as the assistant-prose glyph
  (`docs/tui-design.md:37`) and as the empty-section marker (`:194`). Banning it
  there would contradict the spec this skill enforces. This scoping amends the
  source doctrine's absolute rule: a page fails on one `—` in a **rendered
  string**; a doc, and the TUI, do not.
- **Concrete verbs**, no filler. Blocked outright: "Elevate", "Seamless",
  "Unleash", and their kin.
- **One-line descriptions**; no micro-meta-sentences under a heading; `·` ≤1 per
  metadata line.

## §6 — AI tells (tagged)

- `[both]` No pure black where the surface owns its background (`--bg` is off-black
  `#0a0e14`, `style.css:iW24v`; the TUI has no `bg` role — the terminal owns it); no
  oversaturated accents, neon, or outer glows; no filler verbs, performative
  section labels, or a row of three identical cards; `·` ≤1 per metadata line.
- `[web]` No em-dash in a rendered string (§5); no webfont/CDN/remote-image/
  analytics (§4); no mid-page theme flip; no section-numbering eyebrows (`001 ·
  Capabilities`), scroll cues, locale/weather strips, version labels in the hero,
  or decorative status dots; no `div`-based fake screenshots or hand-rolled
  decorative SVG icons.
- `[tui]` No invented glyph, fourth band, title bar, or palette role without a
  call site; no absolute color on a non-truecolor terminal (hex only under
  `Rgb`).

## §7 — Redesign protocol (audit before touching)

1. **Detect the mode**: greenfield · **redesign-preserve** · **redesign-overhaul**.
   Ambiguous → ask once.
2. **Audit first**: for the TUI, read `docs/tui-design.md` + `theme.rs` and list
   the locked invariants; for the site, extract the tokens (`style.css:FGqrn`)
   and the IA (`index.html` order).
3. **Never change a locked invariant silently.** A band, glyph, palette role, or
   the color ladder requires **amending `docs/tui-design.md` first**. A site token
   change is a `:root` change; never inline a stray hex.
4. **Modernisation levers, in order**: typography → spacing → color → motion →
   one section's recomposition → full block replacement (last).
5. **Preserve**: IA/section order, token names, existing a11y wins, the glyph
   vocabulary.

## §8 — Pre-flight matrix

Run every box; any fail means the output is not done.

**Both** — §0 surface line emitted, medium named? · dials stated and reasoned? ·
truth source named (locked spec / site tokens / installed system)? · every
applicable §3–§4 rule met?

**Terminal (A)** — exactly three bands, no title bar, blank line between roles? ·
every glyph from the locked table, no new glyph? · palette roles unchanged, ≤1
accent, any new role has a call site? · hex only under `Rgb`, `NO_COLOR` still
yields `Theme::plain`? · legible at 80 cols (and the 48-col draft holds)? · every
interactive thing has a key + a keymap entry?

**Web (B)** — **ZERO `—` in any rendered string**, **zero external requests**? ·
≤1 accent, all colors are `:root` props (no inline stray hex)? · one theme, no
mid-page flip, section order preserved? · `prefers-reduced-motion` honored and
every animation justified? · hover/focus/disabled states present, landmarks carry
`aria-label`?

## §9 — Out of scope

- **Not the kernel** — no behavior config, MCP, or permission flows (those are
  `Hooks`/code, per `AGENTS.md`).
- **Not markdown prose** — markdown prose (README, `docs/*.md`, `AGENTS.md`)
  gets **no** rule set of its own; §5's copy rules apply to **rendered UI strings
  only**.
- **Not a JS/framework skill** — this repo has no frontend build; the web half is
  native CSS/HTML only.
- **Not native mobile, code-editor skinning, or a general dashboard system** (the
  web half defers to an official system, §2.C).
- **Enforce-only, never owner.** This skill enforces the locked TUI spec
  (`docs/tui-design.md` + `theme.rs`); it does not own it. A genuine TUI redesign
  amends `docs/tui-design.md` first; the skill never re-opens it.

## §10 — References

`docs/tui-design.md` (locked TUI spec) · `crates/wcode-tui/src/theme.rs` (roles +
ladder) · `style.css`, `index.html` (the site's only spec) · `AGENTS.md`
(conventions) · `docs/skills-references-plan.md` (the loader).

## Grounding (anchor index)

- Skills: `.wcode/skills` root `crates/wcode-cli/src/skills.rs:1ads9`; `valid_name`
  `:efuUM`; description ≤1024 `:Vlnli`; prompt clip 200 `:ftQCJ`; only
  `name`+`description` folded in `:7ib2K`. The body loads on demand via the
  existing `read` tool.
- TUI (locked): bands `docs/tui-design.md:W4SAR`; gutter `:zwmWX`; palette
  `:yQCWu`; role gap `:vHHsu`; glyphs `:pMZOm` (lines 34–50); decisions `:npxul`;
  no title bar `:rbQjh`; sidebar off by default, base untouched <80 cols
  `:7TMJb`/`:D1aSS`. The color ladder has shipped, so the older NO_COLOR line
  `:4AL6e` and its "detection still open" neighbour `:Y3ofN` predate it — cite the
  ladder (`:7wN6X`, `:QNCsA`), not those.
- Theme: 17 named roles `crates/wcode-tui/src/theme.rs:3dRHq`; color ladder
  `:7wN6X`, `ColorMode` `:QNCsA`; hex honored only under `Rgb` `:IWs97`.
- Site (its only spec): "Dark terminal, always. Zero external requests: system
  font stack only." `style.css:V3G80`; tokens `:FGqrn`; **`main`** width
  `:AaYeK` (footer `:wd8KM`); blink `:rvKBt`; IA `index.html:bO8Jn`…`:Jomcn`.

## Editing this file

Keep the frontmatter **quoted** and on one line: an unquoted `: ` is invalid YAML, and
the loader then drops the whole skill with only a stderr warning. `name` is validated by
`valid_name` (`crates/wcode-cli/src/skills.rs:efuUM`; `:cBdoy` is only its doc comment).
The description is capped at 1024 chars (`:Vlnli`) and clipped to 200 in the prompt
(`:ftQCJ`), so the trigger must sit inside the first 200. Only `name` + `description` reach
the prompt (`:7ib2K`); this body loads on demand via `read`.
