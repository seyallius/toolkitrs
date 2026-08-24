# AGENTS.md

## Project overview

`toolkitrs` is a Rust 2021 command-line application for small, dependable
FFmpeg media workflows. The maintained application is the Cargo crate at the
repository root. It currently provides the `ts2mp4`, `mkv2mp3`, `mp32mp4`, and
`vidwrap` subcommands.

The `bck/` directory contains archived PowerShell and Go implementations.
Treat it as historical reference only; do not change it unless the task
explicitly concerns a legacy implementation. `bash/` contains standalone shell
utilities and is not part of the Rust CLI build.

## Prerequisites

- Rust toolchain with the 2021 edition support (use `cargo`).
- `ffmpeg` on `PATH` to run actual media conversions, or pass
  `--ffmpeg-path <path>` to the CLI.
- `just` is optional and supplies developer shortcuts.
- `cross` is only required for the Windows cross-build recipe.

## Build and run

Run commands from the repository root.

```bash
cargo build
cargo build --release
cargo run -- --help
cargo run -- ts2mp4 --help
just clippy
just build-windows-cross # requires cross
```

Do not run conversions against user media as a routine validation step:
commands create output files, and `vidwrap` can offer destructive cleanup
choices. Prefer `--help`, unit tests, and fake process runners for automated
verification.

## Validation

Before handing off a Rust change, run the checks relevant to it:

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Use `cargo test -- <name>` for a focused iteration, but run the full suite for
changes that affect shared code, CLI parsing, FFmpeg arguments, or file/output
handling. Integration tests live in `tests/cli.rs`; module unit tests should
sit in a nearby `#[cfg(test)] mod tests` block.

At the time this file was added, `cargo test` has two pre-existing failing
tests: `util::output::tests::names_output_and_skips` and
`util::files::tests::finds_priority_and_special_names`. Do not report a clean
test suite unless these failures are fixed or otherwise accounted for. The
current build also emits existing `dead_code` warnings, so the strict Clippy
command may require cleanup before it can pass.

## Code layout and extension points

- `src/main.rs`: parses CLI arguments and dispatches execution.
- `src/cli.rs`: Clap command, global options, and shared `BatchArgs`.
- `src/commands/`: subcommand handlers. Batch-style converters implement
  `commands::batch::BatchTask` and use `run_batch` for discovery, overwriting,
  summaries, and failure reporting.
- `src/ffmpeg/args.rs`: pure FFmpeg argument-vector builders. Keep FFmpeg flag
  construction here rather than embedding it in command handlers.
- `src/ffmpeg/runner.rs`: execution seam. Keep `ProcessRunner` generic in
  command code so tests can inject a fake runner; use `RealRunner` only at the
  application boundary in `commands::run`.
- `src/util/`: filesystem discovery and output-path decisions.
- `src/components/`: terminal UI helpers. Interactive code must remain
  testable and should avoid assuming stderr/stdout is a terminal.

To add a subcommand, create its module under `src/commands/`, define Clap
arguments, add a `Command` variant in `src/cli.rs`, export and dispatch it in
`src/commands/mod.rs`, then add CLI and behavior tests. Reuse `BatchArgs` and
`BatchTask` when the command processes multiple files.

## Conventions

- Follow `rustfmt`; use idiomatic Rust 2021 and let Clippy guide choices.
- Document modules, public APIs, command-line options, constants, and
  user-visible behavior with `///`/`//!` comments, matching the existing
  style.
- Keep command handlers thin. Put reusable filesystem logic in `util`, FFmpeg
  invocations in `ffmpeg::args`, and batch control flow in `commands::batch`.
- Build paths with `Path`/`PathBuf`, never by string concatenation. Preserve
  case-insensitive extension matching and sorted discovery behavior.
- Return `anyhow::Result` with context; do not panic in production paths.
  `unwrap`/`expect` are acceptable only in tests or when an invariant is
  genuinely guaranteed and documented.
- Preserve safe defaults: create output directories, skip existing output
  unless `--force` is supplied, and make destructive operations explicit.
- Keep normal progress on stdout and diagnostics/warnings on stderr. Respect
  the global `--verbose` and `--no-color` options.
- FFmpeg argument builders return `Vec<String>` and must keep input/output
  paths as individual arguments. Add assertions for important flags and maps
  when changing them.
- Clean up temporary files after successful workflows. If a multi-step
  workflow fails, retain useful temporary artifacts only when that matches the
  current command's documented debugging behavior.

## Repository hygiene

Inspect `git status --short` before editing and preserve unrelated changes.
Do not edit `Cargo.lock` by hand. Do not commit, push, rebase, reset, or change
dependencies unless the task explicitly asks for it. Keep generated artifacts
out of version control (`target/` and media outputs are ignored). Update
`README.md` when user-facing commands, options, or supported workflows change.
