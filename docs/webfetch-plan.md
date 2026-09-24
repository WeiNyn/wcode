# wcode — `webfetch` (one-shot URL fetch)

Status: **design — decisions D1–D12 locked, Q1–Q5 settled by the first-layer review
(verdict: BLOCK → fixes folded); ready to sketch.** Companion to
[`gap-analysis-jcode.md`](gap-analysis-jcode.md) §3 and the tracker item 35.

## 1. Why

wcode cannot read a URL. The model can't pull a doc page, a GitHub issue, a raw
file from a repo, or an RFC — it can only work from what's on disk. `bash` could
`curl`, but that is a poor fit: `curl`/`wget` may be absent, the shell has no
timeout discipline for a slow host, and a raw HTML page (often >150 KB) lands in
the context unstripped and unbounded. A native `webfetch` closes the URL gap the
way `read` closes the file gap: one GET, HTML reduced to readable text, output
capped, errors fed back as a recoverable tool result.

## 2. Ground truth

- **Tool authoring.** A tool is a `TypedTool` (`crates/wcode-harness/src/tool.rs`):
  an `Args: DeserializeOwned + JsonSchema`, a `name()`/`description()`, and an
  `async fn execute(&self, args, &ToolContext) -> ToolOutput`. Wrap with `erased()`
  and add to `default_tools()` (`crates/wcode-cli/src/tools/mod.rs`). Read-only
  tools override `parallel_safe() -> true`; mutators override `mutating() -> true`
  (kept in sync with `hooks::MUTATING_TOOLS` by a CLI test).
- **`ToolOutput`** (`tool.rs`) is `{ output: String, is_error: bool, diff, path }`;
  `diff`/`path` are presentation-only and stay `None` here. `ToolContext` carries
  `cancel: CancellationToken`, `working_dir`, and an `events` sender.
- **Config gating.** `ToolsConfig` (`crates/wcode-cli/src/config.rs:50`) gates
  `grep`/`find` **off by default** because they are redundant with `bash`; env
  overrides `WCODE_GREP`/`WCODE_FIND`. `default_tools` reads the resolved config.
  `webfetch` is **not** gated (D6).
- **HTTP client.** The CLI declares **no** HTTP client today. `rig-core` 0.42
  (a workspace dep) already resolves **`reqwest` 0.13.4** with the **`rustls`**
  feature active — the workspace declares
  `rig = { version = "0.42", default-features = false, features = ["reqwest", "rustls"] }`,
  and `cargo tree -e features -i reqwest@0.13.4` shows `hyper-rustls`/`rustls`/
  `tokio-rustls`/`rustls-platform-verifier` already compiled. So adding
  `reqwest = { version = "0.13", default-features = false, features = ["rustls"] }`
  to `crates/wcode-cli/Cargo.toml` pulls **no new transitive crates**.
  - **Feature-name trap (build-breaking).** In reqwest 0.13 the feature is
    **`rustls`**; 0.12's `rustls-tls` was renamed. jcode pins `reqwest = "0.12"`
    (`../jcode/crates/jcode-app-core/Cargo.toml`) and uses `rustls-tls` — copying
    that string into wcode fails with *"feature `rustls-tls` does not exist"*.
    Use `rustls`. The sketch's first commit must run
    `cargo tree -e features -i reqwest@0.13.4` before/after to prove the
    no-new-crates claim empirically (this was verified by feature-graph analysis,
    not an applied `cargo update`).
  - rig's own `Client` wraps a pooled `reqwest::Client` but is not exposed for
    reuse (`crates/wcode-harness/src/streamfn.rs:86`).
- **Timeout/cancel pattern.** `bash` (`crates/wcode-cli/src/tools/bash.rs`) takes a
  `timeout_secs` arg, defaults via **`DEFAULT_TIMEOUT_SECS`** (30), and races the
  work against `ctx.cancel` with `tokio::select!`, returning
  `ToolOutput { output: "cancelled", is_error: true }` on cancel. `webfetch` mirrors
  this shape (D9).
