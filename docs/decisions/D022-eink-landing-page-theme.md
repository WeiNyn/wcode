# D022 — The landing page keeps dark as its default and gains an opt-in e-ink theme

- **Status:** accepted (human sign-off — this session)
- **Date:** this session
- **Relates to:** the GitHub-Pages landing page (`index.html` + `style.css`, served
  from the repo root; tracker item 45)

## Context

The landing page shipped a single hard-coded look. Its stylesheet said so in the
first line: `style.css:1` — "Dark terminal, always. Zero external requests: system
font stack only." That invariant is deliberate: the brand *is* a dark terminal,
and the page never fetches anything off-origin.

The human asked, verbatim, to "make the pages have eink styles". An e-ink reader
(Boox/Kobo/Kindle browsers) renders a dark, glow-and-gradient page poorly — the
radial glow (`style.css` `body` `radial-gradient`), the accent-painted links, and
the muted-blue secondary text all read as low-contrast mud on a reflective panel.

A change was needed that adds a paper reading **without** discarding the dark
default or the zero-request rule. Three shapes were on the table: replace the
look outright; an opt-in toggle; or auto-switch on `prefers-color-scheme`. The
human chose the **toggle**.

## Decision

1. **Dark stays the default for every first-time visitor** — including one on
   `prefers-color-scheme: light`. The brand page is not varied by OS preference.
2. **E-ink is opt-in**, via a real header control: `index.html:62` —
   `<button id="theme-toggle" class="theme-toggle" type="button" aria-pressed="false">E-ink</button>`,
   keyboard-focusable, its state restored from `localStorage` (`wcode-theme`) and
   applied before first paint by the inline `<head>` script (`index.html:29-56`).
3. **The theme is a token swap, not a fork.** `style.css:181` —
   `html[data-theme="eink"] { … }` — re-reads the *same* 11 custom properties as
   paper + ink (`--bg: #f6f4ed`, `--fg: #16150f`, `--muted: #444239`) and
   overrides only the painted structure: gradient off (`:196`), shadow off
   (`:200`), motion off (`:199`). Every existing class and every SVG hook is
   reused, so the inline diagrams recolor for free (the markup carries no inline
   `fill`/`stroke`).
4. **Zero hue and zero external requests in e-ink:** `style.css:189-192` collapse
   `--accent/--prompt/--code/--warn` to ink `#16150f` and `--glow: transparent`;
   the system font stack is unchanged.

The stylesheet's own invariant comment is amended to record the new rule rather
than contradicting the code (`style.css:1-4`).

## Consequences

- A reader on an e-ink display gets a legible page (muted text is ≥ 7:1 on every
  paper surface: `--muted #444239` on `--panel #ece9df` is 8.29:1; on `--bg`
  9.16:1), and the dark-terminal identity is preserved as the default.
- **Cost:** a second token block to keep in sync with `:root` (all 11 tokens must
  stay redefined), plus one small inline script on an otherwise-static page.
- **Foreclosed:** auto-switching on `prefers-color-scheme` as the *default*
  (e-ink is a manual choice); and any color, glow, gradient, shadow, or motion in
  e-ink mode.
- The change is presentation-only — no prose, no new page section, and the dark
  rules are untouched apart from the additive toggle.
