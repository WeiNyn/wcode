# VS Code surface — exploration & design brief

**Status:** research + draft. The prototype is
[`vscode-ui-draft.html`](vscode-ui-draft.html) (open it in a browser, or read
it as the spec). This doc is the *why*; the HTML is the *what*.

**Surface read (design-taste §0):** a **web screen** (the VS Code webview), for
a developer driving a multi-agent session inside VS Code, in **VS Code's own
theme-variable language**, with the dominant constraint that the panel must use
**only `--vscode-*` tokens and zero remote assets**.

**Dials:** `VARIANCE` 3 (a tool panel, not a landing page — but enough
recomposition to fix the flatness) · `MOTION` 2 (spinner, live cursor, all
reduced-motion gated) · `DENSITY` 8 (a work surface).

The locked plan this refines: [`vscode-extension-plan.md`](../vscode-extension-plan.md)
(§3.2 client, §3.3 diffs, §3.4 approval, §3.5 team). The extension is a **second
client** of the kernel; nothing here changes the kernel.

---

## 1. What the shipped panel does today

Read from `editors/vscode/src/webview/chat.ts`, `.../webview/view.ts`,
`editors/vscode/media/chat.css`:

- A **status strip** (one line of `·`-separated segments: state dot, session,
  target, plan, running, `ctx N`, last error), the **transcript** (role-labelled
  blocks: `you` / `wcode` / `error` / `btw` / tool rows), and a **composer**
  (a textarea + `Send` + `Cancel` + a static hint).
- Tool rows collapse to `⚙ name summary duration` with a `show diff` button.
- Empty / starting / stopped / crashed / working states exist.

It is *correct* and *honest*. Where it is thin:

1. **Identity is a word.** Every block is a small uppercase `YOU` / `WCODE`
   label. There is no rhythm, no avatars, no message grouping, no sense of a
   conversation with *members*.
2. **The composer is a textarea and two buttons.** No `@`-mentions, no `/`
   commands, no model or mode control, no attachments, no context affordance.
3. **Approvals and diffs are off to the side.** The diff opens the native editor;
   there is no in-panel change review, and (P4) no gate at all.
4. **The team is a tree of names.** No "who is doing what right now", no per-member
   transcript in context, no agent map.
5. **The status strip is a sentence**, not a glanceable header.

## 2. How the field does it (grounded)

| tool | what it does that we should borrow | what to skip |
|---|---|---|
| **Claude Code** (VS Code ext) | A **prompt box** footed by a **permission-mode indicator** (Auto / Manual / Plan / Edit-automatically), a **model + effort** picker, and a **context indicator** with a **cache clock**. `@file#5-10` mentions from selection. `/` command menu. **Collapsed thinking** blocks (Ctrl+O expands all). An **agent map**: subagents as a tree with status, elapsed, tokens; click to read a transcript or stop. **Per-change** Accept / Reject buttons inside a side-by-side diff, plus accept/reject the whole file. A **Focus view** that folds tool calls into expandable rows. Sessions list in the Activity Bar with groups + search; the panel can **dock** (sidebar or an editor tab). A status-bar entry. | Its sign-in / OAuth surface; the plugin marketplace. |
| **GitHub Copilot Chat** | Inline **Apply / Insert** actions on code blocks; **`@participants`** and `/slash` commands; **inline chat** in the editor; review comments (**Accept / Reject**) on proposed diffs. | The participants model (wcode has real members, not personas). |
| **Cline / Roo Code** | A **Plan / Act** (or **mode**) toggle; **inline `Say`/`Ask` blocks with Approve / Reject buttons** for each action; **auto-approve** rules; a **token + cost** footer and a context gauge; **checkpoints / restore**; `@`-mentions, slash commands; a task **todo list**. | MCP / marketplace / checkpoint plumbing (out of wcode doctrine). |

Three patterns recur and are the ones wcode is *missing*:

- **A real composer**: mentions, commands, model + mode, context gauge.
- **Inline review**: approve / reject proposed changes *in place*, per change and
  per file, with the diff shown.
- **A live sense of the workers**: an agent map / roster with status and action,
  not a static list.

wcode has one thing none of them has: **a first-class team**. Claude's subagents
are the closest, but they are a by-product of one conversation. In wcode every
member is a session with its own transcript, addressable and steerable. **That is
the hero of this design**, not the chat box.

## 3. The design

### 3.1 One glyph vocabulary, shared with the terminal

wcode has a locked glyph table for the TUI (`docs/tui-design.md`). Reusing it in
the panel is a deliberate identity bridge, so the editor surface and the terminal
read as one product: `❯ ⚙ ✓ ✗ ⋯ ▌ ⠋ ● ○`. No invented glyphs.

