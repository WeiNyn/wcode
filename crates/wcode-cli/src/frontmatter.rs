//! Shared YAML-frontmatter split, extracted from `skills.rs::parse_frontmatter`
//! (design §Parser). One rule, two callers: `skills.rs` (reads only the YAML)
//! and `agent_files.rs` (reads YAML + body).
//!
//! The split is DELIBERATELY loose, matching the historical skills behavior: the
//! terminator is the first bare `\n---` substring (so `\n---x` / `\n----` also
//! terminate). Do NOT "tighten" this — that would change how skill files parse.

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
///  - `body` — everything after the closing delimiter line. The boundary is
///    explicit: after the `\n---`, consume the `---`, then one optional `\r`,
///    then one `\n`; `body` is the remainder. So for `…\n---\n\nBody\n` the body
///    is `"\nBody\n"` (only the delimiter line's own newline is dropped). Because
///    the terminator is a loose `\n---` substring, a non-delimiter tail like
///    `…\n---x` still terminates, and the trailing `x…` becomes the body.
///
/// For `agent_files`, `body` is the member's `role`; for `skills`, it is
/// discarded (the `SKILL.md` body is read on demand).
pub fn split(text: &str) -> Result<(&str, &str), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))
        .ok_or("missing frontmatter (no leading `---`)")?;
    let end = rest.find("\n---").ok_or("unterminated frontmatter")?;
    // After the `\n`, the `---`, then one optional `\r` and one `\n`.
    let mut after = end + 1 + 3; // past the `\n---`
    if rest[after..].starts_with('\r') {
        after += 1;
    }
    if rest[after..].starts_with('\n') {
        after += 1;
    }
    Ok((&rest[..end], &rest[after..]))
}

/// Convenience for callers that ignore the body (e.g. `skills.rs`): the YAML
/// slice only. Pure; identical errors to [`split`].
pub fn frontmatter(text: &str) -> Result<&str, String> {
    split(text).map(|(yaml, _)| yaml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_returns_yaml_and_body() {
        // Only the delimiter line's own newline is dropped; the blank line stays.
        let (yaml, body) = split("---\nname: x\n---\n\nBody\n").unwrap();
        assert_eq!(yaml, "name: x");
        assert_eq!(body, "\nBody\n");
    }

    #[test]
    fn split_strips_a_leading_bom() {
        let (yaml, body) = split("\u{feff}---\nname: x\n---\nbody\n").unwrap();
        assert_eq!(yaml, "name: x");
        assert_eq!(body, "body\n");
    }

    #[test]
    fn split_accepts_crlf_opening_and_terminator() {
        let (yaml, body) = split("---\r\nname: x\r\n---\r\nbody\r\n").unwrap();
        assert_eq!(yaml, "name: x\r");
        assert_eq!(body, "body\r\n");
    }

    #[test]
    fn split_errors_without_a_leading_delimiter() {
        assert_eq!(
            split("# no frontmatter\n").unwrap_err(),
            "missing frontmatter (no leading `---`)"
        );
    }

    #[test]
    fn split_errors_when_unterminated() {
        assert_eq!(split("---\nname: x\n").unwrap_err(), "unterminated frontmatter");
    }

    #[test]
    fn split_matches_the_old_substring_terminator_rule() {
        // `\n----` and `\n---x` still terminate (behavior parity with skills.rs):
        // the YAML is everything before the loose `\n---`, the truncated tail is
        // the body.
        let (yaml, body) = split("---\nname: x\n----\n").unwrap();
        assert_eq!(yaml, "name: x");
        assert_eq!(body, "-\n");
        let (yaml, body) = split("---\nname: x\n---x\n").unwrap();
        assert_eq!(yaml, "name: x");
        assert_eq!(body, "x\n");
    }

    #[test]
    fn frontmatter_returns_only_the_yaml() {
        assert_eq!(frontmatter("---\nname: x\n---\nbody\n").unwrap(), "name: x");
        assert_eq!(
            frontmatter("# nope").unwrap_err(),
            "missing frontmatter (no leading `---`)"
        );
    }
}
