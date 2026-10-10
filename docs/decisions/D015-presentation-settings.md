# D015 — VS Code extension: three PRESENTATION settings (and nothing behavioral)

- **Status:** accepted (human sign-off 2026-10-10, this session)
- **Date:** 2026-10-10
- **Amends / knowingly overrides:** D013's "**no new behavior-config knob**" and
  the `AGENTS.md:64-66` minimalism rule — for **presentation only** (see below)

## Context

`AGENTS.md:64-66` says config-for-behavior is absent by design:

> Minimalism is the point. Absent by design: MCP, subagents, permission prompts,
> approval flows, **config files for behavior**. That logic belongs in code, via
> `Hooks` — build the variant you want instead of configuring one.

D013 then recorded, for the multi-tab work, "no new behavior-config knob". The
extension therefore exposed exactly ONE setting (`wcode.path`), and three genuinely
user-facing **presentation** defaults were hardcoded:

| default | was | lived in |
|---|---|---|
| view mode (all/focus) | `"all"`, reset on restart | `surface.ts` `mode: ViewMode = "all"` |
| thinking fold | open while **live**, collapsed after | `render.ts` `renderContent` |
| tool fold | open while **running**, collapsed when done | `webview/view.ts` `foldOpen` |

## Decision

The `AGENTS.md` rule targets the **agent's behavior/policy** (what tools run, what
is permitted, approvals). How the surface *looks and folds* is **editor
presentation**, a different category that VS Code extensions are expected to expose.
So the rule is honoured for behavior and **narrowly overridden for presentation**:
exactly **three** settings, all under `wcode.view.*`, all with a default that is
**byte-identical to the previous hardcoded behavior**:

| key | values | default |
|---|---|---|
| `wcode.view.mode` | `all` \| `focus` | `all` |
| `wcode.view.thinking` | `auto` \| `collapsed` \| `expanded` | `auto` |
| `wcode.view.tools` | `auto` \| `collapsed` \| `expanded` | `auto` |

`auto` = the old rule, verbatim.

**What is deliberately NOT configurable** (the `AGENTS.md` list): approvals/plan
gates, permission prompts, tool allow-lists, spawn policy, model/effort defaults
(the CLI owns agent config in `config.toml` — the extension does not duplicate it).

**These are DEFAULTS, not locks.** A manual fold click still wins (the per-`callId`
override map), and `wcode.view.mode` is the mode of a **new** session only — a
change to it never fights the All/Focus the user is already looking at.

## Mechanism — one path

`ViewPrefs` (`{ mode, thinking, tools }`) is part of `ToWebview`: the `Manager`
reads `workspace.getConfiguration("wcode")` and rides the prefs on every snapshot.
`SurfaceController.setPrefs` repaints; `onDidChangeConfiguration` (for `wcode.*`)
re-pushes to every tab, so a change is live. Mode + thinking apply host-side
(`SurfaceController`, `render.ts`); the tool default is consumed in the webview
(`webview/view.ts::foldOpen`'s new `def` param, set from `snapshot.prefs.tools`).
Parsing is LENIENT like `context`/`verdicts`: a bad value degrades to the built-in
default and the snapshot survives.

## Consequences

- `editors/vscode/package.json` gains three `configuration.properties` entries.
- `src/webview.ts` gains `FoldDefault`, `ViewPrefs`, `DEFAULT_PREFS`, and a `prefs`
  field on `ToWebview` (parsed leniently).
- Tests: `foldOpen`'s `def` behaviour, the thinking-fold preference, and prefs
  parsing (`test/view.test.ts`, `test/render.test.ts`). `npm test` stays green.
- Behavior/policy remains fixed in code — a future request to configure approvals
  or tool policy is REFUSED by this record, not merely skipped.
