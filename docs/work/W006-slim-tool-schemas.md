# W006 — Slim tool schemas (prompt-token diet, Phase 1)

- **Status:** in progress — Phase 1 implemented (`crates/wcode-harness/src/tool.rs`),
  `cargo test --workspace` (1173 passed) + `cargo clippy --workspace --all-targets`
  clean, live-measured; **second-layer review rejected the first cut (§5) and the fix
  is in — re-review pending**
- **Work item:** W006
- **Decisions:** **no new `DNNN`** — this changes neither the wire contract (the same
  JSON-Schema *meaning*, minus annotation keys that no model acts on and that
  validation does not depend on) nor any locked plan. The
  standing rule is enforced at one choke point and regression-tested, so it cannot
  silently regress (see §3 and §5).
- **Tracker:** [`next-steps.md`](../next-steps.md) item 63
- **Author:** orchestrator (measurement + Phase 1); second-layer reviewer (the §5 bug)
- **Date:** 2026-10-09

## 1. The ask

The human observed that the init prompt costs 10K+ tokens and asked which part costs
what. Measure the composition, then cut the part that is free to cut.

## 2. Where it actually goes — measured

Method (reproducible): a local capture server (`http.server` on `127.0.0.1:8731`, saved
as `/tmp/cap.py`) records the exact request body; the CLI is pointed at it with
`--base-url`. Cross-checked with the in-repo diagnostic
(`cargo test -p wcode-cli print_tool_definition_sizes -- --ignored --nocapture`,
`crates/wcode-cli/src/tools/mod.rs:olDqb`).

Captured payload, 19 tools, live project config:

| Part | chars | ~tok | share |
|---|---:|---:|---:|
| **Tool definitions (19)** | 27,796 | **~6,950** | **61 %** |
| System prompt | 17,850 | ~4,460 | 39 % |
| ├ base agent preamble | 296 | ~75 | 1 % |
| ├ Project instructions (`AGENTS.md`) | 5,654 | ~1,415 | 12 % |
| ├ Available skills | 2,832 | ~710 | 6 % |
| ├ Your team (6 agents) | 6,086 | ~1,520 | 13 % |
| └ Orchestrator workflow | 2,982 | ~745 | 7 % |
| **Grand total** | **45,646** | **~11,400** | 100 % |

Inside the tool definitions, the 27.8K chars split three ways:

| | chars | ~tok |
|---|---:|---:|
| top-level `description()` prose | 6,216 | ~1,554 |
| schema `parameters` — nested per-field prose | 9,662 | ~2,416 |
| schema `parameters` — structure (`$schema`, `title`, `format`, `type`, …) | 10,170 | ~2,543 |

**The finding that drove the plan.** The tool half is the cost (61 %), and prose is
~15.9K of its 27.8K chars — but half that prose is the schemars-generated per-field
`description`, not the top-level `description()`. Trimming prose therefore means
editing `///` doc comments across 19 tool files (Phase 2, judgment-heavy). The
*structural* third, by contrast, is validator scaffolding with a single choke point.
That split is why Phase 1 exists on its own.

**Separable by construction.** Every tool's schema is produced at one place —
`crates/wcode-harness/src/tool.rs:CvmaR` `ErasedToolCore::parameters()` — reached via
`Tool::definition()` (`tool.rs:egRrk`) from `loop_.rs`. Nothing is per-tool.

## 3. Phase 1 — what shipped

`slim_schema` (`crates/wcode-harness/src/tool.rs:KooPP`) walks the generated schema and
drops four **keywords**:

| dropped | why it is safe |
|---|---|
| `$schema` | the meta-schema URL (19 copies of `https://json-schema.org/draft/2020-12/schema`) — read by a validator, never by the model |
| `title` | schemars' `"ReadArgs"` / `"WebFetchArgs"`; the function name already rides `name` |
| `format` | schemars hints (`uint64`, `uint`) — not standard JSON-Schema `format`, and not a JSON type constraint |
| `default: null` | a null default says "optional", which the prose `description` already says |

**Keyword vs. property name — the distinction the first cut got wrong.** `title` and
`format` are keywords *in a schema position*, but arbitrary **property names** inside a
name map. `webfetch` really does take a `format` argument and `task` really does take a
`title`; filtering by key name alone deleted both from the wire schema. So `is_name_map`
(`tool.rs:eNJMV`) exempts the keys of `properties` / `patternProperties` /
`dependentSchemas` / `$defs` / `definitions` — the set schemars itself treats as name
maps — their keys are names the tool author chose, and only their *values* are
`definitions` — their keys are names the tool author chose, and only their *values* are
slimmed as schemas. See §5.

