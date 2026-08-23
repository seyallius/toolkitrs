//! module tui - Thin command wrapper that hands off to the ratatui application.
//! Kept separate from the tui module so `commands/` stays the single dispatch surface.

use crate::tui;
use anyhow::Result;
use std::path::PathBuf;

/// Launches the interactive terminal UI.
///
/// # Arguments
/// * `ffmpeg_path` - Optional explicit ffmpeg binary path (from `--ffmpeg-path`).
///
/// # Errors
/// Propagates any terminal setup/teardown error.
pub fn run(ffmpeg_path: Option<PathBuf>) -> Result<()> {
    tui::run(ffmpeg_path)
}
