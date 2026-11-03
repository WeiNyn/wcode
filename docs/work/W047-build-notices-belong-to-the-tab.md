# W047 — The build must not print: a conversation's notices belong to the tab that carries them — brief

- **Status:** **SHIPPED** — commits `3225094`, `bc742ed`, `ee9415d`, `0d44c5b`. §4.2 implemented verbatim.
- **Item:** W047 (W046 is closed; W041–W045 reserved per `docs/work/W040-multi-root-conversations.md` §14).
- **Predecessor:** W040. This item **closes W040 amendment (h)**, which named the defect and deferred it:
  > "`build_root` still emits the three informational stderr lines the single-root boot does (`INERT_WORKFLOW_NOTICE`, `team: …`, `workflow: {n} nodes`), because it runs the same `build_runtime`. On one terminal they interleave; in a TUI they would paint over the alternate screen. **Either they move into `RootParts`/`ConversationSpec` so the host chooses where they go, or the TUI tolerates them** — but `build_root`'s 'never a stdout write' is true and insufficient."
  W040 chose "tolerate" because nothing could build a second root yet. W046 made `/new`, `/resume` and `/tab new` build one, so the defect now fires on the paths the human uses.
- **Decisions:** adds an errata/closure row to W040's §15 for amendment (h). No new ADR (the rule is presentation, recorded in `docs/tui-design.md`).

---

## 1. The ask

> "1 thing left is when resume/new/etc trigger, there will be println of `team: xxx` which break the TUI"

Correct, and it is **stderr**, not stdout — `eprintln!`. Five sites, all inside `build_runtime`:

| site | line |
|---|---|
| `crates/wcode-cli/src/main.rs:1670` | `eprintln!("{INERT_WORKFLOW_NOTICE}");` |
| `:1729` | `eprintln!("warning: member `{name}` not resumed: {reason}");` |
| `:1741` | `eprintln!("team: {} (restored, {seeded} prior messages)", names.join(", "));` |
| `:1776` | `eprintln!("team: {}", names.join(", "));` |
| `:1792` | `eprintln!("workflow: {n} nodes");` |

Why they hurt now: `build_runtime` (`main.rs:1556`) is called by **two** parents — the single-root boot (`main.rs:2614`, which runs *before* the alternate screen, so printing is correct there) and the host factory (`roots.rs:905`, which runs *inside* the TUI, where a raw `eprintln!` writes into the alternate screen mid-frame and the next ratatui redraw leaves the artifact — exactly what the human saw).

---

## 2. Scope / Non-scope

### 2.1 In scope

1. **`build_runtime` stops printing and returns its notices**: `Result<(Runtime, Vec<String>), BootError>` (or an `out: &mut Vec<String>` — §4.1 picks one).
2. **The single-root boot prints them, unchanged and in the same order**, before the TUI/REPL starts — so `wcode -p …`, `--dump-*`, `--help` and the refused-port one-shot stay **byte-identical**.
3. **The host carries them to the client**: `RootParts` gains `notices: Vec<String>`, `build_root` forwards them, and `ConversationEvent::Added` carries them so the TUI can render them on the tab that owns them (§4.2 is the render rule).
4. **The same treatment for the startup root when it is adopted**: the TUI's root 0 is built by the boot, so its notices are already printed — no double-render (§4.3 states the rule).
5. **Tests**: the build returns rather than prints; the host forwards; the TUI renders; and the single-root output is byte-identical to today.

### 2.2 Non-scope (each names something a reasonable reader would assume is in scope)

