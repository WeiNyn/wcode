# W046 — Tabs that stay put: `/new` and `/resume` open a conversation, and a chord to move between them — brief

- **Status:** **SHIPPED** — 8 commits `ed5309e`…`30486ec` plus the review-clearance commits; second-layer review **BLOCKED on 3 cheap items then cleared** (§14). Recorded in the tracker as row **113** and as **D046** (`docs/decisions/D046-one-chord-one-axis.md`). Scope narrowed in one place against the brief: §13.2 (the dedupe carries `resume_path`, not `Status.session`).
- **Item:** W046 (W041–W045 reserved per `docs/work/W040-multi-root-conversations.md` §14; W046 was free).
- **Predecessor:** W040, which shipped the machinery this item only *steers*. Read `docs/work/W040-multi-root-conversations.md` §13.2, §13.6.1 and §13.2.1 before this brief.
- **Brief author:** `agent:brainstormer` (read-only; no build was run). §12's chord table is `agent:designer`'s, recorded by the orchestrator.
- **Decisions:** proposes **D046** — *one chord, one axis: the cycle goes to the axis with no direct addressing; the bounded axis keeps direct keys plus the one direction it cannot address* (§12).

---

## 1. The ask

The human, verbatim:

> "recently we implement multi tab/conversation for wcode TUI, but:
> - /new /resume trigger the re execution, so it break the tab things
> - we should how a shortcut to move around tab"

Two defects in one report, and they are different in kind. The first is **reducer semantics**: the TUI's `/new` and `/resume` are still *quit-and-re-exec* hand-offs (`crates/wcode-tui/src/app.rs:6757` `fn request_new(&mut self) {` → `:6762` `self.pending_new = true;` `:6763` `self.should_quit = true;`; `app.rs:6620` `PickerKind::Resume =>` → `:6623` `self.pending_resume = Some(PathBuf::from(selected));` `:6624` `self.should_quit = true;`), so using either **destroys every open tab** — the whole in-process root set — and re-execs the binary (`crates/wcode-cli/src/main.rs:2432`/`:2447` → `repl::exec_self` → `crates/wcode-cli/src/repl.rs:648` `let err = std::process::Command::new(&exe).args(args).exec();`). The multi-tab machinery that should absorb them already exists and is live (`Action::NewConversation` at `app.rs:155`, `/tab new` at `app.rs:5448`, the host loop at `crates/wcode-cli/src/roots.rs:997` `pub async fn run_host(roots: Roots, factory: Factories, channel: HostChannels) -> Roots {`), so this is a change of *which* path the two commands take, not new plumbing.

The second is **discoverability**: there is no chord that unambiguously means "move between conversations". `Ctrl-N`/`Shift-Tab` mean *conversations* when N > 1 and *surfaces* when N = 1 (`app.rs:5752` `fn cycle_or_focus(&mut self, step: isize) {` → `:5753` `let len = self.conversations().len();` `:5755` `self.focus_step(step);`), the `F1` table still claims surfaces only (`app.rs:1332` `("Ctrl-N / Shift-Tab", "focus the next / previous surface"),`), and `docs/tui-design.md:495` says the same — so **the keymap lies at N > 1**.

---

## 2. Scope / Non-scope

### 2.1 In scope

1. **`/new` opens a conversation tab** and switches to it — the same emit as `/tab new` (`Action::NewConversation(RootSpec::Fresh { label: None })`), nothing else. No quit, no re-exec.
2. **`/resume` (and `/resume <filter>`) opens the picked session as a new tab** and switches to it.
3. **Legacy flat `.jsonl` sessions resume as tabs too** — `build_root`'s `Resume` arm gains the dispatch the single-root boot already has. Without this, `/resume` becomes a *narrower* command than it is today.
4. **A pick of a session that is already open as a tab switches to that tab** instead of asking the host to build a second writer over one `root.jsonl`.
5. **Deleting the now-dead re-exec plumbing**: `Outcome::{New, Resume}`, `pending_new`, `pending_resume`, their accessors, `outcome_of`'s two arms, and the four `main.rs` arms (two local, two socket), plus `repl::new_session_args` and its test.
6. **A navigation chord for conversations** (§12 locks it): dispatch + `KEYS` row + `/help` (one table, so free) + a reducer test at N = 1 and N = 2, with the `docs/tui-design.md` §4 amendment landed in the same commit.
7. **Deleting the four stale `SKETCH` blocks** (§4.8) — as its own commit; the two *live* blocks stay.
8. **Docs**: `README.md` (the TUI slash-command paragraph and table), `docs/book/04-the-tui.html` §4.7, `docs/tui-design.md` §4 Keys + the `/resume` sentence, W040 §13.4's overruled recommendation + a §15 errata row, `docs/next-steps.md` row 112's "still open" tally.
9. **Deleting the `/clear` alias** (D2) — user-visible, and in scope because a `/clear` that opens a tab lies (§13.4 item 7 names its two tripwires).

### 2.2 Non-scope (each names something a reasonable reader would assume is in scope)

- **Clicking a tab.** The band is still not clickable (`crates/wcode-tui/src/ui.rs:309` "**Not yet clickable**: the row's tab extents are not published into a hit map"). Its design exists as a live sketch (`app.rs:2817`–`:2877`) and belongs to W040's remaining work; the human asked for a *shortcut*.
- **A `Starting`/ghost tab while a build is in flight.** `ConversationState` is three-valued — `Idle`/`Running`/`Failed` (`crates/wcode-tui/src/conversations.rs:106`) — so a fourth state is a design amendment for a window this brief declines to invent (§4.6).
- **`pending_conversation`** (the sketched "one build at a time" gate, `app.rs:3498`–`:3502`). The host loop is a single sequential task (`roots.rs:1007` `loop { tokio::select! {`), two `Start`s build two roots, and both are things the user asked for.
- **Per-tab model / effort / team.** `RootSpec` deliberately has no such field — `crates/wcode-protocol/src/conversation.rs:131` "There is deliberately **no field for the model, the team, the tools or the hooks**". `/team` therefore keeps re-exec'ing.
- **`/reload`.** It rebuilds the *binary* and re-execs by design; a tab cannot absorb it. It keeps its meaning and its status as the one command that deliberately ends all tabs.
- **The REPL's `/new` and `/resume`.** `repl.rs:1002`/`:1115` rebuild the agent in-process; they never re-exec and they have no tabs to break. Untouched.
- **`/clear`'s old meaning.** Dropped, not re-homed (§4.0 D2). `/reload --no-session` is *not* a substitute: it disables sessions entirely (`repl.rs:449` `if no_session || session.is_none() { args.push("--no-session") }`), whereas the old `/new` minted a fresh *persisted* session.
- **W041's browser hub, the tab rail's visual redesign, `/tab move|rename`.** Out; W041 starts from the same `Roots`.
- **`#[allow(dead_code)]` residue and the `Roots::{iter,len,...}` tally.** W040 amendment (k)'s business.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|---|---|---|---|---|
| W1 | `/new` → tab | **high** | **low** | **low** | The body's two lines are replaced by the `Action::NewConversation` emit that `/tab new` already proves end to end (`app.rs:5448`, `lib.rs:673`, `roots.rs:1012`); the only new fact is "no quit". |
| W2 | `/resume` → tab | **high** | **low** | **medium** | Same one-arm swap, but two decisions are genuinely new — the legacy row (W3) and the self-resume pick (W4) — and both are *user-visible refusal paths* if done cheaply. |
| W3 | legacy `.jsonl` as a tab | **medium** | **medium** | **medium-high** | It touches `build_root`, the path W040's review blocked on twice (amendments (g)/(i)); the failure mode is a second writer to one transcript. It is a ~15-line lift of `load_session`'s existing dispatch, not new logic. |
| W4 | dedupe an already-open session | **medium** | **low** | **low** | One comparison per tab over `Status.session` (`app.rs:1522` `pub session: Option<String>,`), and `Roots::contains_session_path` (`roots.rs:552`) stays the backstop — a wrong comparison's worst case is today's behaviour. |
| W5 | delete dead `Outcome::{New,Resume}` | **medium** | **low** | **low** | Compiler-enforced, and the producer census is closed (`grep` finds only `outcome_of` and two tests). |
| W6 | conversation nav chord | **high** | **low in code, medium in decision** | **medium** | The dispatch is one line and the `KEYS` row one string, but the chord must dodge five keyboard tiers and its meaning is the design doc's call — most of the cost is the amendment, not the diff. Low now: §12 locks it to `Ctrl-P`, which needs no `Key` variant. |
| W7 | delete 4 stale `SKETCH` blocks | **medium** | **low** | **low** | Text only, each block checked against shipped code first; the risk is the two blocks that are *not* stale, which §4.8 names explicitly. |
| W8 | docs (README, book, design, W040 errata, next-steps) | **medium** | **low** | **low** | Five prose files; `scripts/check-book.py` is the machine gate, and every claim is a sentence this item made true. |

