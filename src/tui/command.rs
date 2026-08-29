//! module command - Trait abstraction for TUI-executable commands.
//!
//! Decouples the TUI from specific workflow domains (media, GitHub, etc.).
//! Each command implements this trait to declare its capabilities and
//! execution logic. The TUI becomes a generic shell that drives any
//! `TuiCommand` implementation.

use crate::tui::{app::RunOptions, event::AppEvent};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------- Types ----------------------------------------- //

/// Declares whether a command needs the file-picker screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePickerMode {
    /// Command needs the user to select input files (e.g., media workflows).
    Required {
        /// Extension to filter by (e.g., "ts", "mkv").
        extension: &'static str,
        /// Optional stem suffix to exclude from discovery.
        exclude_stem_suffix: Option<&'static str>,
    },
    /// Command does not need file selection (e.g., gh-contrib fetches from API).
    NotNeeded,
}

/// A single configurable option shown on the options screen.
#[derive(Debug, Clone)]
pub enum CommandOption {
    /// Boolean toggle (e.g., force overwrite).
    Toggle { label: &'static str, default: bool },
    /// Numeric stepper (e.g., bitrate, cover size).
    Numeric {
        label: &'static str,
        default: i32,
        step: i32,
        min: i32,
        max: i32,
        unit: &'static str,
    },
    /// Text input (e.g., username, date).
    Text {
        label: &'static str,
        default: &'static str,
        placeholder: &'static str,
    },
}

/// The interface every TUI-executable command must implement.
///
/// # Design rationale
///
/// The TUI is a state machine that renders screens and dispatches actions.
/// It should not know whether it's driving ffmpeg, curl, or a database query.
/// This trait is the seam that makes that possible.
pub trait TuiCommand: std::fmt::Debug + Send + Sync {
    // -------- Identity --------

    /// Unique identifier for this command.
    fn id(&self) -> &'static str;
    /// Short title shown in the home list (e.g., "ts2mp4", "gh-contrib").
    fn title(&self) -> &'static str;
    /// One-line description shown next to the title.
    fn description(&self) -> &'static str;

    // -------- Capabilities --------

    /// Whether and how the file picker is used.
    fn file_picker_mode(&self) -> FilePickerMode;
    /// The configurable options for this command.
    ///
    /// Return an empty vec if the command has no user-configurable options.
    fn options(&self) -> Vec<CommandOption>;

    // -------- Execution --------

    /// Runs the command asynchronously.
    ///
    /// # Arguments
    /// * `files` - Selected input files (empty if `file_picker_mode` is `NotNeeded`).
    /// * `options` - Resolved run options from the options screen.
    /// * `cancel` - Cancellation token for graceful shutdown.
    /// * `ffmpeg_path` - Path to ffmpeg binary (for media commands; ignored otherwise).
    /// * `tx` - Commands can send logs/events to the TUI.
    ///
    /// # Returns
    /// The output path(s) or a completion signal.
    fn execute(
        &self,
        files: Vec<PathBuf>,
        options: &RunOptions,
        cancel: CancellationToken,
        ffmpeg_path: &std::path::Path,
        tx: std::sync::mpsc::Sender<AppEvent>,
    ) -> anyhow::Result<()>;
}
