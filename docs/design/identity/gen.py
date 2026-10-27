#!/usr/bin/env python3
"""Generate the wcode identity candidates (round 1, design only).

Every artefact in this directory is derived from this file, so a tweak to a
number here re-emits all eight SVGs and the contact sheet. Nothing outside
`docs/design/identity/` is touched; the live marks
(`editors/vscode/media/wcode.svg`, `favicon.svg`, `docs/book/favicon.svg`) are
untouched by design.

Run: `python3 docs/design/identity/gen.py`

Geometry vocabulary
-------------------
`ribbon()` outlines a polyline stroke as a *filled polygon* (butt caps, miter
joins clamped to a limit, a clamped join becoming a bevel). Fill-only output is
deliberate: the activity-bar contract is `fill="currentColor"` on the root, and
some hosts additionally force `svg { fill: currentColor }` from CSS, which would
turn a `stroke=`-only icon into a filled blob.

The coordinate grid is the 24-unit viewBox. Axis-aligned edges are placed on a
**1.5-unit** grid so they land on a whole pixel at 16, 32, 64 and 128 px
(24/16 = 1.5, so coordinate c lands at 2c/3 px; c in multiples of 1.5 -> integer).
At 24 px, DPR 1, those edges sit on a half pixel — soft, not wrong; at DPR 2
(48 px) they are whole again. Diagonals cannot snap to any raster, so they are
placed on a 0.1-unit grid and are honest about it.
"""

import math
import pathlib

OUT = pathlib.Path(__file__).resolve().parent

BRAND_SURFACE = "#0b0e13"  # the live book's --bg (docs/book/style.css:6)
BRAND_INK = "#3fb950"      # the only hex ever attached to the *mark*: docs/book/favicon.svg
TILE_RADIUS = 26           # 128 * 0.203; the mark's own radius is 4.5/24 = 0.1875
TILE_SCALE = 4             # 24 * 4 = 96 in a 128 tile -> a 16-unit inset


# --------------------------------------------------------------------------- #
# stroke -> outline
# --------------------------------------------------------------------------- #
def _unit(v):
    n = math.hypot(*v)
    return (v[0] / n, v[1] / n)


def ribbon(pts, width, miter_limit=1.6):
    """The outline polygon of a constant-width stroke along `pts`.

    A join is *convex* on the side its two offsets diverge from, and only a
    convex join may spike; clamping it to `miter_limit` turns the spike into a
    flat bevel (which is exactly what a pixel font does to the 39-degree
    vertices of a `w`). A *concave* join takes the plain offset intersection —
    clamping it there would fold the outline back over itself.
    """
    half = width / 2.0
    n = len(pts)
    dirs = [_unit((pts[i + 1][0] - pts[i][0], pts[i + 1][1] - pts[i][1])) for i in range(n - 1)]
    nrm = [(-d[1], d[0]) for d in dirs]  # left normal of each segment
    left, right = [], []
    for i in range(n):
        px, py = pts[i]
        if i == 0 or i == n - 1:
            n1 = nrm[0] if i == 0 else nrm[-1]
            left.append((px + half * n1[0], py + half * n1[1]))
            right.append((px - half * n1[0], py - half * n1[1]))
            continue
        n1, n2 = nrm[i - 1], nrm[i]
        sx, sy = n1[0] + n2[0], n1[1] + n2[1]
        s = math.hypot(sx, sy)
        if s < 1e-9:  # 180 degree reversal, nonsense here: butt it
            left.append((px + half * n1[0], py + half * n1[1]))
            right.append((px - half * n1[0], py - half * n1[1]))
            continue
        cross = dirs[i - 1][0] * dirs[i][1] - dirs[i - 1][1] * dirs[i][0]
        ratio = 2.0 / s  # 1/cos(half turn)
        mx, my = sx / s, sy / s
        d = half * ratio  # offset distance along the miter bisector
        miter_l = (px + mx * d, py + my * d)
        miter_r = (px - mx * d, py - my * d)
        if ratio > miter_limit and cross < 0:  # convex on the +normal side
            left.append((px + half * n1[0], py + half * n1[1]))
            left.append((px + half * n2[0], py + half * n2[1]))
        else:
            left.append(miter_l)
        if ratio > miter_limit and cross > 0:  # convex on the -normal side
            right.append((px - half * n1[0], py - half * n1[1]))
            right.append((px - half * n2[0], py - half * n2[1]))
        else:
            right.append(miter_r)
    return left + right[::-1]