- **The other `eprintln!`s in `main.rs`.** `serve`'s `serving N session(s)` (`:2073`), the `error:`/`warning:` lines on failure paths (`:2501`, `:2934`), `detected endpoint: {url}` (`:2514`), `--status`/`--provider` output, and `main.rs:1743`-region's restored-team line are *terminal* programs writing to a terminal. Only the five build-path lines are converted, because only those run inside the alternate screen. If a sixth is ever added to `build_runtime`, the test in §7.1 item 1 is the tripwire.
- **`println!` in the TUI's own crash/teardown paths.** Out of scope; they fire after the screen is restored.
- **A general logging/event system.** Not asked, and `AGENTS.md` keeps policy in code: the seam is one `Vec<String>`, not a logger.
- **Rewriting the `warning:` into a new severity level in the TUI.** The designer rules on wording (§4.2); no new `Block` variant and no new palette role unless that ruling says otherwise.
- **The `root.jsonl` / manifest artifacts.** Unchanged — notices are display, not session state.
- **`/tab close`, the clickable band, W041's browser hub.** Untouched.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|---|---|---|---|---|
| W1 | `build_runtime` returns notices | **high** | **low** | **low** | Five `eprintln!`s become `notices.push(…)`, its return type gains a `Vec<String>`, and the two call sites thread it. Compiler-enforced at both. |
| W2 | the boot prints them unchanged | **high** | **low** | **medium** | The risk is **ordering**: today the lines interleave with the build's other output; the boot must print them at the same point the build produced them, or `wcode -p`'s stderr changes. §4.1 fixes the position (immediately after the build returns, before any other print). |
| W3 | the host carries them | high | low | low | `RootParts.notices` + one field on `ConversationEvent::Added` + one push in `run_host`; no wire change (`wcode-tui` types). |
| W4 | the TUI renders them | **high** | **low** | low | One loop in `add_conversation` pushing the designer's block shape onto the new tab's root. |
| W5 | the single-root/TUI boundary (no double render) | medium | low | **medium** | The subtle one: root 0's notices are printed by the boot *and* would ride `Added` if the startup root went through the host. §4.3 pins which one wins, and a test pins it. |
| W6 | tests + docs | medium | low | low | Five unit tests and one `docs/tui-design.md` clause. |

---

## 4. Interface & structure

### 4.1 The mechanism: the build returns what it would have said

**Changed — `crates/wcode-cli/src/main.rs`:**

- `fn build_runtime(…, conversation: ConversationId) -> Result<Runtime, BootError>` (`:1556`) becomes `-> Result<(Runtime, Vec<String>), BootError>`, and each of the five sites becomes a `notices.push(...)` with **the exact same string** (the tests in §7.1 pin three of them character-for-character, including `INERT_WORKFLOW_NOTICE`, whose text is LOCKED at `:1502` and asserted by `f1_inert_notice_text_is_stable`).
- The doc comment at `:1504`-region currently ends "… print the `team: ...` line" — becomes "… collect the `team: ...` notice".
- The boot caller (`:2614`-region) destructures and **prints each notice in order, immediately**, before it does anything else with the runtime — that is what keeps `wcode -p …` and the one-shot byte-identical.
- **Why a returned `Vec` and not an `out: &mut Vec<String>`:** the signature already returns a `Result`, so a tuple costs one destructure at each of the two call sites, while an out-param lets a caller silently drop the notices (the exact failure mode being fixed). The tuple makes "you have notices" unmissable at both.

**Changed — `crates/wcode-cli/src/roots.rs`:**

- `pub struct RootParts` (`:117`) gains `pub notices: Vec<String>` with a doc line: *what the build would have printed; the host forwards them to the tab, and the single-root boot prints them instead.*
- `build_root_inner` (`:905`-region) keeps the pair and moves the `Vec` into `RootParts`. `build_root`'s teardown wrapper is unchanged: a failed build has no notices to report (they die with the error, as they do today).

**Changed — `crates/wcode-cli/src/roots.rs` `run_host` (`:1057`-region)** and **`crates/wcode-tui/src/conversations.rs`:**

- `ConversationEvent::Added(ConversationSpec)` (`conversations.rs:527`) → `Added { spec: ConversationSpec, notices: Vec<String> }`, updating the four construction/read sites (`roots.rs:1057`, `roots.rs:1131`, and the tests at `:2116`, `:2151`-region).

### 4.2 The render rule — the designer's ruling

**A build notice is a permanent `Block::Notice` on the arriving conversation's ROOT surface, one block per notice, verbatim from the CLI, pushed when the tab is added — so it is the tab's prologue, above the replayed history. It is dim, it has no glyph, and its emphasis is a leading WORD; `error` red stays reserved for a run that failed.**

