import assert from "node:assert/strict";
import { test } from "node:test";

import { SLASH_COMMANDS, filterCommands, findCommand, parseSlash } from "../src/commands.ts";

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

test("every command has a help line, and arg-requiring ones name an arg", () => {
  for (const command of SLASH_COMMANDS) {
    assert.ok(command.help.length > 0, `${command.name} needs help`);
    if (command.requiresArg === true) assert.ok(command.arg !== undefined, `${command.name} needs an arg hint`);
  }
});
