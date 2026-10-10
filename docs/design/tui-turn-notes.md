# Preprint's `§N` head & the notes — alternatives

- **Status:** draft, for a human to react to. **Not authorized** — no code, no
  `docs/tui-design.md` change. A pick amends that spec first (`design-taste` §7.3).
- **Companion to:** [`tui-editorial-directions.md`](tui-editorial-directions.md)
  §3 (Preprint) and §3(g) (the head pin). This doc **revises §3(g)'s pin**.
- **What it answers:** the human dislikes (1) the **`§N` section head** and
  (2) the **notes** — the tool output renders in the **wrong order and place**.
- **Also (§9-§17):** the human's newer ideas — a **drop cap**, **tool restyles**,
  a **book header** (and its **layout**, §17), and a **speaker head**
  (`YOU`/`WCODE`) that replaces `§N`.
- **Touches no code** — the shipped behaviour is *traced* below.
- **Date:** 2026-02-14

---

## 0. Surface read

> Reading this as: **a terminal surface (Medium A)**, for **a human reading a
> multi-round agent turn**, in the locked wcode spec language, with the dominant
> constraint that **the turn is split across several assistant messages and the
> fold is retrospective** — so the placement, the count and the numbering can all
> disagree.

---

## 1. What is wrong today (traced)

The shipped fold is in `crates/wcode-tui/src/app.rs` `commit_turn`
(`:1652-1671`). It **takes the apparatus out of the flow *first*, then pushes the
head + the reply + the notes** — `:1658-1666`:

```
        // Take the turn's apparatus out of the flow FIRST (it sits above the head),
        // then push the head + the reply, then re-home the apparatus at the foot.
        let notes = self.take_trailing_tools();
        self.turns += 1;
        let title = take_heading(&mut content);
        self.push_block(Block::TurnHead { n: self.turns, title });
        self.push_block(Block::assistant(content));
        if !notes.is_empty() { self.push_block(Block::Notes(notes)); }
```

`take_trailing_tools` (`:1689-1712`) walks **backward** over `Block::Tool` +
*text-less* `Block::Assistant`, and **stops at the first other block** (`:1700-1701`):

```
                Block::Assistant { content, .. } if !content_has_text(content) => {
                    start -= 1;
                }
                _ => break,
```

And a head is emitted **only for a reply that carries text** (`:1651-1652`
"A head is emitted **only for a reply that carries text**"), so a text-less
tool-call round folds into the *next* text reply's foot.

### The broken case — one exchange, two prose rounds

Trace `A(text+tc1) → T1 → A(text+tc2) → T2 → A(answer)`; after the answer commits,
the transcript is **three** sections, and **`T1` lands at `§2`'s foot** (the walk
stops at the text-bearing `A(text+tc1)`, `:1700-1701`):

````
 ❯ why does the anchor move when I reformat?

 §1  the anchor hashes the raw line

   An anchor hashes the line's raw content, so indentation is part of its
   address.¹

 §2  a reformat moves it

   A reformat reindents the line, so its address changes.¹

   ── notes ──
   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms

 §3  re-read after a formatter

   Re-read after a formatter; the new address is stable.

   ── notes ──
   1 » edit  crates/wcode-cli/src/tools/edit.rs
     ✓ edit · +3 −0 · 9ms
````

*(the transcript is taller than an 80×24 pane — it scrolls; the point is the
**shape**.)* Four defects, all visible:

1. **Three sections for one exchange.** The human reads one prompt; `§1 §2 §3`
   read as three.
2. **`T1` (`read`) is at `§2`'s foot** — but `§1` called it. The tool is attached
   to the **wrong** section.
3. **The `¹` ordinals disagree with the foot.** `§1`'s `¹` came from `tc1`
   (`read`) — but `§1` has no foot; `§2`'s `¹` came from `tc2` (`edit`) — but
   `§2`'s note **1** is `read` (T1). The reference and the note never match.
4. **`§3`'s note is unreferenced** — the answer carries no `ToolCall`, so the
   `¹` (`crates/wcode-tui/src/ui.rs:707-709`, emitted only at a `ToolCall`
   position) never appears, yet `edit` sits in its foot.

### The cause, in one line each

| symptom | cause | anchor |
|---|---|---|
| tools attach to the *wrong* section | the fold is **retrospective + tail-based**, stopped by any text-bearing block | `app.rs:1698-1701` |
| many sections per exchange | a head is **per text-bearing reply**, not per exchange | `app.rs:1651-1652` |
| `¹` ≠ the foot's number | `refn` is **per message**; the foot's `k + 1` is **per turn** | `ui.rs:685`, `ui.rs:742` |
| the tools **reflow** at commit | inline during the run, then popped + re-homed to a foot | `app.rs:1658`, `app.rs:1706-1708` |

---

## 2. A · **One block per turn** (recommended)

**Stance:** replace the `TurnHead` + `Assistant` + `Notes` trio with **one
`Block::Turn`** built for the whole *exchange*: the head at its top, **all** the
exchange's prose, and **all** its apparatus at the foot — so placement, order and
numbering are right **by construction**.

### The broken case, fixed

````
 ❯ why does the anchor move when I reformat?

 §1  the anchor is the raw line

   An anchor hashes the line's raw content, so indentation is part of the
   address.¹

   A reformat reindents the line, so the address changes.²

   Re-read after a formatter; the new address is stable.

   ── notes ──
   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms
   2 » edit  crates/wcode-cli/src/tools/edit.rs
     ✓ edit · +3 −0 · 9ms
