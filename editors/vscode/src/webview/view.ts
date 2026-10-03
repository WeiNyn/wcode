/**
 * The webview's PURE half — no `vscode`, no DOM, so plain node can drive it.
 *
 * `chat.ts` owns the DOM (createElement, scroll glue, the click handlers);
 * everything here is a plain function over the snapshot, mirroring the
 * `render.ts`-pure / `panel.ts`-impure split on the host side.
 */
import type { SessionMember } from "../reducer.ts";
import type { RenderedBlock, RenderedState } from "../render.ts";
import type { SessionState } from "../session.ts";
import type { PanelSessionInfo, SelectionContext } from "../webview.ts";

/** The FSM state as the user reads it. */
export function stateLabel(state: SessionState): string {
  switch (state) {
    case "ready":
      return "ready";
    case "starting":
      return "starting";
    case "crashed":
      return "crashed";
    default:
      return "stopped";
  }
}

/** One cell of the two-row panel header (draft `.phead .r1/.r2 > *`). */
export interface HeaderCell {
  /**
   * The full class list, composed here so the webview paints dumb (mirrors the
   * old `StatusSegment`): e.g. "dot dot-ready", "seg state", "target",
   * "pchip plan", "sep", "spacer".
   */
  className: string;
  text: string;
  /**
   * True for the `.target` chip: the DOM builds a <button> + the member menu (P3).
   * Absent/undefined = a plain <span>. The pure layer only MARKS the control;
   * the webview wires the events.
   */
  interactive?: boolean;
}

/** The two glanceable rows of the header (draft `header.phead`). */
export interface PanelHeader {
  /** `.r1`: dot · state · sep · session · sep · target-chip · plan-chip · running · error. */
  r1: HeaderCell[];
  /** draft `.r2`: model · sep · ctx · spacer · "N members". */
  r2: HeaderCell[];
}

/**
 * The header as two rows of cells (replaces the flat `statusSegments`). Pure.
 *
 * r1: a `dot dot-{session.state}`, `stateLabel(session.state)` as `seg state`,
 *     a `sep`, the session id (`seg mono`, "no session yet" when null), a `sep`,
 *     the target chip when `state.target !== null` — the `target` cell marked
 *     `interactive: true` (P3): the webview builds a <button> + the member menu,
 *     showing `❯ {label} ▾`, a
 *     `pchip plan` when `state.status.planMode`, then the running / error cells.
 * r2: the TARGET member's `model`, a `sep`, `seg ctx` (`ctx N` from
 *     `state.status.contextUsed`), a `spacer`, and `"{members.length} members"`.
 *
 * `ViewStatus.model` is DEAD (declared, never set); `SessionMember.model` IS set,
 * so the model MUST come from the TARGETED member. Effort and a context window
 * are NOT on the wire, so the header shows the model and `ctx N` only — never a
 * `▰▰▰▱▱ N / M` gauge. The draft's `⧗ 4m` cache clock is not tracked; omitted.
 *
 * `running`/`error` are KEPT even though the draft's `.phead` omits them: the
 * panel surfaced both before, and the per-target `running` is the roster's
 * liveness fact — dropped, a stalled run would be invisible.
 */
export function panelHeader(state: RenderedState, session: PanelSessionInfo): PanelHeader {
  const r1: HeaderCell[] = [
    { className: `dot dot-${session.state}`, text: "" },
    { className: "seg state", text: stateLabel(session.state) },
    { className: "sep", text: "·" },
    { className: "seg mono", text: session.id ?? "no session yet" },
  ];
  if (state.target !== null) {
    r1.push(
      { className: "sep", text: "·" },
      { className: "target", text: `❯ ${state.target.label} ▾`, interactive: true },
    );
  }
  if (state.status.planMode) {
    r1.push({ className: "pchip plan", text: "plan" });
  }
  if (state.status.running) {
    r1.push({ className: "sep", text: "·" }, { className: "seg running", text: "running…" });
  }
  if (state.status.lastError) {
    r1.push({ className: "sep", text: "·" }, { className: "seg error", text: state.status.lastError });
  }

  const r2: HeaderCell[] = [];
  const model = state.members.find((member) => member.id === state.target?.id)?.model;
  if (model !== undefined && model !== "") {
    r2.push({ className: "seg mono", text: model });
  }
  if (typeof state.status.contextUsed === "number") {
    if (r2.length > 0) r2.push({ className: "sep", text: "·" });
    r2.push({ className: "seg ctx", text: `ctx ${state.status.contextUsed}` });
  }
  r2.push({ className: "spacer", text: "" });
  r2.push({ className: "seg", text: `${state.members.length} members` });

  return { r1, r2 };
}