---

## 4. Interface & structure

### 4.0 The decisions this brief takes (each a real question, not boilerplate)

| # | question | decision | why (with the anchor) |
|---|---|---|---|
| D1 | Does `/new` keep the destructive meaning? | **No.** `/new` opens a tab. The fresh-process meaning survives only at the process boundary (`wcode` with no `--resume`) and as `/reload --no-session` (sessions *off*). | The human overrules W040 §13.4's "Recommendation: keep `/new` unchanged for this item" (`docs/work/W040-multi-root-conversations.md:555`) — that sentence is why the bug exists. `RootSpec::Fresh` is literally the new-session request (`conversation.rs:163`). |
| D2 | Does `/clear` survive as the destructive alias? | **No — the alias is deleted** (`app.rs:832` `aliases: &["clear"],`). Summary becomes "open a new conversation". | `/clear` promises "clear this conversation", which no code path can do in place (the transcript *is* the session file). An alias whose name promises a different, destructive act is a trap. |
| D3 | Legacy flat `.jsonl` rows in the picker? | **Teach `build_root` the legacy file**, by reusing `session_groups::group_dir_of` (`crates/wcode-cli/src/session_groups.rs:315`) — the dispatch `load_session` already uses (`main.rs:1344` `match session_groups::group_dir_of(&p) {` … `:1376` `None => match Session::open(&p) {`). | Filtering hides sessions that exist; refusing makes `/resume` narrower than today. `conversation.rs:150` already *promises* this: "a bare flat `.jsonl` is the legacy path and stays reachable by resuming it as a one-member group." Today `build_root` rejects both a flat file (`roots.rs:851` `if !crate::session_groups::is_group_dir_name(name)`) and a `<dir>/root.jsonl` that `group_dir_of` accepts. `Roots::contains_session_path` (`roots.rs:552`) then needs a `|| root.session_path == dir` clause, because a legacy root has `group: None`. `RootError::BadResume`'s text (`roots.rs:83`) becomes "`{} is not a session`". |
| D4 | Picking a session that is already a tab? | **Switch to that tab; emit nothing** — by carrying a real identity, NOT by comparing `Status.session`. **SUPERSEDED BY §13.2 (the reviewer blocked D4 as drafted): `Status.session` is `None` for every runtime root, so the comparison could not fail when wrong.** The cleared design is: a `resume_path: Option<PathBuf>` on `SurfaceInfo`/`SurfaceSpec`, filled by the host, compared path-for-path. | W035's filter drops only *blank* sessions (`main.rs:2798` `&& !s.messages().iter().any(is_real_user_message)`), so **the tab you are in is listed as soon as it has one message** — the pick you are most likely to make is your own. Today's answer is `RootError::AlreadyLive(dir.display())` → a transient naming a group dir at a user who clicked a title. Cutting the dedupe instead is refused: it would turn a pick that works today (re-exec into it) into a failure. |
| D5 | Await the tab before switching? | **No.** Keep the async round trip; the honest UI is the existing `transient("opening a conversation…")` (`app.rs:5451`) plus the view switching when `Added` lands (`app.rs:3277`). | The build runs in the host task (`main.rs:2364` `let host = tokio::spawn(roots::run_host(`), so the TUI never blocks; the band cannot show a `Starting` tab because the state table is three-valued. Anything better is a design amendment, not this item. |
| D6 | `/resume` over `--socket`? | **Refuse at pick time**, mirroring `/tab new` (`app.rs:5444` `if self.remote {`), not at exit. | A socket client's `Options.sessions` is already empty (`main.rs:1108` `sessions: Vec::new(),`), but the emit path is unguarded and the exit path is `eprintln!("tui: cannot resume over a socket"); exit(1)` (`main.rs:1240`). A refusal that keeps the process alive is strictly better. |
| D7 | Are `Outcome::New`/`Resume` dead? | **Yes → DELETE both**, with `pending_new`/`pending_resume`, both accessors, `outcome_of`'s `:374`/`:378` arms, `main.rs:2432`/`:2447` (local) and `:1240`/`:1248` (socket), `repl::new_session_args` (`repl.rs:464`) + its test, and the app/lib tests that pin them. `Outcome::{Team, Reload}` survive (both have live producers). | Nothing else sets the fields; dead code is a defect in this repo. |
| D8 | Which `SKETCH` blocks go? | **Four of six are stale → delete**: `app.rs:982`–`:1129`, `app.rs:3479`–`:3636` (fold its one unfinished idea, `pending_conversation`, into the §2.2 refusal), `app.rs:4811`–`:4849`, `ui.rs:174`–`:249`. **KEEP two, because their code does not exist**: `app.rs:2817`–`:2877` (the tab hit map) and `main.rs:1924`–`:1996` (`serve()` adoption). | This **corrects the premise handed to the brief** ("six live marker pairs above already-implemented code"): two sit above code that is not implemented, and deleting them would erase the only written design for the clickable band and the `serve()` adoption. W040's row 112 claims "the `main.rs` in-branch SKETCH block is gone" — it is not; W046's doc step corrects that sentence rather than acting on it. Before deleting the other four, each surviving rationale lands in the real doc comment. |
| D9 | Which chord? | **`Ctrl-N` / `Ctrl-P` = next / previous conversation tab; `Shift-Tab` = previous surface; `Alt-1..9` = direct surface; `/tab <n>` = direct tab.** The forward surface cycle retires as redundant. **Locked in §12 on the designer's ruling.** | A chord whose meaning depends on N is the ambiguity the human reported. The full argument, the rejected alternatives (`Ctrl-Tab`, `Alt-←/→`, `Alt-h/l`, `Ctrl-F/B`, others) and the measurements are §12. |

### 4.1 The reducer swap (W1, W2)

**Changed — `crates/wcode-tui/src/app.rs`:**

