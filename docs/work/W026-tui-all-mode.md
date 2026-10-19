# W026 — TUI All mode: merge members into the main transcript

- **Status:** ☑ **done** — `b860a25`; first-layer APPROVED (round 3); second-layer APPROVED; zero `SKETCH` markers; workspace tests + clippy clean.
- **Decisions:** [D031](../decisions/D031-tui-all-focus-view-mode.md)
- **Tracker:** [next-steps.md](../next-steps.md) item 86
- **Sketch:** [`docs/sketches/tui-all-mode.md`](../sketches/tui-all-mode.md) (review-only; source of the interface)
- **Reference:** the VS Code All/Focus mode (`editors/vscode/src/render.ts:224`
  `renderMerged`, `reducer.ts:131`/`:535` the arrival `seq`, `webview.ts:40`
  `ViewMode`).

## 1. The ask

The TUI showed only the **focused** surface's transcript; a team member's turns
appeared solely in the working-team strip and the sidebar. Bring the VS Code
client's **All / Focus** view to the TUI: an **All** mode that merges the root's
and every member's committed blocks into one transcript interleaved by arrival
order, each under a dim origin head (`wcode` / `w1` …); **Focus** stays
byte-identical to today.

## 2. Scope / Non-scope

**Scope**
- A `ViewMode { Focus, All }` (default Focus) + `parse`/`label`.
- A client-local, arrival-ordered merge over the per-surface transcripts
  (`merged_order` + `append_block_lines_at`), the origin head, and the drawer
  branch.
- A monotonic App-level arrival ordinal stamped at the append chokepoints.
- A dedicated merged-scroll offset + its four `all_*` mirror fields.
- Chrome routing (root in All), the header `.seg`, `Ctrl-O`, `/view`,
  `[tui] view`, `--view`.

**Non-scope**
- No protocol/kernel change — the merge is client-side over the transcripts the
  app already holds (as `vscode-ui-v2-plan.md:75` states).
- No cross-surface selection/browse — All is display-only (its own follow-on).
- No merge of a member's **live** (uncommitted) stream.
- No `FOCUS` seg — the header adds a ` · ALL` run only.

## 3. Estimates

| workstream | value | complexity | risk | why |
|---|---|---|---|---|
| Ordinal + merge render | high | med | med | the design's core; the stamp must sit at every append chokepoint or the user's own messages sort astray |
| Merged scroll (mirror fields) | med | med | low | a view-aware copy of the five-field per-surface scroll; isolates merged scroll from a surface's |
| Chrome routing | med | low | low | the All transcript is nobody's, so identity/state come from the root |
| Config / flag / command / key | low | low | low | mirrors the existing `[tui] width` / `--tui` shapes |

## 4. Interface & structure (integration points)

- **`crates/wcode-tui/src/app.rs`** — `pub enum ViewMode` (`:2342`); `parse`
  (`:2352`) / `label` (`:2363`); `App` fields `view`, `next_seq` (`:2543`),
  `all_scroll` + `all_max_scroll`/`all_viewport`/`all_last_total`/`all_last_width`;
  `Surface.seqs` (a 4th parallel vec, lockstep at `push_block`/`insert_block`/
  `clear_hint`/`retire_transient`/`seed_hint_if_empty`); `merged_order` (`:4561`),
  `surface_label` (`:4551`), `append_block_lines_at` (`:4640`); `push_on` (`:4588`),
  `stamp_new` (`:4603`), `stamp_replayed` (`:4627`); the `AppEvent::Agent` arm and
  `seed_history` stamp; view-aware `scroll`/`sync_scroll`/`scroll_up`/`scroll_down`/
  `page`; `chrome_surface`/`chrome_status`/`chrome_running`/`chrome_elapsed`
  (`:5328`–`:5350`); the `Ctrl-O` arm + `KEYS` row; the `view` `Command` + arm.
- **`crates/wcode-tui/src/ui.rs`** — `origin_row` (`:765`); the All branch in
  `draw_transcript` (`:584`); the header seg (`:837`); `corner_titles` reads
  `chrome_*` (`:2029`).
- **`crates/wcode-tui/src/lib.rs`** — `ViewMode` re-export (`:37`); `TuiSpec.view`
  (`:51`); the `run` seed beside `tui.width`.
- **`crates/wcode-cli/src/config.rs`** — `TuiConfig.view` (`:413`) resolved via
  `ViewMode::parse` → `ConfigError::Tui`.
- **`crates/wcode-cli/src/main.rs`** — the `--view` flag (`:200`/`:319`), folded
  into `cfg.tui.view` before `Options` so both sites pick it up.

## 5. Plan

1. Sketch the interface — **deliverable** `docs/sketches/tui-all-mode.md`;
   **gate** first-layer review (APPROVE). ✔ (3 rounds; 6 blockers cleared)
2. Implement the ordinal + merge render — **deliverable** the `app.rs`/`ui.rs`
   core; **gate** `merged_order`/origin-head tests.
3. Merged scroll, chrome routing, config/flag/command/key — **deliverable** the
   remainder; **gate** the §8 tests.
4. Verify — **deliverable** the diff; **gate** second-layer review (APPROVE). ✔

## 6. Quality gates

- `cargo build --workspace` → ok.
- `cargo test --workspace` → 1347 passed (535 cli · 212 harness · 465 tui + suites).
- `cargo clippy --workspace --all-targets` → clean.
- `grep -rn 'SKETCH\|todo!()' crates/` → none.

*(The `markdown` syntax-theme tests race on a process-global `SYNTAX_THEME` — a
pre-existing, diff-orthogonal flake, `docs/next-steps.md:82`/`:1202`; they pass
36/36 in isolation.)*

## 7. Testing

### 7.1 Automated
36 new tests: 25 `app.rs` (`ViewMode` parse/label, the ordinal stamp incl. the two
hint-present off-by-one guards, `merged_order` interleave/ties/span, `seqs`
lockstep, `append_block_lines_at` + the cache-isolation guard, seed stamp, view-aware
scroll, `chrome_surface`, the `Ctrl-O`/`/view` no-op guards), 5 `ui.rs` (Focus
byte-identical; All interleave + origin head; the header seg All-only + Focus
byte-identical), 1 `lib.rs`, 3 `config.rs`, 2 `main.rs`.

### 7.2 How a human verifies it
```
cargo run -p wcode-cli -- --tui            # then press Ctrl-O (or /view all)
#  look for: a ` · ALL` seg after WCODE in the head; member blocks interleaved
#            under dim ` w1`/` wcode` origin heads; Ctrl-O back to the byte-identical Focus frame
cargo run -p wcode-cli -- --dump-config --view all   # tui.view: all
cargo run -p wcode-cli -- --view sideways            # exits loudly
```

## 8. Expected outcome

A TUI session can show one merged, arrival-ordered transcript of the whole team
(All), or today's focused transcript (Focus, the default). The mode is
client-local: no protocol change, no kernel change, a byte-identical frame until
opted in.

## 9. Log

- **2026-10-19** — opened from the untracked sketch `docs/sketches/tui-all-mode.md`.
  First-layer review BLOCKED twice (5 items, then 1 off-by-one); the sketcher
  amended each in place and the third pass APPROVED. Implemented in `b860a25`
  (`tui: All mode — merge members into the main transcript`, 5 files, +1056/−49).
  Second-layer review APPROVED (all amendments landed; the pinned invariants hold;
  the only test failure is the documented `markdown` theme flake). D031 recorded.
