//! `session_search` — a read-only tool over prior session transcripts.
//!
//! Two scopes (design D1-D5):
//! - `Scope::All` (default): walk the sessions dir and search every transcript —
//!   flat legacy files AND session groups (`root.jsonl` + `members/<name>.jsonl`,
//!   see `crate::session_groups`) — grouping hits by session. The current session
//!   is excluded unless `include_current`.
//! - `Scope::Current`: search only THIS run's transcript (`ToolContext.session_path`),
//!   reading `Session::entries()` — ALL `Message` entries, including the
//!   pre-compaction ones `Session::messages()` hides. This is the recovery path
//!   for turns a compaction replaced with a summary.
//!
//! Bounded by construction: `limit` sessions (default [`DEFAULT_LIMIT`], max
//! [`MAX_LIMIT`]), [`MAX_SESSIONS_SCANNED`] transcripts examined,
//! [`MAX_MEMBERS_PER_GROUP`] members per group, [`MAX_SNIPPETS_PER_SESSION`]
//! snippets per session, [`SNIPPET_CHARS`] chars per snippet — so the tool can
//! never blow the model's context.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use wcode_harness::compaction::estimate_tokens;
use wcode_harness::message::{AgentMessage, ContentBlock};
use wcode_harness::session::{Session, SessionEntry};
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use crate::session_groups::{self, GroupEntry};

/// Sessions reported when `limit` is omitted.
pub const DEFAULT_LIMIT: u64 = 10;
/// Hard ceiling on `limit`.
pub const MAX_LIMIT: u64 = 50;
/// Snippets emitted per session (matches beyond this only bump the score).
pub const MAX_SNIPPETS_PER_SESSION: usize = 5;
/// Character cap on a single snippet's matched text.
pub const SNIPPET_CHARS: usize = 200;
/// Ceiling on transcripts examined in one `scope:"all"` call (recency-ordered),
/// so a very large sessions dir cannot stall the tool.
pub const MAX_SESSIONS_SCANNED: usize = 500;
/// Ceiling on members examined per session group, so one huge team cannot
/// dominate a scan.
pub const MAX_MEMBERS_PER_GROUP: usize = 50;

/// Which transcripts to search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Cross-session scan of every `.jsonl` under the sessions dir (excludes the
    /// current session unless `include_current`). Default.
    #[default]
    All,
    /// Only the current session's transcript (`ToolContext.session_path`).
    Current,
}

/// `turns{start,end}` — a verbatim ordinal range over the session's `Message`
/// entries (0-based, inclusive). `scope:"current"` only.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
pub struct TurnsRange {
    pub start: usize,
    pub end: usize,
}

/// `session_search` arguments. At least one of `query`/`turns`/`stats` is
/// required (validated in `execute`).
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct SessionSearchArgs {
    /// Case-insensitive terms; ALL must appear in a message's text. `None` when
    /// only `turns`/`stats` is requested.
    pub query: Option<String>,
    /// `all` (default, cross-session) or `current` (this session only).
    pub scope: Option<Scope>,
    /// `scope:"all"`: only sessions whose header `cwd` equals this.
    pub working_dir: Option<String>,
    /// Max sessions reported (default 10, clamped to 50).
    pub limit: Option<u64>,
    /// `scope:"all"`: include the current session too (default false).
    pub include_current: Option<bool>,
    /// Include `Thinking`/`ToolResult` text (default false: user+assistant text only).
    pub include_tools: Option<bool>,
    /// `scope:"current"`: return this verbatim ordinal range instead of searching.
    pub turns: Option<TurnsRange>,
    /// `scope:"current"`: return turn count + a token estimate instead of searching.
    pub stats: Option<bool>,
}

/// The tool. Holds the sessions dir injected by `default_tools`.
pub struct SessionSearch {
    sessions_dir: PathBuf,
}

impl SessionSearch {
    /// `sessions_dir` is the scan root for `scope:"all"` (`repl::session_dir()` in
    /// production). `scope:"current"` ignores it and reads `ctx.session_path`.
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }
}

