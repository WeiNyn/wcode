/**
 * The webview's PURE half — no `vscode`, no DOM, so plain node can drive it.
 *
 * `chat.ts` owns the DOM (createElement, scroll glue, the click handlers);
 * everything here is a plain function over the snapshot, mirroring the
 * `render.ts`-pure / `panel.ts`-impure split on the host side.
 */
import type { MemberState } from "../protocol.ts";
import { memberGlyph, type RosterItem, type SessionMember } from "../reducer.ts";
import type { RenderedBlock, RenderedState } from "../render.ts";
import type { SessionState } from "../session.ts";
import type { PanelSessionInfo, SelectionContext, ViewMode } from "../webview.ts";

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

/** One cell of the ONE-row panel header (draft `.bar > *`). */
export type HeaderCell = IdentCell | SegCell | SpacerCell | MeterCell | MoreCell;

/** The identity cell (draft `.ident`): the state glyph, the chip, the state word. */
export interface IdentCell {
  kind: "ident";
  /** The state glyph: `⠋` running · `✓` done · `✗` failed · `○` idle. */
  glyph: string;
  /** The glyph's class list: `glyph g-run` (plus ` spin` while running). */
  glyphClass: string;
  /** The full state, carried on the glyph's `title`/`aria-label`. */
  stateTitle: string;
  /** The state WORD — only starting/stopped/crashed; "" for a healthy session. */
  word: string;
  /** The target chip's text (`❯ {label} ▾`), or null when there is no target. */
  chip: string | null;
  /** All mode hides the chip (the merged transcript is nobody's); KEPT in the DOM. */
  chipHidden: boolean;
}

/** The All / Focus segmented control (draft `.seg`) — the ONE mode control. */
export interface SegCell {
  kind: "seg";
  mode: ViewMode;
}

/** Layout: the flexible gap between the identity and the meter. */
export interface SpacerCell {
  kind: "spacer";
}

/** The context meter — the TEXT `ctx N`, or the gauge `▰▰▰▱▱ 42k / 200k` when the window is known. */
export interface MeterCell {
  kind: "meter";
  /** The plain numerator-only text (`ctx 42k`) — the form used when the window is unknown. */
  text: string;
  /** The gauge parts, when the TARGET member's window is known (`▰▰▰▱▱ 42k / 200k`). */
  gauge?: { filled: string; empty: string; text: string };
}

/** The `▾` disclosure (draft `.more`): session + model behind the ONE popover. */
export interface MoreCell {
  kind: "more";
  /** The popover rows, in order: session · model · effort (each omitted when unknown). */
  rows: Array<{ key: string; value: string }>;
}

/** The ONE glanceable row of the header (draft `header.bar`). */
export interface PanelHeader {
  cells: HeaderCell[];
}

/** `42000` -> `"42k"`, `1500` -> `"1.5k"`, `2_000_000` -> `"2M"`; `<1000` verbatim. Pure. */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n < 1000) return `${Math.trunc(n)}`;
  const scaled = (value: number, suffix: string): string =>
    `${value < 10 ? Number(value.toFixed(1)) : Math.round(value)}${suffix}`;
  return n < 1_000_000 ? scaled(n / 1000, "k") : scaled(n / 1_000_000, "M");
}

/**
 * The `▰▰▰▱▱` bar for `used` of `window`, `cells` wide (draft `.meter.gauge`). Pure.
 * The ratio is CLAMPED to [0,1] (an over-run pins the bar full); a non-positive window
 * paints all-empty rather than NaN. `▰` is the filled run, `▱` the empty one.
 */
export function gauge(used: number, window: number, cells = 5): string {
  if (cells <= 0) return "";
  const ratio = window > 0 && Number.isFinite(used) ? Math.min(1, Math.max(0, used / window)) : 0;
  const filled = Math.round(ratio * cells);
  return "▰".repeat(filled) + "▱".repeat(cells - filled);
}

/** The ROOT member — the merged transcript's identity (roster order, root first). Pure. */
function rootMember(state: RenderedState): SessionMember | undefined {
  return state.members.find((member) => member.isRoot) ?? state.members[0];
}

