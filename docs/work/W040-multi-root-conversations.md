# W040 — many conversations in one process: a multi-root core, TUI tabs now, the browser hub second — brief

- **Status:** **brief + first-layer sketch, in review.** Sequencing locked by §13: the transport question (`F1`) is **approved as `axum` but deferred to W041**; this item is the multi-root core plus the **TUI tab** proof (W040), because the design must be proven on a surface that already renders N sessions before any HTTP exists. Human sign-off on the phasing: **yes** ("sketch the tab feature for TUI, so we can know if it is really feasible for web").
- **Work item:** W040 (this file) · tracker row **112** (`docs/next-steps.md`; rows 1–111 present, 112 free).
- **Decisions:** proposes **D045** — *the control plane is a fourth client that speaks **HTTP + SSE** beside the NDJSON socket, is **hosted by the `serve` process**, and **renders on the server***. (D044 is the highest on disk.)
- **Reading edition:** none. W032/W031 have generated `.html` editions; producing one is the orchestrator's call, not a read-only worker's.
- **Supersedes:** nothing. It **splits** the ask: W040 is phase 0 of five (§2.2), and it settles the W010 collision (§4.7) without reopening W010.
- **Brief author:** `agent:brainstormer` (read-only; `bash` mutations refused). Every anchor below was re-verified in this session against the tree at `e6a2d82`.
- **Date:** not recorded — a read-only worker has no shell clock and this file is not mine to stamp.

---

## 1. The ask

> "What we want here is an improvement for wcode in serve mode, where it will be a control plan for user to work with multiple session including: file folder tree, file review including syntax highlighting, markdown render and html render for html file; multi session control let's them work parrally; team timeline view, where we can see multiple agent in the team working in the timeline basis; team orchestration work visuallization; Change Walkthrough feature; tasks/workflow dashboard for control, submit; setting"