#[async_trait::async_trait]
impl TypedTool for SessionSearch {
    type Args = SessionSearchArgs;

    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search prior session transcripts. scope=all (the default) scans every \
         session in the sessions dir (groups + legacy files), grouped by session, \
         excluding the current one unless include_current; scope=current reads only \
         this session, including turns a compaction replaced with a summary. All \
         whitespace-separated query terms must appear in a message's text \
         (case-insensitive). thinking/tool-result text is skipped unless \
         include_tools. scope=current also supports turns{start,end} (a verbatim \
         ordinal range) and stats (turn count + token estimate)."
    }

    /// Read-only: safe to run alongside other calls in the same batch.
    fn parallel_safe(&self) -> bool {
        true
    }

    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        // Validation: exactly one selector (design C, tightened), and it must be
        // usable. "Exactly one" subsumes the former turns+stats and query+turns
        // rejections and closes the gap where `query`+`stats` was silently
        // ignored (stats won).
        let scope = args.scope.unwrap_or_default();
        let has_query = args.query.is_some();
        let has_turns = args.turns.is_some();
        let has_stats = args.stats == Some(true);
        let selectors = [has_query, has_turns, has_stats]
            .into_iter()
            .filter(|b| *b)
            .count();
        if selectors != 1 {
            return error("session_search: provide exactly one of `query`, `turns`, or `stats`");
        }
        if let Some(q) = &args.query
            && q.trim().is_empty()
        {
            return error("session_search: `query` must not be empty");
        }
        if scope != Scope::Current && (has_turns || has_stats) {
            return error("session_search: `turns`/`stats` require scope=\"current\"");
        }

        let report = match scope {
            Scope::Current => match &ctx.session_path {
                Some(path) => scan_current(path, &args),
                None => {
                    return error("session_search: scope=\"current\" but this run has no session path");
                }
            },
            Scope::All => scan_all(&self.sessions_dir, ctx.session_path.as_deref(), &args),
        };

        match report {
            Ok(report) => ToolOutput {
                output: render(&report),
                is_error: false,
                diff: None,
                path: None,
            },
            Err(e) => error(e),
        }
    }
}

fn error(message: impl Into<String>) -> ToolOutput {
    ToolOutput {
        output: message.into(),
        is_error: true,
        diff: None,
        path: None,
    }
}

/// One matched snippet within a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Snippet {
    /// `None` for a root/flat file; `Some(name)` for `members/<name>.jsonl`.
    pub member: Option<String>,
    /// "user" | "assistant" | "tool_result".
    pub role: &'static str,
    /// Verbatim matched text, trimmed and capped to [`SNIPPET_CHARS`].
    pub text: String,
}

/// All hits for one session (its root/flat file, plus any members).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionHit {
    /// Session id: the group dir name or the flat file stem (`<millis>_<id8>`).
    pub id: String,
    /// Header `created` (RFC3339), if the header parsed.
    pub created: Option<String>,
    /// Header `cwd`, if the header parsed.
    pub cwd: Option<String>,
    /// The root/flat transcript path.
    pub path: PathBuf,
    /// Total occurrences across the session (the score; recency breaks ties).
    pub score: usize,
    pub snippets: Vec<Snippet>,
}

/// What a call resolved to, before rendering.
#[derive(Debug, PartialEq)]
pub enum Report {
    /// `scope:"all"` (and `scope:"current"` text search): hits, session-grouped.
    Sessions(Vec<SessionHit>),
    /// `scope:"current"` + `turns`: the verbatim ordinal range `(ordinal, message)`.
    Turns(Vec<(usize, AgentMessage)>),
    /// `scope:"current"` + `stats`: turn count + a `compaction::estimate_tokens`
    /// sum over the message entries.
    Stats { messages: usize, tokens: u64 },
}

/// Split a query into lowercased, whitespace-separated terms. Empty when the
/// query is blank.
pub fn terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// True when `haystack_lower` (already lowercased by the caller) contains every
/// term. The raw pre-filter (`line.contains(term)`) is a cheaper superset gate
/// applied before `serde_json::from_str` on the scan path.
pub fn matches_all(haystack_lower: &str, terms: &[String]) -> bool {
    terms.iter().all(|t| haystack_lower.contains(t.as_str()))
}

/// The searchable text of a message: `Text` blocks always; `Thinking` only when
/// `include_tools`; `ToolResult.output` only when `include_tools`.
pub fn searchable_text(msg: &AgentMessage, include_tools: bool) -> String {
    match msg {
        AgentMessage::User { content } | AgentMessage::Assistant { content, .. } => content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::Thinking { text } if include_tools => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        AgentMessage::ToolResult { output, .. } => {
            if include_tools {
                output.clone()
            } else {
                String::new()
            }
        }
    }
}