def poly(pts, close=True, dec=2):
    fmt = f"{{:.{dec}f}}"
    d = "M" + " L".join(f"{fmt.format(x).rstrip('0').rstrip('.')},{fmt.format(y).rstrip('0').rstrip('.')}" for x, y in pts)
    return d + ("Z" if close else "")


def num(v, dec=2):
    return f"{v:.{dec}f}".rstrip("0").rstrip(".")


def rect_path(x0, y0, x1, y1, r):
    cx = min(r, (x1 - x0) / 2, (y1 - y0) / 2)
    n = num
    return (
        f"M{n(x0 + cx)},{n(y0)} H{n(x1 - cx)} A{n(cx)},{n(cx)} 0 0 1 {n(x1)},{n(y0 + cx)} "
        f"V{n(y1 - cx)} A{n(cx)},{n(cx)} 0 0 1 {n(x1 - cx)},{n(y1)} H{n(x0 + cx)} "
        f"A{n(cx)},{n(cx)} 0 0 1 {n(x0)},{n(y1 - cx)} V{n(y0 + cx)} A{n(cx)},{n(cx)} 0 0 1 {n(x0 + cx)},{n(y0)} Z"
    )


# --------------------------------------------------------------------------- #
# the three marks, in 24-unit coordinates
# --------------------------------------------------------------------------- #
# A — prompt + cursor: today's `>_` with the 2.2-unit underscore promoted to a
# 3-unit cursor block (a 2 px block at 16 px, where today's bar is 1.47 px).
A_CHEVRON = [(3.6, 5.0), (10.6, 12.0), (3.6, 19.0)]
A_WIDTH = 2.5
A_BLOCK = (13.5, 16.5, 21.0, 19.5)  # x0,y0,x1,y1

# B — the wordmark's `w`, four strokes, 2.2 wide (today's chevron stroke),
# acute vertices bevelled flat where a miter would draw a needle.
B_PTS = [(3.7, 6.1), (7.85, 18.1), (12.0, 6.1), (16.15, 18.1), (20.3, 6.1)]
B_WIDTH = 2.2
B_MITER = 1.6

# C — the cursor cell: a filled rounded square with the prompt knocked out.
C_BOX = (2.5, 2.5, 21.5, 21.5)
C_RADIUS = 4.5          # 18.75% of 24, the ratio docs/book/favicon.svg uses at 32
C_CHEVRON = [(9.1, 7.4), (13.7, 12.0), (9.1, 16.6)]
C_WIDTH = 3.2


def mark_a():
    return [
        f'<path d="{poly(ribbon(A_CHEVRON, A_WIDTH, miter_limit=8))}"/>',
        f'<rect x="{num(A_BLOCK[0])}" y="{num(A_BLOCK[1])}" width="{num(A_BLOCK[2] - A_BLOCK[0])}" '
        f'height="{num(A_BLOCK[3] - A_BLOCK[1])}"/>',
    ]


def mark_b():
    return [f'<path d="{poly(ribbon(B_PTS, B_WIDTH, B_MITER))}"/>']


def mark_c():
    outer = rect_path(*C_BOX, C_RADIUS)
    hole = poly(ribbon(C_CHEVRON, C_WIDTH, miter_limit=8))
    return [f'<path fill-rule="evenodd" d="{outer} {hole}"/>']


MARKS = {
    "a": ("Prompt + cursor", mark_a),
    "b": ("The wordmark's w", mark_b),
    "c": ("The cursor cell", mark_c),
}


