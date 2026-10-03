# Sketch — a `design-taste` skill (NEW `.wcode/skills/design-taste/SKILL.md`)

**Status:** interface sketch, review-only. A skill is prose, so the "interface" is its
frontmatter, its rule set, and the checks that make each rule provable. Fillable as-is.

**Design:** an agent **design skill** that mines the `taste-skill` *method* (§0 brief read,
§1 dials, §4 directives, §9 tells, §11 redesign, §14 pre-flight) and re-scopes it to
wcode's two surfaces. It **enforces** the locked TUI spec (`docs/tui-design.md` +
`theme.rs`) and the site's existing tokens (`style.css`); it redesigns neither.

**Grounding (fresh anchors):**
- Skills: `.wcode/skills` root `crates/wcode-cli/src/skills.rs:1ads9`; `valid_name` `:cBdoy`;
  description ≤1024 `:Vlnli`; prompt clip 200 `:ftQCJ`; only `name`+`description` folded in
  `:7ib2K`. The body loads on demand via the existing `read` tool.
- TUI (locked): bands `docs/tui-design.md:W4SAR`; gutter `:zwmWX`; palette `:yQCWu`; NO_COLOR +
  open truecolor `:4AL6e`; role gap `:vHHsu`; glyphs `:pMZOm`; decisions `:npxul`; no title bar
  `:rbQjh`; sidebar off by default, base untouched <80 cols `:7TMJb`/`:D1aSS`.
- Theme: 17 named roles `crates/wcode-tui/src/theme.rs:3dRHq`; color ladder `:7wN6X`,
  `ColorMode` `:QNCsA`; hex honored only under `Rgb` `:IWs97`.
- Site (its only spec): "Dark terminal, always. Zero external requests: system font stack only."
  `style.css:V3G80`; tokens `:FGqrn`; body width `:AaYeK`; blink `:rvKBt`; IA `index.html:bO8Jn`…`:Jomcn`.

---

## The file (skeleton)

### Frontmatter — literal, frozen

```yaml
---
name: design-taste
description: Visual-design taste for wcode's two UI surfaces: the terminal TUI (crates/wcode-tui; enforces docs/tui-design.md + theme.rs) and the static site (index.html/style.css). Use when building or restyling any user-facing screen: read the brief, set three dials, run the pre-flight. Not for prose docs or the kernel.
---
```

`name` is valid per `:cBdoy`; the description is **one line**, ≤1024 `:Vlnli`, its **first
~200 chars** carrying the trigger because the prompt clips there `:ftQCJ`. The `—` here is
deliberate (§5, Friction 1): the ban is scoped to web **UI strings**, and a skill's own
frontmatter is repo prose.

### §0 — Surface read (before anything)

Emit **one line**, then proceed:

```
Reading this as: <medium: a terminal surface | a web screen>, for <who>, in the
<locked wcode spec | the site's dark-terminal tokens | <named system>> language,
with <the one dominant constraint — density at 80×24, or zero external requests>.
```

- **Name the surface first.** Medium **A** = terminal (`crates/wcode-tui`); **B** = web
  (`index.html`/`style.css`, or a greenfield page). The rule sets are disjoint; picking
  wrong is the failure mode.
- **Ambiguous brief → ask exactly ONE question**, never a battery, and only when the medium
  genuinely diverges.
- **Anti-default** — reach past the LLM defaults: centered hero over a mesh, three equal
  feature cards, AI-purple gradients, an em-dash flourish, a new accent color.
  <!-- SKETCH: TODO(prose) — one sentence naming the likeliest default per medium. -->

### §1 — The three dials

`DESIGN_VARIANCE` (1 symmetry → 10 chaos) · `MOTION_INTENSITY` (1 static → 10 cinematic) ·
`VISUAL_DENSITY` (1 airy → 10 packed). Inferred from §0, never asked. **The TUI reading of
each dial is not the web reading:**

| brief signal | VARIANCE | MOTION | DENSITY |
|---|---|---|---|
| TUI — any surface (the spec is the design) | **1, fixed** | **1–2** | 7–8 |
| TUI — a new region (sidebar/overlay) | **1, fixed** | 1–2 | 8–9 |
| web — the wcode site | 5–6 | 3–4 | 4–5 |
| web — greenfield marketing | 6–8 | 4–6 | 3–5 |

- **The TUI dials are pinned**: `VARIANCE` 1 (the spec is the design), `MOTION` ≤2 (a spinner
  `⠋⠙⠹⠸` + the live cursor, both gone under `NO_COLOR`/reduced-motion), `DENSITY` high (an 80×24
  grid, legible to 48 cols). A reading that implies inventing a layout is a misread (Friction 2).

### §2 — Where the truth already lives