fn role_of(msg: &AgentMessage) -> &'static str {
    match msg {
        AgentMessage::User { .. } => "user",
        AgentMessage::Assistant { .. } => "assistant",
        AgentMessage::ToolResult { .. } => "tool_result",
    }
}

/// `scope:"current"`: search THIS session's `Session::entries()` (every `Message`,
/// pre-compaction included). Handles `turns` and `stats` verbatim. Torn final
/// lines are tolerated by `Session::open` (`wcode_harness::session`).
pub fn scan_current(session_path: &Path, args: &SessionSearchArgs) -> Result<Report, String> {
    let session = Session::open(session_path)
        .map_err(|e| format!("open {}: {e}", session_path.display()))?;

    let messages: Vec<&AgentMessage> = session
        .entries()
        .iter()
        .filter_map(|e| match e {
            SessionEntry::Message { message, .. } => Some(message),
            _ => None,
        })
        .collect();

    if args.stats == Some(true) {
        let tokens: u64 = messages.iter().map(|m| estimate_tokens(m)).sum();
        return Ok(Report::Stats {
            messages: messages.len(),
            tokens,
        });
    }

    if let Some(range) = args.turns {
        if range.start > range.end {
            return Ok(Report::Turns(Vec::new()));
        }
        let turns = (range.start..=range.end)
            .filter_map(|i| messages.get(i).map(|m| (i, (*m).clone())))
            .collect();
        return Ok(Report::Turns(turns));
    }

    let terms = terms(args.query.as_deref().unwrap_or(""));
    let include_tools = args.include_tools.unwrap_or(false);
    let (created, cwd) = header_of(session.entries());
    let mut snippets = Vec::new();
    let mut score = 0usize;
    for m in &messages {
        let text = searchable_text(m, include_tools);
        let lower = text.to_ascii_lowercase();
        if matches_all(&lower, &terms) {
            let count: usize = terms
                .iter()
                .map(|t| lower.matches(t.as_str()).count())
                .sum();
            score += count.max(1);
            if snippets.len() < MAX_SNIPPETS_PER_SESSION {
                snippets.push(Snippet {
                    member: None,
                    role: role_of(m),
                    text: snippet_text(&text, &terms),
                });
            }
        }
    }
    if snippets.is_empty() {
        return Ok(Report::Sessions(Vec::new()));
    }
    Ok(Report::Sessions(vec![SessionHit {
        id: session_id_of(session_path),
        created,
        cwd,
        path: session_path.to_path_buf(),
        score,
        snippets,
    }]))
}


/// `scope:"all"`: walk `sessions_dir`, respecting `working_dir`, skipping the
/// `history` prompt file and non-`.jsonl` entries, grouping members under their
/// root group (members tagged by name) and excluding `current` unless
/// `include_current`. Newest sessions first (`session_groups::list_groups`).
pub fn scan_all(
    sessions_dir: &Path,
    current: Option<&Path>,
    args: &SessionSearchArgs,
) -> Result<Report, String> {
    let entries = session_groups::list_groups(sessions_dir)
        .map_err(|e| format!("list sessions in {}: {e}", sessions_dir.display()))?;
    let terms = terms(args.query.as_deref().unwrap_or(""));
    let include_tools = args.include_tools.unwrap_or(false);
    let include_current = args.include_current.unwrap_or(false);
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let working_dir = args.working_dir.as_deref();

    let mut hits: Vec<SessionHit> = Vec::new();
    for entry in entries.into_iter().take(MAX_SESSIONS_SCANNED) {

        let (id, root_path, members): (String, PathBuf, Vec<(PathBuf, String)>) = match &entry {
            GroupEntry::Group(g) => (g.id(), g.root_path.clone(), member_files(&g.members_dir)),
            GroupEntry::Legacy { path } => (stem_of(path), path.clone(), Vec::new()),
        };

        // Exclude the WHOLE group when the current session is any of its files.
        if !include_current
            && current.is_some_and(|c| {
                root_path.as_path() == c || members.iter().any(|(p, _)| p.as_path() == c)
            })
        {
            continue;
        }

        // Root/flat transcript: a malformed non-final line skips this SESSION.
        let root = match scan_file(&root_path, &terms, include_tools, None) {
            Ok(scan) => scan,
            Err(_) => continue,
        };
        let mut score = root.score;
        let mut snippets = root.snippets;
        let created = root.created;
        let cwd = root.cwd;

        // Members (capped): a malformed member skips just that member.
        for (path, name) in members.iter().take(MAX_MEMBERS_PER_GROUP) {
            if let Ok(scan) = scan_file(path, &terms, include_tools, Some(name.clone())) {
                score += scan.score;
                snippets.extend(scan.snippets);
            }
        }

        if snippets.is_empty() {
            continue;
        }
        if working_dir.is_some_and(|wd| cwd.as_deref() != Some(wd)) {
            continue;
        }
        hits.push(SessionHit {
            id,
            created,
            cwd,
            path: root_path,
            score,
            snippets,
        });
    }

    // Score desc; `list_groups` is newest-first and `sort_by` is stable, so
    // equal scores keep newest-first (the recency tiebreak, D2).
    hits.sort_by(|a, b| b.score.cmp(&a.score));
    hits.truncate(limit);
    Ok(Report::Sessions(hits))
}