def tile_paths(key, bg, ink):
    """The same 24-unit drawing, 4x, on a 128 tile. Returns (bg_svg, ink_svg)."""
    if key == "c":
        # C is the only candidate whose mark *is* its tile, so the tile is the
        # brand field with the prompt knocked out of it.
        bg_svg = f'<rect width="128" height="128" rx="{TILE_RADIUS}" fill="{BRAND_INK}"/>'
        hole = poly(ribbon(C_CHEVRON, C_WIDTH, miter_limit=8))
        ink_svg = (
            f'<g transform="translate({(128 - 24 * TILE_SCALE) / 2},{(128 - 24 * TILE_SCALE) / 2}) '
            f'scale({TILE_SCALE})"><path fill-rule="evenodd" d="{hole}" fill="{bg}"/></g>'
        )
        return bg_svg, ink_svg
    bg_svg = f'<rect width="128" height="128" rx="{TILE_RADIUS}" fill="{bg}"/>'
    inner = "".join(MARKS[key][1]())
    ink_svg = (
        f'<g transform="translate({(128 - 24 * TILE_SCALE) / 2},{(128 - 24 * TILE_SCALE) / 2}) '
        f'scale({TILE_SCALE})" fill="{ink}">{inner}</g>'
    )
    return bg_svg, ink_svg


# --------------------------------------------------------------------------- #
# emit
# --------------------------------------------------------------------------- #
def write_mark(key):
    body = "\n  ".join(MARKS[key][1]())
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor">\n'
        f"  {body}\n"
        "</svg>\n"
    )
    (OUT / f"candidate-{key}.svg").write_text(svg)
    return body


def write_tile(key):
    bg_svg, ink_svg = tile_paths(key, BRAND_SURFACE, BRAND_INK)
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" '
        'width="128" height="128">\n'
        f"  {bg_svg}\n  {ink_svg}\n"
        "</svg>\n"
    )
    (OUT / f"candidate-{key}-tile.svg").write_text(svg)


def bbox(pts):
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return (min(xs), min(ys), max(xs), max(ys))


def report():
    a = ribbon(A_CHEVRON, A_WIDTH, miter_limit=8) + [
        (A_BLOCK[0], A_BLOCK[1]), (A_BLOCK[2], A_BLOCK[3])
    ]
    b = ribbon(B_PTS, B_WIDTH, B_MITER)
    c = ribbon(C_CHEVRON, C_WIDTH, miter_limit=8)
    for key, pts in (("A", a), ("B", b), ("C void", c)):
        x0, y0, x1, y1 = bbox(pts)
        print(f"  {key:7s} bbox {x0:5.2f},{y0:5.2f} -> {x1:5.2f},{y1:5.2f}"
              f"  w {x1 - x0:5.2f} h {y1 - y0:5.2f}"
              f"  margins L{x0:4.2f} R{24 - x1:4.2f} T{y0:4.2f} B{24 - y1:4.2f}")


def main():
    report()
    bodies = {k: write_mark(k) for k in MARKS}
    for k in MARKS:
        write_tile(k)
    (OUT / "candidates.html").write_text(render_html(bodies))
    for k in MARKS:
        print(f"candidate-{k}.svg ->", bodies[k][:72].replace("\n", " "), "...")


# --------------------------------------------------------------------------- #
# the contact sheet
# --------------------------------------------------------------------------- #
SIZES = [16, 24, 32, 64, 128]

CONCEPTS = {
    "a": (
        "Prompt + cursor",
        "The two glyphs the TUI is made of: the <code>❯</code> prompt and the "
        "<code>▌</code> live cursor. Today's <code>&gt;_</code> with the underscore "
        "promoted to a cursor block.",
        "Nothing to relearn: it still reads <code>&gt;_</code> at a glance.",
        "Category-typical — every CLI has a chevron.",
    ),
    "b": (
        "The wordmark's <code>w</code>",
        "The initial, drawn as four strokes on the same chevron family as "
        "<code>❯</code>, with every acute vertex cut flat where a miter would "
        "draw a needle.",
        "The identity is the name, not a borrowed terminal trope.",
        "A letter says nothing about what the product does; dense at 16 px.",
    ),
    "c": (
        "The cursor cell",
        "The live cursor made solid, with the prompt knocked out of it: one mass, "
        "one ink, and the only candidate whose mark and app tile are one shape.",
        "Reads as an app icon rather than a toolbar glyph.",
        "A filled square is a blob among the activity bar's line icons.",
    ),
}


