# Sketch — `crates/wcode-cli/src/frontmatter.rs` (NEW module)

**Status:** interface sketch, review-only. No working logic. Fillable as-is.
**Design:** `docs/project-team-loading.md` §Parser (CVm9v, KLi0N) — extract the
delimiter split out of `skills.rs::parse_frontmatter` so both `skills.rs` and the
new `agent_files.rs` share one rule. Behavior must be **unchanged** for skills.

**Grounding (current code, fresh anchors):**
- `crates/wcode-cli/src/skills.rs:249` (anchor `9870X`) — `fn parse_frontmatter(text: &str) -> Result<Frontmatter, String>`
  - BOM strip: `skills.rs:250` (anchor `iqmrI`) — `text.strip_prefix('\u{feff}')`
  - opening delimiter: `skills.rs:252`/`253` (anchors `bFiOT`/`N7Gdp`) — `"---\r\n"` or `"---\n"`
  - terminator: `skills.rs:255` (anchor `avEm6`) — `rest.find("\n---")`
  - YAML deserialize: `skills.rs:256` (anchor `rwobB`) — `serde_yaml_ng::from_str`
- `serde_yaml_ng` is already a dep: `crates/wcode-cli/Cargo.toml:28`.

## Module contents

```rust
// crates/wcode-cli/src/frontmatter.rs
//! Shared YAML-frontmatter split, extracted verbatim from
//! `skills.rs::parse_frontmatter` (design §Parser). One rule, two callers:
//! `skills.rs` (reads only the YAML) and `agent_files.rs` (reads YAML + body).
//!
//! The split is DELIBERATELY loose, matching today's behavior: the terminator is
//! the first bare `\n---` substring (so `\n---x` / `\n----` also terminate). Do
//! NOT "tighten" this — that would change skills behavior.

/// Split a leading YAML frontmatter block from the markdown body.
///
/// Rules (extracted unchanged from `skills.rs::parse_frontmatter`):
///  - strip a leading UTF-8 BOM (U+FEFF) if present;
///  - the text must begin with `---\r\n` or `---\n`, else
///    `Err("missing frontmatter (no leading `---`)")`;
///  - the YAML block ends at the first `\n---` substring; if absent,
///    `Err("unterminated frontmatter")`.
///
/// Returns `(yaml, body)`:
///  - `yaml` — the frontmatter text, delimiters excluded; feed it to
///    `serde_yaml_ng::from_str` for the caller's own target struct;
///  - `body` — everything after the closing `---` delimiter line (a leading
///    `\r?\n` is dropped). For `agent_files`, this is the member's `role`; for
///    `skills`, it is discarded (the `SKILL.md` body is read on demand).
pub fn split(text: &str) -> Result<(&str, &str), String>;

/// Convenience for callers that ignore the body (e.g. skills): the YAML slice
/// only. Pure; identical errors to [`split`].
pub fn frontmatter(text: &str) -> Result<&str, String>;   // = split(text).map(|(y, _)| y)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_returns_yaml_and_body() {
        // "---\nname: x\n---\n\nBody\n" -> (yaml="name: x", body="\nBody\n")
        todo!()
    }

    #[test]
    fn split_strips_a_leading_bom() { todo!() }

    #[test]
    fn split_accepts_crlf_opening() { todo!() }

    #[test]
    fn split_errors_without_a_leading_delimiter() { todo!() }

    #[test]
    fn split_errors_when_unterminated() { todo!() }

    #[test]
    fn split_matches_the_old_substring_terminator_rule() {
        // `\n----` and `\n---x` still terminate (behavior parity with skills.rs).
        todo!()
    }
}
```

## Notes / open questions for review
- **Return type:** this sketch returns `Result<(&str, &str), String>` (not
  `Option`) so the two exact error strings survive unchanged — `skills.rs` tests
  assert on `"missing frontmatter"`/`"unterminated frontmatter"`. If a `Result`
  is deemed too heavy, the `Option` variant must still let `skills.rs` rebuild
  those two messages; that is a behavior risk, so `Result` is preferred.
- **Empty body:** `split` makes no judgement about emptiness; `agent_files.rs`
  decides the `description` fallback (D-C4). Skills ignores `body` entirely.