- **A. TUI → `docs/tui-design.md`** ("agreed; implemented through P3", `:npxul`). §3 cites
  and enforces it; a genuine redesign must **first amend that spec**, not the skill.
- **B. The site → `style.css`'s custom properties.** There is **no** design doc for the site:
  `index.html` + `style.css` are the spec (`:FGqrn`, `:V3G80`). §4 derives from those tokens;
  writing them down is this skill's first act of authority on the site (Friction 3).
- **C. Greenfield → install a real system, or label the aesthetic honestly.** A named system
  (Primer, GOV.UK, Material, …) means the official package, not a re-draw; one system per project.

### §3 — Medium A rules (terminal) — enforcement, not invention

- **Three bands, always** — transcript (flex) → input → status; no fourth (`:W4SAR`). An overlay
  never reflows the base.
- **Gutter**: 1-col margin, marker column, content at a fixed column; wrapped continuations
  align under the content (`:zwmWX`).
- **Glyphs from the locked table only** (`:pMZOm`): `❯ ··· ⚙ ✓ ✗ ⋯ ▌ ⠋ ▤ · \` │ ─ ┼ █░`.
  Inventing a glyph is a fail.
- **Palette = the 17 named roles in `theme.rs`** (`:3dRHq`); a new role needs a real call
  site. Named ANSI only (palette B default); **≤1 accent** (`accent`, cyan).
- **Color ladder is fixed** (`:7wN6X`, `:QNCsA`): `NO_COLOR`/`TERM=dumb`→`Plain`; truecolor
  `COLORTERM`→`Rgb`; `256color`→`Indexed`; else `Named`. Hex honored **only** under `Rgb`
  (`:IWs97`). **Do not lock the open truecolor item** — `:4AL6e` marks the tier palettes still
  open; cite the ladder, decide nothing.
- **No title bar** (`:rbQjh`). **Sidebar off by default**, byte-identical base while closed,
  docked only ≥80 cols; below 80 the layout is untouched (`:7TMJb`, `:D1aSS`).
- **Blank line between roles** (`:vHHsu`); only prose and the prompt are full-strength, the rest
  dim. **Key-first**: every interactive thing has a key and a keymap entry (`docs/tui-design.md`
  Keys block); a feature with no key binding is undiscoverable.

### §4 — Medium B rules (web) — derived from the tokens

- **Zero external requests** (`:V3G80`): system font stack only; no webfont `<link>`, CDN
  script/style, remote image, or analytics. A new off-origin `<script src=…>`/`<link rel=…>`
  is a fail.
- **The token set is the palette** (`:FGqrn`): `--bg --panel --panel-2 --fg --muted --border
  --accent --prompt --code --warn --glow`. Adding a color means adding a prop; **≤1 accent**
  (`--accent`, `:SWk8U`), used identically everywhere a link is (`a`, `:gn1if`);
  `--prompt`/`--code`/`--warn` are semantic, not decorative.
- **Dark terminal, always, one theme** (`:V3G80`, `color-scheme: dark`): no light toggle, no
  section flips mid-page.
- **Typography/layout**: monospace system stack; body `max-width: 62rem` (`:AaYeK`); the
  `h1`/`h2`/`h3` scale (`:tJQYv`,`:FOd6L`,`:CwAsx`) is the hierarchy — weigh weight+color before
  raw size.
- **IA is preserved**: the section order in `index.html` (`:bO8Jn`…`:Jomcn`) is the
  information architecture; do not reorder or drop a section without asking.
- **States**: hover / `:focus-visible` / disabled for every interactive element; landmarks
  carry `aria-label`, diagrams `role="img"`+`aria-label`.
- **Motion**: exactly one animation today (blink, `:rvKBt`); any addition is justified in one
  sentence and honors `prefers-reduced-motion`.

### §5 — Shared copy rules