````

### A simple turn

````
 ❯ how does edit resolve an anchor?

 §1  how edit resolves an anchor

   An edit now matches the quoted text literally, so there is no hash to
   resolve and no address to move.¹

   ── notes ──
   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms
````

**Legend:** the head `§N  title` `heading`; prose `body`; the `¹`/`²` reference
`link` (`crates/wcode-tui/src/theme.rs` `link`); the `── notes ──` rule `dim`; a
note head `   {n} » {name}  {target}` — `{n}` `dim`, `» {name}` `tool_name` — the
`▸`/`▣` `dim`; the `✓` summary `success`/`dim`.

**Seams.** `commit_turn` (`app.rs:1652-1671`) builds one `Block::Turn`; the
**whole exchange's** content is buffered (the run boundary = the user prompt) and
its tools collected in **call order** (`ContentBlock::ToolCall` order, not the
transcript tail); `take_trailing_tools` (`:1689-1712`) is deleted;
`block_lines` (`ui.rs:633-657`) gains a `Turn` arm (head + prose + `notes_lines`);
the reference ordinal (`ui.rs:685-709`) and `notes_lines`' `k + 1`
(`ui.rs:742`) become **one per-turn series**.

**Spec amendment.** `docs/tui-design.md` §4 Transcript — the block kinds
`TurnHead` / `Notes` **merge into one `Turn`**; §2 — `§` and `¹²³` keep their
rows but the **`§N` scope is the *exchange*** (one head per user prompt, not per
reply); §4 Layout unchanged.

**Cost / risk.** The **largest** change: the block model (one block per turn),
the `(rev, width)` cache/`ranges` (`app.rs` `append_block_lines`), browse
(`app.rs:1366-1369` "a `TurnHead` is chrome … never a selection target" — a
`Turn` mixes chrome + selectable prose), copy, and the D32 expand/collapse
(`app.rs:2292-2306`) all move from *blocks* to *sub-parts of one block*. The run
boundary must be known before committing (today it commits per message).

**Fixes / does not fix.** Fixes **all four** defects (§1). Does **not** fix:
nothing in scope — but it makes the head **per exchange**, which is what the
human wants (one section per prompt).

**Open question.** The head's title = the exchange's **first `# h1`** (the
opening) or the **final answer's** `# h1`? (Recommend: the first.)

---

## 3. B · **Fold tools into the reply that CALLED them**

**Stance:** keep the per-reply heads, but attach each tool to the **reply whose
`content` carried its `ContentBlock::ToolCall`** — so `T1` sits under `§1`.

### The broken case, after B

````
 ❯ why does the anchor move when I reformat?

 §1  the anchor hashes the raw line

   An anchor hashes the line's raw content, so indentation is part of its
   address.¹

   ── notes ──
   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms

 §2  a reformat moves it

   A reformat reindents the line, so its address changes.¹

   ── notes ──
   1 » edit  crates/wcode-cli/src/tools/edit.rs
     ✓ edit · +3 −0 · 9ms

 §3  re-read after a formatter

   Re-read after a formatter; the new address is stable.
````

**Legend:** as A. Note each `§` now carries **its own** `1 »` note.

**Seams.** `commit_turn` folds the tools **per reply** (the `ContentBlock`s of the
message being committed) instead of calling `take_trailing_tools`;
`take_trailing_tools` (`app.rs:1689-1712`) narrows to "the tools of *this* reply".

**Spec amendment.** `docs/tui-design.md` §4 Transcript — `Notes` stays, but is
**per reply**; no other change.

**Cost / risk.** **Small.** Keeps the block model, the cache, browse, copy. The
fold must be driven by the message's own `ToolCall`s, not the tail — and a tool
whose `ToolCall` is in an *earlier* message than its `ToolExecutionEnd` must land
with the caller (it has the id).

**Fixes / does not fix.** Fixes defect **2** (placement). Does **not** fix **1**
(still 3 sections), **3** (`¹` vs foot numbering still per-message) or **4**.

**Open question.** A text-less tool-call round still gets no head — so its tools
fold into an *earlier* reply's foot (fine) — confirm that is intended.

---

## 4. C · **No fold — the tool is a footnote in place**

**Stance:** drop the fold entirely; render each tool **where it already sits** in
the transcript, with the `¹` reference in the reply that called it. Ordering is
inherently correct and nothing reflows.

### The broken case, after C

````
 ❯ why does the anchor move when I reformat?

 §1  the anchor hashes the raw line

   An anchor hashes the line's raw content, so indentation is part of its
   address.¹

   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms

 §2  a reformat moves it

   A reformat reindents the line, so its address changes.¹

   1 » edit  crates/wcode-cli/src/tools/edit.rs
     ✓ edit · +3 −0 · 9ms
````

**Legend:** as A, but there is **no `── notes ──` rule** — the tool lives in place.

**Seams.** Delete `take_trailing_tools` (`app.rs:1689-1712`) and `Block::Notes`;
`Block::Tool` already renders via `notes_lines` (`ui.rs:654`
`Block::Tool(tool) => notes_lines(std::slice::from_ref(tool), width)`).

**Spec amendment.** `docs/tui-design.md` §4 Transcript — **`Notes` is removed**;
a tool is an in-flow block again; §2 unchanged.

**Cost / risk.** **Small** (a deletion). But the tool now sits **mid-turn**, so a
run interleaves prose and apparatus — noisier than a foot, and it forfeits the
"apparatus collects at the foot" stance (§3). Every tool is a full in-flow block
(collapsed by default).

