# Workspace concurrency — whole-file digest CAS

> Status: **design decisions locked**; detail design pending. Companion to
> [`next-steps.md`](next-steps.md) (item 18). The minimal team-workspace answer
> to jcode's server-side swarm machinery — with **no stateful bus and no
> ownership register**.

## 1. Problem

wcode's team workspace (spawn/message/peers, session groups) lets several
agents work one repo, but nothing catches a **stale base**: an agent reads a
file, a peer edits it, and the first agent's write silently clobbers the peer's
change. Line-level anchors already catch this *per line*, but a whole-file
rewrite by peer A leaves B's line anchors "valid" while B's write overwrites A's
header changes — a lost update.

The destructive race is the aspect that matters, and a whole-file digest
catches it exactly. The one coordination problem the digest *cannot* see —
duplicate work ("two agents start on the same task") — is already handled
out-of-band: **the orchestrator allocates targeting and scope when it dispatches
a worker (the prompt)**. A separate ownership register would only duplicate that
allocation — and, being model-initiated, it would be advisory anyway. Dropped
(§2 D2).

## 2. Locked decisions

| # | Decision | Choice |
|---|----------|--------|
| D1 | Stale-base on digest mismatch | **Refuse-and-ask** — `E_STALE_DIGEST`-style: the mutation refuses and tells the agent to re-read. Never silently apply against a moved base. |
| D2 | Ownership register / claims | **Dropped.** The orchestrator already allocates targeting and scope by prompt; a `claim` tool would be a model-initiated extra call, advisory-only, and had no working release (the `Registry` has no deregistration — `crates/wcode-protocol/src/registry.rs`). The digest covers the destructive race; the rest is the prompt's job. |
| D3 | The `bash` hole | **Accept** — a bash command can modify files with no checksum possible. Document it; the tool-mediated path is what we protect. |
| D4 | Digest display | **Short prefix** — a truncated digest (≈12 hex chars / 48 bits) is enough to *compare*, not audit. |

## 3. Mechanism — the whole-file digest

- `read` already emits per-line anchors (which hash the FULL line — content
  digests today). Add a **whole-file content digest** (short-prefixed per D4;
  §5.4 settles the hash — the in-tree `hash64`) alongside them. Read-only, local,
  state.
- `edit` / `edits` / `replace` / `write` gain an optional `expected_digest`
  (or the harness auto-attaches the digest captured at the last `read` of that
  path, via a `Hooks` impl). On mismatch → **refuse** (D1), same shape as
  `E_STALE_ANCHOR` ("file changed since your read; re-read"), but whole-file —
  closing the per-line gap.
- This is pure compare-and-swap on the file's own bytes. It works across
  processes, sockets, hosts, and crashes. **The only shared state is the file
  itself.**

## 4. Where it plugs into wcode (structure sketch)

- `crates/wcode-cli/src/tools/`: `read` (+digest), `edit`/`edits`/`replace`/
  `write` (+`expected_digest`, refuse on mismatch).
- `crates/wcode-cli/src/workspace.rs` (new): a `WorkspaceHooks` impl — the
  policy: auto-attach the session's last-read digest to a mutation, refuse a
  stale write.
- Kernel gains nothing new: `Hooks` already provides every interception needed
  (`transform_tool_input` to attach the digest to a call; `after_tool_call` to
  capture the digest `read` emitted). The whole policy is an added `Hooks`
  impl, enabled by default (disable with `[workspace] digest_cas = false`), like
  `[team]`/`rtk` today.

## 5. Open questions — settled in the detail design

1. **Digest source of truth** → **per-call re-read-and-hash (stateless)**: each
   mutator hashes the bytes it already read; no harness cache is authoritative.
2. **`expected_digest` plumbing** → **harness auto-attach** (in
   `transform_tool_input`), with an explicit tool arg as a per-call override.
3. **Which tools get `expected_digest`** → **all four** (`write`/`edit`/`edits`/
   `replace`); `write` is the whole-file overwriter, the others close the
   per-line gap.
4. **Hashing** → **reuse the in-tree `anchor::hash64`**, truncated to 12 hex
   (48 bits); no crypto dependency. D4 truncates the display regardless, so a
   crypto hash buys no forgery resistance beyond the prefix — and the CAS is an
   optimistic-concurrency check, not a security boundary.

## 6. Non-goals

- **No ownership register, no `claims` map, no `claim` tool** (D2): targeting and
  scope are the orchestrator's, allocated by prompt.
- No stateful file-touch bus, no server-side touch history, no TTL sweeps
  (that is jcode's machinery; the digest makes it unnecessary).
- No permission prompts/approval flows (philosophy: advisory-only).
- No enforcement against `bash` writes (D3).
- No cross-process ownership (there is no register); the digest is the only
  thing that coordinates across processes.