def render_html(bodies):
    """One judge matrix: rows are the five real sizes, columns are
    candidate x surface. Marks are left-aligned inside a column so their
    heights and ink weight line up for comparison; the row height is the
    size's own height, so a 16 px row stays quiet."""

    def symbol(key):
        # the file's own root attributes, so the sheet inherits ink exactly as
        # the host does: `color` on the surface, `fill="currentColor"` on the mark
        return (
            f'<symbol id="mark-{key}" viewBox="0 0 24 24" fill="currentColor">'
            f"{bodies[key]}</symbol>"
        )

    def use(key, px):
        return (
            f'<svg class="mark" viewBox="0 0 24 24" width="{px}" height="{px}" '
            f'role="img" aria-label="candidate {key.upper()} at {px} px">'
            f'<use href="#mark-{key}"/></svg>'
        )

    head = "".join(
        f'<th colspan="2" scope="colgroup"><span class="key">{k.upper()}</span>'
        f"{CONCEPTS[k][0]}</th>"
        for k in MARKS
    )
    subhead = "".join(
        '<th scope="col">dark</th><th scope="col">light</th>' for _ in MARKS
    )
    body = []
    for px in SIZES:
        cells = "".join(
            f'<td class="on-dark">{use(k, px)}</td>'
            f'<td class="on-light">{use(k, px)}</td>'
            for k in MARKS
        )
        body.append(
            f'<tr><th scope="row">{px}<span>px</span></th>{cells}</tr>'
        )

    notes = "".join(
        f'<section class="note"><h3><span class="key">{k.upper()}</span>{name}</h3>'
        f'<p class="what">{concept}</p>'
        f'<p class="verdict"><b>Buys</b> {good}</p>'
        f'<p class="verdict"><b>Costs</b> {bad}</p></section>'
        for k, (name, concept, good, bad) in CONCEPTS.items()
    )
    bars_dark = "".join(f'<div class="bar dark">{use(k, 24)}</div>' for k in MARKS)
    bars_light = "".join(f'<div class="bar light">{use(k, 24)}</div>' for k in MARKS)
    tiles = "".join(
        f'<figure><img src="candidate-{k}-tile.svg" width="128" height="128" alt="">'
        f"<figcaption>{k.upper()} \u00b7 128 px</figcaption></figure>"
        for k in MARKS
    )

    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>wcode identity \u00b7 round 1</title>
