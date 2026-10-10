import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { TEAMS_DIR, listTeams, teamName } from "../src/teams.ts";

test("teamName: a `<name>.toml` stem, skipping noise (D017)", () => {
  assert.equal(teamName("scout.toml"), "scout");
  assert.equal(teamName("review-team.toml"), "review-team", "a dash is part of the name");
  assert.equal(teamName("scout.md"), null, "only .toml");
  assert.equal(teamName(".hidden.toml"), null, "a dotfile is skipped");
  assert.equal(teamName(".toml"), null, "an empty stem is skipped");
  assert.equal(teamName("scout"), null, "no extension");
});

test("listTeams: sorted, files only, and a missing dir is empty", async () => {
  const root = mkdtempSync(join(tmpdir(), "wcode-teams-"));
  const dir = join(root, TEAMS_DIR);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "scout.toml"), "");
  writeFileSync(join(dir, "beta.toml"), "");
  writeFileSync(join(dir, "notes.md"), ""); // not a team
  writeFileSync(join(dir, ".hidden.toml"), ""); // skipped
  mkdirSync(join(dir, "adir.toml")); // a DIRECTORY named *.toml is not a team
  assert.deepEqual(await listTeams(root), ["beta", "scout"]);
  rmSync(root, { recursive: true, force: true });

  assert.deepEqual(await listTeams(join(tmpdir(), "wcode-teams-does-not-exist")), []);
});