### 3.2 The layout

```
┌ title bar ────────────────────────────────────────────────────────────┐
├──┬────────────┬───────────────────────────────────────────────────────┤
│  │            │  tabs:  ⌁ wcode   main.rs   diff                       │
│ A│  wcode     │  ┌ panel ────────────────────────────────────────────┐ │
│ c│  container │  │ header: ● ready · ❯ target ▾ · plan · mode · model │ │
│ t│            │  │         ctx ▰▰▰▱▱ 42k/128k · ⧗ 4m                  │ │
│ i│  TEAM      │  │ ─────────────────────────────────────────────────  │ │
│ v│   ● orch.  │  │ transcript                                          │ │
│ i│   ○ explor │  │   ...                                               │ │
│ t│   ✓ dev    │  │   [ inline approval card ]                          │ │
│ y│            │  │ ─────────────────────────────────────────────────  │ │
│  │  TASKS     │  │ composer: @chips  · textarea ·  toolbar             │ │
│  │  ☑ 3/5     │  └────────────────────────────────────────────────────┘ │
│  │  HISTORY   │                                                        │
├──┴────────────┴────────────────────────────────────────────────────────┤
│ status bar: ⎇ main   ✻ wcode · 3 members · 42k tok                     │
└──────────────────────────────────────────────────────────────────────────┘
```

- **Panel is dockable**: primary sidebar, secondary sidebar, or an **editor tab**
  (Claude parity, and the plan already says "a webview panel").
- **Sidebar = the team container**: Team (roster) · Tasks (todo) · History
  (sessions). The sidebar is the durable "who and what"; the panel is the active
  conversation.

### 3.3 The improvement levers, in order (design-taste §7.4)

1. **Typography & rhythm** — group a turn (thinking + tools + reply) into one
   unit with a left rail; drop the shouting uppercase labels for a quiet role
   chip; give code blocks a filename header and actions.
2. **Spacing** — an 8px grid; the header becomes two glanceable rows, not a
   sentence.
3. **Color** — **one accent** (`--vscode-charts-blue`) for wcode identity (the
   target member, tool names, links); member *state* uses the TUI's semantic
   glyph colors (green done, red error, muted idle); the diff uses VS Code's own
   diff tokens (semantic, not decorative).
4. **Motion** — the spinner (`⠋`) and the live cursor (`▌`); both gated.
5. **Recomposition** — the composer becomes a real control surface; the transcript
   gains the inline change-review and approval cards.
6. **New structure** — the Team screen (agent map + per-member transcript) and the
   reusable **change-review** component (diff → accept/reject).

### 3.4 New components (the deliverables the HTML prototypes)

- **Composer**: `@`-mention chips (file + line range from selection), a `/`
  command button, mode (`Plan`/`Act`), model+effort, a context gauge + cache
  clock, `Send` / `Stop`.
- **Change-review card**: a compact diff with **per-change Accept / Reject**
  (Claude parity) and file-level **Accept all / Reject all**; the "show native
  diff" escape hatch to the P2 `vscode.diff` provider.
- **Approval card** (P4): "wcode wants to edit `X`" with `Approve` / `Reject` /
  `Always allow <tool>`, a pending-count, and a mode matrix. This is the one
  place a *kernel* change is implied (a CLI `Hooks` impl + one Request/event
  pair) — the card is designed so the plan-gate is a strict subset of it.
- **Team screen**: the roster as a tree with state + action, and a member detail
  with its own transcript and `peek` / `ask` / `stop` (mirroring the `member`
  tool, never inventing verbs).
- **Plan review**: plan mode renders the plan as a document with inline comment
  affordances and `Approve & execute`, mapping to `Request::SetPlanMode`.
- **States**: empty / starting / working / crashed, with the stderr tail.

## 4. Open questions for review

1. **Panel home**: editor tab vs. secondary sidebar by default? (Claude defaults
   to the sidebar; an editor tab matches "a webview panel" and gives width for
   diffs.)
2. **Approval UX**: is the inline card the right shape, and does it earn a kernel
   change (P4), or does the plan-gate + a one-click native diff suffice?
3. **Team duplication**: roster in the sidebar *and* the panel header, or one?
4. **Change review in-panel vs. native diff**: which is primary? (The plan's P2
   chose the native diff editor; the field increasingly reviews in-panel.)
5. **Modes**: `Plan`/`Act` (Cline) vs. wcode's existing plan-mode toggle. Keep one.
