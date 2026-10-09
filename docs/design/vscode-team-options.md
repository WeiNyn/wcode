# VS Code paper surface — the team list: three options

- **Status:** proposal, for a human to react to. **Not authorized** — nothing
  under `editors/vscode/` is touched and no spec is amended.
- **Prototype:** [`vscode-team-options.html`](vscode-team-options.html) — self-contained; the toolbar switches the three placements.
- **Shipped surface:** the paper page (W008) — `editors/vscode/media/chat.css`,
  `src/webview/chat.ts`, `src/webview/view.ts`.
- **History:** v3 gave the team a permanent rail; W005/D006 removed it and the
  team became a dim **caption** under the running head
  (`docs/decisions/D006-editorial-supersedes-v3-decisions.md`).
- **Date:** 2026-10-09

## §0 — surface read

> Reading this as: **a web screen (the VS Code webview)**, for **a developer
> reading a multi-agent session**, in **VS Code's own theme-variable language**,
> with the dominant constraint that **nothing may be a container that is not
> content — so the roster must read as page matter, not a status strip.**

## What the roster is today

One dim line under the running head: a flex row of `<span class="m">`, one per
member, each `glyph name action`. The CSS is `.masthead .team`
(`chat.css:pLaZr`), `.masthead .team .m` (`:50gNH`, with `:hover` `:XwsBt`,
`:focus-visible` `:mCZQd`, `.sel` `:hCMRr`) and `.act` (`:2A7UY`). The DOM is a
`<div id="team" class="team" role="list">` in the masthead skeleton
(`chat.ts:6Rpsp`, array `:Bnfe4`), painted by `renderCaption` (`:KWHuo`) from
`teamCaption` (`view.ts:uWZi8`, rows `:LXxM3`). **Each row is the retarget
affordance**: `tabindex="0"` + Enter/Space (`view.ts:DQHV6`)
→ `focus-member` (`chat.ts:KWHuo`). That key **must survive whatever moves.**