/** Which empty state to show (the transcript is empty). */
export type EmptyKind = "crashed" | "starting" | "stopped" | "working" | "idle" | "no-session";

export function emptyKind(state: RenderedState, session: PanelSessionInfo): EmptyKind {
  if (session.state === "crashed") return "crashed";
  if (session.state === "starting") return "starting";
  // No id means the child never handed us a root session — "No session.", not
  // "Stopped" (a session that had run and then exited keeps its id).
  if (session.id === null) return "no-session";
  if (session.state === "stopped") return "stopped";
  return state.status.running ? "working" : "idle";
}

/**
 * The content element's class list (draft `.body` / `details.fold.tool`).
 * `expanded` is only meaningful for a tool block — it is the fold's `open`.
 */
export function classNames(block: RenderedBlock, expanded = false): string {
  switch (block.kind) {
    case "tool":
      return `fold tool${block.tool?.isError ? " error" : ""}${expanded ? " open" : ""}`;
    case "assistant":
      return `body${block.live ? " live" : ""}`;
    case "user":
      return "body";
    case "notice":
      return "body notice";
    case "error":
      return "body error";
    case "btw":
      return "body btw";
    default:
      return "body";
  }
}

/** ONE transcript turn (draft `.turn` = grid `rail | content`). */
export interface Turn {
  /** → `.turn.{role}`; `err` = a turn containing an error block (draft `.turn.err`). */
  role: "you" | "wcode" | "err";
  /** → the `.who` line (draft `.who.you` / `.who.wcode`). */
  who: { className: string; avatar: string; name: string };
  /** The blocks of this turn, in order. */
  blocks: RenderedBlock[];
}

/**
 * Group `blocks` into turns (draft `.transcript` children). Pure.
 *
 * A `user` block opens a "you" turn holding just it; every following
 * `assistant`/`tool`/`notice`/`btw` block accumulates into ONE "wcode" turn
 * (draft: thinking + tools + reply are one unit). Leading non-user blocks open a
 * "wcode" turn. A turn containing an `error` block is role "err". The draft's
 * `.who .stamp` ("09:41") has NO source on the wire — omitted.
 */
export function turns(blocks: RenderedBlock[]): Turn[] {
  const grouped: Turn[] = [];
  for (const block of blocks) {
    const last = grouped[grouped.length - 1];
    const opensYou = block.kind === "user";
    if (opensYou || last === undefined || last.role === "you") {
      grouped.push({ role: opensYou ? "you" : "wcode", who: who(opensYou), blocks: [block] });
    } else {
      last.blocks.push(block);
    }
  }
  for (const turn of grouped) {
    if (turn.role !== "you" && turn.blocks.some((block) => block.kind === "error")) {
      turn.role = "err";
    }
  }
  return grouped;
}

function who(isYou: boolean): Turn["who"] {
  return isYou
    ? { className: "who you", avatar: "Y", name: "you" }
    : { className: "who wcode", avatar: "❯", name: "wcode" };
}

/** One empty/error state card (draft States `.statecard`). */
export interface EmptySpec {
  /** → `.empty-title`. */
  title: string;
  /** → `.empty-sub`. */
  sub: string;
  /** Prefix the title with the `⠋` `.spin` (the "starting" state). */
  spin: boolean;
  /** Render `session.stderrTail` in a `.stderr` block (crashed only). */
  stderr: boolean;
}