**Fixes / does not fix.** Fixes **2** and **3** (a tool's `¹` and its own block
agree) and **4** (its `¹` is in the same reply). Does **not** fix **1** (still
3 sections) — the head is still per reply.

**Open question.** Does the apparatus belong in the flow at all, or does the
human want the paper's *foot* (§3)? C is the flow; A is the foot.

---

## 5. D · **The head** (drop it, or quiet it)

**Stance:** the human dislikes the `§N` head *itself*. Two variants, combinable
with A/B/C:

### D1 · No head at all

````
 ❯ why does the anchor move when I reformat?

   An anchor hashes the line's raw content, so indentation is part of the
   address.

   A reformat reindents the line, so the address changes.

   ── notes ──
   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms
````

### D2 · A quiet head — a title over a rule (no `§N`)

````
 ❯ why does the anchor move when I reformat?

   how an anchor resolves
   ──────────────────────────────────────────────────────────
   An anchor hashes the line's raw content, so indentation is part of the
   address.
````

**Legend:** D1 — no head; the exchange is set off by the blank between roles
(`docs/tui-design.md:42`). D2 — the title `heading_sub`, the rule `dim`.

**Spec amendment.** D1 — `docs/tui-design.md` §2: the `§` row is **removed**
(the `¹²³` row stays); §3(g) (this doc `tui-editorial-directions.md` §3(g)) is
superseded. D2 — the `§` row becomes a **rule + title** (no `§`); still no new
glyph (`─` is in the table).

**Cost / risk.** Tiny. But D1 loses the "a paper numbers its sections" stance and
the folio's `§N` series anchor (`tui-editorial-directions.md` §3(g) pin #4).

**Fixes / does not fix.** Fixes the **head** complaint only — the notes are
untouched (pair D with A/B/C).

**Open question.** Is the `§N` *glyph* the problem, or the **per-reply count**
(which A fixes)? (If it is the count, keep `§` and take A.)

---

## 6. Comparison + recommendation (the turn model)

| option | sections / exchange | tool placement | `¹` vs foot | reflow | change size | fixes |
|---|---|---|---|---|---|---|
| **today** | 3 (per reply) | wrong section | mismatch | yes | — | — |
| **A · one block/turn** | **1** | the foot | **aligned** | **none** | **large** | 1,2,3,4 |
| B · fold to caller | 3 | **right reply** | per-reply ok | yes | small | 2 |
| C · no fold (in place) | 3 | **in place** | **aligned** | **none** | small | 2,3,4 |
| D · quiet/drop head | 1 (D1) or 3 | — | — | — | tiny | head only |

**Recommendation: A (one block per turn), optionally with D2.** In the skill's
terms:

- **It fixes all four defects *by construction*** — the turn becomes one
  `Block::Turn`, so the count, the placement, the `¹`/foot numbering and the
  reflow cannot disagree. There is no "retrospective fold" left to get wrong.
- **It matches the human's model:** one *exchange* (one user prompt) = one
  section — the paper's `§1`, `§2`, … per prompt, not per model round.
- **It is the biggest change** (the block model + the cache/browse/copy/D32
  seams, §2), which is exactly why it should be decided **before** the last
  implementation slice.
- **C runner-up** — it fixes the *notes* with a deletion and no reflow, but
  leaves the section count; take **C + D** if the block-model change is too big.
  **B** is the smallest notes fix and fixes only the placement.

## 7. Spec-amendment list (the turn model)

- **A** — `docs/tui-design.md` **§4 Transcript** (`TurnHead`/`Notes` → one
  `Turn`); **§2** (`§`/`¹²³` rows kept; the `§N` scope is the *exchange*).
- **B** — **§4 Transcript** — `Notes` is **per reply**.
- **C** — **§4 Transcript** — `Notes` **removed**; the tool is in-flow.
- **D1** — **§2** — the `§` row **removed**; **D2** — **§2** — the `§` row → a
  rule + title. Both supersede `tui-editorial-directions.md` §3(g).
- **All** — no new glyphs (`─`, `»`, `¹²³`, `▸`/`▾`/`▣` are declared).

## 8. What I did NOT verify (honest)

- **No terminal was run.** The frames are **hand-authored from a code trace**, not
  captured. The **broken case is traced, not executed** — the trace follows
  `commit_turn` (`app.rs:1652-1671`), `take_trailing_tools` (`:1689-1712`) and
  `content_lines` (`ui.rs:674-727`) by reading them; I did **not** run the app.
- **The reflow claim** ("tools render inline then jump to a foot") is inferred
  from `take_trailing_tools` popping + re-homing (`:1706-1708`), not observed.
- **No code was changed**; the seams named are the functions I read, not edits.
- **`docs/tui-design.md` was not amended**; nothing outside this doc was touched.

## 9. Idea 1 · The **drop cap**

**Stance:** "set the first letter bigger, like an old book" — but **a terminal
cell is fixed-size** (`crates/wcode-tui/src/ui.rs:58` `const MEASURE_MAX: usize = 68;`
— a column is a column), so *bigger* must be **faked**. Five fakes, each with a
cost; all are the *reply's first paragraph*, under the §3(b) head.

### (a) A 3-row drawn initial (FIGlet-like)

````
 §1  the anchor is the raw line

   ▄▀█  n anchor is the text you quote — the old 5-char
   █▀█  hash is gone (D007), so a line's address is its
   ▀▀▀  own content.
