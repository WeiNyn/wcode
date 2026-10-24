//! CLI-level flag behavior that needs a real process.
//!
//! `parse_args` does not validate `--endpoint`; `load_config` is the program's
//! only validation site. A bad value must therefore fail LOUDLY (exit 2, naming
//! the value) rather than fall through. `--dump-system-prompt` keeps the run
//! offline and config-free: `load_config` fails before the prompt is dumped.

use std::process::Command;

#[test]
fn an_invalid_endpoint_flag_exits_2_and_names_the_value() {
    let output = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .args(["--endpoint", "grpc", "--dump-system-prompt"])
        .output()
        .expect("spawn the wcode binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a bad --endpoint must exit 2; stderr: {stderr}"
    );
    assert!(
        stderr.contains("grpc"),
        "the error must name the bad value, got: {stderr}"
    );
}

/// A temp cwd shipping `./.wcode/config.toml` + `./.wcode/team.toml` (and a temp
/// `$HOME` so no global config leaks in). Returns `(dir, `.wcode` path)`.
fn project_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wcode = dir.path().join(".wcode");
    std::fs::create_dir_all(&wcode).unwrap();
    // config.toml folds first (layer 2), team.toml second (layer 3): the team and
    // the orchestrator guidelines both come from the auto-discovered pair.
    std::fs::write(
        wcode.join("config.toml"),
        "[orchestrator]\nguidelines = \"from-config\"\n",
    )
    .unwrap();
    std::fs::write(
        wcode.join("team.toml"),
        "[[team]]\nname = \"explorer\"\nrole = \"map the ground\"\n",
    )
    .unwrap();
    dir
}

#[test]
fn a_project_team_is_auto_discovered_and_rendered() {
    let dir = project_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .arg("--dump-system-prompt")
        .output()
        .expect("spawn the wcode binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Your team"), "team block missing: {stdout}");
    assert!(stdout.contains("explorer"), "member missing: {stdout}");
    // `config.toml` (layer 2) folded before `team.toml` (layer 3).
    assert!(
        stdout.contains("# Orchestrator workflow") && stdout.contains("from-config"),
        "orchestrator guidelines from config.toml missing: {stdout}"
    );
}

#[test]
fn no_project_config_disables_discovery() {
    let dir = project_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .args(["--no-project-config", "--dump-system-prompt"])
        .output()
        .expect("spawn the wcode binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("# Your team"),
        "--no-project-config must drop the auto team: {stdout}"
    );
    assert!(
        !stdout.contains("# Orchestrator workflow"),
        "--no-project-config must drop the auto guidelines: {stdout}"
    );
}

#[test]
fn wcode_project_config_env_off_disables_discovery() {
    let dir = project_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("WCODE_PROJECT_CONFIG", "off")
        .env_remove("WCODE_CONFIG")
        .arg("--dump-system-prompt")
        .output()
        .expect("spawn the wcode binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("# Your team"),
        "WCODE_PROJECT_CONFIG=off must drop the auto team: {stdout}"
    );
}

#[test]
fn a_project_team_auto_enables_agents_without_the_flag() {
    // D-B1: a non-empty declared `[team]` turns agents mode on, so the
    // `[team] requires --agents` guard must NOT fire (exit 2). The run proceeds
    // to a refused local port and fails on the provider, not the guard.
    let dir = project_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("WCODE_RETRY_MAX", "0")
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .args([
            "--no-session",
            "-p",
            "hi",
            "--model",
            "test",
            "--base-url",
            "http://127.0.0.1:9/v1",
        ])
        .output()
        .expect("spawn the wcode binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(
        output.status.code(),
        Some(2),
        "the [team]-requires--agents guard fired: {stderr}"
    );
    assert!(
        !stderr.contains("requires --agents"),
        "agents must auto-enable from the team: {stderr}"
    );
    // The team actually spawned: `team: <names>` on stderr proves agents mode
    // turned on (a team-less run prints no such line). The diagnostic is stderr
    // — stdout is reserved for the `--stdio` frame stream.
    assert!(
        stderr.contains("team: explorer"),
        "the team must be instantiated: {stderr}"
    );
}