/**
 * The copy for one empty/error state — moved OUT of the DOM so plain node can
 * assert it. Pure. Titles/subs are the draft's States screen. There is no
 * `label`: the live panel shows ONE centered card, so the draft's upper-case
 * `.sc-head` heading is gallery-only chrome.
 *
 *   crashed    -> "wcode crashed.",  'Run "wcode: Restart" to try again.', stderr:true
 *   starting   -> "Starting wcode…", "Spawning wcode serve --stdio.", spin:true
 *   working    -> "Working…", "The first token has not arrived. The idle timeout is 60s."
 *   idle       -> "No messages yet.", "Type below, or press / for commands and @ to mention a file."
 *   no-session -> "No session.", 'Run "wcode: Start Session".'
 *   stopped    -> "wcode is stopped.", 'Run "wcode: Start Session" to resume.'
 */
export function emptySpec(kind: EmptyKind): EmptySpec {
  switch (kind) {
    case "crashed":
      return { title: "wcode crashed.", sub: 'Run "wcode: Restart" to try again.', spin: false, stderr: true };
    case "starting":
      return { title: "Starting wcode…", sub: "Spawning wcode serve --stdio.", spin: true, stderr: false };
    case "working":
      return {
        title: "Working…",
        sub: "The first token has not arrived. The idle timeout is 60s.",
        spin: false,
        stderr: false,
      };
    case "idle":
      return {
        title: "No messages yet.",
        sub: "Type below, or press / for commands and @ to mention a file.",
        spin: false,
        stderr: false,
      };
    case "no-session":
      return { title: "No session.", sub: 'Run "wcode: Start Session".', spin: false, stderr: false };
    case "stopped":
      return { title: "wcode is stopped.", sub: 'Run "wcode: Start Session" to resume.', spin: false, stderr: false };
  }
}

/**
 * Count the `+`/`-` BODY lines of ONE wcode presentation diff → the `+N −M` in
 * the tool head (draft `.tmeta .add` / `.tmeta .del`). Pure.
 *
 * A wcode diff is EXACTLY one hunk (`diff.ts` is the spec): line 0 is
 * `@@ -a,b +c,d @@`; body lines are prefixed ` `/`-`/`+`; the generator NEVER
 * emits a `---`/`+++` header, so a `-`/`+` prefix is always a real count. Skip
 * line 0; count `+` → added, `-` → removed. The `… (+N more lines)` truncation
 * tail starts with `…` and is NOT counted — a LOWER BOUND when the diff is
 * truncated. Returns `{ added: 0, removed: 0 }` for "" or a non-hunk.
 */
export function diffStat(diff: string): { added: number; removed: number } {
  const lines = diff.split("\n");
  if (lines.length === 0 || !lines[0].startsWith("@@")) return { added: 0, removed: 0 };
  let added = 0;
  let removed = 0;
  for (const line of lines.slice(1)) {
    if (line.startsWith("+")) added += 1;
    else if (line.startsWith("-")) removed += 1;
  }
  return { added, removed };
}
/**
 * The composer's non-text controls. Pure.
 *   mode  = `state.status.planMode` ? "Plan" : "Act"
 *   model = the TARGETED member's `model` — the SAME lookup `panelHeader`'s r2 does
 *           (`SessionMember.model` IS set; `ViewStatus.model` is dead) — or null.
 * Effort and a context WINDOW are not on the wire, so neither appears here
 * (plan §6 — the gauge and the model picker are deferred).
 */
export interface ComposerControls {
  /** `Mode: Plan` / `Mode: Act` — reflects `status.planMode`. */
  mode: "Plan" | "Act";
  /** The TARGETED member's model, or null (READ-ONLY; no picker). */
  model: string | null;
}

export function composerControls(state: RenderedState): ComposerControls {
  const model = state.members.find((member) => member.id === state.target?.id)?.model;
  return {
    mode: state.status.planMode ? "Plan" : "Act",
    model: model !== undefined && model !== "" ? model : null,
  };
}

