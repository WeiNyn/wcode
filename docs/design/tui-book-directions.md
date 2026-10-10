# Reading the transcript as a **book page** — five directions

- **Status:** draft, for a human to react to. **Not authorized** — no code, no
  `docs/tui-design.md` change. A pick amends that spec first (`design-taste` §7.3).
- **Companion to:** [`tui-turn-notes.md`](tui-turn-notes.md) — composes with what
  the human already picked: **A** (one block per turn), **(b) reverse-video
  speaker heads** (`YOU`/`WCODE`), the **H1 running head**, **R2 the ledger**.
- **The ask:** the earlier element-level options were *not enough* — make the
  **whole transcript read like reading a book**.
- **Touches no code.** Frames are hand-authored.
- **Date:** 2026-02-14

---

## 0. Surface read

> Reading this as: **a terminal surface (Medium A)**, for **a human reading a long
> session as a printed page**, in the locked wcode spec language, with the
> dominant constraint that **a terminal has no serif, no proportional type, and no
> real page** — so a "book" is assembled from the **idioms of printed matter**,
> and every idiom costs rows or columns.

## 1. The vocabulary, and the constraints that do not move

The book's idioms, mapped to what a terminal can carry: **front matter** (a title
page / a colophon line), **running heads + folio**, **chapter opens**, **drop
caps**, **the measure + margins**, **small caps + leading**, **marginalia**,
**footnotes**, **the colophon**, **verso/recto**.

Fixed for all five (cited, not invented):

- the **base** is the transcript over the composer, and the transcript renders in
  a **measure** (`docs/tui-design.md:14-19` "the transcript renders in a **measure**
  (a ≤68-col content column, centered once the band reaches 84 cols), while the
  composer stays full-width"); `crates/wcode-tui/src/ui.rs:58`
  `const MEASURE_MAX: usize = 68;` · `:62` `const MEASURE_MIN_BAND: usize = 84;`.
- the **gutter** — "a 1-col margin, a marker column, content at a fixed column"
  (`docs/tui-design.md:25`).
- the **glyph table** (`docs/tui-design.md:55-84`): a **new glyph needs a call
  site**; `❯` = the user prompt (`:57`), `—` = assistant prose (`:58`), `·` = the
  status separator (`:71`), `✓ note · note` = a tool done (`:64`).
- **no title bar** — "**Header/title bar**: decided **no**" (`docs/tui-design.md:351`).
  So a top head is a **scroll-away block** unless a direction **explicitly amends
  the spec** (each says which).
- the **composer** is the head line + a rule + the input + the foot line
  (`docs/tui-design.md:15-16`); the **folio** `N/M` rides its foot line
  (`crates/wcode-tui/src/ui.rs:1531-1537`), counting the surface's turns
  (`crates/wcode-tui/src/app.rs:1134-1136`).
- **≤1 `accent`**; **`NO_COLOR` reads**.

**Where the composer's head chrome goes if a direction moves it to a top head:**
the composer keeps the **input + foot line** (the gauge, state, folio) and loses
its head line — say so per direction.

---

## 2. Direction I · **The novel**

**Stance:** a novel's *dialogue* — the speaker heads are the spine; a **drop cap**
opens the chapter; a running head + a folio frame the page.

```
WCODE · session a1b2c3d4 ──────────────────────────── wcode · ⎇ main
────────────────────────────────────────────────────────────────────
 YOU    why does the anchor move when I reformat?

 WCODE  A  n anchor hashes the line's raw content, so indentation is
        part of its address. A reformat reindents the line, so the
        address changes. Re-read after a formatter.

        1  read   crates/wcode-cli/src/tools/edit.rs   128 ln  12ms

                                                          ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────── ⏻ plan · ⏸ idle   1/1
```

**Legend.** `WCODE`/`YOU` speaker heads = a **reverse-video** run
(`Modifier::REVERSED`, no role); the drop-cap `A` = the same `REVERSED` cell; the
header + rules `dim`/`border` (`wcode` `muted`); prose `body`; the ledger row
`dim` + `tool_name`; the folio `dim`.

**Spec amendment.** `docs/tui-design.md` §4 Transcript — a **session-head block**
(H1), a **speaker head** per message, a **drop cap** on the reply's first
paragraph; §2 — the **`§` row is removed**, `█ ▀ ▄` are **not** needed (the cap is
`REVERSED`).

**Cost / risk.** Drop cap `0` rows (a `REVERSED` cell); the header `2` rows
(scroll away). At **48 cols** the header sheds the session; the prose measure =
the band. `NO_COLOR` keeps `REVERSED` + the `─` rules.

**What makes it a book / what breaks it.** The *dialogue* (speakers + a cap +
running head + folio) is a novel's page; **breaks** if a reply is many paragraphs
(the cap loses meaning) or if the speaker name — not a character — reads as chrome.

---

## 3. Direction II · **The scholarly edition**

**Stance:** a critical edition — a **measure**, **numbered sections**, **turn-foot
footnotes**, and a **colophon**; the *apparatus is the point*.

```
 §1  the anchor is the raw line

   An anchor hashes the line's raw content, so indentation is part of
   its address.¹

   ── notes ──
   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms

   wcode 0.3.6 · gpt-5-codex · high · 14.2k / 272k · §1

                                                          ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────── ⏻ plan · ⏸ idle
```

`120×40` — the measure centers:

```
                                        §1  the anchor is the raw line

                                          An anchor hashes the line's raw content, so
                                          indentation is part of its address.¹

                                          ── notes ──
                                          1 » read  crates/wcode-cli/src/tools/edit.rs
                                            ✓ read · 128 lines · 12ms

                                          wcode 0.3.6 · gpt-5-codex · high · 14.2k / 272k
```

**Legend.** `§1` `heading`; the prose `body`; the `¹` `link`; the `── notes ──`
rule `dim`; the colophon `dim` (`muted` for `wcode`).

**Spec amendment.** §4 Transcript — the shipped **`§N` section head** (kept, not
the speaker), the turn-foot note list, and a **colophon block** (end matter);
§2 — the `§`/`¹²³` rows stay.

**Cost / risk.** Numbered sections + footnotes are the shipped shape (cheap); the
colophon is `1` row *at the end* (scroll-away). At 48 cols the measure = the band.

**What makes it a book / what breaks it.** A paper's page (measure, numbered
sections, footnotes, colophon) is *the* book of the academy; **breaks** if the
apparatus swamps the prose (the ledger helps) or if the reader wanted *dialogue*.

