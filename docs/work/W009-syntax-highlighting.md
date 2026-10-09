# W009 — VS Code surface: syntax highlighting for code blocks, tool output and tool input — brief

- **Status:** draft, for a human/orchestrator to record
- **Work item:** W009
- **Depends on:** [W008](W008-paper-vscode-surface.md) — **landed** (tracker row 65: `348600f`…`5f6343c`). The tool apparatus is now turn-foot footnotes; this change sits *inside* that markup. Sequences **after** W008.
- **Closes:** the deferred decision *"P1 ships fenced code WITHOUT token highlighting (the `syntect` analogue is a later decision)"* — `editors/vscode/src/markdown.ts:7`, and mirrored at `docs/sketches/vscode-extension.md:480`.
- **Tracker:** row **66** (`docs/next-steps.md`; 64 = W007, 65 = W008)
- **Author:** session `agent:brainstormer` (recorded by the orchestrator)
- **Date:** 2026-10-09

---

## 1. The ask

Add **token syntax highlighting** to the three code-bearing surfaces of the VS Code webview, which today render code as flat escaped text:

1. **the assistant's fenced code blocks** — the `markdown-it` `fence` rule captures the info string as a `.chead` label but emits no tokens: `editors/vscode/src/markdown.ts:26-31`
   ```ts
   md.renderer.rules.fence = (tokens, idx) => {
     const token = tokens[idx];
     const lang = token.info.trim().split(/\s+/)[0] ?? "";
     const label = md.utils.escapeHtml(lang);
     const code = md.utils.escapeHtml(token.content);
     return `<div class="code"><div class="chead"><span>${label}</span></div><pre class="pre">${code}</pre></div>\n`;
   ```
2. **tool output** — the output recess `pre.quote` built by `rawOutput`: `editors/vscode/src/webview/chat.ts:277-279`
   ```ts
   function rawOutput(tool: RenderedTool): HTMLElement {
     return el("pre", "quote", tool.outputText);
   }
   ```
3. **tool input** — the `edit`/`write` tool's written content, rendered as the diff body's `.txt` span: `editors/vscode/src/webview/chat.ts:312` (`row.appendChild(el("span", "txt", line.text));`).

The human's resolution rule: **`bash` output as bash; `read`/`edit` content highlighted by the file's extension; fenced blocks by the fence info string; else plain.**

