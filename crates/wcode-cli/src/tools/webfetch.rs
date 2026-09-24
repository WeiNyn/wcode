//! `webfetch` — one-shot URL fetch, HTML reduced to readable text.
//!
//! A CLI tool (D8): it needs an HTTP client, and the kernel
//! (`wcode-harness`) stays free of network egress. Always registered (D6), not
//! gated by `[tools]`; read-only (D5) so it is `parallel_safe`.
//!
//! Design locked in `docs/webfetch-plan.md` (D1–D12, Q1–Q5); reference
//! implementation `../jcode/crates/jcode-app-core/src/tool/webfetch.rs`.
//!
//! Ground truth (cited by function):
//! - `crates/wcode-harness/src/tool.rs` — `TypedTool` (`Args: DeserializeOwned +
//!   JsonSchema`, `parallel_safe`), `ToolOutput { output, is_error, diff, path }`.
//! - `crates/wcode-cli/src/tools/bash.rs` — the `tokio::select!` on
//!   `ctx.cancel.cancelled()` returning `ToolOutput { output: "cancelled",
//!   is_error: true }` (D9 parity), and `DEFAULT_TIMEOUT_SECS`.
//! - `crates/wcode-cli/src/tools/mod.rs::default_tools` — the registration seam.

use std::time::Duration;

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

/// Body cap (D3): reject a `content-length` over this; otherwise truncate while
/// streaming. 5 MiB, jcode parity.
const MAX_SIZE: usize = 5 * 1024 * 1024;
/// Output cap (D3): the converted text handed back to the model. Full pages
/// routinely exceed 150 KB (~40k tokens), rarely worth the context budget.
const MAX_OUTPUT_CHARS: usize = 40_000;
/// Links whose target exceeds this length are rendered as their anchor text
/// only (D2) — long URLs are usually encoded payloads, not addresses.
const MAX_URL_CHARS: usize = 300;
/// Default `timeout` seconds (D1).
const DEFAULT_TIMEOUT: u64 = 30;
/// Clamp for `timeout` (D1).
const MAX_TIMEOUT: u64 = 120;

/// The `webfetch` tool (D1/D10).
///
/// The `reqwest::Client` is built ONCE in [`WebFetch::new`] and lives on the
/// struct (D10) — TLS/connection-pool reuse, and it keeps the tool
/// `parallel_safe`-sound (a `reqwest::Client` is `Sync` + cheap to clone). It is
/// NOT built per call. Note: `default_tools` runs per agent (root + each worker
/// and each `/new` rebuild), so each agent gets its own `Client` — there is no
/// process-global pool (acceptable; D10 says "on the tool struct").
pub struct WebFetch {
    client: reqwest::Client,
}

impl WebFetch {
    /// Build the shared client once (D10). Uses reqwest defaults for redirects
    /// (10 hops, D7) and rustls TLS; no proxy/env wiring beyond reqwest's own.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .build()
                .expect("build reqwest client"),
        }
    }
}

impl Default for WebFetch {
    fn default() -> Self {
        Self::new()
    }
}

/// Args (D1): GET only — no method/headers/body/cookies.
///
/// Schema note: `format`/`timeout` are `Option`, so schemars emits them as
/// optional. An absent `format` is `markdown`; an unknown value is treated as
/// `markdown` (the matrix's catch-all row).
#[derive(Deserialize, schemars::JsonSchema)]
pub struct WebFetchArgs {
    /// The URL; must start with `http://` or `https://` (D4).
    pub url: String,
    /// `"text" | "markdown" | "html"` (default `markdown`).
    pub format: Option<String>,
    /// Timeout in seconds (default 30, clamped to max 120) (D1).
    pub timeout: Option<u64>,
}

#[async_trait::async_trait]
impl TypedTool for WebFetch {
    type Args = WebFetchArgs;

    fn name(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL over http(s) and return its content. `format` is \
         `text`|`markdown`|`html` (default `markdown`); `timeout` seconds \
         (default 30, max 120). HTML is reduced to readable text; the body and \
         the returned output are both capped."
    }

