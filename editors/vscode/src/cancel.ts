/**
 * The host's Cancel-target decision — pure (no `vscode`), so plain node can drive it.
 *
 * The webview's Stop and Esc both post `{ kind: "cancel" }`, and the surface passes its
 * current `target` (`null` in All mode). WHICH member ids the host then addresses is this
 * function's job:
 *   - **Focus mode** (`target !== null`): the focused member alone.
 *   - **All mode** (`target === null`): every in-flight run — the root plus each member
 *     whose `state === "running"` ("All = the whole session").
 */
import type { SessionMember } from "./reducer.ts";

/** The member ids a Cancel must reach. Pure. */
export function cancelTargets(target: string | null, members: SessionMember[], rootId: string | null): string[] {
  if (target !== null) return [target];
  const ids = new Set<string>();
  if (rootId !== null) ids.add(rootId);
  for (const member of members) {
    if (member.state === "running") ids.add(member.id);
  }
  return [...ids];
}
