# wcode

A minimal, pi-like coding agent for the terminal. It streams an LLM conversation
and gives it content-addressed tools — read (anchors per line), grep/find, bash,
edit/edits/replace, write, and ast-grep search/rewrite — and stays out of the way.
Built on a small kernel
([`wcode-harness`](crates/wcode-harness)) that speaks to any OpenAI-compatible
chat-completions endpoint.

## Install

Install with [Homebrew](https://brew.sh) — a tap by URL, no extra repo:

```
brew tap WeiNyn/wcode https://github.com/WeiNyn/wcode
brew install wcode
```

From crates.io (a Rust toolchain is required):

```
cargo install wcode-cli
```

Or grab a prebuilt binary — Linux (x86_64, aarch64) and macOS (Intel, Apple
Silicon) tarballs are attached to each
[release](https://github.com/WeiNyn/wcode/releases). Build the latest from source
with `cargo install --path crates/wcode-cli`.

More at **[weinyn.github.io/wcode](https://weinyn.github.io/wcode/)**.

## Configure

`~/.config/wcode/config.toml`:

```toml
model = "gpt-4.1"      # required
base_url = "https://api.openai.com/v1"  # optional; any OpenAI-compatible endpoint
api_key = "sk-..."     # optional
endpoint = "chat"      # optional; chat|responses, default chat
effort = "high"        # optional; free-style reasoning effort, omitted = not sent

# Per-model providers: when a model id is selected (mid-session `/model`, a
# worker's `spec.model`, or `--model`), this entry's endpoint/base_url/api_key
# re-point the provider. Absent fields inherit the globals above. Quote ids that
# contain dots or slashes.
[models."gpt-5.1"]
endpoint = "responses"
base_url = "https://api.openai.com/v1"
api_key = "sk-..."

[hooks]
rtk = "auto"           # optional; auto|true|false — route bash output through the
                       # rtk proxy (https://github.com/rtk-ai/rtk) to cut tokens
                       # (default auto = on only if the rtk binary is on PATH)

[tools]
grep = true            # optional; true|false — register the grep tool (off by default)
find = true            # optional; true|false — register the find tool (off by default)
parallel = true        # optional; true|false — run independent tool calls from one batch
                       # concurrently (default true; --sequential forces it off)

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
ttft_ms = 60000            # optional; time-to-first-token timeout (0 disables)
idle_ms = 120000           # optional; inter-item idle timeout (0 disables)

[theme]
# TUI theme, presentation-only. `name` selects a built-in preset —
# default | dark | light | gruvbox-dark | nord | solarized-dark | solarized-light.
# `default` is named-16 (safe on any terminal); the others are truecolor, so they
# are a no-op on a non-truecolor terminal (a `light` preset also needs a light
# terminal). A role value is a named ANSI color (`cyan`, `light-yellow`,
# `dark-gray`, ...) or hex `#rrggbb`; absent roles inherit the palette. Roles:
# accent, dim, muted, border, user, body, error, success, warn, code, heading,
# heading_sub, link, tool_name, thinking, diff_add, diff_del. Hex is honored only
# under truecolor; NO_COLOR wins over everything. Select a preset live with the
# TUI's `/theme [name]` (bare `/theme` opens a picker) or start with `--theme <name>`;
# `--list-themes` prints the catalog. Explicit role keys survive a preset switch.
name = "gruvbox-dark"      # optional; a catalog preset (default: palette B)
link = "light-cyan"        # optional; role = color (overrides the preset)
muted = "#5f5f5f"          # optional; hex is truecolor (opt-in)

# A preset team the orchestrator starts with (requires --agents). One [[team]]
# entry per member; names are unique (a duplicate fails the load loudly). Keys:
#   name      (required) the member's address, unique
#   role      extra system-prompt text (`# Role`) appended to the worker blurb
#   model     model id override (default: the orchestrator's)
#   tools     tool allow-list (`message` is always kept; unknown names rejected)
#   read_only enforce read-only: refuse the mutating tools and mutating bash/bg
#   effort    reasoning-effort override (`-`/`none`/`off` clears; default: inherit)
#   base_url  provider base URL override (default: the orchestrator's)
#   api_key   provider API key override (default: the orchestrator's)
[[team]]
name      = "explorer"
role      = "recon only; never edit; cite file:line"
tools     = ["read", "grep", "find"]
read_only = true

[[team]]
name   = "reviewer"
role   = "verify diffs; PASS / NITS / FAIL"
effort = "high"
```

### `[workflow]` — a plan template (requires `--agents`)

A single `[workflow]` table seeds the task DAG at boot. Each `[[workflow.node]]`
becomes a node, instantiated in **topological order** (so a `depends_on` may name
a later-authored sibling):

```toml
[workflow]
max_attempts = 3            # optional; the rework cap (default 3)

[[workflow.node]]
id = "explore"              # required; a unique label
member = "explorer"         # EXACTLY ONE of member | script

[[workflow.node]]
id = "implement"
member = "builder"
depends_on = ["explore"]    # sibling ids; resolved to Task ids at boot

[[workflow.node]]
id = "verify"
depends_on = ["implement"]
gate = true
script = "cargo test --workspace"   # a physical gate (exit code = verdict)
```

A node is a `member` (a `[team]` name) OR a `script` (`sh -c`, run under the bash
automation policy); `gate = true` makes its reject re-open its deps, and a gate
needs at least one `depends_on`. The graph is validated acyclic at load. The plan
shows in the TUI's `/tasks` listing as
`#id [state] title (owner) ← #deps ×attempts`.

```toml
[orchestrator]
# Root-only workflow guidance (requires --agents), folded into the system prompt
# as a `# Orchestrator workflow` section after the team roster.
guidelines = """
Step 1: message a member by name; Step 2: verify their result.
"""

[peers]
# Persistent phonebook entries for `message`'s `to` (requires --agents). A value
# containing `:` is an address alias (`agent:<id>`); otherwise it is a socket
# path, connected lazily (Unix only).
explorer = "/tmp/wcode/explorer.sock"
helper   = "agent:w7"
```

Environment variables beat the toml: `WCODE_BASE_URL`, `WCODE_API_KEY`, and
`OPENAI_API_KEY` as a key fallback, plus `WCODE_ENDPOINT`, `WCODE_EFFORT`,
`WCODE_RTK` and `WCODE_GREP`/`WCODE_FIND` (which override
`tools.grep`/`tools.find`), and `WCODE_COMPACT_BUDGET`, `WCODE_COMPACT_WINDOW`,
`WCODE_COMPACT_MIN_REMAINING`, `WCODE_COMPACT_KEEP_RECENT_TOKENS`,
`WCODE_COMPACT_KEEP_RECENT_TURNS`, and `WCODE_INSTRUCTIONS` (a file name/path, or
`off`), `WCODE_SKILLS` (extra skill roots, or `off`), and `WCODE_RETRY_MAX`,
`WCODE_RETRY_BASE_MS`, `WCODE_RETRY_CAP_MS`, `WCODE_RETRY_TTFT_MS`,
`WCODE_RETRY_IDLE_MS` (the `[retry]` table).

Provider precedence is `[models.<id>]` > flags/env/config: the toml globals,
`WCODE_*`, and `--base-url`/`--endpoint` together set the **default provider**;
a `[models.<id>]` entry for the selected model overrides the
`endpoint`/`base_url`/`api_key` fields it names (an unmapped model — or a field
the entry omits — uses the default).

`--config <path>` (or `WCODE_CONFIG`; flag beats env, relative to the cwd)
points at an **overlay** file deep-merged over the global config: tables merge
per key, so an overlay's `[tools] grep = true` adds that flag without clobbering
the rest of the global `[tools]`, while a scalar or array is replaced wholesale.
This keeps team/model overlays (per repo, per project) out of the provider
config. A missing or unparseable overlay is an error.

`--model` / `--base-url` / `--endpoint` / `--effort` / `--config` override a
successfully loaded config (and `--model` rescues a missing `model`), but
cannot rescue an unreadable or invalid `config.toml` — that still exits with an
error. `--effort -` (or `none`/`off`) clears back to send-nothing. When `--model`
selects an id, its `[models.<id>]` entry overrides the default provider for the
fields it names, and switching back to an unmapped model restores the default
provider (a model switch is reversible).

`--no-instructions` skips the instruction files; `--dump-system-prompt` prints
the composed system prompt (instructions included) and exits — no model needed.

With `--agents` and a `[team]`, wcode spawns each member at startup and the
root's system prompt gains a `# Your team` block listing them by name (and role);
members are addressed by name with `message`. A `[team]` without `--agents` is an
error, and a served `--owner` worker never sees the block. An
`[orchestrator] guidelines` string (also requiring `--agents`) is appended as a
`# Orchestrator workflow` section after the team roster, so the workflow can
reference the team. The repo ships `.wcode/team.toml` as exactly this — an
overlay that adds the dev team, the orchestrator workflow, and
`[tools] grep/find = true`, leaving your provider/model in the global config:
run it with `wcode --agents --config .wcode/team.toml` (or
`WCODE_CONFIG=.wcode/team.toml wcode --agents`).

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
wcode --list-themes          # print the built-in theme names, exit
wcode --theme nord           # select a theme preset (or `[theme] name` in config)
wcode --no-instructions      # run without loading instruction files
wcode --dump-system-prompt   # print the composed system prompt, exit
wcode --no-skills            # run without discovering skills
wcode --sequential           # run each tool call one at a time
wcode --tui                  # force the full-screen TUI (default on a TTY)
wcode --no-tui               # force the line REPL (pipes/CI)
```

The interactive path: with no `-p`, `wcode` starts a full-screen TUI when stdin
and stdout are a terminal, and the line REPL otherwise. `--tui`/`--no-tui` force
either; `-p`, `serve`, and piped input always take the non-TUI path. The TUI
renders the same `AgentEvent` stream, over a local session or a `--socket` one.

TUI keys: `Enter` submit · `Shift-Enter`/`Ctrl-J` newline · `Up`/`Down` recall
prompts · `PgUp`/`PgDn` (or the wheel) scroll · `Esc`/`Ctrl-C` cancel a run or
quit · `Ctrl-Y` copy the last reply (OSC-52) · `Ctrl-T` expand/collapse every
tool's output · `Ctrl-N`/`Shift-Tab` (or `Alt-1..9`) switch surface · `Ctrl-B`
toggle the sidebar · `Ctrl-A`/`E`/`W`/`U`/`K` readline-style input editing.
`Ctrl-G` enters **browse mode**, a `▌` cursor over the transcript (`j`/`k` next/
prev, `g`/`G` first/last, `Enter` expand/collapse the selected block, `y` copy
it, `Esc`/`q`/`Ctrl-G` leave). `F1` (or `/help`) opens the full keymap. The input
box wraps and grows (capped at 8 rows, or half the screen, then scrolls to the
cursor); a paste over 100 chars or more than 3 lines collapses to a `❰ pasted … ❱`
chip; the full text is sent (outer whitespace trimmed). TUI `/`-commands: `/exit`, `/model <id>`, `/effort
[level]`, `/theme [name]` (bare `/theme` opens a picker), `/compact [text]`, `/usage`, `/copy`, `/team`,
`/surface`, `/help` — the
subset that maps to a `Request` under the current session — plus `/changes` (list
the files this run changed, re-showing a diff), `/resume` (pick a session; the
CLI re-execs into it), `/new` (alias `/clear`; start a fresh session — a new
group for a team — by re-exec'ing with no `--resume`; refused over a socket), and
[--no-session]` (rebuild + re-exec into the current session, mirroring the REPL;
refused over a socket). The remaining session-lifecycle commands below stay in
the REPL.
`--agents` and a team, each member runs as its own **surface** (the root, plus one
per member): input and `/`-commands go to the **focused** surface — switch with
`/surface` (a picker), `Ctrl-N`/`Shift-Tab` (cycle), or `Alt-1..9` — and a
right-hand sidebar lists the members (`label · model · state`, idle/running/done,
the focused one bolded; shown at ≥ 60 columns, `Ctrl-B` toggles it). `/team`
prints the roster.

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
| `/btw <question>` | a tool-free side question answered from the current context (never committed); a dim `btw: …` prints while it is in flight — the TUI's status corner shows `⠹ btw…` until the reply |
| `/compact [prompt]` | summarize older messages now, keeping the most recent; an optional `prompt` focuses the summary |
| `/skills` | list the discovered `SKILL.md` packages (name, description, file) |
| `/skill <name> [args]` | force-load a skill's body into a turn — for when the model doesn't pick it up from the prompt section on its own; extra `args` become the task |
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
| `webfetch` | GET a URL over http(s); HTML reduced to readable text. `format` = `text`/`markdown`/`html` (default `markdown`); `timeout` seconds (default 30, max 120). Body capped at 5 MiB, output at 40k chars. Read-only, always registered |
| `read` | file contents as `ANCHOR│line` — the 5-char anchor is the line's content address and the `edit` target (no line numbers; `plain:true` restores `cat -n`). Anchors survive inserts/deletes above; they hash the line's raw content, so indentation is semantically meaningful (a nested `}` is a different anchor from a top-level one) — but a formatter that reindents moves an indented line's anchor (re-read after formatting). `offset`/`limit` page |
| `grep` | regex search; results carry anchors so a hit feeds straight into `edit` (`path:line  ANCHOR│line  <--`). **Respects `.gitignore`** (plus `.git`/`target`/`node_modules` as a built-in floor) and skips binaries; `no_ignore:true` searches ignored files too. Glob include filters, context, case-insensitive. **Registered only when `[tools] grep = true` (off by default — `bash` can search)** |
| `find` | glob-based file/dir listing, one path per line. **Respects `.gitignore`**; `no_ignore:true` lists ignored entries too. **Registered only when `[tools] find = true` (off by default — `bash` can list files)** |
| `bash` | `sh -c` in the working dir; stdout, labeled `[stderr]`, exit code; 30s default timeout, Ctrl-C kills. An always-on catastrophic-risk gate (`BashRiskHooks`) refuses device writes, `mkfs`/`wipefs`/`shred` on a device, recursive `rm`/`chmod`/`chown` of `/`/`~`/`$HOME` (incl. the `/*` fold), and the fork bomb — returned as a `blocked:` tool error, no config knob |
| `bg` | manage tasks started with `bash { background: true }`. `list` (all tasks), `status <id>`, `output <id> [tail_lines]` (tail + the spill file path), `wait <id> [timeout_secs]` (block until it finishes; default 60s, max 300s), `kill <id>`. A finished task **pushes** `[message from bg]` back to the session — a turn if idle, next-turn context if running. Always registered |
| `ast_search` | AST-structural search via `ast-grep` (`$UPPERCASE` wildcards); **registered only when an `ast-grep`/`sg` binary is on PATH** |
| `ast_edit` | AST-structural rewrite of **one file** via `ast-grep` (`pattern`/`rewrite` with `$UPPERCASE` wildcards); `commit:false` dry-runs (diff only), default commits atomically and echoes the diff. **Registered only when an `ast-grep`/`sg` binary is on PATH** |
| `edit` | replace the line range covered by `from`/`to` anchors with `replacement`. Content-addressed: edits above never shift the target; stale/ambiguous anchors are rejected with candidates, nothing written. Echoes the fresh-anchor region so edits chain without re-reads. `replacement` is verbatim — preserve leading indentation; re-read after any external change first |
| `edits` | apply a batch of anchor-range edits (`{edits: [{path,from,to?,replacement,…}]}`) to **one file** in a single call. Every op resolves against the same snapshot and the batch is atomic — any stale/ambiguous/overlapping op aborts with nothing written |
| `replace` | exact string replace without a read for quick unique substitutions; fails on 0 or (without `replace_all`) multiple matches |
| `write` | create/overwrite; parents created automatically. Content is written byte-for-byte — indentation preserved, never reformatted |

`edit`/`replace`/`write` share a mutation lock and write via a temp file +
rename, so concurrent file mutation can't interleave or truncate.

When the model issues several tool calls in one message, the independent ones run
**concurrently** — `read`/`grep`/`find`/`ast_search`/`webfetch` are read-only and opt in;
mutating tools and `bash` are barriers, so no read can race a write inside a
batch. Results are still appended in call order. `--sequential` (or
`[tools] parallel = false`) restores strictly one-at-a-time execution.

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

On a clean interactive exit (`/exit`, or EOF; the TUI's `/exit`) wcode prints the
exact command to bring the session back, e.g.
`resume: wcode --resume <sessions>/<id>/root.jsonl --model <id> --agents` for a
team — copy-paste it to relaunch. A remote (`--socket`) session prints nothing.

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

## License

[MIT](LICENSE) © 2026 Wei