/** The member the header identifies: the TARGET in Focus, the ROOT in All. Pure. */
function identityMember(state: RenderedState, mode: ViewMode): SessionMember | undefined {
  if (mode === "all") return rootMember(state);
  return state.members.find((member) => member.id === state.target?.id) ?? rootMember(state);
}

/** The identity glyph for one session + member, in the TUI's locked vocabulary. Pure. */
function identityGlyph(
  session: PanelSessionInfo,
  member: SessionMember | undefined,
): { glyph: string; className: string; stateTitle: string; spin: boolean } {
  // The session FSM owns the terminal states; `ready` falls through to the member's
  // own liveness. Either way the glyph IS `memberGlyph`'s (`reducer.ts:RNWdV`).
  const state: MemberState =
    session.state === "crashed"
      ? "failed"
      : session.state === "starting"
        ? "running"
        : session.state === "stopped"
          ? "idle"
          : member?.state ?? "idle";
  const { glyph, className } = memberGlyph(state);
  return {
    glyph,
    className,
    // The title spells out the state: the session vocabulary for the terminal words, but
    // the MEMBER's own liveness when the session is merely `ready` (so an idle glyph and
    // a running glyph do not read the same).
    stateTitle: session.state === "ready" ? state : stateLabel(session.state),
    spin: state === "running",
  };
}

/**
 * The header as ONE row of cells (draft `.bar`). Pure.
 *
 * In order: (1) the IDENTITY — the target's (Focus) or the root's (All) state glyph,
 * plus the target chip (Focus only; `chipHidden` in All); (2) the All/Focus `.seg`;
 * (3) a spacer; (4) the context meter — the gauge `▰▰▰▱▱ 42k / 200k` (monochrome) when the
 * TARGET member's window is known, else the numerator-only `ctx 42k`; (5) the `▾`
 * disclosure, whose rows are the session id, the model, and the effort when known. The
 * state WORD survives only for starting/stopped/crashed, and
 * a `crashed` session reads `✗` (never a recoloured spinner).
 */