````

*legend:* the drawn block-letter `accent`; the prose `body`. **Cost:** 3 rows × 3
cols, and the **first 3 lines indent** to clear it.

### (b) Reverse-video / filled-cell initial *(recommended)*

````
 §1  the anchor is the raw line

   A  n anchor is the text you quote — the old 5-char hash is gone
      (D007), so a line's address is its own content.
````

*legend:* the initial `A` is one cell with `Modifier::REVERSED` — the shipped
**selection** modifier (`crates/wcode-tui/src/ui.rs:602-604`
`fn selection_style() -> Style { Style::default().add_modifier(Modifier::REVERSED) }`).
**No new glyph, no `theme.rs` role** (a modifier, not a colour). It *pops* without
being bigger; `NO_COLOR` keeps `REVERSED` (a modifier, not colour).

### (c) Boxed initial

````
   ╭─╮
   │A│ n anchor is the text you quote — the old 5-char hash
   ╰─╯ is gone (D007), so a line's address is its own content.
````

*legend:* frame `border`. **Cost:** 3 rows × 3 cols (the frame glyphs are declared).
But the box is the *container* idiom §11 retired — it fights the doc's own stance.

### (d) Hanging initial in the margin

````
 A   n anchor is the text you quote — the old 5-char hash is
     gone (D007), so a line's address is its own content.
````

*legend:* the initial hangs in the **1-col margin** (`docs/tui-design.md:22-24`),
`heading`. **Cost:** 0 rows, **0 extra indent** — the gutter already indents. At
48 cols the margin is still 1 col, so it holds exactly one char.

### (e) Uppercase + bold (the small-caps substitute) *(cheapest)*

````
   AN ANCHOR IS THE TEXT you quote — the old 5-char hash is gone
   (D007), so a line's address is its own content.
````

*legend:* the first 3-4 words `body` + `Modifier::BOLD`. **Cost:** zero rows,
zero cols, zero glyphs — but not *bigger*.

| variant | +rows | +cols | glyphs | 48 cols | wrap interaction | `NO_COLOR` |
|---|---|---|---|---|---|---|
| **(a)** drawn | +2 | 3-4 | `█ ▀ ▄` (**new**) | eats ~12% of the 48-col band | first 3 lines indented | fine |
| **(b)** reversed | 0 | +1 | none (`REVERSED`) | fine | first line +2 | `REVERSED` survives |
| **(c)** boxed | +2 | 3 | `╭ ╮ ╰ ╯ │` (declared) | eats ~10% | first 3 lines | fine |
| **(d)** hanging | 0 | +0 | none | fine | none | fine |
| **(e)** caps | 0 | +0 | none | fine | none | bold survives |

