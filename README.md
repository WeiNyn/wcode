# wcode

A minimal, pi-like coding agent for the terminal. It streams an LLM conversation
and gives it content-addressed tools — read (anchors per line), grep/find,
bash, edit/replace, write — and stays out of the way. Built on a small kernel
([`wcode-harness`](crates/wcode-harness)) that speaks to any OpenAI-compatible
chat-completions endpoint.

## Install

```
cargo install --path crates/wcode-cli
```

## Configure

`~/.config/wcode/config.toml`:

```toml
model = "gpt-4.1"      # required
base_url = "https://api.openai.com/v1"  # optional; any OpenAI-compatible endpoint
api_key = "sk-..."     # optional
endpoint = "chat"      # optional; chat|responses, default chat
effort = "high"        # optional; free-style reasoning effort, omitted = not sent

[hooks]
rtk = "auto"           # optional; auto|true|false — route bash output through the
                       # rtk proxy (https://github.com/rtk-ai/rtk) to cut tokens
                       # (default auto = on only if the rtk binary is on PATH)

[tools]
grep = true            # optional; true|false — register the grep tool (off by default)
find = true            # optional; true|false — register the find tool (off by default)

[compaction]
# Compact the conversation as it grows. An absent table = harness defaults.
budget = 100000            # optional; working token ceiling to compact near
                           # (default: the model's context window)
window = 200000            # optional; context-window override for unknown models
min_remaining = 16384      # compact once fewer than this many tokens remain
keep_recent_tokens = 20000 # recent verbatim context kept after compacting, in tokens
keep_recent_turns = 2      # complete turns always kept, whatever their size

[instructions]
# Instruction ("reference") files folded into the system prompt. An absent table
# = discover the candidate names from the working dir up to the repo root, plus
# a global file in the config dir, merged global-first. `file` overrides
# discovery with a single name/path; "off" disables.
file = "AGENTS.md"          # optional; load exactly this instead of discovery
names = ["AGENTS.md"]       # optional; candidate names per directory
                            # (default: AGENTS.override.md, AGENTS.md, CLAUDE.md)
global = true               # optional; also load one from ~/.config/wcode

[skills]
# Discover `SKILL.md` packages and fold their name + description into the system
# prompt; the body loads on demand when the agent `read`s the path. An absent
# table = discover from the standard roots.
enabled = true              # optional; discover skills at all. Default true
dirs = ["./team-skills"]    # optional; extra roots, scanned first
disabled = ["noisy-skill"] # optional; names to skip

[retry]
# Retry the connect/handshake (and the first streamed item) on transient
# failures — 429/5xx statuses and transport errors. An absent table = defaults.
max = 3                    # optional; retries after the first attempt (0 disables)
base_ms = 500              # optional; base backoff for the first retry
cap_ms = 8000              # optional; cap on a single backoff wait
```

Environment variables beat the toml: `WCODE_BASE_URL`, `WCODE_API_KEY`, and
`OPENAI_API_KEY` as a key fallback, plus `WCODE_ENDPOINT`, `WCODE_EFFORT`,
`WCODE_RTK` and `WCODE_GREP`/`WCODE_FIND` (which override `tools.grep`/
`tools.find`), and `WCODE_COMPACT_BUDGET`, `WCODE_COMPACT_WINDOW`,
`WCODE_COMPACT_MIN_REMAINING`, `WCODE_COMPACT_KEEP_RECENT_TOKENS`,
`WCODE_COMPACT_KEEP_RECENT_TURNS`, and `WCODE_INSTRUCTIONS` (a file name/path, or
`off`), `WCODE_SKILLS` (extra skill roots, or `off`), and `WCODE_RETRY_MAX`,
`WCODE_RETRY_BASE_MS`, `WCODE_RETRY_CAP_MS` (the
`[retry]` table).
`--model` / `--base-url` / `--endpoint` / `--effort` override a
successfully loaded config (and `--model` rescues a missing `model`), but
cannot rescue an unreadable or invalid config.toml — that still exits with
an error. `--effort -` (or `none`/`off`) clears back to send-nothing.
`--no-instructions` skips the instruction files; `--dump-system-prompt` prints
the composed system prompt (instructions included) and exits — no model needed.