Eight surfaces, one host: a **browser control plane over `wcode serve`**. Read plainly, the ask is *one* screen with a left rail (servers → sessions), a center (the focused session's transcript *or* a file *or* a diff), and switchable panels (timeline, task graph, settings). That reading decides half the design: **it is one application with views, not eight pages.**

Three facts re-shape it before any code is chosen, and each is verified:

1. **The server is already multi-session and multi-client.** `crates/wcode-protocol/src/server.rs:93` `loop {` → `:94` `let (stream, _addr) = listener.accept().await?;` → `:95` `tokio::spawn(serve_stream(`. Each connection fans every served session from a `broadcast` (`server.rs:408` `fn fan(`), is seeded with an unsolicited roster (`:174` `let _ = out.send(Frame {` … `:180` `body: AgentEvent::Sessions {`), and a watch task re-pushes it as the roster grows (`:196` `tokio::spawn(async move {` … `:214` `let _ = out.send(Frame {`). Lag is dropped, not fatal: `:428` `Err(broadcast::error::RecvError::Lagged(_)) => continue,` (`EVENT_BUFFER = 1024`, `crates/wcode-harness/src/actor.rs:137`). `serve_stdio` is the one-client exception (`:122` `pub async fn serve_stdio(` with the doc at `:117`–`:121`). **The server side of "multi session" exists; what is missing is a browser client and six verbs.**
2. **Nothing on the wire is a file, a task, an ownership edge, or a timestamp.** `Request` is 18 variants (`crates/wcode-harness/src/protocol.rs:232` `pub enum Request {`); `AgentEvent` is 27 (`crates/wcode-harness/src/event.rs:61` `pub enum AgentEvent {`); `Frame` (`protocol.rs:396`) carries `v,id,reply_to,session,sender` and no time; `ToolExecutionEnd.duration_ms` (`event.rs:101`) is the only time value anywhere. The **only** ownership record is a private map (`crates/wcode-protocol/src/registry.rs:237` `pub fn set_owner(&self, worker: SessionId, owner: SessionId) {` into `:65` `owners: Mutex<HashMap<SessionId, SessionId>>,`) with **no getter**; `SessionInfo` (`crates/wcode-harness/src/protocol.rs:175`) carries `model/effort/context_window/state` and nothing else.
3. **There is no HTTP anywhere.** Zero `hyper`/`axum`/`warp`/`tiny_http`/`tungstenite` in any manifest; `wcode-protocol`'s deps are exactly harness+tokio+serde+serde_json. **So `F1` (§4.1) is the item's central question** and it needs the human, because `AGENTS.md` says "Do not add dependencies unless explicitly requested."

And one correction to the recon I was handed: **W010 is not a draft awaiting sign-off — it shipped.** `docs/work/W010-multi-tab-multi-session-vscode.md:3` `**Status:** accepted (human sign-off 2026-10-10) — decisions recorded in [D013]…`, its plan carries `**☑ DONE**` markers (`:125`, `:129`), and `editors/vscode/src/manager.ts:1` ``The `Manager` — the multi-tab composition root (D013/W010 P2).`` exists with `Map<tabId, SessionTab>` and `nextActiveTab`. That matters, and §4.7 says exactly what it does and does not constrain.

---

## 2. Scope / Non-scope

### 2.1 Where each of the eight asks lands

| # | the ask | where it lands | what it costs |
|---|---|---|---|
| 1 | file tree, file review, syntax highlight, markdown, html render | **W042** (files) | needs its own reader (the `read` tool's caps are *model*-facing) + 3 render paths |
| 2 | multi-session control, parallel work | **W040** (the roster + N live surfaces) + W045 (a multi-*server* rail) | roster exists; "parallel" needs a precise definition (§4.6) |
| 3 | team timeline | **W044** | needs an ordering stamp (§F3) and a live ring; **not** reconstructible for past runs (§4.5) |
| 4 | team orchestration visualization | **W044** | needs the ownership edge on the roster (+1 serde-default field) |
| 5 | change walkthrough | **W043** | `ToolExecutionEnd.diff`/`.path` already exist (`event.rs:82`–`:99`); the walkthrough is ordering + UI |
| 6 | tasks/workflow dashboard, control, submit | **W045** | needs the `TaskRow` seam (TUI precedent) + a read-only rule (§F4) |
| 7 | settings | **W045** | every row is an existing `Request`; **no config writer** (D036 stands) |
| — | the transport, the shell, the live transcript | **W040 (this item)** | the whole of §5 |

### 2.2 In scope — the browser front-end. **DEFERRED TO W041; superseded for W040 by §13.2.**

> **Read §13.2 instead.** Items 1–7, 10 and 12 below describe the **browser front-end**, which §14 assigns to **W041**; they are kept verbatim because W041 will need them, not because they are this item's scope. **Item 8 (`SessionInfo.owner` + `Registry::owner_of`) is withdrawn** — §13.3 refuses it. W040's in-scope list is §13.2's ten decisions plus the sketch's six deliverables, and its expected outcome is §13.6's.

1. **A new crate `crates/wcode-serve`** (§4.2), published fifth, whose library is a function the CLI calls: no binary of its own.
2. **A hand-written HTTP/1.1 subset + SSE server on tokio** (§F1): one request per connection for POST, keep-alive for GET, header/line caps, no TLS, no HTTP/2, no chunked *request* bodies.
3. **`wcode serve --web [addr]`** — default `127.0.0.1:7878`, **off unless asked**; refused with `--stdio` (§4.4).
4. **A `Store`**: the served roster (root + live team), a bounded event ring, a monotone `seq`, and the server clock.
5. **The HTTP surface** (§4.3): `GET /`, `/assets/*`, `/api/sessions`, `/api/events` (SSE), `GET /api/transcript/{id}`, `POST /api/session/{id}/request`.
6. **One served application** (`assets/app.html|app.js|app.css`, vanilla, embedded by `include_str!`): a session rail with live `[state]`, a live transcript, an input box that POSTs to the **focused session** (the address is in the URL, so the TUI's `Submit` vs `SubmitTo` trap cannot occur by construction).
7. **Server-side rendering** of the transcript: markdown via `pulldown-cmark` → HTML, fenced code via `syntect::html`, prose escaping through the crate's own escaper, never `format!`.
8. **The ownership edge on the roster**: `Registry::owner_of` + `SessionInfo.owner` (`#[serde(default)]`, additive, no version bump) — the one protocol change, and it is W042's prerequisite paid early because the rail needs the team tree to be a tree.
9. **The `TaskRow` feed seam** — `wcode-serve::TaskRow` + `Option<watch::Receiver<Vec<TaskRow>>>`, wired exactly like `wcode_tui::TaskItem` (`crates/wcode-tui/src/lib.rs:294` `pub tasks: Vec<TaskItem>,` and `:379` `new_tasks: Option<mpsc::UnboundedReceiver<Vec<TaskItem>>>,`), so the dashboard is a view in W043 rather than a re-plumb.
10. **The security floor** (§4.8): loopback bind, `Origin`/`Host` checks, `Content-Type: application/json` requirement, path-traversal guard, request-size caps, `CSP` on every response.
11. **The visual spec** `docs/design/web-control-plane.md` (§F6) + the two skill/agent-line amendments that currently misname `index.html` as "the wcode web page" (§4.9).
12. **The release plumbing**: `[workspace] members`, `publish.yml`'s two hardcoded crate lists, `DEPENDENCY_GRAPH.md`, `docs/book/14-protocol-and-clients.html`, `README.md`, the tracker row, D045.

### 2.3 Non-scope (each names something a reader would reasonably assume is in scope)

- **Any new runtime dependency except the one asked for (§9 Q1).** `AGENTS.md`: "Do not add dependencies unless explicitly requested". No `axum`/`hyper`/`tower`/`tungstenite`; no npm/esbuild/TypeScript toolchain in the control plane's build; the client is hand-written JS embedded in the binary. The markdown `html` feature (+`pulldown-cmark-escape`) is the single exception and it is *asked*, not smuggled.
- **A client-side reducer.** The VS Code webview has one (`editors/vscode/src/reducer.ts:1` "The view-model reducer — pure") and **W040 does not reuse or fork it**: it is TypeScript (needs a build step) and it is the extension's. §F5 records why server rendering wins here and what it costs.
- **Multiple *roots* in one process.** A second independent session stays a second `serve` process. That is W010's Option-B rejection (`docs/work/W010-multi-tab-multi-session-vscode.md` §2.2 "**Option B — one server hosting multiple root sessions** … **Not built**") and this item does **not** revisit it: the rail lists servers, not roots-in-one-server (§4.6).
- **A durable timeline.** `SessionEntry` has no timestamp (`crates/wcode-harness/src/session.rs:13` `pub enum SessionEntry {` … `:20` `Message {` … no `at`), `plan.ndjson` ops have none (`crates/wcode-cli/src/tasks.rs:232` `/// One journaled line per PUBLIC mutation (§7). Tagged like `SessionEntry`.`), and `AgentMessage` has none. A timestamped session entry is a **kernel** change with a resume/compaction blast radius — its own item (W042b), named in §10.
- **`Frame.ts`, or any `PROTOCOL_VERSION` bump.** §F3.
- **Any `config.toml` writer.** D036 (`docs/decisions/D036-tui-settings-panel.md`) forecloses it for the TUI; the web settings screen inherits the refusal verbatim, and `config.rs` still carries no `Serialize` on `FileConfig`.
- **Auth beyond loopback.** No passwords, no tokens by default (§4.8), no TLS, no `0.0.0.0` bind, no remote access. A LAN/remote story is a separate item with a separate threat model.
- **A framework, a bundler, a CSS preprocessor, a component library, icon fonts, or any external request.** `style.css:1`–`:2` records the house rule for the existing page — "Zero external requests: system font stack only" — and the control plane keeps it.
- **Restyling the TUI or the extension.** `docs/tui-design.md` is the TUI's authority (`.wcode/agents/designer.md:11` `- `docs/tui-design.md` is the authority. A genuine redesign amends that spec`) and nothing here touches `crates/wcode-tui` or `editors/vscode/src`.
- **Editing `index.html`/`style.css`.** They are the retired, unserved marketing page — `docs/next-steps.md:117` "the retired landing page (`index.html` + `style.css` at the repo root) stays in place, unserved, as a rollback", and `.github/workflows/pages.yml:2` now publishes `docs/book`. They stay byte-identical; the control plane **copies the token vocabulary, not the file** (§F6).
- **A `wcode web` binary or a separate daemon.** `wcode-serve` is a library; the process is `serve`.
- **Rewriting the VS Code `protocol.ts` to full parity.** It is *already* three variants behind (`set_title`, `usage`, `session_title` are absent; its own doc says "24 tags" against the kernel's 27). W040 adds only `owner?: string` to `SessionInfo` and files the parity gap as a tracker row (§9 Q4).

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|---|---|---|---|---|
| **E1** | the HTTP/1.1 + SSE server (§F1) | **high** | **high** | **high** | It is the whole item's foundation and the only code here with a *correctness* failure mode that is also a *security* one: request-line smuggling, an unbounded header, a missing `Content-Length` read cap, a response split by a `\n` in a header value. |
| **E2** | the crate + the `Options` seam + the `serve` wiring | high | low | low | Nine fields, every one a type `wcode-protocol` already names (`Roster` `:37`, `Root` `:41`), plus `TaskRow` mirroring `wcode_tui::TaskItem` — a copy of a seam that already works twice. |
| **E3** | the served shell (rail + transcript + input) | **high** | medium | medium | The demo-able half and the reason the ask exists; the trap is the live block (`MessageUpdate` carries a **whole** `AgentMessage`, `editors/vscode/src/reducer.ts:16`–`:18` "it REPLACES the live block's content. Never diff-append"), which a server renderer must implement as *replace the region*, not *append*. |
| **E4** | server-side rendering (markdown + syntect + escaping) | **high** | medium | **medium** | `syntect`'s `html` feature is **already on** (verified: `default-fancy` includes `html`, syntect-5.3.0 `Cargo.toml`), so classed HTML is free; the risk is escaping model-controlled text into the page's own HTML — an injection into the surface that holds the verbs. |
| **E5** | the roster/owner/task seams (+1 protocol field) | medium | low | **medium** | `owner_of` is one getter and `SessionInfo.owner` one serde-defaulted field (`protocol.rs:194` is the precedent), but it edits a **published** crate's public type and the TUI/extension both deserialize it. |
| **E6** | the security floor | **high** | medium | **high** | Everything the plane can do (cancel, submit, later task control) sits behind a port; the failure is silent and remote-shaped (a web page the human visits, reaching `127.0.0.1:7878`). |
| **E7** | the fifth crate's release plumbing | medium | low | medium | `publish.yml:83` `crates=(wcode-harness wcode-protocol wcode-tui wcode-cli)` and `:59`–`:60`'s version-consistency loop both hardcode four; a missed edit ships a crate that cannot be resolved from crates.io. |
| **E8** | the spec + docs + D045 | **high** | low | low | This is the item that finally says what `index.html` is; leaving it unsaid leaves the `designer` agent pointed at a rollback artifact (`.wcode/agents/designer.md:8`–`:9` "The **web page** — `index.html` + `style.css`"). |
| **F1** | *fork:* hand-rolled HTTP vs a dependency vs no browser (§4.1) | — | — | — | Not a workstream, the **gate** on the item: it decides E1's size (≈500 lines vs ≈50) and the repo's dependency doctrine. |
| **F3** | *fork:* the timeline's clock (§4.5) | — | — | — | Decides whether W042 needs a kernel change (a timestamped `SessionEntry`) or a 40-line envelope field; picking wrong makes the timeline a rewrite. |

Reading the three together: **E2, E5 and E8 are high-value/low-complexity — land them first to prove the shape.** E1 is the only step where the wrong *design* (not the wrong line) is expensive, so it lands behind its own gate and its own 8-case reject table. E3/E4 are the demo; if the human wants an early look, stop after step 4 and do not build step 5.

Sizing (revised by §13, then by the first-layer sketch): **W040 is ~3,500–4,500 lines** — the multi-root core (the sketch's `roots.rs` alone is ~1,100 lines of contract) plus the TUI tab container, band, commands and keys; ~40 new tests; 8–10 commits; **zero new dependencies**. The browser front-end is W041 and is small *given* W040. The four surfaces after it are W042–W045 (§2.1), each 250–700 lines. Four things the sketch exposed drove the increase over the original ~2,000–2,600: the crate-boundary move (finding 6), `Result`-ifying a build path that still calls `exit(2)` at seven sites, root-scoped surface ids, and the `Background` actor-lifecycle fix.

---

## 4. Interface & structure

### 4.1 F1 — the transport: recommendation, and the exact dependency list it implies

**Recommendation: (a) a hand-written HTTP/1.1 subset + SSE on tokio, in the new crate, with zero new runtime dependencies.**

| option | verdict | why |
|---|---|---|
| **(a) hand-rolled HTTP/1.1 + SSE** | **chosen** | The protocol we need is tiny and we control both ends: `GET` with a path+query, `POST` with `Content-Length` + one JSON body, and `text/event-stream` for the outbound side. SSE needs **no handshake crypto** (that is the whole point of it over WebSocket): `data: {json}\n\n` per event, `Last-Event-ID` on reconnect for free. Cost: ~500–650 lines in one file with a table-driven parser test — the size of `crates/wcode-protocol/src/server.rs` (665 lines), which is the same kind of code this repo already wrote and tested by hand. |
| **(b) axum / hyper / tungstenite** | rejected | `hyper`+`http`+`tower`+`axum` is ~20 transitive crates to serve one loopback listener, against `AGENTS.md`'s dependency rule, and it buys nothing we need (no TLS, no HTTP/2, no routing table, no extractors). Recorded, not hidden: if the human wants `axum`, the answer is "yes, and then E1 drops from ~600 lines to ~120" — the *design* in §4.3 is unchanged. |
| **(c) no browser surface** | rejected | It fails the ask outright. Named only because it is the honest baseline: `docs/book` already renders the docs statically, and a *generated* HTML run report (`wcode --status --html`) would cover 2 of the 8 asks for ~150 lines. If the human's real need is "a report after the fact", (c) is 5% of the cost — so the sign-off on (a) is really a sign-off on "live, not after the fact". |
| **WebSocket** | rejected inside (a) | Its handshake is `SHA-1(key + GUID)` + base64 + frame masking: hand-written crypto-adjacent code for a full-duplex stream we do not need. Our traffic is **many events out, few requests in**, which is SSE + POST exactly. If we ever need client→server streaming (it is not in any of the eight asks), that is the moment to reconsider. |

The dependency list (a) implies, in full:

- **Runtime:** none new. `tokio` (full), `serde`, `serde_json`, `thiserror` — all already workspace deps.
- **Rendering:** `syntect` (**already a dep**, `default-fancy`, whose feature set includes `html` — verified in the registry copy) and `pulldown-cmark` (**already a dep** of `wcode-tui`, `Cargo.toml:23`, but with `default-features = false`, so its `html` module is **not** compiled in today: `pulldown-cmark 0.13.5` `[features] default = ["getopts", "html"]`, `html = ["pulldown-cmark-escape"]`). Enabling `html` in the **new** crate's manifest adds exactly **one** transitive crate, `pulldown-cmark-escape` (absent from `Cargo.lock` today), same version family, by the same authors.
- **Tree walking:** `ignore = "0.4"` (a `wcode-cli` dep) — moved into `[workspace.dependencies]`, **zero** new third-party code; it gives the same `.gitignore` awareness the `find`/`grep` tools have (`crates/wcode-cli/Cargo.toml` `ignore = "0.4"`).

**So the sign-off on F1 is one sentence: "add `pulldown-cmark-escape` (transitive) and move `ignore` up to the workspace."** Everything else is code we write.

### 4.2 F2 — where it lives: a new crate `wcode-serve`

**Decision: a fifth crate, `crates/wcode-serve`, published between `wcode-protocol` and `wcode-cli`.**

| option | verdict | why |
|---|---|---|
| **(a) `crates/wcode-serve`** | **chosen** | Deps: `wcode-harness`, `wcode-protocol`, tokio, serde, serde_json, thiserror, syntect, pulldown-cmark(`html`), ignore. It is testable without a binary (`cargo test -p wcode-serve`), its `Options` seam is injected by the CLI exactly as `wcode_tui::Options` is, and its rendering churn never version-bumps the transport. |
| (b) extend `wcode-protocol` | rejected | Tempting — HTTP *is* a transport and the crate already hosts two. But it must then carry `syntect`+`pulldown-cmark` (rendering, not transport), and every `wcode-tui` build pays for a browser's HTML renderer. Keep the transport crate at four deps. |
| (c) a module in `wcode-cli` (`src/web/`) | rejected | `AGENTS.md`: "the CLI is a thin client". A 2,000-line surface inside it is not thin, and every test would need the binary. |

Publish order becomes `harness → protocol → {tui, serve} → cli`. Two files hardcode the old list and must change in the same commit: `.github/workflows/publish.yml:83` (`crates=(…)`) and `:59`–`:60` (the per-crate version loop), plus `docs/DEPENDENCY_GRAPH.md:5`–`:9`'s layered diagram. `Formula/wcode.rb` needs **no** change (it installs the binary; `cargo install wcode-cli` resolves `wcode-serve` from crates.io) — verified by reading its generator (`scripts/gen-homebrew-formula.sh`) is invoked from the release workflow, not from the formula's content.

### 4.3 The HTTP surface

```
GET  /                            → the app shell (embedded HTML, ETag'd)
GET  /assets/{app.js,app.css,syntax.css} → embedded, immutable, ETag'd
GET  /api/sessions                → {"ts":…,"sessions":[SessionInfo + owner],"tasks":[TaskRow]}
GET  /api/events?since=<seq>      → text/event-stream; each frame
                                    {"seq":n,"ts":<ms>,"session":"…","event":<AgentEvent>}
                                    (also honours the standard `Last-Event-ID` header)
GET  /api/transcript/{id}         → {"messages":[…]}   (a `Request::GetHistory` on the target)
POST /api/session/{id}/request    → body is a `Request` VERBATIM, reply is an `AgentEvent`
                                    VERBATIM; requires Content-Type: application/json
```

Two deliberate properties:

- **The body types are the existing ones.** `Request` and `AgentEvent` are already `Serialize + Deserialize` with a `#[serde(tag = "type")]` shape (`protocol.rs:230`–`:232`; `event.rs:59`–`:61`), already round-trip-tested (`protocol.rs:453` `fn request_roundtrip_all_variants()`), and already mirrored in TS. Reusing them verbatim means **one schema for three transports** and a browser that speaks the same verbs the TUI does — and it makes `wcode-serve` a *client*, not a second protocol.
- **The session is in the path.** `POST /api/session/{id}/request` cannot mis-target, which retires the TUI's `Submit` vs `SubmitTo` class of bug (tracker row 88: "`Action::SubmitTo`, so it starts on the queue's OWN surface, not the focused one — `Submit`'s contract is the trap") **by construction** rather than by a rule.

### 4.4 The flag and the wiring

`wcode serve --web [addr]` — `--web` (no value) means `127.0.0.1:7878`; `--web 0.0.0.0:…` is refused (§4.8). **Off unless asked**: nothing opens a port uninvited. Refused with `--stdio` (`main.rs:1893` `if args.stdio {`), because a stdio child's lifetime belongs to its spawning client and a second entry point under it is a surprise; refused with `--task` for the same reason `serve` already is (`main.rs:1446` `if args.serve && args.task.is_some() {`).

The wiring point is `serve()`, after the roster is built and before the transport branches:

```rust
// crates/wcode-cli/src/main.rs — inside `async fn serve(...)`
//   after :1858-1861 (the roster) and :1888 (the `serving …` line)
//   before :1893 (`if args.stdio {`) and :1903 (`#[cfg(unix)]`)
if let Some(addr) = web_addr {
    wcode_serve::spawn(wcode_serve::Options {
        addr,
        root: std::env::current_dir()?,          // the plane's file root
        registry: registry.clone(),              // already built at :1836
        roster: roster.clone(),                  // already built at :1858
        root_session: (session_id.clone(), handle.clone()),
        tasks: task_feed(orchestrator.as_ref()), // new, copies main.rs:2125-2148
        label: /* the cwd basename, as the TUI seeds it (main.rs:1096-1099) */,
    })?;
    eprintln!("control plane on http://{addr}/");
}
```

Every value in that call already exists at that point in `serve()`; the only new thing is `task_feed`, and it is a **transcription** of the TUI's forwarder (`main.rs:2125`–`:2148` `let new_tasks = rt.orchestrator.as_ref().map(|o| { … o.tasks().subscribe() … map(task_item) … })`) with `TaskRow` for `TaskItem`.

`Ephemeral` note for the human's flow: printing the URL is the whole discovery story. No token to copy (§4.8).

### 4.5 F3 — the timeline's clock: the control plane stamps, `Frame` does not change

**Decision: the `ts` lives in the control-plane envelope (§4.3), stamped once at fan-out from the process's own clock. `Frame` gains nothing; `PROTOCOL_VERSION` stays 1 (`protocol.rs:34` `pub const PROTOCOL_VERSION: u32 = 1;`).**

The argument is not "cheaper" — it is that the socket wire's *two* existing consumers do not want it, and one says so in writing: `docs/tui-design.md:733` "**Timestamps** on turns: lean no." Adding `Frame.ts` would be an additive field (the `MemberState` precedent, `protocol.rs:194`) that the TUI, the extension, and the `Backend` seam all deserialize and none reads. Meanwhile the property the timeline actually needs — **one clock for all sessions** — is free in the process that serves them: `wcode-serve` reads each session's broadcast (`server.rs:408` `fn fan(` is the same shape) and stamps on read.

What that buys and what it does not: the plane can order N sessions' events onto one axis live, and can place a tool's *bar* backwards from `duration_ms` (`event.rs:101`). It **cannot** reconstruct yesterday's timeline — nothing on disk has a time (§2.3). The plane's ring is bounded (2,000 frames per server, ~a few MB) and the UI says "live" rather than pretending otherwise.

### 4.6 What "parallel" means, precisely

Three different things are called "multi-session" in this repo and only two are built:

| meaning | today | W040 |
|---|---|---|
| **many clients, one server** | built (`server.rs:94` spawns per connection) | the plane is client #N |
| **many sessions, one server** = one root + its live team | built (roster, `server.rs:196`; the TUI already draws a surface each, `main.rs:1124`–`:1150`) | **yes** — the rail lists root + team, all live at once |
| **many independent roots, one process** | **not built** (W010 Option B, `docs/work/W010-…md` §2.2) | **no** — a second root is a second `serve` process |

And the multi-server rail is where "parallel" is honest: the spread lives in the *supervisor*, not the server. Today two `serve`s collide on one default path (`main.rs:2461` `fn default_socket_path()` → `<config_dir>/wcode.sock` and `main.rs:2464` `std::env::temp_dir().join("wcode.sock")`), so a rail is "scan `<config_dir>/*.sock`, one entry each, N SSE streams". That is **W043**, and it needs no server change and no W010 change.

So W040's parallel story, stated for the human: **your team runs concurrently and you can watch and drive every member; two unrelated projects are two ports, and the rail for them is the next item.** No new spawn semantics, no new session model.

### 4.7 F5 — the W010 collision, settled without reopening it

W010 shipped and chose **N `serve --stdio` children, one session per editor tab**. Its Option B ("one server hosting multiple root sessions") is rejected there and **stays rejected here** (§4.6 row 3). Consequences, stated so nobody re-litigates it:

- **W010 needs no change.** Its stdio argument ("there is no listener, so no second connection is even expressible") remains true of stdio; the plane adds a *listener* to the *socket* transport, which W010 never touched. Its `--socket` non-scope ("The socket path stays for a TUI+editor-share-one-session case, untouched here") is also untouched: the plane is another socket-transport-adjacent client, not a new session owner.
- **What W010's decision forbids here** is exactly one thing: the plane must not become "the one daemon that hosts every root". If a future item wants that, it reopens D013 first.
- **What the plane gives W010 free, later**: the ownership edge (§4.8) and the timeline are server-side, so the extension's team caption could read the same data. Not in scope; named so it is not rediscovered.

### 4.8 The security floor (one table, all of it mandatory in step 6)

| risk | mitigation | where |
|---|---|---|
| a **web page** the human visits reaches the port | `Origin` when present must be absent-or-`http://<our-host>`; `Host` must be one of `127.0.0.1:<port>`/`localhost:<port>`; refuse otherwise with 403 | router, before any handler |
| CSRF via a form POST | require `Content-Type: application/json`; a cross-origin form cannot set it | POST handler |
| DNS rebinding | the `Host` check above + bind `127.0.0.1` only; **refuse a non-loopback `--web`** with a clear error | `Options` validation at spawn |
| path traversal (`../../.ssh/id_rsa`) | canonicalize, then require the result to be under `Options.root`; reject `..` in the *served* path outright | `files.rs` |
| runaway request | caps: 8 KiB request line, 32 headers, 64 KiB headers total, 1 MiB body, 16 KiB per header | `http.rs` |
| smuggling | one request per POST connection; reject `Transfer-Encoding` entirely; require `Content-Length` for POST | `http.rs` |
| XSS into the plane (model-controlled text) | **never** `format!` HTML: escape with one function used by every renderer; markdown via `push_html` + our escaper; **no `innerHTML`** in `app.js` for model content | `render.rs`, `assets/app.js` |
| agent-produced `.html` executing on our origin | served only under `GET /file/{path}` with `Content-Security-Policy: sandbox allow-scripts; default-src 'none'; img-src data:` and `X-Content-Type-Options: nosniff`, embedded by the client in `<iframe sandbox="allow-scripts">` | `http.rs` + W040 |
| a client disconnecting mid-stream | the SSE per-connection task ends on write error; it never unwinds the server | `sse.rs` |

**Rejected: a printed bearer token.** It defends only against "another page in your browser", which `Origin`+JSON+loopback already covers, and it costs the human a copy-paste on every launch (`EventSource` cannot set headers, so the token would end up in the URL anyway). Recorded as the fallback if the human disagrees (§9 Q2).

### 4.9 F6 — where the visual spec lives, and what it inherits

**Decision: `docs/design/web-control-plane.md`** — `docs/design/` already holds per-surface specs and drafts (`wcode-mark.md`, `vscode-ui-v3-draft.html`, `identity/candidates.html`). `docs/tui-design.md` is **not** amended (it is the terminal's authority; nothing here changes a glyph, a band, or a palette role).

It inherits, from sources that already exist:

- **The mark** — `❯▌` per D042 (`docs/decisions/D042-wcode-mark.md`, geometry in `docs/design/wcode-mark.md` §1), shown **once**, on the shell's empty state, retired by the first block — the terminal tier's rule (`docs/tui-design.md:63` "**The mark** is the product's only logo — `❯▌` (D042) — shown once, on the root surface's empty state, and retired by the first block").
- **The 17 palette roles**, verbatim names: `accent, dim, muted, border, user, body, error, success, warn, code, heading, heading_sub, link, tool_name, thinking, diff_add, diff_del` (`README.md:127`–`:129`; the enum-side source is `crates/wcode-tui/src/theme.rs:275` `pub struct Palette {`). The web sheet does **not** re-derive hues; it maps role → CSS custom property, so a role's meaning is the same on both surfaces.
- **The token vocabulary of the retired page**, read as a *source* not a file: `style.css:4`–`:16`'s `--bg:#0a0e14`, `--fg:#c9d1d9`, `--muted:#7d8590`, `--border:#21262d`, `--accent:#58a6ff`, `--prompt:#3fb950`, `--code:#d2a8ff`, `--warn:#d29922`. The control plane's stylesheet lives **with the crate** (`crates/wcode-serve/assets/app.css`), so `style.css` stays byte-identical and remains the rollback `docs/next-steps.md:117` describes.
- **The dials**, re-tuned for a dashboard rather than a marketing page: `VARIANCE 3` (one layout, three bands, no hero), `MOTION 2` (the live cursor + one transition; nothing else), `DENSITY 7` (a control plane is a dense instrument). `.wcode/skills/design-taste/SKILL.md`'s table currently offers only "web — the wcode site" (5/3/4) and "greenfield marketing"; a third row is added by **step 7**.
- **Two lines that are currently wrong and must be fixed by this item**: `.wcode/skills/design-taste/SKILL.md:3` "…or the web page (`index.html`/`style.css`)" and `.wcode/agents/designer.md:8`–`:9` "- The **web page** — `index.html` + `style.css`." After W040: the second web surface is the control plane, spec'd by the new doc; `index.html`/`style.css` are a retired artifact and the agent must not restyle them.

### 4.10 Integration points (re-verified this session)

**Added**

- `crates/wcode-serve/Cargo.toml`, `src/{lib,http,router,sse,files,render,tasks,assets}.rs`, `assets/{app.html,app.js,app.css}`.
- `crates/wcode-serve/src/lib.rs` — `Options { addr, root, registry, roster, root_session, tasks, label }`, `spawn(Options) -> Result<Handle, ServeError>`, `Store` (sessions, ring, `seq`), `TaskRow`.
- `docs/design/web-control-plane.md` — §F6's spec.
- `docs/decisions/D045-web-control-plane.md` — §9's record.

**Changed**

- `crates/wcode-cli/src/main.rs` — the `--web` arg (`Args`), the spawn inside `serve()` (§4.4), `task_feed()`, the `--stdio`/`--task` refusals (`:1893`, `:1446`), the `eprintln!` at `:1910` gaining the URL.
- `crates/wcode-protocol/src/registry.rs` — `pub fn owner_of(&self, id: &SessionId) -> Option<SessionId>` beside `:237` `pub fn set_owner(`, reading `:65` `owners: Mutex<HashMap<SessionId, SessionId>>,` (which `:324` `pub fn permitted(` already consults, so the map is the truth today and only the getter is missing).
- `crates/wcode-harness/src/protocol.rs` — `SessionInfo.owner: Option<SessionId>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` (the same pair `:179`/`:180` uses for `model`; the `state` precedent for tolerating an old peer is `:194`–`:195`), filled in `roster_infos` (`crates/wcode-protocol/src/server.rs:383` `fn roster_infos(`).
- `Cargo.toml` — one `members` entry; `pulldown-cmark` and `ignore` in `[workspace.dependencies]`.
- `.github/workflows/publish.yml:59`–`:60`, `:83` — the fifth crate in both lists.
- `docs/DEPENDENCY_GRAPH.md:5`–`:9`; `docs/book/14-protocol-and-clients.html` (a fourth client), `docs/book/03-run-modes.html` (`--web`).
- `README.md` — the run-modes/flags table plus a "Control plane" section.
- `editors/vscode/src/protocol.ts` — `owner?: string` on `SessionInfo` only.
- `docs/next-steps.md` — row 112 (+ a row for Q4's parity gap).
- `.wcode/skills/design-taste/SKILL.md:3`, the dial table; `.wcode/agents/designer.md:8`–`:9`.

**The anchor table**

| anchor | the fact |
|---|---|
| `crates/wcode-protocol/src/server.rs:93`–`:95` | `loop {` / `listener.accept().await?` / `tokio::spawn(serve_stream(` — multi-client is built |
| `server.rs:37`, `:41`, `:68` | `Roster` / `Root` / `DefineHandler` — the three seam types `wcode-serve` reuses; `DefineHandler`'s doc is the injection precedent ("the composition root injects one, since this crate cannot name a session factory") |
| `server.rs:174`–`:183` | the seeded `Sessions` frame (a client learns the roster on connect, unprompted) |
| `server.rs:196`–`:223` | the watch task that fans late sessions and re-pushes the roster |
| `server.rs:383`–`:403` | `roster_infos` — where `SessionInfo.owner` gets filled |
| `server.rs:408`–`:433` | `fn fan` — the per-session forwarder the plane's SSE loop mirrors; `:428` lag is dropped, not fatal |
| `crates/wcode-harness/src/actor.rs:137` | `pub const EVENT_BUFFER: usize = 1024;` — the broadcast depth the ring must respect |
| `crates/wcode-harness/src/protocol.rs:34` | `pub const PROTOCOL_VERSION: u32 = 1;` — unchanged by §F3 |
| `protocol.rs:175`, `:179`–`:180`, `:194`–`:195` | `pub struct SessionInfo {`, the `model` serde pair to copy, the `state` `#[serde(default)]` tolerance precedent |
| `protocol.rs:230`–`:232` | the `Serialize/Deserialize` + `#[serde(tag = "type")]` pair on `Request` |
| `protocol.rs:396`–`:415` | `pub struct Frame<P> {` and its five fields — no `ts`, unchanged |
| `protocol.rs:453`–`:458` | `fn request_roundtrip_all_variants()` — the round-trip gate a reused body type keeps passing |
| `crates/wcode-harness/src/event.rs:61` | `pub enum AgentEvent {` — 27 variants, **no** `#[serde(other)]` (documented in `:250`–`:262`): a new variant breaks an old *client's* read loop, which is why §F3 adds none |
| `event.rs:101` | `duration_ms: Option<u64>,` — the only time value on the wire |
| `event.rs:82`–`:99` | `diff`/`path` — the change-walkthrough's raw material (W041) |
| `crates/wcode-protocol/src/registry.rs:65`, `:237`, `:324` | the `owners` map, its only writer, and `permitted` reading it — the getter is the only missing piece |
| `crates/wcode-cli/src/main.rs:1812` | `async fn serve(` — the wiring point |
| `main.rs:1836`–`:1839`, `:1858`–`:1861` | the `registry` and `roster` already built there |
| `main.rs:1888`, `:1893`, `:1910`, `:1911` | the `serving …` line, the stdio branch, the socket line + `serve_at` |
| `main.rs:2461`–`:2465` | `fn default_socket_path()` — one path, hence the multi-server constraint (§4.6) |
| `main.rs:2125`–`:2148` | the TUI's task forwarder — the exact loop `task_feed` copies |
| `main.rs:3225`–`:3235` | `fn task_item(&crate::tasks::Task) -> wcode_tui::TaskItem` — the field mapping `TaskRow` extends |
| `crates/wcode-tui/src/lib.rs:294`, `:379` | `pub tasks: Vec<TaskItem>,` / `new_tasks: Option<mpsc::UnboundedReceiver<Vec<TaskItem>>>` — the seam shape to mirror |
| `crates/wcode-tui/src/app.rs:1317`–`:1328` | `pub struct TaskItem {` — six fields; `TaskRow` adds `gate`, `run`, `feedback`, `artifact_len` from `tasks.rs:200`/`:207`/`:209` |
| `crates/wcode-cli/src/tasks.rs:1`–`:4` | "Root-owned; only the root's `task` tool … mutates it, so only the root plans" — the read-only rule (§F4) |
| `crates/wcode-cli/src/tasks.rs:173`–`:192`, `:200`, `:207`, `:209` | `pub struct Task {`, its fields, `feedback`/`gate`/`RunSpec` |
| `crates/wcode-cli/src/tasks.rs:232`, `:297`, `:311`–`:312` | the journal's op line, the sink, `journal: Option<Mutex<Journal>>` — **no timestamps** |
| `crates/wcode-cli/src/session_groups.rs:81` | `pub fn plan_path(&self) -> PathBuf {` — where the journal lives |
| `crates/wcode-harness/src/session.rs:13`–`:69` | `SessionEntry` — `Header.created` is the only time, and it is the session's, not a turn's |
| `crates/wcode-harness/src/message.rs:36`–`:60` | `AgentMessage` — no time, no id |
| `crates/wcode-harness/src/tool.rs:23`–`:36` | `pub struct ToolOutput {` with `diff`/`path` "for *presentation only*" |
| `crates/wcode-cli/src/tools/read.rs:11`, `:15` | `MAX_LINE_CHARS = 300` / `MAX_READ_LINES = 1000` — **model-facing** caps; the plane must not reuse them (§4.9 renders a file, not a prompt) |
| `crates/wcode-tui/src/markdown.rs:43`–`:46` | `Options::ENABLE_TABLES \| ENABLE_STRIKETHROUGH \| ENABLE_TASKLISTS \| ENABLE_FOOTNOTES` — the four flags the web renderer must match |
| `crates/wcode-tui/src/theme.rs:275`–`:300` | `pub struct Palette {` — the role source |
| `Cargo.toml` `[workspace.dependencies]`, `crates/wcode-tui/Cargo.toml:23` | `pulldown-cmark` is declared in the **tui** manifest only, with `default-features = false` — hence the `html` feature ask |
| `.github/workflows/publish.yml:4`, `:59`–`:60`, `:83` | `(wcode-harness → wcode-protocol → wcode-tui → wcode-cli)` and both hardcoded lists |
| `docs/next-steps.md:117` | "the retired landing page … stays in place, unserved, as a rollback" |
| `.github/workflows/pages.yml:1`–`:5` | "Publishes the book (docs/book) … Replaces the old 'legacy' Pages build that served `index.html` from the repo root" |
| `docs/tui-design.md:733` | "**Timestamps** on turns: lean no." — the §F3 argument |
| `docs/tui-design.md:63` | the `❯▌` mark shown once, retired by the first block |
| `.wcode/agents/designer.md:8`–`:9`, `:11`; `.wcode/skills/design-taste/SKILL.md:3` | the two lines that call `index.html` "the wcode web page", and the "never restyle past the spec" rule |
| `docs/work/W010-…md:3`, §2.2 | W010 **shipped** (D013), and Option B is rejected there |

### 4.11 What the plane refuses to claim

- **No "history" it cannot date.** The timeline is live-only and says so; a past-run view waits for a timestamped entry.
- **No second plan writer.** The dashboard is read-only; "submit" is a `Request::Submit` to the root (§F4).
- **No server's config.** The plane shows the `serve` process's own settings (`Request::Settings`), never a client's `config.toml`, and offers no config write (D036).
- **No remote access.** Loopback only, and the flag refuses anything else rather than binding and hoping.
- **No fetch of a file it truncated.** `files.rs` returns an explicit `truncated: bool` + the byte count, so the viewer says "1 MiB of 7 MiB" instead of lying by omission.

---

## 5. Plan

1. **The crate skeleton and the flag.** Deliverable: `crates/wcode-serve` in `[workspace] members` with `Options`, `spawn`, and a `Store` that answers `GET /` with the embedded shell and `GET /api/sessions` with the roster; `serve --web` spawns it and prints the URL; `--web` + `--stdio` errors out. Gate: `cargo build`; `curl -s localhost:7878/api/sessions` returns the root's id; `cargo clippy -p wcode-serve --all-targets` clean; the two refusal tests.
2. **The seams: owner, tasks, `TaskRow`.** Deliverable: `Registry::owner_of`, `SessionInfo.owner`, `roster_infos` filling it, `task_feed` + `TaskRow` in `wcode-cli`, and `/api/sessions` carrying both. Gate: a `registry.rs` unit test (`owner_of_returns_the_recorded_edge`), a `server.rs` roster test (the owner appears in the pushed `SessionInfo`), a live `serve --agents` showing root + workers with `owner` set, and the protocol round-trip still green.
3. **SSE.** Deliverable: `GET /api/events` with `seq`/`ts`, a 2,000-frame ring, `Last-Event-ID` resume, one task per connection, a clean end on write error. Gate: two concurrent clients see identical `seq` sequences; a forced disconnect + reconnect resumes with **no gap and no duplicate** (asserted against a scripted fake); a `RecvError::Lagged` skips forward without ending the stream.
4. **The shell.** Deliverable: the rail (sessions with `[state]`, the team nested by `owner`), the transcript (markdown `push_html`, fences via `syntect::html` + a generated `syntax.css`, the live block **replaced** on `MessageUpdate`, history backfilled from `/api/transcript`), the input box, the `❯▌` empty state. Gate: a live run renders prose, a table, a task list, a footnote, and a highlighted rust fence; a second browser tab shows the same transcript; a refresh mid-run re-renders with no duplicate blocks; a pixel-free DOM assertion that the empty state carries one `❯▌` and the first block retires it.
5. **One verb, to prove the POST shape.** Deliverable: `POST /api/session/{id}/request` with `Cancel` and `Submit` only, the reply echoed into the plane. Gate: a click cancels a live run (the `Stopped`/`Error` arrives, the rail flips to `idle`); a `Submit` starts a turn on the **addressed** session, not the focused one (two sessions, one addressed); the request/response bodies are byte-compatible with `Request`/`AgentEvent` (a round-trip test over the wire JSON).
6. **The security floor.** Deliverable: §4.8's eight rows, each a named check. Gate: a table-driven `http.rs` test with one case per row (a foreign `Origin` → 403, a `text/plain` POST → 415, `../` → 400, a 2 MiB body → 413, `Transfer-Encoding` → 400, 33 headers → 431, a `Host: evil.com` → 403, `--web 0.0.0.0:…` → refused at spawn); plus a manual `curl` pair (no `Origin` → 200, `Origin: https://evil.com` → 403).
7. **Spec, docs, release plumbing.** Deliverable: `docs/design/web-control-plane.md`, D045, the two skill/agent lines, the five doc/book/README edits, the publish.yml lists, tracker rows 112 (+ the TS parity gap). Gate: `cargo package -p wcode-serve --allow-dirty` lists exactly the expected files; `scripts/check-book.py` passes; `--dump-config` and `--dump-system-prompt` byte-identical to before; `npm run typecheck && npm test` green in `editors/vscode` (only `protocol.ts` changed).

Step 4 is the demo; if the human wants to see something before the security floor lands, stop after step 4 **and do not leave the process running unattended** — steps 1–4 without step 6 are a loopback-only, `Origin`-less surface, and that is the one combination I would not ship.

---

## 6. Quality gates

```sh
cargo test --workspace            # all pass; today's figure is 1534 / 4 ignored (tracker row 111)
cargo test -p wcode-serve         # the new crate's unit + seam tests
cargo clippy --workspace --all-targets   # clean, no new #[allow]
cargo build --bin wcode           # necessary, not sufficient
cd editors/vscode && npm run typecheck && npm test
```

Behavioural gates, each with an expected output:

- `./target/debug/wcode --dump-system-prompt` → **byte-identical** to before (nothing here touches the prompt).
- `./target/debug/wcode --dump-config` → **byte-identical** (no new config table; `--web` is a flag, not a setting — and if the human wants `[serve] web = …` that is a config table, so it is refused here by design).
- `./target/debug/wcode serve --stdio --web` → exit 2 with a clear message, no listener.
- `./target/debug/wcode serve --web 0.0.0.0:7878` → exit 2, no listener.
- `cargo package -p wcode-serve --allow-dirty --no-verify` → file list = `src/**`, `assets/**`, `Cargo.toml`, `README.md`.
- `python3 scripts/check-book.py` → green.

Regression tests that must stay green, by name: `request_roundtrip_all_variants` (`crates/wcode-harness/src/protocol.rs:453` — a reused `Request` body type keeps this honest), `unknown_entries_tolerated` (`session.rs`'s forward-compat pin — untouched), `is_inbound_marks_only_the_delivery_verbs` (`protocol.rs` — the plane adds no inbound verb), `fn tag` in `crates/wcode-harness/tests/loop_tests.rs:263` (the workspace's **one total `AgentEvent` match**, 27 arms, no `_` — W040 adds no `AgentEvent`, so it must not change), `a_socket_client_reads_the_servers_settings` and the roster/`Sessions` push tests in `crates/wcode-protocol/src/server.rs`'s test module (the roster's shape changes by one field, so these are the tripwires), and `esc_closes_the_info_modal_and_nothing_else_does` / `keys_do_not_reach_the_input_while_a_modal_is_open` in the TUI (**must not be touched at all** — the TUI is not in this diff, and if they change, something leaked).

---

## 7. Testing

### 7.1 Automated

**Unit — `crates/wcode-serve` (no socket, no process):**

- `a_request_line_parses_into_method_path_query` — the happy path plus a query string with a `&` and a `%` escape.
- `the_parser_rejects_eight_malformed_shapes` — one case per §4.8 row that is a *parse* concern: no `Content-Length` on POST, `Transfer-Encoding: chunked`, 33 headers, a 9 KiB request line, a NUL in a header name, a duplicate `Content-Length`, a bare `\n` line ending, an absolute-URI request target.
- `a_path_that_escapes_the_root_is_refused` — `../../etc/passwd`, `/etc/passwd`, `a/../../b`, a symlink pointing out, a URL-encoded `%2e%2e%2f`.
- `sse_frames_carry_a_monotone_seq_and_one_clock` — 3 events → `seq` 1,2,3 and non-decreasing `ts`.
- `resume_from_last_event_id_has_no_gap_and_no_duplicate` — the property test the ring exists for.
- `a_lagged_subscriber_skips_forward_and_keeps_streaming` — the `server.rs:428` shape.
- `markdown_renders_the_four_enabled_features` — a table, a task list, a footnote, strikethrough; and `an_image_alt_cannot_break_out_of_html` (the escaper's teeth).
- `a_fence_is_classed_html_not_escaped_text` — a rust fence yields ≥2 distinct `<span class=…>` groups (the `syntect` html path is wired, not dead).
- `task_row_maps_every_field` — against a `tasks::Task` fixture with `gate`, `RunSpec::Script`, `feedback`, `attempts`.
- `owner_of_returns_the_recorded_edge` (+ `None` after `remove`) in `registry.rs`.

**Seam — an in-memory duplex `AsyncRead + AsyncWrite` (the `serve_stream` precedent, `server.rs:141`):**

- `the_plane_serves_the_roster_over_a_duplex_stream` — a scripted host with 3 sessions; `GET /api/sessions` returns them in serve order, root first.
- `a_post_reaches_the_addressed_session_not_the_focused_one` — two sessions, POST to the non-first; assert the *other* handle received it (the `Submit`/`SubmitTo` trap retired by construction).
- `a_write_error_ends_one_stream_and_not_the_server` — drop the client's read half mid-SSE; the server keeps serving a second client.

**Live, skipping when the binary is absent (the `routing.test.ts:31` precedent):**

- `the_control_plane_answers_on_a_real_serve` — spawn `./target/debug/wcode serve --web 127.0.0.1:0 --socket <tmp>`, read the printed URL, `GET /api/sessions`, `GET /api/events` for 200 ms, assert ≥1 frame and a clean close.
- `a_refused_port_run_still_renders` — `WCODE_RETRY_MAX=2` + `--base-url http://127.0.0.1:9/v1`; the rail shows `failed`, the transcript shows the retry notices, no hang.

### 7.2 How a human verifies it

```sh
cargo build --bin wcode
```

**(a) It opens, and it is live.** Two terminals:

```sh
./target/debug/wcode serve --web --agents          # prints: serving … ; control plane on http://127.0.0.1:7878/
```
```sh
curl -s localhost:7878/api/sessions | head -c 400   # the root's id + the roster as JSON
```

Open `http://127.0.0.1:7878/`. Look for: the rail with one `wcode` row, `[idle]`; the empty state as one dim `❯▌ wcode`; the input box focused.

**(b) Parallel, literally.** In the page, submit a prompt, then in a second browser tab open the same URL. Look for: **both tabs stream the same tokens at the same time**, the rail flips `[running]`, and the second tab shows the turn already in progress from its start (history backfill) with no duplicated block.

**(c) The team is a tree.** With `--agents`, ask the root to spawn a worker (or use a `.wcode/team.toml`). Look for: the rail gains an indented row under `wcode` (the `owner` edge), its state moving `idle → running → done`, and its transcript reachable by a click.

**(d) Rendering.** Ask for a table, a rust code fence, and a task list in one turn. Look for: a real `<table>`, ≥2 token colours in the fence matching the TUI's `[theme]` preset, checkboxes. Then cancel mid-turn: the rail returns to `idle` and the transcript keeps what was produced.

**(e) The security floor.** From the shell, three checks whose outputs are the gate:

```sh
curl -s -o /dev/null -w '%{http_code}\n' localhost:7878/api/sessions                        # 200
curl -s -o /dev/null -w '%{http_code}\n' -H 'Origin: https://evil.com' localhost:7878/api/sessions   # 403
curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: evil.com' localhost:7878/api/sessions             # 403
curl -s -o /dev/null -w '%{http_code}\n' -X POST -d 'x' localhost:7878/api/session/wcode/request    # 415
curl -s -o /dev/null -w '%{http_code}\n' 'localhost:7878/api/file?path=../../etc/passwd'            # 400
```

**(f) The refusals.**

```sh
./target/debug/wcode serve --stdio --web    # exit 2, a message, nothing listening
./target/debug/wcode serve --web 0.0.0.0:7878  # exit 2, a message about loopback
```

**(g) The error path — no hang, no lie.**

```sh
WCODE_RETRY_MAX=2 ./target/debug/wcode serve --web --base-url http://127.0.0.1:9/v1
```

Open the page, submit. Look for: `retrying 1/2 …` in the transcript, the rail `[running]` then `[failed]`, the SSE stream alive throughout, and no backtrace on stderr.

---

## 8. Expected outcome

- `wcode serve --web` serves a browser control plane on loopback; `wcode serve` without it is byte-for-byte today's behaviour (no port, no new stderr line, no new dep compiled into the path).
- The plane lists **every session the server serves** — root and live team, nested by a new `owner` edge — and every one is live: a second browser tab sees the same stream.
- A turn rendered in the browser is the same turn the TUI renders: same markdown dialect (the four `pulldown-cmark` options), same syntax-highlight *roles* (a `syntax.css` generated from the same syntect theme), same `❯▌` once on the empty state.
- `POST /api/session/{id}/request` carries `Request` **verbatim** and answers `AgentEvent` **verbatim** — one schema for the socket, the extension, and the browser.
- Eight reject paths are covered by eight named tests, and a foreign `Origin`, a `text/plain` POST, a traversal, and a non-loopback bind are all refused with a status the human can paste.
- The timeline's stamps come from one clock in one process; `Frame` is unchanged, `PROTOCOL_VERSION` is still 1, and `docs/tui-design.md:733`'s "lean no" is still true.
- `index.html` and `style.css` are untouched and still described as a rollback; `docs/design/web-control-plane.md` is the authority for the third surface, and `.wcode/agents/designer.md` names it.
- Five crates publish in order; `docs/book` has a fourth client on its protocol page.
- The five remaining asks are **W041 (browser hub) and W042–W045**, each with a one-paragraph boundary, and none of them requires redesigning the core.

---

## 9. Open questions for the human (and the proposed D045)

**Q1 — the only dependency ask.** `pulldown-cmark`'s `html` feature pulls **one** new transitive crate, `pulldown-cmark-escape` (same authors, same version family; the alternative is a hand-rolled HTML escaper over model-controlled text, which I refuse on XSS grounds). `ignore` moves from `wcode-cli` to `[workspace.dependencies]` (no new third-party code). **Everything else is code we write.** Yes?

**Q2 — auth.** I recommend **no token**: loopback bind + `Origin`/`Host` + `Content-Type: application/json` (§4.8), because a printed secret defends against nothing a local page can't already do better, and `EventSource` cannot set headers so the token would ride the URL anyway. If the human wants a token regardless, it changes step 6 only.

**Q3 — is `--web` opt-in right?** I have it **off by default**: nothing opens a port uninvited. The cost is that the human must remember the flag. The alternative (on by default with `serve`) would make every `serve` a listener, which I would argue against.

**Q4 — the VS Code `protocol.ts` is three variants behind** (`set_title`, `usage`, `session_title` absent; its own doc comment says "24 tags" against the kernel's 27). W040 adds `owner?: string` and files the rest as a tracker row. Fix it here, or keep it a separate item?

**Q5 — the empty state's one mark, and the `designer` agent.** W040 proposes `docs/design/web-control-plane.md` as the third surface's authority and edits `.wcode/agents/designer.md:8`–`:9` + `.wcode/skills/design-taste/SKILL.md:3` to stop calling `index.html` "the wcode web page". Sign-off needed because it reassigns a role's scope.

**Q6 — W042's prerequisite, paid now or later?** `SessionInfo.owner` + `Registry::owner_of` is in W040's scope (step 2) because the rail's tree needs it. The alternative is a flat rail in W040 and the field in W042 — but then W040's rail is wrong-shaped for one item.

### Proposed decision record — D045

`docs/decisions/D045-web-control-plane.md`, ~50 lines:

- **Context** — `serve` is already multi-session and multi-client (`server.rs:93`–`:95`, `:196`–`:223`), but the only clients are a terminal and an editor; the ask is a control plane. There is no HTTP server anywhere in the workspace and `AGENTS.md` forbids dependencies unless requested.
- **Decision** — the control plane is a **fourth client** that speaks **HTTP/1.1 + SSE**, hand-written on tokio in a new `wcode-serve` crate, **hosted by the `serve` process** under `--web` (loopback, opt-in), and it **renders on the server** (markdown → HTML, syntect → classed HTML) with `Request`/`AgentEvent` as its body types. Timestamps live in the plane's envelope, not in `Frame`. The task store stays CLI-side; the plane gets a `watch` feed and never writes the plan. No `config.toml` writer (D036 stands). No multiple-roots-per-process (D013 stands).
- **Consequences** — costs: a fifth published crate and two hardcoded workflow lists to edit; ~600 lines of HTTP parser; one new transitive crate; a third rendering path in the repo (TUI-ratatui, extension-markdown-it, web-pulldown) with the first two deliberately not unified here. Forecloses: axum/hyper/tungstenite (reopen only with a dependency request), a client-side redux-style reducer as the plane's architecture, remote/LAN access, a durable timeline without a kernel change, and any restyling of `index.html`.

---

## 10. Deferred / rejected (with the reason it is not "later")

- **The other six surfaces as one item.** Rejected: seven asks over one undecided transport is not one work item. Boundary drawn in §2.1 and §14; **W042** files (tree, viewer, syntax, markdown, sandboxed html), **W043** changes (diff capture per run + the walkthrough), **W044** timeline + orchestration graph (needs §F3's clock; adds the durable-entry kernel question as W044b), **W045** control (task dashboard read-only + submit-as-`Submit`, the settings screen over existing `Request`s, the multi-server rail), and **W041** the browser hub.
- **Reusing `editors/vscode/src/reducer.ts` in the browser.** Rejected for W040: it is TypeScript, so the crate would need a node/esbuild build step (a second toolchain inside `cargo build`), and the versioned asset would be a build artifact in the tree. Recorded as the alternative if the human prefers a client-rendered SPA: then the reducer is *copied*, the build step is added, and step 4's cost roughly halves while step 7's grows.
- **`axum`/`hyper`.** §4.1 (b) — reopened only by an explicit dependency request.
- **WebSocket.** §4.1 — needs hand-written SHA-1+base64 for a duplex we have no use for.
- **A token.** §9 Q2.
- **`Frame::ts` / `PROTOCOL_VERSION = 2`.** §F3 — the TUI's spec says "lean no", and the plane does not need it.
- **A timestamped `SessionEntry`.** A kernel change (resume, compaction, the JSONL wire) for a *history* feature; its own item after W042 proves the timeline's value.
- **A second plan writer (assign/reopen/cancel a node from the browser).** `tasks.rs:1`–`:4` makes the root the only planner and `docs/task-dag-plan.md` §2.3 ("The model proposes; the scheduler disposes") is the doctrine. A UI-driven node write is a design decision with a decision record, not a feature slice.
- **Editing `index.html`/`style.css` to become the control plane.** They are a rollback artifact (`docs/next-steps.md:117`). The plane's sheet lives with its crate and copies the token *names*.
- **A framework/bundler/CSS preprocessor/icon font/external font.** §2.3, on the house rule in `style.css:1`–`:2`.
- **Restyling the TUI or the extension.** `docs/tui-design.md` is the authority; nothing here amends it.
- **`serve` shutting down from the browser.** A "stop the server" verb is `POST /api/shutdown` — trivial, and a footgun while the human is watching a run. Later, with a confirm.
- **A `[serve] web = …` config table.** A flag is the whole knob; a config table would be behaviour-in-config, which `AGENTS.md` refuses.

---

## 11. What was not verified

1. **Nothing was compiled or run.** `bash` mutations are refused for this worker: no `cargo build`/`test`/`clippy`, no `curl`, no browser, and no shell clock for the date. Every "~N lines" and "~N tests" figure is a judgement; every code claim is a *read*.
2. **`syntect`'s `html` feature and `pulldown-cmark`'s absence of it** were verified by reading the registry copies (`syntect-5.3.0/Cargo.toml` `default-fancy = [… "html" …]`; `pulldown-cmark-0.13.5/Cargo.toml` `default = ["getopts","html"]`, `html = ["pulldown-cmark-escape"]`) and by grepping `Cargo.lock` for `pulldown-cmark-escape` (absent). That the **TUI's** `default-features = false` therefore excludes `html` is my inference from the feature table — I did not compile `wcode-tui` to prove `syntect::html` is linked there (it is `syntect`'s features, not `pulldown-cmark`'s, that matter for the fence renderer, and syntect is fine).
3. **The 27 `AgentEvent` variants and the 18 `Request` variants are counts from reading the enums**, cross-checked against `protocol.ts`'s "24 tags" comment. No compile asserted them.
4. **`crates/wcode-protocol/src/server.rs` was read end to end (665 lines) — it is small.** `crates/wcode-cli/src/main.rs` (5,096 lines) was read only at `serve()` (`:1809`–`:1924`), the socket client (`:1028`–`:1269`), the task forwarder (`:2118`–`:2150`), `task_item` (`:3225`), `default_socket_path` (`:2461`), and the arg table (`:49`–`:83`, `:364`). **Not** read: the rest of arg parsing, so whether `--web` collides with an existing prefix or abbreviation is **unverified**.
5. **`crates/wcode-tui/src/ui.rs` (7,303 lines) and `app.rs` (12,547) were not read.** The TUI claims here are from `docs/tui-design.md`, `lib.rs`, and `main.rs` — so "the TUI renders a surface per roster entry" rests on `main.rs:1124`–`:1150`, not on the reducer.
6. **`editors/vscode/` was swept, not read.** I read `manager.ts:1`–`:40`, `protocol.ts:45`–`:105`, `highlight.ts:1`–`:40`, `reducer.ts:1`–`:30`, `package.json`'s scripts/deps, and the purity test. I did **not** run `npm test`, and I did not read `tab.ts`/`session.ts`/`webview.ts` in full — so whether the extension already renders a team *tree* (which would be a second implementation of W042's view) is **unknown**.
7. **No browser was involved.** Every claim about `EventSource`/`Last-Event-ID`, `fetch` streaming, iframe `sandbox`, and `CSP: sandbox` is from the specifications, not from a run. `CSP: sandbox` on a `Content-Type: text/html` response is the piece I would test first (§7.1 `an_image_alt_cannot_break_out_of_html` covers escaping, not CSP).
8. **The 1 MiB body / 8 KiB line / 32-header caps are invented**, not measured against a real browser's behaviour. A browser's largest request here is a `Submit` payload (a pasted document could exceed 1 MiB) — the cap may need to be 8 MiB, and the `413` path needs a UI ("your message is too large").
9. **`main.rs:1124`–`:1150` builds one surface per roster entry — I read that it does**, but I did not verify what a *worker's* `Backend` does on a socket when the worker ends (a `remove` from the registry, `registry.rs:202`). A rail row that outlives its session is a plausible bug I have not checked.
10. **Tracker rows 96–111 were read from the table but the prose sections below it (~line 200–1244) were only skimmed** — a later section may already track a control plane, a web surface, or a timestamp gap, which would make part of this brief a duplicate.
11. **`docs/book` has 19 pages; I listed them and read none.** The two pages step 7 edits (`03`, `14`) may have a house style this brief does not match.
12. **The claim that `cargo install wcode-cli` resolves a fifth crate needs no formula change** is reasoning from the formula's generator, not a `brew install`.
13. **`docs/design/web-control-plane.md`'s URL is an assumption** about where design docs should live (`docs/design/` holds drafts + `wcode-mark.md`; there is no precedent for a *locked* web spec), and `docs/tui-design.md`'s location is the only precedent for a locked spec at the top level.

---

## 12. References

- `AGENTS.md` — the dependency rule, the kernel/CLI split, "config files for behavior" absent by design, `Hooks` as the policy home.
- `docs/decisions/D036-tui-settings-panel.md` — the no-config-writer boundary the web settings screen inherits.
- `docs/decisions/D042-wcode-mark.md` + `docs/design/wcode-mark.md` — the `❯▌` mark and its two tiers.
- `docs/decisions/D013-multi-tab-sessions-vscode.md` + `docs/work/W010-multi-tab-multi-session-vscode.md` — Option A shipped, Option B refused (§4.7).
- `docs/task-dag-plan.md` §2.3, §4 — "the model proposes; the scheduler disposes", and the journal's home.
- `docs/tui-design.md` §1.3 (the 17 roles), §1.4 (the mark), §6 (timestamps: lean no).
- `docs/vscode-extension-plan.md` §3.1, `docs/sketches/serve-stdio.md` — the stdio one-client contract.
- `docs/next-steps.md` row 111 (1534 tests), row 112 (this item), `:117` (the retired page).
- `.wcode/skills/design-taste/SKILL.md`, `.wcode/skills/live-verification/SKILL.md`, `.wcode/skills/traceability-log/SKILL.md`, `.wcode/agents/designer.md`.
- `docs/work/W032-tui-settings.md` — the shape of this brief, and the proof that a read-only worker's brief survives implementation.

---

## 13. Pivot: tabs first, browser second (locked 2026-10-29)

The human's objection to §4.2's topology killed the socket hub, and what replaced it reorders the whole item: **a conversation is an in-process root**, so the capability is not web-specific — any surface that can hold several `Backend`s can show several conversations. The browser is the *second* consumer, not the first.

### 13.1 What was rejected, and why

`Client` is `#[cfg(unix)]` (`crates/wcode-protocol/src/backend.rs:45`) and non-Unix `serve` exits 2 (`main.rs:1921`), so a hub built on one `serve --socket` child per conversation is a Unix-only product and needs a supervisor (spawn, readiness retry, pids, orphans, a socket dir, 0600 perms) for a capability the process can just *have*.

### 13.2 The locked decisions

1. **A conversation is an in-process root runtime**: `Backend::Local(SessionHandle)`. No child process, no socket, no pid, no readiness wait, cross-platform.
2. **A `RootSpec` factory injected by the CLI** — the `DefineHandler` precedent (`crates/wcode-protocol/src/server.rs:68`, "the composition root injects one, since this crate cannot name a session factory"). `wcode-serve`/the TUI never build a session; the CLI does, exactly as `serve()` does today (`main.rs:1812-1924`).
3. **`Background`'s `LIVE` becomes scoped** (`crates/wcode-cli/src/tools/background.rs:287`): one process-global weak list today, so `shutdown_all()` covers every agent; closing one conversation must kill only that conversation's `bg` tasks.
4. **Roots are supervised, not joined**: each root's actor task gets a `JoinHandle`; its exit marks `ConversationState::Failed`, never unwinds the process. (R3 catches a panicking *tool*; a panicking *actor* is new ground.)
5. **The TUI proves it as tabs** (W040): `App.surfaces: Vec<Surface>` (`app.rs:2632-2635`, "One entry per conversation surface. Index 0 is the root; the rest are team members") gains a **two-level container** — conversations, each holding its root + members. N = 1 must be byte-identical to today.
6. **Tabs = roots; the team region = members of the focused root.** `Ctrl-N`/`Shift-Tab` (today "focus the next / previous surface", `app.rs:1146`) cycles **conversations**; `Alt-1..9` (today "focus sidebar row N") keeps focusing **member rows** inside the active conversation.
7. **`/tab new | close | <n>`** join `COMMANDS` (so `F1` and completion pick them up). `/rename` needs nothing new — it already reaches `Agent::set_title` on the focused session.
8. **A reverse channel:** the TUI can only answer the CLI at exit today (`pending_new`/`pending_reload`/`pending_resume`, then a re-exec), which is why "new conversation" is "new terminal". New conversations need `Action::NewConversation` out and a `SurfaceSpec` back down the existing `new_surfaces` channel (`lib.rs:378`, consumed at `:579`).
9. **No new dependency in W040.** `axum` arrives with W041 and only there; the HTTP surface, `ts`/`seq` envelope, `pulldown-cmark`/`syntect` rendering and the security floor stay as specified in §4.3/§4.8 for the browser consumer.
10. **Deferred (not cancelled):** a per-root socket so a TUI or the editor can attach to a hub-owned conversation ("multi-UI one session"), the disk catalog reader, and every §2.1 surface beyond the transcript.

### 13.3 Ownership, for the browser's benefit

`wcode-harness` and `wcode-protocol` are **unchanged *on the wire*** by W040 — no `Frame.ts`, no `PROTOCOL_VERSION` bump, no `SessionInfo.owner` (the hub knows its own topology, and each root's `Registry::subscribe()` gives its live roster). Two additive, non-wire exceptions the sketch needs — recorded here because "unchanged" as first written was false:

1. **`crates/wcode-harness/src/actor.rs` gains one additive `SessionActor::spawn_supervised(agent) -> (SessionHandle, JoinHandle<()>)`.** `spawn` keeps working and is reduced to `spawn_supervised(agent).0`, so **all 17 call sites in `wcode-cli` keep their signature**: `main.rs` 1899/2022/2036/2323/3543 · `repl.rs` 957/1098/1254/1303 · `agents.rs` 402/1108 · `tools/member.rs` ×2, `tools/spawn.rs`, `tools/background.rs` · `scheduler.rs:1347` · `session_groups.rs:858`. Supervised roots (item 4) cannot exist without the handle, which `spawn` discards at `actor.rs:273`; a dead conversation is otherwise invisible except through a failing `send` (`:161`).
2. **`wcode-protocol` gains one non-wire module, `crates/wcode-protocol/src/conversation.rs`** (`ConversationId`, `RootSpec`) and nothing else. It cannot live in the TUI (the browser must not link `ratatui`) or in the CLI (the TUI must not link the CLI), so the only crate all three parties can name it from is the protocol — which is also why it must stay free of anything serialized.

The container and the root lifecycle live in the CLI + TUI, which is what makes them *candidate* reuse for the web hub rather than web-shaped — pending finding 6 (see §13.5).

### 13.4 Open questions for the sketcher's first-layer review

- Does `Ctrl-N` get redefined, or does a new chord take conversation cycling (and `Shift-Tab` keep member cycling)?
- Where does the tab bar live — band 0 above the pinned header (D010), or inside it? `docs/tui-design.md` is the authority and a new band amends it.
- `/tab close` on the focused conversation mid-run: cancel-then-close, or refuse while `Running`?
- `/new` keeps its destructive re-exec meaning, or becomes `/tab new`'s alias? (Recommendation: keep `/new` unchanged for this item.)

---

## 14. This item's number, and the succession (authoritative)

> **Section order, for a reader who finds this out of sequence:** §13.5, §13.5.1, §13.5.2, §13.6 and §13.6.1 were appended after this section while the review ran. Read them as the tail of §13; *this* section's table remains the authority for numbering.

**This brief was written as W039 and renumbered to W040 during the sketch review, because W039 was already taken — twice.** `docs/work/W039-octopus-banner.md` was added in `c1ee923` ("tui: replace the empty-state mark with an octopus avatar banner (W039)") and removed by the reverts `0ee113e` and `e6a2d82`; separately `docs/work/W038-model-catalog-sync.md:221` records "W039 folded into W038". A number is a name, and two live meanings for one name is how a tracker stops being an index.

**The succession, and it supersedes every other numbering in this file:**

| item | what | state |
|---|---|---|
| **W040** | **the multi-root core + TUI tabs** — `RootSpec` factory, N in-process roots, scoped `Background`, supervised roots, the tab container/band/commands/keys | **this item**; sketch in first-layer review |
| W041 | the **browser hub** — `axum` HTTP + SSE front-end over `Roots`, server-rendered markdown/syntect, the security floor (§4.3, §4.8) | transport approved, deferred |
| W042 | **files** — tree, viewer, syntax highlighting, markdown render, sandboxed HTML render | not started |
| W043 | **change walkthrough** — per-run diff capture, the ordered tour | not started |
| W044 | **team timeline + orchestration graph** — one clock in the hub/front-end, the ownership edge as the graph's edge | not started |
| W045 | **control** — task dashboard (read-only plan, submit = `Request::Submit`), the settings screen over existing `Request`s, the multi-server rail | not started |

`D045` (proposed, and now awaiting one decision — §13's **finding 6**) records the transport choice; its text stays as written in §9, with `axum` substituted for the hand-rolled server per the human's call.

**Where the sketch lives** (all uncommitted, all inert — comments in tracked files plus three unregistered modules):

- `crates/wcode-protocol/src/conversation.rs` — `ConversationId`, `RootSpec` (the only thing this item promotes into a published crate; **nothing serialized**, `PROTOCOL_VERSION` stays 1).
- `crates/wcode-tui/src/conversations.rs` — the TUI container + the `ConversationEvent` vocabulary.
- `crates/wcode-cli/src/roots.rs` — `Root`, `Roots::{start,stop,close}`, `Factories`/`build_root`/`mint_surface_id`, `RootEvent`/`spawn_supervisor`, `HostChannels`/`run_host`, `install_surface_sink`.
- in-place blocks: `app.rs` (5), `ui.rs` (2), `main.rs` (2), `repl.rs` (1), `tools/background.rs` (1), all `// ==== SKETCH ====`-delimited.

---

## 13.5 The first-layer review of the sketch: findings and their status

Three passes. The first **BLOCKED** with ten findings; the second **BLOCKED** with six, of which five are text/wiring and one is the human's. Recorded here so the next reviewer has a ledger rather than a rumour.

| # | finding | status |
|---|---|---|
| 1 | the sketch was **not inert** (real `Action` variants, unregistered types, plus four `///` lines at column 0 that became real doc attributes) | **closed** — build/test/clippy green, three independent runs |
| 2 | two contradictory data models (`SurfaceInfo.conversation` vs none) | **closed** in `app.rs`, `conversations.rs` **and** `conversation.rs` (the leftover is gone) |
| 3 | member surface ids collided across conversations (every root's worker is `agent:w1`) | **closed** — minted per surface in `Root::surface_specs` *and* in `install_surface_sink` |
| 4 | the N = 1 fallback broke N = 1 (`Conversations::new` dropped members) | **closed** — `Conversations::single(&[SurfaceInfo])` |
| 5 | `Background.notify` was an `Arc` cycle: every closed conversation leaked its actor + one watcher per peer | **closed** — `release_notify`, released before the `LIVE` prune |
| 6 | **the web-reuse claim is false: `wcode-cli` is a binary-only crate** | **OPEN — the human's decision (§13.5.2)** |
| 7 | `Factories` could not receive the id `build_root` needs | **closed** — `Fn(ConversationId, RootSpec)` |
| 8 | the id mint source was double-booked (first `/tab new` re-minted `c0`) | **closed** — seq 0 reserved, `insert` refuses an occupied id |
| 9 | `RootInputs` was not the factory's input set and leaked a private `Args` | **closed** — one `repl::RunInputs`. **One deliberate deviation, accepted by the review:** `SessionSetup`'s `session`/`context`/`resuming_group` are **derived from `RootSpec`**, not carried, because a captured `Session` (a file handle) would make every `/tab new` re-open the startup conversation's transcript — a data-loss bug, not a preference (§13.6) |
| 10 | `Failed` was not sticky and `/tab close` refused the wrong condition (a running *member* read `Idle`) | **closed** — truth table + "any surface running" predicate |

### 13.5.1 The rule this produces

**Every statement of the form "the browser reuses this" is a *candidate*, conditional on §13.5.2.** The sketch's `roots.rs` header, its per-item notes and §14's W041 row all say so in those words now; a reader must not treat the split as verified until the crate question is settled. What *is* verified: the transport-free half (§§1–5 of `roots.rs` — `Root`, `Roots`, `RootError`, `RootEvent`, `RootParts`, `Factories`, `build_root`, `mint_surface_id`) names **no `wcode_tui` type**, while `§6`'s `HostChannels`/`run_host` and `§7` name two. `roots.rs`'s "nothing in §§1–6 names a `wcode_tui` type" was wrong and is corrected to §§1–5.

### 13.5.2 Finding 6 — the open decision

`crates/wcode-cli/Cargo.toml` declares `[[bin]] name = "wcode"` and there is no `src/lib.rs`, so **nothing in `roots.rs` is reachable from another crate today** — and §4.2 publishes `wcode-serve` *before* `wcode-cli`. So the question is not "does the shape fit" (it does) but **"does the shape justify promoting a crate, or duplicating the lifecycle?"** The three resolutions, unchanged from the reviewer's ruling:

- **(D) — recommended.** Move the reverse-channel payloads into `wcode-protocol` (`ConversationId`, `RootSpec`, `ConversationRequest`, `ConversationEvent`, plus a protocol-side `Surface { id, label, model, is_root, backend }` — `Backend` is already there). The CLI keeps `Roots` and services the channel; `wcode-serve` becomes a **wire-only front-end the CLI spawns**, handed one channel pair and, per conversation, `{ id, surface, backend, registry, roster }`. Keeps §4.2's crate layout and publish order, honours §13.2 item 2 literally, deletes the `HostChannels`-names-TUI-types wart. Cost: two types move; one host loop serves both front-ends.
- **(C) — cheapest.** The HTTP surface lives in the `wcode` binary as a second host loop (`wcode serve --http`). Reuse becomes trivially true. Cost: reverses §4.2 option (c)'s "the CLI is a thin client" — weaker post-pivot, since the HTTP surface is composition-root work, not rendering — and loses the fifth crate and its publish-order work.
- **(A)/(B) — largest, and (B) collapses into (A).** A `wcode-host` crate must absorb `config`/`agents`/`session_groups`/`skills`/`instructions`/`tools` (because `Root` names `Arc<Background>`, `Orchestrator`, `SessionGroup`, `Config`, `ToolsConfig`, `InstructionSet`, `SkillSet`, `Args`, `SessionSetup`), i.e. it *is* the CLI library. Cost: ~3,500 lines of `main.rs` re-homed, publish order flips to `… → host → {tui, serve} → cli`, and `wcode-serve` ends up depending on the whole CLI.

**Status: unanswered.** W040 does not depend on it; W041 does.

---

## 13.6 Decisions settled by the first-layer review

**`SessionSetup`'s `session`/`context`/`resuming_group` are derived, not carried — ACCEPTED**, with two conditions: (a) **do not** add `Fresh { resume_from: Option<PathBuf> }` — a fork of the *current* conversation is not a path (the live transcript can be ahead of `root.jsonl`, and `--no-session` has no file at all), so a path-shaped field compiles and then silently forks a stale transcript; (b) when forking is wanted it arrives as a **per-call parameter** (`RootSeed { session: Option<Session>, context: Vec<AgentMessage> }`, or a `RootSpec::Fork` carrying that pair), never a field. **Fork is deferred** because §13.2 item 7 locks `/tab` to `new | close | <n>`.

**`Ctrl-N`/`Shift-Tab` cycle *conversations*, `Alt-1..9` stay on member rows — CORRECT**, and all four tripwires survive unmodified: `backtab_moves_focus_backwards_and_wraps` (twice over — `Conversations::len() == 1` delegates to `focus_step`, *and* the N = 1 container holds all surfaces), the browse-mode `Alt-1` test, `slash_surface_and_the_cycle_key_switch_focus`, and the `KEYS`-length assertion (one new `COMMANDS` row, no new `KEYS` row). Two conditions, both now part of item 6: **at N > 1 member cycling is a capability regression** (movement becomes `Alt-1..9`, the sidebar click and `/surface`) and must be recorded as one; and `cycle_or_focus` must be a function of `conversations.len()`, with `Conversations::cycle` returning `false` at `len() <= 1`.

**The tab band — CORRECT as sketched:** first `Layout::vertical` constraint inside the bands column (the `SIDEBAR_MIN_WIDTH` precedent, applied vertically), so at N = 1 nothing is reserved and the frame is byte-identical; roles `accent`/`dim`/`muted`/`error`/`body` only (**no new role**); glyphs `▌`, `⠋…`, `✗`, `·` all already in `docs/tui-design.md:67-104` (**no new glyph**); D010 needs no amendment — the header stays the pinned top *session* band and the tab band is selection chrome above it. **`docs/tui-design.md` takes exactly four edits** (Principle 1's band list; a Tab band entry in §4 *before* Header; a §2 note that `▌` gains a third meaning; no new §3 draft), and the band's own comment list in `ui.rs`. That is `designer`'s pen, not the sketcher's.

**W040's expected outcome, replacing §8's browser-centric one:**`wcode` starts with one conversation and can open, switch, rename and close several — each an in-process root with its own transcript, input draft, model/effort chip and live event stream — with `+`/`−` visible only once there is more than one, and a closed conversation's files intact on disk for `--resume`.

---

## 15. Errata — statements in this document that are no longer true

Written as a table because silently rewriting nine sections is how a design doc starts lying. Each row names what a reader must not act on.

| where | what it says | act on this instead |
|---|---|---|
| §2.2 items 1–7, 10, 12 | the browser crate, HTTP surface, rendering and security floor are *this item's* scope | **W041**; W040's scope is §13.2 |
| §2.2 item 8 | `SessionInfo.owner` + `Registry::owner_of` are in scope | **withdrawn**; §13.3 refuses it |
| §4.1 (a)/(c) | recommends a hand-written HTTP server, rejects `axum` | the human chose **`axum`** (§14, D045); the design in §4.3 is unchanged by that choice |
| §4.2 option (c) | rejects the HTTP surface living in the CLI binary | **pending finding 6** (§13.5.2, option (C)) |
| §5 steps 1–7 | the web hub's implementation plan | **W041's** plan; W040's plan is the sketch's six deliverables |
| §8 | the expected outcome mentions `wcode serve --web` | §13.6's outcome |
| §9 Q1–Q3 | the transport, auth and `--web` questions | **W041's**; Q4/Q5 apply to W040 |
| §10 | the old four-item succession | §14's five-item table |
| §13.3 | "`wcode-harness` and `wcode-protocol` are unchanged" | corrected in place: **the wire** is unchanged; two additive non-wire exceptions |
| §14 | W041 "reuses" `Roots` | **candidate** reuse, conditional on finding 6 |

### 13.6.1 Two rulings that belong in the ledger

**Ids (finding 8): `0` is reserved for the startup conversation, and a failed build burns its id.** `Roots::new` starts minting at `1`; `insert` refuses an occupied id; a `/tab new` whose factory fails inserts nothing and does **not** return the id to the pool (monotone, never reused). That is pinned by `a_failed_build_inserts_nothing_and_burns_the_id`, and it is why the first `/tab new` cannot re-mint `c0`.

**The tab label's precedence: `agent.title()` → `RootSpec::Fresh { label }` → the cwd basename.** This replaces an earlier framing of mine ("the wart": a `/tab new` root built with `--title` sets its title through `Agent::set_title` at build time rather than `Request::SetTitle`, so no `SessionTitle` streams and the tab keeps its cwd label). The review showed the mechanics are worse than a wart — **two tabs disagree permanently**, and the band is only visible at N > 1, which is exactly when it shows — but also that it is not forced: tab 0 is already correct from the first frame because `main.rs:2094` injects `title: agent.title()` into `Options.status`. So seed `Root.label` from the **same accessor**, and one line removes the disagreement without adding a second source of truth (it is the same `Agent`). It covers every case: `--title` (set at build), `Resume` (`load_root` restores the `Title` entry, so `title()` is `Some`), auto-title (`SessionTitle` streams later and wins), and no title at all (the cwd basename is then correct). The action-reply ban is untouched — seeding from the `Agent` is not an action reply.
