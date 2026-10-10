# D016 — `[tui] width`: the TUI's initial transcript measure is configurable

- **Status:** accepted (human sign-off 2026-10-10, this session)
- **Date:** 2026-10-10
- **Relates to:** D011 (the adjustable `measure` + `/width`, `Alt-[`/`Alt-]`);
  D015 (the same "presentation is configurable, behavior is not" split for the
  VS Code surface)

## Context

The TUI's transcript measure (content width, columns) was `DEFAULT_MEASURE = 68`
— a hardcoded `const` in `wcode-tui/src/app.rs`. It could be changed **only at
runtime**, session-scoped, via `/width <cols>` or `Alt-[` / `Alt-]` (step 8,
clamped `40..=200`). Nothing persisted it: a new `wcode` run always began at 68,
and it was absent from `config.toml`, the `WCODE_*` env vars, and the CLI flags.

`[theme]` already set the precedent that a TUI **presentation** knob lives in
`config.toml` (`config.rs`, `parse_theme_table`). The measure is the same kind of
thing.

## Decision

Add **`[tui] width`** — the initial measure in columns:

```toml
[tui]
width = 96   # optional; 40..=200 (default: 68)
```

- **Config only.** No `--width` flag (a durable setting makes a flag redundant and
  the CLI stays lean), and nothing is persisted at runtime — `/width` and
  `Alt-[`/`Alt-]` keep their role as the *session* override.
- **Loud on a bad value.** Out of `MEASURE_MIN..=MEASURE_LIMIT` is a hard
  `ConfigError::Tui` at load, mirroring `[theme]`'s "unparseable overlay fails
  loudly" rule — not a silent clamp.
- **Presentation, not behavior.** This stays inside the D015 line: it changes how
  the surface looks, not what the agent does.

## Mechanism

- `wcode-tui` exposes `TuiSpec { width: Option<usize> }` (serde-free — the crate
  has no serde), `MEASURE_MIN`/`MEASURE_LIMIT`/`DEFAULT_MEASURE`, and
  `validate_measure(cols) -> Result<usize, String>`. `Options` gains `tui: TuiSpec`;
  `run` applies `app.set_measure(cols)` when `Some` (already scope-visible, now
  `pub(crate)`).
- `wcode-cli::config` gains `TuiConfig` (`Deserialize`) on `FileConfig`, resolved
  into `Config.tui` via `validate_measure`, with `ConfigError::Tui`.
- `--dump-config` prints `tui.width` (the value, or `(default 68)`).

## Consequences

- `[tui] width` sets the startup measure; a terminal narrower than
  `MEASURE_MIN_BAND` (84 cols, `ui.rs`) still uses the full band, so the setting
  has no visible effect there.
- Tests: the resolve/reject paths (`config.rs`), and `validate_measure`'s bounds
  (`wcode-tui`). `cargo test --workspace` (1139) + `clippy --all-targets` clean.