- `:5282` `"new" => self.request_new(),` → the `/tab new` body, factored into one helper both commands call, so the two cannot drift. Today the emit is inline at `:5447`–`:5451`.
- `:6757`–`:6764` `fn request_new` — **deleted** (its `remote` refusal moves into the shared helper). `:6733` `pub fn pending_new(&self) -> bool {` — deleted. `:2930` `pending_new: bool,` and `:3129` `pending_new: false,` — deleted.
- `:6620`–`:6624` `PickerKind::Resume =>` → the same helper with `RootSpec::Resume { dir: PathBuf::from(selected) }`. The picker's `values` are the host path (`app.rs:6269`), and for a group row that is the group **dir** (`session_groups.rs:127` `GroupEntry::Group(g) => &g.dir,`), which is what `RootSpec::Resume { dir }` wants (`conversation.rs:177`).
- `:6260` `fn open_session_picker(&mut self, filter: Option<&str>)` — unchanged behaviour (the argument pre-fills `picker.query`, `:6272`), plus the D6 `remote` refusal and the D4 dedupe hook at select time.
- `:2921` `pending_resume: Option<PathBuf>,`, `:3126`, `:6715` `pub fn pending_resume(&self) -> Option<&Path> {` — deleted.
- `:1321` `pub(crate) const KEYS: &[(&str, &str)] = &[` — the `:1332` row replaced per §12 (and `Shift-Tab` gains its own row; `KEYS` 35 → 36).

**Changed — `crates/wcode-tui/src/lib.rs`:**

- `:344` `pub enum Outcome {` — `:352` `Resume(PathBuf),` and `:362` `New,` deleted; `:368` `fn outcome_of(app: &App) -> Outcome {` loses its `:374`/`:378` arms (the force-quit precedence at `:369` must stay first).

**Changed — `crates/wcode-cli/src/main.rs`:** `:1240`, `:1248`, `:2432`, `:2447` — all four arms deleted. **`repl::new_session_args` STAYS** (corrected by §13.1: it has a second caller, `main.rs:2467` `let mut argv = repl::new_session_args(` in the `Outcome::Team` arm that survives), and so does `new_session_args_has_no_session_flags_but_keeps_launch_opts` (`repl.rs:2702`). `repl::reload_args` survives too (`:2415`-region).

**Added:** one helper, `fn open_conversation_tab(&mut self, spec: RootSpec)` — `remote` refusal, D4 dedupe, emit, transient. Its two callers are `/new` and the resume pick.

### 4.2 The legacy resume (W3)

**Changed — `crates/wcode-cli/src/roots.rs`:** the `Resume` arm at `:846` `RootSpec::Resume { dir } => {` replaces its `is_group_dir_name` gate (`:851`) with `session_groups::group_dir_of`'s dispatch, mirroring `main.rs:1344`–`:1381`: a group dir (or `<dir>/root.jsonl`) → `open_group` + `load_root`, `resuming_group = true`, `group = Some`; anything else → `Session::open(&dir)` + its messages, `resuming_group = false`, `group = None`. The model/effort restore at `:858`–`:867` stays on both branches.

**Changed — `roots.rs:552`** `pub fn contains_session_path(&self, dir: &Path) -> bool {` — add `|| root.session_path.as_deref() == Some(dir)`.

**Changed — `roots.rs:83`** `RootError::BadResume(dir) => write!(f, "{} is not a session group directory", …)` → "`{} is not a session`", test at `:2197` updated.

**Unchanged:** `conversation.rs`'s `RootSpec` shape (no new field — the tag is the *path*, and `group_dir_of` decides). Note the divergence from `conversation.rs:151`'s "one-member group" wording: a legacy file resumes as itself, not as a manufactured group. That sentence is fixed in step 4.

### 4.3 The dedupe (W4) — **SUPERSEDED BY §13.2**

As originally drafted this compared the picked path's stem to `Status.session`. That is unfalsifiable: `app.rs:1543` `session: None,` for every runtime root. The cleared design is §13.2 — a `resume_path` field on the surface spec, filled by the host, compared path-for-path. `conversation_of_session` survives as the accessor, but it reads `resume_path`, never `Status.session`.

### 4.4 The nav chord (W6) — as locked by §12

- Dispatch: `app.rs:5076` `Key::Ctrl('n') => self.cycle_conversation(1),` and a new `Key::Ctrl('p') => self.cycle_conversation(-1),`; `app.rs:5077` `Key::BackTab => self.focus_step(-1),` (byte-identical behaviour at every N).
- `app.rs:5752` `cycle_or_focus` → `cycle_conversation(step)`: inert at N = 1, no `focus_step` fallback, **no transient** (§12.3 — `activate_conversation` already sets `dirty`, `app.rs:3353`).
- `app.rs:1332` — the `KEYS` rows become `("Ctrl-N / Ctrl-P", "next / previous tab")`, `("Shift-Tab", "focus the previous surface")`, `("Alt-1..9", "focus sidebar row N")` (unchanged).
- `/help` needs no edit: `help_rows()` (`:1391`) iterates `KEYS` (`:1399`), and `ui.rs:500` draws `F1` from the same table.
- **No `Key` variant, no `event.rs` edit, no band legend, no digit chord.**

### 4.5 What is NOT added (so a reviewer can see the shape of the diff)

No new `Action`, no new `ConversationEvent`, no `RootSpec` field, no protocol change (`PROTOCOL_VERSION` stays `1`), no new crate, no new dependency, no config key. **One TUI-local field IS added, corrected by §13.2: `resume_path: Option<PathBuf>` on `SurfaceInfo`/`SurfaceSpec`** — it is not the wire, and it retires `Root::session_path`'s `#[allow(dead_code)]`.

### 4.6 The open UI question, stated instead of invented

Between the emit and `Added` the user sees a transient in the transcript of the tab they are leaving, and nothing in the band (which does not exist at N = 1). That is the whole truth; D5 records why no placeholder is added.

### 4.7 The chord's constraint set (the input the designer ruled on)