Its friction on the paper page: it is the one **status strip** left (it competes
with the folio for the head's first glance), it duplicates the **liveline**
(`view.ts:KeUWa`/`:psxWY` already reports who is active), and it prints the whole
cast on every screen even though the turn headings already name each speaker
(`view.ts:nsUTe`).

## Option 1 · Imprint — the status quo, refined

**Stance (one line):** the roster is the page's **imprint** — one quiet line of
names under the running head, read once and left alone.

```
  ⠋ orchestrator            ctx 42k   All Focus        12
  ⠋ orchestrator   ⠋ explorer   ○ developer   ✓ reviewer      ← the team
  ───────────────────────────────────────────────────────────
```

**What it costs.**
- **Space:** one line above the page, always. **Discoverability:** highest — it
  is the first thing on screen. **Retarget:** unchanged (the rows).
- The strip reading persists; the head carries identity + status + folio +
  roster, which is a lot for one line.

**Improvements available without moving it** (all *pure*, no DOM change):
- **active-first order** — sort by the liveline's predicate
  (`running || liveAction !== undefined`, `view.ts:KeUWa`);
- **drop the per-member `.act`** where the liveline already carries the action;
- **elide the idle tail** to a count (`+2`) — trades retarget for quiet.

**Touch-points.** `view.ts:teamCaption` (`:uWZi8`) + `CaptionRow` (`:LXxM3`) for
the ordering/fields; `chat.css:.masthead .team` (`:pLaZr`), `.m` (`:50gNH`),
`.act` (`:2A7UY`). No `chat.ts` change.

## Option 2 · Cast — the dramatis personae (paper-native)

**Stance (one line):** the roster is the play's **cast list** — a `CAST` block
printed once at the head of the text as front matter, then it scrolls away and
the liveline carries the live case.

```
  CAST
  ⠋ orchestrator      editing chat.css
  ⠋ explorer          grep onOpenDiff
  ○ developer         waiting
  ✓ reviewer          turn done
  ─────────────────────────────────────────────
  You
  Drop the rail and the spine. I want this to read like a page.
```

**What it costs.**
- **Space:** a block at the top of the transcript (you scroll past it once).
  **Discoverability:** highest on open (it is the first thing you read), lowest
  mid-read — there is **no persistent roster**; that is the point (the liveline
  covers *who is active now*). **Retarget:** the rows, now in the text.
- It is **content on the measure** — a cast list is part of a play's text — so it
  satisfies "a container that is not content" without a second column.

**Touch-points.**
- Move the roster out of the head: drop `<div id="team">` from the skeleton
  (`chat.ts:6Rpsp`, list `:Bnfe4`).
- `renderCaption` (`chat.ts:KWHuo`) becomes a **cast builder** that prepends to
  `.col` in `render` (`chat.ts:482`, where `col` is assembled).
- `mastheadSignature` (`chat.ts:L284G`) currently folds the caption into the
  masthead key — the cast needs **its own key** (or re-render with the transcript).
- Reuse `teamCaption` (`view.ts:uWZi8`) / `CaptionRow` (`:LXxM3`); keep
  `tabindex` + `isActivationKey` (`:DQHV6`).
- CSS: a new `.cast`; remove `.masthead .team` (`:pLaZr`); the `.m` block
  (`:50gNH`) moves under `.cast`.

## Option 3 · Margin — marginalia (paper-native, widest)

**Stance (one line):** the roster's **state is marginalia** — each member's
action set in the outer margin, aligned to the turn it belongs to; the names are
already the turn headings.

```
  ⠋ orchestrator                         ⠋ editing chat.css
  Done. The transcript is a single column ...
  ...
  ⠋ explorer                             ⠋ grep onOpenDiff
  The diff hook is at chat.ts:88.
```

**What it costs.**
- **Space:** zero in the column (it uses the margin). **Discoverability:**
  contextual — you see the state beside the turn you are reading, never the whole
  roster. **Retarget:** the margin note (or the heading) must take the key.
- A margin **is a second column**, which tensions the single-measure stance, and
  it needs width: below 560px it **falls back to Option 1**.

**Touch-points.**
- New `.mnote` markup in `renderTurn` (`chat.ts:KoM4E`); the action text from
  `CaptionRow.action` (`view.ts:uWZi8`) or `workingGroup` (`:KeUWa`).
- CSS: `.stream`/`.col` (`chat.css:wKAYF`) gain a margin; a narrow fallback in
  the shipped `@media` block (`chat.css:1106`).

## Comparison

| | where | carries | space | discoverability | retarget home | width |
|---|---|---|---|---|---|---|
| 1 · imprint | head line | glyph + name (+action) | a line, always | highest (first glance) | the rows | safe |
| 2 · cast | text front matter | glyph + name + role | a top block, then gone | highest on open, none mid-read | the rows | safe |
| 3 · margin | outer margin | the live action | none in the column | per-turn only | the note | editor only |

## Which fits the paper stance best, and why

**Option 2, the cast.** The paper stance is *nothing may be a container that is
not content* and *the words are the interface*. A cast list is **content** — a
play prints its dramatis personae as part of the text — so it lives on the
**measure** with the prose, with no second column and no box; and it turns the
roster from a **glance widget** into **read matter**, which is exactly the move.
Its one loss (no persistent whole-roster glance) is the least harmful here,
because the **liveline already reports who is active now** (`view.ts:KeUWa`) and
the **turn headings already name each speaker** (`view.ts:nsUTe`) — so the cast
never has to be on screen to tell you what is happening.

Option 3 is the most book-true *form* but breaks the one-measure stance and only
works at editor width; Option 1 is the safest and its improvements can ship on
their own today.

## Recommendation

1. **Now:** ship the **Option 1 improvements** (active-first, drop the redundant
   `.act`) — pure, no DOM change, quieter head.
2. **Target:** **Option 2, the cast** as the roster's home, keeping the liveline
   as the live case. This is the one proposal that both removes the strip and
   respects the single measure.
3. **Optional:** **Option 3** at editor width only, for a reader who wants the
   state beside the text.

## Open questions

- **Q1.** Cast, imprint, or margin — and does the cast replace the caption or sit
  beside it (caption as a *sticky* one-line cast once scrolled)?
- **Q2.** In the cast, is the per-member `role` the state (`waiting`) or the
  `liveAction`? The two differ (`view.ts:xXGU6`).
- **Q3.** Does the cast stay whole, or fold to one line as you scroll (needs
  scroll-spy — more JS, and motion-adjacent)?
- **Q4.** In margin mode, does the retarget key move to the note or stay on the
  heading?
- **Q5.** Is a roster still wanted at all, or are the headings + liveline enough
  (the pure Option-2 answer)?

## Facts NOT verified

- **No browser and no VS Code host were run.** The prototype was parse-checked
  (`node --check` on the inline script) and token-audited (tag/brace balance,
  locked glyph set, zero external refs, no rendered em-dash) — but **not painted**.
- **The `chat.ts` touch-points are read, not exercised.** `renderCaption` →
  a cast builder and the `mastheadSignature` split are proposals; the memo/sig
  interaction was not traced end-to-end.
- **The margin fallback is approximate in the gallery** (an *app* width stands in
  for the webview's own viewport); the real webview's `@media` governs there.
- **Not confirmed:** that the cast should be session-scoped; that `.act` should
  leave the roster; that the retarget key can move without a keymap change.
- **Not touched:** `editors/vscode/**` and every spec doc — this is a proposal.

## References

- [`vscode-team-options.html`](vscode-team-options.html) — the three placements, switchable.
- [`vscode-paper-direction.md`](vscode-paper-direction.md) — the paper stance, dials and refusals.
- `editors/vscode/media/chat.css` (`.masthead .team:pLaZr`, `.m:pLaZr`/`:50gNH`, `.act:2A7UY`, `.folio:E4MMN`, `.stream:wKAYF`).
- `editors/vscode/src/webview/chat.ts` (skeleton `:Bnfe4`, `#team:6Rpsp`, `renderMasthead:T33oI`, `renderCaption:KWHuo`, `mastheadSignature:L284G`).
- `editors/vscode/src/webview/view.ts` (`teamCaption:uWZi8`, `CaptionRow:LXxM3`, `isActivationKey:DQHV6`, `workingGroup:KeUWa`, `who:nsUTe`).
- `docs/decisions/D006-editorial-supersedes-v3-decisions.md` — the rail's removal.
