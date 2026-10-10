/**
 * Project team discovery for the `/team` command (D017) — the VS Code twin of
 * `crates/wcode-cli/src/teams.rs`.
 *
 * A named team is `./.wcode/teams/<name>.toml`; the host lists the available
 * names and opens a NEW session with `--team <name>` (the CLI resolves the path,
 * so this file never builds one). `teamName` is the pure rule; `listTeams` is the
 * thin fs reader the `Manager` calls.
 */
import { readdir, stat } from "node:fs/promises";
import { join } from "node:path";

/** The project teams directory (mirrors the CLI's `TEAMS_DIR`). */
export const TEAMS_DIR = join(".wcode", "teams");

/**
 * The team name a directory entry yields, or `null` when it is not a
 * `<name>.toml` — a dotfile, a non-`toml`, a bare `.toml`. Pure.
 */
export function teamName(fileName: string): string | null {
  if (fileName.startsWith(".")) return null;
  if (!fileName.endsWith(".toml")) return null;
  const stem = fileName.slice(0, -".toml".length);
  return stem === "" ? null : stem;
}

/**
 * Every team name under `<root>/.wcode/teams`, sorted. A missing (or unreadable)
 * directory is an EMPTY list, never an error — the caller decides whether "no
 * teams" matters. A non-file entry (a directory named `x.toml`) is skipped.
 */
export async function listTeams(root: string): Promise<string[]> {
  const dir = join(root, TEAMS_DIR);
  let entries: string[];
  try {
    entries = await readdir(dir);
  } catch {
    return [];
  }
  const names: string[] = [];
  for (const entry of entries) {
    const name = teamName(entry);
    if (name === null) continue;
    try {
      if (!(await stat(join(dir, entry))).isFile()) continue;
    } catch {
      continue; // a broken symlink
    }
    names.push(name);
  }
  return names.sort();
}