1. **Reachable** by crossterm on the terminals this repo targets; no invented key.
2. **Expressible** as an existing `Key` variant, or an explicit `event.rs` addition under `translate_key` (`crates/wcode-tui/src/event.rs:34` "An Alt chord is its own thing"). A modified arrow costs a new `Key` variant **plus** `translate_key` arms **plus** every exhaustive `Key` match.
3. **Free in all five keyboard tiers**, in order (`app.rs:4997` `fn on_key`): overlay → settings panel → inline completion → browse → composer. A conversation chord is therefore **inert in browse** unless the amendment says otherwise.
4. **Already bound** (input tier): `Enter`, `Newline`/`Ctrl-J`, arrows, `Home`/`End`, `PgUp`/`PgDn`, `Esc`, `Ctrl-C`, `Ctrl-Y`, `Ctrl-N`, `BackTab`, `Tab`, `Alt-<char>` (all of it), `F1`, `F2`, `Ctrl-G`, `Ctrl-B`, `Ctrl-O`, `Ctrl-T`, `Ctrl-A`, `Ctrl-E`, `Ctrl-W`, `Ctrl-U`, `Ctrl-K`.
5. **Fits the `F1` column**: chord ≤ 22 cells (`ui.rs:35` `pub(crate) const ITEM_LABEL_COLS: usize = 22;`), detail ≤ 39 cells (the 80-col panel's real budget: `panel_inner_width` `ui.rs:609` = 62, `item_line` `ui.rs:485` spends 1 + 22).
6. **Both directions**, or one chord with a documented wrap.
7. **One row in `KEYS`** whose detail keeps the panel's margin; the row-render tests derive their height (`ui.rs:7304`, `app.rs:10293`, `app.rs:12464`), so only the strings matter — but the F1 scroll window is at a **37 = 37 cliff** (`ui.rs:4205` renders at 64×40; `info_visible_rows` `app.rs:408` = `min(37, 37)`), so a *second* new row would clip.
8. **Terminal caveats**: `Ctrl-[` is usually `Esc`; `Ctrl-I`/`Ctrl-M`/`Ctrl-H` collide with `Tab`/`Enter`/`Backspace`; `Ctrl-Tab` collapses to `Tab` because `event.rs:51` `KeyCode::Tab => Key::Tab,` is reached before any modifier guard and `terminal.rs:30` pushes no keyboard-enhancement flags.

### 4.8 The `SKETCH` census (measured)

| block | describes | verdict |
|---|---|---|
| `app.rs:982`–`:1129` | the `/tab` `COMMANDS` entry + the `KEYS` row + `cycle_or_focus` + the two close/open requests | **stale** — all shipped (`:5438`, `:1332`, `:5752`, `:5726`) |
| `app.rs:2817`–`:2877` | `TabsHit`/`set_tabs_hit`/`tab_at`/`activate_conversation_id` + the click branch | **live** — none exists (`grep` returns only the sketch) |
| `app.rs:3479`–`:3636` | the `conversations` + `pending_conversation` fields and seven methods | **stale except `pending_conversation`** (shipped: `:3232`, `:3244`, `:3286`, `:3293`, `:3309`, `:3339`, `:5752`) |
| `app.rs:4811`–`:4849` | the `SessionTitle` → `set_label` hook and the state hook | **stale** — both shipped (`:4808`-region, `:4986`) |
| `ui.rs:174`–`:249` | `tabs_h`, the layout renumbering, `draw_tab_bar`, and the why | **stale** — shipped (`ui.rs:180`-region, `:256`-region, `:343`), rationale already in `draw_tab_bar`'s doc |
| `main.rs:1924`–`:1996` | `serve()` adopting root 0 into `Roots` | **live** — `serve()` still calls `SessionActor::spawn` directly |

---

## 5. Plan

Each step is one commit, in order. Step 0 is done (§12) and gates step 6.

0. **The designer's keymap amendment — DONE, recorded in §12.** Deliverable: `docs/tui-design.md` §4 Keys names `Ctrl-N`/`Ctrl-P`, settles `Ctrl-N`'s single meaning, records that the chord is inert in browse, and appends the §4 Tab-band clause. Gate: `grep -n "Ctrl-N" docs/tui-design.md` shows one meaning, and that sentence matches the `KEYS` string byte for byte (lands in step 6's diff).

1. **`/new` opens a conversation.** Deliverable: one shared `open_conversation_tab` helper; `"new"` and `/tab new` both call it; `request_new`/`pending_new` deleted; app tests rewritten. Gate: `cargo test -p wcode-tui` green including `slash_new_opens_a_conversation_instead_of_quitting`; `cargo clippy -p wcode-tui --all-targets` clean.

2. **`/resume` opens a conversation.** Deliverable: the `PickerKind::Resume` arm emits `RootSpec::Resume { dir }`; the `remote` refusal; `pending_resume` deleted; the five resume tests rewritten. Gate: `cargo test -p wcode-tui resume` green, including the filter case and the socket refusal.

3. **Delete the dead re-exec plumbing.** Deliverable: `Outcome::{New,Resume}`, both `outcome_of` arms, both accessors, both fields, the four `main.rs` arms — gone; `Outcome` documented as "Quit · Abandoned · Team · Reload". **`repl::new_session_args` and its test STAY** (§13.1). Gate: `cargo build --workspace` has no unused-variant/unused-fn warning; `grep -rn "pending_resume\|pending_new\|Outcome::New\|Outcome::Resume" crates/` returns nothing (**do NOT grep `new_session_args` — it survives**); `cargo test --workspace` green, with `new_session_args_has_no_session_flags_but_keeps_launch_opts` still passing.

4. **Legacy `.jsonl` resumes as a tab.** Deliverable: `build_root`'s `Resume` arm dispatches through `group_dir_of`; `contains_session_path` also matches `session_path`; `BadResume` reworded; `conversation.rs:151`'s "one-member group" sentence corrected. Gate: `resume_from_a_legacy_flat_jsonl_builds_a_root` and `a_legacy_file_resumed_twice_is_refused` green, and `a_failed_team_build_releases_what_it_already_spawned` still green (the teardown contract must survive the new branch).

5. **Dedupe an already-open session** (design per §13.2, not §4.3 as first drafted). Deliverable: `resume_path` on `SurfaceInfo`/`SurfaceSpec`, filled at `Root::surface_specs` (`roots.rs:1168`-region) and for the startup root in `main.rs`, plus `conversation_of_session` comparing `PathBuf`s; the pick activates instead of emitting. Gate: `a_pick_of_an_already_open_session_activates_that_tab` and `…_legacy` green **with the field set by the production path, not hand-written in the test** (that is the whole point of §13.2); `contains_session_path`'s existing tests untouched and green; `Root::session_path`'s `#[allow(dead_code)]` gone.

6. **The conversation chord.** Deliverable: the dispatch + `KEYS` rows of §12, and the `docs/tui-design.md` §4 text in the same commit. Gate: `ctrl_p_is_the_previous_tab_and_ctrl_n_the_next`, `cycle_keys_are_inert_at_n1`, `switching_tabs_writes_no_block`, plus `help_rows_are_items_in_f1_style` (`app.rs:12455`) and `help_rows_render_in_f1s_two_column_style` (`ui.rs:7288`) green unmodified — they prove `F1` and `/help` still agree.

7. **Delete the four stale `SKETCH` blocks.** Deliverable: `grep -rn SKETCH crates/` returns exactly two pairs (`app.rs:2817`, `main.rs:1924`), and those two headers say why they stay and what retires them. Gate: that grep, plus `cargo build --workspace`.

8. **Docs.** Deliverable: `README.md` (the TUI slash-command paragraph and table: `/new` = open a conversation, `/resume` = open it as one, `/tab new|close|<n>` documented — it never was); `docs/book/04-the-tui.html` §4.7 rewritten (it says "Four commands are *client-side re-execs*" and lists three; after W046 only `/reload` and `/team` re-exec); W040 §13.4's `/new` recommendation struck with a pointer to this item plus a §15 errata row; `docs/next-steps.md` row 112's tally corrected to what W046 actually deleted/kept. Gate: `python3 scripts/check-book.py` clean; `grep -rn "re-exec" README.md docs/book/04-the-tui.html` shows only `/reload` and `/team`.

---

## 6. Quality gates

**Regression list, corrected by §13.3:** the four `Ctrl-N` tripwires below go red when step 6 removes the N = 1 delegation — they press `Ctrl-N` to move between *surfaces*. Re-point each at **`Shift-Tab`** (a drop-in at two surfaces: 0 → 1 → 0), never at `Alt-0` (`focus_digit` has no root case; `alt_n_focuses_the_numbered_sidebar_row`, `app.rs:10614`, pins that `Alt-0` is a no-op): `ui.rs:4899` `the_root_row_is_clickable_through_the_published_hits`, `ui.rs:5452` `the_mark_is_seeded_only_on_the_root_surface`, `app.rs:7482`+`:7487` `dequeue_targets_the_queue_s_own_surface`, `app.rs:7506` `a_multi_surface_dequeue_starts_while_another_is_running`.

Exact commands, at the end of every step that touches code:

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
python3 scripts/check-book.py          # step 8 only
grep -rn "SKETCH" crates/              # step 7 only: exactly 2 pairs remain
grep -rn "pending_resume\|pending_new\|Outcome::New\|Outcome::Resume" crates/   # step 3 only: empty
```

**Regression tests that must stay green, by name:**

- `backtab_moves_focus_backwards_and_wraps` (`app.rs:10633`) — the N = 1 contract for `Shift-Tab`. §12 keeps it untouched.
- `slash_surface_and_the_cycle_key_switch_focus` (`app.rs:10077`) — its `Ctrl-N` half becomes `Alt('1')` + `Shift-Tab`; rename it.
- `cycle_or_focus_cycles_surfaces_at_n1_and_conversations_at_n2` (`app.rs:9972`) — **encodes the rejected N-dependent choice**; rewritten as `ctrl_p_is_the_previous_tab_and_ctrl_n_the_next` + `cycle_keys_are_inert_at_n1`, and the diff must say so rather than quietly deleting the assertion.
- `slash_tab_new_requests_a_conversation` (`app.rs:9899`-region) and `slash_tab_close_…` (`:9945`-region) — the emit predicates W1/W2 reuse.
- `resume_with_no_sessions_says_so` (`app.rs:10142`), `reload_requests_a_reexec_and_quits` (`:10152`-region), `reload_is_refused_over_a_socket`, `idle_quit_is_not_abandoned` (`lib.rs:1583`), `force_quit_maps_to_outcome_abandoned` (`lib.rs:1551`).
- `a_root_error_says_what_a_human_must_do` (`roots.rs:2195`) — its `BadResume` string changes in step 4.
- `a_failed_team_build_releases_what_it_already_spawned` — the step-4 branch must not weaken W040 amendment (g).
- `help_rows_are_items_in_f1_style` (`app.rs:12455`), `help_rows_render_in_f1s_two_column_style` (`ui.rs:7288`).

**Live check — feasible and required.** Start the TUI against a refused port (`--base-url http://127.0.0.1:9/v1`): nothing is sent to the model until a prompt is submitted, so tabs, the band, `/new`, `/resume` and the chord all work; only an actual turn would hang in retries. A build alone proves nothing here — half of this item is a rendering/keys claim that only a real pty shows.

---

## 7. Testing

### 7.1 Automated (what each asserts about behaviour, not that it runs)

`crates/wcode-tui/src/app.rs` (reducer):

1. `slash_new_opens_a_conversation_instead_of_quitting` — `/new` yields exactly one `Action::NewConversation(RootSpec::Fresh { label: None })`, `should_quit()` is **false**, and the transcript carries the "opening a conversation" transient. (Today the same submit sets `pending_new` and quits.)
2. `slash_new_and_slash_tab_new_emit_the_same_request` — the two paths are byte-identical `Action`s, so they cannot drift again.
3. `slash_new_is_refused_over_a_socket` — `set_remote(true)` ⇒ no `Action`, no quit, a transient naming the socket.
4. `resume_selection_opens_a_tab_and_keeps_the_process` — selecting a group row emits `RootSpec::Resume { dir }`, closes the overlay, does **not** quit.
5. `the_resume_filter_prefills_and_opens_a_tab` — `/resume <stem>` narrows to one row and Enter opens it.
6. `resume_is_refused_over_a_socket` — no `Action`, no quit.
7. `resume_with_no_sessions_says_so` — kept verbatim.
8. `a_pick_of_an_already_open_session_activates_that_tab` — two conversations, tab 0's root `status.session == "X"`; picking `/s/X` activates index 0 and emits nothing.
9. `a_pick_of_an_already_open_session_activates_that_tab_legacy` — the same with a flat-file path (`/s/X.jsonl`).
10. `ctrl_p_is_the_previous_tab_and_ctrl_n_the_next` — N = 2, both directions, wraps.
11. `cycle_keys_are_inert_at_n1` — no actions, no panics, no transcript write at N = 1.
12. `switching_tabs_writes_no_block` — `transcript().len()` unchanged across a cycle (the gate for §12.3).
13. `the_keymap_names_the_conversation_chord` — a `KEYS` row whose detail mentions tabs (the pixel tests then cover rendering).

`crates/wcode-tui/src/lib.rs`:

14. `new_and_resume_never_end_the_tui` — after `/new` and after a resume pick, `outcome_of(&app) == Outcome::Quit` and `should_quit()` is false (the "one process" claim, pinned where `Outcome` lives). `new_command_maps_to_outcome_new` (`:1571`) is deleted with the variant.

`crates/wcode-cli/src/roots.rs`:

15. `resume_from_a_legacy_flat_jsonl_builds_a_root` — `RootSpec::Resume { dir: <tmp>/<millis>_<id8>.jsonl }` succeeds, the root's `session_path` is that file, `group` is `None`, and no sibling `members/` appears.
16. `a_legacy_file_resumed_twice_is_refused` — the second `start` returns `RootError::AlreadyLive` **before** the factory runs (the `never_called` pattern at `:1835`).
17. `a_resume_of_a_path_that_is_neither` — a nonexistent path ⇒ `BadResume`, message "`… is not a session`".

Seams intentionally *not* unit-tested: the host's `Start → Added` round trip (already covered by W040's `run_host` tests and by the live check).

### 7.2 How a human verifies it

```sh
cargo build --bin wcode
./target/debug/wcode --base-url http://127.0.0.1:9/v1 --model probe
```

1. **`/new` does not destroy.** `/new` → the band appears with two tabs and the view is on the second; tab 1's transcript is still there when you cycle back. Twice more → three tabs, first intact. *(Before: `/new` printed `starting a new session ...` and the process restarted.)*
2. **The chord moves.** `Ctrl-N` / `Ctrl-P` → the `▌` moves and **nothing is written to the transcript**; it wraps at the ends and is inert with one tab. `Shift-Tab` still moves between surfaces; `Alt-1..9` still jumps.
3. **`/resume` opens a tab.** `/resume` → pick any earlier session (`~/.local/share/wcode/sessions`) → a new tab opens with that transcript replayed and the previous tabs are still in the band. A **flat `.jsonl`** row must also open a tab, not refuse.
4. **Self-resume is a switch.** `/resume` → pick the session you are *in* → the band does not grow and there is no "… is already live".
5. **`/help` and `F1` agree.** Both list `Ctrl-N / Ctrl-P` as next/previous tab and `Shift-Tab` as the previous surface.
6. **`/reload` still ends everything** (deliberately, and named as such).

---

## 8. Expected outcome

- `/new` opens a new conversation tab and switches to it; **no re-exec, no quit, no lost tabs** — three `/new`s leave three tabs and the first one's transcript.
- `/resume` and `/resume <filter>` open the picked session as a new conversation tab, group dir or legacy flat file alike; the previous tabs survive.
- Picking a session that is already a tab switches to it and says so; no `AlreadyLive` transient naming a directory.
- `Ctrl-N`/`Ctrl-P`, `Shift-Tab`, `Alt-1..9` and `/tab <n>` each mean exactly one thing, and `F1`/`/help` print those meanings.
- `Outcome` has four variants, all with live producers; `grep -rn "pending_resume\|pending_new\|Outcome::New\|Outcome::Resume" crates/` is empty.
- `docs/tui-design.md` says what the code does about `/resume` and the tab keys; no sentence in it still promises a re-exec.
- `grep -rn "SKETCH" crates/` returns exactly two live pairs, each with a stated retirement.
- `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `python3 scripts/check-book.py` all clean.

---

## 9. References

- `docs/work/W040-multi-root-conversations.md` — §13.2 (locked decisions), §13.4 (the `/new` recommendation this item overrules), §13.5.2 (finding 6, open), §13.6.1 (labels, id burn), §13.2.1 (a)–(k), §14 (succession), §15 (errata).
- `docs/tui-design.md` §4 "Tab band (W040)" (`:257`), §4 **Keys** (`:495`-region), §5 Decisions.
- `docs/decisions/D013-multi-tab-sessions-vscode.md`, `D035`/`D041` (session titles, `/resume` rows), `D027` (label-column/row shape), `D010` (the pinned header the band sits above).
- `docs/session-groups.md`, `docs/session-groups-review.md` — group vs legacy vocabulary.
- `docs/next-steps.md` row 112 (W040's tally) and row 97 (the missing live-TUI driver, which is why §7.2 is hand-driven).

---

## 10. What this brief did NOT verify

1. **No build, test or clippy run.** Every test name above is either grepped (existing) or proposed (new). No test count is asserted.
2. **`build_agent_for` with `session: Some(..)` + `active_group: None`.** Step 4 rests on that combination being well-formed (the `--no-session` path suggests it is). The *call site* was read, not the function body. **Step 4's first gate must confirm it.**
3. **Terminal delivery of candidate chords** under tmux, foot, alacritty, kitty — read from `event.rs`/`terminal.rs`, not observed on a live terminal.
4. **Prompt-history sharing across tabs.** `Options.history` is one path per process; a `/new` root gets no history seeding. Not examined; left as is (a plausible follow-up).
5. **Whether the `Added` round trip feels instant** for a team build (the factory spawns members synchronously). No timing measured; D5 declines to invent a placeholder regardless.
6. **`examples/dump.rs` and the `--socket` client's `Options`** beyond `sessions: Vec::new()`.
7. **W040's own claims** (the id-burn rule, `close_all` on every exit path, `Root::surface_specs`'s minting) were consumed as given; steps 3–5 must keep their tests green.
8. **Premise corrected:** six `SKETCH` pairs, **four** above implemented code; also `SessionItem` is at `app.rs:1484`, not `lib.rs:296`.
9. **A deliberate divergence:** `conversation.rs:151` promises legacy resume "as a one-member group"; D3 resumes the file as itself (no group). Doc sentence fixed in step 4.
10. **The `docs/book` figcaption conventions** — `check-book.py` will tell step 8.

---

## 11. Numbering

`docs/work/` has no `W039` (taken twice, reverted, per W040 §14); W041–W045 are reserved by that table. **W046 is free** and is this item.

---

## 12. Orchestrator's record — D9 resolved: the designer's ruling

`agent:designer` ruled on the one decision §4.0 left open. Recorded verbatim in substance, with its anchors.

### 12.1 The ruling, and why the axes are asymmetric

**Tabs are the unbounded axis with no direct addressing and no click target** (`ui.rs:309` "**Not yet clickable**"), so a cycle is the *only* movement that axis can ever have. **Surfaces are bounded (≤ 9 badged rows) and already directly addressable** — `Alt-1..9` (`app.rs:5080` `Key::Alt(c) => self.focus_digit(c),` → `:3682`), the sidebar click, `/surface` — so a *forward* cycle over them is redundant arithmetic. And exactly one direction surfaces cannot address is **backwards to the unbadged root**: `focus_digit` guards `n >= 1` (`app.rs:3684` `&& n >= 1`) and the root is deliberately unbadged, so `Shift-Tab` is the only key that reaches it.

| axis | chord | note |
|---|---|---|
| next / previous **tab** | **`Ctrl-N` / `Ctrl-P`** | unconditional at every N, wraps, inert at N = 1, silent (the band is the feedback) |
| previous **surface** | **`Shift-Tab`** | unchanged; the only key that reaches the unbadged root backwards |
| direct **surface** jump | `Alt-1..9` | unchanged |
| direct **tab** jump | `/tab <n>` | no chord; the click is the future direct affordance |

Retired: the *forward* surface cycle (redundant with `Alt-1..9`) and `Ctrl-N`'s N-dependence. `KEYS` 35 → 36 rows.

### 12.2 The alternatives, priced

| rejected | why |
|---|---|
| `Alt-←` / `Alt-→` (the universal tab chord) | Costs a new `Key` variant + `translate_key` arms (`event.rs:34` `KeyCode::Left => Key::Left,` has no modifier guard), and the app pushes no keyboard-enhancement flags (`terminal.rs:30`), so on terminals without xterm-keys `Alt-Left` arrives as **`Esc` + `Left`** — it would *cancel the run* (`Key::Esc => self.interrupt(false)`) and move the cursor. |
| `Ctrl-Tab` / `Ctrl-Shift-Tab` (the browser chord) | Not available at all: `event.rs:51` `KeyCode::Tab => Key::Tab,` is reached before any modifier guard (it is not a `Char`), so they arrive as plain `Tab`/`BackTab` — i.e. the completion accept and the key we are keeping. |
| `Alt-h` / `Alt-l` | Zero infra cost, but the same ESC-prefix class, a mnemonic with no established meaning, and `Alt-` + punctuation is already the occupied corner (`Alt-[`/`Alt-]`). |
| `Ctrl-F` / `Ctrl-B` | `Ctrl-B` is the sidebar (`app.rs:5090`). Half a pair. |
| `Ctrl-D/U`, `Ctrl-R/V`, `Ctrl-X/Z`, `Ctrl-S/Q` | No established next/prev meaning; `Ctrl-Z`/`Ctrl-S`/`Ctrl-Q` are terminal-hostile (SUSP/XOFF); `Ctrl-I`/`Ctrl-M`/`Ctrl-H` alias `Tab`/`Enter`/`Backspace`. |
| `Ctrl-N` only, prev later | A one-directional cycle is the bug, not the fix. |

`Ctrl-P` (0x10) is a single control byte, unambiguous in tmux/ssh/linux console, no signal, no flow control, not aliased; one new arm in an existing match, inert today, shadowed by overlay/completion/browse exactly like `Ctrl-N` — so **tab switching stays input-mode-only**, as today.

### 12.3 No legend on the band, and no transient on switch

**No band hint.** The band has exactly one reading (`docs/tui-design.md` §2 "One glyph at three scales, one reading: *you are here*") and its clip rule is structural ("a crowded bar sheds the tabs to its right, and the active tab is drawn first"). A right-aligned legend would be a second, non-tab group on a row whose contract is "everything here is a tab being shed from the right". `F1`/`/help` are the disclosure channel, and the band appears exactly when the chord becomes useful (N = 2).

**Amended `docs/tui-design.md` §4 Tab band clause** (append after "…carry the active tab with no hue (D023)."):

> It is switched with `Ctrl-N` / `Ctrl-P` (next / previous tab, wrapping, inert at N = 1) or `/tab <n>`. The row carries **no legend of its own**: `F1`'s key map and `/help` are the disclosure channel, because the row has exactly one reading (`▌` = you are here) and its shed rule must never have to shed something that is not a tab. A switch is silent in the transcript — the `▌` and the label's weight are the feedback — and the count, when the clip hides it, is `/tab`'s bare form (`tabs: 1 wcode (active) · 2 api`), not a notice per switch.

**No §2 vocabulary row** (no new glyph, no new role) and no new §3 draft.

**Delete the transient on switch** (`app.rs:5761` `self.transient(format!("tab {} of {len}", next + 1));`). It is the only transcript-writing navigation key in the app — `toggle_view`, `toggle_sidebar`, `focus_step`, `bump_measure` all just set `dirty`, and `activate_conversation` already sets `dirty` (`app.rs:3353`), so nothing is lost but the block. Today it drops `tab 2 of 3` **into the tab you just arrived at**, and cycling five tabs leaves five blocks across five transcripts. The rule, already written in the code for `Ctrl-Y` (`app.rs:5653`-"the `Ctrl-Y` keystroke is a keyboard affordance, not a generation of chatter"): **commands ack, keys don't.** `/tab <n>`'s `"tab {k}"` echo stays.

### 12.4 The `KEYS` rows, and the measurement

```rust
    ("Ctrl-N / Ctrl-P", "next / previous tab"),
    ("Shift-Tab", "focus the previous surface"),
    ("Alt-1..9", "focus sidebar row N"),
```

- The old `("Ctrl-N / Shift-Tab", "focus the next / previous surface")` is **replaced**, not appended to: one chord, one axis.
- `Shift-Tab` needs its own row now, so `KEYS` goes 35 → 36.
- `("Alt-1..9", "focus sidebar row N")` stays: after the split it is unambiguous and still true (`canonical_order` is narrowed to the active conversation, `app.rs:3435`).
- Measurements: `ITEM_LABEL_COLS` = 22 (`ui.rs:35`); the detail budget is **39** (`panel_inner_width` `ui.rs:609` = 62 at 80 cols, minus 1 indent + 22). New rows: chord 15/9 ≤ 22, details 20/27 ≤ 39. The F1 body becomes `KEYS.len() + 1` = 37, which is exactly `info_visible_rows` at 64×40 (`app.rs:408`) — **one row fits; a second would clip and redden `ui.rs:4205`**, which is why `Shift-Tab` gets a row and `/tab` gets none.

### 12.5 The designer's two open questions, answered by the orchestrator

1. **Should `Shift-Tab` also switch tabs** (muscle memory)? **No** — one chord, one axis; and it would cost a `KEYS` row the F1 height cliff cannot afford (§12.4). The human may overrule this at sign-off; the cost is stated above.
2. **The `Alt-1..9`-cannot-reach-the-root gap** becomes visible once the forward surface cycle is gone. **Not W046** — it is a separate tiny item (badge the root `0`, or `Alt-0` to focus the root). Recorded here so it is not lost.

---

## 13. First-layer review: BLOCK, and the amendments that clear it

`agent:reviewer`, read-only, gates run on the untouched tree: `cargo build --workspace` ✓, `cargo test --workspace` ✓ (582 in `wcode-tui` alone), `cargo clippy --workspace --all-targets` ✓ — all exit 0, no warnings. **Verdict BLOCK, two blockers + four plan amendments.** This section is AUTHORITATIVE where it disagrees with §3, §4 or §5.

### 13.1 BLOCKER 1 — `repl::new_session_args` is NOT dead

`crates/wcode-cli/src/main.rs:2467` `let mut argv = repl::new_session_args(` lives inside `Ok(wcode_tui::Outcome::Team(name)) => {` (`:2462`) — **the arm this item keeps**, because `RootSpec` has no team field and `/team` must keep re-exec'ing. Deleting `new_session_args` per §4.1/step 3 breaks the build there, and `new_session_args_has_no_session_flags_but_keeps_launch_opts` (`repl.rs:2702`) is green today and must stay. `repl::reload_args` likewise survives (`main.rs:2415`-region, the `Reload` arm).

**Amendment:** keep both. The deletable set is exactly: `Outcome::{New,Resume}`, `outcome_of`'s two arms, `pending_new`/`pending_resume` (fields + accessors), and the four `main.rs` arms (`:1240`, `:1248`, `:2432`, `:2447`).

### 13.2 BLOCKER 2 — the dedupe as drafted cannot fail, so it proves nothing

The reviewer's census of `Status.session`: the startup root holds a **full UUID** (`main.rs:2200` `session: agent.session_id(),` → `agent.rs:591-596`); every runtime root holds `None` (`app.rs:1543`, and no `ConversationEvent` path writes it — the only writer is the *member* arm at `app.rs:1771`). The picker's value is a **path** (group dir or flat file, `session_groups.rs:120-128`). And the identifiers cannot meet: a group root's stem is literally `"root"` (`session_groups.rs:181` `Session::create_named_with_cwd(dir, "root", cwd)`), the group dir is `<millis>_<8hex>` from the clock, and a legacy stem `<millis>_<id8>` is only a *prefix* of that file's header UUID. Tests 8/9 would have had to hand-set `status.session = Some("X")` — a state no production path produces.

**Amendment (the reviewer's preferred option A; cutting the step was considered and refused):** carry the resume identity itself. Add `resume_path: Option<PathBuf>` to `SurfaceInfo` (`crates/wcode-tui/src/lib.rs:253`-region) and `SurfaceSpec`, filled by the host at `Root::surface_specs` (`roots.rs:1168`-region) from `self.group.as_ref().map(|g| g.dir.clone()).or_else(|| self.session_path.clone())` — the group dir for a group, the file for legacy, i.e. exactly what `--resume` would take. Fill the **startup** root in `main.rs` beside `agent.session_id()` (where `session_path` is already in scope). `conversation_of_session` compares `PathBuf == PathBuf`. This also **retires** `Root::session_path`'s `#[allow(dead_code)]` (`roots.rs:194-200` "Read by nobody yet (amendment (k))") instead of adding a new one.

Why not cut it: with the dedupe gone, a pick of your own session answers `RootError::AlreadyLive(dir.display())` → a transient naming a group dir — a *regression* of a pick that works today (it re-execs into it). §4.5's "no `SurfaceInfo`/`SurfaceSpec` field" is therefore corrected to **one field, `resume_path`, and it is TUI-local (not the wire — `PROTOCOL_VERSION` stays 1)**.

### 13.3 Four plan amendments

1. **The four `Ctrl-N` tripwires were missing from §6.** `app.rs:5750-5751` says the N = 1 delegation "is what keeps the four key tripwires green unmodified" — step 6 removes it, so each test that presses `Ctrl-N` to move between surfaces goes RED: `ui.rs:4899`, `ui.rs:5452`, `app.rs:7482`+`:7487`, `app.rs:7506`. Re-point them at **`Shift-Tab`** (a drop-in at two surfaces), **not `Alt-0`** (`focus_digit` has no root case; `alt_n_focuses_the_numbered_sidebar_row`, `app.rs:10614`, pins `Alt-0` as a no-op). §6 now lists them.
2. **"byte-identical at every N" → "at N ≤ 1".** `Key::BackTab => self.cycle_or_focus(-1)` (`app.rs:5077`) is byte-identical to `focus_step(-1)` only at N ≤ 1; at N > 1 it cycles conversations today, which is exactly what is being retired. The N = 1 claim that matters still holds for the right reason: `backtab_moves_focus_backwards_and_wraps` (`app.rs:10633`) builds `two_surfaces()`, i.e. one conversation.
3. **`BadResume` must not swallow the missing-file case.** As §4.2 drafted ("group dir → `open_group`/`load_root`, else → `Session::open`"), a nonexistent path reaches `Session::open` and yields `RootError::Session("session io: …")`, so test 17 fails. The legacy arm must read `else if dir.is_file() { Session::open(&dir) } else { return Err(RootError::BadResume(dir.clone())) }`.
4. **The legacy label seed keeps `.jsonl`.** `roots.rs:900-905` labels a `Resume` from `dir.file_name()`, so a flat file would render the tab `1768….jsonl`. Use `file_stem` for the legacy branch (or `GroupEntry::id`'s rule, `session_groups.rs:131-140` `name.strip_suffix(".jsonl").unwrap_or(name)`).

### 13.4 Small amendments, and the docs half (already landed)

1. `crates/wcode-protocol/src/conversation.rs` has **two** sites, not one: `:150-152` ("`Resume` takes a group dir, not a file … one-member group") **and** `:176-181` (the `Resume` variant's doc, `<session_dir>/<millis>_<id8>`). Both become "a group dir, or a legacy flat `.jsonl` resolved by `group_dir_of`".
2. `conversations.rs:499-504` still names the dying field (`pending_resume`/`pending_team`/`pending_reload`/`pending_new`) — drop `pending_new`.
3. `outcome_of`'s doc (`lib.rs:365-367` "wins over a pending `/reload` or `/resume`") → `/reload` or `/team`.
4. `roots.rs:517-521` `insert`'s guard is narrower than `start`'s (a legacy root has no `group`, so only `start`'s pre-build check `:449-453` catches a legacy duplicate) — sufficient for test 16; say so in the code comment or widen `insert`.
5. `/tab <n>` keeps its transient (`app.rs:5466` `self.transient(format!("tab {k}"));`) while the chord loses its. Coherent ("commands answer, keys do not"); the design doc says "the *keys* are silent", which is what `switching_tabs_writes_no_block` asserts.
6. **`Ctrl-P` verified free and reachable:** no `Key::Ctrl('p')` anywhere in `crates/wcode-tui/src/`, and `event.rs:33` `KeyCode::Char(c) if ctrl => Key::Ctrl(c),` delivers it. Overlay/completion/browse shadow it exactly like `Ctrl-N`.
7. **Scope, made explicit (§2.1 gains a line):** deleting the `/clear` alias (D2) is user-visible and appears only in D2 — it is in scope because a `/clear` that opens a tab lies, and its tripwire `clear_is_an_alias_for_new` (`app.rs:10224`) plus `app.rs:12491` `assert_eq!(label("new"), "/new [clear]", "aliases, no args")` must change with it. The `SKETCH` deletion (item 7) is unrelated housekeeping kept deliberately as its own commit.
8. **§2.1 item 8's doc inventory is stale because the designer already landed it:** `docs/tui-design.md` (status line, §4 tab-band clause, Keys, `/resume`, §5 decision), `docs/book/04-the-tui.html` (`:138` figcaption repinned to `app.rs:1321`, `:142-144` excerpt, §4.4 level, §4.7 two re-execs), `docs/book/14` and `15`, `README.md` (`:412`, `:441-448`, `:453-457`, `:461-462`), with `python3 scripts/check-book.py` → `check-book: ok — 19 pages, 286 KB`. **Still open for step 8:** `README.md:412`/`:449`'s surface-switch sentence, the W040 §13.4 errata row + §15 row, `docs/next-steps.md` row 112's tally — the orchestrator's to file.
9. **Docs currently lead the code** (`app.rs:1332` still says `Ctrl-N / Shift-Tab`, `app.rs:832` still has the `/clear` alias, `app.rs:1369` and `:12491` still say `/new [clear]`) — so step 6's code commit must land the `KEYS` row with the §4 text already in place, or the tree momentarily contradicts itself. That is expected and closes here, not a review finding.

### 13.5 What the review confirmed (so the developer does not re-litigate)

- `activate_conversation` sets `dirty` (`app.rs:3348`), so deleting the switch transient loses only the block — and the block lands in the transcript of the tab you *arrive at* (`transient` pushes onto `self.focus()` **after** the move), i.e. it pollutes the transcript the user is about to read.
- `KEYS` is exactly 35 entries today (`app.rs:1322-1364`); 36 after, so `KEYS.len() + 1` = 37 = `info_visible_rows` at h = 40 — **exactly fits, zero margin**, and `f1_renders_the_help_overlay_with_the_keymap` (`ui.rs:4197-4211`, 64×40) is the tripwire. **No other `KEYS` row may enter this item.**
- `Outcome::{New,Resume}` are genuinely producerless after the swap: all nine `should_quit = true` setters were censused and only `lib.rs:375`/`:378` construct the variants.
- The legacy-resume shape is a lift of live code, not an untested corner: `main.rs:1376` `None => match Session::open(&p) {` leaves `active_group: None`, `resuming_group: false`, and `main.rs:1419-1424` builds `SessionSetup { …, session, context }` from it. The "one-member group" sentence is a doc fix, not a contract.
- Editing is content-addressed; nothing in this list depends on a line number staying still — every item carries its quoted snippet.

### 13.6 What the review did NOT check

No live terminal (`Ctrl-P` delivery under foot/tmux/kitty is read from `event.rs`/`terminal.rs`), `session_groups::list_groups`' ordering, W040 §13/§15 in full (taken on trust), and whether `/new`'s `Fresh` should inherit the *focused tab's* cwd (it does not — `Fresh` uses `inputs.dir`).

---

---

## 14. Second-layer review, and what shipped

**Verdict: BLOCK on three cheap items, then cleared.** The feature itself was accepted as correct on the first pass: both first-layer blockers landed (§13.1's `repl::new_session_args` survives and is still called by the `Outcome::Team` arm; §13.2's `resume_path` is filled by `Root::surface_specs`, so **one** fill point reaches both the startup root and every runtime root), gates green (`cargo build`/`cargo test`/`cargo clippy` — 612 + 247 + 21 + 12 tests, zero warnings), `check-book: ok`, scope clean, `SKETCH` at exactly the two labelled pairs, `/reload` and `/team` still re-exec, `repl.rs` untouched.

The three blockers, and why each was real rather than cosmetic:

1. **`cycle_keys_are_inert_at_n1` could not fail.** Its fixture is one conversation, so restoring the retired `Ctrl-N => focus_step(1)` fallback still left the transcript unchanged and `active_index == 0` — all three assertions passed. The test existed to prevent exactly that regression, so it gained `assert_eq!(app.focus(), 0, "no surface fallback at N = 1")`.
2. **Nothing pinned `Root::surface_specs`' `resume_path`.** Delete that one production line and all 612 tests stayed green while a self-`/resume` silently reverted to the `AlreadyLive` transient — the amendment was landed but unprovable. The roots test that already holds a live root now asserts the spec's `resume_path`, legacy shape and group shape.
3. **The diff left false statements behind** — orphaned `pending_*` doc comments above unrelated functions, a test doc still describing the deleted `cycle_or_focus` and its N-dependent rule, `Outcome`'s doc still naming a pending `/resume`, `roots.rs`'s duplicated `session_path` sentence, "three aliased commands" when deleting `/clear` left two, and — the sharpest — `docs/book/04-the-tui.html`'s figcaption pinned to `app.rs:1321` for `KEYS`, which this item's own `SKETCH` deletion moved to `:1173`. A comments-only commit fixed all of it.

**Where the shipped tree differs from this brief, deliberately:**

- **§14.1 (recorded, not fixed):** the dedupe is path *equality*, and `--resume` does not canonicalize (`repl.rs:313`-region `if p.exists() { p } else { session_dir().join(arg) }`), so a boot with a relative `--resume ./x.jsonl` stores a relative `session_path` while the picker yields absolute paths — no match, and the self-resume falls back to today's `AlreadyLive` transient. Same worst case as before W046; filed for a follow-up, not fixed here.
- **§7.1 test 14 (`new_and_resume_never_end_the_tui`) was not written** — with the variants deleted the assertion became structural, and the reviewer agreed it is not worth a commit. Recorded so the absence is deliberate.
- **The `/tab` band is still not clickable**, and the two surviving `SKETCH` blocks now name their retirement: the click, and `serve()` adopting root 0.