1. **`Block::Notice`, never a transient.** `docs/tui-design.md:244`-`:250` already splits `Notice` by *origin*: "command chatter is transient … all retired at the next submit on that surface" versus "Run events are permanent … they are the record of what the run did." A build is a run event of the session itself, and it **cannot be re-run** — re-running means opening another tab. The asymmetry is the rule: chatter is transient because the user can ask again; a build is permanent because they cannot. Concretely, `retire_transient` fires at the next submit, so `warning: member w2 not resumed: <reason>` would die exactly when the user types their first prompt into that tab.
2. **Verbatim strings, with ONE source-side fix.** The fact is the CLI's, the role is the TUI's, and `Block::Notice` renders `wrap(text, width, "   ", "   ", dim())` (`ui.rs:1070`) — a plain dim row, which is exactly §2's "dim row in the content column, no glyph of its own". So `team: a, b` and `workflow: 3 nodes` cross over untouched, `warning:` **stays** (the prefix *is* the mark, the same mechanism as `⏹ aborted` — converting it to `⋯` would overload the compaction/retry prefix), and the one edit is `` main.rs:1729 `` dropping its backticks (`` `{name}` `` → `{name}`): a notice is plain text, never markdown, so backticks would print literally.
3. **One block per notice.** The block is the addressable unit — what browse selects and what `y` copies. `render_todos`' multi-line block (`app.rs:527`) is the app's one multi-row notice because it has a single subject (`todos:`); `team:`/`warning:`/`workflow:` are three.
4. **Pushed by `add_conversation`, immediately after `activate_conversation`** (`app.rs:3094`-region, `:3128`) — **before** the replay, because `seed()` inserts the `⋯ N earlier message(s)` divider at `transcript.len()` (`app.rs:2449`/`:2507`). Push later and the notices land *below* the history, describing a build that scrolled past.
5. **The seeded hint retires — deliberately.** `push_block` calls `clear_hint()` (`app.rs:1822`), so a tab arriving with a team opens **without** the `❯▌` mark. That is §1.4's own contract ("retired by the first block"): a tab that arrives with teammates is not a blank page. A tab with nothing to announce is unchanged.
6. **The two counts are not redundant.** The notice's `(restored, N prior messages)` sums the **members'** seeded messages (`main.rs:1738`); the divider's `N` is the **root's** replayed messages. Different subjects; do not merge them.
7. **No modal, no dedupe across tabs.** An arriving tab must not capture keys; and each transcript must explain itself alone, so tab 2's record never depends on tab 1 existing.
8. **Browse needs no change.** All blocks are selectable, so `Ctrl-G` on a fresh tab lands on the last notice rather than a turn — harmless, and exactly what `⏹ aborted` does today.