    /// Read-only (D5): no workspace effect; runs concurrently with other
    /// read-only calls in a batch.
    fn parallel_safe(&self) -> bool {
        true
    }

    // `mutating()` keeps the trait default `false` (D5): `webfetch` is absent
    // from `hooks::MUTATING_TOOLS`, so plan mode already permits it with no code
    // change (D11).

    /// One GET. Failures are `ToolOutput { is_error: true }`, never a panic
    /// (D4). `diff`/`path` stay `None` (D5).
    ///
    /// Pipeline (pin — jcode parity):
    /// 1. **scheme** — not `http://`/`https://` → `URL must start with http:// or
    ///    https://` (D4).
    /// 2. **timeout** — `args.timeout.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT)`,
    ///    applied as `reqwest`'s per-request `.timeout(Duration)` (NOT a separate
    ///    `sleep` race — reqwest owns the timeout; only cancel is raced).
    /// 3. **request** — `self.client.get(url).header(USER_AGENT,
    ///    format!("wcode/{}", env!("CARGO_PKG_VERSION")))` (D7).
    /// 4. **cancel race** (D9) — `tokio::select! { _ =
    ///    ctx.cancel.cancelled() => cancelled, r = request.send() => r }`,
    ///    returning `ToolOutput { output: "cancelled", is_error: true }` on
    ///    cancel (bash parity).
    /// 5. **status** — non-2xx → `HTTP error: <status>` (D4).
    /// 6. **body cap** (D3) — a `content-length` over `MAX_SIZE` is an ERROR; a
    ///    longer stream is truncated to `MAX_SIZE` and the note `… (truncated,
    ///    showing first <MAX_SIZE> bytes)` is appended to the RAW BODY (jcode
    ///    parity) before conversion.
    /// 7. **format matrix** (D2) — see [`format_body`].
    /// 8. **output cap** (D3) — [`truncate_output`]; on truncation append
    ///    `(output truncated to <MAX_OUTPUT_CHARS> of <full_len> chars; …)`.
    /// 9. **frame** — `Fetched <url> (<full_len> bytes)\n\n<output><note>`, where
    ///    `<full_len>` is the CONVERTED length, post-format (§4/Q5), not the raw
    ///    download size.
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        // 1. scheme (D4).
        if !args.url.starts_with("http://") && !args.url.starts_with("https://") {
            return error("URL must start with http:// or https://".to_string());
        }

        // 2. timeout (D1) + 3. request (D7).
        let timeout = args.timeout.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
        let request = self
            .client
            .get(&args.url)
            .header(
                reqwest::header::USER_AGENT,
                format!("wcode/{}", env!("CARGO_PKG_VERSION")),
            )
            .timeout(Duration::from_secs(timeout))
            .send();

        // 4. cancel race (D9) — bash parity: a mid-flight cancel returns
        //    `cancelled` and drops the request future.
        let response = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => return error("cancelled".to_string()),
            r = request => match r {
                Ok(response) => response,
                Err(e) => return error(format!("fetch {}: {e}", args.url)),
            },
        };

        // 5. status (D4).
        let status = response.status();
        if !status.is_success() {
            return error(format!("HTTP error: {status}"));
        }

        // 6. body cap: reject early on a declared `content-length` (D3).
        if let Some(len) = response.content_length()
            && len as usize > MAX_SIZE
        {
            return error(format!(
                "Response too large: {len} bytes (max {MAX_SIZE} bytes)"
            ));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        // 6. body cap: truncate while streaming (D3). `Response::chunk()` is the
        //    ungated loop (not `bytes_stream()`, which needs the `stream`
        //    feature). The cancel token is still raced, so a mid-body cancel
        //    aborts too.
        let mut response = response;
        let mut body_bytes: Vec<u8> = Vec::new();
        let mut body_truncated = false;
        loop {
            let chunk = tokio::select! {
                biased;
                _ = ctx.cancel.cancelled() => return error("cancelled".to_string()),
                c = response.chunk() => match c {
                    Ok(chunk) => chunk,
                    Err(e) => return error(format!("fetch {}: {e}", args.url)),
                },
            };
            let Some(chunk) = chunk else {
                break;
            };
            let remaining = MAX_SIZE.saturating_sub(body_bytes.len());
            if chunk.len() > remaining {
                body_bytes.extend_from_slice(&chunk[..remaining]);
                body_truncated = true;
                break;
            }
            body_bytes.extend_from_slice(&chunk);
        }

        let mut body = String::from_utf8_lossy(&body_bytes).into_owned();
        if body_truncated {
            body.push_str(&format!(
                "...\n\n(truncated, showing first {MAX_SIZE} bytes)"
            ));
        }

        // 7. format matrix (D2).
        let output = format_body(&body, args.format.as_deref().unwrap_or("markdown"), &content_type);

        // 8. output cap (D3).
        let full_len = output.len();
        let (output, output_truncated) = truncate_output(output);
        let note = if output_truncated {
            format!(
                "\n\n(output truncated to {MAX_OUTPUT_CHARS} of {full_len} chars; \
                 fetch a more specific URL or anchor for the rest)"
            )
        } else {
            String::new()
        };

        // 9. frame (D4/§4/Q5) — `<full_len>` is the converted length.
        ok(format!(
            "Fetched {} ({full_len} bytes)\n\n{output}{note}",
            args.url
        ))
    }
}