#[test]
fn a_team_of_md_files_without_a_team_toml_does_not_enable_agents() {
    // W033 §3.8 / §5.2 — the deliberate break, owned loudly: a repo full of
    // `.wcode/agents/*.md` and no `[[team]]` must NOT switch agents mode on. It is
    // the D-B1 test above with the declaration taken away.
    let dir = project_with_agent_md("");
    let run = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("WCODE_RETRY_MAX", "0")
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .args([
            "--no-session",
            "-p",
            "hi",
            "--model",
            "test",
            "--base-url",
            "http://127.0.0.1:9/v1",
        ])
        .output()
        .expect("spawn the wcode binary");
    let stderr = String::from_utf8_lossy(&run.stderr);
    // (1) The `[team] requires --agents` guard cannot fire — there is no team.
    assert_ne!(run.status.code(), Some(2), "{stderr}");
    // (2) Nothing was instantiated: a booted team prints `team: <names>`.
    assert!(
        !stderr.lines().any(|l| l.starts_with("team: ")),
        "agents mode must not switch on without a declaration: {stderr}"
    );
    // (3) And the config oracle agrees: no `team.` line in that cwd.
    assert!(
        !dump_config(&dir, &[]).contains("team."),
        "a `.md` on disk is not a declaration"
    );
}

/// A temp cwd shipping two agent `.md`s nobody has declared (`explorer`, body
/// `recon body`; `hidden`, body `hidden body`) plus a temp `$HOME`. `toml`, when
/// non-empty, is written to `.wcode/team.toml` — that is the ONLY thing that can
/// make a `.md` here a team member (W033/D037), so the tests below are its
/// declared/undeclared pair. `hidden` is the tooth: it is uniquely named, so a
/// re-introduced directory scan cannot hide behind the fold's name-dedup.
fn project_with_agent_md(toml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wcode = dir.path().join(".wcode");
    let agents = wcode.join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("explorer.md"),
        "---\nname: explorer\ndescription: recon\n---\nrecon body\n",
    )
    .unwrap();
    std::fs::write(
        agents.join("hidden.md"),
        "---\nname: hidden\n---\nhidden body\n",
    )
    .unwrap();
    if !toml.is_empty() {
        std::fs::write(wcode.join("team.toml"), toml).unwrap();
    }
    dir
}

