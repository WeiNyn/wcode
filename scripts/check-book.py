#!/usr/bin/env python3
"""Validate the wcode book — the static site published to GitHub Pages.

Run locally before pushing:

    python3 scripts/check-book.py docs/book

Checks (all fatal, exit 1 on any failure):

  1. every page is present and `index.html` exists;
  2. HTML tags balance (void elements and XML self-closing tags excepted);
  3. no raw `&` outside an entity — the classic way a code sample breaks a page;
  4. every relative `href` resolves to a file in the book;
  5. every `#anchor` (same-page or cross-page) names an existing `id`.

No dependencies beyond the standard library, so CI needs no install step.
"""

import re
import sys
from pathlib import Path

# Elements that never take a closing tag. `style`/`script` are NOT here: inside
# an SVG they have a real closing tag, and the book uses them that way.
VOID = {
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link",
    "meta", "param", "source", "track", "wbr",
    # SVG shapes used live in the diagrams (always self-closed by convention,
    # but tolerate the HTML-style bare form too).
    "path", "rect", "circle", "line", "polygon", "polyline", "use", "stop",
}

TAG = re.compile(r"<(/?)([a-zA-Z][a-zA-Z0-9]*)([^>]*?)(/?)>")
RAW_AMP = re.compile(r"&(?!#?[a-zA-Z0-9]+;)")
HREF = re.compile(r'href="([^"]+)"')
ID = re.compile(r'id="([^"]+)"')


def check_page(path: Path, names: set[str]) -> list[str]:
    errors: list[str] = []
    text = path.read_text(encoding="utf-8")
    name = path.name

    for m in RAW_AMP.finditer(text):
        line = text[: m.start()].count("\n") + 1
        errors.append(f"{name}:{line}: raw `&` — escape it as &amp;")

    stack: list[tuple[str, int]] = []
    for m in TAG.finditer(text):
        closing, tag, _attrs, self_closed = m.groups()
        tag = tag.lower()
        line = text[: m.start()].count("\n") + 1
        if tag in VOID or self_closed:
            continue
        if closing:
            if stack and stack[-1][0] == tag:
                stack.pop()
            else:
                errors.append(
                    f"{name}:{line}: stray </{tag}>, open stack: "
                    + ", ".join(f"<{t}>:{ln}" for t, ln in stack[-3:])
                )
                if any(t == tag for t, _ in stack):
                    while stack and stack[-1][0] != tag:
                        stack.pop()
                    stack.pop()
        else:
            stack.append((tag, line))
    for tag, line in stack:
        errors.append(f"{name}:{line}: unclosed <{tag}>")

    ids = set(ID.findall(text))
    for href in HREF.findall(text):
        if href.startswith(("http://", "https://", "mailto:")):
            continue
        target, _, anchor = href.partition("#")
        if target and target not in names:
            errors.append(f"{name}: broken link `{href}` — no such file in the book")
            continue
        if anchor:
            owner = Path(target).read_text(encoding="utf-8") if target else text
            if f'id="{anchor}"' not in owner:
                errors.append(f"{name}: link `{href}` — no id `{anchor}` at the target")
    return errors


def main(argv: list[str]) -> int:
    root = Path(argv[1] if len(argv) > 1 else "docs/book")
    if not root.is_dir():
        print(f"check-book: no such directory: {root}", file=sys.stderr)
        return 1

    pages = sorted(root.glob("*.html"))
    if not pages:
        print(f"check-book: no pages under {root}", file=sys.stderr)
        return 1
    names = {p.name for p in pages} | {p.name for p in root.iterdir() if p.is_file()}

    errors: list[str] = []
    if "index.html" not in names:
        errors.append("index.html is missing — the site has no landing page")
    for page in pages:
        errors.extend(check_page(page, names))

    if errors:
        print("check-book: FAILED", file=sys.stderr)
        for e in errors:
            print("  " + e, file=sys.stderr)
        return 1

    size = sum(p.stat().st_size for p in pages) / 1024
    print(f"check-book: ok — {len(pages)} pages, {size:.0f} KB")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
