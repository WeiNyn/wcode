# Sketch — VS Code extension P3: the team surface (roster · per-member surfaces · todos · plan mode)

**Status:** interface sketch, review-only. No working logic. Fillable as-is.

**Design:** `docs/vscode-extension-plan.md` §3.5 (`:IGSpz`) — the sidebar of live
members with **per-member surfaces, status, and peek/ask/stop** (`:uJp6k`); everything on
the wire — the pushed roster (`Sessions`), `with_session` to narrow (`:HKr4Z`),
`GetHistory` to hydrate any member, `MessageReceived` for inter-agent traffic (`:TwZw6`),
`Todo`, `Spawned`. Base tree = P2 (`editors/vscode`, the panel + `chat.ts` fold-in).

**Grounding (fresh anchors).**

- **The route a frame to a worker takes.** Server demux on `frame.session`:
  `server.rs:16GMy` (`session == root_id` → root), `:LGECt` (a roster session of that
  name), `:RbcAo` (the single-session fallback: `live.is_empty()` → root), `:W9GQG`
  (else `None`), `:AoQwI` (`unknown session {session}`), `:GhImE` (`handle.ask(body)`).
  The worker is in that roster: `agents.rs:4R9DN` (`id = SessionId::agent(name)`),
  `:GneBa` (`registry.register(id, handle)`), `main.rs:J8U02` (`live_roster` — the
  registry's local sessions minus the root alias).
- **What the worker's actor does with a `Submit`.** `actor.rs:xhKKa`
  (`Request::Submit { text } => run(...)`) — the SAME arm the root uses; **no
  root-only gate**. If a run is in flight the outer loop defers it:
  `actor.rs:8SGyA` ("another `Submit` | deferred, becomes the next run"), `:Dk3Bl`
  (`deferred.pop_front()` before the inbox), `:BvAaE` (`other => deferred.push_back`).
  `MemberState` is set at the run boundary — `Running` `:jLOPN`, `Done`/`Failed`
  `:rNq8T`-area (`protocol.rs:qqtfh`, "the server-side liveness fact" `:dqh1u`).
- **The verbs the repo already uses for peers.** The `member` tool —
  **status** = `Request::Status` → `AgentEvent::Status` (no model call), **ask** =
  `Request::SideAsk { text }` → `AgentEvent::SideAnswer` (a tool-free model call),
  **cancel** = `Request::Cancel` (`member.rs` `op` dispatch — status `:LBObH`, cancel `:XWK2x`; the impls below them). The `message` tool rides the delivery verbs —
  `notify|wake|interrupt` → `Request::Notify`/`Wake`/`Interrupt` (`message.rs:HaWGQ`).
  **The extension's peek/ask/stop must mirror the `member` tool, not invent verbs.**
- **Per-session addressing.** `Client::with_session(&self, session: SessionId) -> Client`
  `client.rs:u9PyK` (the doc: "`session` must be the id the server actually **serves**
  … learn it from `ListSessions`"). On the TS side today **every** request is hard-wired
  to the root: `session.ts:WteVT` (`session: this.rootId`) — a P3 change.
- **The roster is pushed, the reducer already folds it.**
  `reducer.ts:uxNek` (`sessions` → `members`), `:XYLjs` (`spawned` → append),
  `:cBKlD` (`todo` → `todos`), `:yUhyh` (`message_received` → a `notice`). `ViewState`
  already has `members`/`todos` (`:xSLIN`/`:msSgY`) and `ViewStatus.planMode` `:DqoV6`
  (declared, **never set**).
- **The TUI sidebar is the prior art.** `draw_sidebar` `ui.rs:OVnem`; one row per member
  `{glyph} {name}  {dim action}` (`team-and-tui-plan.md` §2) — `member_rows`
  `app.rs:bXdmA`, `team_text` `app.rs:ZndCd`, glyph→style `state_style` `ui.rs:tzCo1`,
  `Surface::state` `app.rs:66hxt`. The **Todos** section: `// ---- Todos` `ui.rs:5R7Lv`
  with a `Todos  ☑ {done}/{total}` header `ui.rs:YrNms`. A `Todo` event also lands as a
  transcript notice (`app.rs:UHptB`, `render_todos` `app.rs:5XxjU`).
- **`--stdio` is one client** (`docs/sketches/serve-stdio.md` §4/§7): one process = one
  session tree; members are in-process workers; a *remote* peer cannot be reached.

---

## 0. The question, answered: does a `Submit` to `agent:<name>` run a turn in that worker? — **YES**

Traced end to end (`server.rs` → `agents.rs` → `actor.rs`):

1. The server resolves `frame.session` against the **live roster** (`server.rs:16GMy`
   … `:LGECt`); a worker spawned via `SessionFactory::spawn` is `SessionId::agent(name)`
   (`agents.rs:4R9DN`) and is `registry.register`ed (`:GneBa`), so `live_roster`
   (`main.rs:J8U02`) serves it.
2. The matched handle gets `handle.ask(body)` (`server.rs:GhImE`) — the same call the
   root gets.
3. The worker's actor runs `Request::Submit { text } => run(...)` (`actor.rs:xhKKa`) —
   **there is no "root-only" arm**; `Submit` runs a turn in whatever session receives it.
4. If the worker is **busy**, the `Submit` is **deferred** to the run boundary and
   *becomes the next run* (`actor.rs:8SGyA`, `:Dk3Bl`, `:BvAaE`) — queued, never refused.

**Consequence: the sidebar CAN be a control surface** — the composer of a member's
surface can `submit` addressed to that member's id and it runs a turn there. Two caveats
the sketch must honour:

- **"Ask" the repo already means `SideAsk`, not `Submit`.** The `member` tool's `ask` is
  `Request::SideAsk` (a *side* call: no turn, not recorded — `member.rs`); a `Submit` is a
  real user turn. Both work to a worker; P3 uses `SideAsk` for **ask** (peek-like) and
  reserves `Submit`/`Wake` for the composer of the member surface.
- **`--stdio` sees one session tree.** The workers here are in-process (spawned by the
  root orchestrator); a remote peer is unreachable over stdio (`serve-stdio.md` §7).

## 1. The sidebar — a native `TreeView` (recommended), not a `WebviewViewProvider`

**Recommendation: `TreeDataProvider<RosterItem>` + `vscode.window.createTreeView`.** The
roster is small and mostly text (`name · state · action`); a tree gives native theming,
codicon icons, keyboard nav, accessibility, context-menu buttons, and selection — all
free. A `WebviewViewProvider` would re-implement that, duplicate the `render.ts` pipeline,
and lose native a11y/theme for no gain. The panel already carries the heavy rendering.

```ts
// src/roster.ts — the tree; the ONLY vscode import is the TreeItem/TreeDataProvider types.
export interface RosterItem {
  id: string;              // the session id (root-1, agent:w1) — the routing key
  label: string;           // short_name(id) — mirrors reducer.shortLabel (reducer.ts:TLDK8)
  model?: string;          // SessionInfo.model (protocol.rs:lStv9) — may be absent
  state: MemberState;      // idle|running|done|failed (protocol.ts:bmYdd)
  isRoot: boolean;
  liveAction?: string;     // NEW: "{tool} {target}" for that member's current run (see §5)
}

export class RosterProvider implements vscode.TreeDataProvider<RosterItem> {
  private items: RosterItem[] = [];
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;
  /** PUSH: called from the reducer, never polled. */
  set(items: RosterItem[]): void { this.items = items; this.changed.fire(); }
  getTreeItem(item: RosterItem): vscode.TreeItem;   // builds the TreeItem below
  getChildren(item?: RosterItem): RosterItem[];     // the top-level list only
}
```

**`TreeItem` shape** (per row):

```ts
const ti = new vscode.TreeItem(item.label, vscode.TreeItemCollapsibleState.None);
ti.description = item.liveAction ?? item.state;        // the dim right-hand text
ti.tooltip = `${item.model ?? "unknown model"} · ${item.state}`;
ti.iconPath = memberIcon(item.state);                   // a codicon + a ThemeColor
ti.contextValue = item.isRoot ? "wcode.root" : "wcode.member"; // for `when` menus
ti.command = { command: "wcode.member.focus", title: "Focus", arguments: [item.id] };
```

- **Icon + theme color** (mirror the TUI glyphs `● ○ ✓ ✗` → `ThemeIcon`s):
  `running → new ThemeIcon("circle-filled", new ThemeColor("charts.green"))`,
  `idle → "circle-outline" / "descriptionForeground"`,
  `done → "check" / "charts.blue"`, `failed → "error" / "charts.red"`.
  The TUI's glyph→style map is `ui.rs:tzCo1` + `lib.rs` `TeamState::glyph`.
- **Selection / focus:** `treeView.onDidChangeSelection(e => target = e.selection[0]?.id)`
  → retarget the panel (§2). Clicking a row fires `wcode.member.focus` (the `TreeItem.command`),
  which sets the target and reveals the panel. A newly `spawned` member is revealed via
  `treeView.reveal(item)` (not selected — do not steal focus; the TUI's rule is
  "active-first, don't steal focus" `team-and-tui-plan.md` §2 / `app.rs:bXdmA`).
- **`package.json`:** `contributes.views` under a new `viewsContainers` entry `wcode`
  (activity bar), view id `wcode.members`. The tree is created in `activate` and its
  provider fed from the reducer (§5). **Placement settled: a dedicated activity-bar
  container** (the user can drag it elsewhere), and the view must render a **deliberate empty
  state** when no session is running ("No session. Run `wcode: Start Session`.") — never a
  blank tree (the P1b rule).
- **Push, never poll.** `getChildren` returns the cached `items`; the reducer drives
  `set(...)`. No `setInterval`, no manual refresh command — a poll is a smell (§7).

## 2. The per-member surface

**Settled (D013): a panel PER TAB.** The recommendation that stood here — ONE panel
that re-targets (`panel.ts:2NpqG`) — is superseded: one workspace hosts several editor
tabs, each an independent wcode session with its own `WcodeSession` +
`SurfaceController` + `ViewState`. Within a tab the surface still re-targets its
**members** (the target stays visible in the status strip); what multiplies now is the
**tab**, not the panel's target. See
`docs/decisions/D013-multi-tab-sessions-vscode.md`.

- **Binding.** `ChatPanel` gains `private target: string`. Every send from the composer is
  addressed there — which needs `WcodeSession` to stop hard-coding the root
  (`session.ts:WteVT`): **`send(request: Request, session?: string): number`** (default =
  `rootId`), and `PanelHandlers` becomes session-aware
  (`onSubmit(text, target)`, `panel.ts:9bTic`). The extension host passes `panel.target`.
- **The target is visible in the status strip at all times (settled).** A re-targeting panel
  whose target is invisible is the worst failure mode here: the user cannot tell whose
  transcript they are reading, and will attribute one member's words to another. Render the
  member id ("orchestrator" for the root) as a `statusSegments` entry, beside the plan chip.
- **Hydration.** On target change, `GetHistory` **addressed to that member's id**
  (`next.send({ type: "get_history" }, target)`) → `AgentEvent::History { messages }` →
  `seedFromHistory` (`reducer.ts:SvgFw`) builds that member's transcript. A wrong id
  answers `unknown session {id}` (`server.rs:AoQwI`) — the P1 lesson
  (`session.ts:jDCjn`): address the id the roster names, never `"remote"`/the root.
- **In-flight state on a retarget.** Nothing is lost **iff the transcript is
  per-session** (§5): events are stamped with their origin `session` (`server.rs:Ebbvf`),
  so the reducer keys blocks by `frame.session`, and a retarget only changes *which*
  key the panel renders. With a single shared `transcript` the retarget would drop the
  old member's live block — the reason `transcript` must become per-session.
- **Two panels at once?** **Yes — settled (D013): a panel per TAB.** `ChatPanel` becomes
  one instance per tab (its `static current` singleton goes); within a single tab the
  surface still re-targets its members. Cross-tab visibility is **not** shared — a tab
  is its own `serve --stdio` child, so a worker spawned in one tab is invisible to
  another. A VS Code window reload does not yet restore tabs (a `WebviewPanelSerializer`
  is a named follow-up).

## 3. Actions — the exact request each sends

Every action is a command; all four are the **`member` tool's** verbs, not new ones
(`member.rs`). Each is addressed to the member's session id.

| command | request sent (to the member) | reply | what the user sees |
|---|---|---|---|
| `wcode.member.peek` | `Request::Status` | `AgentEvent::Status { last_assistant_text }` | a modal/inline: `[{state}] {id} · {work}` — **no model call** (`member.rs` status) |
| `wcode.member.ask` | `Request::SideAsk { text }` | `AgentEvent::SideAnswer { text, usage }` | a modal/inline answer; a *side* question, **not a turn, not recorded** (`member.rs` ask) |
| `wcode.member.stop` | `Request::Cancel` | `AgentEvent::Ack` | the row's `MemberState` goes `running→done` (watch, via the roster push) |
| `wcode.member.message` *(optional)* | `Request::Wake { content }` (or `Notify`) | `AgentEvent::MessageReceived` + a run | deliver a task/message to the member — the `message` tool's verb (`message.rs:HaWGQ`) |

- **Stop is optimistic-but-observed.** `Cancel` on an idle session is a no-op `Ack`
  (`actor.rs:SdqMr`); the UI must not claim "stopped" until `MemberState` leaves
  `running` (the `member` tool itself waits for the run to end — `CANCEL_TIMEOUT`).
- **Peek vs the transcript.** `Status` is "a lean read (one string), not the whole
  transcript" (`member.rs` header) — a peek shows the *last work*, not history; history
  is the surface's `GetHistory` (§2).
- **Plan-mode toggle** — `wcode.plan.toggle` sends `Request::SetPlanMode { on }` →
  `AgentEvent::Ack` (infallible; `protocol.rs:8tGXF`). **Where its state lives: the
  panel's status strip** (`render.ts`/`view.ts:LPae8` `statusSegments`), as a `plan`
  segment — mirroring how the TUI keeps `plan` in its status and settles an optimistic
  toggle on `Ack`, reverting on `Error` (the reducer's `ViewStatus.planMode` `:DqoV6`
  is declared for exactly this; §5 wires it). A status-bar item is the lighter
  alternative. <!-- SKETCH: TODO(prose) — strip chip vs status-bar item. -->

## 4. `MessageReceived` and `Todo` — where each renders

- **`MessageReceived` → the transcript of the session it was stamped with.** The reducer
  already folds it to `{ kind: "notice", from, text }` (`reducer.ts:yUhyh`), and
  `render.ts` carries `from` (`:Uz7SX`) — **but the webview never shows it**: `chat.ts`
  renders a notice with `blockShell(block, null)` (`:oQG0O`), i.e. no role line. Fix:
  pass the sender as the role (`blockShell(block, block.from ?? null)`) so inter-agent
  traffic reads `from: agent:w1`. Because the frame carries the member's `session`, a
  member's inbound/outbound message lands in **that member's** transcript; the root's
  own traffic lands in the root's.
- **`Todo` → the sidebar, scoped to the target member.** The todo list is a
  session-local fact (the `todo` tool), so it belongs beside the member it belongs to,
  not in a global transcript. Add a **Todos section** to the sidebar (or a second
  `TreeView`), mirroring the TUI's `// ---- Todos` section with a `Todos  ☑ {done}/{total}`
  header (`ui.rs:5R7Lv`, `:YrNms`) and `☑`/`☐` rows — **and keep the existing transcript
  label format). **Settled: the sidebar is authoritative and the transcript notice is
  REMOVED.** The model calls `todo` often, so a notice per update is noise in a long session,
  and a list duplicated in two places invites them to disagree. `todoBadge` puts the
  at-a-glance `☑ done/total` on the section header instead.

## 5. Reducer / `ViewState` additions (keep the purity discipline)

All pure — no `vscode`, no I/O — so plain node drives them (`reducer.ts` header).

```ts
// reducer.ts — additions
export interface ViewState {
  transcripts: Record<string, Block[]>;   // NEW: blocks PER session id (was one `transcript`)
  targeted: string | null;                // NEW: the focused member (a session id)
  members: SessionMember[];               // unchanged (sessions/spawned)
  todos: Record<string, TodoItem[]>;      // NEW: todos per session (was one `todos`)
  status: ViewStatus;                     // + planMode now DRIVEN (§3)
}

export interface SessionMember {          // reducer.ts:xSLIN — one NEW field
  id: string; label: string; model?: string; state: MemberState; isRoot: boolean;
  liveAction?: string;                    // NEW: "{tool} {target}" for the current run
}

/** Reduce one event into the session it belongs to (the frame's `session`). */
export function reduce(state: ViewState, event: AgentEvent, session: string): ViewState;

// New pure helpers (view-model; testable):
export function target(state: ViewState, id: string | null): ViewState;   // set `targeted`
export function memberViews(members: SessionMember[]): RosterItem[];      // for the tree
export function todoBadge(todos: TodoItem[]): string;                     // "☑ done/total"
export function planModePending(on: boolean): Partial<ViewStatus>;        // optimistic
export function planModeAck(state: ViewState): ViewState;                 // settle on Ack
export function planModeRevert(state: ViewState, prev: boolean): ViewState; // on Error
```

- **`reduce` gains the session.** Today it takes `(state, event)` (`reducer.ts:nc3Hz`);
  the caller has the frame (`extension.ts` `next.on("event", (event, frame) => …)`), so
  P3 passes `frame.session`. Every arm that touches the transcript writes
  `transcripts[session]`; `sessions`/`spawned`/`todo` keep their existing arms
  (`:uxNek`/`:XYLjs`/`:cBKlD`) but key `todos` by session.
- **`liveAction` from tool events — ALLOWED (the ruling was amended).** The roster carries no
  action field (`SessionInfo { id, model?, state }`; `MemberState` is liveness only), but this
  is derived from the **live event stream** for that member's session (`tool_execution_start`),
  not from the roster — so it is neither stale nor invented, and it mirrors the TUI
  (`team-and-tui-plan.md` §3, `app.rs:bXdmA`). **Condition: it must be cleared on `agent_end`**,
  so a finished member never shows a stale action; the row then falls back to `MemberState`.
- **Purity (corrected, second-layer review):** there are FOUR `vscode` importers —
  `extension.ts` (the composition root), `panel.ts`, `roster.ts`, `diffProvider.ts`.
  The *meaning* of the rule is that the view model and the tree's data stay pure:
  `reduce`, `memberViews`, `memberIconSpec`, `todoBadge`, `planMode*`, `HydratedSet`
  import nothing (no `vscode`, no I/O), so plain node drives them.

## 6. Tests & fixtures

```ts
// test/roster.test.ts — roster folding (pure). The committed fixture already covers growth.
it("folds a `sessions` push into members, root first", () => { todo!(); });
it("appends a `spawned` worker without disturbing the roster", () => { todo!(); }); // sessions_roster.handwritten.ndjson
it("marks exactly the first member root", () => { todo!(); });
it("a `message_received` lands in the SENDER's session transcript, not the target's", () => { todo!(); });
it("`todo` is scoped to the emitting session", () => { todo!(); });
it("plan-mode: optimistic set, settle on `ack`, revert on `error`", () => { todo!(); });
it("memberViews/memberIcon map each MemberState (idle|running|done|failed)", () => { todo!(); });
```

**Fixtures — which are captured vs hand-written** (`test/fixtures/README.md`):

- **Already covered:** `sessions_roster.handwritten.ndjson` holds a `sessions` push
  (`root-1` + `agent:w1`) **and** a `spawned` (`agent:w2`) — i.e. it covers roster
  **growth** (verified). Reuse it; only the *transcript keying* is new.
- **Capturable model-free** (extend `scripts/capture.mjs`): `sessions` (via
  `list_sessions`), `spawned`/roster-growth (via `define` on a `--agents` server — the
  handler replies `Spawned`, `server.rs`), and **`message_received`** (send a
  `notify` frame to a session — the actor emits `MessageReceived` with no model call,
  `actor.rs` `Notify` arm). These need no model.
- **Must be hand-written meanwhile:** `todo` — the `todo` tool is model-invoked, so a
  real `Todo` frame needs a model call; hand-write it from `todo.rs`/`protocol.ts`
  (as the README's `*.handwritten.ndjson` convention already does), and capture it during
  the manual pass.

## 7. Friction (do not smooth over)

- **The roster is pushed, never polled.** `Sessions`/`Spawned` arrive unsolicited
  (`server.rs` seeded push + the roster `watch`). The tree must fire
  `onDidChangeTreeData` *from the reducer* (§1); a `getChildren` poll or a manual
  refresh command is a smell and a race.
- **`GetHistory` must use *that member's* id.** A wrong id answers `unknown session`
  (`server.rs:AoQwI`); the P1 hard rule is "address the served id" (`session.ts:jDCjn`).
  The panel's `target` is the routing key; the composer must not fall back to the root.
- **`--stdio` serves one client by construction** (`serve-stdio.md` §4): one process =
  one session tree. The members are **in-process** workers the root spawned; a *remote*
  peer cannot be reached over stdio (`serve-stdio.md` §7) — the sidebar shows this tree
  only.
- **`MemberState` is server-side liveness — and nothing more.** It says
  `idle|running|done|failed` (`protocol.rs:qqtfh`, born at the run boundary
  `actor.rs:jLOPN`); it does **not** say *what* the member is doing (that is
  `liveAction`), nor its transcript, nor its todos. Do not overload it.
- **A member that appears after the panel opened** (`Spawned`): the reducer appends
  (`reducer.ts:XYLjs`) and the tree fires — no re-seed, no refetch. **A member that
  finishes** settles to `done` (sticky); the row **stays** (the roster is durable — the
  TUI keeps it too). Do not remove finished members.
- **`Submit` to a busy member is *deferred*, not concurrent** (§0): "send a task to a
  member" queues behind its current run. Say so in the UI, or it looks dropped.
- **A retarget must not lose a transcript** — the reason `transcript` becomes
  per-session (§2/§5). With the single shared list, switching members would erase the
  previous one's live block.
- **The `from` field is carried but unrendered** — the inter-agent sender is invisible
  until `chat.ts` passes it as the notice's role (§4, `:oQG0O`).
- **The `Ask` on a busy member can time out** (`member.rs` `ASK_TIMEOUT = 60s`); peek is
  answered from a turn-boundary snapshot mid-run, so it stays instant — surface a
  timeout as "busy", not as a failure.

## 8. Not verified / left to the implementer

- **The `Submit`-to-a-worker answer: the ADDRESSING is now live-verified, the TURN is
  not.** `test/routing.test.ts` drives a real `wcode serve --stdio` and shows, with no
  model, that a request to `agent:<name>` is validated against the live roster — the
  proof is the CONTROL (a bogus id refused `unknown session agent:does-not-exist`),
  while `Status` is answered and correlated, `SetPlanMode` → `Ack`, and `Define` →
  `Spawned` grows the roster. **The reply's own `session` is an ECHO** of the request
  (`server.rs` `reply_to: Some(id), session, …`), so it is not evidence on its own.
  What is still NOT observed: a real `Submit` **turn inside a worker** (it needs a
  model; it takes the same `handle.ask` path), and a busy member's `Submit`
  **deferral** (`actor.rs:8SGyA`/`:Dk3Bl`/`:BvAaE`).
- The five `TODO(prose)` decisions: sidebar container placement (§1, SETTLED: a
  dedicated activity-bar container); one-panel-retargets vs a panel-per-member (§2,
  SETTLED (D013): a panel **per tab**, each an independent session; within a tab, ONE
  panel that re-targets, with the target always visible); the plan-mode
  chip's home (§3, SETTLED: the panel's status strip); the Todos home (§4, SETTLED:
  the sidebar is authoritative — the transcript notice is REMOVED); whether
  `liveAction` lands (§5, SETTLED: yes, cleared on `agent_end`).
- I did **not** exercise the `TreeView`/`WebviewViewProvider` APIs (review-only); the
  icon/selection recommendations are API-shaped, not run.
- The `notify`-to-capture-`message_received` idea is derived from the actor's `Notify`
  arm, not captured; the `todo` model-free capture is stated as impossible, not tried.
