# TUI team sidebar — plan

Status: **S1 (glyph + live action, model dropped) in progress.** Revises decision
**D28** in [`team-and-tui-plan.md`](team-and-tui-plan.md). Presentation-only, inside
`crates/wcode-tui`.

## 1. Problem

The sidebar renders one line per member: `name · model · state`
(`ui.rs:draw_sidebar`, `app.rs:member_rows`). The **model column is the waste** — it is
usually identical across members, and over `--socket` it is *wrong* (every member gets
the root's model, `main.rs:452`). Meanwhile nothing shows *what a teammate is doing*;
`Surface` retains no tool/action state (`ToolExecutionStart` only pushes a `Block::Tool`).

## 2. Decision — Option 2: status glyph + live action, no model

One line per member: a **status glyph**, the **name**, and a dim **live action**:

```
┌────────────────────────┐  team
│ ● explorer    read src/…│
│ ○ developer   —         │
│ ✓ reviewer    done      │
└────────────────────────┘
```

- glyph: `●` running · `○` idle · `✓` done; row styled by state as today
  (`accent`/`dim`/`muted`), the focused member bold.
- **The model is dropped from the sidebar entirely.** The root keeps its full status
  line (D28's second half stands). This also makes the `--socket` model bug *moot for
  the sidebar* — the `Surface::model` field **stays** (it still seeds a member
  surface's status line, `app.rs:685`).

## 3. Data — a per-surface live action

- `Surface` gains `last_action: Option<String>`.
- Set on `AgentEvent::ToolExecutionStart { call_id, name }` (`Surface::apply`,
  `app.rs:750`) to `"{name} {target}"` (target optional). Cleared on `AgentStart`
  and `AgentEnd` (`app.rs:801`, `:806`) so it reflects the *current* run only.
- **Target resolution is UI-only, no protocol change.** `ToolExecutionStart` carries
  `call_id`; the committed assistant block `Block::Assistant(Vec<ContentBlock>)` holds
  `ContentBlock::ToolCall { id, arguments }` with the *same* id (the loop commits the
  assistant message before the tool runs). Match by `id`, then take the first present
  string argument among `path, file_path, file, pattern, command, cmd, query, url,
  name` (fall back to no target). Truncate to fit.
  - This is why the mock's `read main.rs` is reachable without widening
    `AgentEvent::ToolExecutionStart` — the arguments ride the assistant message.

## 4. Render

- `app.rs:member_rows()` → `Vec<(&str, TeamState, bool, Option<&str>)>`
  (label, state, focused, action); `/team`'s `team_text` updated to match.
- `ui.rs:draw_sidebar` builds `"{glyph} {name}"` + the dim action, clipped to the
  pane (`SIDEBAR_WIDTH = 26`); `state_style` unchanged; drop the `model` argument.
- Tests that hard-code the model in the row must change:
  `app.rs::the_team_command_lists_the_member_surfaces` and
  `ui.rs::the_team_sidebar_lists_members_and_hides_when_narrow` (plus the
  `with_member`/`set_surfaces` helpers' model arg).

## 5. Phases

- **S1 (this).** Glyph + live action; drop the model from the row. Tests: sidebar
  shows glyph + name and **not** the model; the action is set from
  `ToolExecutionStart` (target resolved from the assistant block) and cleared on
  `AgentEnd`; `/team` lists without the model. Commit
  `tui: sidebar shows member status + live action`.
- **S2 (separate) — the socket member-model fix.** See §6.

## 6. S2 — the `--socket` member-model bug (separate effort)

Root cause: `AgentEvent::Sessions { ids }` (`event.rs:116`) carries ids only, and
`SessionHandle` (`actor.rs:135`) holds no metadata, so the client cannot learn a
member's model — `main.rs:452/478` substitutes the root's. The clean fix **widens
`Sessions`** to carry records (`SessionInfo { id, model, label }`) and plumbs the
metadata into the `Registry`/`SessionHandle` (the real change), rippling through
`server.rs` (`roster_ids`, the three push sites), `client.rs` (`roster` watch,
`subscribe_roster`), `registry.rs`, `main.rs`, and the roundtrip tests. Tracked
separately; not part of S1.

## 7. Non-goals

Not the activity-log variant (Option 3) — the sidebar stays one line per member.
Not a change to the kernel or the wire in S1.