---

## 4. Direction III · **The chapter book**

**Stance:** a chapter per exchange — a **big chapter open**, **indent-led prose**,
the **gutter as a margin**.

```
 CHAPTER 3 · the anchor

     An anchor hashes the line's raw content, so indentation is part
 of its address. A reformat reindents the line, so the address
 changes. Re-read after a formatter.

   1 » read  crates/wcode-cli/src/tools/edit.rs · ✓ 128 lines · 12ms

                                                          ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────── ⏻ plan · ⏸ idle
```

**Legend.** `CHAPTER 3 · the anchor` `heading`; the indent-led prose `body`; the
note row `dim`/`tool_name`.

**Spec amendment.** §4 Transcript — a **chapter open** per exchange (the
`§N` → `CHAPTER N · title`); §1.4 — within-speaker paragraphs separated by the
**indent**, not a blank (the indent *frees* rows); §1.2 — the gutter *is* the
margin.

**Cost / risk.** The indent **trades** the between-paragraph blank for a first-line
indent (≈0 rows — the cheapest air); the chapter open is `2` rows. At 48 cols the
indent still fits.

**What makes it a book / what breaks it.** Indent-led prose + a chapter open + a
margin is a *printed chapter*; **breaks** if every exchange is a "chapter" (the
numbers race) or if the prose has no internal paragraphs.

---

## 5. Direction IV · **The pocket paperback**

**Stance:** dense and pocket-sized — a running head + folio, minimal air, the
**whole page fits**; the leanest reading.

```
wcode ⎇ main                     a1b2c3d4 · gpt-5-codex · 1/1
 YOU  why does the anchor move when I reformat?
WCODE An anchor hashes the line's raw content, so indentation is part
      of its address. A reformat reindents the line, so the address
      changes. Re-read after a formatter.
 1 read  edit.rs                 128 ln · 12ms

                                                          ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────── ⏻ plan · ⏸ idle
```

**Legend.** The running head `muted`/`dim`; the speaker heads `heading`
(or `REVERSED`); prose `body`; the note row `dim`; the folio `dim`.

**Spec amendment.** §4 Transcript — a **one-line running head + folio** (the head
folds `project ⎇ branch` + `session · model · N/M`); minimal leading (no blank
between a speaker and its text).

**Cost / risk.** Fits the most per page (no blanks); `NO_COLOR` fine. The risk is
**crowding** — a dense page is the opposite of the *novel*'s air.