/// Render a [`Report`] to the tool's `output` string.
pub fn render(report: &Report) -> String {
    match report {
        Report::Sessions(hits) if hits.is_empty() => "no matches".to_string(),
        Report::Sessions(hits) => {
            let mut out = String::new();
            for h in hits {
                out.push_str(&format!("session {} (score {})", h.id, h.score));
                if let Some(created) = &h.created {
                    out.push_str(&format!(" created {created}"));
                }
                if let Some(cwd) = &h.cwd {
                    out.push_str(&format!(" cwd {cwd}"));
                }
                out.push('\n');
                out.push_str(&format!("  {}\n", h.path.display()));
                for s in &h.snippets {
                    let who = match &s.member {
                        Some(member) => format!("{}/{member}", s.role),
                        None => s.role.to_string(),
                    };
                    out.push_str(&format!("  [{who}] {}\n", s.text));
                }
            }
            out
        }
        Report::Turns(turns) => {
            let mut out = String::new();
            for (ordinal, msg) in turns {
                out.push_str(&format!("[{ordinal}] {}: {}\n", role_of(msg), msg.as_text()));
            }
            out
        }
        Report::Stats { messages, tokens } => {
            format!("{messages} messages, ~{tokens} tokens")
        }
    }
}

/// The header's `(created, cwd)`, if a `Header` entry is present.
fn header_of(entries: &[SessionEntry]) -> (Option<String>, Option<String>) {
    for e in entries {
        if let SessionEntry::Header { created, cwd, .. } = e {
            return (Some(created.clone()), Some(cwd.clone()));
        }
    }
    (None, None)
}