/// A non-error `ToolOutput` (D5: `diff`/`path` stay `None`).
fn ok(output: String) -> ToolOutput {
    ToolOutput {
        output,
        is_error: false,
        diff: None,
        path: None,
    }
}

/// An error `ToolOutput` (D4/D9).
fn error(output: String) -> ToolOutput {
    ToolOutput {
        output,
        is_error: true,
        diff: None,
        path: None,
    }
}

// ==========================================================================
// The format matrix (D2) + converter
// ==========================================================================

/// The exact `format` matrix (D2, jcode parity — do not improvise):
///
/// | `format` | content-type contains `text/html` | otherwise |
/// |---|---|---|
/// | `html` | raw body | raw body |
/// | `text` | `html_to_text(body)` | `html_to_text(body)` |
/// | `markdown` (default) / unknown | `html_to_markdown(body)` | body |
///
/// `text` ALWAYS runs the stripper (matching jcode), even on a non-HTML body
/// (there it is a near no-op).
fn format_body(body: &str, format: &str, content_type: &str) -> String {
    match format {
        "html" => body.to_string(),
        "text" => html_to_text(body),
        // `markdown` and any unknown value share the catch-all row.
        _ => {
            if content_type.contains("text/html") {
                html_to_markdown(body)
            } else {
                body.to_string()
            }
        }
    }
}

/// Strip HTML to plain text (D2). Order (jcode parity): remove
/// `<script>`/`<style>`/comments, then the non-prose `CHROME` elements, map
/// `<br>`/`</p>`/`</div>`/`</li>`/`</tr>` to newlines, drop all remaining tags,
/// decode the six entities (`&nbsp; &lt; &gt; &amp; &quot; &#39;`), collapse
/// blank-line runs, trim. Pure `&str -> String`.
fn html_to_text(html: &str) -> String {
    let mut text = html.to_string();
    text = html_regex::SCRIPT.replace_all(&text, "").to_string();
    text = html_regex::STYLE.replace_all(&text, "").to_string();
    text = html_regex::COMMENT.replace_all(&text, "").to_string();
    for re in html_regex::CHROME.iter() {
        text = re.replace_all(&text, "").to_string();
    }

    text = text.replace("<br>", "\n");
    text = text.replace("<br/>", "\n");
    text = text.replace("<br />", "\n");
    text = text.replace("</p>", "\n\n");
    text = text.replace("</div>", "\n");
    text = text.replace("</li>", "\n");
    text = text.replace("</tr>", "\n");

    text = html_regex::TAG.replace_all(&text, "").to_string();

    text = text.replace("&nbsp;", " ");
    text = text.replace("&lt;", "<");
    text = text.replace("&gt;", ">");
    text = text.replace("&amp;", "&");
    text = text.replace("&quot;", "\"");
    text = text.replace("&#39;", "'");

    text = html_regex::WHITESPACE.replace_all(&text, "\n\n").to_string();

    text.trim().to_string()
}

