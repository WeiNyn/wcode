import assert from "node:assert/strict";
import { test } from "node:test";

import { renderMarkdown } from "../src/markdown.ts";

test("renders common markdown", () => {
  const html = renderMarkdown("# Title\n\nsome **bold** text");
  assert.match(html, /<h1>Title<\/h1>/);
  assert.match(html, /<strong>bold<\/strong>/);
});

test("html:false escapes raw HTML instead of passing it through", () => {
  const html = renderMarkdown('<img src=x onerror="alert(1)">');
  assert.ok(!html.includes("<img"), "raw HTML must not survive into the output");
  assert.match(html, /&lt;img/);
});

test("fenced code renders (without token highlighting in P1)", () => {
  const html = renderMarkdown("```js\nconst x = 1;\n```");
  assert.match(html, /<pre><code class="language-js">/);
  assert.ok(!html.includes("hljs"), "no highlighting in P1");
});
