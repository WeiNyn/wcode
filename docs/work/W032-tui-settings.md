# W032 — an editable settings panel in the TUI — brief (v2)

- **Status:** approved — **human sign-off 2026-10-24** (session-scoped; the §10 fork resolved **No writer**: nothing persists), in implementation
- **Work item:** W032 (this file, rewritten in place from v1)
- **Decisions:** proposes **D036** — *the settings panel edits **session** state, never a file* (D035 is the highest on disk)
- **Reading edition:** [`W032-tui-settings.html`](W032-tui-settings.html)
- **Supersedes:** W032 v1 (read-only view + a `Request::Settings` getter, arguing *against* an editor). v1's recommendation is **withdrawn**; see §1.1.
- **Brief author:** `agent:brainstormer` (read-only). Recon #1 excerpted by `agent:explorer`, recon #2 supplied by the orchestrator; every anchor below was re-verified in that session.
- **Date:** set at sign-off — a read-only worker has no shell (`bash` is refused), so no date in this file is observed.

---

## 1. The ask

> "the setting here is setting edit as model, control by keyboard (arrows, enter, tab etc)"

An **editable, keyboard-driven settings panel**: a row cursor, arrows to move and change, `Enter` to act, `Tab` to move between groups — "edit as model" meaning editing should feel like the `/model` picker does today. The problem it solves: the TUI's knobs are scattered and only half-discoverable. Seven of them live on chords with no command (`app.rs:4032` `Alt-[`, `:4034` `Alt-<digit>`, `:4036` `Ctrl-G`, `:4037` `Ctrl-B`, `:4043` `Ctrl-O`, `:4046` `Ctrl-T`), and settings that *do* have commands (`/effort`, `/plan`, `/theme`, `/width`, `/view`) are one-shot verbs you must already know. There is no place that lists what is in effect and lets you change it in one gesture.

**Session scope, stated once and assumed everywhere below:** every row this panel edits is **session state that already has a live setter**. Nothing is written to `config.toml`, because nothing needs to be. That single sentence is why the editor is cheap (§3, §4.6). The one fork is in §10: *stick-for-next-launch* would be this repository's **first `config.toml` write path** and would double the item.

### 1.1 What v1 got wrong, and the refusal that survives

v1 read "a central setting" as "a place to *see* settings", recommended a read-only view, and argued against an editor on the grounds that an editor implies a writer. **That inference was wrong**: an editor over settings that already have live setters needs no writer at all. v1 was aimed at a target that does not exist — it priced a `config.toml` writer into a feature that does not require one, and used that price to refuse the feature the human actually asked for.

What survives, restated precisely:

> **No runtime `config.toml` writer.** Not because editing is refused — editing is the feature — but because the editable set and the config-file set barely overlap. Seven rows (model, effort, plan, theme, width, view, sidebar) are session settings with existing setters; the four that also appear in `config.toml` already have a runtime override path (`/model`, `/theme`, `/width`, `/view`). D016 already assigned persistence to *config*, and the session override to the runtime: "**Config only.** … nothing is persisted at runtime — `/width` and `Alt-[`/`Alt-]` keep their role as the *session* override" (`docs/decisions/D016-tui-width-setting.md:30`). This brief keeps that split exactly and merely gives the session half a face.

Also surviving from v1, and now **load-bearing** rather than a nice-to-have: the `Request::Settings` getter and the two `Status` staleness defects. An editor that shows a value it cannot re-read is worse than a read-only view — you would be pressing keys against a number that might be a lie. §4.5 settles this.

---

## 2. Scope / Non-scope

**In scope.**

1. A new TUI module `crates/wcode-tui/src/settings.rs`: a closed row taxonomy (`Toggle`, `Cycle`, `Nudge`, `Pick`, `ReadOnly`), the row list built from a pure snapshot struct, and the key grammar as a pure function.
2. A panel hosted on `App.settings: Option<SettingsState>` (§4.1 — the host decision), leaving `overlay` free so a picker can open **on top of** the editor without destroying it.
3. Seven editable rows, each delegating to a setter that already exists: `model` (picker), `effort` (cycle + off-list rule, §4.4), `plan` (toggle), `theme` (picker), `width` (nudge, clamped), `view` (cycle), `sidebar` (toggle).
4. One `ReadOnly` row with an honest note (`tool output` — there is no global flag) and one `ReadOnly` numeric (`context`).
5. A read-only `config (this client)` section: the finished `config_dump` string injected by the CLI, one `ReadOnly` row per line (§4.6).
6. `Request::Settings` → `AgentEvent::Settings { model, effort, plan, context_limit }`, which **settles** the panel's rows and fixes the two live `Status` defects (§4.5).
7. Pending marks for a change that the harness has accepted but deferred to the run boundary (§4.5).
8. Two entry points: `/settings` (in `COMMANDS`, so `/help` + completion + discoverability are free) and `F2` (one `KEYS` row, three dispatch sites — §4.7).
9. One line added to `config_dump` (`compaction budget`), which closes a tracked gap (`docs/next-steps.md:121`) for both the flag and the panel.
10. `editors/vscode/src/protocol.ts` gains the two wire union members. **No** settings UI in the extension.
11. Docs: D036 (§9), this file's §12 log, `README.md`'s command table, `docs/tui-design.md` §4 (the modal inventory), one `docs/next-steps.md` row.

**Non-scope** (each names something a reader would reasonably assume is in scope):