/// Convert HTML to markdown (D2). Same pre-pass as [`html_to_text`], then
/// headings (`#`..`######`), links ([`render_link`]), `**strong**`, `*em*`,
/// `` `code` ``, fenced `pre>code`, `- ` list items; then the tag strip, the six
/// entities, the **empty-bullet cleanup** (`EMPTY_BULLETS`), whitespace collapse,
/// trim. Pure `&str -> String`.
fn html_to_markdown(html: &str) -> String {
    let mut md = html.to_string();
    md = html_regex::SCRIPT.replace_all(&md, "").to_string();
    md = html_regex::STYLE.replace_all(&md, "").to_string();
    md = html_regex::COMMENT.replace_all(&md, "").to_string();
    for re in html_regex::CHROME.iter() {
        md = re.replace_all(&md, "").to_string();
    }

    for i in 0..6 {
        let prefix = "#".repeat(i + 1);
        md = html_regex::H_OPEN[i]
            .replace_all(&md, &format!("\n{prefix} "))
            .to_string();
        md = html_regex::H_CLOSE[i].replace_all(&md, "\n").to_string();
    }

    md = html_regex::LINK
        .replace_all(&md, |caps: &regex::Captures<'_>| {
            render_link(
                caps.get(1).map_or("", |m| m.as_str()),
                caps.get(2).map_or("", |m| m.as_str()),
            )
        })
        .to_string();
    md = html_regex::STRONG.replace_all(&md, "**$1**").to_string();
    md = html_regex::EM.replace_all(&md, "*$1*").to_string();
    md = html_regex::CODE.replace_all(&md, "`$1`").to_string();
    md = html_regex::PRE_CODE
        .replace_all(&md, "\n```\n$1\n```\n")
        .to_string();
    md = html_regex::LI.replace_all(&md, "\n- ").to_string();

    md = md.replace("<br>", "\n");
    md = md.replace("<br/>", "\n");
    md = md.replace("<br />", "\n");
    md = md.replace("</p>", "\n\n");

    md = html_regex::TAG.replace_all(&md, "").to_string();

    md = md.replace("&nbsp;", " ");
    md = md.replace("&lt;", "<");
    md = md.replace("&gt;", ">");
    md = md.replace("&amp;", "&");
    md = md.replace("&quot;", "\"");
    md = md.replace("&#39;", "'");

    md = html_regex::EMPTY_BULLETS.replace_all(&md, "").to_string();
    md = html_regex::WHITESPACE.replace_all(&md, "\n\n").to_string();

    md.trim().to_string()
}

/// Render one anchor as markdown (D2). Three general rules, none site-specific:
/// - **empty text** → the whole link is dropped (`[]()` conveys nothing);
/// - **`#fragment` href** or **href longer than [`MAX_URL_CHARS`]** → keep the
///   text, drop the target;
/// - otherwise → `[text](href)`.
fn render_link(href: &str, text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') || href.chars().count() > MAX_URL_CHARS {
        return text.to_string();
    }
    format!("[{text}]({href})")
}

/// Truncate to [`MAX_OUTPUT_CHARS`] at a char boundary, preferring the last
/// newline (D3): walk back to a boundary, then cut at `rfind('\n')` when it is
/// past the halfway point (so the tail is not a half-formed line). Returns
/// `(text, truncated)`.
fn truncate_output(output: String) -> (String, bool) {
    if output.len() <= MAX_OUTPUT_CHARS {
        return (output, false);
    }
    let mut cut = MAX_OUTPUT_CHARS;
    while cut > 0 && !output.is_char_boundary(cut) {
        cut -= 1;
    }
    let cut = match output[..cut].rfind('\n') {
        Some(nl) if nl > MAX_OUTPUT_CHARS / 2 => nl,
        _ => cut,
    };
    (output[..cut].to_string(), true)
}

