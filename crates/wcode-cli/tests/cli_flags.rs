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
    // D-B1: a non-empty folded `[team]` turns agents mode on, so the
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

/// A temp cwd shipping an agent `.md` (and a temp `$HOME`). `toml`, when
/// non-empty, is written to `.wcode/team.toml` so a TOML/`.md` collision can be
/// exercised.
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
    if !toml.is_empty() {
        std::fs::write(wcode.join("team.toml"), toml).unwrap();
    }
    dir
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
fn an_agent_md_is_discovered_into_the_team() {
    let dir = project_with_agent_md("");
    let stdout = dump_prompt(&dir, &[]);
    assert!(stdout.contains("# Your team"), "{stdout}");
    // Body → role (D-C4), rendered on the roster line.
    assert!(stdout.contains("- explorer — recon body"), "{stdout}");
}

#[test]
fn a_toml_team_member_beats_a_same_named_agent_md() {
    let dir = project_with_agent_md("[[team]]\nname = \"explorer\"\nrole = \"from-toml\"\n");
    let stdout = dump_prompt(&dir, &[]);
    assert!(stdout.contains("from-toml"), "the TOML member must win: {stdout}");
    assert!(!stdout.contains("recon body"), "the same-named .md must lose: {stdout}");
}

#[test]
fn no_project_config_disables_agent_md_discovery() {
    let dir = project_with_agent_md("");
    let stdout = dump_prompt(&dir, &["--no-project-config"]);
    assert!(!stdout.contains("# Your team"), "{stdout}");
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
