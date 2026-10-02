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
}