- **Reference implementation.** jcode's `webfetch`
  (`../jcode/crates/jcode-app-core/src/tool/webfetch.rs`): GET only; args
  `url`/`format`/`timeout`; a `Mozilla/5.0 (compatible; JCode/1.0)` UA; a 5 MiB body
  cap (content-length reject + streaming truncate); regex HTML→text/markdown; a
  40 000-char output cap cut at a char boundary preferring the last newline;
  non-2xx → `HTTP error: <status>`. A shared client via `shared_http_client()`.
  reqwest 0.13.4's default redirect policy is 10 hops (`redirect.rs`).

## 3. Decisions (locked)

- **D1 — GET only, no request control.** Args: `url` (required), `format`
  (`"text" | "markdown" | "html"`, default `markdown`), `timeout` (seconds, default
  30, clamped to max 120). No method/headers/body/cookies.
- **D2 — HTML → text via a small regex converter, no HTML-parser dependency.**
  Strip `<script>`/`<style>`/comments and non-prose chrome (`nav`, `aside`, `form`,
  `select`, `svg`, `iframe`, `template`, `dialog`, `canvas`, `noscript`); map
  headings/links/emphasis/code/lists to markdown; decode a handful of entities.
  **Exact format matrix** (jcode parity — do not improvise):

  | `format` | `content-type` contains `text/html` | otherwise |
  |---|---|---|
  | `html` | raw body | raw body |
  | `text` | `html_to_text(body)` | `html_to_text(body)` |
  | `markdown` (default) / unknown | `html_to_markdown(body)` | body |

  (`text` always runs the stripper, matching jcode; on a non-HTML body this is a
  near no-op.) The converter must reproduce jcode's three general rules:
  - **Links** — drop an anchor with empty text; keep the text and drop the target
    for a `#fragment` href or an href longer than `MAX_URL_CHARS` (300); otherwise
    emit `[text](href)`.
  - **Empty-bullet cleanup** — collapse list items left empty after tag stripping.
  - **Attribute-safe tag regex** — the tag stripper must match quoted attribute
    values *before* a bare `>`, so a `data-mw='…>…'`-style payload (Parsoid) does
    not leak its contents into the text.
  The converter is a pure `&str -> String` fn, unit-tested against fixtures.
- **D3 — Two caps, both enforced.** Body: 5 MiB — reject early on a
  `content-length` over the cap, else truncate while streaming. Output: 40 000
  chars, cut at a char boundary preferring the last newline, with an appended
  truncation note.
- **D4 — Failures are `ToolOutput { is_error: true }`, never a panic.** A URL not
  starting with `http://`/`https://` is refused; a non-2xx status yields
  `HTTP error: <status>`; a transport error yields its message.
- **D5 — Read-only.** `parallel_safe() -> true`, `mutating() -> false`, no
  workspace effect, `diff`/`path` stay `None`.
- **D6 — Always on (locked).** A core read-only capability like
  `read`/`session_search`; **no** `[tools]` flag. `webfetch` is *not* redundant with
  `bash` the way `grep`/`find` are, and doctrine forbids behavior config.
- **D7 — Transport.** A fixed descriptive `User-Agent` (`wcode/<version>`);
  reqwest's default redirect policy (10 hops); rustls TLS. No proxy/env wiring
  beyond reqwest's own.
- **D8 — Placement.** A CLI tool (`crates/wcode-cli/src/tools/webfetch.rs`): it needs
  an HTTP client, and the kernel (`wcode-harness`) stays free of network egress.
  Registered unconditionally in `default_tools()`.
- **D9 — Cancel.** The request races `ctx.cancel` (bash parity): a mid-flight
  cancel aborts the fetch and returns
  `ToolOutput { output: "cancelled", is_error: true }`.