**What makes it a book / what breaks it.** A paperback's *tight* page (a running
head, a folio, no waste) reads as a book you *carry*; **breaks** the paper stance
it composes with (**A**'s foot, the ledger) if the page is too tight to scan.

---

## 6. Direction V · **The illuminated manuscript**

**Stance:** the most decorated — a **drawn initial** per chapter, **marginalia**,
and **rules**; the page ornamented.

```
WCODE · session a1b2c3d4 ─────────────────── wcode · ⎇ main
────────────────────────────────────────────────────────────
 YOU   why does the anchor move when I reformat?

 WCODE ▄▀█      ── read ─────────
       █▀█ n anchor hashes the    crates/wcode-cli/src/
       ▀▀▀ line's raw content,     tools/edit.rs
                                   ✓ 128 lines · 12ms

                                                          ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────── ⏻ plan · ⏸ idle
```

**Legend.** The drawn initial `accent` (`█ ▀ ▄`); the marginalia `dim` under a
`── read ──` rule `border`; the header + rule `dim`.

**Spec amendment.** §2 — **add `█ ▀ ▄`** (call site: the chapter's first paragraph,
`crates/wcode-tui/src/markdown.rs` `wrap`/`flush_leaf` — a first-lines indent);
§4 Layout — a **right margin region ≥120** (R1, `tui-turn-notes.md` §10).

**Cost / risk.** The drawn initial **+2 rows** + a glyph amendment; the marginalia
needs **≥120 cols** (it folds to the foot below). At **48 cols** the initial eats
the measure; the margin is impossible.

**What makes it a book / what breaks it.** A manuscript's page (an initial +
marginalia + rules) is the *ornamented* book; **breaks** at 48/80 cols and against
the skill's "≤1 accent / minimalism".

---

## 7. Comparison + recommendation

| direction | spine | apparatus | air | new glyph | 48 cols | composes with |
|---|---|---|---|---|---|---|
| **I · the novel** | **(b) speaker heads** | R2 ledger | medium | none | header sheds the session | A, (b), H1, R2 |
| II · the scholarly edition | `§N` | turn-foot footnotes + a colophon | medium | none | fine | the shipped Preprint |
| III · the chapter book | a chapter open | the margin / a note row | **cheap** (indent) | none | fine | the Chapter move |
| IV · the pocket paperback | a running head | the ledger | **none** | none | fine | A, the folio |
| V · the illuminated manuscript | a drawn initial | marginalia | high | **`█ ▀ ▄`** | **breaks** | R1 |

**Recommendation: I · The novel, over II's substance.** In the skill's terms:

- **The human's asks converge on dialogue + a page.** The **novel** is the one
  direction that answers "reading a book" *and* uses the picks already made —
  **(b) reverse-video speaker heads**, **A** (one block per turn), **H1**, **R2** —
  so it needs **no new glyph** and **no new region**, only the drop cap
  (`REVERSED`, 0 rows).
- **II (the scholarly edition)** is the same *substance* the shipped Preprint
  already has (a measure, numbered sections, footnotes) **plus a colophon** — so it
  is the **fallback** that keeps `§N` if the human reverts the speaker heads.
- **III (chapter book)** is the **cheapest air** (the indent trades the blank);
  take it **if density** matters more than dialogue.
- **IV (pocket)** is the **densest**; take it for a *scan*, not for reading.
- **V (illuminated)** is the most decorated and the most expensive — an opt-in,
  wide-terminal mode.

**Compose the pick:** the **novel**'s page (H1 head + a `REVERSED` cap + `YOU`/
`WCODE` speaker heads + a ledger) on the **chapter book**'s **indent** (III's
cheap air) — dialogue *and* density, no new glyph.

## 8. What I did NOT verify (honest)

- **No terminal was run.** Every frame is **hand-authored** — including the
  **drawn initial (V)**, whose **width (`█ ▀ ▄`, 3-4 cells) is unmeasured**, and
  whose font coverage I did not check.
- **The measure positions are asserted, not measured** — the 120×40 centering (a
  26-col margin each side) is arithmetic; the drawn-initial and drop-cap
  interactions with **wrapping** are reasoned, not rendered.
- **48-col behaviour is asserted.** `Modifier::REVERSED` was **not painted**; its
  legibility on a light terminal is unverified.
- **No code changed; `docs/tui-design.md` not amended.**

## 9. References

- [`tui-turn-notes.md`](tui-turn-notes.md) — A (one block per turn), (b) the
  speaker head, H1 the running head, R2 the ledger, R1 the margin.
- [`tui-editorial-directions.md`](tui-editorial-directions.md) — §3 Preprint, §4
  Chapter, §5 Marginalia, §6 Broadsheet, §10 tables, §11 the frame, §12 composer.
- [`docs/tui-design.md`](../tui-design.md) §1 (the base + the measure), §2
  (glyphs), §4 (Transcript/Layout), §5 (no title bar).
- [`crates/wcode-tui/src/ui.rs`](../../crates/wcode-tui/src/ui.rs) — `MEASURE_MAX`
  (`:58`), `MEASURE_MIN_BAND` (`:62`), the composer (`:1362`), the folio (`:1531-1537`).