/// `--dump-config` prints one `team.<name>:` line per member and NO other line
/// contains `team.`, so `contains("team.")` is a clean roster-empty oracle and
/// `matches("team.").count()` a clean member-count one. It needs a model
/// (`--dump-system-prompt` does not).
fn dump_config(dir: &tempfile::TempDir, extra: &[&str]) -> String {
    let mut args: Vec<&str> = extra.to_vec();
    args.extend(["--model", "m", "--dump-config"]);
    let output = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .args(&args)
        .output()
        .expect("spawn the wcode binary");
    assert!(
        output.status.success(),
        "--dump-config must exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn dump_prompt(dir: &tempfile::TempDir, extra: &[&str]) -> String {
    let mut args: Vec<&str> = extra.to_vec();
    args.push("--dump-system-prompt");
    let output = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .args(&args)
        .output()
        .expect("spawn the wcode binary");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn an_agent_md_alone_boots_no_team() {
    // W033/D037: one way to be a member — a `[[team]]` entry. A `.md` on disk
    // that nothing references contributes nothing, on either surface.
    let dir = project_with_agent_md("");
    let prompt = dump_prompt(&dir, &[]);
    assert!(!prompt.contains("# Your team"), "no roster block: {prompt}");
    assert!(!prompt.contains("recon body"), "no member line: {prompt}");
    assert!(!prompt.contains("hidden body"), "no scanned member: {prompt}");
    // The other surface, so this cannot rot into an absence-only assertion.
    let config = dump_config(&dir, &[]);
    assert!(
        !config.contains("team."),
        "an unreferenced `.wcode/agents/*.md` is not a member: {config}"
    );
}

#[test]
fn a_declared_file_member_boots_the_role() {
    // The SAME cwd plus one line of declaration: now it is a team.
    let dir = project_with_agent_md("[[team]]\nfile = \".wcode/agents/explorer.md\"\n");
    let prompt = dump_prompt(&dir, &[]);
    assert!(
        prompt.contains("# Your team"),
        "roster block missing: {prompt}"
    );
    // Body → role (D-C4) still holds: the file supplies the member.
    assert!(prompt.contains("- explorer — recon body"), "{prompt}");
    let config = dump_config(&dir, &[]);
    assert!(
        config.contains("team.explorer:"),
        "roster line missing: {config}"
    );
    // Exactly one member, and NOT the undeclared sibling on disk: a re-introduced
    // directory scan would make this 2 (and would print `team.hidden:`).
    assert_eq!(
        config.matches("team.").count(),
        1,
        "one declaration is one member: {config}"
    );
    assert!(
        !config.contains("team.hidden:"),
        "`hidden.md` is on disk but undeclared: {config}"
    );
}

#[test]
fn an_unreferenced_same_named_md_is_not_a_member() {
    // No fold, so nothing "beats" anything: the inline member renders because it
    // is declared, and the same-named `.md` is simply not a member.
    let dir = project_with_agent_md("[[team]]\nname = \"explorer\"\nrole = \"from-toml\"\n");
    let prompt = dump_prompt(&dir, &[]);
    // Positive: the DECLARED member renders, with its inline role.
    assert!(prompt.contains("- explorer — from-toml"), "{prompt}");
    // The `.md` body is nowhere — being on disk is not membership.
    assert!(!prompt.contains("recon body"), "{prompt}");
    // Positive count on the other surface: one declaration, one member — and the
    // undeclared `hidden.md` never appears (a scan would add it).
    let config = dump_config(&dir, &[]);
    assert_eq!(
        config.matches("team.").count(),
        1,
        "exactly one declared member: {config}"
    );
    assert!(
        !config.contains("team.hidden:"),
        "an unreferenced `.wcode/agents/*.md` is not a member: {config}"
    );
}

/// The escape hatch: a named team pulls a project role in via `file =`
/// (the `full-stack` shape), which resolves regardless of the suppressed scan.
#[test]
fn a_named_team_still_resolves_a_file_member() {
    let dir = project_with_agent_md("");
    std::fs::create_dir_all(dir.path().join(".wcode/teams")).unwrap();
    std::fs::write(
        dir.path().join(".wcode/teams/pull.toml"),
        "[[team]]\nfile = \".wcode/agents/explorer.md\"\n",
    )
    .unwrap();

    let stdout = dump_prompt(&dir, &["--team", "pull"]);
    assert!(
        stdout.contains("- explorer — recon body"),
        "the file member must resolve into the named team: {stdout}"
    );
    assert!(
        !stdout.contains("- scout"),
        "the helper writes no scout; sanity that only file roles are present: {stdout}"
    );
}

/// A temp `$HOME` whose GLOBAL `~/.config/wcode/config.toml` carries one
/// `[[team]]` member (layer 1 — never gated by `--no-project-config`).
fn global_team_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".config/wcode");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("config.toml"),
        "[[team]]\nname = \"explorer\"\nrole = \"global recon\"\n",
    )
    .unwrap();
    home
}

#[test]
fn no_project_config_keeps_the_global_team_and_auto_enables_agents() {
    let home = global_team_home();
    let cwd = tempfile::tempdir().unwrap(); // no project config at all

    // Observable (a): the GLOBAL team still loads under the opt-out — it renders.
    let prompt = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .args(["--no-project-config", "--dump-system-prompt"])
        .output()
        .expect("spawn the wcode binary");
    let stdout = String::from_utf8_lossy(&prompt.stdout);
    assert!(
        stdout.contains("# Your team"),
        "the global [team] must survive --no-project-config: {stdout}"
    );
    assert!(stdout.contains("- explorer — global recon"), "{stdout}");

    // Observable (b): it still auto-enables agents, so the `[team] requires
    // --agents` guard does NOT fire. A control run with a team-less global config
    // would exit 2 — here the run proceeds to the (refused) provider.
    let run = Command::new(env!("CARGO_BIN_EXE_wcode"))
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .env("WCODE_RETRY_MAX", "0")
        .env_remove("WCODE_PROJECT_CONFIG")
        .env_remove("WCODE_CONFIG")
        .args([
            "--no-project-config",
            "--no-session",
            "-p",
            "hi",
            "--model",
            "test",
            "--base-url",
            "http://127.0.0.1:9/v1",
        ])
        .output()
        .expect("spawn the wcode binary");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_ne!(
        run.status.code(),
        Some(2),
        "the [team]-requires--agents guard fired: {stderr}"
    );
    assert!(
        !stderr.contains("requires --agents"),
        "the global [team] must auto-enable agents: {stderr}"
    );
}