/// The static regex/const set (D2). Compiles each pattern once (`LazyLock`).
///
/// PIN (friction): jcode returns `Option<&Regex>` and degrades gracefully on a
/// compile error. wcode's patterns are fixed and covered by tests, so
/// `LazyLock<Regex>` + `expect` is used instead — a compile failure is a
/// programming error, not a runtime condition.
mod html_regex {
    use regex::Regex;
    use std::sync::LazyLock;

    macro_rules! re {
        ($name:ident, $pat:expr) => {
            pub static $name: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new($pat).expect(concat!("static regex ", stringify!($name)))
            });
        };
    }

    re!(SCRIPT, r"(?is)<script[^>]*>.*?</script>");
    re!(STYLE, r"(?is)<style[^>]*>.*?</style>");
    // **Attribute-safe tag regex (D2):** matches quoted attribute values
    // (`"…"`/`'…'`) *before* a bare `>`, so a Parsoid-style `data-mw='…>…'`
    // payload does not leak its contents into the text.
    re!(
        TAG,
        r#"(?s)</?[A-Za-z!/][^\s/>]*(?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*))?)*\s*/?>"#
    );
    re!(WHITESPACE, r"\n\s*\n\s*\n");
    // Runs of empty markdown list items left behind after tag stripping.
    re!(EMPTY_BULLETS, r"(?m)^[ \t]*-[ \t]*$\n?");
    re!(LINK, r#"(?i)<a[^>]*href=["']([^"']+)["'][^>]*>([^<]*)</a>"#);
    re!(STRONG, r"(?i)<(?:strong|b)>([^<]*)</(?:strong|b)>");
    re!(EM, r"(?i)<(?:em|i)>([^<]*)</(?:em|i)>");
    re!(CODE, r"(?i)<code>([^<]*)</code>");
    re!(PRE_CODE, r"(?is)<pre[^>]*><code[^>]*>(.+?)</code></pre>");
    re!(LI, r"(?i)<li[^>]*>");
    re!(COMMENT, r"(?s)<!--.*?-->");

    /// HTML elements whose *spec* excludes primary content: navigation,
    /// complementary content, interactive controls, embedded non-text. Notably
    /// EXCLUDES `<header>`/`<footer>` (article title/byline/attribution).
    pub const CHROME_TAGS: [&str; 10] = [
        "nav", "aside", "form", "noscript", "svg", "iframe", "template", "select", "dialog",
        "canvas",
    ];

    /// `<h{n}[^>]*>` for n = 1..=6.
    pub static H_OPEN: LazyLock<[Regex; 6]> = LazyLock::new(|| {
        std::array::from_fn(|i| Regex::new(&format!(r"(?i)<h{}[^>]*>", i + 1)).unwrap())
    });
    /// `</h{n}>` for n = 1..=6.
    pub static H_CLOSE: LazyLock<[Regex; 6]> = LazyLock::new(|| {
        std::array::from_fn(|i| Regex::new(&format!(r"(?i)</h{}>", i + 1)).unwrap())
    });
    /// One `(?is)<tag\b[^>]*>.*?</tag\s*>` per [`CHROME_TAGS`].
    pub static CHROME: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        CHROME_TAGS
            .iter()
            .map(|tag| Regex::new(&format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}\s*>")).unwrap())
            .collect()
    });
}

