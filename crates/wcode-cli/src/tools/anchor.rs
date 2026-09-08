//! Content-addressed line anchors — the wcode hashline core.
//!
//! Stateless by design. A line's anchor is a pure function of its *canonical*
//! text (ASCII whitespace runs collapsed to a single space), so:
//!
//! - inserting or deleting lines elsewhere never changes an intact line's
//!   anchor (no line-number drift), and anchors are reproducible across
//!   processes and sessions — no store, no session state;
//! - anchors survive formatters (rustfmt / prettier / black) that only
//!   reindent, because whitespace is ignored by `canon`;
//! - the tool is self-healing under external edits: every `read`/`edit`
//!   recomputes anchors from the current file contents.
//!
//! The one honest limitation: two lines with identical canonical text share
//! the same anchor. `edit` therefore *rejects* ambiguous targets (listing the
//! candidate line numbers and suggesting `old_string` disambiguation) rather
//! than guessing — fail closed, never silent, exactly like a hash map lookup.
//!
//! Anchors are 5 chars over `[A-Za-z0-9]` (62^5 = 916M addresses). Unequal
//! lines collide in practice only when a real file holds >~10k distinct lines;
//! identical lines colliding is *expected* and handled by ambiguity rejection.

/// Follows pi-better-edit's anchor alphabet and separator convention.
pub const ALPHA: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
pub const ANCHOR_LEN: usize = 5;
pub const ANCHOR_SEP: char = '│';
const ALPHA_LEN: u64 = 62;
const ANCHOR_SPACE: u64 = ALPHA_LEN.pow(ANCHOR_LEN as u32); // 62^5

/// Canonical form of a line: ASCII whitespace runs collapse to a single space
/// and leading/trailing whitespace is trimmed. Formatter-tolerant (indentation
/// and spacing changes keep the anchor), but whitespace *between* words still
/// separates tokens, so `foo bar` and `foobar` are different anchors.
pub fn canon(line: &str) -> String {
    line.split(|c: char| c.is_ascii_whitespace())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// FNV-1a 64 with a splitmix64 finalizer (good avalanche, no dependencies,
/// deterministic across processes — required for stable, global anchors).
fn hash64(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    h
}

/// 5-char anchor for a line: pure function of the canonicalized text.
pub fn anchor(line: &str) -> String {
    let mut idx = hash64(canon(line).as_bytes()) % ANCHOR_SPACE;
    let mut out = [0u8; ANCHOR_LEN];
    for slot in out.iter_mut().rev() {
        *slot = ALPHA[(idx % ALPHA_LEN) as usize];
        idx /= ALPHA_LEN;
    }
    String::from_utf8(out.to_vec()).expect("anchor alphabet is ascii")
}

pub fn is_anchor(s: &str) -> bool {
    s.len() == ANCHOR_LEN && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Split file content into logical lines. A trailing `\n` is a separator, not
/// a line, so `"a\n"` yields `["a"]`; a byte-empty file yields zero lines.
/// A lone `"\n"` file yields the single degenerate line `[""]` — treat it as
/// empty via [`is_degenerate_empty`] rather than as a real blank line.
pub fn split_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return vec![];
    }
    let mut v = content
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<String>>();
    if content.ends_with('\n') {
        v.pop();
    }
    v
}

/// True when a file has no logical lines at all: byte-empty, or only the
/// newline separator. `read` and `edit` treat both as one anonymous
/// insertion point (`anchor("")`). A `"\n\n"` file has a real blank line and
/// is *not* degenerate.
pub fn is_degenerate_empty(content: &str) -> bool {
    content.is_empty() || content == "\n"
}

pub fn anchors_for(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| anchor(l)).collect()
}

/// Render a line as an anchor line, `ANCHOR│content`.
pub fn render(anchor: &str, line: &str) -> String {
    format!("{anchor}{ANCHOR_SEP}{line}")
}

/// One closed line range, 0-based line indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

/// Every range `[start..=end]` in `lines` whose anchor run starts at `from`
/// and ends at `to` (`None` = single line), optionally verified against
/// `old_string`. `old_string` is a filter: it must appear somewhere in the
/// file *up to and including* the candidate range — so a duplicated line can
/// be pinned by giving the line(s) right before it. Empty result = stale
/// target; >1 = ambiguous target.
pub fn find_ranges(
    lines: &[String],
    anchors: &[String],
    from: &str,
    to: Option<&str>,
    old_string: Option<&str>,
) -> Vec<Range> {
    let mut out = Vec::new();
    for (i, a) in anchors.iter().enumerate() {
        if a != from {
            continue;
        }
        let end = match to {
            None => i,
            Some(t) => match anchors[i..].iter().position(|x| x == t) {
                Some(off) => i + off,
                None => continue,
            },
        };
        if let Some(os) = old_string
            && !lines[..=end].join("\n").contains(os)
        {
            continue;
        }
        out.push(Range { start: i, end });
    }
    out
}