<style>
  :root {{
    --bg: #0b0e13;
    --line: #232b37;
    --text: #dce3ed;
    --muted: #93a0b4;
    --dim: #6b7788;
    --dark: #0b0e13;
    --light: #f4f4f0;
    --chrome-dark: #2c313a;
    --on-dark: #c5c5c5;
    --on-light: #424242;
    --mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0; padding: 2.5rem 1.5rem 5rem;
    background: var(--bg); color: var(--text);
    font: 14px/1.6 var(--mono); color-scheme: dark;
  }}
  main {{ max-width: 62rem; margin: 0 auto; }}
  h1 {{ font-size: 1rem; font-weight: 600; letter-spacing: .04em; margin: 0 0 .4rem; }}
  .sub {{ color: var(--muted); margin: 0 0 2rem; max-width: 44rem; }}
  h2 {{ font-size: .8rem; font-weight: 600; text-transform: uppercase;
        letter-spacing: .08em; color: var(--dim); margin: 2.75rem 0 .9rem; }}
  .key {{ color: var(--muted); margin-right: .45rem; }}
  table {{ border-collapse: collapse; width: 100%; }}
  th, td {{ border: 1px solid var(--line); }}
  thead th {{
    color: var(--muted); font-weight: 400; font-size: .74rem;
    padding: .4rem .6rem; text-align: left; white-space: nowrap;
  }}
  thead tr:first-child th {{ color: var(--text); font-size: .78rem; }}
  tbody th {{
    color: var(--muted); font-weight: 400; font-size: .75rem;
    padding: .4rem .6rem; text-align: left; vertical-align: top; white-space: nowrap;
    width: 4.5rem;
  }}
  tbody th span {{ color: var(--dim); }}
  td.on-dark, td.on-light {{ padding: .5rem; vertical-align: top; width: 8.5rem; }}
  td.on-dark {{ background: var(--dark); }}
  td.on-light {{ background: var(--light); }}
  td.on-dark .mark {{ color: var(--on-dark); }}
  td.on-light .mark {{ color: var(--on-light); }}
  .mark {{ display: block; }}
  .bars {{ display: flex; gap: .5rem; align-items: stretch; }}
  .bar {{
    display: flex; align-items: center; justify-content: center;
    width: 3rem; height: 5.5rem; border-radius: 4px;
  }}
  /* the host's chrome, not ours: a simulated activity bar, so a dark bar is
     visible against a dark page */
  .bar.dark {{ background: var(--chrome-dark); color: var(--on-dark); }}
  .bar.light {{ background: var(--light); color: var(--on-light); }}
  .bar .mark {{ color: currentColor; }}
  .tiles {{ display: flex; gap: 1.5rem; margin-top: .5rem; }}
  .tiles figure {{
    margin: 0; background: var(--chrome-dark); border: 1px solid var(--line);
    border-radius: 10px; padding: .75rem .75rem .5rem;
  }}
  .tiles figcaption {{ color: var(--dim); font-size: .72rem; margin-top: .5rem; }}
  .notes {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: 1.75rem; }}
  .note {{ border-top: 1px solid var(--line); padding-top: .9rem; }}
  .note h3 {{ font-size: .82rem; font-weight: 400; margin: 0 0 .5rem; }}
  .note .what {{ color: var(--muted); font-size: .8rem; margin: 0 0 .6rem; }}
  .verdict {{ color: var(--dim); font-size: .78rem; margin: 0 0 .35rem; }}
  .verdict b {{ color: var(--muted); font-weight: 600; margin-right: .3rem; }}
  code {{ color: var(--text); }}
  footer {{ border-top: 1px solid var(--line); margin-top: 3rem; padding-top: 1rem;
            color: var(--dim); font-size: .76rem; max-width: 44rem; }}
</style>
</head>
<body>
<main>
  <h1>wcode identity \u00b7 round 1</h1>
  <p class="sub">Three candidate marks, each at every size the product actually
  draws. 16 px is the VS Code activity bar at DPR 1; 24 px is the icon file's own
  grid and the bar at DPR 1.5; 32 px a tab strip; 128 px the marketplace tile and
  the favicon. Each mark is one ink (<code>fill="currentColor"</code>, no hex) and
  is shown inheriting its colour from the surface, not from a fill in the file.
  Rows are sizes, so the row height is the size's own height; marks share a left
  edge down each column.</p>

  <table>
    <thead>
      <tr><th></th>{head}</tr>
      <tr><th></th>{subhead}</tr>
    </thead>
    <tbody>{''.join(body)}</tbody>
  </table>

  <h2>Mono-inherited at 24 px, on a simulated bar</h2>
  <div class="bars">{bars_dark}{bars_light}</div>

  <h2>Tiles, 128 px</h2>
  <div class="tiles">{tiles}</div>

  <h2>The three ideas</h2>
  <div class="notes">{notes}</div>

  <footer>Generated by <code>gen.py</code> in this directory; the eight SVGs are
  its output. The live marks are untouched: this is a candidate sheet, not a
  wiring round. Zero external requests; system font stacks only.</footer>
</main>
<svg style="display:none" aria-hidden="true">{''.join(symbol(k) for k in MARKS)}</svg>
</body>
</html>
"""


if __name__ == "__main__":
    main()
