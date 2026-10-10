# W014 — Sidebar root row, honest compaction windows, transient `/`-command notices, and a collapsed live-thinking annotation — **brief**

- **Status:** **approved by the human (2026-10-10)** · slice a shipped + APPROVED (below) · slices b/c/d spec-amended, implementation in progress
- **Work item:** W014
- **Decisions:** none locked. Needs one new record — **D018** (TUI: in-flight thinking is an annotation, not an expansion), amending `docs/tui-design.md` §1.3-B, §2 glyph table, §5 "Thinking — resolved (D33/D010)"
- **Author:** `agent:brainstormer` (read-only worker)
- **Amends:** `docs/tui-design.md` §4 (sidebar), §1.3-B + §5 (thinking); `docs/next-steps.md` line 986 (gap item 6)

---

## 1. The ask

The human named four defects: (1) "the TUI sidebar has no root/wcode row to click on"; (2) "carefully verify the current compact logic: is step-5-preview-free a 1M-context model (or other)? It seems to compact too frequently"; (3) "some commands such as /usage /help or copy should be cleaned after bot gen instead of stuck (in the transcript)"; (4) "thinking should not expand by default; instead, while thinking, use a running annotation + timing to show thinking is active." Item (2) is a correctness bug with a known root cause — wcode assumes a 128k window for any model missing from its static catalog, so a 1M-token model compacts at ~11% of its real window and silently throws away context; items (1), (3), (4) are three separate TUI behaviour defects that all touch the designer's locked spec.

## 2. Scope / Non-scope

**In scope.**

- **W014a (harness).** Refresh `limits::OPENCODE_GO` with the verified-missing entries (incl. `step-5-preview-free` = 1M/65,536), and make the *effective* context window visible without a network call (`--dump-config`, and the REPL banner/startup diagnostic when the model is unknown).
- **W014b (TUI).** Render a **root/wcode row** as the first row of the sidebar's `agents` section, carrying the root surface's index so `set_sidebar_hit` can focus it on click.
- **W014c (TUI).** Slash-command echo + command output notices become **transient** transcript blocks, retired at the next submit ("after bot gen").
- **W014d (TUI).** In-flight thinking stops streaming expanded: one dim annotation row (`··· thinking 1.2s`) + a composer-foot state chip that reads *thinking* rather than *running*.
- The doc amendments each TUI slice needs (`docs/tui-design.md`, `docs/next-steps.md` tracker row, the new D-record).

**Non-scope.**