- **Any write to `config.toml` — the fork, §10.** No `Serialize` on `FileConfig` (`crates/wcode-cli/src/config.rs:567` is `Deserialize` only), no `toml::to_string` emitter, and every `fs::write` in that file is inside `#[cfg(test)]`. It is the repository's first writer, it needs its own decision record, and over `--socket` it would write the *client's* config while the agent lives on the server.
- **A typed numeric or text field.** There is no reusable single-line input: the composer is App-level `app.rs:2493` `input: Vec<Atom>,` + `:2495` `cursor: usize,` with private `&mut self` edit methods (`insert_char` `:4578`, `backspace` `:4612`, `delete_word` `:3113`) — not a widget, and not pointable at a second buffer without a refactor. The other two buffers are cursor-less `String`s (the picker's `query`, `app.rs:5184`–`:5187`; `BrowseSearch.query`, `app.rs:2423`). So: **nudge-only integers and a cycle-only effort**, with typed entry as a named follow-up. `/width <cols>` and `/effort <free text>` remain the escape hatches and are unchanged.
- **A global `tool-output` row.** Confirmed not a setting: `toggle_all_tools` (`app.rs:2803`) is a bulk write whose "all expanded" answer reads the blocks (`app.rs:2819`–`:2821`), new blocks default collapsed (`app.rs:2113`), and an errored one expands (`app.rs:1897`). The panel says so in a `ReadOnly` note instead of inventing a value.
- **Rows for the nine config tables with no TUI representation** — `[skills]`, `[instructions]`, `[retry]`, `[compaction]`, `[tools]`, `[models.<id>]` profiles (the TUI gets ids only, `lib.rs:287`), `[peers]`, `[workflow]`, `[orchestrator] guidelines`, `[hooks] rtk`. The panel shows what `--dump-config` shows and nothing more (§4.6); adding a table is its own item.
- **Provenance for anything but the four provider fields.** `ProviderProvenance` carries exactly `endpoint`/`base_url`/`api_key`/`model` (`crates/wcode-cli/src/config.rs:662`–`668`), `Source` a five-way enum (`:633`). No other setting records where it came from, so no session row may claim a source (§4.8).
- **Confirmations, "apply?", "restart required", per-row approval.** `AGENTS.md`: "Absent by design: MCP, permission prompts, approval flows".
- **A dirty/commit model.** Every setter applies immediately; `Esc` closes. §4.2 argues this is not a simplification but the only grammar that keeps the panel's one existing invariant (`Esc` closes, `app.rs:5164`).
- **A `SETTINGS` table that generates the `COMMANDS` dispatch** (v1's candidate C-f). Parked: `settings.rs` centralises the *enumeration* — the cheap half, and now the useful half — but `/`-command dispatch stays `COMMANDS` (`app.rs:747`), whose order is `/help`'s order.
- **An overlay stack.** Adding one would touch every opener; the host decision (§4.1) makes it unnecessary.
- **A live-updating panel that polls.** It rebuilds from `Status` each frame and settles on a `Settings` reply. No timer.
- **VS Code consuming the getter.** The TS union mirrors it; the extension shows nothing.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| **E1** | host + module skeleton (`settings.rs`, `App.settings`, precedence, draw) | **high** | medium | **medium** | The load-bearing decision: get it wrong and a picker row destroys the editor; medium risk because `on_overlay_key` swallows unknown variants **silently** (§5, trap 2) while `ui.rs:235` errors loudly. |
| **E2** | taxonomy + the four client-local rows (`width`, `view`, `sidebar`, `plan`) | **high** | medium | low | Every one delegates to an existing setter (`set_measure` `:2704`, `set_view_mode` `:5460`, `toggle_sidebar` `:5442`, the `/plan` arm `:4289`), so the row code is data plus a match, not new behaviour. |
| **E3** | `Pick` rows: model + theme | **high** | **low** | low | Host (c) makes these nearly free: the pickers already exist and their accept arms already do the right thing (`app.rs:5217`–`:5221`, `:5241`–`:5245`) — no new accept path, no new `PickerKind`. |
| **E4** | effort cycle + the off-list rule | medium | low | **medium** | Introduces the first effort vocabulary in the TUI (the kernel does not validate, `protocol.rs:262`), duplicated against `editors/vscode/src/tab.ts:69`; the failure mode is a cycle silently clearing a hand-set value. |
| **E5** | `Request::Settings` + the two `Status` defects | **high** | medium | medium | The editor's honesty floor: without it the panel's own rows can be stale (today `Status.effort` has no writer outside startup, and `context_limit` is seeded once). |
| **E6** | pending marks + settle-on-reply | medium | low | low | The harness defers settings mutations to the run boundary (`actor.rs:681`–`:691`), so a keystroke needs one visible state; `AgentStart`/`Error` clear it. |
| **E7** | read-only `config` section (injected string) + the `compaction budget` line | medium | **low** | low | One injected `String`, split into `ReadOnly` rows; zero config→display duplication, and it closes a tracked `--dump-config` gap. |
| **E8** | `F2` as a global chord | medium | low | **medium** | Three dispatch sites must stay in sync (composer/browse/search, mirroring `F1` at `:4035`/`:3272`/`:3309`) and a global chord is a real-terminal conflict surface. |
| **E9** | *fork:* stick-for-next-launch (config writer) | medium | **high** | **high** | Doubles the item (dirty state, which file, merge semantics, atomic write, `Serialize`) and its worst case is corrupting the one file a human cannot regenerate. §10. |

Reading the three together: **E3 and E7 are high-value/low-complexity — do them early to shrink the item's tail.** E1 is the only step where a wrong choice is expensive, so it carries its own gates and lands first. E9 is "low value / high complexity" *for this item* — the rubric says say so in the non-scope list rather than estimate it as if it were in scope, which is why it is §10's fork and not a step.

---

## 4. Interface & structure — the eight decisions

### 4.1 The host: `App.settings`, and here is why not the other two

**Decision: `App.settings: Option<SettingsState>`, with `overlay` left free.** Precedence becomes `overlay` → `settings` → `completion` → `Mode::Browse` → composer.

The constraint that decides it: `overlay` is a **single slot with no stack**. Every opener assigns unconditionally (`app.rs:5032` `self.overlay = Some(Overlay::Pick(picker));`, `:5109`, `:5119`, `:5135`), and accepting consumes it first (`app.rs:5203` `let Some(Overlay::Pick(picker)) = self.overlay.take() else {`). So a panel that *is* the overlay cannot host a picker.

| option | verdict | why |
|---|---|---|
| **(a)** `Overlay::Settings(SettingsState)` | **rejected** | Two sub-variants possible. *(a1)* A picker opens as `Overlay::Pick`, destroying the editor — needs a new overlay stack touching every opener. *(a2)* The variant carries its own `sub: Option<Picker>`, which keeps one slot but forces `accept_picker` to be refactored away from `self.overlay.take()` and forces the renderer to draw a modal *inside* a modal. Both are more code than (c) for no gain. |
| **(b)** `Mode::Settings` | **rejected** | `Mode` is the *composer-ownership* axis — `app.rs:2372` "The app's top-level input mode. `Input` is the composer (the default); `Browse` moves a selection over the committed transcript." A settings panel is a modal, not a third owner of the composer. It also sits **below** the completion popup in the precedence chain (`app.rs:3978`), so a stale completion could shadow it, and every `match self.mode` grows an arm. |
| **(c)** `App.settings: Option<SettingsState>` | **chosen** | The panel's lifetime is independent of any modal's, so `Enter` on the model row opens the ordinary `Overlay::Pick(Model)` and **the accept arm that already exists applies it** — `app.rs:5217`–`:5221` writes `status.model`, echoes a transient, and pushes `Request::SetModel`. Zero new accept code. Closing the picker reveals the still-open panel, whose rows are rebuilt from `Status` and therefore already show the new value. |

Precedent for a second App-level lifetime beside `overlay`: `app.rs:2561` `usage_collect: Option<UsageCollect>` — "A `/usage` fan-out in flight … `None` = no fan-out running."

Cost, stated plainly: a fourth tier in `on_key`, and one new field. That is the whole price of not building a stack.

### 4.2 The grammar — no dirty state, one cursor, no cancel

| key | action |
|---|---|
| `↑` / `↓` | move the cursor one row (clamped) |
| `PgUp` / `PgDn` | move the cursor one page (clamped) |
| wheel | move the cursor three rows (the transcript's wheel constant) |
| `←` / `→` | per row kind, §4.3 (`Nudge`/`Cycle` change; `Toggle`/`Pick`/`ReadOnly` are no-ops) |
| `Enter` | per row kind (`Toggle` flips, `Cycle` advances, `Pick` opens the picker, `ReadOnly` sets a one-line hint) |
| `Tab` / `BackTab` | jump the cursor to the first row of the *next* / *previous* section (`session` ⇄ `config`) |
| `Esc` | close — **the only close key**, unchanged |
| anything else | swallowed |

**There is no dirty/commit state, and that is a design constraint, not a shortcut.** `Esc` is unconditionally the close key today and a test pins it: `app.rs:5164` `Key::Esc => self.overlay = None, // the only close key (Q2)` and `app.rs:9532` `fn esc_closes_the_info_modal_and_nothing_else_does()`. A two-level cursor/edit state would force `Esc` to mean "cancel the edit" sometimes — breaking that invariant to buy a commit model **for setters that already apply immediately**. So: every act is immediate, `Esc` closes, and the invariant survives.

**Two invariants must be restated because this panel is where they diverge from `Info`:**

- **"`Esc` is the only close key" — preserved.** No new closer is added.
- **"`Enter` is swallowed" — variant-specific, deliberately broken here.** The `Info` invariant exists because of a trap: `accept_picker`'s `self.overlay.take()` would destroy the modal (`app.rs:9539`–`:9545`). In this host a picker's `take()` removes the *picker*, not the panel, so `Enter` is safe to act on. The `Info` test stays green untouched; a parallel test pins the Settings behaviour (§7.1).

**`Tab` is new grammar with no precedent and no conflict.** `Tab` is handled only in the completion popup (`app.rs:4417`); `BackTab` only in the composer, where it means `focus_prev` (`app.rs:4031`). Inside the panel both are free. But because the panel swallows keys, `BackTab` no longer switches surfaces while it is open — a real consequence, named in `KEYS` so it is documented rather than surprising.

### 4.3 The row taxonomy — five kinds, one key behaviour each

```rust
// crates/wcode-tui/src/settings.rs
pub enum SettingKind {
    Toggle { on: bool },
    Cycle  { value: String, options: &'static [&'static str] },
    Nudge  { value: usize, min: usize, max: usize, step: usize },
    Pick   { value: String, kind: PickerKind },   // Model | Theme
    ReadOnly { note: Option<&'static str> },
}
pub struct Setting { pub id: &'static str, pub label: &'static str, pub kind: SettingKind, pub pending: bool }
```

The panel is **data**: `settings::rows(&snapshot, &state) -> Vec<Section>` is a pure function over a snapshot struct (`model`, `effort`, `plan`, `theme`, `width`, `view`, `sidebar`, `context`, `config`, `running`), and the only match is over `SettingKind` — never over row ids. Applying a row returns a `SettingAction` that `App` executes against the setters it already has:

| row | kind | action on act | setter it delegates to |
|---|---|---|---|
| `model` | `Pick{ Model }` | open the model picker | `open_model_picker` `:5114` → accept `:5217` |
| `theme` | `Pick{ Theme }` | open the theme picker | `open_theme_picker` `:5107` → accept `:5241` |
| `effort` | `Cycle` | advance/retreat; `Request::SetEffort` | the `/effort` arm `:4252`–`:4261` |
| `plan` | `Toggle` | flip; `Request::SetPlanMode` | the `/plan` arm `:4289`–`:4304` |
| `width` | `Nudge{ 40..=200, step 8 }` | columns ± 8 | `set_measure` `:2704` (clamps), `MEASURE_STEP` `:2631` |
| `view` | `Cycle{ Focus, All }` | flip | `set_view_mode` `:5460` (no-op when unchanged) |
| `sidebar` | `Toggle` | flip | `toggle_sidebar` `:5442` |
| `tool output` | `ReadOnly` | hint line | — (no global flag, §2) |
| `context` | `ReadOnly` | — | from the `Settings` reply |
| `config …` | `ReadOnly` × N | hint: "read-only — `--dump-config`" | the injected string |

Four of the seven act purely client-side (`theme`, `width`, `view`, `sidebar`) and settle instantly — no request, no pending mark. Three push a `Request` (`model`, `effort`, `plan`) and can be deferred, hence §4.5.

The panel must work in both view modes: `toggle_all_tools` is a no-op in All (`app.rs:2805`) and browse is a no-op in All (`app.rs:3235`), but the settings panel brackets neither — it is a modal over whatever is behind it.

### 4.4 The effort vocabulary — one Rust const, mirrored, off-list preserved

Facts: the TUI has no effort list and no effort picker (`PickerKind` at `app.rs:629` has no `Effort` arm); `/effort` takes free text with three clear-synonyms (`"-" | "none" | "off"` → `None`, `app.rs:4255`); the kernel does not validate (`protocol.rs:262` `SetEffort { effort: Option<String> }`); the only vocabulary in the repository is the VS Code client's, `editors/vscode/src/tab.ts:69` `export const EFFORT_LEVELS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const;` — with its own caveat one line above (`:68`): "the Responses enum; Chat is free-style".

**Decision:** a `pub const EFFORT_LEVELS: &[&str]` in `settings.rs` holding those seven, with a doc comment naming `editors/vscode/src/tab.ts:69` as its twin and repeating the free-style caveat; the TS side gains the reciprocal comment. The cycle offers `(none)` + the seven. **An off-list value is preserved and shown verbatim** (`effort  medium-high`), and any arrow press moves to the first list entry (`(none)`), with the new value visible in the row — a deliberate action with a visible result, not a silent clamp.

Rationale for not validating in the kernel: adding validation is a *behaviour* change (a currently-accepted value would start failing), and behaviour is non-scope; the list belongs to a presentation layer, which is what `settings.rs` is. The duplication is real and named: one const in Rust, one in TS, each pointing at the other.

### 4.5 The two `Status` defects, and what a row does during a run

**The defects, re-verified.** `Status.effort` (`app.rs:1235`) has no writer outside the two startup seeds (`main.rs:1099`, `:2013`) — `/effort` pushes the request and nothing else (`app.rs:4252`–`:4261`) — while the header prints `model · effort` (`ui.rs:838` `let effort = head.effort.as_deref();`). `Status.context_limit` (`app.rs:1240`) is likewise seeded once (`main.rs:2017`) and never refreshed, so the gauge's denominator is stale after any `/model` switch. **Both are display lies, and the second one the panel would otherwise inherit as a `ReadOnly` row.**

**Fix: read truth.** `Request::Settings` → `AgentEvent::Settings { model, effort, plan, context_limit }`, answered from the agent's own state — `agent.effort()` (`agent.rs:178`), `agent.llm()` (`agent.rs:471`, `LlmOpts.model`/`effort`, `streamfn.rs:90`/`:106`), `agent.plan_mode` (`agent.rs:47`) with `PlanModeHandle::get()` (`hooks.rs:75`), and `crate::limits::model_limit(base_url, model)?.context` (`limits.rs:63`) for the window. Mid-run it answers from the read-only side handle (`actor.rs:530` `let side = agent.side_handle();`), plus a `PlanModeHandle` clone taken beside `let hooks = agent.hooks().clone();` (`actor.rs:528`). Idle it uses `agent` directly, joining the arms at `:431`/`:437`/`:465`/`:488`, and it **must** also claim a slot in the exhaustive tail at `:497` (`Request::ListSessions | Request::ListModels | Request::Unknown => AgentEvent::Ack,`) or the crate will not compile. Additive on the wire, so `PROTOCOL_VERSION` stays 1 — the argument already written out at `protocol.rs:316`–`:319` for `Usage`.

**What a row does while a run is in flight.** The harness *accepts* a settings mutation mid-run and applies it at the boundary: `actor.rs:681`–`:691` sends `AgentEvent::Ack` then `deferred.push_back(Message::Tell { from, request: req })` for `SetModel | SetTitle | SetEffort | SetPlanMode`. So all three Request rows are **optimistic with a pending mark**:

- On act, if `self.running()` (`app.rs:5502`, reading `Surface.running`), the row is marked `pending` and the intended value is written to `Status` optimistically (as `/plan` already does at `:4303`, and as the model picker already does at `:5218`). Render: `effort  high · pending` — the mark in `dim`, never a hue (`error` red is the only hue, `docs/tui-design.md` §1).
- Retired by **truth**, in four ways (amended after re-review 2 — the original "cleared on any `Error`" was both wrong and unimplementable):
  - **Settled** when a `Settings` reply's field **equals the stored intent** — the intent, not `Status`: `Surface::apply` has already written the reply there, so comparing against `Status` is true by construction.
  - **Cleared for all rows on `AgentStart`** — a new run means the previous boundary passed and every deferral applied.
  - **Cleared on an IDLE reply that does not confirm the intent.** This is the failure path: a deferred mutation that fails at the boundary emits *nothing* (the re-queued `Tell`'s `Err` is returned, never broadcast — `actor.rs:521`, `:308`), so an intent comparison alone would keep the mark forever while the row claimed a change was in flight that had been refused. Once `!self.running()` the harness has drained every deferral, so an unconfirmed mark is dead.
  - **`Error` alone does not clear**: a `/btw` failure reaches the reducer with truthful marks, and "clear the marks it can explain" is undecidable from `AgentEvent::Error { message }` (no request correlation).
- **This reverses v1's position** that `/effort` should not write `Status` optimistically. v1's reasoning was that the *command* had no on-screen editor, so showing the target before it took effect was the more deceptive option. With a panel, the user pressed `Enter` on a row they can see: a panel that does not echo its own keystroke reads as broken. The pending mark is what makes the echo honest — and it is available precisely because the `Settings` reply now exists to settle it.

Refusal-while-running was rejected (a run can be long, and mid-run model/effort changes are the harness's explicitly supported path); showing old truth until the boundary was rejected (it makes the panel feel dead).

### 4.6 The read-only `config` section: the finished string, not a refactor

`config_dump` is a flat builder — `main.rs:504` `fn config_dump(cfg: &Config) -> String {`, `let mut out = String::new();`, one `writeln!` per field (`:507`, `:533`, `:541`, …), table loops at `:570`/`:579`, `return out` at `:589`. It is not structured.

**Decision: inject the finished string (`Options.config: String`), split it on `\n` into `ReadOnly` rows.** Not a `Vec<ConfigRow>` refactor.

Why: (i) the section is never edited, so it needs no columns and no cursor *semantics* — but making each line a `ReadOnly` row keeps the panel's single-cursor/`Tab` grammar uniform instead of inventing a second scroll region with `Info`'s `scroll` field and its two tested clamp rules (`app.rs:9553`, `:9911`); (ii) a structured refactor would rewrite a 90-line builder whose exact output is a user-visible contract (`--dump-config`), risking the one thing this item must not break, for cosmetics; (iii) `config_dump` already prints `(source: …)` for the four fields that have provenance, so the panel inherits provenance without claiming any of its own. A `Vec<ConfigRow>` refactor is named as a follow-up if the panel proves cramped — not now.

One line changes in the builder: `compaction budget: …` (the tracked gap). `--dump-config`'s output therefore changes **deliberately and by exactly one line**, and that is the gate (§6).

Both TUI constructions inject the string — `main.rs:2031` (local) and `main.rs:1118` (`--socket`) — and the socket one labels its section `config (this client)` because the server's config is not ours.

### 4.7 `/settings` **and** `F2`

v1 said command-only. With an editor, that is too slow for a panel you use while working, so: **both**.

- `/settings` in `COMMANDS` (`app.rs:747`) with `args: None`, summary `"edit the session settings"`, placed after `view` so the settings cluster reads contiguously. `/help` and the completion popup are free — `help_rows()` derives from `COMMANDS` (`app.rs:1119`).
- `F2` as a global chord. `F1` is the only function key in `KEYS` and has a precedent in **three** dispatch sites that must all be mirrored: composer `app.rs:4035` `Key::F(1) => self.open_help(),`, browse `:3272` `Key::F(1) | Key::Char('?') => self.open_help(),`, browse-search `:3309`. So the cost of a chord is exactly: one `KEYS` row (`app.rs:1056`), three one-line call sites, and three tests. `Ctrl-S` was rejected (XOFF in many terminals); `F2` is unused and pairs with `F1` semantically.

### 4.8 What the panel refuses to claim

- **No provenance on any editable row.** It shows values, plus *durability* facts it can actually know ("session — not persisted", "client-local — not persisted"). It never says "config.toml" or "flag", because `ProviderProvenance` covers four fields (`config.rs:662`–`668`) and the TUI cannot distinguish a `--model` flag from a `/model` override.
- **No rows for the nine config tables with no representation** (§2). The `config` section is exactly `--dump-config`'s scope.
- **No `tool-output` value** — it is per block, so the row carries the key (`Ctrl-T`) and says so.
- **No claim to know the server's config.** Over `--socket`, the `config` section is labelled `(this client)`; only the session rows are server truth, and only because of `Request::Settings`.
- **No "pending" without a reply path.** A client-local row never shows `pending`; only the three Request rows can (§4.5).

### Integration points (re-verified; a line moves if the line itself changes — the snippet pins it)

**Added**

- `crates/wcode-tui/src/settings.rs` — `SettingKind`, `Setting`, `Section`, `SettingsState { cursor }`, `SettingsSnapshot`, `rows(&snapshot, &state)`, `on_key(state, key) -> Option<SettingAction>`, `EFFORT_LEVELS`.
- `crates/wcode-harness/src/event.rs` — `AgentEvent::Settings { model, effort, plan, context_limit }`, beside `Status` (`:169`).
- `crates/wcode-harness/tests/loop_tests.rs` — `fn tag`, the workspace's **one total `AgentEvent` match** (27 arms, no `_`), so a new variant is a compile error there: it gains `AgentEvent::Settings { .. } => "settings",`.
- `docs/decisions/D036-tui-settings-panel.md` — §9.

**Changed**

- `crates/wcode-harness/src/protocol.rs` — `Request::Settings` beside `Status` (`:302`) / `Usage` (`:319`); `is_inbound` (`:44`) stays false for it.
- `crates/wcode-harness/src/actor.rs` — an idle dispatch arm; a mid-run arm beside the `Usage`/`SideAsk` arms (`:642`–`:669`); a `PlanModeHandle` clone beside `:528`.
- `crates/wcode-tui/src/app.rs` — the `settings` field; the `overlay` → `settings` tier in `on_key` (`:3971`); `on_settings_key`; `open_settings`; `apply_setting`; a `Surface::apply` arm for `AgentEvent::Settings` (the match ends `_ => false,` at `:2036`, so it must be explicit); `pending` clearing on `AgentStart`/`Error`; the `COMMANDS` entry; the three `F2` sites; one `KEYS` row.
- `crates/wcode-tui/src/ui.rs` — **no `draw_overlay` arm** (the host decision moved the panel off `Overlay`, so there is no exhaustive match to extend and **no compiler guard** — first-layer A8: the tripwire is a pixel test, `the_settings_panel_draws_below_a_picker_and_above_the_bands`), a `draw_settings` call in `draw` *before* `draw_overlay` (`:175`), and `draw_settings` beside `draw_info` (`:303`) reusing `draw_info_panel` (`:347`).
- `crates/wcode-tui/src/theme.rs` — `active_name()` (the palette is swappable at `theme.rs:126` but its name is not readable).
- `crates/wcode-tui/src/lib.rs` — `Options.config: String` (the `Options` seam, `:283`); the `settings` field's construction.
- `crates/wcode-cli/src/main.rs` — `Options.config` at `:2031` / `:1118`; the `compaction budget` line in `config_dump` (`:504`).
- `editors/vscode/src/protocol.ts` — `| { type: "settings" }` in `Request` (`:51`) and the event member (`:118`ff).

**The anchor table**

| anchor | the fact |
|---|---|
| `app.rs:483` / `:507`–`:510` | `pub enum Overlay {` … `Info { title, rows, scroll }` — three arms, no cursor, a single slot |
| `app.rs:5032`, `:5109`, `:5119`, `:5135` | openers assign `self.overlay` unconditionally |
| `app.rs:5203` | `let Some(Overlay::Pick(picker)) = self.overlay.take() else {` — accept consumes the slot |
| `app.rs:2372`–`:2378` | `Mode` doc + `Input`/`Browse` — the composer axis (rejects host (b)) |
| `app.rs:3969`–`:3986` | `fn on_key`: `overlay` (`:3971`) → `completion` (`:3978`) → `Mode::Browse` (`:3983`) → composer — where `settings` slots in |
| `app.rs:5152`, `:5164`, `:5177`, `:5203` | the only `self.overlay = None`/`take()` sites — nothing else clears a modal; no `AgentEvent` does |
| `app.rs:5149`–`:5175` | `on_overlay_key`'s `if matches!` + `_ => {}` — a new variant swallows every key **silently** |
| `ui.rs:235`–`:242` | `match app.overlay()` is **exhaustive** — a new `Overlay` variant is a compile error (the good trap) |
| `app.rs:9532`–`:9548` | `esc_closes_the_info_modal_and_nothing_else_does` — `Esc` closes; `Enter` is swallowed *because* of the `take()` trap |
| `app.rs:8663` | `f1_opens_the_help_overlay_and_only_esc_or_f1_closes_it` — the chord precedent for `F2` |
| `app.rs:4035` / `:3272` / `:3309` | the three `F(1)` dispatch sites `F2` must mirror |
| `app.rs:4031` | `Key::BackTab => self.focus_prev(),` — `BackTab`'s composer meaning, shadowed while the panel is open |
| `app.rs:4417` / `:4409` | `Tab`/`BackTab` exist only in the completion popup |
| `app.rs:1056` | `pub(crate) const KEYS: &[(&str, &str)] = &[` — one row for `F2` |
| `app.rs:1119`, `:8318`, `:8321`–`:8325` | `help_rows()` derives from `COMMANDS`; the count assertion derives too, but the **hardcoded name list** does not and omits `view` today |
| `app.rs:2493`–`:2495` | `input: Vec<Atom>,` / `cursor: usize,` — the composer is not a reusable widget (rejects a typed field) |
| `app.rs:5184`–`:5187`, `:2423` | the two cursor-less `String` buffers |
| `app.rs:2704`–`:2716` | `set_measure` clamps; `bump_measure` nudges — the `Nudge` row |
| `app.rs:2628`–`:2631` | `MEASURE_MIN` 40 / `MEASURE_LIMIT` 200 / `MEASURE_STEP` 8 |
| `app.rs:5442`–`:5451`, `:5460`–`:5465`, `:2388`/`:2398` | `toggle_sidebar`, `set_view_mode`, `ViewMode` + its parse |
| `app.rs:2803`–`:2823`, `:2113`, `:1897` | `toggle_all_tools` is a bulk write; new blocks collapsed; errored expands — no global flag |
| `app.rs:4252`–`:4261` | `/effort` pushes `SetEffort` and writes no `Status` |
| `app.rs:4289`–`:4304` | `/plan`'s optimistic chip + `plan_pending`; revert at `:1961`–`:1964`; cleared at `:1932` |
| `app.rs:5217`–`:5221`, `:5241`–`:5245` | the picker accept arms the panel reuses verbatim |
| `app.rs:629`–`:641` | `PickerKind` — no `Effort` arm (a cycle, not a picker) |
| `app.rs:1233`–`:1251` | `Status` — the panel's live source |
| `app.rs:5502`–`:5504` | `pub fn running(&self) -> bool` — the pending trigger |
| `app.rs:2561` | `usage_collect: Option<UsageCollect>` — the precedent for host (c) |
| `ui.rs:431` | `Span::styled("❯ ", accent())` — the picker's cursor glyph (chosen for the panel) |
| `ui.rs:719`, `:707` | the browse bar `▌` and `selection_style() -> REVERSED` — the two precedents **not** used |
| `ui.rs:253`–`:261` | `item_line` — accent label padded to `ITEM_LABEL_COLS`, dim detail; the panel's row body |
| `ui.rs:303` | `fn draw_info(...)` — the panel to sit beside |
| `agent.rs:178`, `:471`, `:47` | `effort()`, `llm()`, `plan_mode` |
| `hooks.rs:75` | `PlanModeHandle::get()` |
| `limits.rs:63` | `pub fn model_limit(base_url: Option<&str>, model: &str) -> Option<ModelLimit>` — `.context` |
| `actor.rs:431`, `:437`, `:465`, `:488`, `:497` | the idle dispatch arms + the exhaustive tail a new variant must join |
| `actor.rs:528`/`:530` | `hooks` clone / `side_handle()` — the mid-run read seam |
| `actor.rs:681`–`:691` | `Ack` + `deferred.push_back(...)` for the four settings mutations — the deferral fact |
| `protocol.rs:232`, `:262`, `:296`, `:302`, `:316`–`:319`, `:366`, `:44` | `Request`, free-style `SetEffort`, read verbs, the additive argument, `#[serde(other)]`, `is_inbound` |
| `config.rs:567`, `:633`, `:662`–`:668` | `FileConfig` (Deserialize only), `Source`, four-field `ProviderProvenance` |
| `main.rs:497`, `:504`, `:1099`, `:2013` | `print_config` exits; `config_dump` is a flat `String`; the two `Status` seeds |
| `theme.rs:126`, `:398`, `:403` | the `RwLock<Theme>` (no name), `preset()`, `names()` |
| `settings.rs` (new) / `tab.ts:68`–`:69` | `EFFORT_LEVELS` and its TS twin + caveat |
| `lib.rs:283`, `:287`, `:400` | the `Options` seam; models injected; `app.set_models` |

### The panel, drawn

```
 settings ───────────────────────────────────────────────────────
   model          qwen2.5-coder:7b                        ›
 ❯ effort         high · pending                        ‹ ›
   plan           off                                   [ ]
   theme          nord                                    ›
   width          68 cols                               ‹ ›
   view           focus                                 ‹ ›
   sidebar        shown                                 [x]
   tool output    Ctrl-T expands / collapses all
   context        128000

   config (this client)
   provider: openai-compatible
   endpoint: chat (source: auto)
   base_url: http://localhost:11434/v1 (source: flag)
   model: qwen2.5-coder:7b (source: config)
   api_key: (none) (source: default)
   compaction budget: 60000
   …
 ────────────────────────────────────────────────────────────────
   ↑↓ move · ←→ change · enter edit · tab section · esc close
```

Cursor: `❯ ` in `accent()` — the picker's glyph (`ui.rs:431`), one of the three existing precedents rather than a fourth. Row body: `item_line`'s accent-label + dim-detail idiom, so the panel looks like `F1` and `/help`. Affordances (`‹ ›`, `›`, `[ ]` / `[x]`) at the right in `dim`; `pending` as a dim mark, never a hue. The scroll offset is **derived from the cursor** in the renderer (keep-the-row-visible), so `SettingsState` carries one `usize` and there is no second clamp rule to test.

---

## 5. Plan

1. **Host + skeleton.** Deliverable: `settings.rs` with the types and an empty row list; `App.settings`; the `overlay` → `settings` tier in `on_key`; `F2` wired at all three sites; a panel that draws a title, an empty body and the hint line; `Esc` closes it. Gate: `cargo test -p wcode-tui` with the precedence trio (§7.1); `cargo clippy --workspace --all-targets` clean; a manual `F2` / `Esc` round trip.
2. **Taxonomy + the four client-local rows** (`width`, `view`, `sidebar`, `plan`). Deliverable: `rows()` and `apply_setting()` producing real rows; `Nudge` clamps at 40/200; `Cycle` wraps; `Toggle` flips; `plan` pushes `SetPlanMode`. Gate: the pure `settings.rs` unit tests (no terminal) + two integration tests through `handle(Key)`; nothing in the transcript or composer moves.
3. **`Pick` rows** (`model`, `theme`). Deliverable: `Enter` opens the existing `Overlay::Pick`, accepting applies, **and the panel is still open afterwards** — the (c) payoff. Gate: `enter_on_the_model_row_opens_the_picker_and_returns_to_the_panel`; the accept-path assertions (one `Action::Ask(Request::SetModel)`, `Status.model` updated).
4. **Effort cycle.** Deliverable: `EFFORT_LEVELS` (+ the reciprocal TS comment), the `(none)` entry, the off-list rule, the `SetEffort` payload. Gate: cycle-wrap, off-list-preserved, and payload assertions; `cargo test -p wcode-tui settings`.
5. **`Request::Settings` + the two defects.** Deliverable: the wire variant both ways, the actor's idle and mid-run arms, the `Surface::apply` arm, `context_limit` refreshed on `/model`; `AgentStart`/`Error`/reply clearing. Gate: the harness pair (§7.1) and the two TUI regressions that **fail on today's tree**; `cargo test -p wcode-harness` + `-p wcode-tui`.
6. **Pending marks.** Deliverable: the dim `· pending` mark on the three Request rows while `running()`, settled by a reply, cleared by `AgentStart`/`Error`; absent on the four client-local rows. Gate: the mid-run test in §7.1 + a live check with a slow local model.
7. **Read-only config section.** Deliverable: `Options.config` injected at both TUI sites, `config (this client)` heading over `ReadOnly` rows, and `compaction budget` in `config_dump`. Gate: `--dump-config` gains exactly one line and loses none; the panel contains the text verbatim; the empty-string case renders a placeholder rather than an empty section.
8. **Discovery + docs.** Deliverable: the `COMMANDS` entry, the `KEYS` row, `README.md`, `docs/tui-design.md` §4, the TS union mirror, D036, the tracker row. Gate: `help_lists_every_command_and_the_keymap_from_the_tables` passes with `settings` added to its name list; `./target/debug/wcode --dump-system-prompt` byte-identical; `cd editors/vscode && npm run typecheck && npm test` green; second-layer review APPROVED.

Steps 2–3 are the demo-able core; if the human wants an early look, land 1–3 and stop for feedback before the kernel step (5), which is the only step that touches another crate's public surface.

---

## 6. Quality gates

- `cargo test --workspace` → all pass. Today's figure is quoted as ~1385 (`docs/next-steps.md:114`); this item adds roughly 20, so expect ~1405. **The number is unverified (§11)** — the gate is "all pass", not a count.
- `cargo clippy --workspace --all-targets` → clean, no new `#[allow]`.
- `cargo build --bin wcode` → success (necessary, not sufficient).
- `./target/debug/wcode --dump-config` → **exactly one new line** (`compaction budget: …`), every existing line byte-identical, `api_key:` still `(set)`/`(none)`.
- `./target/debug/wcode --dump-system-prompt` → **byte-identical** to before (nothing here touches the prompt).
- `cd editors/vscode && npm run typecheck && npm test` → green.
- Regression tests kept green, by name: `esc_closes_the_info_modal_and_nothing_else_does` (`app.rs:9532` — the invariant this panel must *not* break), `f1_opens_the_help_overlay_and_only_esc_or_f1_closes_it` (`:8663`), `keys_do_not_reach_the_input_while_a_modal_is_open` (`:7163`), `paste_is_ignored_while_an_overlay_is_open` (`:5743`), `help_lists_every_command_and_the_keymap_from_the_tables` (`:8288`), `help_rows_are_items_in_f1_style` (`:9865`), `command_labels_carry_args_or_aliases_in_brackets` (`:9897`), `the_info_modal_scrolls_and_clamps` (`:9553`), the Info-scroll arithmetic at `:9911`, `all_mode_is_off_by_default_and_the_frame_is_unchanged` (`:9942` — the frame golden), `the_command_table_lists_reload` (`:8258`), `ask_budget_is_long_only_for_model_call_asks` (`lib.rs:831`), `help_rows_render_in_f1s_two_column_style` (`ui.rs:5955`).

---

## 7. Testing

### 7.1 Automated

**The traps the plan must pin (two are compile errors, one is silent):**

- `ui.rs:235`'s `match app.overlay()` is exhaustive — the new variant cannot be forgotten here (the compiler is the gate; no test needed).
- `actor.rs:497` is the exhaustive `Request` tail — a missing `Settings` arm does not compile (same).
- **`on_overlay_key`'s `if matches!` + `_ => {}` is the silent one** (`app.rs:5149`–`:5175`): a variant routed there without an arm would swallow every key with no error, so `the_settings_panel_answers_keys_rather_than_swallowing_them` must assert *positive movement* (cursor advanced, a value changed), not merely `overlay.is_some()`.

**Pure (`settings.rs`, no terminal):**

- `the_row_list_names_every_session_knob_once` — model/effort/plan/theme/width/view/sidebar each appear exactly once; `tool output` and `context` are `ReadOnly`. The drift guard, and the cheap half of v1's candidate C-f.
- `nudge_clamps_at_the_measure_bounds` — at 40, `←` stays 40; at 200, `→` stays 200; elsewhere `±8` (`MEASURE_STEP`).
- `cycle_wraps_and_preserves_an_off_list_value` — the effort cycle wraps `(none)→minimal→…→max→(none)`; an off-list `medium-high` renders verbatim and the first arrow press lands on `(none)`.
- `read_only_rows_yield_a_hint_not_an_action` — every `ReadOnly` returns `None` plus a hint regardless of key.
- `tab_jumps_between_the_two_sections` — from any `session` row, `Tab` lands on the first `config` row; `BackTab` returns to the first `session` row.

**Integration (through `App::handle(AppEvent::Key(..))`):**

- `esc_closes_the_settings_panel_and_nothing_else_does` — `Esc` closes; a `Char` never reaches the composer; `Enter` on a `Toggle` **does** act (the deliberate divergence from `Info`, §4.2).
- `the_settings_panel_precedes_the_composer_and_never_the_picker` — with a picker open, keys go to the picker; with only the panel open, `↑` moves the cursor and the composer is untouched.
- `enter_on_the_model_row_opens_the_picker_and_returns_to_the_panel` — after accept, `Overlay` is `None`, `App.settings` is still `Some`, `Status.model` is the pick, exactly one `Action::Ask(Request::SetModel)` was pushed.
- `f2_opens_the_panel_from_the_composer_browse_and_search` — three assertions, one per dispatch site (the `f1_opens_…` precedent, extended).
- `a_mid_run_setting_shows_pending_until_the_reply` — `running` surface, cycle effort: the row is `pending` and `Status.effort` is the new value; apply `AgentEvent::Settings { effort: Some(<same>) }` → not pending; apply `AgentEvent::AgentStart` → not pending; apply `AgentEvent::Error` → not pending.
- `the_header_effort_follows_the_settings_reply` — **fails on today's tree** (§4.5).
- `a_model_switch_refreshes_the_context_limit` — **fails on today's tree**.
- `the_settings_panel_shows_the_injected_config_text` — non-empty `Options.config` appears verbatim as `ReadOnly` rows; empty renders the placeholder.

**Kernel / wire:**

- `settings_reply_reports_the_agents_live_state` — ask, assert the four fields; `SetModel`/`SetEffort`/`SetPlanMode`, ask again, assert each moved. A getter, not a constant.
- `settings_is_answered_mid_run_from_the_side_handle` — ask during a blocked run, assert a bounded reply before the run ends (the `Usage`/`GetHistory` precedent at `actor.rs:642`–`:669`).
- `settings_is_not_an_inbound_request` — in the existing `is_inbound_marks_only_the_delivery_verbs` (`protocol.rs:564`).
- `request_settings_roundtrips_on_the_wire` — `roundtrip(Request::Settings, "settings")`.
- `a_socket_client_reads_the_servers_settings` — `wcode-protocol` roundtrip over a real socket pair: the client learns the **server's** model. This is the assertion that makes a `--socket` panel honest.

### 7.2 How a human verifies it

```sh
cargo build --bin wcode
```

**(a) The editor, no model** — needs a TTY:

```sh
WCODE_BASE_URL=http://localhost:11434/v1 ./target/debug/wcode --tui
```

Press `F2`. Look for: ` settings`, the cursor `❯` on `model`, the affordance column, the hint line. Then: `↓↓` to `width`, `→` three times → `width  92 cols` (clamped, stepping by 8); `Tab` → the cursor jumps to the first `config` line and the heading reads `config (this client)`; `Enter` on a config line → the hint says read-only; `Esc` → back to the composer with the width visibly applied.

**(b) "edit as model"** — the `Pick` row proves the host decision:

```sh
./target/debug/wcode --tui        # F2 → cursor is on model → Enter
```

Look for: the ordinary ` model ` picker over the panel; filtering works; `Enter` on a model → the picker closes **and you are back in the settings panel** with the new model in the row. Before the host decision this would have closed the editor.

**(c) The two defects, fixed** (needs any answering endpoint):

```sh
wcode --no-session --base-url http://localhost:11434/v1 --tui
```

Say `hi`, let it answer, `F2` → `↓↓` to `effort` → `→`. Look for: the row shows the new level, the **header's `model · effort` agrees**, and the gauge's denominator follows a `/model` switch (previously it stayed at the startup model's window).

**(d) Pending, mid-run** — a slow local model makes the window wide enough to see:

```sh
wcode --no-session --base-url http://localhost:11434/v1 --tui
```

Submit a long prompt, and while it streams press `F2` → effort → `→`. Look for: `effort  high · pending` while the run is in flight; the `· pending` disappears once the run reaches its boundary (the next `AgentStart`) or a `Settings` reply lands.

**(e) The socket tell-the-truth case** — start the server with a model the client's own config does not have:

```sh
./target/debug/wcode serve --socket /tmp/w.sock &
./target/debug/wcode --socket /tmp/w.sock --tui
```

`F2`. Look for: the `session` rows report the **server's** model/effort/plan; the section heading says `config (this client)`.

**(f) The flag and the panel agree, one builder:**

```sh
./target/debug/wcode --dump-config
```

Look for: every `config` line in the panel is present here too, including the new `compaction budget:` line — **`60000 tokens`** when `[compaction] budget` is set, **`(the model window)`** when it is not (`budget` is `Option<u64>`, `config.rs:164`); a `diff` against the previous output shows exactly one added line and nothing else.

**(g) Error path — no hang, no lie:**

```sh
WCODE_RETRY_MAX=2 ./target/debug/wcode --base-url http://127.0.0.1:9/v1 --tui
```

`F2` opens instantly from client state; the `Settings` ask times out on the existing `ASK_TIMEOUT` budget (`lib.rs:707`) leaving the panel readable; a run reports its retry lines; no hang, no backtrace.

---

## 8. Expected outcome

- `F2` (and `/settings`) opens a keyboard-driven panel: `↑↓` cursor, `←→`/`Enter` act per row kind, `Tab` jumps section, `Esc` closes — and the picker rows prove the host choice by returning to the panel.
- Seven settings are editable in one place, each through a setter that already existed; the four client-local ones settle instantly, the three Request ones show `· pending` until the harness applies them.
- The `effort` vocabulary exists in the TUI for the first time, with an explicit off-list rule and a named TS twin.
- The header's `model · effort` and the gauge's denominator are true after any settings change — two defects closed, each with a test that fails without the change.
- A `Settings` reply exists on the wire, is mirrored in `editors/vscode/src/protocol.ts`, and a `--socket` client reports the server's agent settings rather than its own startup seed.
- `config.toml` is still never written by anything at runtime; the panel's own heading says which config it is showing.
- `--dump-config` gains exactly the `compaction budget` line it was missing.
- D036 records the boundary: the panel edits **session** state, and the file writer is foreclosed by that record — not left open.

---

## 9. References and the proposed decision record

- `AGENTS.md` — minimalism ("config files for behavior" absent by design), the kernel/presentation split, `Hooks` as the place for policy.
- `docs/decisions/D016-tui-width-setting.md:30` — "Config only … nothing is persisted at runtime"; the session-override split this brief keeps.
- `docs/decisions/D015-presentation-settings.md:43`–`45` — "model/effort defaults (the CLI owns agent config in `config.toml`)", and the presentation-vs-behaviour line.
- `docs/decisions/D026-tui-info-modals.md` / `D027-tui-help-info-rows.md` — the modal panel, `InfoRow`'s two-column idiom, clip-not-wrap.
- `docs/tui-design.md` §1 — no reflow; `dim` the workhorse; `error` red the only hue; every interactive thing has a key and a `KEYS` entry.
- `docs/work/W022-tui-info-overlay.md`, `docs/work/W023-help-two-column.md` — the two precedents for adding a modal cheaply.
- `docs/next-steps.md:121` (the `--dump-config` compaction gap this closes), `:106` (item 85, adjacent, not taken), `:117` (W031).
- `.wcode/skills/live-verification/SKILL.md` §1–§3 — the recipes in §7.2.
- `.wcode/skills/traceability-log/SKILL.md` §2/§4 — the D036 and WNNN formats.

### Proposed decision record — D036

`docs/decisions/D036-tui-settings-panel.md`, ~45 lines:

- **Context** — seven knobs are chord-only or command-only, and two on-screen values are provably stale (`app.rs:4252`–`:4261` never writes `Status.effort`, which `ui.rs:838` prints; `main.rs:2017` seeds `context_limit` once). The overlay is a single slot (`app.rs:5203`), `Esc` is pinned as the only close key (`app.rs:5164`, `:9532`), and no reusable text field exists (`app.rs:2493`).
- **Decision** — a settings **panel** hosted on `App.settings`, not on `Overlay` (so a picker can open on top of it without an overlay stack); a five-kind row taxonomy where every act applies immediately through an existing setter; no dirty/commit state (`Esc` closes); `Request::Settings` supplies live truth and settles optimistic marks; the read-only config section is the finished `--dump-config` string. **No runtime `config.toml` writer** — a request to make a setting persist for the next launch is refused by this record and needs its own.
- **Consequences** — costs: a fourth precedence tier; `BackTab` no longer switches surfaces while the panel is open; the effort vocabulary is duplicated in `settings.rs` and `tab.ts`; the panel shows `--dump-config`'s scope and not the nine unrepresented config tables. Forecloses: the writer, the editable-config rows, and a typed-field grammar (follow-up, not a slip).

---

## 10. The one open question — the fork

**RESOLVED 2026-10-24: session-scoped, no writer.** The human approved the brief as written; the default stands. A persistence request is a separate item and its own record.

**Does a change stick for the next launch?**

- **Default (this brief, and every step above): session-scoped, no writer.** It is the D016-consistent reading, it is why the editor is cheap, and it is what "edit as model" describes — `/model` does not persist either.
- **Fork: stick-for-next-launch.** A different item, roughly doubling the work, and this repository's **first `config.toml` write path**: a `Serialize` on `FileConfig` (`config.rs:567` is `Deserialize` only), a target choice (global `~/.config/wcode/config.toml` vs project `./.wcode/config.toml`, per the project-team overlay), merge-vs-replace semantics against a partially hand-edited file, atomic write + concurrent-edit safety, per-row validation, and a new decision record. It also has a subtlety that argues for keeping it separate: over `--socket` the writer would edit the **client's** config while the agent lives on the server, so "stick" would not mean what the user expects.

**This is the only question that changes the plan.** Everything else is decided above. Recommendation: ship session-scoped, and let a request for persistence be judged on its own record.

---

## 11. What was not verified

1. **No code was compiled or run.** `bash` is refused for the briefing worker: no `cargo test`, `cargo clippy`, `cargo build`, `npm test`, `--dump-config`, and no date. Every "~N tests" and "~N lines" figure is a judgement or a quote, not a measurement.
2. **The two `Status` defects are code-inferred, not observed.** That no writer exists outside the two startup seeds was re-verified by grep; nobody watched a header go stale in a real terminal.
3. **`crates/wcode-tui/src/ui.rs` was not read end to end** (~6000 lines). Read: `draw_overlay`/`item_line`/`draw_info` (`:233`–`:350`), the header row (`:818`–`:882`), the foot row (`:1940`–`:2090`), the picker body (`:425`–`:440`), the selection helpers (`:700`–`:730`). Another `Status`-derived readout remains possible.
4. **The exhaustive-vs-silent match claim.** `ui.rs:235` has no `_` arm and `actor.rs:497` is the `Request` tail; `on_overlay_key` uses `if matches!` + `_ => {}`. Not every other `match` on `Overlay`/`Mode`/`Request` in the workspace was read, so a third silent site is possible.
5. **`editors/vscode/` was swept, not read.** Confirmed: `package.json`'s four settings, `protocol.ts:51`/`:118`, `tab.ts:68`–`:69`. Not read: `manager.ts`/`tab.ts` in full — so whether the extension already displays model/effort somewhere that would also be stale is unknown, as is whether adding a union member alone keeps `npm test` green (an unhandled event type in a `switch` could trip a lint rule).
6. **The `help_lists_…` name list is already incomplete** — it omits `view` (`app.rs:8321`–`:8325`) while its count assertion is derived (`:8318`), so adding `/settings` will **not** fail it. Whether to also fix the `view` omission here or file it is undecided; the plan asks only for `settings`.
7. **Real-terminal key reality for `F2`.** No `F2` mapping exists in this codebase and no conflict *within* it. Whether a given terminal/tmux profile delivers `F2` as `Key::F(2)` is unverified — the same class of risk `F1` already carries, and the reason `/settings` is a second door.
8. **`context_limit` is `None` for most models today** — `limits::model_limit` only recognizes opencode-go (`limits.rs:63`–`:66`), so the panel's `context` row will usually read `unknown`, and the gauge fix will be invisible on a local Ollama model. The fix is still correct (an honest `unknown` beats a stale number), but nobody should expect the gauge to improve on a keyless endpoint.
9. **`docs/next-steps.md` was read to ~line 1000 only** (items 1–94 plus the open-gaps section). Later entries may already track a settings panel or the `context_limit` staleness — which would make part of this brief a duplicate.
10. **The `omarchy` theme label.** `active_name()` has no precedent: `theme.rs:398` resolves a *name* but nothing stores the active one, and `omarchy` is a sentinel pushed into `names()` (`:406`), not a catalog entry. How the `theme` row should render an omarchy-derived palette is **undecided**, left to step 3 rather than guessed.
11. **`--dump-config`'s exact current bytes** are unverified (the builder was read, not its output). Step 7's gate ("exactly one new line") needs a captured before/after.

---

## 12. Log

- **v1 (superseded).** Read-only settings view + `Request::Settings`; argued *against* an editor because an editor implies a `config.toml` writer. **Withdrawn** — the inference was wrong (§1.1).
- **v2 (this document).** The ask is an **editor**: `App.settings` host, five-kind row taxonomy, no dirty state, picker-on-top, effort cycle with an off-list rule, optimistic-with-pending for the deferred Request rows, the finished `config_dump` string as read-only rows, `/settings` + `F2`. The surviving refusal is narrow: **no runtime `config.toml` writer**. Open question: §10.

## 13. Sign-off log

- **2026-10-24 — APPROVED, session-scoped.** "approve, go". The §10 fork is resolved at its default: **no runtime `config.toml` writer**; every row edits session state through a setter that already exists. All eight plan steps proceed in order (§5); steps 1–3 are the demo-able core, step 5 is the only one touching another crate's public surface.

---

## 14. First-layer review — BLOCKED, then amended

The sketch (`crates/wcode-tui/src/settings.rs` + 27 in-place blocks) was reviewed
2026-10-24. The **design held** — host (c), `SettingAction`-only writes, session
scope/no writer, and one builder for `--dump-config` all verified against the
code — and the verdict was **BLOCK on seven sketch defects**, all surgical. They
are fixed in the sketch; the sixteen amendments below are the second layer's
checklist.

### 14.1 The seven blockers and their fixes

| # | blocker | fix landed in the sketch |
|---|---|---|
| **B1** | `actor.rs` had two real lines reformatted (`}                    Message::Ask {`), so the sketch was not additions-only | restored — `git diff --numstat` is additions-only outside `settings.rs` and `docs/` |
| **B2** | `AgentEvent::Settings` breaks the **one total `AgentEvent` match**, `crates/wcode-harness/tests/loop_tests.rs:262` (`fn tag`, 27 arms, no `_`) | a block there, and `tests/loop_tests.rs` added to §4's Changed list |
| **B3** | the `compaction budget` line did not compile — `cfg.compaction.budget` is `Option<u64>` (`config.rs:164`), not `Display` | renders both cases (`{budget} tokens` / `(the model window)`), and step 7's gate pins both |
| **B4** | `Section.heading: Option<…>` contradicted design amendment 5 (the **first** section is labelled), and `line_of` omitted the blank separator — desyncing the cursor's line by one exactly where `Tab` lands | `heading: &'static str`; `line_of` counts headings + separators + rows |
| **B5** | `open_settings` never pushed the ask, so nothing could ever refresh or settle | the push (exactly once per open), and an ask on every boundary that leaves marks outstanding |
| **B6** | `F2`'s browse/search sites were unsketched and the cross-reference dangled | three real anchors (`:3435` browse, `:3472` search, `:4263` composer) + a block at each |
| **B7** | `ReadOnly` rows swallowed `↑`/`↓`, making the whole `config` section a trap `Tab` could enter but arrows could not leave | split: `Enter` → hint, `←`/`→` → no-op, `↑`/`↓` → move and clear the hint; the hint falls back to `note` first (`tool output` must not cite `--dump-config`) |

### 14.2 The sixteen amendments (layer 2 checks these)

- **A1** `Setting.value: String` — ratified (one formatter in `rows`).
- **A2** `Cow<'static, str>` label — ratified.
- **A3** `Toggle{what}`/`Cycle{what}` — ratified, and **extended**: `SettingAction` splits `OpenPicker(PickerKind)` into `OpenModelPicker`/`OpenThemePicker`, so `apply_setting` needs no wildcard for impossible kinds.
- **A4** `pending` in `App`, via the snapshot, no `running` field — ratified (`Esc` drops `App.settings`, and `Esc` is not a clearer).
- **A5** settlement — **amended**: store the *intent* (`Vec<(id, String)>`) and settle a mark iff the reply's field equals it; `Error` must not clear every mark (a `/btw` failure reaches `App`); **and `AgentEnd` must ask too**, or a mid-run edit shows `· pending` forever for a change that was applied.
- **A6** `SettingsState { cursor, hint, page }` — ratified, with `page: 0` (no `DEFAULT_PAGE = 10`) so `PgUp`/`PgDn` are a no-op before the first frame.
- **A7** `rows(snapshot)` only — ratified; `pending` membership is not dispatch.
- **A8** draw tripwire — the **test is adequate and, for z-order, stronger** than the compiler ever was; but it must assert **pixels**, never `settings().is_some()`. Spelled out so it cannot degrade: (a) *panel only* — the ` settings ` title and a row detail are in the buffer **and** a base-band string outside the panel's rect is still present (drawn over, no reflow); (b) *panel + `Overlay::Pick`* — the picker wins at the overlapping cells while the panel is still drawn outside the picker's rect. Buffer-reading precedent: `ui.rs`'s `help_rows_render_in_f1s_two_column_style`.
- **A9** `active_name()` — record in **`set`**, derived from the spec (`ThemeSpec::name()`), not in `with_preset` (a builder that can be discarded). And the row prints **`palette b`**, never `default`. **The reason was corrected by the designer and verified:** `default` and palette B are the *same colours* on this tree (`from_palette` returns `palette_b` for `DEFAULT`, `theme.rs:615`–`:616`; the shipped `the_default_preset_is_palette_b` at `:1354` pins it), so the lie is one of **source identity** — `Source::PaletteB` is not `Source::Named("default")`, and borrowing the name would make the `/theme` picker show as current a name the user never chose. `omarchy` verbatim; an inline `[theme]` also reads `palette b`; a test must not assert magenta headings for `default`.
- **A10** one `ReadOnly` row per `config` line — ratified; split on the first `": "`, keeping the colon in the label; the `Vec<ConfigRow>` refactor stays a follow-up.
- **A11** **five** `KEYS` rows (design overrides the brief's one), with the designer's wording (`"F2" → "open the settings panel"`, not "edit"). **Fix the hardcoded name list's existing `view` omission in the same edit** (`app.rs:8672`).
- **A11b** bump `help_rows_render_in_f1s_two_column_style`'s height from the data — after step 8 the body is 60 lines against `visible == 61` at h=64: one row of margin.
- **A5b** the failure-path case, folded into `a_settled_reply_clears_only_matching_marks`: intent `x`, reply `y`, harness **idle** → no mark, `Status` shows `y` (the boundary refused `x`).
- **A12** six added tests: `slash_settings_opens_the_panel` (`run_command` falls through to an "unknown command" transient), `the_settings_panel_answers_keys_rather_than_swallowing_them` (§7.1 named it and omitted it), `opening_the_panel_asks_for_the_settings_once`, `a_settled_reply_clears_only_matching_marks`, `the_panel_reports_the_focused_surface_in_all_mode` (**focused**, not `chrome_status()` — the panel edits the focused surface), and the pixel test of A8. The two `Status`-defect regressions must drive `app.handle(AppEvent::Agent(..))`, not `Surface::apply`, or they cannot catch the `_ => false,` arm.
- **A13** nothing unrequested — and two minimality nits: `MEASURE_STEP` and `WHEEL_LINES` become `pub(crate)` and are **reused**, not copied.
- **A14** `settings()`/`settings_snapshot()` are `pub(crate)` (a `pub` method returning a private module's type is a `private_interfaces` warning; clippy stays clean with no `#[allow]`).
- **A15** `draw_settings`'s borrow: copy the scalars out before `set_settings_page` (a `&App` borrow and a `&mut App` call cannot coexist).
- **A16** doc drift to close by step 8: this file's §5 step-2 sentence (done, above), `tui-design.md`'s "`default` before the first switch" (A9), the tracker row's "only open question" tail, the `W014a` paragraph's "`--dump-config` prints no `[compaction] budget`", and `settings.rs`'s module doc's "SKETCH (review-only) … NOT registered".
