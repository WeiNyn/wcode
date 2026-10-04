/**
 * The `/resume` picker's data: the session files under the session dir.
 *
 * The PURE half (`parseHeader`, `firstUserText`, `formatWhen`) is testable under
 * plain node; `listSessions` is the thin fs reader the host calls. The layout is
 * `session_groups.rs`'s:
 *
 * ```text
 * <base>/<millis>_<id8>/root.jsonl   # a session GROUP (its whole team resumes)
 * <base>/<millis>_<id8>.jsonl        # a legacy flat session
 * ```
 *
 * The file is a `{type:"header"}` line (cwd + created) then `{type:"message"}`
 * lines; we read only the head, so a 1 MB transcript costs 32 KiB.
 */
import { open, readdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { parseInbound } from "./reducer.ts";

/** One resumable session, ready for the picker. */
export interface SessionChoice {
  /** Handed back verbatim to `--resume`: a group dir or a flat file. */
  path: string;
  /** The FIRST user message (tag stripped, clipped) — never the raw id. */
  label: string;
  /** The working dir the session recorded. */
  cwd: string;
  /** `YYYY-MM-DD HH:MM` in local time, or "" when the header had none. */
  when: string;
}

/** The CLI's session dir (`repl::session_dir`). */
export function sessionDir(): string {
  return join(homedir(), ".local/share/wcode/sessions");
}


function safeParse(line: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(line);
    return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** The `{type:"header"}` line's fields, or `null` when this is not a header. Pure. */
export function parseHeader(line: string): { cwd: string; created: string } | null {
  const record = safeParse(line);
  if (record === null || record.type !== "header") return null;
  return {
    cwd: typeof record.cwd === "string" ? record.cwd : "",
    created: typeof record.created === "string" ? record.created : "",
  };
}

/**
 * The first USER message's text across a session file's head, tag stripped and
 * clipped — the picker's label. Pure. `""` when the head holds no user message.
 */
export function firstUserText(lines: string[], max = 90): string {
  for (const line of lines) {
    const record = safeParse(line);
    if (record === null || record.type !== "message") continue;
    const message = record.message;
    if (message === null || typeof message !== "object") continue;
    const envelope = message as Record<string, unknown>;
    if (envelope.role !== "user") continue;
    const text = textOfContent(envelope.content);
    if (text.trim() === "") continue;
    // The harness tags every inbound message (`[message from user]\n…`) — the
    // human's own too. A tag naming a PEER is not the human's words, so it is not
    // the label; the human's own tag is stripped.
    const inbound = parseInbound(text);
    if (inbound !== null && inbound.from !== "user") continue;
    const body = (inbound?.body ?? text).replace(/\s+/g, " ").trim();
    return body.length > max ? `${body.slice(0, max - 1)}…` : body;
  }
  return "";
}

/** Join a message's `text` content blocks. Pure. */
function textOfContent(content: unknown): string {
  if (!Array.isArray(content)) return "";
  return content
    .filter((b): b is { type: string; text: string } => {
      const block = b as Record<string, unknown> | null;
      return block !== null && typeof block === "object" && block.type === "text" && typeof block.text === "string";
    })
    .map((b) => b.text)
    .join("");
}

/** `YYYY-MM-DD HH:MM` (local) for an ISO timestamp, or "" when unparseable. Pure. */
export function formatWhen(created: string): string {
  const date = new Date(created);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Read at most `bytes` from a file's start (a transcript can be megabytes). */
async function readHead(path: string, bytes = 32 * 1024): Promise<string> {
  const handle = await open(path, "r");
  try {
    const buffer = Buffer.alloc(bytes);
    const { bytesRead } = await handle.read(buffer, 0, bytes, 0);
    return buffer.subarray(0, bytesRead).toString("utf8");
  } finally {
    await handle.close();
  }
}

/**
 * Every resumable session in `dir`, NEWEST FIRST. When `cwd` is non-empty, only
 * sessions whose header recorded that working dir — a picker of another project's
 * conversations is noise. A group dir (or a flat file) whose head cannot be read
 * is skipped, never fatal.
 */
export async function listSessions(dir: string, cwd = ""): Promise<SessionChoice[]> {
  let entries: string[];
  try {
    entries = await readdir(dir);
  } catch {
    return [];
  }
  // `{millis}_{hex}` names sort lexicographically = chronologically; newest first.
  entries.sort((a, b) => b.localeCompare(a));

  const choices: SessionChoice[] = [];
  for (const name of entries) {
    if (name.startsWith(".")) continue;
    const isFlat = name.endsWith(".jsonl");
    const path = join(dir, name);
    const root = isFlat ? path : join(path, "root.jsonl");
    let head: string;
    try {
      head = await readHead(root);
    } catch {
      continue; // not a session (a stray dir, an unreadable file)
    }
    const lines = head.split("\n");
    const header = lines.map(parseHeader).find((h) => h !== null) ?? null;
    if (header === null) continue;
    if (cwd !== "" && header.cwd !== cwd) continue;
    choices.push({
      path,
      label: firstUserText(lines) || "(no user message)",
      cwd: header.cwd,
      when: formatWhen(header.created),
    });
  }
  return choices;
}