/// The session id a `scope:"current"` hit reports: a group's root transcript is
/// `<dir>/root.jsonl`, whose id is the **group dir** name (not `"root"`); a flat
/// legacy file keeps its own stem.
fn session_id_of(path: &Path) -> String {
    if path.file_name().and_then(|n| n.to_str()) == Some("root.jsonl")
        && let Some(dir) = path.parent().and_then(|p| p.file_name())
    {
        return dir.to_string_lossy().into_owned();
    }
    stem_of(path)
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `members/*.jsonl` as `(path, name)`, name-sorted for a stable scan order.
fn member_files(members_dir: &Path) -> Vec<(PathBuf, String)> {
    let read = match fs::read_dir(members_dir) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<(PathBuf, String)> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .map(|p| (p.clone(), stem_of(&p)))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// What one file contributed to a session's hit.
#[derive(Default)]
struct FileScan {
    created: Option<String>,
    cwd: Option<String>,
    snippets: Vec<Snippet>,
    score: usize,
}

/// Parse one transcript, applying the raw pre-filter before `serde_json::from_str`.
/// A torn FINAL line is tolerated (like `Session::open`); a malformed NON-final
/// line is an `Err` so the caller can skip the session (amendment E).
fn scan_file(
    path: &Path,
    terms: &[String],
    include_tools: bool,
    member: Option<String>,
) -> Result<FileScan, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let lines: Vec<&str> = raw.lines().collect();
    let last_non_empty = lines.iter().rposition(|l| !l.trim().is_empty());
    // A term containing `"` or `\` is JSON-escaped in the raw line, so a naive
    // `contains` would false-negative — parse every line instead (amendment D).
    let skip_prefilter = terms.iter().any(|t| t.contains('\\') || t.contains('"'));

    let mut out = FileScan::default();
    let mut header_seen = false;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        // The header (the first non-blank line) is always parsed for created/cwd.
        let first = !header_seen;
        header_seen = true;
        // Pre-filter: a cheap superset gate before serde. Lowercased to match D2
        // (amendment A) — a raw compare would drop `Needle` for `needle`.
        if !first && !skip_prefilter && !matches_all(&line.to_ascii_lowercase(), terms) {
            continue;
        }
        let entry: SessionEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(e) => {
                if Some(i) == last_non_empty {
                    break; // torn final write; ignore the rest
                }
                return Err(format!("{}: {e}", path.display()));
            }
        };
        match entry {
            SessionEntry::Header { created, cwd, .. } => {
                out.created = Some(created);
                out.cwd = Some(cwd);
            }
            SessionEntry::Message { message, .. } => {
                let text = searchable_text(&message, include_tools);
                let lower = text.to_ascii_lowercase();
                if matches_all(&lower, terms) {
                    let count: usize = terms
                        .iter()
                        .map(|t| lower.matches(t.as_str()).count())
                        .sum();
                    out.score += count.max(1);
                    if out.snippets.len() < MAX_SNIPPETS_PER_SESSION {
                        out.snippets.push(Snippet {
                            member: member.clone(),
                            role: role_of(&message),
                            text: snippet_text(&text, terms),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A snippet around the earliest matched term, trimmed and capped to
/// [`SNIPPET_CHARS`] (char-safe: never splits a multi-byte boundary).
fn snippet_text(text: &str, terms: &[String]) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= SNIPPET_CHARS {
        return trimmed.to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    let hit = terms
        .iter()
        .filter_map(|term| lower.find(term.as_str()))
        .min()
        .unwrap_or(0);
    let start_char = trimmed[..hit].chars().count();
    let from = start_char.saturating_sub(SNIPPET_CHARS / 4);
    let mut out: String = trimmed.chars().skip(from).take(SNIPPET_CHARS).collect();
    if from > 0 {
        out.insert(0, '…');
    }
    if trimmed.chars().count() > from + SNIPPET_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write encoded NDJSON `lines` (no trailing newline) to `path`, creating
    /// parents — a test stand-in for what the kernel's `Session::append` writes.
    fn write_lines(path: &Path, lines: &[String]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, lines.join("\n")).unwrap();
    }

    /// Encode one `SessionEntry` (Header/Message/Compaction) as a line.
    fn entry_line(entry: &SessionEntry) -> String {
        serde_json::to_string(entry).unwrap()
    }

    fn header_line(cwd: &str) -> String {
        entry_line(&SessionEntry::Header {
            version: 1,
            id: "s".into(),
            cwd: cwd.into(),
            created: "2024-01-01T00:00:00Z".into(),
        })
    }

    fn user_line(id: &str, text: &str) -> String {
        entry_line(&SessionEntry::Message {
            id: id.into(),
            parent_id: None,
            message: AgentMessage::user_text(text),
        })
    }

    fn ctx(session_path: Option<PathBuf>) -> ToolContext {
        let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            call_id: "s1".into(),
            name: "session_search".into(),
            working_dir: std::env::temp_dir(),
            cancel: tokio_util::sync::CancellationToken::new(),
            events,
            session_path,
        }
    }

    fn query(q: &str) -> SessionSearchArgs {
        SessionSearchArgs {
            query: Some(q.into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn no_selector_is_an_error() {
        let tool = SessionSearch::new(std::env::temp_dir());
        let out = tool.execute(SessionSearchArgs::default(), &ctx(None)).await;
        assert!(out.is_error, "{out:?}");
        assert!(
            out.output.contains("query") && out.output.contains("stats"),
            "message names the requirement: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn an_empty_query_is_rejected() {
        // Finding #3: a blank query used to yield `terms == []` → match
        // everything. It is now an error.
        let tool = SessionSearch::new(std::env::temp_dir());
        for q in ["", "   ", "\t\n"] {
            let out = tool.execute(query(q), &ctx(None)).await;
            assert!(
                out.is_error && out.output.contains("must not be empty"),
                "blank query {q:?} rejected: {out:?}"
            );
        }
    }

    #[tokio::test]
    async fn turns_and_stats_require_current_scope() {
        let tool = SessionSearch::new(std::env::temp_dir());
        // turns with scope=all (default)
        let out = tool
            .execute(
                SessionSearchArgs {
                    turns: Some(TurnsRange { start: 0, end: 0 }),
                    ..Default::default()
                },
                &ctx(None),
            )
            .await;
        assert!(out.is_error && out.output.contains("current"), "{out:?}");
        // stats with scope=all
        let out = tool
            .execute(
                SessionSearchArgs {
                    stats: Some(true),
                    ..Default::default()
                },
                &ctx(None),
            )
            .await;
        assert!(out.is_error && out.output.contains("current"), "{out:?}");
    }

    #[tokio::test]
    async fn more_than_one_selector_is_rejected() {
        let tool = SessionSearch::new(std::env::temp_dir());
        let cases = vec![
            // turns + stats
            SessionSearchArgs {
                scope: Some(Scope::Current),
                turns: Some(TurnsRange { start: 0, end: 0 }),
                stats: Some(true),
                ..Default::default()
            },
            // query + turns
            SessionSearchArgs {
                scope: Some(Scope::Current),
                query: Some("x".into()),
                turns: Some(TurnsRange { start: 0, end: 0 }),
                ..Default::default()
            },
            // query + stats (finding #5: this used to be silently ignored)
            SessionSearchArgs {
                scope: Some(Scope::Current),
                query: Some("x".into()),
                stats: Some(true),
                ..Default::default()
            },
        ];
        for args in cases {
            let out = tool.execute(args, &ctx(None)).await;
            assert!(
                out.is_error && out.output.contains("exactly one"),
                "two selectors rejected: {out:?}"
            );
        }
    }

    #[tokio::test]
    async fn current_scope_without_a_session_is_an_error() {
        let tool = SessionSearch::new(std::env::temp_dir());
        let out = tool
            .execute(
                SessionSearchArgs {
                    scope: Some(Scope::Current),
                    query: Some("x".into()),
                    ..Default::default()
                },
                &ctx(None),
            )
            .await;
        assert!(out.is_error && out.output.contains("no session"), "{out:?}");
    }

    #[tokio::test]
    async fn groups_flat_and_group_and_members() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A flat legacy file.
        write_lines(
            &root.join("100_aaaa.jsonl"),
            &[header_line("/work"), user_line("m1", "a needle here")],
        );
        // A group: root + one member, both matching.
        write_lines(
            &root.join("200_bbbbbbbb/root.jsonl"),
            &[header_line("/work"), user_line("m1", "needle in root")],
        );
        write_lines(
            &root.join("200_bbbbbbbb/members/w1.jsonl"),
            &[header_line("/work"), user_line("m1", "needle from the worker")],
        );

        let report = scan_all(root, None, &query("needle")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!("expected sessions")
        };
        // The member is grouped UNDER its root group, so two sessions: the flat
        // file and the group (root + member).
        assert_eq!(hits.len(), 2, "flat + group (root+member): {hits:?}");
        let group = hits.iter().find(|h| h.id == "200_bbbbbbbb").expect("group hit");
        assert!(
            group
                .snippets
                .iter()
                .any(|s| s.member.as_deref() == Some("w1")),
            "member tagged: {:?}",
            group.snippets
        );
        let flat = hits.iter().find(|h| h.id == "100_aaaa").expect("flat hit");
        assert!(flat.snippets.iter().all(|s| s.member.is_none()));
    }

    #[tokio::test]
    async fn scope_all_excludes_current_unless_include_current() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("100_aaaa.jsonl");
        write_lines(&path, &[header_line("/w"), user_line("m1", "needle")]);

        let excluded = scan_all(root, Some(&path), &query("needle")).unwrap();
        let Report::Sessions(hits) = excluded else {
            panic!()
        };
        assert!(hits.is_empty(), "current excluded by default: {hits:?}");

        let args = SessionSearchArgs {
            include_current: Some(true),
            ..query("needle")
        };
        let included = scan_all(root, Some(&path), &args).unwrap();
        let Report::Sessions(hits) = included else {
            panic!()
        };
        assert_eq!(hits.len(), 1, "include_current surfaces it: {hits:?}");
    }

    #[tokio::test]
    async fn scope_all_excludes_a_whole_group_holding_current() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // The current session is a MEMBER; its group root also matches.
        let member = root.join("200_bbbbbbbb/members/w1.jsonl");
        write_lines(
            &root.join("200_bbbbbbbb/root.jsonl"),
            &[header_line("/w"), user_line("m1", "needle in root")],
        );
        write_lines(&member, &[header_line("/w"), user_line("m1", "needle")]);

        let report = scan_all(root, Some(&member), &query("needle")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert!(hits.is_empty(), "the whole group is excluded: {hits:?}");
    }

    #[tokio::test]
    async fn scope_all_excludes_a_group_whose_root_is_current() {
        // Finding #7: `current` is the group's ROOT (not a member); the whole
        // group is still excluded.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let group_root = root.join("200_bbbbbbbb/root.jsonl");
        write_lines(&group_root, &[header_line("/w"), user_line("m1", "needle")]);

        let report = scan_all(root, Some(&group_root), &query("needle")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert!(hits.is_empty(), "the group root is excluded: {hits:?}");
    }

    #[tokio::test]
    async fn current_on_a_group_root_reports_the_group_id() {
        // Finding #4: a group's root is `<dir>/root.jsonl`; the reported id is
        // the GROUP DIR name, not "root".
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("200_bbbbbbbb/root.jsonl");
        write_lines(&path, &[header_line("/w"), user_line("m1", "needle")]);

        let args = SessionSearchArgs {
            scope: Some(Scope::Current),
            ..query("needle")
        };
        let report = scan_current(&path, &args).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].id, "200_bbbbbbbb", "group dir name, not \"root\"");
    }

    #[tokio::test]
    async fn member_cap_limits_members_scanned() {
        // Finding #2(a): the per-group member cap has teeth. 51 members, the
        // needle only in the last-sorted one (#51) — beyond `MAX_MEMBERS_PER_GROUP`.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let group = root.join("100_aaaaaaaa");
        write_lines(&group.join("root.jsonl"), &[header_line("/w")]);
        for i in 0..MAX_MEMBERS_PER_GROUP {
            write_lines(
                &group.join(format!("members/m{i:02}.jsonl")),
                &[header_line("/w"), user_line("m1", "filler")],
            );
        }
        // Member #51 sorts last (`m50` > `m49`), so `.take(50)` never reaches it.
        write_lines(
            &group.join("members/m50.jsonl"),
            &[header_line("/w"), user_line("m1", "needle")],
        );

        let report = scan_all(root, None, &query("needle")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert!(
            hits.iter().all(|h| h.id != "100_aaaaaaaa"),
            "the needle is past the member cap: {hits:?}"
        );
    }

    #[tokio::test]
    async fn limit_caps_the_sessions_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for id in ["100_aaaa", "200_bbbb", "300_cccc"] {
            write_lines(
                &root.join(format!("{id}.jsonl")),
                &[header_line("/w"), user_line("m1", "needle")],
            );
        }
        let args = SessionSearchArgs {
            limit: Some(1),
            ..query("needle")
        };
        let report = scan_all(root, None, &args).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].id, "300_cccc", "newest first on an equal score");
    }

    #[tokio::test]
    async fn session_scan_cap_limits_sessions_scanned() {
        // Finding #2(b): `MAX_SESSIONS_SCANNED` has teeth. `MAX_SESSIONS_SCANNED
        // + 1` sessions; the needle is only in the OLDEST, which is beyond the
        // `MAX_SESSIONS_SCANNED` newest that `list_groups` returns.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..(MAX_SESSIONS_SCANNED + 1) {
            // Zero-padded so name order is chronological; the needle is in `i == 0`.
            let text = if i == 0 { "needle" } else { "filler" };
            write_lines(
                &root.join(format!("{i:04}_aaaa.jsonl")),
                &[header_line("/w"), user_line("m1", text)],
            );
        }
        let report = scan_all(root, None, &query("needle")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert!(
            hits.is_empty(),
            "the needle is in the oldest, past the scan cap: {hits:?}"
        );
    }

    #[tokio::test]
    async fn mixed_case_line_is_found_by_the_lowercased_pre_filter() {
        // Amendment A: the raw pre-filter lowercases, so `Needle` is found for
        // the term `needle` (a raw compare would drop the line).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_lines(
            &root.join("100_aaaa.jsonl"),
            &[header_line("/w"), user_line("m1", "A Needle In CamelCase")],
        );
        let report = scan_all(root, None, &query("needle")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0].snippets[0].text.contains("Needle"),
            "snippet carries the matched text: {:?}",
            hits[0].snippets
        );
    }

    #[tokio::test]
    async fn a_malformed_non_final_line_skips_the_session_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A good session and a broken one (garbage in the middle).
        write_lines(
            &root.join("100_aaaa.jsonl"),
            &[header_line("/w"), user_line("m1", "needle")],
        );
        write_lines(
            &root.join("200_bbbb.jsonl"),
            &[header_line("/w"), "{ not json needle".into(), user_line("m1", "needle")],
        );
        let report = scan_all(root, None, &query("needle")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert_eq!(hits.len(), 1, "the broken session is skipped: {hits:?}");
        assert_eq!(hits[0].id, "100_aaaa");
    }

    #[tokio::test]
    async fn a_term_with_a_quote_is_found_by_skipping_the_pre_filter() {
        // Amendment D: the term contains a `"`, which is JSON-escaped in the raw
        // line, so the pre-filter is bypassed and the line is parsed.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_lines(
            &root.join("100_aaaa.jsonl"),
            &[header_line("/w"), user_line("m1", "say \"hello\" now")],
        );
        let report = scan_all(root, None, &query("\"hello\"")).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert_eq!(hits.len(), 1, "escaped term still matches: {hits:?}");
    }

    #[tokio::test]
    async fn current_recovers_pre_compaction_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.jsonl");
        let mut lines = vec![header_line("/w")];
        lines.push(user_line("m0", "needle in the summarized prefix"));
        for i in 1..5 {
            lines.push(user_line(&format!("m{i}"), "filler"));
        }
        lines.push(entry_line(&SessionEntry::Compaction {
            id: "c1".into(),
            summary: "summary".into(),
            first_kept_message: 3,
            tokens_before: None,
            usage: None,
        }));
        write_lines(&path, &lines);

        // `messages()` hides the summarized prefix; `entries()` (what
        // scope=current reads) still has it.
        let session = Session::open(&path).unwrap();
        assert!(
            !session.messages().iter().any(|m| m.as_text().contains("needle")),
            "compaction hides the prefix from messages()"
        );

        let args = SessionSearchArgs {
            scope: Some(Scope::Current),
            ..query("needle")
        };
        let report = scan_current(&path, &args).unwrap();
        let Report::Sessions(hits) = report else {
            panic!()
        };
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0].snippets.iter().any(|s| s.text.contains("needle")),
            "a summarized turn is still searchable: {:?}",
            hits[0].snippets
        );
    }

    #[tokio::test]
    async fn current_turns_returns_a_verbatim_ordinal_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.jsonl");
        write_lines(
            &path,
            &[
                header_line("/w"),
                user_line("m0", "zero"),
                user_line("m1", "one"),
                user_line("m2", "two"),
            ],
        );
        let args = SessionSearchArgs {
            scope: Some(Scope::Current),
            turns: Some(TurnsRange { start: 1, end: 2 }),
            ..Default::default()
        };
        let report = scan_current(&path, &args).unwrap();
        let Report::Turns(turns) = report else {
            panic!("expected turns")
        };
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].0, 1);
        assert_eq!(turns[0].1.as_text(), "one");
        assert_eq!(turns[1].0, 2);
        assert_eq!(turns[1].1.as_text(), "two");
    }

    #[tokio::test]
    async fn current_stats_reports_turn_count_and_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.jsonl");
        write_lines(
            &path,
            &[
                header_line("/w"),
                user_line("m0", "one"),
                user_line("m1", "two"),
            ],
        );
        let args = SessionSearchArgs {
            scope: Some(Scope::Current),
            stats: Some(true),
            ..Default::default()
        };
        let report = scan_current(&path, &args).unwrap();
        let Report::Stats { messages, tokens } = report else {
            panic!("expected stats")
        };
        assert_eq!(messages, 2);
        assert!(tokens > 0, "a non-empty transcript estimates >0 tokens");
    }

    #[test]
    fn terms_split_and_lowercase() {
        assert_eq!(terms("  Foo BAR "), vec!["foo", "bar"]);
        assert!(terms("").is_empty());
        assert!(terms("   ").is_empty());
    }

    #[test]
    fn matches_all_requires_every_term() {
        assert!(matches_all("the quick brown fox", &terms("quick fox")));
        assert!(!matches_all("the quick brown fox", &terms("quick cat")));
        // Vacuous truth: no terms means every line is a candidate.
        assert!(matches_all("anything", &[]));
    }
}
