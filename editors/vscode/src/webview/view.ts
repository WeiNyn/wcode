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
export type HeaderCell = IdentCell | SpacerCell | MeterCell | ModeCell;

/** The identity cell (draft `.ident`): the state glyph + the member NAME (a byline, not a chip). */
export interface IdentCell {
  kind: "ident";
  /** The state glyph: `⠋` running · `✓` done · `✗` failed · `○` idle. */
  glyph: string;
  /** The glyph's class list: `glyph g-run` (plus ` spin` while running). */
  glyphClass: string;
  /** The full state, carried on the glyph's `title`/`aria-label`. */
  stateTitle: string;
  /** The member's display name — the identity is a byline now, NOT a target chip. */
  name: string;
}

/** Layout: the flexible gap between the identity and the status line. */
export interface SpacerCell {
  kind: "spacer";
}

/** The mode text toggle (A1): the quiet All/Focus control, in the `.status` line. */
export interface ModeCell {
  kind: "mode";
  /** The current `ViewMode`; → the toggle's pressed state. */
  mode: ViewMode;
}

/** The `.status` line (draft `.status`): the state WORD + the plain `ctx N`. */
export interface MeterCell {
  kind: "meter";
  /** The state WORD — only starting/stopped/crashed; "" for a healthy session. */
  word: string;
  /** The plain numerator-only text (`ctx 42k`), or "" when the count is unknown. */
  text: string;
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
 * The masthead as ONE row of cells (draft `.masthead .row1`). Pure.
 *
 * In order: (1) the IDENTITY — the target's (Focus) or the root's (All) state glyph +
 * its display NAME; (2) a spacer; (3) the `.status` — the state WORD (only
 * starting/stopped/crashed) and the plain `ctx N`; (4) the mode text toggle (`ModeCell`).
 * No chip, no All/Focus `.seg`, no gauge, no `▾` disclosure (all dropped, W005).
 */
export function panelHeader(state: RenderedState, session: PanelSessionInfo, mode: ViewMode): PanelHeader {
  const member = identityMember(state, mode);
  const glyph = identityGlyph(session, member);
  const name = member === undefined ? "wcode" : member.isRoot ? "orchestrator" : member.label;
  const used = state.status.contextUsed;
  const text = typeof used === "number" ? `ctx ${formatTokens(used)}` : "";
  return {
    cells: [
      {
        kind: "ident",
        glyph: glyph.glyph,
        glyphClass: `glyph ${glyph.className}${glyph.spin ? " spin" : ""}`,
        stateTitle: glyph.stateTitle,
        name,
      },
      { kind: "spacer" },
      { kind: "meter", word: session.state === "ready" ? "" : stateLabel(session.state), text },
      { kind: "mode", mode },
    ],
  };
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
  /** → the `.byline` line (draft `.byline` / `.byline.you`). */
  who: { className: string; glyph: string; name: string };
  /** True when this turn OPENS a speaker (a `--wc-5` beat); false when it continues
   *  the speaker above it (a tight `--wc-3`). → the `.turn.new` class. */
  isNew: boolean;
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
 *
 * Each turn also carries `isNew` — it OPENS a speaker (a `--wc-5` beat) vs CONTINUES
 * the speaker above it (a tight `--wc-3`), the transcript's role-aware rhythm. */
export function turns(
  blocks: RenderedBlock[],
  memberOf?: (origin: string | undefined) => TurnMember | undefined,
): Turn[] {
  const grouped: Turn[] = [];
  let lastSpeaker: string | null = null;
  for (const block of blocks) {
    const last = grouped[grouped.length - 1];
    const lastKind = last?.blocks[last.blocks.length - 1]?.kind;
    const opensYou = block.kind === "user";
    const opensPeer = block.kind === "peer";
    // A NEW turn starts on a `user`/`peer` block, right after one, OR on an ORIGIN
    // change (the merged run boundary) — so the timeline reads as discrete messages.
    const startsRun = last === undefined || last.blocks[last.blocks.length - 1]?.origin !== block.origin;
    // V14: the turn's SPEAKER — `you`, else the member that owns the block (a peer's
    // sender, else its origin). A turn whose speaker matches the one above CONTINUES it.
    const speaker = opensYou ? "you" : (block.from ?? block.origin ?? "wcode");
    if (opensYou || opensPeer || last === undefined || lastKind === "user" || lastKind === "peer" || startsRun) {
      // A peer block is labeled by its SENDER (`from`); everything else by its origin.
      const member = memberOf?.(opensPeer ? block.from : block.origin);
      grouped.push({
        role: opensYou ? "you" : "wcode",
        who: who(opensYou, member),
        isNew: speaker !== lastSpeaker,
        blocks: [block],
      });
      lastSpeaker = speaker;
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
  /** → the `.byline` leading glyph (`⠋ ✓ ✗ ○`), from `memberGlyph(state)`. */
  glyph: string;
}

/** The `.byline` line: `You` / the root's `❯` / a member's state glyph. Pure. */
function who(isYou: boolean, member: TurnMember | undefined): Turn["who"] {
  if (isYou) return { className: "byline you", glyph: "", name: "You" };
  if (member === undefined || member.isRoot) {
    return { className: "byline", glyph: "❯", name: member?.name ?? "wcode" };
  }
  return { className: "byline", glyph: member.glyph, name: member.name };
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
 * The composer's control set (draft `.composer .foot`). Pure.
 *   mode     = `state.status.planMode` ? "Plan" : "Act"  (the toggle's pressed segment)
 *   segments = the mode toggle — Act then Plan, exactly one pressed
 *   stop     = `status.running`  (Show `Stop`? Conditional chrome: Esc already cancels.)
 * The `Model: …` span is GONE, and the model no longer renders anywhere (the `▾`
 * disclosure was dropped in W005); there is no `model` field. Effort and a context
 * window are not on the wire.
 */
export interface ComposerControls {
  /** The mode — the pressed segment of the mode toggle. */
  mode: "Plan" | "Act";
  /** The mode toggle's segments, in order; EXACTLY one is pressed. */
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
/** One live subagent row — the live type-line's source. */
export interface WorkingRow {
  id: string;
  /** Display name; the ROOT reads as "orchestrator" (mirrors `reducer.displayLabel`). */
  name: string;
  /** True when the row's member is `state === "running"` (→ `.g-run`). */
  running: boolean;
  /** The dim right-hand text (the member's `liveAction`). */
  action: string;
  /** The member's identity swatch (roster order) — kept on the row; no avatar consumes it now. */
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
 *   swatch  = memberSwatch(index, isRoot)   (kept on the row; unused by the surface)
 *   initial = memberInitial(name, isRoot)   (the root's ❯, else the initial)
 *   count   = rows.length
 *
 * `liveAction` is CLEARED on `agent_end`, so a finished member drops out — correct,
 * and why the predicate is NOT widened to done/failed. A `running` member that has
 * not emitted a tool call yet has `action === ""`.
 */
/** The one live type-line's text (A6: count-only). Pure. */
export function livelineLabel(count: number): string {
  return `${count} working`;
}

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

/* ------------------------------------------------------------- member swatches */

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
 * collide.
 */
export function memberSwatch(index: number, isRoot: boolean): MemberSwatch {
  if (isRoot) return MEMBER_SWATCHES[0];
  return MEMBER_SWATCHES[index % MEMBER_SWATCHES.length];
}

/** One team-caption row (draft `.team .m`): glyph + name + dim action. */
export interface CaptionRow {
  id: string;
  /** → the `<span class="m">` class list; `sel` on the target (the retarget affordance). */
  className: string;
  /** → the state glyph's class (`g-run` / `g-done` / `g-err` / `g-idle`) — colour is STATE. */
  glyphClass: string;
  /** → the state glyph (`⠋ ✓ ✗ ○`), from `memberGlyph(row.state)`. */
  glyph: string;
  name: string;
  /** → the dim `.act` text (`row.liveAction ?? row.state`). */
  action: string;
}

/**
 * The team caption rows, ready to paint (draft `.team`). Pure. Identity is the NAME; the
 * glyph's colour is STATE. There is NO `--sw` edge (the swatches are dropped) and NO `meta`.
 */
/** The caption row's activation keys: Enter or Space (it is a `tabindex="0"` element). Pure. */
export function isActivationKey(key: string): boolean {
  return key === "Enter" || key === " " || key === "Spacebar";
}

export function teamCaption(rows: RosterItem[], targetId: string | null): CaptionRow[] {
  return rows.map((row) => {
    const glyph = memberGlyph(row.state);
    return {
      id: row.id,
      className: row.id === targetId ? "m sel" : "m",
      glyphClass: glyph.className,
      glyph: glyph.glyph,
      name: row.label,
      action: row.liveAction ?? row.state,
    };
  });
}

/** The avatar glyph: the root's `❯`, else the name's initial. Mirrors `who()`. Pure. */
export function memberInitial(label: string, isRoot: boolean): string {
  return isRoot ? "❯" : label.charAt(0).toUpperCase();
}