Keyless local OpenAI-compatible endpoints (Ollama, llama.cpp, vLLM, ... over
`localhost` / `127.0.0.1` / `[::1]`) work without a key — wcode sends a
placeholder bearer token so `--base-url http://localhost:11434/v1` just works.

## Use

```
wcode                        # REPL
wcode -p "explain this repo" # one-shot: run, print reply, exit
wcode --resume               # continue the latest session
wcode --no-session --model m --base-url http://localhost:11434/v1
wcode --list-models          # print GET {base_url}/models ids, exit
wcode --no-instructions      # run without loading instruction files
wcode --dump-system-prompt   # print the composed system prompt, exit
wcode --no-skills            # run without discovering skills
```

REPL commands (unknown `/...` lines go to the LLM as prompt text):

| command | effect |
|---|---|
| `/exit` | quit |
| `/new` | fresh conversation + session file |
| `/model <id>` | switch model mid-conversation (logged as a `ModelChange`); bare `/model` lists models |
| `/models [filter]` | list `GET {base_url}/models` ids, `*` marks current; optional case-insensitive substring filter |
| `/effort [level]` | show/set reasoning effort (free-style, logged as `EffortChange`); `/effort -` clears to send-nothing |
| `/resume [path]` | reopen a session and replay its history as context (default: latest); restores model + effort |
| `/sessions` | list sessions, `*` marks the current one |
| `/usage` | print aggregate token usage for the current conversation (input/output, plus cache read/write when reported) |
| `/compact [prompt]` | summarize older messages now, keeping the most recent; an optional `prompt` focuses the summary |
| `/reload [--no-session]` | `cargo build --bin wcode` then re-exec into the same session (`--no-session` starts fresh); a failed build keeps the current binary running |

Effort fans out per endpoint: Chat sends `reasoning_effort`, Responses
sends `reasoning: { effort }`, unset sends nothing. Model listing only
returns id metadata — no capability flags — so effort support stays
user-managed.

Ctrl-C aborts the run in flight and stays in the REPL; when idle it exits.
Output: text streams to stdout, thinking and tool output are dimmed
(`⚙ name ✓ first-line-of-output`), errors to stderr.

## Tools

| tool | behavior |
|---|---|
| `read` | file contents as `ANCHOR│line` — the 5-char anchor is the line's content address and the `edit` target (no line numbers; `plain:true` restores `cat -n`). Anchors survive inserts/deletes above; they hash the line's raw content, so indentation is semantically meaningful (a nested `}` is a different anchor from a top-level one) — but a formatter that reindents moves an indented line's anchor (re-read after formatting). `offset`/`limit` page |
| `grep` | regex search; results carry anchors so a hit feeds straight into `edit` (`path:line  ANCHOR│line  <--`). **Respects `.gitignore`** (plus `.git`/`target`/`node_modules` as a built-in floor) and skips binaries; `no_ignore:true` searches ignored files too. Glob include filters, context, case-insensitive. **Registered only when `[tools] grep = true` (off by default — `bash` can search)** |
| `find` | glob-based file/dir listing, one path per line. **Respects `.gitignore`**; `no_ignore:true` lists ignored entries too. **Registered only when `[tools] find = true` (off by default — `bash` can list files)** |
| `bash` | `sh -c` in the working dir; stdout, labeled `[stderr]`, exit code; 30s default timeout, Ctrl-C kills |
| `ast_search` | AST-structural search via `ast-grep` (`$UPPERCASE` wildcards); **registered only when an `ast-grep`/`sg` binary is on PATH** |
| `ast_edit` | AST-structural rewrite of **one file** via `ast-grep` (`pattern`/`rewrite` with `$UPPERCASE` wildcards); `commit:false` dry-runs (diff only), default commits atomically and echoes the diff. **Registered only when an `ast-grep`/`sg` binary is on PATH** |
| `edit` | replace the line range covered by `from`/`to` anchors with `replacement`. Content-addressed: edits above never shift the target; stale/ambiguous anchors are rejected with candidates, nothing written. Echoes the fresh-anchor region so edits chain without re-reads. `replacement` is verbatim — preserve leading indentation; re-read after any external change first |
| `edits` | apply a batch of anchor-range edits (`{edits: [{path,from,to?,replacement,…}]}`) to **one file** in a single call. Every op resolves against the same snapshot and the batch is atomic — any stale/ambiguous/overlapping op aborts with nothing written |
| `replace` | exact string replace without a read for quick unique substitutions; fails on 0 or (without `replace_all`) multiple matches |
| `write` | create/overwrite; parents created automatically. Content is written byte-for-byte — indentation preserved, never reformatted |

