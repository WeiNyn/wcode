/**
 * Markdown rendering for the chat surface.
 *
 * `markdown-it` with `html: false`: raw HTML is ESCAPED rather than passed
 * through, so the injection safety holds without a separate sanitizer. The TUI
 * renders markdown with `pulldown-cmark` + `syntect` (`crates/wcode-tui`), not
 * by hand; P1 ships fenced code WITHOUT token highlighting (the `syntect`
 * analogue is a later decision).
 */
import MarkdownIt from "markdown-it";

const md = new MarkdownIt({
  html: false, // escape raw HTML — never pass it through
  linkify: false,
  breaks: false,
  typographer: false,
});

/**
 * A fenced block renders as the draft's `.code` card: a `.chead` header (the
 * fence LANGUAGE, or "" when the fence has none) over the `.pre` body. We
 * override markdown-it's `fence` rule rather than string-surgery the default
 * `<pre><code>` — the escaping stays in markdown-it's hands. A real filename
 * header belongs to a TOOL's diff preview (P4), never to assistant prose.
 */
md.renderer.rules.fence = (tokens, idx) => {
  const token = tokens[idx];
  const lang = token.info.trim().split(/\s+/)[0] ?? "";
  const label = md.utils.escapeHtml(lang);
  const code = md.utils.escapeHtml(token.content);
  return `<div class="code"><div class="chead"><span>${label}</span></div><pre class="pre">${code}</pre></div>\n`;
};

/** Render markdown source to HTML. */
export function renderMarkdown(source: string): string {
  return md.render(source);
}
