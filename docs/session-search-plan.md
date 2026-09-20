# wcode — session search (cross-session + in-session)

Status: **design — decisions D1–D5 settled**; ready to sketch. Companion to [`gap-analysis-jcode.md`](gap-analysis-jcode.md) Tier-1 rows 3–4.

## 1. Why
wcode persists every session as JSONL but can only replay one at a time. Two recall gaps: **cross-session** ("have I solved this before?" — nothing reads past sessions) and **in-session** (compaction replaces the older turns with a summary; they stay on disk but the model cannot see them and has no path back).

## 2. Ground truth
- A session is JSONL, one `SessionEntry` per line: `Header{version,id,cwd,created}`, `Message{id,parent_id,message}`, `ModelChange`, `EffortChange`, `Compaction{summary,first_kept_message,…}`, `Unknown` (`crates/wcode-harness/src/session.rs`).
- **Compaction deletes nothing**: it appends a boundary entry; `Session::messages()` applies it, but `Session::entries()` still returns every `Message` — so the summarized turns are recoverable with `Session::open` (this is exactly the in-session case).
- Store `~/.local/share/wcode/sessions/`: session **groups** are a dir (`root.jsonl` + `members/<name>.jsonl`); plus legacy flat `<millis>_<hex>.jsonl`; plus a plain-text `history` file. `list_sessions`/`list_groups` never list member files, so a search that wants worker findings must walk `members/` deliberately (`crates/wcode-cli/src/session_groups.rs`).
- `AgentMessage` = `User{content}`/`Assistant{content,stop_reason,usage,model}`/`ToolResult{…}`; `ContentBlock` = `Text`/`Thinking`/`ToolCall`; `as_text()` exists (`crates/wcode-harness/src/message.rs`).

## 3. Decisions
- **D1 — one tool.** A single `session_search` with `scope: "all" | "current"`, not jcode's two tools. Fewer surfaces; the current session is just one file to scan.
- **D2 — matching.** Case-insensitive; split the query on whitespace; ALL terms must appear (AND) in a message's text; score = occurrence count, recency as tiebreak; top-N grouped by session. `Thinking`/`ToolResult` excluded unless `include_tools`. Raw pre-filter (`line.contains(term)`) before `serde_json::from_str`.
- **D3 — output.** Grouped per session (id, timestamp, cwd, path) with snippets that carry the matched TEXT (the raw JSONL is unreadable, so the tool returns usable content). `scope:"current"` additionally supports `turns{start,end}` (verbatim range) and `stats` (turn count + token estimate).
- **D4 — plumbing.** Current session path via a new `ToolContext.session_path: Option<PathBuf>` (kernel), filled by the loop from `cfg.session`; sessions dir injected at tool construction in the CLI.
- **D5 — prompt tie-in.** One line in compaction's `SUMMARY_PROMPT` telling the model the summarized turns remain searchable via `session_search` (makes the recall path discoverable exactly when it matters).

## 4. Interface
`session_search { query?, scope?, working_dir?, limit?, include_current?, include_tools?, turns?, stats? }` — at least one of `query`/`turns`/`stats`; `limit` default 10 / max 50; `scope` default `all`; `include_current` default false.

## 5. Non-goals (v1)
Cross-**harness** import (Claude Code / Codex / opencode stores — the separate `jcode-import-core` gap); embeddings/semantic memory; the Bloom-filter index (jcode's 657-LOC pre-filter is unneeded for one harness's store).

## 6. Sizing
Kernel `ToolContext.session_path` + loop fill **S**; the tool + scan + output **M**; tests (temp sessions dir fixtures: grouping, current-session exclusion, bounds, post-compaction recovery) **S–M**; docs **S**. Call it **M**.

## 7. References
- wcode: `crates/wcode-harness/src/{session,message,compaction,tool,loop_}.rs`, `crates/wcode-cli/src/{tools/mod.rs,repl.rs,session_groups.rs}`.
- jcode: `crates/jcode-app-core/src/tool/{session_search,conversation_search,session_search_index}.rs`.
