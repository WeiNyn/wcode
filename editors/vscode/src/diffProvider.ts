/**
 * The `wcode-diff:` content provider and the click handler that opens a native
 * diff. The ONLY place the diff feature meets `vscode`; everything it can test
 * lives in `diff.ts` (pure).
 *
 * Plan §3.3: reverse-apply the patch to reconstruct the pre-image, serve both
 * sides to `vscode.diff`, open **on request**. The provider resolves purely from
 * an in-memory registry — it NEVER reads disk. The right side is the real
 * on-disk file, which `vscode.diff` opens itself.
 */
import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";

import {
  BeforeRegistry,
  DIFF_SCHEME,
  diffTokenPath,
  findToolBlock,
  lastDiffCallId,
  reverseApply,
  tokenFromDiffPath,
} from "./diff.ts";
import { transcriptOf, type ViewState } from "./reducer.ts";

/** A diagnostic sink (the extension's OutputChannel). */
export type DiffLogger = (message: string) => void;

/**
 * Module-level: the provider (resolve) and the click handler (mint) share it,
 * and it must survive across `openDiff` calls.
 */
const beforeRegistry = new BeforeRegistry(32);

/** Drop every minted token — on session stop / panel dispose. */
export function clearDiffs(): void {
  beforeRegistry.clear();
}

class WcodeDiffProvider implements vscode.TextDocumentContentProvider {
  private readonly onDidChangeEmitter = new vscode.EventEmitter<vscode.Uri>();
  private readonly log: DiffLogger;

  constructor(log: DiffLogger) {
    this.log = log;
  }

  /** VS Code re-requests an open doc through this event. */
  readonly onDidChange = this.onDidChangeEmitter.event;

  /** Resolve purely from the registry; an evicted/unknown token yields `""`. */
  provideTextDocumentContent(uri: vscode.Uri, _token: vscode.CancellationToken): string {
    const token = tokenFromDiffPath(uri.path);
    const before = beforeRegistry.resolve(token);
    if (before === undefined) {
      this.log(`wcode: unknown or evicted diff token \`${token}\``);
      return "";
    }
    return before;
  }

  dispose(): void {
    this.onDidChangeEmitter.dispose();
  }
}

/** Register the provider (pushed onto the extension's subscriptions). */
export function registerDiffProvider(context: vscode.ExtensionContext, log: DiffLogger): void {
  const provider = new WcodeDiffProvider(log);
  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider(DIFF_SCHEME, provider),
    provider,
  );
}

/**
 * Open the diff for a tool call: the reconstructed before-image on the left, the
 * real on-disk file on the right. When `reverseApply` cannot reconstruct it, open
 * the patch text read-only instead — never an enabled control that opens nothing.
 */
export async function openDiff(
  state: ViewState,
  workspaceRoot: string | undefined,
  callId: string | undefined,
  log: DiffLogger,
): Promise<void> {
  const blocks = transcriptOf(state, state.targeted);
  const id = callId ?? lastDiffCallId(blocks);
  if (id === undefined) {
    void vscode.window.showInformationMessage("wcode: no diff to open yet.");
    return;
  }

  const tool = findToolBlock(blocks, id);
  const diff = tool?.diff;
  if (tool === undefined || diff === undefined || diff === "") {
    void vscode.window.showWarningMessage("wcode: that tool call has no diff to open.");
    return;
  }

  const resolved = resolvePath(workspaceRoot, tool.path);
  if (resolved === undefined) {
    void vscode.window.showWarningMessage("wcode: this tool call names no file to diff.");
    return;
  }
  if (!fs.existsSync(resolved)) {
    void vscode.window.showWarningMessage(`wcode: ${resolved} no longer exists.`);
    return;
  }

  let current: string;
  try {
    current = fs.readFileSync(resolved, "utf8");
  } catch (err) {
    void vscode.window.showWarningMessage(`wcode: could not read ${resolved}: ${messageOf(err)}`);
    return;
  }

  const basename = path.basename(resolved);
  const before = reverseApply(current, diff);

  if (before === null) {
    // The patch cannot be reversed exactly (truncated, foreign, or the file has
    // moved on). Show the patch itself, read-only, rather than a wrong window.
    log(`wcode: reverse-apply failed for ${resolved}; opening the patch text`);
    const patch = diffUri(beforeRegistry.mint(diff));
    const empty = diffUri(beforeRegistry.mint(""));
    await vscode.commands.executeCommand("vscode.diff", empty, patch, `wcode: ${basename} (patch only)`, {
      preview: true,
    });
    return;
  }

  const left = diffUri(beforeRegistry.mint(before));
  const right = vscode.Uri.file(resolved);
  // No em-dash in a UI string (the repo bans it); one parenthesised direction.
  await vscode.commands.executeCommand("vscode.diff", left, right, `wcode: ${basename} (before → after)`, {
    preview: true,
  });
}

/* ------------------------------------------------------------------ helpers */

function diffUri(token: string): vscode.Uri {
  return vscode.Uri.from({ scheme: DIFF_SCHEME, path: diffTokenPath(token) });
}

/** `ToolOutput.path` is "as the caller named it" — often RELATIVE to the workspace. */
function resolvePath(root: string | undefined, toolPath: string | undefined): string | undefined {
  if (toolPath === undefined || toolPath === "") return undefined;
  if (path.isAbsolute(toolPath)) return toolPath;
  return root === undefined ? undefined : path.join(root, toolPath);
}

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
