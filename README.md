# wcode

A minimal, pi-like coding agent for the terminal. It streams an LLM conversation
and gives it four tools — read, bash, edit, write — and stays out of the way.
Built on a small kernel ([`wcode-harness`](crates/wcode-harness)) that speaks to
any OpenAI-compatible chat-completions endpoint.

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
```

Environment variables beat the toml: `WCODE_BASE_URL`, `WCODE_API_KEY`, and
`OPENAI_API_KEY` as a key fallback, plus `WCODE_ENDPOINT`, `WCODE_EFFORT` and
`WCODE_RTK`.
`--model` / `--base-url` / `--endpoint` / `--effort` override a
successfully loaded config (and `--model` rescues a missing `model`), but
cannot rescue an unreadable or invalid config.toml — that still exits with
an error. `--effort -` clears back to send-nothing.

## Use

```
wcode                        # REPL
wcode -p "explain this repo" # one-shot: run, print reply, exit
wcode --resume               # continue the latest session
wcode --no-session --model m --base-url http://localhost:11434/v1
wcode --list-models          # print GET {base_url}/models ids, exit
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
| `read` | file contents with line numbers, 1-based `offset`/`limit` |
| `bash` | `sh -c` in the working dir; stdout, labeled `[stderr]`, exit code; 30s default timeout, Ctrl-C kills |
| `edit` | exact string replace; fails on 0 or (without `replace_all`) multiple matches |
| `write` | create/overwrite; parents created automatically |

`edit`/`write` share a mutation lock and write via a temp file + rename, so
concurrent file mutation can't interleave or truncate.

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