It is **recursive**, so `$defs` are covered (`edits`'s `EditOp`, `todo`'s `TodoItem`,
`session_search`'s `Scope`/`TurnsRange`).

**Deliberately not dropped** — everything that carries meaning survives, and a test
pins each one: property names (including ones named like a keyword), `required`, `$ref`,
`enum`, `const`, `oneOf`, `minimum`, non-null `default` (`spawn.read_only`'s `false`),
and a **nullable type is left intact** (`["integer","null"]` stays a two-element array,
so nullability is still expressed).

> **Why not collapse `["T","null"] → "T"` too?** It was measured: 65 occurrences, only
> **~158 tok**. It is the one rule that changes a *hint* rather than removing noise, so
> it is not worth the risk for 158 tokens. Left for a decision, not bundled in.

## 4. Measurement — before / after (live, 19 tools)

```
tool definitions as-is   27,796 chars   ~6,950 tok
after Phase 1            25,523 chars   ~6,380 tok
                         −2,273 chars    −568 tok   (−8.2 % of the tool half)
```

Whole payload: **45,646 → 43,373 chars**; the system prompt is byte-identical
(17,850 both runs), so the entire delta is the tool definitions.

> An earlier cut of this change measured **−2,474 chars / −618 tok** — but ~200 of those
> chars were `webfetch.format` and `task.title` being *deleted*, not slimmed. The
> corrected, honest number is **−568 tok**.

Default-config cross-check (12 tools, `print_tool_definition_sizes`):
**16,928 → 15,743 chars**, −1,185 chars ≈ −296 tok.

Gates: `cargo test --workspace` → **1173 passed, 4 ignored**; `cargo clippy --workspace
--all-targets` → **no issues**. New tests:

- `slim_schema_drops_boilerplate_but_keeps_semantics` (`tool.rs:5qnOc`) — one assertion
  per survivor, on a hand-built schema with `$defs`, a non-null default, and properties
  literally **named** `format` / `title`.
- `real_tool_definitions_are_slim` (`tool.rs:FJhF7`).
- `slimming_keeps_a_property_named_like_a_keyword`
  (`crates/wcode-cli/src/tools/mod.rs`) — pins the real `webfetch` schema, which is
  where the §5 bug was visible.

Both new behavioural tests were **verified to fail** against the pre-review
implementation (temporarily forcing `is_name_map` → `false` turned exactly those two
red), so they assert behaviour rather than merely passing.

Effect: since tool definitions ride **every** request, this is ~568 tokens off *every*
turn, in every session — orchestrator and workers alike (the workers build their
definitions through the same `parameters()`).

## 5. The bug the second-layer review caught

The first cut filtered by key name alone, anywhere in the tree. Because `properties` is
itself a JSON object, its **keys** — the property names — were walked as if they were
schema keywords, and any argument named `title`/`format` was deleted:

```
webfetch properties = ['timeout', 'url']          ← `format` gone (it is a real arg)
task     properties = [..., 'op', 'reason']       ← `title` gone (needed by `create`)
```

The prose `description`s still *mention* `format`/`title`, which masked it — but the
schema objectively no longer declared them. `is_name_map` fixes it; `webfetch properties
= ['format', 'timeout', 'url']` and `task properties = [..., 'title']` are restored
(verified on the live capture).

The reviewer also correctly noted **why CI was green**: the first cut's tests built a
`properties` map with no keyword-named keys, so they never exercised the path. Both
gaps are now covered (§4).

## 6. Phase 2 — deferred, needs a judgment call

Trim ~40 % of the 15.9K prose chars:

- `edits` re-declares the whole `EditOp` field set that `edit` already describes —
  duplicated verbatim (`from`/`old_string`/`path`/`replace_all`/`replacement`/`to`).
- `expected_digest`'s paragraph is repeated across `edit`, `replace`, `write` (~1,108
  chars total for one concept).
- Internal design-archaeology leaks into the model's prompt: `(D1)`, `(D4)`, `(D12)`,
  `(D6/Q3)`, `§5`, `[`AgentEvent`]`, `ToolContext.session_path` — 8 `DNNN` refs and
  5 crate/type refs that mean nothing to a model.

Estimated ≈ **−1.6K tok**. Not bundled with Phase 1 because it edits the prose that
encodes real semantics (the anchor contract, `E_STALE_DIGEST`) and can regress
tool-calling quality — it deserves its own reviewable diff.

## 7. Facts NOT verified

- **Real provider token counts.** The `/chat/completions` replay to the configured
  endpoint was refused (`HTTP 403`, Cloudflare `error code: 1010`), so every token
  figure here is the `chars/4` heuristic, not `usage.prompt_tokens`. JSON tokenizes
  worse than prose, so real counts are plausibly **10–20 % higher** — which makes the
  *relative* shares (tools 61 %) the load-bearing claim, not the absolutes.
- **Tool-calling quality.** Phase 1 removes no prose and no constraint, so it is
  expected to be inert — but that is an argument, not a measurement. Nothing here
  evaluates a model's behaviour before/after.
- **Non-OpenAI `parameters` consumers.** Only `Tool::definition()` →
  `rig::completion::ToolDefinition` → the wire is checked (`tool.rs:egRrk`,
  `loop_.rs:117`); rig forwards `parameters` as an opaque `Value`
  (`rig-core` `src/completion/request.rs:169`). No in-repo consumer reads `$schema`,
  `title` or `format`, and arg parsing is plain `serde_json::from_value` — not
  JSON-Schema validation.