**Spec amendment.** Any drawn initial adds `█ ▀ ▄` to `docs/tui-design.md` §2 (a
**call site**: the reply's first paragraph, `crates/wcode-tui/src/markdown.rs`
`flush_leaf`/`wrap` — a first-lines indent like §4's `p + p`). (b)/(d)/(e) add
**no glyph**.

**What it fixes / breaks.** Gives the "old book" look; **(a)** genuinely *is*
bigger but costs 2 rows + a glyph + a wrap indent, and eats the 48-col measure;
**(c)** fights §11; **(b)** is a pop, not a size; **(d)/(e)** are free but subtle.

**Recommendation: (b) reverse-video** (a real "bigger-looking" mark for ~0 cost,
no new glyph) — or **(e)** if even the reverse cell is too loud, **(a)** only if
the human accepts the rows + the glyph amendment.

---

## 10. Idea 2 · **Tool restyles** (beyond A/B/C)

**Stance:** the note apparatus is a *vertical stack*; three different shapes.

### R1 · A margin apparatus (≥120 cols)

````
 ❯ why does the anchor move when I reformat?

 An anchor hashes the line's raw content, so           notes
 indentation is part of its address.¹                  1 » read  edit.rs
                                                         ✓ 128 lines · 12ms
 A reformat reindents the line, so its address         2 » edit  edit.rs
 changes.²                                               ✓ +3 −0 · 9ms

 Re-read after a formatter.
````

At <120 the margin folds to the foot (R2). *legend:* margin rule `border`; the
head `muted`; `1`/`2` `dim`; `» read` `tool_name`; `✓` `success`.

### R2 · A one-line ledger (the foot, compact)

````
 §1  the anchor is the raw line

   An anchor hashes the line's raw content.¹  A reformat reindents it.²
   Re-read after a formatter.

   ── notes ──────────────────────────────────────────────────────
   1  read   crates/wcode-cli/src/tools/edit.rs     128 lines  12ms
   2  edit   crates/wcode-cli/src/tools/edit.rs      +3 −0      9ms
````

*legend:* `1`/`2` `dim`; `read`/`edit` `tool_name`; target `body`; the stats
right-aligned `dim`; `+3` `diff_add`, `−0` `diff_del`.

### R3 · A run-in footnote

````
 §1  the anchor is the raw line

   An anchor hashes the line's raw content.
   ¹ read  crates/wcode-cli/src/tools/edit.rs · ✓ 128 lines · 12ms
   A reformat reindents it.
   ² edit  crates/wcode-cli/src/tools/edit.rs · ✓ +3 −0 · 9ms
   Re-read after a formatter.
````

*legend:* the run-in note `dim`, the `¹`/`²` `link`, `read`/`edit` `tool_name`,
`✓` `success`.

| restyle | +rows | shape | fixes | breaks/costs |
|---|---|---|---|---|
| R1 margin | 0 (≥120) / foot (<120) | a right column | the foot entirely; note hugs the turn | a new region (Marginalia §5); dies with the sidebar |
| **R2 ledger** | −(N-1) vs a stack | **one row/tool** | the vertical sprawl; scannable | a row can't expand in place (params/body need a below-row fold) |
| R3 run-in | 0 | the note *is* the next line | reference→note distance | interrupts the prose (noisy); reflows |

**Spec amendment.** R1 — `docs/tui-design.md` §4 Layout (a right margin region
≥120); R2 — §4 Transcript (the note list becomes a **ledger** table); R3 — §4
Transcript (a note runs in after its reference). **No new glyphs.**

**Recommendation for idea 2: R2 (the one-line ledger).** It is the smallest change
that removes the vertical sprawl, keeps the `¹` reference, and needs no new
region — and it composes with **A** (one block per turn, §2): one head, one
prose, one **ledger**. R1 if wide, R3 only if the human wants notes *at* the
reference.

---

## 11. Idea 3 · The **book header**

**Stance:** move `project · ⎇ branch · session` (left) and `model · effort`
(right) to the **top**, separated from the transcript by a **rule**. But a *fixed*
top row is a **title bar**, and `docs/tui-design.md` **bans it** —
":322 `- **Header/title bar**: decided **no** — the **input box's corners** carry
session/project/branch and model/effort`". So two honest readings:

### H1 · A **scroll-away header block** (allowed) *(recommended)*

````
 wcode · ⎇ main · session a1b2c3d4            gpt-5-codex · high
 ──────────────────────────────────────────────────────────────
 ❯ why does the anchor move when I reformat?

 §1  the anchor is the raw line
   An anchor hashes the line's raw content, …
````

The header is the session's **first transcript block** (like Broadsheet's
masthead, `tui-editorial-directions.md` §6) — it **scrolls away**. The rule sits
**on the measure** (`crates/wcode-tui/src/ui.rs:416`
`let measure = if band >= MEASURE_MIN_BAND { MEASURE_MAX } else { band };`), not
full-bleed. The head **keeps the shipped three-`·` string**, under
`tui-editorial-directions.md` §9's head-row exemption (H1/H2 re-home the string;
neither changes it).

### H2 · A **fixed top band** (a genuine redesign — amends the spec)

````
 wcode · ⎇ main · session a1b2c3d4            gpt-5-codex · high
 ──────────────────────────────────────────────────────────────
 ❯ why does the anchor move when I reformat?
 …
 ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────────────── ⏻ plan · ⏸ idle   1/1
````

A **fourth band** above the transcript — it breaks "exactly three bands"
(`docs/tui-design.md:14-21`) and the title-bar ban. It must **amend the spec
first**; it is a real redesign, not a restyle.

**The composer's head line.** If the chrome moves to the top, the composer's head
line **loses** `project · ⎇ branch · session` / `model · effort`
(`crates/wcode-tui/src/ui.rs:1350-1436` `corner_titles` top row) — under the
**C** composer that line becomes a **bare `─` rule** (or is dropped, leaving only
the foot line). The **foot line** keeps the **gauge + state + folio** (`N/M`).

**Spec amendment.** H1 — `docs/tui-design.md` §4 Transcript (a session-head
block); **no** band change. H2 — §1.1 + §4 Layout: **a fourth band** — the one
genuine spec break in this whole doc.

**Recommendation: H1.** It gives the "book header" reading (a head + a measured
rule) **without** breaking the ban; H2 only if the human wants a real top band
and accepts a spec amendment.

---

## 12. Comparison + recommendation (the three ideas)

| idea | option | cost | fixes | verdict |
|---|---|---|---|---|
| 1 drop cap | **b reverse-video** | 0 rows, no glyph | the "old-book" look, cheaply | **recommend** |
| 1 drop cap | a drawn initial | +2 rows, `█ ▀ ▄` | genuinely bigger | only if the human pays |
| 2 tool render | **R2 ledger** | −(N-1) rows | the vertical sprawl | **recommend** |
| 2 tool render | R1 margin | 0 (≥120) | the foot | if wide |
| 3 book header | **H1 scroll-away** | 0 (a block) | a book header, measured rule | **recommend** |
| 3 book header | H2 fixed band | +1 band | a real header | **breaks the ban** |

**All three recommendations compose** with **A** (one block per turn): one
exchange → one `§` head with a **reverse-video cap**, its prose, and a
**ledger** note; the **scroll-away book header** above the transcript. None adds
a *band*; only **(a)**'s drawn initial adds a glyph.

---

## 13. Spec-amendment list (the three ideas)

- **Idea 1 (a)** — `docs/tui-design.md` §2 — add `█ ▀ ▄` (call site: the reply's
  first paragraph; `markdown.rs` `wrap`/`flush_leaf` gains a first-lines indent).
- **Idea 1 (b)/(d)/(e)** — **no glyph** (`REVERSED`/margin/caps).
- **Idea 2 R1** — §4 Layout — a right margin region ≥120. **R2** — §4 Transcript —
  the note list is a **ledger**. **R3** — §4 Transcript — a run-in note.
- **Idea 3 H1** — §4 Transcript — a session-head block. **H2** — §1.1 + §4 Layout —
  **a fourth band** (a genuine spec break).
- **All** — no new `theme.rs` role.

## 14. What I did NOT verify (honest)

- **No terminal was run.** Every frame here is **hand-authored** — including the
  drawn initial **(a)**, whose **width (3-4 cells) is unmeasured**, and whose
  `█ ▀ ▄` coverage across terminal fonts I did **not** check.
- **The 48-col interaction is asserted, not measured** — the "~12% of the band"
  figures are arithmetic, not a rendered result.
- **`Modifier::REVERSED` on a single cell** was **not** painted; its legibility on
  a light terminal is unverified.
- **No code changed; `docs/tui-design.md` not amended.**

---

## 15. A **speaker head** — the transcript as dialogue

**Stance:** drop `§1` entirely; open **each message** with a **speaker name** —
`YOU` (the user) / `WCODE` (the assistant) — so the transcript reads like a play
or a novel's dialogue. It replaces **both** the `§N` head and, in the transcript,
the `❯` marker (`docs/tui-design.md:57` `| user prompt | ❯ | accent, bold |`).

**The constraint, again:** a cell is fixed-size (`crates/wcode-tui/src/ui.rs:416`
`let measure = if band >= MEASURE_MIN_BAND { MEASURE_MAX } else { band };`), so a
"big name" is **faked**. Five fakes; the dialogue is a user message then a reply.

### Today (for contrast)

````
 ❯ why does the anchor move when I reformat?

 §1  the anchor is the raw line
   An anchor hashes the line's raw content, so indentation is part of its
   address.¹

   ── notes ──
   1 » read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms
````

### (a) A drawn banner (multi-row block letters)

````
█ █ █▀█ █ █
 █  █ █ █ █     YOU
 █  ▀▀▀ █▄█

   why does the anchor move when I reformat?

█ █ █▀▀ █▀█ █▀▄ █▀▀
█▄█ █   █ █ █ █ █▀▀     WCODE
▀ ▀ ▀▀▀ ▀▀▀ ▀▀   ▀▀▀

   An anchor hashes the line's raw content, so indentation is
   part of its address.¹
````

*legend:* the banner `accent`. **Cost:** `YOU` = 3 cols × 3 rows, `WCODE` = 5 cols
× 3 rows — **+3 rows per message** (+1 blank) — **8 rows of banner** for one
exchange. At **48 cols** the words fit (`WCODE` = 5 cols) but the 3 rows each are
the cost. **Glyphs:** `█ ▀ ▄` are **not** in `docs/tui-design.md` §2 → a §2
amendment with a call site (the speaker head, per message).

### (b) A reverse-video banner *(recommended)*

````
 YOU    why does the anchor move when I reformat?

 WCODE  An anchor hashes the line's raw content, so indentation is
        part of its address.¹

        ── notes ──
        1 » read  crates/wcode-cli/src/tools/edit.rs · ✓ 128 lines · 12ms
````

`120×40` (the name on the **measure**):

````
                                              YOU    why does the anchor move when I reformat?

                                              WCODE  An anchor hashes the line's raw content, so
                                                     indentation is part of its address.¹
````

*legend:* the name is one filled run with `Modifier::REVERSED` — the shipped
**selection** modifier (`crates/wcode-tui/src/ui.rs:603-604`
`Style::default().add_modifier(Modifier::REVERSED)`) — **no new glyph, no role**.
**Cost:** **1 row per message**; at **48 cols** `WCODE` (5) + a gutter still fits.
`NO_COLOR` keeps `REVERSED` (a modifier, not colour).

### (c) A bracketed label

````
 [ YOU ]    why does the anchor move when I reformat?

 [ WCODE ]  An anchor hashes the line's raw content, so indentation
            is part of its address.¹
````

*legend:* brackets `dim`, name `heading`. **Cost:** 1 row; `[` `]` are ASCII and
already in chrome (the composer corners). Fits 48 cols.

### (d) Bold uppercase + a rule (the quiet head)

````
 YOU
 ────────────────────────────────────────────────────────
   why does the anchor move when I reformat?

 WCODE
 ────────────────────────────────────────────────────────
   An anchor hashes the line's raw content, so indentation is part
   of its address.¹
````

*legend:* name `heading` (bold), rule `dim` **on the measure**. **Cost:** **2 rows
per message** (name + rule). Fits 48 cols.

### (e) A margin speaker (like a play's `HAMLET.`)

````
YOU    why does the anchor move when I reformat?

WCODE  An anchor hashes the line's raw content, so indentation is
       part of its address.¹
````

*legend:* the name hangs in the **gutter**, `heading`. **Cost:** 0 extra rows —
but `WCODE` (5 cols) needs the **gutter widened from 3 → 6** (`docs/tui-design.md:25`
"**Role lives in the left gutter.** A 1-col margin, a marker column"): a §1.2
amendment, and it eats 6 cols of a 48-col band.

### The name, and what it displaces

- **`WCODE` (the product), not `gpt-5-codex`** (the model id) — the model id is
  variable and already rides the composer; the speaker is the *character*
  (`app.rs:990` `pub model: String;` stays the composer's).
- **`YOU` replaces the transcript's `❯`** — the user block drops `❯`
  (`crates/wcode-tui/src/ui.rs:635`
  `Block::User(text) => wrap(text, width, " ❯ ", "   ", user()),`); `❯` **stays**
  as the composer's prompt.
- **The `§N`/`Block::TurnHead`** is dropped → a `docs/tui-design.md` §2 amendment:
  the **`§` row is removed** (the `¹²³` row stays).
- **The folio (`N/M`)** — §3(g) pin #4 anchored it on the `§N` series; the
  **counter survives** (`app.rs:1134-1136` `turns: usize` "the `§N` series"); only
  the *rendered* `§` goes. The folio keeps counting **exchanges** (`N/M`).
- **The notes** — unaffected; composes with **A** (one block per turn, §2) → a
  `WCODE` head + prose + a **ledger** (§10 R2).

| variant | +rows/msg | glyph | 48 cols | gutter | `NO_COLOR` | "book" |
|---|---|---|---|---|---|---|
| (a) drawn banner | **+3** | `█ ▀ ▄` (**new**) | fits, but 3 rows | ok | fine | **most** |
| **(b) reverse-video** | **+0** (1 row) | none | fits | ok | `REVERSED` lives | **yes** |
| (c) bracketed | +0 (1 row) | none | fits | ok | fine | mild |
| (d) name + rule | **+1** | none | fits | ok | fine | quiet |
| (e) margin speaker | +0 | none | **eats 6 cols** | **6 (was 3)** | fine | yes |

---

## 16. Comparison + recommendation (the speaker head)

| option | verdict |
|---|---|
| **(b) reverse-video banner** | **recommend** — one row, a filled **name**, no new glyph, 48-col-safe; the cheapest "character message" |
| (d) bold name + rule | the quiet alternative (2 rows) if a filled run is too loud |
| (e) margin speaker | 0 rows but **widens the gutter** to 6 — a real §1.2 amendment |
| (a) drawn banner | the **most** "book" and the **most** expensive (+3 rows/msg + a `█ ▀ ▄` §2 amendment) |
| (c) bracketed | legible, 1 row, but reads as a *tag*, not a *speaker* |

**Recommendation: (b) the reverse-video banner.** It is **the trade-off the
human asked for** — the name *looks* set apart (a filled run) for **one row**,
**no new glyph**, and it survives **48 cols**; it drops `§N` (`§` row removed)
without a drawn banner's +3 rows. **(d)** if the human wants the quietest; **(a)**
only if the human explicitly wants drawn letters and pays +3 rows/msg + the glyph
amendment.

**Spec amendments.** The `§` row is **removed** from `docs/tui-design.md` §2 (the
`§N`/`Block::TurnHead` goes); the user block drops `❯` (transcript only — §4 Input
unchanged); **(a)** adds `█ ▀ ▄` (call site: the speaker head); **(e)** widens the
gutter (§1.2). **(b)/(c)/(d)** add **no glyph**.

**What it fixes / breaks.** It gives the **dialogue** reading and drops the
disliked `§N`; the **count** the human may still want is the **folio**, not a
section mark. It **breaks** the paper's "numbered sections" stance (§3(g)) — that
is the point. **(a)** breaks the row budget; **(e)** breaks the narrow band.

**What I did NOT verify.** **No terminal was run** — the dialogue frames are
**hand-authored**; the drawn banner in **(a)** is an **approximation** whose width
is **unmeasured**, and `█ ▀ ▄` font coverage was not checked; the **(b)**
`REVERSED` run was not painted; the **(d)** rule is on the measure only by
arithmetic; **(e)**'s gutter-6 cost is asserted, not rendered. **No code changed;
`docs/tui-design.md` not amended.**

---

## 17. The **book header** — the layout

**Stance:** the human picked the **reverse-video speaker head** (§15 (b)) and now
wants the **top header** laid out left→right:

> `WCODE` → `session id`   `───────────`   `project` ← `branch`

— left: `WCODE` then the session; a **rule filling the middle**; right: the project
and the branch — a running head on a page. It is **§11 H1** (a scroll-away block —
the allowed reading); §11 H2 (a fixed band) is noted below.

**The header is a transcript block (§11 H1)**, one row + a `─` rule **on the
measure** (`crates/wcode-tui/src/ui.rs:416`
`let measure = if band >= MEASURE_MIN_BAND { MEASURE_MAX } else { band };` — at 80
cols the measure *is* the band; at ≥84 it is a centered 68-col column).

### Primary — H1, reading (i), model on the composer

````
WCODE · session a1b2c3d4 ──────────────────────────────── wcode · ⎇ main
────────────────────────────────────────────────────────────────────────
 YOU    why does the anchor move when I reformat?

 WCODE  An anchor hashes the line's raw content, so indentation is
        part of its address.¹
````

*legend:* `WCODE` is the **reverse-video** run — `Modifier::REVERSED`, matching
the speaker heads (§15 (b); `crates/wcode-tui/src/ui.rs:603-604`
`Style::default().add_modifier(Modifier::REVERSED)`), **no role, no glyph**;
`· session a1b2c3d4` `dim`; the **middle rule** and the **separator** `dim`/`border`;
`wcode` `muted`, `· ⎇ main` `dim`; `YOU`/`WCODE` speaker heads as §15.

**Two rules.** The human's layout has the rule *inside* the header row (the middle
fill) **and** "separated from the transcript by a rule" — two `─` runs. The
**single-rule** variant drops the separator (the header row's fill is the only
rule); offer both.

### 120×40 — the header on the measure

````
                              WCODE · session a1b2c3d4 ──── wcode · ⎇ main
                              ──────────────────────────────────────
                               YOU    why does the anchor move when I reformat?

                               WCODE  An anchor hashes the line's raw content, so
                                      indentation is part of its address.¹
````

The header **centers on the 68-col measure** (a 26-col margin each side), not
full-bleed — like the §10/§11 rules.

### Reading (ii) — the branch first (the arrow is ambiguous)

````
WCODE · session a1b2c3d4 ──────────────────────────────── ⎇ main · wcode
````

The human's `project ← branch` reads either way: **(i)** `project · ⎇ branch`
(project then branch — matches the shipped `join_title(project, branch)`), or
**(ii)** `⎇ branch · project` (the branch's `⎇` first). **Recommend (i).**

### Model/effort — (A) far right, or (B) composer-only

````
WCODE · session a1b2c3d4 ──────────────────── wcode · ⎇ main   gpt-5-codex · high
````

**(A)** appends `model · effort` past the branch as a **second right group**
(space-separated, its own one `·`); **(B)** keeps it on the composer only.
**The composer loses this chrome either way** (`crates/wcode-tui/src/ui.rs:1464-1499`
`corner_titles`' top row) — the composer's **head row is dropped** and it keeps the
**foot line** (the gauge + state + folio; `:1501-1539`, the folio `{n}/{n}` at
`:1531-1537`).

### Separators, and the ≤1 `·` rule — *relaxed*

The new groups each carry **one `·`**: `WCODE · session` (1) and
`wcode · ⎇ main` (1) — so the header is now **≤1 `·` per group *without* a
special case**, and `tui-editorial-directions.md` §9's head-row exemption (the
whole shipped head row carried **three `·`**) is **no longer needed** for the
header — the split *improves* on it. (Reading (ii) is the same: `⎇ main · wcode`
= 1 `·`.) The middle fill and the separator are bare `─` runs, no `·`.

### H2 — the fixed band (noted, not recommended)

Under **§11 H2** the header row + separator are **pinned above the transcript** (a
**fourth band**, `docs/tui-design.md:14-21`) — it *never* scrolls away. The layout
is identical; only the position changes. It **breaks the title-bar ban**
(`docs/tui-design.md:322`) and must **amend the spec first**.

### Spec amendment

- **H1** — `docs/tui-design.md` **§4 Transcript** — a **session-head block** (the
  header row + the measured rule). No band change.
- **H2** — **§1.1 + §4 Layout** — **a fourth band** (a genuine spec break).
- **Composer** — **§4 Layout** — the head row is **dropped**; the input + the foot
  line remain.
- **`·` rule** — `tui-editorial-directions.md` §9's head-row exemption **relaxes**
  (the header is ≤1 `·` per group).
- **No new glyph, no `theme.rs` role** (`WCODE` reuses `REVERSED`; `─` is declared).

### Cost / risk

| cols | behaviour |
|---|---|
| **≥84** | the header + rule center on the **68-col measure** (a 26-col margin each side) |
| **80** | the measure **is** the band — the header row spans the full 80 |
| **48** | `WCODE · session a1b2c3d4 ──── wcode · ⎇ main` ≈ 52 cols — **does not fit**; the shipped ladder drops the **session** first, then the branch |

**Rows:** H1 = **2 rows** (the header row + the rule) *in the transcript* — they
**scroll away**, so a long session pays them once. H2 = **+1 band** (it steals a
transcript row *every* frame). **The composer** loses its head row (**−1 row** at
rest) — a net wash for H1.

### What it fixes / breaks

**Fixes:** the "book header" reading — a running head with the content in the
middle and the page's identity at the edges; and it **drops the composer's head
row**, so the top chrome is the single place for `project`/`branch`/`session`.
**Breaks / risks:** under **H1 the chrome scrolls away** (once scrolled, no
`project`/`branch`/`model` is visible — the composer keeps only the gauge/state/
folio), and **the human loses the model id at a glance** unless **(A)** keeps it
in the header. **H2** breaks the ban. At **48 cols** the header row **must** shed
the session (and likely the branch), so it degrades to `WCODE ──── wcode`.

**Recommendation: H1, reading (i) `project · ⎇ branch`, model on the composer
(B), `WCODE` reverse-video, the single-rule variant.** It gives the running-head
look, **no new glyph**, **≤1 `·` per group** (relaxing the exemption), and it
respects the title-bar ban; **(A)** only if the human wants the model id in the
header, **(ii)** only if the branch should lead, **H2** only with a spec amendment.

**What I did NOT verify.** **No terminal was run** — the frames are hand-authored;
the **48-col overflow (≈52 cols) is arithmetic, not rendered**; the header's
**centered-measure position at 120 is asserted**, not measured; the `REVERSED`
`WCODE` run was **not painted**; and whether the **two-rule** form reads as heavy
is **untested**. **No code changed; `docs/tui-design.md` not amended.**

---

## 18. References

- [`tui-editorial-directions.md`](tui-editorial-directions.md) §3 (Preprint) and
  §3(g) (the head pin) — the pins this doc revises.
- [`crates/wcode-tui/src/app.rs`](../../crates/wcode-tui/src/app.rs) —
  `commit_turn` (`:1652-1671`), `take_trailing_tools` (`:1689-1712`),
  `turns` (`:1134-1136`), `Block::TurnHead` (`:133-141`).
- [`crates/wcode-tui/src/ui.rs`](../../crates/wcode-tui/src/ui.rs) —
  `block_lines` (`:633-657`), `content_lines` (`:674-727`), `footnote_mark`
  (`:725-731`), `notes_lines` (`:738-768`).
- [`docs/tui-design.md`](../tui-design.md) §2 (glyphs), §4 (Transcript/Layout).