/// Replace `range` in `lines` with `replacement`, preserving whether the file
/// ends with a newline. Returns the new content and the 1-based first/last
/// changed line (`None` when the replacement is a literal no-op).
pub fn apply_replace(
    lines: &[String],
    range: &Range,
    replacement: &str,
    ends_with_newline: bool,
) -> (String, Option<(usize, usize)>) {
    let repl = split_lines(replacement);
    let mut out_lines: Vec<String> = Vec::new();
    out_lines.extend(lines[..range.start].iter().cloned());
    let first = out_lines.len() + 1; // 1-based first changed line
    out_lines.extend(repl.iter().cloned());
    let last = out_lines.len(); // 1-based last changed line
    out_lines.extend(lines[range.end + 1..].iter().cloned());

    let mut out = out_lines.join("\n");
    if ends_with_newline {
        out.push('\n');
    }
    let expected = lines.join("\n") + if ends_with_newline { "\n" } else { "" };
    if out == expected {
        return (out, None);
    }
    (out, Some((first, last.max(first))))
}

/// First/last 1-based line that differ between two file contents, or `None`
/// when identical. Used to echo the fresh-anchor view of a just-edited region.
pub fn changed_range(orig: &str, new: &str) -> Option<(usize, usize)> {
    if orig == new {
        return None;
    }
    let a = split_lines(orig);
    let b = split_lines(new);
    let mut first = 0;
    while first < a.len() && first < b.len() && a[first] == b[first] {
        first += 1;
    }
    let mut ia = a.len();
    let mut ib = b.len();
    while ia > first && ib > first && a[ia - 1] == b[ib - 1] {
        ia -= 1;
        ib -= 1;
    }
    if first >= ia && first >= ib {
        // pure insertion/removal collapsed to a boundary by the front skip
        return Some((first + 1, first + 1));
    }
    Some((first + 1, ib.max(first + 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_is_deterministic_and_position_independent() {
        let a = anchor("fn main() {");
        assert_eq!(a, anchor("fn main() {"));
        assert_eq!(a.len(), ANCHOR_LEN);
        assert!(is_anchor(&a));
    }

    #[test]
    fn anchors_survive_indentation_rewrites() {
        assert_eq!(anchor("  let x = 1;"), anchor("let x = 1;"));
        assert_eq!(anchor("}\n"), anchor("}"));
    }

    #[test]
    fn whitespace_collapses_but_words_stay_distinct() {
        // Runs of whitespace are formatter-tolerant…
        assert_eq!(anchor("foo bar"), anchor("  foo   bar "));
        assert_eq!(anchor("let n = 1;"), anchor("let  n  =  1;"));
        // … but distinct token boundaries are NOT conflated (the old canon
        // collided these: it stripped every whitespace char).
        assert_ne!(anchor("foo bar"), anchor("foobar"));
        assert_ne!(anchor("let x = 1;"), anchor("letx=1;"));
    }

    #[test]
    fn anchors_do_not_move_under_insert_above() {
        // The core promise: editing lines above must not change anchors below.
        let plain = ["a = 1", "b = 2", "c = 3"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let with_insert = ["x", "y", "a = 1", "b = 2", "c = 3"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let hs1 = anchors_for(&plain);
        let hs2 = anchors_for(&with_insert);
        assert_eq!(hs1[0], hs2[2]); // "a = 1" kept its anchor at a different line
        assert_eq!(hs1[1], hs2[3]);
        assert_eq!(hs1[2], hs2[4]);
    }

    #[test]
    fn duplicate_lines_share_anchor_and_ranges_are_discoverable() {
        let lines = ["x", "}", "y", "}"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let anchors = anchors_for(&lines);
        assert_eq!(anchors[1], anchors[3]); // identical lines -> same anchor
        let ranges = find_ranges(&lines, &anchors, &anchors[1], None, None);
        assert_eq!(ranges.len(), 2); // ambiguous: both "}" lines
        let with_context = find_ranges(&lines, &anchors, &anchors[1], None, Some("y"));
        assert_eq!(with_context.len(), 1); // old_string disambiguates
        assert_eq!(with_context[0].start, 3);
    }

    #[test]
    fn split_lines_preserves_trailing_separator_semantics() {
        assert_eq!(split_lines(""), Vec::<String>::new());
        assert_eq!(split_lines("a"), ["a"]);
        assert_eq!(split_lines("a\n"), ["a"]);
        assert_eq!(split_lines("a\nb\n"), ["a", "b"]);
        assert_eq!(split_lines("\n"), [""]);
        assert_eq!(split_lines("a\n\n"), ["a", ""]);
    }

    #[test]
    fn degenerate_empty_is_only_byte_empty_or_lone_separator() {
        assert!(is_degenerate_empty(""));
        assert!(is_degenerate_empty("\n"));
        // A real blank line makes the file (trivially) non-empty.
        assert!(!is_degenerate_empty("\n\n"));
        assert!(!is_degenerate_empty("a"));
        assert!(!is_degenerate_empty("a\n"));
    }

    #[test]
    fn apply_replace_mid_file_and_preserve_final_newline() {
        let lines = ["a", "b", "c"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let (out, changed) = apply_replace(&lines, &Range { start: 1, end: 1 }, "B1\nB2", true);
        assert_eq!(out, "a\nB1\nB2\nc\n");
        let (first, last) = changed.unwrap();
        assert_eq!((first, last), (2, 3));
    }

    #[test]
    fn apply_replace_delete_range() {
        let lines = ["a", "b", "c"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let (out, _) = apply_replace(&lines, &Range { start: 1, end: 2 }, "", true);
        assert_eq!(out, "a\n");
    }
}