/**
 * The composer chip text AND the submit prefix for one selection. Pure — ONE
 * function, ONE form, so the RENDERED chip and the SUBMITTED reference are
 * IDENTICAL (no drift between what you see and what is sent):
 *   `@{path}#L{startLine}-{endLine}`   e.g. `@src/panel.ts#L88-104`
 * 1-based inclusive; a single line prints `#L12-12`. The `L` is kept and
 * JUSTIFIED: it matches VS Code's own `path#L12` link syntax, so `#L88-104`
 * reads as a line range at a glance; the draft chip drops the `L`, but chip==ref
 * is worth more than matching a prototype's decoration.
 *
 * The reference is OUR text convention — `Submit{text}` is a free string the
 * model reads, so neither the host nor the CLI parses it.
 */
export function selectionRef(context: SelectionContext): string {
  return `@${context.path}#L${context.startLine}-${context.endLine}`;
}

/**
 * The text actually submitted: the reference line, a blank line, then the user's
 * text when a context is attached; else the text verbatim. Pure.
 *   `@src/panel.ts#L88-104\n\n<text>`
 */
export function composeSubmit(text: string, context: SelectionContext | null): string {
  return context === null ? text : `${selectionRef(context)}\n\n${text}`;
}
/** One live subagent row (draft `.running`). */
export interface WorkingRow {
  id: string;
  /** Display name; the ROOT reads as "orchestrator" (mirrors `reducer.displayLabel`). */
  name: string;
  /** True when the row's member is `state === "running"` (→ `.g-run`). */
  running: boolean;
  /** The dim right-hand text (draft `.running .what`) — the member's `liveAction`. */
  action: string;
}

/** The live "N members working" box (draft `.group`). */
export interface WorkingGroup {
  count: number;
  rows: WorkingRow[];
}

/**
 * The members OTHER than the target that are active (draft `.group`). Pure, over
 * `RenderedState.members` — the roster push and the `liveAction` events already
 * reach the panel.
 *
 * A member is INCLUDED iff `id !== targetId` AND (`state === "running"` OR
 * `liveAction !== undefined`). Order follows `members` (roster order, root first).
 *   name    = member.isRoot ? "orchestrator" : member.label
 *   running = member.state === "running"   (a row always renders `●`; this picks
 *             `.g-run` / `.g-idle`; the animated `⠋` lives only in the `.ghead`)
 *   action  = member.liveAction ?? ""      (empty ⇒ the DOM shows just the name)
 *   count   = rows.length
 *
 * `liveAction` is CLEARED on `agent_end`, so a finished member drops out — correct,
 * and why the predicate is NOT widened to done/failed. A `running` member that has
 * not emitted a tool call yet has `action === ""`.
 */
export function workingGroup(members: SessionMember[], targetId: string | null): WorkingGroup {
  const rows: WorkingRow[] = [];
  for (const member of members) {
    if (member.id === targetId) continue;
    if (member.state !== "running" && member.liveAction === undefined) continue;
    rows.push({
      id: member.id,
      name: member.isRoot ? "orchestrator" : member.label,
      running: member.state === "running",
      action: member.liveAction ?? "",
    });
  }
  return { count: rows.length, rows };
}

/** A manual override of a tool fold, recorded for the PHASE it was made in. */
export interface FoldOverride {
  /** The tool's `done` value at the moment of the toggle. */
  done: boolean;
  /** The open state the user chose for that phase. */
  open: boolean;
}

/** Per-callId tool-fold overrides. */
export type FoldOverrides = ReadonlyMap<string, FoldOverride>;

/**
 * The open state of a TOOL fold. Pure.
 *   - an override MADE IN THIS PHASE (`o.done === done`) WINS;
 *   - otherwise the default: open while RUNNING (`!done`), collapsed when done.
 * So `new Map()` yields `true` while running and `false` once done, and an override
 * silently EXPIRES at the running -> done transition (the "next state change").
 */
export function foldOpen(overrides: FoldOverrides, callId: string, done: boolean): boolean {
  const override = overrides.get(callId);
  return override !== undefined && override.done === done ? override.open : !done;
}

/**
 * Flip a tool fold and RECORD the user's choice for the CURRENT phase (a NEW map; the
 * input is untouched). Pure. `done` is the tool's current flag.
 */
export function toggleFold(overrides: FoldOverrides, callId: string, done: boolean): Map<string, FoldOverride> {
  const next = new Map(overrides);
  next.set(callId, { done, open: !foldOpen(overrides, callId, done) });
  return next;
}