- **Widening the catalog to non-opencode-go providers** (`docs/gap-analysis-jcode.md` item 6's real ask) — still deferred. This brief refreshes the existing snapshot and does not add providers.
- **A runtime models.dev fetch** at startup — network at boot, non-deterministic, contradicts minimalism.
- **Changing `DEFAULT_CONTEXT_WINDOW = 128_000`** (`crates/wcode-harness/src/limits.rs:19`) — raising it would let a genuinely small model overflow silently. The default stays; the fix is a correct catalog + a visible diagnostic.
- **`[models.<id>] context_window = …` per profile** — sign-off option A4, not built unless asked; `[compaction] window` / `WCODE_COMPACT_WINDOW` already covers the escape.
- **Making the compaction ceiling subtract the model's output reserve** (`ModelLimit::trigger()` at `limits.rs:51` is used only by `is_full`) — makes compaction *earlier*, opposite of the complaint; separate item.
- **`/team` roster showing the root**, and the transcript's team strip listing the root — both are documented as member-only (`docs/tui-design.md` §4, §1.3 band 2).
- **A toast/overlay for command output** — evaluated and rejected (§4c); a redesign of the notice system, not the asked-for cleanup.
- **Live thinking expand/peek on click** — the live block is deliberately never a selection/affordance target; a hit region on a transient row is a separate change.
- **A new spinner glyph or animation** — the TUI is MOTION 1 (`⠹` is a static char, `ui.rs:1750`); no animation is added.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | a — catalog refresh + window diagnostic | high | low | low | A data-table add plus one print; the risk is only a wrong entry, verified against models.dev. |
| P1 | b — sidebar root row | med | low | low | `sidebar_member_at` already dispatches any surface index; only the `—` placeholder and the root's label need decisions. |
| P2 | c — transient notices | med | med | med | Splices three parallel vecs and can shift the browse `selected` index; ~20 existing tests assert `transcript().last()` is a notice. |
| P3 | d — thinking annotation | med | med | med | The live render is cached per `(live_rev, width)`, so a per-frame timer needs a cache-invalidation fix, and one spec decision (D018) must land first. |

*Splitting:* recommended **one work item W014 with four named slices and four commits** (`harness:` for a, `tui:` for b/c/d). The alternative — four separate items — is worth it only if the human wants four independent second-layer reviews; **slice a is the only kernel-side change and can ship alone at any time.**

## 4. Interface & structure

### 4a — compaction windows (harness)

**Verified fact (models.dev api.json, fetched 2026-10-10):** the `opencode-go` provider carries `step-5-preview-free` at `limit.context = 1_000_000`, `limit.output = 65_536` ("StepFun's next-generation flagship base model ... 1M-token context window"). It is **absent** from wcode's table.

**Changed.**

- `crates/wcode-harness/src/limits.rs:80` — `static OPENCODE_GO: &[(&str, u64, u64)]` gains the verified-missing rows (`step-5-preview-free 1M/65,536`, `mimo-v2.6-pro 1M`, `grok-4.7 500k`, `longcat-2.5-preview-free 1M`, `space-bunny 1M`, `space-bunny-free 1M`, `mimo-v2.6-flash 1M`, `claude-haiku-5-5 1M`, `gpt-6-luna 1M` — re-verify each with the same `curl` before committing). No signature change.
- `crates/wcode-cli/src/main.rs:437` `config_dump` — add one line, `context: {window} tokens (source: model catalog | [compaction] window | default)`, computed with the same precedence the kernel uses (`model_limit` → `CompactionConfig::window` → `DEFAULT_CONTEXT_WINDOW`). This is the *keyless* rung-3 proof for the bug.
- `crates/wcode-cli/src/repl.rs:800` — the banner prints the context only when the model is known; add the unknown case so a REPL user sees which window is being assumed.

**Integration points (unchanged, cited because the fix rests on them).**

- `crates/wcode-harness/src/compaction.rs:149` — `pub fn context_window(&self, base_url: Option<&str>, model: &str) -> u64 { crate::limits::model_limit(base_url, model).map(|l| l.context).or(self.window).unwrap_or(crate::limits::DEFAULT_CONTEXT_WINDOW) }`
- `crates/wcode-harness/src/compaction.rs:158` — `pub fn should_compact(&self, window: u64, used: u64) -> bool { used >= self.ceiling(window).saturating_sub(self.min_remaining) }`, with `DEFAULT_MIN_REMAINING = 16_384` (`compaction.rs:92`).
- `crates/wcode-harness/src/loop_.rs:172` — the trigger: `let window = cfg.compaction.context_window(cfg.llm.base_url.as_deref(), &cfg.llm.model); if cfg.compaction.should_compact(window, used) { … }`.
- `crates/wcode-harness/src/limits.rs:61` — `pub fn model_limit(...)`: `if is_opencode_go(base_url) { return opencode_go(model); } None` — the only path that returns a window.
- Existing escape hatches that stay: `crates/wcode-cli/src/config.rs:163` `pub window: Option<u64>`, `WCODE_COMPACT_WINDOW` (`config.rs:550`).

**Consequence:** for `step-5-preview-free` the trigger moves from `128_000 − 16_384 = 111_616` to `1_000_000 − 16_384 = 983_616` input tokens (~8.8× later).

### 4b — sidebar root row (TUI)

**Changed.**

- `crates/wcode-tui/src/app.rs:3157` — add a sibling of `member_rows_indexed`, e.g. `sidebar_rows_indexed() -> Vec<(usize, &str, TeamState, bool, Option<&str>, bool /*is_root*/)>` that does **not** filter `.filter(|&i| !self.surfaces[i].is_root)`, so the root is row 0. `member_rows`/`member_rows_indexed` keep their documented "non-root" contract (`app.rs:2546`, `app.rs:3153`) — `/team` and the team strip are untouched.
- `crates/wcode-tui/src/ui.rs:2045` `draw_sidebar` — the row loop at `ui.rs:2066` `for (n, (idx, label, state, focused, action)) in indexed.into_iter().enumerate()` iterates the new list; the root row gets an **empty number badge** so members keep `1..N` (open decision O2).
- `crates/wcode-cli/src/main.rs:978` and `:1796` — `label: "root".to_string()` → `"wcode"`, so the row reads `wcode` (matching the user's words and the pinned header's `WCODE` badge at `ui.rs:760`). No test asserts the rendered root label; fixtures use `surf("root", "wcode", true)` (`ui.rs:3820`).

**Integration points (unchanged).**

- `crates/wcode-tui/src/app.rs:3016` — `pub(crate) fn set_sidebar_hit(&mut self, rect: Rect, members: Vec<(usize, u16)>)`; `ui.rs:2090` already pushes every drawn row's `(idx, y)`, so the root needs no hit-test change.
- `crates/wcode-tui/src/app.rs:3056` — `if let Some(idx) = self.sidebar_member_at(row, col) { self.set_focus(idx); … }` focuses whatever index the row published.
- `crates/wcode-tui/src/app.rs:3145` — `fn sidebar_member_at`: `hit.members.iter().find(|(_, y)| *y == row).map(|(idx, _)| *idx)` — index-agnostic already.
- Regression: `crates/wcode-tui/src/app.rs:6474` `click_a_sidebar_row_focuses_the_surface`, `:7064` `alt_n_focuses_the_numbered_sidebar_row` (doc comment: `Alt-1` focuses "the FIRST MEMBER (sidebar row 1) — not `surfaces[0]`, the root").
- `ui.rs:2060` — the empty-section placeholder `lines.push(Line::from(Span::styled("  —", dim())));` becomes unreachable while any root exists; keep it as a defensive branch or delete it (open decision O3).

### 4c — transient command notices (TUI)

**Design space evaluated.** (a) *Ephemeral blocks* — a new `Block::Transient(String)` variant that renders exactly like `Notice` (`ui.rs:692`) and is spliced out at the next submit. (b) *Status-band one-liner* — costs the scrollback, a bigger behavioural change than asked. (c) *In-place replacement* — keep only the newest notice; "cleaned after bot gen" is more literal under (a). **Recommended: (a)**, with the command echo and its output both transient.

**Added.**

- `Block::Transient(String)` in `crates/wcode-tui/src/app.rs:190` (the `Block` enum) — a rendering twin of `Notice`.
- `Surface::retire_transient()` — the splice, modelled on `clear_hint` (`app.rs:1431`), which removes one block from **all three parallel vecs**: `self.transcript.remove(i); self.block_revs.remove(i); self.cache.remove(i);`. Must also clamp `surface.selected` (browse) — `clear_hint` does not, and a removed block below the selection shifts it (risk R2).

**Changed.**

- `crates/wcode-tui/src/app.rs:3450` — `self.notice(line)` in `fn command` → `transient(line)`.
- The command-output arms in `run_command` (`app.rs:3455`): `/usage` (`:3531` → `render_usage`, `app.rs:1796` `push_notice`), `/help` (`:3547` `self.notice(help_text())`), `/copy` (`:3539` → `copy_last`, `app.rs:3696`/`:3698`), `/verify` (`:3530`), `/tasks` (`:3545`), `/team roster` (`:3542`), plus the `usage:` error notices and `unknown command:` (`app.rs:3452`).
- `crates/wcode-tui/src/app.rs:3410` `fn submit` — call `retire_transient()` on the focused surface **before** pushing `Block::User`, so the cleanup lands with the next generation.
- **Stay permanent** (run events, not command chatter): `⋯ compaction skipped` (`app.rs:1649`), `⋯ compacted N messages` (`:1653`), `⋯ retrying (n/m)` (`:1663`), `⏹ aborted` (`:1605`), the changes summary (`:1609`), `⋯ N earlier message(s)` (`:1962`).

**Integration points.**

- `crates/wcode-tui/src/app.rs:1796` — `fn push_notice(&mut self, text: impl Into<String>) { self.push_block(Block::Notice(text.into())); }` — the single choke point for every run-event notice; `render_usage` uses it, so the transient/permanent split is decided *per call site*, not inside `push_notice`.
- `crates/wcode-tui/src/app.rs:1409` `fn push_block` — the three-vec invariant the splice must preserve.
- `crates/wcode-tui/src/app.rs:199` — `Block` is `pub` and `PartialEq`; the copy arm at `app.rs:347` matches `Block::Notice(text) | Block::Error(text) | Block::Btw(text)` and must learn the new variant.

### 4d — live thinking as an annotation (TUI)

**Changed.**

- `crates/wcode-tui/src/ui.rs:940` — the live arm: `if live { /* In-flight: stream inline, expanded — as before D33. */ lines.extend(wrap(text, width, THINK_FIRST, THINK_CONT, thinking())); }` becomes a **single row**: `thinking_block_lines`-shaped header carrying a timer, i.e. `··· thinking  1.2s`, dim, plus the live cursor `▌` that `content_lines` already appends to the last line (`ui.rs:965`).
- `crates/wcode-tui/src/ui.rs:1748` `corner_titles` — the state chip: `(true, Some(d)) => (format!("⠹ running {}", format_ms(…)), accent())` becomes *thinking* when the focused surface's live message's last content block is `Thinking`. No new glyph (`⠹` is already the running glyph; `···` is the thinking marker at `ui.rs:1062`).
- `crates/wcode-tui/src/ui.rs:912` `pub(crate) fn live_lines(message: &AgentMessage, width: usize)` and `:923` `fn content_lines(...)` — gain the elapsed (`Option<std::time::Duration>`), threaded from `crates/wcode-tui/src/app.rs:1344` `append_live_lines`. The renderer stays pure: it never reads a clock.
- `crates/wcode-tui/src/app.rs:4312` `set_run_elapsed` — **must** bump the surface's `live_rev` (risk R1), otherwise the timer freezes between deltas.

**Integration points.**

- `crates/wcode-tui/src/app.rs:4305` — `pub fn run_elapsed(&self) -> Option<std::time::Duration> { self.focused().run_elapsed }` — the injected clock (`lib.rs:565` `app.set_run_elapsed(id, started.elapsed());`), reused verbatim; already displayed on the sidebar's focused row (`ui.rs:2080`).
- `crates/wcode-tui/src/app.rs:1472` `fn state()` / the live message shape: the kernel already separates the two deltas — `crates/wcode-harness/src/loop_.rs:259` `Some(LlmStreamEvent::ThinkingDelta(delta)) => { append_thinking(&mut content, &delta); … }` and `:247` `TextDelta => append_text(...)`, and `append_thinking` (`loop_.rs:866`) pushes a **new** `ContentBlock::Thinking` when the last block is not thinking. So "thinking is active" is derivable in the reducer as *live message present && `content.last()` is `Thinking`* — **no kernel change**.
- Committed behaviour is unchanged: `ui.rs:943` `else { … thinking_block_lines(text, width, thinking_open, i == 0) }`, `thinking_open: false` at commit (`app.rs:1725`).

**Risks.**

- **R1 (must fix).** `crates/wcode-tui/src/app.rs:1349` — `let stale = self.live_cache.rev != self.live_rev || self.live_cache.width != width;` — `set_run_elapsed` only sets the app-level `dirty`, so a cached live render would keep a stale timer. Fix: `Surface::bump_live()` on `set_run_elapsed` for that surface.
- **R2 (slice c).** Removing a block shifts `focused().selected` and every later `ranges` entry; the splice clamps `selected` (pattern: `app.rs:531` `let mut next = (self.selected as isize + delta).clamp(0, last);`).
- **R3 (slice b).** The root row makes the `agents` section non-empty in every frame; a snapshot test that expects only `  —` in a member-less sidebar would need updating (none exists today).

## 5. Plan

1. **a — catalog refresh.** Deliverable: the 9 verified rows in `limits::OPENCODE_GO` + `limits::tests` coverage. Gate: `cargo test -p wcode-harness --lib limits` green, incl. a new `step_5_preview_free_is_a_1m_window`.
2. **a — window diagnostic.** Deliverable: `--dump-config` prints the effective window and its source; the REPL banner names the assumed window for an unknown model. Gate: `cargo run -p wcode-cli -- --model step-5-preview-free --base-url https://opencode.ai/zen/go/v1 --dump-config` shows `context: 1000000 tokens (source: model catalog)`, and with `--model not-a-real-model` shows `(source: default)`.
3. **b — sidebar root row.** Deliverable: the row renders and clicks; `member_rows_indexed` untouched. Gate: `cargo test -p wcode-tui` green, plus a new `click_the_root_row_focuses_the_root_surface` beside `app.rs:6474`, and `alt_n_focuses_the_numbered_sidebar_row` (`app.rs:7064`) still green.
4. **c — transient notices.** Deliverable: `Block::Transient` + `retire_transient()` wired into `submit()`; command echo/output transient, run events permanent. Gate: `cargo test -p wcode-tui` green; the ~20 notice tests still pass at the point they assert; a new test asserts a `/usage` notice is gone after the next `Enter`.
5. **d — thinking annotation.** Deliverable: the one-row live annotation + the *thinking* foot chip + the `live_rev` bump, and `docs/tui-design.md` amended. Gate: `cargo test -p wcode-tui` green with `thinking_auto_collapses_on_turn_end` (`ui.rs:4967`) rewritten to assert the collapsed annotation; `cargo run -p wcode-tui --example dump` reviewed.
6. **Docs + tracker.** Deliverable: the D018 record, the `docs/tui-design.md` §4/§1.3-B/§5 amendments, and `docs/next-steps.md` line 986 re-worded (item 6 stays **deferred** for non-opencode-go, with a pointer to W014a). Gate: the doc diff names every touched anchor.

## 6. Quality gates

```
cargo test --workspace                       # all pass (currently ~1100+ tests)
cargo clippy --workspace --all-targets       # clean
```

Live checks (per `.wcode/skills/live-verification/SKILL.md`):

```sh
# a — keyless, model-free: the effective window and where it came from.
cargo run -p wcode-cli -- --model step-5-preview-free \
  --base-url https://opencode.ai/zen/go/v1 --dump-config
#   expect: context: 1000000 tokens (source: model catalog)

# a — no regression on the error/retry path.
WCODE_RETRY_MAX=2 cargo run -p wcode-cli -- --base-url http://127.0.0.1:9/v1 -p "hi"
#   expect: the retry lines, then a clean failure — no panic, no hang.

# b/c/d — headless frames of the real TUI (the design reviewer's rung 3).
cargo run -p wcode-tui --example dump     # then read target/tui-render/{80x24,120x40,48x20}.txt
```

Regression tests that must stay green, by name: `limits.rs::looks_up_configured_model`, `limits.rs::unknown_model_or_provider_is_none`, `limits.rs::reserve_is_capped_and_trigger_and_is_full_agree`; `app.rs::click_a_sidebar_row_focuses_the_surface`, `app.rs::sidebar_click_is_ignored_when_closed_or_narrow`, `app.rs::alt_n_focuses_the_numbered_sidebar_row`, `app.rs::a_second_submit_while_running_is_refused`; `ui.rs::thinking_renders_one_line_until_expanded`, `ui.rs::only_the_first_thinking_row_draws_the_affordance`, `ui.rs::thinking_auto_collapses_on_turn_end` (**amended**, not merely kept), `ui.rs::a_rendered_thinking_row_publishes_a_clickable_toggle`, `ui.rs::the_open_sidebar_shows_its_section_headers_and_a_member_row` (asserts `1 ○ explorer`), `ui.rs::a_narrow_terminal_never_docks_the_sidebar`.

**Not run by the brief's author:** the read-only sandbox refuses `cargo test`/`cargo clippy`, so the two gates above are *specified*, not observed.

## 7. Testing

### 7.1 Automated

- `limits::step_5_preview_free_is_a_1m_window` — asserts `model_limit(ZEN, "step-5-preview-free")` is `Some(ModelLimit { context: 1_000_000, output: 65_536 })`.
- `compaction::catalog_window_beats_the_override_beats_the_default` — asserts `CompactionPolicy::context_window` precedence over three cases.
- `config_dump_names_the_context_window_source` (cli) — asserts the new line names `model catalog`, `[compaction] window`, and `default` respectively.
- `click_the_root_row_focuses_the_root_surface` — publishes the root's `(0, 1)` and asserts `app.focus() == 0`; the sibling of `app.rs:6474`.
- `the_sidebar_root_row_keeps_member_badges_stable` — renders the sidebar with root + 2 members and asserts `1 ○ explorer` still starts at 1 and the root row carries no badge.
- `a_command_notice_is_retired_by_the_next_submit` — `/usage` (or `/help`) leaves a notice; one submit removes it; the run-event notices survive it.
- `retire_transient_keeps_the_three_vecs_in_step` — after a retirement, `transcript.len() == block_revs.len() == cache.len()` and a browse `selected` below the splice still selects the same block.
- `in_flight_thinking_is_one_annotation_row` — rewrites `ui.rs:4967`: a `MessageStart` with a `Thinking` block renders **one** dim `··· thinking` row with the elapsed and no body; the body appears only after `Enter` on the committed block.

### 7.2 How a human verifies it

```sh
# (1) The compaction window — no key needed.
cargo run -p wcode-cli -- --model step-5-preview-free \
  --base-url https://opencode.ai/zen/go/v1 --dump-config
```
Look for `context: 1000000 tokens (source: model catalog)`. Then, against the real endpoint (needs `OPENCODE_API_KEY`), run a long session and watch `/usage`: the compaction line should not appear before ~950k input tokens.

```sh
# (2) The sidebar root row — a real TUI against a keyless local model.
cargo run -p wcode-cli -- --base-url http://localhost:11434/v1 --model deepseek-v4.1-flash
```
Press `Ctrl-B`, then click the top row. Look for the row (`wcode ●` or `root ●`, `*` when focused) and the transcript switching to the orchestrator's view; `Alt-1` must still land on the first *member*.

```sh
# (3) Transient notices — same TUI.
```
Type `/usage`, then `/help`, then send a real prompt. The notices disappear as the new turn's blocks arrive; `⋯ compacted`/`⏹ aborted` must stay.

```sh
# (4) Thinking — same TUI, or headless:
cargo run -p wcode-tui --example dump && cat target/tui-render/120x40.txt
```
While the model reasons, look for one `··· thinking 1.2s` row (no reasoning body) and a composer foot reading `⠹ thinking 1.2s`; on commit the row collapses to the familiar `··· thinking` affordance row.

## 8. Expected outcome

- `--dump-config` names the effective context window and its source for every model, so "it compacts too frequently" is answerable without reading code.
- `step-5-preview-free` compacts at 983,616 input tokens, not 111,616 — an ~8.8× later trigger, and the gauge denominator reads 1M instead of 128k.
- The sidebar's `agents` section always has a clickable root/wcode row; member badges stay `1..N` and `Alt-N` still means member N.
- A transcript shows at most one generation of command chatter: `/usage`, `/help`, `/copy` replies are gone once the next turn starts, while compaction/abort notices persist.
- While a model reasons, the transcript shows one dim `··· thinking <t>` row and the composer foot reads *thinking <t>*; the reasoning body is reachable only after the turn commits.

## 9. References

- `docs/tui-design.md` — the locked spec: §1.3-B (the running panel's `⠹ running 3.1s`), §2 (the glyph table), §4 (the sidebar's two sections and "the root is the orchestrator, excluded"), §5 ("Thinking — resolved (D33/D010): in flight it streams **expanded** inline") — the line slice d amends.
- `docs/next-steps.md`:986 — gap item 6, "Widen the per-model context catalog … ☐ deferred".
- `docs/gap-analysis-jcode.md`:16 — the item-6 row with its stated reason for deferral.
- `docs/work/W013-tui-color-width-sidebar.md` — the format this brief continues, and the last sidebar change (`f6519ae`).
- `docs/decisions/D011-tui-width-color-sidebar.md` — the last spec amendment to the sidebar.
- Skills: `brainstorm-brief`, `interface-sketch`, `design-taste`, `live-verification`, `two-layer-review`.

## 10. Open decisions for sign-off (the human picks; recommendations in italics)

- **O1 — the split.** *(Recommended: one W014, four slices, four commits; slice a can ship alone.)* The alternative is four items W014–W017 for four independent reviews.
- **O2 — the root row's badge.** *(Recommended: no badge — members keep `1..N`, `Alt-N` parity and `app.rs:7064` survive untouched.)* The alternative gives the root `1` and shifts every member, which changes `Alt-N` for every team.
- **O3 — the root row's label.** *(Recommended: render the surface's label and change the runtime root label `"root"` → `"wcode"` in `main.rs:978`/`:1796`, so it matches the header badge and the user's own words.)* The alternative hardcodes `wcode` in the renderer and leaves the label inconsistent everywhere else.
- **O4 — when a notice is retired.** *(Recommended: at the next submit on that surface — literally "after bot gen".)* The alternative retires on `AgentStart`, which would also clear a notice typed mid-run the moment the run resumes.
- **O5 — the single blocker.** Only **O3** changes code beyond the TUI crate; the rest are local. If the human wants to defer the compiler-visible rename, O3's fallback ("hardcode in the renderer") keeps slice b inside `crates/wcode-tui`.

## 11. Facts not verified

- `cargo test` / `cargo clippy` were not run — the sandbox refuses them. All gate results are specified, not observed.
- The 9-entry missing-model diff is the explorer's, verified today only for `step-5-preview-free` (directly against models.dev). The other eight ids/limits must be re-read before the table edit.
- REPL parity for `/usage`, `/help`, `/copy`: the REPL prints to stdout with no notice concept, so nothing to clean; slice c's scope is unaffected (it is TUI-only).
- The exact composer-foot layout under a narrow terminal. The chip shares a width-budgeted ladder (`ui.rs:1816` `bottom_levels`, dropped least-important-first); `⠹ thinking 12.3s` vs the gauge/`plan`/`browse`/folio at 48 cols was not measured.
- Mouse behaviour of the planned root row beyond `set_focus` — `on_mouse_down` (`app.rs:3056`) is the only path; a click on a row published at the panel's very first content line (`ui.rs:2063` "The `agents` header owns area.y") was not tested.
- A real end-to-end compaction run against `opencode.ai/zen/go/v1` — needs `OPENCODE_API_KEY`; the keyless evidence is the unit test plus `--dump-config`.
- The VS Code extension is untouched by all four slices; whether it renders notices or thinking that would drift from the TUI's new behaviour was not checked.
- `step-5-preview-free`'s `interleaved.field = "reasoning_content"` is how models.dev describes its reasoning channel; the *kernel* handles reasoning via `LlmStreamEvent::ThinkingDelta`/`ThinkingReplace` (`streamfn.rs:1122`/`:1129`), but rig populating `reasoning_content` for this specific provider on the chat-completions wire was not confirmed.

---

## 12. Progress log

**2026-10-10 — human sign-off.** Approved with the recommendations: one W014, four slices; O2 no root badge; O3 root label → `wcode`; O4 notices retire at the next submit; O5 accepted.

**2026-10-10 — slice a shipped + second-layer APPROVED.**
`7e116cc` (catalog: 9 verified rows; `step-5-preview-free` 1M/65,536, so compaction fires at 983,616 input tokens, not 111,616) and `7c6d863` (`--dump-config` + the REPL banner name the effective window and its source; precedence mirrors `CompactionPolicy::context_window`). Gates re-run by the reviewer: `cargo build` / `cargo test --workspace` / `cargo clippy --workspace --all-targets` all green; live checks on the catalog / default / `WCODE_COMPACT_WINDOW` rungs and the refused-port retry path pass. Residual notes (non-gating, tracked in `docs/next-steps.md` row 77): 9 stale catalog ids to retire; the TUI gauge still ignores `[compaction] window` for unknown models; `--dump-config` prints no `[compaction] budget`; REPL `/usage` can read a stale model window.

**2026-10-10 — D018 + spec amendments landed.** `docs/decisions/D018-tui-live-thinking-annotation.md`; `docs/tui-design.md` amended at the real anchors (the brief's §1.3-B/§5 references had drifted): the sidebar section, the Notices bullet, the Thinking rewrite, the foot ladder, the glyph table, the Keys wording ("member row N").

**2026-10-10 — slice b sketch: BLOCKED at first layer, then settled.** The sketch (5 in-place `SKETCH` blocks) was verified against the locked design; the reviewer blocked two items and settled nine amendments (A0–A9): the root's badge column must reserve three blanks (`clipped_row` drops empty spans, so `String::new()` misaligns the glyph by 3 columns); the app-side click stub was unfalsifiable (hand-published hit map) and is replaced by a draw-based ui.rs test; the label fix extends to the socket live-roster path; the 6-tuple stands (no `SidebarRow`, no `#[allow]`); the numbering rule is a member counter (badge *k* ↔ `canonical_order()[k]`); `examples/dump.rs`, `examples/demo.rs`, `default_root()` and 8 doc-drift comments join slice b; commit hygiene excludes the slice-c sketch blocks and the stray `crates/wcode-tui/Cargo.toml` `toml` dep (an unrequested edit found during the review and reverted).

**2026-10-10 — slice c sketch parked.** Seven blocks sketched in place, then extracted verbatim to `docs/work/W014c-sketch-blocks.md` so slice b could commit clean (A9). The sketch carries: `Block::Transient(String)`, `Surface::retire_transient()` (descending index splice over `transcript`/`block_revs`/`cache` + `selected` clamp), the `submit()` call-site placement, the 30-anchor transient call-site switch list, the two exhaustive-`Block::` matches (`app.rs` `copy_text`, `ui.rs` `block_lines`), and the 19 existing tests that assert `Block::Notice` on a reclassified block and must gain `| Block::Block::Transient(text)`. Two corrections it found: W014's `app.rs:531` clamp citation is wrong (it is `PickerState::move_selection`; the right precedents are `insert_block`'s shift and `set_block_ranges`' clamp), and `copy_last` is shared by `/copy` and `Ctrl-Y`, so the split needs a `transient: bool` parameter rather than a context-sensitive `notice()`.

**2026-10-10 — tree hygiene.** A stray, unrequested `toml = { workspace = true }` dependency (an Omarchy theme comment) appeared in `crates/wcode-tui/Cargo.toml` during the sketch round; nothing consumes it (`examples/dump.rs`'s `toml::from_str` is fake `read`-tool output inside a `const`), and it was reverted before slice b was dispatched.

**2026-10-10 — slice b shipped + second-layer APPROVED.** `1bc3382 tui: draw a clickable wcode row atop the sidebar (W014b)` (5 files: `main.rs`, `app.rs`, `ui.rs`, `examples/{demo,dump}.rs`). `sidebar_rows_indexed()` (`app.rs:3221`) is the root-inclusive sibling — `canonical_order()` mapped with no filter, trailing `is_root`; the row loop (`ui.rs:2074-2089`) draws the root unbadged (badge column reserved as three spaces — `clipped_row` drops empty spans) with a member counter so badge *k* ↔ `canonical_order()[k]` for `Alt-N` parity; hit publish unchanged, so the root is clickable through the index-agnostic dispatch. The `wcode` label now covers the socket roster fallback, the live roster and the runtime feed (`main.rs:1006/:1017/:1049`), the local TUI root (`:1838`), `default_root()` and both examples. Two deviations from the first-layer verdict, both adjudicated: `dump.rs:227` docks the sidebar so the A7 frames can show the row (`Ctrl-B`, one line, dev-only, disclosed); and A2's clippy premise proved false at the boundary — a scratch crate showed clippy 1.97 fires `type_complexity` on the bare 6-tuple but not the 5-tuple — so the escape is a `pub(crate) type SidebarRow<'a>` alias (`app.rs:2255`), clippy's own suggested fix, no `#[allow]`, tuple semantics intact. Reviewer verdict: APPROVE, no blockers; all four negative controls trip a test; gates re-run green (cli 450 · tui 393 · harness 184, clippy 0). Residuals recorded in `docs/next-steps.md`.

**2026-10-10 — slice c shipped + second-layer APPROVED.** `12065f8 tui: retire slash-command notices at the next submit (W014c)` (`app.rs` +390/-58, `ui.rs` +28). `Block::Transient(String)` (`app.rs:213`) is a rendering twin of `Notice` (`ui.rs:697` byte-identical arm); `Surface::retire_transient` (`app.rs:1484`) is a descending splice over `transcript`/`block_revs`/`cache` with `open_turn`/`selected` decrements then a `set_block_ranges`-shaped clamp; `submit` retires at `app.rs:3579`, between the empty guard and the `/`-dispatch, so chatter dies at the next submit *on that surface* (the §7.2 script — `/usage` → `/help` → prompt — leaves one generation). 25 call sites reclassified, including the reviewer's blocker `accept_picker`'s `"no match to select"` (`:4318`), the running-guard notice (now through `push_transient`, closing the pre-existing three-vec desync) and the async `/usage` reply (`render_usage` → `push_transient`, `:1897`). Both exhaustive `Block::` matches (`copy_text`, `block_lines`) learned the variant; 22 assertions across 20 tests accept the twin; 4 new tests. Reviewer verdict APPROVE, no blockers; 11 negative controls; gates green (tui 397 · cli 450 · harness 184, clippy 0). Residuals in `docs/next-steps.md`.

**2026-10-10 — slice d sketch parked.** `docs/work/W014d-sketch-blocks.md` (454 lines) — the render change (`live_lines`/`content_lines` gain an `elapsed` parameter; the live arm becomes one `live_thinking_line(width, elapsed)` built from `thinking_header_line(width, false, false)` + the reused `format_ms` + the unchanged `▌` cursor), the `Surface::live_is_thinking()` predicate (no new field — `content.last()` on `Surface.live`, justified by the kernel's `append_thinking`/`append_text` split) with the `App::live_thinking()` wrapper, the composer-foot chip word (`⠹ thinking` vs `⠹ running` in `corner_titles`), the guarded `bump_live` in `set_run_elapsed` (R1), and four `todo!()` test skeletons. Friction recorded: the existing `thinking_auto_collapses_on_turn_end` pins today's OPPOSITE behaviour and must be rewritten; `wrap` must survive everywhere but the live arm; `content_lines`'s two 4-arg test callers break; a multi-stretch live message draws one row per stretch (settled at review); the number is the run's elapsed, not a per-stretch timer (deliberate, per D018).

**2026-10-10 — slice d shipped + second-layer APPROVED (W014 complete).** `cebd687 tui: render in-flight thinking as one annotation row (W014d)` (`app.rs` +175/-1, `ui.rs` +197/-31). The live arm (`ui.rs:953-962`) now pushes a single `live_thinking_line(width, elapsed)` (`ui.rs:1098`) — `thinking_header_line(width, false, false)` plus one dim `" {ms}"` span; the body `wrap(...)` is gone from the live path only, and the shared `▌` tail draws the cursor. `Surface::live_is_thinking` (`app.rs:1364`) reads `content.last()` on `Surface.live` (no new field, justified by `loop_.rs`'s `append_thinking`/`append_text` split); the foot chip (`ui.rs:1796-1800`) reads `⠹ thinking <t>` and reverts at the first text delta; `set_run_elapsed` (`app.rs:4530-4534`) bumps `live_rev` only while reasoning, so the timer advances without re-rendering prose. `thinking_auto_collapses_on_turn_end` — which pinned today's OPPOSITE behaviour — was REWRITTEN into `in_flight_thinking_is_one_annotation_row` (inverted first half, collapse half kept), and three tests joined it. Reviewer verdict APPROVE, no blockers; 9 negative controls (one, N2, caught a self-passing assertion in the first draft — the foot chip printed the same `1.2s`); gates green (tui 400 · cli 450 · harness 184, clippy 0); D018's citations refreshed and the docs committed separately.