- **D10 — Shared HTTP client.** One `reqwest::Client` lives on the tool struct,
  built once in `new()` (not per call) — TLS-pool reuse, and it keeps the tool
  `parallel_safe`-sound (a shared `Client` is `Sync` + cheap to clone).
- **D11 — Plan mode: allowed, no code change.** `webfetch` is absent from
  `MUTATING_TOOLS` (`crates/wcode-harness/src/hooks.rs:62`) and is not `bash`, so
  `PlanModeHooks::before_tool_call` (`hooks.rs:98`) already permits it. Plan mode
  gates *workspace mutation*; `bash` already permits arbitrary network egress, so a
  read-only fetch needs no new gate.
- **D12 — SSRF/localhost guard: out for v1.** `bash` already allows arbitrary
  egress (`curl http://169.254.169.254/…`), so webfetch's SSRF surface is a strict
  **subset** of the existing one — a guard here would be theater. Recorded as a
  follow-up, not v1.

## 4. Interface

```
webfetch {
  url:     String,                                   // http(s) only
  format?: "text" | "markdown" | "html",             // default "markdown"
  timeout?: u64,                                     // seconds, default 30, max 120
}
```

Returns `Fetched <url> (<bytes>)\n\n<converted body>`, with a trailing
`(output truncated …)` note when D3's output cap bites. **`<bytes>` is the
converted-body length** (jcode's `full_len`, post-format), not the raw download
size.

## 5. Non-goals (v1)

`websearch` (needs a search backend — its own gap); POST/headers/auth/cookies;
PDFs and binaries (a lossy UTF-8 decode or a refusal, not a parser); JS-rendered
pages (static HTML only); response caching; a per-host allowlist; an SSRF guard
(D12); a rich HTML→markdown engine (tables, nested lists stay crude).

## 6. First-layer review — resolutions

Verdict **BLOCK** (one build-breaker + un-made decisions), now folded: the
`rustls` feature name (D2/§2), the format matrix and converter rules (D2), and
D9–D12. Q1–Q5 settled:

- **Q1 registration → always on** (D6).
- **Q2 converter → full jcode parity** (D2), with the link rules, empty-bullet
  cleanup, and attribute-safe tag regex named.
- **Q3 plan mode → allow, no hook change** (D11).
- **Q4 SSRF → out for v1**, on the subset argument (D12).
- **Q5 framing → keep** `Fetched <url> (<bytes>)`, `<bytes>` = converted length (§4).

**Doc/test touchpoints the sketch must include (larger than a one-liner):**
- README **Tools** table (`README.md:220`) — add a `webfetch` row.
- README **parallel-safe list** (`README.md:237` — "`read`/`grep`/`find`/`ast_search`
  are read-only and opt in") — add `webfetch`.
- The always-on test `default_tools_omit_grep_and_find`
  (`crates/wcode-cli/src/tools/mod.rs`) pins the core tool list — add `webfetch`
  there. (Grep for any other test asserting the exact read-only set first.)

## 7. Sizing & test plan

`reqwest` dep **S** (after the `rustls` fix); the tool + converter + caps **M**;
docs (README Tools table + parallel-safe list + the `default_tools` test) **S–M**.
Call it **M**.

**Test seam (no live network).** Non-2xx / body-cap / cancel paths must not hit the
internet: stand up a local `tokio::net::TcpListener` in the test that serves canned
responses (the repo avoids heavy dev-deps — a raw listener, not a mock-server
crate). Converter unit tests use inline HTML fixtures (mine
`../jcode/crates/jcode-app-core/src/tool/webfetch_corpus_tests.rs` for the set).

## 8. References

- wcode: `crates/wcode-harness/src/tool.rs`, `crates/wcode-cli/src/tools/{mod.rs,bash.rs}`,
  `crates/wcode-cli/src/config.rs`, `crates/wcode-cli/Cargo.toml`, `README.md`,
  `crates/wcode-harness/src/hooks.rs`.
- jcode: `crates/jcode-app-core/src/tool/webfetch.rs` (+ `webfetch_corpus_tests.rs`).
