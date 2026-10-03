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

/** Render markdown source to HTML. */
export function renderMarkdown(source: string): string {
  return md.render(source);
}