- **Plain, functional labels** over performative ones ("Testimonials", not "From the field").
- **The em-dash ban is scoped to web UI strings** (headlines, eyebrows, labels, buttons,
  captions, alt text, nav) — **not** repo markdown prose (`AGENTS.md`, `README.md`,
  `docs/*.md`, this skill's frontmatter). This **amends** the source doctrine's absolute §9.G:
  a page fails on one `—` in a **rendered string**; a doc does not.
- **Concrete verbs**, no filler ("Elevate", "Seamless", "Unleash").
- **One-line descriptions**; no micro-meta-sentences under a heading; `·` ≤1 per metadata line.

### §6 — AI tells (tagged)

- `[both]` No pure black (`--bg` is off-black `#0a0e14`, `:iW24v`); no oversaturated accents, neon,
  or outer glows; no filler verbs, performative section labels, or three-identical-cards row; `·` ≤1
  per metadata line.
- `[web]` No em-dash in a rendered string (§5); no webfont/CDN/remote-image/analytics (§4); no mid-page
  theme flip; no section-numbering eyebrows (`001 · Capabilities`), scroll cues, locale/weather
  strips, version labels in the hero, or decorative status dots; no `div`-based fake screenshots or
  hand-rolled decorative SVG icons.
- `[tui]` No invented glyph, fourth band, title bar, or palette role without a call site; no absolute
  color on a non-truecolor terminal (hex only under `Rgb`).

### §7 — Redesign protocol (audit before touching)

1. **Detect the mode**: greenfield · **redesign-preserve** · **redesign-overhaul**. Ambiguous →
   ask once.
2. **Audit first**: for the TUI, read `docs/tui-design.md` + `theme.rs` and list the locked
   invariants; for the site, extract the tokens (`:FGqrn`) and the IA (`index.html` order).
3. **Never change a locked invariant silently.** A band, glyph, palette role, or the color
   ladder requires **amending `docs/tui-design.md` first** (Friction 2). A site token change is
   a `:root` change; never inline a stray hex.
4. **Modernisation levers, in order**: typography → spacing → color → motion → one section's
   recomposition → full block replacement (last).
5. **Preserve**: IA/section order, token names, existing a11y wins, the glyph vocabulary.

### §8 — Pre-flight matrix

Run every box; any fail means the output is not done.

**Both** — §0 surface line emitted, medium named? · dials stated and reasoned? · truth source
named (locked spec / site tokens / installed system)? · every applicable §3–§4 rule met?

**Terminal (A)** — exactly three bands, no title bar, blank line between roles? · every glyph
from the locked table, no new glyph? · palette roles unchanged, ≤1 accent, any new role has a
call site? · hex only under `Rgb`, `NO_COLOR` still yields `Theme::plain`? · legible at 80 cols
(and the 48-col draft holds)? · every interactive thing has a key + a keymap entry?

**Web (B)** — **ZERO `—` in any rendered string**, **zero external requests**? · ≤1 accent, all
colors are `:root` props (no inline stray hex)? · one theme, no mid-page flip, section order
preserved? · `prefers-reduced-motion` honored and every animation justified? · hover/focus/
disabled states present, landmarks carry `aria-label`?

### §9 — Out of scope

- **Not the kernel** — no behavior config, MCP, or permission flows (those are `Hooks`/code,
  per `AGENTS.md`).
- **Not prose docs** — markdown prose (README, `docs/*.md`, `AGENTS.md`) is exempt except §5;
  the em-dash ban does not reach it.
- **Not a JS/framework skill** — this repo has no frontend build; the web half is native
  CSS/HTML only.
- Not native mobile, code-editor skinning, or a general dashboard system (the web half defers to
  an official system, §2.C). It does **not own** the TUI spec; it enforces one that exists.

### §10 — References

`/tmp/taste-skill.md` (source doctrine, method only) · `docs/tui-design.md` (locked TUI spec) ·
`crates/wcode-tui/src/theme.rs` (roles + ladder) · `style.css`, `index.html` (the site's only
spec) · `AGENTS.md` (conventions) · `docs/skills-references-plan.md` (the loader).
<!-- SKETCH: TODO(prose) — keep the /tmp path, or copy the doctrine's method into a
skill-local references/ file? (OQ3.) -->

## Notes / friction

- **Em-dash ban vs. house style (amendment).** `taste-skill` §9.G bans `—` outright; this
  repo's prose uses it throughout. Scoped to **web UI strings**, not repo markdown — stated in
  §5 so a future reader does not "fix" the docs. (OQ4.)
- **Medium A has no freedom left.** `docs/tui-design.md` is agreed and implemented through P3
  with a locked-decisions list (`:npxul`); the skill enforces those, and a real TUI redesign
  amends the spec first. Stated in §2 and §7 so it never reads as permission to re-open them.
- **The site has no spec.** `index.html`/`style.css` are the only source; §4 is that source
  written down for the first time — first-authority, not a change.
- **The loader reads only `name` + `description`** (`:7ib2K`); the body is load-on-demand via
  `read`. Keep the description self-sufficient.

## For the reviewer

- **OQ1:** does the skill **own** the TUI design spec or only **enforce** it? (Sketch assumes
  enforce-only; a redesign amends `docs/tui-design.md`.)
- **OQ2:** does "docs/README prose" get a rule set of its own, or only §5's copy rules?
- **OQ3:** ship a `references/` block library now (the `taste-skill` §12 analogue) or defer?
  (§10 has a TODO.)
- **OQ4:** is the em-dash scope in §5 correct — web UI strings only, repo prose exempt?