**Adopted approach (settle it, don't re-open):** a **small, fully offline, bundled** highlighter — Prism-light (`prismjs` core + a handful of grammars: bash, json, javascript/typescript, rust, python, plus toml/yaml/markdown as warranted). **No remote assets** (the extension is `@vscode/vsce`-packaged; the highlighter must be **statically imported** so esbuild bundles it — no runtime path resolution). Token **colour** must come from VS Code's own `--vscode-symbolIcon-*` family so the "only `--vscode-*`" rule holds and it themes correctly.

---

## 2. Scope / Non-scope

### 2.1 In scope

- A new pure host module `src/highlight.ts`: `highlight(code, lang) -> string | null` + the language resolvers (`languageForPath`, `languageForTool`) — §4.1.
- The **fence rule** (`markdown.ts:26-31`) highlighting the code body; the `.code`/`.chead`/`.pre` plate markup is otherwise unchanged (W008's caption stays).
- **Tool output**: `RenderedTool` gains `outputHtml?` (`editors/vscode/src/render.ts:18-39`, beside `outputText` at `:23`); `rawOutput` (`chat.ts:277-279`) inserts it.
- **Tool input (the diff body)**: `RenderedTool` gains `diffLinesHtml?: string[]` (beside `diff?` at `render.ts:35`); the `reviewBlock` line loop (`chat.ts:307-313`) inserts each line's html.
- The **token palette**: a `.token.*` rule set in `media/chat.css` mapping Prism's token classes to `--vscode-symbolIcon-*` (with `--vscode-editor-foreground` / `--vscode-descriptionForeground` fallbacks), after `pre.quote` (`media/chat.css:458`).
- The **one new dependency**: `prismjs` in `editors/vscode/package.json` `dependencies` (`package.json:118-120`, beside `markdown-it` at `:119`) — **host-side** (see §2.2).
- The **record (E0)**: the placement + dependency + colour-budget audit in `docs/plans/vscode-ui-editorial-plan.md` + tracker row 66.

### 2.2 Non-scope (the important list — incl. the two "escaped" questions)

- **The "no new dependency" question — answered explicitly: the *webview* gains no dependency.** The webview is *stated* to be dependency-free and to ship pre-rendered HTML:
  - `editors/vscode/src/render.ts:9` — *"webview stays dependency-free and never ships `markdown-it`"*
  - `editors/vscode/src/webview.ts:10` — *"carry the pre-rendered `RenderedState` instead, so the webview stays dependency-free."*
  - `editors/vscode/esbuild.mjs:5` — *"dependency-free IIFE"*; `editors/vscode/README.md:37` — *"(typed, dependency-free)"*.
  Therefore **the highlighter goes in the host** (`src/highlight.ts`, bundled into `out/extension.js`, which already bundles `markdown-it`). The webview bundle (`media/chat.js`) stays **dependency-free and unchanged in size**; only cheap DOM branches are added. **Non-scope: adding Prism (or any dep) to `media/chat.js`, which would break the invariant above.** *(If the human instead wants the highlighter in the webview, that is a deliberate relaxation of a stated invariant and must be recorded in E0 — see §9; it is not this brief's recommendation.)*
- **The bundle-size question — answered with the placement.** The ~35 KB figure is the **webview** bundle (`media/chat.js`), which this change does **not** grow. The addition lands on the **host** bundle `out/extension.js` (already ~`markdown-it`); the Prism cost is estimated in §3 and must be **measured** in P1 (`ls -l out/extension.js` before/after).
- **The kernel / NDJSON wire — untouched.** The only payload that changes is the **host → webview** `RenderedState` (`editors/vscode/src/webview.ts`); `parseToWebview` validates `state` leniently (it only checks `blocks` is an array and `status` is an object), so the added `RenderedTool` fields need **no parser change**.
- **The TUI.** `crates/wcode-tui` already highlights fenced code with `syntect` (`crates/wcode-tui/src/markdown.rs:583` `highlight_line`; syntax resolved at `markdown.rs:554` `fence_syntax` via `find_syntax_by_token` at `:556`). Rust `syntect` is **not usable in the webview**. Highlighting the **TUI tool panel** (`crates/wcode-tui/src/ui.rs:813` `tool_panel_body`, `:797` `panel_param_lines` — both `dim()`/`added_style()`, unhighlighted) is a **separate change with its own tests**, cited here only as the parity reference.
- **A webview theme catalog / preset picker.** The webview has no theme of its own — it tracks the host's `--vscode-*`. The TUI's multi-preset `syntect` themes (`crates/wcode-tui/src/theme.rs:564-569`) do not travel here.
- **A full language set / auto-detection.** Ship a **curated subset** matched to wcode's tools + the TUI's bundled syntaxes. **No `Prism.highlightAuto`** (auto-detect is fuzzy and pulls in every grammar).
- **Restyling the plate / recess / card.** Only token spans are added *inside* `.code .pre` (`media/chat.css:446`), `pre.quote` (`:458`) and `.hunk .ln .txt` (`:1000`); the layout, the `.hunk .ln.add/.del` backgrounds and the `reviewBlock` card (`chat.ts:279`) are unchanged.
- **A new wcode colour token or a second accent.** `--wc-accent` (`media/chat.css:28`, *"the one wcode accent"*) is unchanged; token colour is `--vscode-*` only.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why (one line) |
|---|------------|-------|------------|------|----------------|
| **E0** | Record the decision (placement + dependency + colour budget) in the plan + tracker 66 | high | low | low | The colour budget and the "no webview dependency" boundary must be written before code (`design-taste` §7.3); docs-only, reversible. |
| **P1** | `src/highlight.ts` + `prismjs` + the language resolvers | high | medium | medium | One dependency, one pure module — but the language set must be curated and the host bundle measured. |
| **P2** | The fence rule (assistant code blocks) | high | low | low | One call site inside the existing `fence` rule; the plate markup is untouched. |
| **P3** | Tool output (`RenderedTool.outputHtml` → `rawOutput`) | high | low | low | One payload field + one webview branch; `parseToWebview` already tolerates it. |
| **P4** | Tool input (the diff body) | medium | medium | medium | Per-line highlight of a diff can mis-tokenise multi-line constructs; the language must come from `tool.path`. |
| **P5** | The token palette in `media/chat.css` | medium | low | low | Pure CSS; the taste question is *which* host variables, not the logic; a missing variable must degrade gracefully. |
| **P6** | The sweep + the visual proof (F5) | medium | low | medium | The webview was never run (`editors/vscode/README.md`, *"no webview and no VS Code was ever run"*); a highlight is a painted claim. |

**Sizing (estimates — basis stated, and must be measured in P1).** Prism core minified ≈ 18–20 KB; the grammars in the adopted set ≈ 2–7 KB each (javascript is the largest, ~7 KB; typescript adds ~2 KB on top of javascript; bash ~2 KB; json ~1.5 KB; rust ~2.5 KB; python ~4 KB; toml ~1 KB; yaml ~2 KB; markdown ~4 KB). **Total ≈ +45–60 KB minified on `out/extension.js`** (not gzipped; these are the minified sizes). The webview bundle `media/chat.js` is **unchanged**.

---

## 4. Interface & structure

### 4.1 `src/highlight.ts` (new, pure — host)

```
export function highlight(code: string, lang: string): string | null;
export function languageForPath(path: string): string | null;
export function languageForTool(name: string, path: string | undefined): string | null;
```

- **`highlight`** resolves `lang` to a loaded Prism grammar and returns `Prism.highlight(code, grammar)` — HTML with `class="token …"` — or **`null`** when the language is unknown/not loaded. **Prism encodes its own output**, so the result is safe to insert via `innerHTML`; a `null` return falls back to the existing `escapeHtml` (injection safety preserved). *Pure* — no `vscode`, no `node:` — so it joins the `test/purity.test.ts` list (`editors/vscode/src/render.ts:9`, `webview/view.ts` et al.).
- **`languageForPath`** — extension → grammar: `.rs`→rust, `.ts`/`.tsx`→typescript, `.js`/`.jsx`/`.mjs`/`.cjs`→javascript, `.json`→json, `.py`→python, `.sh`/`.bash`/`.zsh`→bash, `.toml`→toml, `.yaml`/`.yml`→yaml, `.md`/`.markdown`→markdown, `.css`→css, `.html`→markup; else `null`.
- **`languageForTool`** — `bash`→`bash`; `read`/`edit`/`edits`/`write`/`ast_edit`→`languageForPath(path)`; anything else → `languageForPath(path)` (so `grep`/`find` plain, a path-bearing tool highlighted by ext).
- The **fence info string** normalizer (alias map: `ts`→typescript, `js`→javascript, `sh`/`shell`→bash, `yml`→yaml, `rs`→rust, `py`→python, `md`→markdown) lives here too — the webview-agnostic analogue of the TUI's `fence_syntax` (`crates/wcode-tui/src/markdown.rs:554-557`).

### 4.2 Integration points (`file:line` + snippet)

| # | file:line | snippet | change |
|---|-----------|---------|--------|
| 1 | `editors/vscode/src/markdown.ts:30` | `const code = md.utils.escapeHtml(token.content);` | `const body = highlight(token.content, lang) ?? md.utils.escapeHtml(token.content);` and emit `body` at `:31` (use the **raw** `lang` for the grammar, keep the escaped `label` at `:29`). |
| 2 | `editors/vscode/src/render.ts:23` / `:35` | `outputText: string;` / `diff?: string;` | add `outputHtml?: string;` and `diffLinesHtml?: string[];`. |
| 3 | `editors/vscode/src/render.ts:165` (`renderTool`, the fields at `:169-176` e.g. `outputText: tool.output,` `:171`) | `outputText: tool.output,` | `outputHtml: highlight(tool.output, languageForTool(tool.name, tool.path)),` and, when `hasDiff` (`:176`), `diffLinesHtml` from `reviewHunk(tool.diff)` (`src/review.ts`, pure) — join the line texts with `\n`, `highlight` once (preserves multi-line context), split by `\n`. |
| 4 | `editors/vscode/src/webview/chat.ts:277-279` | `return el("pre", "quote", tool.outputText);` | insert `tool.outputHtml` when present (a small helper: create `pre.quote`, set `innerHTML`, else `textContent = outputText`). Its one caller is `fnNote` (`chat.ts:425`, `rawOutput(note.tool)`). |
| 5 | `editors/vscode/src/webview/chat.ts:307-312` | `row.appendChild(el("span", "txt", line.text));` | when `tool.diffLinesHtml?.[i]` is present, set the `.txt` span's **`innerHTML`** (Prism escapes its own output — do NOT route it through `el(tag, class, text)`, which sets `textContent`); else `textContent = line.text`. |
| 6 | `editors/vscode/media/chat.css:458` (`pre.quote`), `:446` (`.code .pre`), `:1000` (`.hunk .ln .txt`) | — | add a `.token.*` block (below). |
| 7 | `editors/vscode/package.json:118-120` | `"dependencies": { "markdown-it": "^14.1.0" }` | add `"prismjs": "^1.29.0"` (host dependency; bundled by `esbuild.mjs:13`/`:42`, `out/extension.js`). |

**No wire/parser change:** `parseToWebview` checks only `Array.isArray(rendered.blocks)` and `rendered.status` — the added `RenderedTool` fields pass through (`editors/vscode/src/webview.ts`).

### L1 review amendments (settled — follow exactly)

- **Use `highlight`, not `highlightHost`** (§4.2 row 3).
- **Set the `.txt` span's `innerHTML`** when `diffLinesHtml` is present (§4.2 row 5) — `el(tag, class, text)` sets `textContent` and would show literal `<span>` text.
- **`background` → `bash`** in `languageForTool` (it is a bash runner, not a path-bearing tool).
- **E0 also opens a DNNN** (the highlighter choice + host-side placement + colour budget) — W009 closes the deferred `markdown.ts:7` decision and adds the first new dependency since `markdown-it`.
- **P6/F5 is MANDATORY** for the palette: if the `--vscode-symbolIcon-*` family is absent, the fallback collapses every token to `--vscode-editor-foreground` (a silent no-op). F5 must confirm tokens paint, or record "highlighting unavailable" — never a silent no-op.
- **P4's gate** must include a multi-line construct (a block comment / triple-quoted string) and assert the per-line HTML is well-formed (re-open continuation spans), or explicitly accept that a continuation line loses colour.
- **P1 must measure** the `out/extension.js` delta and confirm `media/chat.js` does not grow.
- **Refreshed refs (D007):** `chat.css` `pre.quote` `:458`, `.code .pre` `:446`, `.hunk .ln .txt` `:1000`; `chat.ts` `fnNote` `:425`.

### 4.3 The token palette (CSS) — `--vscode-symbolIcon-*`, no new token

Add after `media/chat.css:458` (no new `:root` token; `--wc-accent` stays the one accent):

| Prism class | `--vscode-*` |
|---|---|
| `comment`, `prolog`, `doctype`, `cdata` | `--vscode-descriptionForeground` *(there is **no** `symbolIcon-commentForeground`)* |
| `keyword`, `atrule`, `important` | `--vscode-symbolIcon-keywordForeground` |
| `string`, `char`, `attr-value`, `regex`, `url` | `--vscode-symbolIcon-stringForeground` |
| `number` | `--vscode-symbolIcon-numberForeground` |
| `boolean` | `--vscode-symbolIcon-booleanForeground` |
| `constant`, `symbol` | `--vscode-symbolIcon-constantForeground` |
| `function`, `method` | `--vscode-symbolIcon-functionForeground` |
| `class-name`, `struct` | `--vscode-symbolIcon-classForeground` |
| `interface` | `--vscode-symbolIcon-interfaceForeground` |
| `builtin`, `namespace`, `module`, `package` | `--vscode-symbolIcon-namespaceForeground` |
| `property`, `attr-name` | `--vscode-symbolIcon-propertyForeground` |
| `variable`, `parameter` | `--vscode-symbolIcon-variableForeground` |
| `operator` | `--vscode-symbolIcon-operatorForeground` |
| `punctuation`, `tag`, `selector`, `entity` | inherit `--vscode-editor-foreground` |

Every declaration carries a **fallback** so a missing variable degrades gracefully instead of unsetting the colour, e.g.
```css
.token.keyword { color: var(--vscode-symbolIcon-keywordForeground, var(--vscode-editor-foreground)); }
```
*(Which `--vscode-symbolIcon-*` variables are actually injected into a webview could not be verified here — see §9. No `symbolIcon` reference exists anywhere in the repo today, so this is a fresh family.)*

---

## 5. The plan (each step: deliverable + gate)

1. **E0 — record the decision.** Deliverable: a "W009 — syntax highlighting" note in `docs/plans/vscode-ui-editorial-plan.md` recording (a) the **placement** (host-side; the webview stays dependency-free — §2.2), (b) the **dependency** (Prism-light + the language set), (c) the **colour budget** (token colour is `--vscode-symbolIcon-*` with fallbacks; no new `--wc-*`; `--wc-accent` unchanged — so the stated invariant *"colour is ONLY `--vscode-*` plus the ONE accent"* holds); plus tracker row **66** in `docs/next-steps.md`. Gate: layer-1 review APPROVE; the audit is anchored; **no code touched** (`git show --stat` = `docs/` only).
2. **P1 — the highlighter module.** Deliverable: `src/highlight.ts` (`highlight`, `languageForPath`, `languageForTool`), `prismjs` in `package.json`, and the **measured** `out/extension.js` size delta. Gate: new `test/highlight.test.ts` (a `rust` snippet yields `token keyword`; an unknown/empty lang returns `null`); `highlight.ts` added to `test/purity.test.ts`; `npm run typecheck && npm test && npm run build` green; `ls -l out/extension.js` before/after stated.
3. **P2 — the fence rule.** Deliverable: `markdown.ts:26-31` emits highlight spans; the `.code`/`.chead` plate markup unchanged. Gate: `test/markdown.test.ts:18-23` updated (its `assert.ok(!html.includes("hljs"), "no highlighting in P1")` flips to a positive `token` assertion — see §6); the no-language fence still escapes (`&lt;b&gt;`); `npm test` green.
4. **P3 — tool output.** Deliverable: `RenderedTool.outputHtml` (`render.ts`) + the `rawOutput` branch (`chat.ts:277-279`). Gate: a `test/render.test.ts` case — a `bash` tool's `outputHtml` contains `token` (bash grammar); a `read` of a `.rs` `path` yields rust tokens; a no-path/bash-less tool yields `outputHtml === undefined`; **F5** shows a highlighted output recess inside a footnote.
5. **P4 — tool input (the diff).** Deliverable: `RenderedTool.diffLinesHtml` (`render.ts`) + the `.txt` insertion (`chat.ts:312`). Gate: a `test/render.test.ts` (or `test/review.test.ts`) case — a `.ts` diff's `diffLinesHtml` lines carry `token`; the `.ln.add`/`.ln.del`/`.ln.ctx` kinds and the `reviewBlock` structure are unchanged; **F5**.
6. **P5 — the token palette.** Deliverable: the `.token.*` rules in `media/chat.css`. Gate: `node scripts/check-css.mjs` prints `check-css: ok (chat.css)`; a CSS assertion that **every** `.token.*` value is a `var(--vscode-…` (no `#hex`, no new `--wc-*`); **F5**.
7. **P6 — sweep + visual proof.** Deliverable: the "no highlighting in P1" notes removed (`markdown.ts:7`; `docs/sketches/vscode-extension.md:480` updated to record the closing decision); the F5 screenshot. Gate: `grep -rn "no highlighting\|WITHOUT token highlighting"` returns nothing in `src/` + that sketch; **§7.2**.

---

## 6. Quality gates

Run **in `editors/vscode`** (cargo is *not* this change's gate — the webview is not a Rust surface):

```sh
cd editors/vscode
npm run typecheck && node scripts/check-css.mjs && npm test && npm run build
```
- `npm run typecheck` (`package.json:111` = `tsc --noEmit && tsc --noEmit -p tsconfig.webview.json`) → **no output, exit 0**.
- `node scripts/check-css.mjs` → `check-css: ok (chat.css)`.
- `npm test` (`package.json:113`) → `check-css: ok (chat.css)` then `# pass <n>` / `# fail 0`.
- `npm run build` (`package.json:109` = `node esbuild.mjs`) → `out/extension.js` and `media/chat.js` written; **`media/chat.js` must not grow a dependency** (its size is unchanged); state the `out/extension.js` delta.

**Untouched guard (no Rust is touched):**
```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```
Both must stay clean — a guard, not a gate.

**Regression tests kept green (by name):** `renders common markdown`; `html:false escapes raw HTML instead of passing it through`; `a fence with no language gets a blank header, and the code is escaped` (the escaping must survive the highlighter); `an assistant message renders to markdown HTML, not raw text`; `tool blocks render a collapsed summary and flag a diff`; `user, error and notice text is escaped, never rendered as HTML`; `escapeHtml covers the four dangerous characters`; `a fenced code block renders as a titled .code card`; `parseFromWebview accepts the well-formed messages` / `rejects junk`; `renderState …`; `turnNotes …`; `purity.test.ts` — `view.ts`/`render.ts`/`highlight.ts` import no host or I/O.

**Tests that MUST change (by name):**
- `test/markdown.test.ts:18` — `"fenced code renders as a titled .code card (no highlighting in P1)"`; its `assert.ok(!html.includes("hljs"), "no highlighting in P1")` (`:22`) is now *meaningless* (Prism emits `class="token …"`, not `hljs`) and must flip to a **positive** `token` assertion.
- `test/render.test.ts:119` — the same `!html.includes("hljs")` placeholder in `"a fenced code block renders as the draft's .code card"`.

---

## 7. Testing

### 7.1 Automated

- **New — `test/highlight.test.ts` (pure).** (a) `highlight("fn main() {}", "rust")` contains `token` + `keyword`; (b) `highlight("x", "")` and `highlight("x", "nope")` return `null`; (c) a `<script>` in the code never survives as raw HTML (Prism escapes); (d) `languageForPath("src/a.rs") === "rust"`, `languageForPath("README") === null`; (e) `languageForTool("bash", undefined) === "bash"`, `languageForTool("read", "x.ts") === "typescript"`.
- **Changed — `test/markdown.test.ts`.** `renderMarkdown("```js\nconst x = 1;\n```")` now matches `class="token` (and still the `.code`/`.chead`/`.pre` plate); the no-language fence still escapes (`&lt;b&gt;`).
- **New — `test/render.test.ts`.** `renderTool` — a `bash` tool yields `outputHtml` with `token`; a `read` with `path: "a.rs"` yields rust `token`s; a pathless tool yields no `outputHtml`; a `.ts` diff yields `diffLinesHtml` lines with `token`.
- **Kept green.** The W005/W008 render/markdown/view/webview/purity set above (behaviour, not shape).

### 7.2 How a human verifies it

```sh
cd editors/vscode
npm install
npm run typecheck && node scripts/check-css.mjs && npm test && npm run build
# (real surface) open the repo root in VS Code, press F5,
# then Cmd+Shift+P -> "wcode: Start Session"
```

Then ask wcode to: (a) show a fenced code block (a `.ts`/`.rs` snippet) — expect token colour **inside** the `.code` plate, the caption line unchanged; (b) `read` a source file — expand the footnote's `output` disclosure (`details.fn-out`) and expect the recess highlighted **by the file's extension**; (c) run `bash` — expect its output highlighted as bash; (d) make an edit — open the change-review card's diff and expect the `.hunk` lines highlighted, with the add/del backgrounds intact. Confirm **no external request** fires (DevTools → Network must stay empty; the highlighter is bundled in `out/extension.js`), and that colours follow the host theme (switch themes; the tokens track `--vscode-symbolIcon-*`). If a variable is absent, the fallback shows `--vscode-editor-foreground` (the test in §4.3). If no endpoint is configured, "Start Session" ends `✗ crashed` — the failure state, not a silent view.

---

## 8. Expected outcome

- `editors/vscode/src/highlight.ts` exists (pure; `highlight`/`languageForPath`/`languageForTool`); `purity.test.ts` lists it.
- `markdown.ts`'s fence rule emits Prism tokens; `RenderedTool` (`render.ts`) carries `outputHtml` and `diffLinesHtml`; `chat.ts:rawOutput` and `chat.ts:reviewBlock` insert them.
- `media/chat.css` carries a `.token.*` palette using **only** `--vscode-*` (with fallbacks); **no new `--wc-*`**; `--wc-accent` unchanged.
- `package.json` adds `prismjs` to `dependencies`; **`media/chat.js` gains no runtime dependency** and its size is unchanged; the `out/extension.js` size delta is stated.
- The tests named in §6 are green; `cargo test --workspace` / `cargo clippy --workspace --all-targets` remain clean (untouched).
- `docs/plans/vscode-ui-editorial-plan.md` records the W009 decision; `docs/next-steps.md` has row **66**; the "no highlighting in P1" notes are gone from `markdown.ts` + the sketch.

---

## 9. Log

### 2026-10-09 — W009 landed (E0–P6); F5 pending

- **E0** `0a5e89c` (the plan note + D008 + row 66 + the brief). **P1** `e375fdf` (`src/highlight.ts` + `prismjs` + `@types/prismjs`, in `purity.test.ts`). **P2** `285a3c2` (the fence rule). **P3** `d4aacf9` (`RenderedTool.outputHtml` + `rawOutput`). **P4** `2c7bcfa` (`diffLinesHtml` + `splitHighlightedLines`). **P5** `343b53f` (the `.token.*` palette).
- **Measured bundle delta (P1/P6):** `out/extension.js` **316,271 → 426,006 B (+109,735**, Prism core + 11 grammars, UNMINIFIED — `esbuild.mjs` sets no `minify`); `media/chat.js` **37,884 → 38,161 B (+277**, the DOM branches) — **no Prism in the webview** (`grep -c Prism media/chat.js` → 0).
- **Gates:** `npm run typecheck` 0; `check-css: ok (chat.css)`; `npm run build` ok; `node --test --test-concurrency=1 test/*.test.ts` → **219 pass / 0 fail**. The two `!includes("hljs")` placeholders flipped to positive `class="token ` assertions (markdown.test.ts, render.test.ts).
- **P4 multi-line construct:** a 2-line `.ts` block comment — `splitHighlightedLines` closes the span at the line end and RE-OPENS it on the continuation, so every fragment is well-formed AND the continuation keeps its colour (mutation-verified: a naive split fails the test).
- **F5 — NOT run (no VS Code host here).** Mandatory: confirm the tokens actually PAINT, i.e. that the webview injects the `--vscode-symbolIcon-*` family. If absent, every declaration falls back to `--vscode-editor-foreground` and highlighting is a **silent no-op** (one colour). No `symbolIcon` reference exists in the repo (this is a fresh family), so this could not be verified from code. **This is the one open step.**

## 10. References

- `editors/vscode/src/markdown.ts` (`:7` the deferred note, `:26-31` the fence rule), `src/render.ts` (`:18-39` `RenderedTool`, `:165-176` `renderTool`, `:9` the dependency-free note), `src/review.ts` (`reviewHunk`), `src/webview/chat.ts` (`:277-279` `rawOutput`, `:307-313` the diff line loop, `:425` `fnNote`, `:433` the `.to` command), `src/webview.ts` (`:10` the pre-rendered note), `media/chat.css` (`:446` `.code .pre`, `:458` `pre.quote`, `:1000` `.hunk .ln .txt`, `:28` `--wc-accent`).
- `editors/vscode/esbuild.mjs` (`:5` "dependency-free IIFE", `:13`/`:42` the host bundle), `package.json` (`:109`/`:111`/`:113` scripts, `:118-120` dependencies), `editors/vscode/README.md` (`:37`).
- TUI parity: `crates/wcode-tui/src/markdown.rs` (`:554` `fence_syntax`, `:556` `find_syntax_by_token`, `:583` `highlight_line`), `crates/wcode-tui/src/theme.rs:564-569` (the scope→colour map), `crates/wcode-tui/src/ui.rs:797`/`:813` (the unhighlighted tool panel).
- `docs/plans/vscode-ui-editorial-plan.md`, `docs/work/W008-paper-vscode-surface.md`, `docs/next-steps.md` (row 65 = W008 landed; row 66 = W009), `docs/sketches/vscode-extension.md:480`, `.wcode/skills/design-taste/SKILL.md` (§6 `[web]` tells; §7.3 amend-first).

---

## Facts NOT verified

- **No `npm`, node, browser or VS Code host was run** (read-only worker). All bundle sizes are **estimates** (minified, not gzipped); the "*~35 KB webview bundle*" and the Prism deltas were **not measured** — `media/chat.js` and `out/extension.js` are generated and absent from the tree. P1 must measure `out/extension.js`.
- **The `--vscode-symbolIcon-*` variables' existence in a webview is unverified.** No `symbolIcon` reference exists anywhere in the repo (grep: 0 matches). VS Code's theme-colour registry documents the `editor.symbolIcon.*Foreground` keys, and webviews surface theme colours as `--vscode-*`, but this was **not observed**; hence the fallbacks in §4.3. **`symbolIcon-commentForeground` is asserted not to exist** — comments are mapped to `--vscode-descriptionForeground`.
- **The language set and the ext→grammar map are the author's proposal**, not a spec; the exact set should be confirmed against wcode's tools.
- **Per-line diff highlighting fidelity** (multi-line constructs mis-tokenising) is reasoned, not observed; the join-then-split mitigation (§4.2 row 3) preserves multi-line context but Prism's grammar state across the whole body was not tested.
- **`.hljs`/`hljs` in the existing tests is treated as a placeholder**; the adopted highlighter is Prism (`class="token …"`), so those assertions no longer test the property they name.
- **`chat.ts` was changing under this session** (W008 landed between reads); the `file:line` citations are from the current tree and will drift if a parallel session edits further.
- **Not read:** `webviewView.ts`, `panel.ts`, `surface.ts`, `.vscode/launch.json`, `tsconfig.webview.json` — asserted to need no change from the host-side placement and the lenient `parseToWebview`.
- **The tracker row number (66)** is inferred from rows 64 (W007) and 65 (W008), not verified against a counter.

The brief ends with the brief.
