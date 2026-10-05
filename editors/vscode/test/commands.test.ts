import assert from "node:assert/strict";
import { test } from "node:test";

import { SLASH_COMMANDS, completionText, filterCommands, findCommand, parseSlash } from "../src/commands.ts";

test("findCommand resolves a name and every alias", () => {
  assert.equal(findCommand("model")?.name, "model");
  assert.equal(findCommand("new")?.name, "new");
  assert.equal(findCommand("clear")?.name, "new", "`clear` is an alias of `new`");
  assert.equal(findCommand("nope"), undefined);
});

test("filterCommands narrows by prefix and offers everything for an empty query", () => {
  assert.equal(filterCommands("").length, SLASH_COMMANDS.length);
  assert.deepEqual(filterCommands("mo").map((c) => c.name), ["model"]);
  assert.deepEqual(filterCommands("re").map((c) => c.name), ["reload", "resume"]);
  // An alias matches too, and still yields the CANONICAL command.
  assert.deepEqual(filterCommands("cle").map((c) => c.name), ["new"]);
  assert.deepEqual(filterCommands("zzz"), []);
});

test("parseSlash splits a known command and its argument", () => {
  assert.deepEqual(parseSlash("/model gpt-x"), { command: findCommand("model"), arg: "gpt-x" });
  assert.deepEqual(parseSlash("/plan"), { command: findCommand("plan"), arg: "" });
  assert.deepEqual(parseSlash("  /btw why?  "), { command: findCommand("btw"), arg: "why?" });
  // An alias resolves to the canonical command.
  assert.equal(parseSlash("/clear")?.command.name, "new");
  // The argument keeps interior spaces (a side question is a sentence).
  assert.equal(parseSlash("/btw why this way?")?.arg, "why this way?");
});

test("parseSlash rejects a non-command so the caller submits it as prompt text", () => {
  assert.equal(parseSlash("hello"), null);
  assert.equal(parseSlash("/nonsense x"), null, "an unknown /… is prompt text, the CLI's rule");
  assert.equal(parseSlash("/"), null);
});

test("completionText arms a space only for arg-taking commands", () => {
  const text = (name: string): string => {
    const command = findCommand(name);
    assert.ok(command !== undefined, `/${name} is a command`);
    return completionText(command);
  };
  assert.equal(text("model"), "/model ");
  assert.equal(text("resume"), "/resume ");
  assert.equal(text("plan"), "/plan", "no arg hint ⇒ the bare name");
  assert.equal(text("new"), "/new");
});

test("/model and /effort are OPTIONAL-arg now (an empty arg opens a host picker)", () => {
  assert.notEqual(findCommand("model")?.requiresArg, true);
  assert.notEqual(findCommand("effort")?.requiresArg, true);
  // The arg HINT stays, so the menu shows it and Tab completes to `/<name> `.
  assert.notEqual(findCommand("model")?.arg, undefined);
  assert.notEqual(findCommand("effort")?.arg, undefined);
  // A bare `/model`/`/effort` parses with an EMPTY arg — the picker path.
  assert.deepEqual(parseSlash("/model"), { command: findCommand("model"), arg: "" });
  assert.deepEqual(parseSlash("/effort"), { command: findCommand("effort"), arg: "" });
  // A typed arg still carries through (the direct path).
  assert.equal(parseSlash("/model gpt-x")?.arg, "gpt-x");
  assert.equal(parseSlash("/effort high")?.arg, "high");
});

test("every command has a help line, and arg-requiring ones name an arg", () => {
  for (const command of SLASH_COMMANDS) {
    assert.ok(command.help.length > 0, `${command.name} needs help`);
    if (command.requiresArg === true) assert.ok(command.arg !== undefined, `${command.name} needs an arg hint`);
  }
});