export function panelHeader(state: RenderedState, session: PanelSessionInfo, mode: ViewMode): PanelHeader {
  const member = identityMember(state, mode);
  const glyph = identityGlyph(session, member);
  const cells: HeaderCell[] = [
    {
      kind: "ident",
      glyph: glyph.glyph,
      glyphClass: `glyph ${glyph.className}${glyph.spin ? " spin" : ""}`,
      stateTitle: glyph.stateTitle,
      word: session.state === "ready" ? "" : stateLabel(session.state),
      chip: state.target !== null ? `❯ ${state.target.label} ▾` : null,
      chipHidden: mode === "all",
    },
    { kind: "seg", mode },
    { kind: "spacer" },
  ];
  // (4) The context meter — the TEXT `ctx N` (the window is not on the wire).
  if (typeof state.status.contextUsed === "number") {
    const used = state.status.contextUsed;
    const window = member?.contextWindow;
    const text = `ctx ${formatTokens(used)}`;
    if (window !== undefined && window > 0) {
      // The gauge does NOT exist without a window (never `▰▰▰▱▱ 42 / ?`).
      const bar = gauge(used, window);
      const cut = bar.indexOf("▱");
      cells.push({
        kind: "meter",
        text,
        gauge: {
          filled: cut < 0 ? bar : bar.slice(0, cut),
          empty: cut < 0 ? "" : bar.slice(cut),
          text: `${formatTokens(used)} / ${formatTokens(window)}`,
        },
      });
    } else {
      cells.push({ kind: "meter", text });
    }
  }
  // (5) The ▾ disclosure: session + model (the effort row is omitted while absent).
  const rows: Array<{ key: string; value: string }> = [{ key: "session", value: session.id ?? "no session yet" }];
  const model = member?.model;
  if (model !== undefined && model !== "") rows.push({ key: "model", value: model });
  const effort = member?.effort;
  if (effort !== undefined && effort !== "") rows.push({ key: "effort", value: effort });
  cells.push({ kind: "more", rows });
  return { cells };
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
    case "peer":
      return "body peer";
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

/** The tool output card's head text — `⚙ {tool} · output` (V12, §1.5e). Pure. */
export function outputHead(tool: string): string {
  return `⚙ ${tool} · output`;
}

/** Which L2a card a tool fold's body is: the `.review` (a diff-bearing tool) or the output
 *  card. EXACTLY ONE body per fold; the fold itself stays L1. Pure. */
export function bodyKind(tool: { hasDiff: boolean }): "review" | "output" {
  return tool.hasDiff ? "review" : "output";
}

/** A tool row's state (V13): EXACTLY one of three — a tool block is always running or
 *  done, so there is no idle tool. The colour budget is spent on the two EXCEPTIONS. */
export type ToolState = "running" | "error" | "complete";

/** Which of the three states a tool row is in. Pure. */
export function toolState(tool: { done: boolean; isError: boolean }): ToolState {
  if (!tool.done) return "running";
  return tool.isError ? "error" : "complete";
}

/** The meta slot's content for a tool row (V13). `className` is the status colour class
 *  (`run` / `fail`); a COMPLETE tool carries NO glyph and NO word — the norm is quiet
 *  (its meta is the `+N −M · 38ms` stat instead). Pure. */
export interface ToolStatus {
  className: "run" | "fail" | null;
  /** The locked-vocabulary glyph (`⠋` running, `✗` failed); "" for a complete tool. */
  glyph: string;
  /** The word beside the glyph (`running…` / `failed`); "" for a complete tool. */
  word: string;
}

/** The status descriptor for a tool row, off the member-state vocabulary (`memberGlyph`).
 *  Pure — the ONE place the running/error glyph + word are decided. */
export function toolStatus(tool: { done: boolean; isError: boolean }): ToolStatus {
  switch (toolState(tool)) {
    case "running":
      return { className: "run", glyph: memberGlyph("running").glyph, word: "running…" };
    case "error":
      return { className: "fail", glyph: memberGlyph("failed").glyph, word: "failed" };
    case "complete":
      return { className: null, glyph: "", word: "" };
  }
}

/** ONE transcript turn (draft `.turn` = grid `rail | content`). */
export interface Turn {
  /** → `.turn.{role}`; `err` = a turn containing an error block (draft `.turn.err`). */
  role: "you" | "wcode" | "err";
  /** → the `.who` line (draft `.who.you` / `.who.wcode`). */
  who: { className: string; avatar: string; name: string };
  /** The member's swatch, carried onto the turn so the rail takes its colour. */
  swatch?: MemberSwatch;
  /** The blocks of this turn, in order. */
  blocks: RenderedBlock[];
}

/**
 * Group `blocks` into turns (draft `.transcript` children). Pure.
 *
 * A `user` block opens a "you" turn holding just it; every following
 * `assistant`/`tool`/`notice`/`btw` block accumulates into ONE "wcode" turn. A turn ALSO
 * starts on an `origin` CHANGE (All mode's merged blocks: an origin change IS a run
 * boundary, so every turn has exactly one origin and first-block labelling is correct).
 * A turn containing an `error` block is role "err". `memberOf` (All mode) resolves an origin
 * to the member that labels the turn; absent ⇒ the current `who("wcode")`.
 */
export function turns(
  blocks: RenderedBlock[],
  memberOf?: (origin: string | undefined) => TurnMember | undefined,
  fallbackSwatch?: MemberSwatch,
): Turn[] {
  const grouped: Turn[] = [];
  for (const block of blocks) {
    const last = grouped[grouped.length - 1];
    const lastKind = last?.blocks[last.blocks.length - 1]?.kind;
    const opensYou = block.kind === "user";
    const opensPeer = block.kind === "peer";
    // A NEW turn starts on a `user`/`peer` block, right after one, OR on an ORIGIN
    // change (the merged run boundary) — so the timeline reads as discrete messages.
    const startsRun = last === undefined || last.blocks[last.blocks.length - 1]?.origin !== block.origin;
    if (opensYou || opensPeer || last === undefined || lastKind === "user" || lastKind === "peer" || startsRun) {
      // A peer block is labeled by its SENDER (`from`); everything else by its origin.
      const member = memberOf?.(opensPeer ? block.from : block.origin);
      grouped.push({
        role: opensYou ? "you" : "wcode",
        who: who(opensYou, member, fallbackSwatch),
        swatch: opensYou ? undefined : member?.swatch ?? fallbackSwatch,
        blocks: [block],
      });
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

/** The member a turn is labeled by — the All-mode resolver's answer, or Focus's fallback. */
export interface TurnMember {
  name: string;
  isRoot: boolean;
  swatch: MemberSwatch;
}

/** The `.who` line: you / the root's ❯ / a member's initial, tinted with its swatch. Pure. */
function who(isYou: boolean, member: TurnMember | undefined, fallback?: MemberSwatch): Turn["who"] {
  if (isYou) return { className: "who you", avatar: "Y", name: "you" };
  const swatch = member?.swatch ?? fallback;
  const sw = swatch === undefined ? "" : ` sw-${swatch}`;
  if (member === undefined || member.isRoot) {
    return { className: `who wcode${sw}`, avatar: "❯", name: member?.name ?? "wcode" };
  }
  return { className: `who wcode${sw}`, avatar: member.name.charAt(0).toUpperCase(), name: member.name };
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
 * The composer's control set (draft `.composer`). Pure.
 *   mode     = `state.status.planMode` ? "Plan" : "Act"  (the `.seg`'s pressed segment)
 *   segments = the mode `.seg` — Act then Plan, exactly one pressed
 *   stop     = `status.running`  (Show `Stop`? Conditional chrome: Esc already cancels.)
 * The `Model: …` span is GONE (the model lives in the header's `▾` disclosure, V6), so
 * there is no `model` field to render. Effort and a context window are not on the wire.
 */
export interface ComposerControls {
  /** The mode — the pressed segment of the mode `.seg`. */
  mode: "Plan" | "Act";
  /** The mode `.seg`'s segments, in order; EXACTLY one is pressed. */
  segments: Array<{ mode: "Plan" | "Act"; pressed: boolean }>;
  /** Show `Stop`? ONLY while a run is in flight — an idle Stop button is chrome. */
  stop: boolean;
}

export function composerControls(state: RenderedState): ComposerControls {
  const mode = state.status.planMode ? "Plan" : "Act";
  return {
    mode,
    segments: [
      { mode: "Act", pressed: mode === "Act" },
      { mode: "Plan", pressed: mode === "Plan" },
    ],
    stop: state.status.running,
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
/** One live subagent row — the working pill's source (draft `.pill`). */
export interface WorkingRow {
  id: string;
  /** Display name; the ROOT reads as "orchestrator" (mirrors `reducer.displayLabel`). */
  name: string;
  /** True when the row's member is `state === "running"` (→ `.g-run`). */
  running: boolean;
  /** The dim right-hand text (the member's `liveAction`). */
  action: string;
  /** The member's identity swatch — the avatar's fill (roster order). */
  swatch: MemberSwatch;
  /** The avatar glyph: the root's `❯`, else the initial (`memberInitial`). */
  initial: string;
}

/** The aggregate the working pill renders: WHO is working (other than the target), and how many. */
export interface WorkingGroup {
  count: number;
  rows: WorkingRow[];
}

/**
 * The members OTHER than the target that are ACTIVE — the working pill's source. Pure, over
 * `RenderedState.members` — the roster push and the `liveAction` events already reach the panel.
 *
 * A member is INCLUDED iff `id !== targetId` AND (`state === "running"` OR
 * `liveAction !== undefined`). Order follows `members` (roster order, root first).
 *   name    = member.isRoot ? "orchestrator" : member.label
 *   running = member.state === "running"
 *   action  = member.liveAction ?? ""      (empty ⇒ the DOM shows just the name)
 *   swatch  = memberSwatch(index, isRoot)   (the avatar's identity fill)
 *   initial = memberInitial(name, isRoot)   (the root's ❯, else the initial)
 *   count   = rows.length
 *
 * `liveAction` is CLEARED on `agent_end`, so a finished member drops out — correct,
 * and why the predicate is NOT widened to done/failed. A `running` member that has
 * not emitted a tool call yet has `action === ""`.
 */
export function workingGroup(members: SessionMember[], targetId: string | null): WorkingGroup {
  const rows: WorkingRow[] = [];
  members.forEach((member, index) => {
    if (member.id === targetId) return;
    if (member.state !== "running" && member.liveAction === undefined) return;
    const name = member.isRoot ? "orchestrator" : member.label;
    rows.push({
      id: member.id,
      name,
      running: member.state === "running",
      action: member.liveAction ?? "",
      swatch: memberSwatch(index, member.isRoot),
      initial: memberInitial(name, member.isRoot),
    });
  });
  return { count: rows.length, rows };
}

/**
 * The overlapping avatar stack: the first `cap` rows, plus the overflow count (the 4th
 * and beyond collapse into ONE neutral `+N`). Pure.
 */
export function avatarStack(rows: WorkingRow[], cap = 3): { shown: WorkingRow[]; overflow: number } {
  return { shown: rows.slice(0, cap), overflow: Math.max(0, rows.length - cap) };
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

/* ------------------------------------------------------- member swatches (rail) */

/**
 * The rail's member palette: one identifiable colour per member, so a row (or a
 * collapsed avatar) reads as a specific teammate at a glance. VS Code's own
 * `charts-*` tokens, so it follows the host theme; the root always takes the first.
 */
export const MEMBER_SWATCHES = ["blue", "green", "orange", "purple", "yellow", "red"] as const;

export type MemberSwatch = (typeof MEMBER_SWATCHES)[number];

/**
 * The colour for the member at roster `index`. Pure. The root takes the first
 * (accent-adjacent blue — it is `members[0]`), then the palette cycles by POSITION:
 * distinct for up to six members and stable across a render, unlike a hash that can
 * collide. The swatch is IDENTITY; the row's border carries STATE (`--st`).
 */
export function memberSwatch(index: number, isRoot: boolean): MemberSwatch {
  if (isRoot) return MEMBER_SWATCHES[0];
  return MEMBER_SWATCHES[index % MEMBER_SWATCHES.length];
}

/** One EXPANDED rail row (draft `.roster li`): the `--sw` identity edge + the STATE glyph. */
export interface RosterRow {
  id: string;
  /** → the `<li>`'s class list: the `--sw` identity swatch, plus `sel` on the target. */
  className: string;
  /** → the state glyph's class (`g-run` / `g-done` / `g-err` / `g-idle`); its colour is STATE. */
  glyphClass: string;
  /** → the state glyph (`⠋ ✓ ✗ ○`). */
  glyph: string;
  name: string;
  /** The dim text beside the name (the member's model, else "root"). */
  meta: string;
  /** The dim action line (`liveAction`, else the state word). */
  action: string;
}

/**
 * The rail rows, ready to paint (draft `.roster`). Pure. Identity is the `--sw` edge colour;
 * STATE is the glyph's colour — the SAME language the transcript turn rail speaks. The 18px
 * `.mav` swatch box is GONE from the EXPANDED row; it survives in the collapsed strip
 * (`.sc`) and the working pill (`.pill .mav`), where a chip needs a fill.
 */
export function rosterRows(rows: RosterItem[], targetId: string | null): RosterRow[] {
  return rows.map((row, index) => {
    const glyph = memberGlyph(row.state);
    return {
      id: row.id,
      className: `sw-${memberSwatch(index, row.isRoot)}${row.id === targetId ? " sel" : ""}`,
      glyphClass: glyph.className,
      glyph: glyph.glyph,
      name: row.label,
      meta: row.model ?? (row.isRoot ? "root" : ""),
      action: row.liveAction ?? row.state,
    };
  });
}

/** The avatar glyph: the root's `❯`, else the name's initial. Mirrors `who()`. Pure. */
export function memberInitial(label: string, isRoot: boolean): string {
  return isRoot ? "❯" : label.charAt(0).toUpperCase();
}
