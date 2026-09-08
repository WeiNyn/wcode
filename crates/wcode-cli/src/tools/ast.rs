//! Shared discovery / output plumbing for the ast-grep-backed structural tools
//! (`ast_search`, `ast_edit`). Mirrors wcode's existing "auto if binary on
//! PATH" pattern (rtk): a tool is only registered when one of the binaries is
//! present, so the model never sees a tool it can't use. `ast-grep` ships via
//! `cargo install ast-grep` / `npm i -g @ast-grep/cli` / `brew install
//! ast-grep`; `sg` is its legacy alias.

const BINS: &[&str] = &["ast-grep", "sg"];

/// The modern `ast-grep` binary is preferred (its `sg` alias prints a
/// deprecation banner on stderr); `sg` is the fallback. The two tools share
/// this so registration never diverges (one registered, the other not).
pub fn find_bin() -> Option<&'static str> {
    let paths = std::env::var_os("PATH")?;
    BINS.iter()
        .find(|name| {
            std::env::split_paths(&paths)
                .filter(|p| !p.as_os_str().is_empty())
                .any(|p| p.join(*name).is_file())
        })
        .copied()
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
