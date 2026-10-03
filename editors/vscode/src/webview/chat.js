// The webview panel — dependency-free vanilla JS, bundled by esbuild as an IIFE.
//
// The host renders markdown and posts pre-rendered HTML (`ToWebview`); this
// script is a pure renderer of that snapshot. It never parses markdown, never
// loads a remote resource, and only talks to the host over `postMessage`.
(function () {
  "use strict";

  var vscode = acquireVsCodeApi();

  var app = document.getElementById("app");
  app.innerHTML = [
    '<header id="status" class="status"></header>',
    '<main id="transcript" class="transcript"></main>',
    '<footer id="composer" class="composer">',
    '  <textarea id="input" rows="1" spellcheck="false"',
    '    placeholder="Message wcode…  (Enter to send, Shift+Enter for newline, Esc to cancel)"></textarea>',
    '  <div class="composer-actions">',
    '    <button id="send" class="btn primary">Send</button>',
    '    <button id="cancel" class="btn" title="Cancel the in-flight run (Esc)">Cancel</button>',
    '    <span id="hint" class="hint">Enter to send · Shift+Enter newline · Esc cancel</span>',
    "  </div>",
    "</footer>",
  ].join("\n");

  var statusEl = document.getElementById("status");
  var transcriptEl = document.getElementById("transcript");
  var inputEl = document.getElementById("input");
  var sendBtn = document.getElementById("send");
  var cancelBtn = document.getElementById("cancel");

  /** callIds whose tool output is expanded (survives a re-render). */
  var expanded = new Set();
  /** The last snapshot, so expand/collapse can re-render without the host. */
  var lastState = null;

  /* ------------------------------------------------------------- utilities */

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function post(message) {
    vscode.postMessage(message);
  }

  function nearBottom() {
    return transcriptEl.scrollHeight - transcriptEl.scrollTop - transcriptEl.clientHeight < 48;
  }

  function rerender() {
    if (lastState) render(lastState.state, lastState.session);
  }

  /* --------------------------------------------------------------- status */

  function stateLabel(state) {
    switch (state) {
      case "ready":
        return "ready";
      case "starting":
        return "starting";
      case "crashed":
        return "crashed";
      default:
        return "stopped";
    }
  }

  function renderStatus(state, session) {
    statusEl.textContent = "";
    statusEl.appendChild(el("span", "dot dot-" + session.state));
    statusEl.appendChild(el("span", "status-state", stateLabel(session.state)));
    statusEl.appendChild(el("span", "status-sep", "·"));
    statusEl.appendChild(el("span", "status-session", session.id || "no session yet"));
    if (state.status.running) {
      statusEl.appendChild(el("span", "status-sep", "·"));
      statusEl.appendChild(el("span", "status-running", "running…"));
    }
    if (typeof state.status.contextUsed === "number") {
      statusEl.appendChild(el("span", "status-sep", "·"));
      statusEl.appendChild(el("span", "status-ctx", "ctx " + state.status.contextUsed));
    }
    if (state.status.lastError) {
      statusEl.appendChild(el("span", "status-sep", "·"));
      statusEl.appendChild(el("span", "status-error", state.status.lastError));
    }
    if (session.state === "crashed" && session.stderrTail) {
      var details = el("details", "stderr-block");
      details.appendChild(el("summary", null, "wcode stderr (tail)"));
      details.appendChild(el("pre", "stderr", session.stderrTail));
      statusEl.appendChild(details);
    }
  }

  /* --------------------------------------------------------------- blocks */

  function renderEmpty(state, session) {
    var wrap = el("div", "empty");
    if (session.state === "crashed") {
      wrap.appendChild(el("p", "empty-title", "wcode crashed."));
      wrap.appendChild(el("p", "empty-sub", "Run “wcode: Restart” to try again."));
    } else if (session.state === "starting") {
      wrap.appendChild(el("p", "empty-title", "Starting wcode…"));
    } else if (session.state === "stopped") {
      wrap.appendChild(el("p", "empty-title", "wcode is stopped."));
      wrap.appendChild(el("p", "empty-sub", "Run “wcode: Start Session”."));
    } else if (state.status.running) {
      wrap.appendChild(el("p", "empty-title", "Working…"));
    } else {
      wrap.appendChild(el("p", "empty-title", "No messages yet."));
      wrap.appendChild(el("p", "empty-sub", "Type below and press Enter."));
    }
    return wrap;
  }

  function blockShell(kind, role, html, extraClass) {
    var wrap = el("div", "block " + kind + (extraClass ? " " + extraClass : ""));
    if (role) wrap.appendChild(el("div", "role", role));
    var body = el("div", "body");
    body.innerHTML = html;
    wrap.appendChild(body);
    return wrap;
  }

  function renderTool(tool) {
    if (!tool) return el("div", "block tool");
    var isOpen = expanded.has(tool.callId);
    var wrap = el("div", "block tool" + (tool.isError ? " error" : "") + (isOpen ? " expanded" : ""));

    var head = el("div", "tool-head");
    var toggle = el("button", "tool-toggle");
    toggle.appendChild(el("span", "chev", isOpen ? "▾" : "▸"));
    toggle.appendChild(el("span", "tool-name", "⚙ " + tool.name));
    toggle.appendChild(el("span", "tool-summary", tool.summary));
    if (typeof tool.durationMs === "number") {
      toggle.appendChild(el("span", "tool-dur", tool.durationMs + "ms"));
    }
    toggle.addEventListener("click", function () {
      if (expanded.has(tool.callId)) expanded.delete(tool.callId);
      else expanded.add(tool.callId);
      rerender();
    });
    head.appendChild(toggle);

    // P2 hook: the diff affordance exists but is disabled — diffs land in P2.
    if (tool.hasDiff || tool.path) {
      var diff = el("button", "tool-diff", "show diff");
      diff.disabled = true;
      diff.title = "Diffs land in P2";
      diff.addEventListener("click", function () {
        post({ kind: "open-diff", callId: tool.callId });
      });
      head.appendChild(diff);
    }
    wrap.appendChild(head);

    if (isOpen) {
      var body = el("div", "tool-body");
      body.innerHTML = tool.outputHtml;
      wrap.appendChild(body);
    }
    return wrap;
  }

  function renderBlock(block) {
    switch (block.kind) {
      case "user":
        return blockShell("user", "you", block.html);
      case "assistant":
        return blockShell("assistant", "wcode", block.html, block.live ? "live" : "");
      case "error":
        return blockShell("error", "error", block.html);
      case "btw":
        return blockShell("btw", "btw", block.html);
      case "tool":
        return renderTool(block.tool);
      default:
        return blockShell("notice", null, block.html);
    }
  }

  function render(state, session) {
    var stick = nearBottom();
    renderStatus(state, session);
    transcriptEl.textContent = "";
    if (!state.blocks || state.blocks.length === 0) {
      transcriptEl.appendChild(renderEmpty(state, session));
    } else {
      for (var i = 0; i < state.blocks.length; i += 1) {
        transcriptEl.appendChild(renderBlock(state.blocks[i]));
      }
    }
    if (stick) transcriptEl.scrollTop = transcriptEl.scrollHeight;
  }

  /* ---------------------------------------------------------------- input */

  // Grow via the `rows` attribute, not an inline style: the panel's CSP is
  // `style-src ${webview.cspSource}` with no 'unsafe-inline'.
  function autoGrow() {
    var lines = inputEl.value.split("\n").length;
    inputEl.rows = Math.max(1, Math.min(lines, 8));
  }

  function submit() {
    var text = inputEl.value;
    if (text.trim() === "") return;
    post({ kind: "submit", text: text });
    inputEl.value = "";
    autoGrow();
    inputEl.focus();
  }

  inputEl.addEventListener("input", autoGrow);
  inputEl.addEventListener("keydown", function (event) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      post({ kind: "cancel" });
    }
  });
  sendBtn.addEventListener("click", submit);
  cancelBtn.addEventListener("click", function () {
    post({ kind: "cancel" });
  });

  /* --------------------------------------------------------------- host io */

  window.addEventListener("message", function (event) {
    var message = event.data;
    if (!message || typeof message !== "object") return;
    if (message.kind === "state") {
      lastState = message;
      render(message.state, message.session);
    } else if (message.kind === "append") {
      if (!lastState) return;
      lastState.state.blocks.push(message.block);
      var stick = nearBottom();
      transcriptEl.appendChild(renderBlock(message.block));
      if (stick) transcriptEl.scrollTop = transcriptEl.scrollHeight;
    } else if (message.kind === "diff") {
      // P2: a diff became available. Nothing to render yet.
    }
  });

  // The handshake: the host holds its snapshot until we say we are listening.
  post({ kind: "ready" });
  inputEl.focus();
})();
