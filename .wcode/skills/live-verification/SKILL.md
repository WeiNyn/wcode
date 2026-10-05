---
name: live-verification
description: "Use to prove a change behaves correctly against a real wcode binary — a keyless local endpoint, a refused port for retry/error paths, and --dump-system-prompt/--dump-config for prompt and config facts with no model. Not for changes that touch only docs or comments, and not as a substitute for the test suite."
---

# Live verification

`cargo build` proves nothing about behaviour. A change is verified by running the
real binary and observing the real output. This skill is the repertoire of
checks that need **no model** or a **keyless local** one.

## §0 — The principle

Three rungs, weakest to strongest:

1. **Compiles** — `cargo build`. Proves almost nothing.
2. **Tests pass** — `cargo test --workspace`. Proves the asserted behaviour,
   through whatever seam the test uses. Necessary, not sufficient.
3. **Runs correctly** — the binary, against a real endpoint or config, doing the
   thing. This is the proof the repo asks for: *prefer a live end-to-end check
   over unit tests alone* (`AGENTS.md`).

Pick the strongest rung the change allows. A pure-docs change stops at rung 1.

## §1 — Keyless local endpoint

Any OpenAI-compatible server on `localhost`/`127.0.0.1`/`[::1]` works without a
key — wcode sends a placeholder bearer token:

```sh
wcode --base-url http://localhost:11434/v1            # Ollama, llama.cpp, vLLM, …
wcode -p "explain this repo" --base-url http://localhost:11434/v1
```

Use this when the change touches the tool loop, streaming, or a tool's real
behaviour. A `--no-session` run keeps the check from littering the session dir:

```sh
wcode --no-session --base-url http://localhost:11434/v1 -p "run the failing test"
```

## §2 — A refused port for error/retry paths

To exercise the retry/backoff and error handling without a server, point at a
closed port and cap the retries:

```sh
WCODE_RETRY_MAX=2 wcode --base-url http://127.0.0.1:9/v1 -p "hi"
```

`127.0.0.1:9` refuses the connection; with `WCODE_RETRY_MAX=2` the run makes a
bounded number of attempts and then fails cleanly. Look for the retry lines and
a non-panic exit — not a hang and not a backtrace.

## §3 — Prompt/config facts with no model

Two flags print state and exit without contacting an endpoint:

```sh
# The composed system prompt (instructions + skills included).
cargo run -p wcode-cli -- --dump-system-prompt | sed -n '/Available skills/,/^$/p'

# The resolved provider/model and where each value came from (never the secret).
cargo run -p wcode-cli -- --dump-config
```

Use `--dump-system-prompt` to prove an instruction/skill/role section is folded
in, and `--dump-config` to prove a config-overlay precedence. Both need no
network — ideal for a change to prompt composition or config folding.

Related: `--list-models` prints `GET {base_url}/models` ids and exits;
`--list-themes` prints the theme catalog and exits.

## §4 — Reading the output honestly

- **Look for the exact line** your change should produce, and quote it in the
  report. "It worked" is not evidence.
- **Say what you could not run.** A change that needs an authenticated `gh` or a
  live provider you do not have is reported as unverified, not as verified.
- **A clean `cargo test` is not a live check** — it is rung 2. If the task
  demanded rung 3 and you ran only rung 2, say so.
- **Keep a check copy-pasteable.** A reviewer must be able to re-run it and see
  the same result.

## §5 — Checklist

- [ ] The check names the exact command and the exact expected output.
- [ ] A keyless endpoint is used where a model is needed, or the run is
      model-free (`--dump-system-prompt` / `--dump-config`).
- [ ] Error/retry paths use a refused port + `WCODE_RETRY_MAX`.
- [ ] The observed output is quoted, not paraphrased.
- [ ] Anything not verified is named as unverified.

## References

`README.md` (§"Configure" for the env table, §"Use" for the flags) ·
`AGENTS.md` (§"Verify against a real binary").

## Editing this file

Keep the frontmatter **quoted** and on one line. `description` ≤ 1024 chars,
clipped to 200 in the prompt. Validation: `crates/wcode-cli/src/skills.rs:efuUM`,
`:Vlnli`, `:ftQCJ`.