// ==========================================================================
// Tests
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- converter fixtures (D2) — mirror jcode's `tests` + corpus invariants ----

    #[test]
    fn strips_non_prose_elements() {
        let html = "<nav><a href='/x'>Menu</a></nav><p>Body text</p>\
                    <aside>Related</aside><form><select><option>Pick</option></select></form>";
        let md = html_to_markdown(html);
        assert!(md.contains("Body text"), "{md}");
        assert!(!md.contains("Menu"), "nav should be dropped: {md}");
        assert!(!md.contains("Related"), "aside should be dropped: {md}");
        assert!(!md.contains("Pick"), "form controls dropped: {md}");
    }

    #[test]
    fn keeps_article_header_and_footer_content() {
        // `<header>` usually holds the title/byline and `<footer>` article
        // attribution, so neither is treated as chrome.
        let html = "<article><header><h1>Real Title</h1><p>By Author</p></header>\
                    <p>Body</p><footer>Published 2026</footer></article>";
        let md = html_to_markdown(html);
        for needle in ["Real Title", "By Author", "Body", "Published 2026"] {
            assert!(md.contains(needle), "{needle} missing from {md}");
        }
    }

    #[test]
    fn drops_empty_links_and_overlong_targets() {
        assert_eq!(render_link("https://example.com", ""), "");
        assert_eq!(render_link("#section", "Jump"), "Jump");
        let long = format!("https://example.com/?code={}", "a".repeat(MAX_URL_CHARS));
        assert_eq!(render_link(&long, "Run"), "Run");
        assert_eq!(
            render_link("https://example.com", "Home"),
            "[Home](https://example.com)"
        );
    }

    #[test]
    fn strips_html_comments() {
        let md = html_to_markdown("<p>Keep</p><!-- build:12345 drop me -->");
        assert!(md.contains("Keep"));
        assert!(!md.contains("drop me"), "comment retained: {md}");
    }

    #[test]
    fn does_not_leak_attributes_containing_angle_brackets() {
        // Parsoid-style tags embed JSON in attributes; a naive `<[^>]+>` regex
        // stops at the first `>` inside the value and dumps the rest as text.
        let html = r#"<span data-mw='{"wt":"[[a]] > [[b]]"}'>Visible</span>"#;
        let text = html_to_text(html);
        assert_eq!(text, "Visible");
    }

    #[test]
    fn format_matrix_routes_by_content_type() {
        // html→raw; text→stripped; markdown/unknown→markdown iff text/html else raw.
        assert_eq!(format_body("<p>Hi</p>", "html", "text/html"), "<p>Hi</p>");
        assert_eq!(format_body("<p>Hi</p>", "html", "text/plain"), "<p>Hi</p>");
        assert_eq!(format_body("<p>Hi</p>", "text", "text/html"), "Hi");
        assert_eq!(format_body("<p>Hi</p>", "text", "text/plain"), "Hi");
        assert_eq!(format_body("<p>Hi</p>", "markdown", "text/html"), "Hi");
        assert_eq!(format_body("<p>Hi</p>", "markdown", "text/plain"), "<p>Hi</p>");
        assert_eq!(format_body("<p>Hi</p>", "bogus", "text/html"), "Hi");
        assert_eq!(format_body("<p>Hi</p>", "bogus", "text/plain"), "<p>Hi</p>");
    }

    // ---- output cap (D3) ----

    #[test]
    fn caps_output_length() {
        let long = "line of text\n".repeat(MAX_OUTPUT_CHARS);
        let (out, truncated) = truncate_output(long);
        assert!(truncated);
        assert!(out.len() <= MAX_OUTPUT_CHARS);
    }

    #[test]
    fn keeps_short_output_intact() {
        let (out, truncated) = truncate_output("hello".to_string());
        assert!(!truncated);
        assert_eq!(out, "hello");
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        // Multi-byte chars straddling the cut must not panic or corrupt output.
        let long = "é".repeat(MAX_OUTPUT_CHARS);
        let (out, truncated) = truncate_output(long);
        assert!(truncated);
        assert!(out.chars().all(|c| c == 'é'));
    }

    #[test]
    fn truncation_prefers_the_last_newline() {
        // A newline past the halfway point is the preferred cut, so the tail is
        // not a half-formed line.
        let mut text = "a".repeat(MAX_OUTPUT_CHARS - 5);
        text.push('\n');
        text.push_str(&"b".repeat(100));
        let (out, truncated) = truncate_output(text);
        assert!(truncated);
        assert_eq!(out.len(), MAX_OUTPUT_CHARS - 5, "cut at the last newline");
        assert!(!out.ends_with('\n'));
    }

    // ---- HTTP seam: a raw local TcpListener, no mock-server crate (§7) ----

    /// Bind `127.0.0.1:0`, spawn a task that accepts ONE connection and writes
    /// `raw` verbatim, and return the `http://127.0.0.1:<port>/` URL. `tokio`'s
    /// `full` features (workspace dep) provide `net`/`io`.
    fn serve_raw(raw: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            if let Ok((mut socket, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
                // Consume the request line/headers so the client is not blocked
                // writing, then respond and close.
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let _ = socket.write_all(raw).await;
                let _ = socket.flush().await;
            }
        });
        format!("http://{addr}/")
    }

    /// Like [`serve_raw`] but never writes — the request hangs, for the cancel
    /// test (D9).
    fn serve_hang() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            if let Ok((socket, _)) = listener.accept().await {
                // Hold the socket open, never responding, until the test runtime
                // shuts the task down.
                let _held = socket;
                std::future::pending::<()>().await;
            }
        });
        format!("http://{addr}/")
    }

    /// A `ToolContext` (via `super::super::test_ctx`) whose `cancel` the test can
    /// trip.
    fn ctx_for(dir: &std::path::Path) -> (ToolContext, tokio_util::sync::CancellationToken) {
        let (ctx, _rx) = super::super::test_ctx(dir);
        let token = ctx.cancel.clone();
        (ctx, token)
    }

    #[tokio::test]
    async fn bad_scheme_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _token) = ctx_for(dir.path());
        let out = WebFetch::new()
            .execute(
                WebFetchArgs {
                    url: "ftp://x".into(),
                    format: None,
                    timeout: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(
            out.output.starts_with("URL must start with"),
            "{}",
            out.output
        );
    }

    #[tokio::test]
    async fn non_2xx_is_http_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _token) = ctx_for(dir.path());
        let url = serve_raw(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let out = WebFetch::new()
            .execute(
                WebFetchArgs {
                    url,
                    format: None,
                    timeout: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("HTTP error: 404"), "{}", out.output);
    }

    #[tokio::test]
    async fn content_length_over_cap_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _token) = ctx_for(dir.path());
        let url = serve_raw(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 6000000\r\nConnection: close\r\n\r\n",
        );
        let out = WebFetch::new()
            .execute(
                WebFetchArgs {
                    url,
                    format: None,
                    timeout: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("Response too large"), "{}", out.output);
    }

    #[tokio::test]
    async fn streaming_body_over_cap_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _token) = ctx_for(dir.path());

        // A body just over MAX_SIZE with NO content-length (Connection: close),
        // so the streaming cap (not the header reject) fires. `HELLO` + a run of
        // `<div>` (5 bytes each) keeps the cut on a tag boundary (MAX_SIZE is a
        // multiple of 5) and strips to nothing, so the note survives the output
        // cap.
        let filler = "<div>".repeat(MAX_SIZE / 5 + 10);
        let mut raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n".to_vec();
        raw.extend_from_slice(b"HELLO");
        raw.extend_from_slice(filler.as_bytes());
        let raw: &'static [u8] = Box::leak(raw.into_boxed_slice());
        let url = serve_raw(raw);

        let out = WebFetch::new()
            .execute(
                WebFetchArgs {
                    url,
                    format: Some("text".into()),
                    timeout: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("HELLO"), "{}", out.output);
        assert!(
            out.output
                .contains(&format!("(truncated, showing first {MAX_SIZE} bytes)")),
            "body-truncation note missing: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn cancel_aborts_the_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, token) = ctx_for(dir.path());
        let url = serve_hang();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
        });
        let out = WebFetch::new()
            .execute(
                WebFetchArgs {
                    url,
                    format: None,
                    timeout: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error, "{}", out.output);
        assert_eq!(out.output, "cancelled");
    }

    #[tokio::test]
    async fn success_is_framed_with_the_converted_length() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _token) = ctx_for(dir.path());
        // `<p>Hello</p>` converts (markdown, text/html) to exactly `Hello` (5).
        let url = serve_raw(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 12\r\nConnection: close\r\n\r\n<p>Hello</p>",
        );
        let out = WebFetch::new()
            .execute(
                WebFetchArgs {
                    url: url.clone(),
                    format: None,
                    timeout: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(
            out.output.starts_with(&format!("Fetched {url} (5 bytes)")),
            "{}",
            out.output
        );
        assert!(out.output.contains("Hello"), "{}", out.output);
    }
}
