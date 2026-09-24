//! Shared discovery / output plumbing for the ast-grep-backed structural tools
//! (`ast_search`, `ast_edit`). Mirrors wcode's existing "auto if binary on
//! PATH" pattern (rtk): a tool is only registered when one of the binaries is
//! present, so the model never sees a tool it can't use. `ast-grep` ships via
//! `cargo install ast-grep` / `npm i -g @ast-grep/cli` / `brew install
//! ast-grep`; `sg` is its legacy alias.

use std::ffi::OsStr;

const BINS: &[&str] = &["ast-grep", "sg"];

/// The modern `ast-grep` binary is preferred (its `sg` alias prints a
/// deprecation banner on stderr); `sg` is the fallback. The two tools share
/// this so registration never diverges (one registered, the other not).
///
/// A name on `PATH` is not enough on its own: on Linux `/usr/bin/sg` is the
/// unrelated "switch group" utility, and picking it would register a tool
/// whose every call fails. So a candidate must also identify itself as
/// ast-grep ([`is_ast_grep`]).
pub fn find_bin() -> Option<&'static str> {
    let paths = std::env::var_os("PATH")?;
    BINS.iter()
        .copied()
        .find(|name| on_path(&paths, name) && is_ast_grep(name))
}

/// Whether `name` resolves to a file on `PATH` (empty entries are skipped).
fn on_path(paths: &OsStr, name: &str) -> bool {
    std::env::split_paths(paths)
        .filter(|p| !p.as_os_str().is_empty())
        .any(|p| p.join(name).is_file())
}

/// Probe `name --version` and confirm the output names ast-grep. Both
/// `ast-grep` and its `sg` alias report `ast-grep <version>` (the alias also
/// prints a deprecation banner), whereas an unrelated `sg` does not.
fn is_ast_grep(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map(|out| {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            is_ast_grep_version(&text)
        })
        .unwrap_or(false)
}

/// Whether `--version` output is ast-grep's. Split out so it is testable.
fn is_ast_grep_version(text: &str) -> bool {
    text.contains("ast-grep")
}

/// Whether either ast-grep binary is on PATH. If false, the tools are simply
/// not registered (see `tools::default_tools`).
pub fn available() -> bool {
    find_bin().is_some()
}

/// The `sg` alias's deprecation banner pollutes stderr; drop it (and its
/// separators) so real errors are unambiguous.
pub fn clean_stderr(s: &str) -> String {
    s.lines()
        .filter(|l| !(l.starts_with("=====") || l.starts_with("WARNING:")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_probe_accepts_ast_grep_and_its_alias() {
        assert!(is_ast_grep_version("ast-grep 0.45.3\n"));
        // The `sg` alias prints a deprecation banner, then the same version line.
        assert!(is_ast_grep_version(
            "=====\nWARNING: `sg` is deprecated. Use `ast-grep` instead.\n=====\nast-grep 0.45.3\n"
        ));
    }

    #[test]
    fn version_probe_rejects_the_linux_switch_group_sg() {
        // `/usr/bin/sg` (util-linux/shadow) shares the name but is not ast-grep.
        assert!(!is_ast_grep_version("Usage: sg [-] [-c] command\n"));
        assert!(!is_ast_grep_version("sg: invalid option -- '-'\n"));
    }
}
