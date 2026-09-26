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
