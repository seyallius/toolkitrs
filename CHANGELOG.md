# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.9](https://github.com/seyallius/toolkitrs/compare/v0.1.8...v0.1.9) - 2026-08-23

### Added

- *(tui)* add ratatui-based interactive terminal UI

### Fixed

- *(tui)* implement list scrolling with ratatui ListState

## [0.1.8](https://github.com/seyallius/toolkitrs/compare/v0.1.7...v0.1.8) - 2026-08-22

### Added

- *(batch)* unify interactive discovery and error policies
- *(vidwrap)* add interactive directory batch processing

### Other

- *(cli)* enhance help text and README with batch features

## [0.1.7](https://github.com/seyallius/toolkitrs/compare/v0.1.6...v0.1.7) - 2026-08-21

### Fixed

- *(mp32mp4)* handle cover extraction failures with cleanup

### Other

- *(ffmpeg)* reorder FfmpegArgs builder above public API functions
- *(ffmpeg)* replace macro with fluent builder
- *(util)* generate tmpfile paths without empty files on disk

## [0.1.6](https://github.com/seyallius/toolkitrs/compare/v0.1.5...v0.1.6) - 2026-08-19

### Added

- *(ui)* add TTY-aware banners and CI/CD-safe prompts
- *(vidwrap)* adapt success output to terminal state

## [0.1.5](https://github.com/seyallius/toolkitrs/compare/v0.1.4...v0.1.5) - 2026-08-18

### Other

- Add 'bash/*' to exclude list in Cargo.toml

## [0.1.4](https://github.com/seyallius/toolkitrs/compare/v0.1.3...v0.1.4) - 2026-08-18

### Fixed

- *(vidwrap)* simplify to single FFmpeg invocation for static image video
- *(vidwrap)* correct FFmpeg mapping flags to match working Go version

### Other

- *(vidwrap)* configurable video generation with functional options
- *(ffmpeg)* introduce args! macro to reduce Vec<String> boilerplate

## [0.1.3](https://github.com/seyallius/toolkitrs/compare/v0.1.2...v0.1.3) - 2026-08-17

### Other

- adopt standard Rust target triple naming for cargo-binstall

## [0.1.2](https://github.com/seyallius/toolkitrs/compare/v0.1.1...v0.1.2) - 2026-08-17

### Other

- *(license)* update README to reflect dual MIT/Apache-2.0 license

## [0.1.1](https://github.com/seyallius/toolkitrs/compare/v0.1.0...v0.1.1) - 2026-08-17

### Added

- *(ui)* add extensive spinner styles and tune animation speed
- *(toolkitrs)* add Rust FFmpeg CLI toolkitrs for media workflows
- *(vidwrap)* add tool for embedding images as video thumbnails
- *(scripts)* add bash train monitor and test notification scripts
- *(ffmpeg)* add MP3-to-MP4 converter with cover art extraction
- *(mkv2mp3)* add PowerShell script and batch wrapper for MKV to MP3 conversion
- *(conv)* add TS to MP4 batch converter script

### Fixed

- *(ffmpeg)* encode_loop argument order for correct image looping (wip)
- *(justfile)* include untracked files in diff-cp recipes
- *(justfile)* include staged changes in diff-cp commands

### Other

- rename package to toolkitrs and prepare for crates.io publication
- *(readme)* rewrite README to document unified Rust FFmpeg CLI
- add agent contribution guide
- enhance doc comments and rename spinner stop field for clarity
- *(backup)* archive legacy scripts and Go vidwrap implementation
- *(commands)* extract generic batch processor to eliminate DRY
- *(toolkitrs)* add module-level documentation and refactor constants
- *(vidwrap)* add comprehensive table-driven tests for all packages
- *(idea)* add JetBrains IDE workspace configuration and format README table
- add README with project overview and usage guide
- *(git)* add .gitignore and extend .treeclipignore
- add .treeclipignore with default ignore patterns
- *(ffmpeg)* reorganize conversion scripts into powershell/ffmpeg directory
- Add basic justfile