`edit`/`replace`/`write` share a mutation lock and write via a temp file +
rename, so concurrent file mutation can't interleave or truncate.

## Design: content-addressed editing

Line numbers are positional — insert one line and every number below silently
means something else, the classic way agents corrupt files. wcode `read`/`edit`
address lines by a 5-char **content anchor** instead (`XXa1b│fn main() {`),
following the hashline ideas in `pi-better-edit`:

- `anchor(line)` is a pure hash of the line's **raw content** (a trailing `\r`
  is dropped for CRLF/LF portability). Inserting or deleting lines elsewhere
  *never* changes an intact line's anchor; whitespace is part of the address, so
  `{`, `  {` and `\t{` are distinct (a nested brace is directly addressable),
  and `foo bar` ≠ `foobar`. Cost: a formatter that reindents moves indented
  lines' anchors — re-read before editing after a format.
- `edit` sends `from`/`to` anchors + the new text — old code is never re-typed
  (token savings) and the target can't drift.
- Identical lines intentionally share an anchor; `edit` **rejects** ambiguous
  targets with the candidate line numbers (or `old_string` to pin one) instead
  of guessing. Stale anchors (line changed since `read`) get the same
  reject-with-a-hint treatment.
- Fully stateless: no anchor store, no session memory, so external edits
  (bash, formatters, other tools) are always picked up — `read`/`edit`
  recompute anchors from current file contents every call.

Structural search is a separate axis: `ast_search`/`ast_edit` shell out to an
`ast-grep` binary (the legacy `sg` alias is the fallback; the same auto-detect
pattern as the rtk hook).

## Skills

A skill is a directory with a `SKILL.md` — YAML frontmatter (`name`,
`description`) plus a body:

```markdown
---
name: pdf-tools
description: Extract text and tables from PDFs. Use for PDF documents.
---

# PDF tools
Run `scripts/extract.sh <file>`.
```

Only `name` and `description` enter the system prompt (as an `# Available
skills` section); the body loads on demand — the agent `read`s the SKILL.md
when a task matches. That is progressive disclosure: many skills cost many
one-line entries, not many bodies. A skill may ship `scripts/`, `references/`,
`assets/`; the body refers to them by relative path.

Roots, highest priority first (first `name` wins):

1. `[skills] dirs` / `WCODE_SKILLS` explicit roots,
2. the working dir and its ancestors up to the repo root — `.wcode/skills/`,
   then the shared dir `.agents/skills/` (falling back to `.claude/skills/`
   where `.agents` is absent),
3. global — `~/.local/share/wcode/skills/`, then the same shared pair.

Malformed skills (bad name, missing description, invalid YAML) warn on stderr
and are skipped. `--no-skills` disables discovery.

## Sessions

JSONL files in `~/.local/share/wcode/sessions`, named `<millis>_<id>.jsonl`
(`~/.local/share` is used even on macOS, matching the config convention).
One entry per message plus `model_change` / `effort_change` markers; torn
final lines are tolerated. `/resume` replays the history as the conversation
context and restores the last model + effort (`--model` / `--effort` flags
win when passed alongside `--resume`).

## Philosophy

pi-like, deliberately small. The kernel is a few hundred lines — event loop,
tool dispatch, hooks, JSONL sessions — and the CLI is a thin client. Absent by
design: MCP, subagents, permission prompts, approval flows, config files for
behavior. That logic belongs in code, via `Hooks`: block or rewrite tool calls
(`before_tool_call`), transform context per turn, force stops. Build the
variant you want instead of configuring one.

### Extending

- **Tools** — implement `TypedTool` (typed args + schema via schemars), wrap
  with `erased()`, add to `default_tools()`.
- **Policies** — implement `Hooks` (`before_tool_call` returning a reason
  blocks execution and feeds the reason back to the LLM as an error).
- **Providers** — implement `StreamFn`: context + system prompt + tool defs
  in, stream of `LlmStreamEvent` out. `rig_stream_fn()` is the built-in
  OpenAI-compatible adapter.