**The `docs/tui-design.md` amendment** (append to §4's **Notices** bullet, after "…`clear_hint` at the first block) and is in neither list."):

> **Build events are permanent too, and they land on the tab they built.** A conversation that arrives already built (`/new`, `/resume`, `/tab new`) carries the notices its root's build produced — `team: …`, `team: … (restored, N prior messages)`, `warning: member … not resumed: …`, `workflow: …` — as **one `Notice` block per notice, on that conversation's ROOT surface**, pushed when the tab is added and therefore **above** the replayed history: the prologue the `⋯ N earlier message(s)` divider already teaches. The strings cross over **verbatim** — the fact is the CLI's, the role is the TUI's — and a notice is plain text, never markdown, so no backticks and no emphasis: **emphasis inside a notice is a WORD, never a hue** (`⏹`, `⋯`, `warning:`), because `error` red is reserved for a run that failed (`Block::Error`) and a skipped team member is not a failed conversation. One block per notice, not one block of several lines: the block is what browse selects and what `y` copies, and `todos:` is the app's one multi-row notice because it has a single subject. **The consequence is deliberate:** a real block retires the seeded hint, so a tab that arrives with a team opens without the `❯▌` mark — it is not a blank page, and the mark is "retired by the first block" (§1.4). A tab with nothing to announce is unchanged, and no tab dedupes another's notice: each transcript explains itself alone.

**§2 needs no row** — no glyph, no role, no new `Block` variant.

### 4.3 The single-root boundary: printed AND rendered, in two media

The TUI's root 0 is built by the **boot** (`main.rs:2614`), adopted into `Roots`, and its `Added` never fires — `Options.conversations` seeds it. Two questions follow, and the designer's rule answers them:

- **Does the boot still print root 0's notices?** **Yes.** The `-p` one-shot, the REPL and `--dump-*` share that build, and the stderr copy is the post-quit record.
- **Does tab 0 also carry them as blocks?** **Yes** — "each transcript explains itself alone". So `main.rs` keeps root 0's `Vec<String>` from the build, prints it (byte-identical to today), *and* passes it into the startup `ConversationSpec`.

That is **not** a duplication inside the frame: one fact, two media — the boot's stderr line (invisible under the alternate screen, and the record after quitting) and the tab's block (the record *in* the transcript). State it in the code doc, or somebody will "fix" it later. A test pins both halves: the startup conversation DOES carry its notices as blocks, and no `Added` fires for it (so nothing renders twice).

### 4.4 What is NOT added

No new `Action`, no `Request`, no protocol field (`PROTOCOL_VERSION` stays 1 — `ConversationEvent` is TUI-side), no new crate, no dependency, no config key, no logging framework, no new `Block` variant.

---

## 5. Plan

1. **`cli:` the build returns its notices** — the five sites, the tuple return, and the boot printing them in order. Gate: `cargo test --workspace` green **including** `f1_inert_notice_text_is_stable`; and the byte-identical check in §7.1 item 2.
2. **`cli:` the host carries a conversation's notices** — `RootParts.notices`, `ConversationEvent::Added { spec, notices }`, the four sites updated. Gate: a host test asserting a `Fresh` root's `Added` carries the `team: …` notice for a two-member `[team]`, and carries none when the team is empty.
3. **`tui:` the tab renders them** — per §4.2, and the startup root gets them through `Options.conversations` per §4.3. Gate: `an_arriving_tab_carries_its_build_notices_on_its_root`, `a_tab_with_notices_never_shows_the_empty_state`, `a_build_notice_sits_above_the_replayed_history`, `a_build_notice_survives_the_next_submit` (`chatter(&app).is_empty()` after a submit — the assertion that separates it from a transient), and `the_startup_root_carries_its_notices_once` (§4.3).
4. **`docs:`** the `docs/tui-design.md` clause, and W040 §15's errata row closing amendment (h) with a pointer here.

---

## 6. Quality gates

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
python3 scripts/check-book.py      # only if a book page is touched
grep -rn 'eprintln!' crates/wcode-cli/src/main.rs   # no site inside build_runtime
```

## 7. Testing

### 7.1 Automated

1. **`build_runtime_collects_instead_of_printing`** — the tripwire for a sixth `eprintln!`: a debug-only assertion or a source-level test is not possible, so the test builds a runtime with a `[team]` and a resumed group and asserts the returned `Vec` carries the expected lines **and** that the five strings are unchanged.
2. **`the_one_shot_stderr_is_byte_identical`** — the byte-identical claim, pinned: run the boot path twice on the same fixture (or capture the printed vector) and compare against the literal lines, so a reordering reddens it.
3. **`a_fresh_root_s_added_carries_the_team_notice`** — `Roots::start` on a two-member team yields `Added { notices }` containing `team: a, b`.
4. **`a_resumed_team_says_it_restored`** — the `(restored, {n} prior messages)` line survives the move.
5. **`an_arriving_tab_carries_its_build_notices_on_its_root`** — N blocks, in order, the focus on that tab, per §4.2.
6. **`a_build_notice_sits_above_the_replayed_history`** — the `⋯ N earlier message(s)` divider lands *below* the notice (§4.2 item 4).
7. **`a_build_notice_survives_the_next_submit`** — `chatter(&app).is_empty()` after a submit: the assertion that separates a block from a transient.
8. **`a_tab_with_notices_never_shows_the_empty_state`** — no `EMPTY_HINT`/`ROOT_EMPTY_HINT` after `seed_empty_hint`, per §4.2 item 5.
9. **`the_startup_root_carries_its_notices_once`** — §4.3: the startup conversation carries them as blocks, and no `Added` renders them again.

### 7.2 How a human verifies it

```sh
cargo build --bin wcode
./target/debug/wcode --base-url http://127.0.0.1:9/v1 --model probe
```

1. **`/new` no longer paints over the screen.** Before: `team: brainstormer, …` written into the alternate screen. After: no stray line; the arriving tab carries the notice per §4.2.
2. **`/resume` a session that had a team** → the tab says it restored, and the previous tabs are untouched.
3. **The single-root path is unchanged**: `./target/debug/wcode -p "hi" --base-url http://127.0.0.1:9/v1` still prints the same `team:`/`workflow:` lines on stderr as before the change.
4. **`/reload`** still prints them once, on the terminal, because it re-execs.

## 8. Expected outcome

- No `eprintln!` inside `build_runtime`; the five notices are values the caller owns.
- The single-root boot's stderr is byte-identical to today.
- Every runtime root's build notices appear on **that tab**, in the designer's ruling's shape, and never paint over the alternate screen.
- The startup root does not double-render.
- W040 amendment (h) is closed by an errata row, not by a silent edit.
