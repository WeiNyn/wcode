/**
 * The `/`-command registry — the ONE list the menu and the host share.
 *
 * **Pure** — no `vscode`, no DOM, no I/O — so plain node drives it: the filter and
 * the parse are the testable halves of the menu, and the host validates a posted
 * name against the same table it rendered.
 *
 * The webview never speaks the wire: it posts `{ kind: "command", name, arg }` and
 * the host decides whether that is a `Request` (`set_model`), a composition-root
 * action (a fresh child for `/new`), or a local picker (`/resume`).
 */

/** One `/` command the surface can run. */
export interface SlashCommand {
  /** The typed name, without the slash. */
  name: string;
  /** Other spellings that resolve here (the CLI's `clear` for `new`). */
  aliases?: string[];
  /** One line — the menu's right column. */
  help: string;
  /** The argument placeholder, or undefined when the command takes none. */
  arg?: string;
  /** True when the command is useless without an argument (the menu waits for it). */
  requiresArg?: boolean;
}

/**
 * Tab's completion text for `command` — `/<name>` plus a trailing space IFF the command
 * takes an argument (`command.arg` set), else the bare name. Pure: the ONE place the
 * `/<name> ` vs `/<name>` form is decided, so the webview and its test agree. Tab ARMS
 * the line; it never RUNS it (Enter runs).
 */
export function completionText(command: SlashCommand): string {
  return "/" + command.name + (command.arg !== undefined ? " " : "");
}

export const SLASH_COMMANDS: readonly SlashCommand[] = [
  { name: "new", aliases: ["clear"], help: "Start a fresh session" },
  { name: "reload", help: "Restart the session" },
  { name: "resume", arg: "[path]", help: "Resume a session: pick one, or pass a path" },
  { name: "model", arg: "[id]", help: "Switch the model: pick one, or pass an id" },
  { name: "effort", arg: "[level | -]", help: "Set the reasoning effort: pick one, or pass a level (- clears it)" },
  { name: "plan", help: "Toggle plan mode" },
  { name: "btw", arg: "<question>", help: "Side question: no turn, not recorded", requiresArg: true },
  { name: "compact", arg: "[focus]", help: "Summarize older messages now" },
  { name: "sessions", help: "List the sessions in this connection" },
  { name: "status", help: "Show the target's last work" },
];

/** Resolve a name or an alias. Pure. */
export function findCommand(name: string): SlashCommand | undefined {
  const wanted = name.toLowerCase();
  return SLASH_COMMANDS.find((c) => c.name === wanted || (c.aliases?.includes(wanted) ?? false));
}

/**
 * The commands a `/`-query matches, in registry order. Pure. The query is the text
 * after the slash (before any space); an empty query offers every command.
 */
export function filterCommands(query: string): SlashCommand[] {
  const q = query.trim().toLowerCase();
  if (q === "") return [...SLASH_COMMANDS];
  return SLASH_COMMANDS.filter(
    (c) => c.name.startsWith(q) || (c.aliases?.some((a) => a.startsWith(q)) ?? false),
  );
}

/**
 * Split a `/name arg` line into a KNOWN command and its argument, or `null` — for
 * an unknown `/…` the caller falls through to a normal submit (the CLI's rule:
 * anything that is not a command is prompt text). Pure.
 */
export function parseSlash(line: string): { command: SlashCommand; arg: string } | null {
  const trimmed = line.trim();
  if (!trimmed.startsWith("/")) return null;
  const rest = trimmed.slice(1);
  const space = rest.search(/\s/);
  const name = space < 0 ? rest : rest.slice(0, space);
  const arg = space < 0 ? "" : rest.slice(space + 1).trim();
  const command = findCommand(name);
  return command === undefined ? null : { command, arg };
}
