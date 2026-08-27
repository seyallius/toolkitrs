//! module workflow - Shared workflow metadata and async execution helpers.
//!
//! This module is the domain-level home for everything that is true about a
//! workflow regardless of UI:
//! - static metadata (titles, extensions, capabilities)
//! - output path resolution
//! - async execution wiring around ffmpeg
//!
//! Keeping that logic here avoids duplicating the same workflow mapping across
//! CLI commands and the TUI runner.

use crate::{
    ffmpeg::{args, runner::run_async as run_ffmpeg_async},
    util::{cancel::Cancelled, files, output},
};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

// ------------------------------------------ Types & Impls ------------------------------------- //

/// The media workflows available to both the CLI and the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workflow {
    /// Remux TS → MP4 (stream copy).
    Ts2Mp4,
    /// Extract audio MKV → MP3.
    Mkv2Mp3,
    /// Create video MP3 → MP4.
    Mp32Mp4,
    /// Wrap a video with its companion image.
    Vidwrap,
}
impl Workflow {
    /// Returns every workflow in display order.
    pub fn all() -> Vec<Self> {
        vec![Self::Ts2Mp4, Self::Mkv2Mp3, Self::Mp32Mp4, Self::Vidwrap]
    }

    /// Short title shown in the home list.
    pub fn title(self) -> &'static str {
        match self {
            Self::Ts2Mp4 => "ts2mp4",
            Self::Mkv2Mp3 => "mkv2mp3",
            Self::Mp32Mp4 => "mp32mp4",
            Self::Vidwrap => "vidwrap",
        }
    }

    /// One-line description shown next to the title.
    pub fn description(self) -> &'static str {
        match self {
            Self::Ts2Mp4 => "Remux TS to MP4 (no re-encode)",
            Self::Mkv2Mp3 => "Extract audio from MKV to MP3",
            Self::Mp32Mp4 => "Turn MP3 into MP4 with cover art",
            Self::Vidwrap => "Wrap video with a companion image",
        }
    }

    /// Human-readable workflow name used in CLI banners.
    pub fn file_type_name(self) -> &'static str {
        match self {
            Self::Ts2Mp4 => "TS",
            Self::Mkv2Mp3 => "MKV",
            Self::Mp32Mp4 => "MP3",
            Self::Vidwrap => "Vidwrap",
        }
    }

    /// File extension the picker should show for this workflow.
    pub fn input_extension(self) -> &'static str {
        match self {
            Self::Ts2Mp4 => "ts",
            Self::Mkv2Mp3 => "mkv",
            Self::Mp32Mp4 => "mp3",
            Self::Vidwrap => "mp4",
        }
    }

    /// File extension produced by this workflow.
    pub fn output_extension(self) -> &'static str {
        match self {
            Self::Ts2Mp4 | Self::Mp32Mp4 | Self::Vidwrap => "mp4",
            Self::Mkv2Mp3 => "mp3",
        }
    }

    /// Whether this workflow uses audio encoding options (bitrate).
    pub fn uses_bitrate(self) -> bool {
        matches!(self, Self::Mkv2Mp3 | Self::Mp32Mp4)
    }

    /// Whether this workflow extracts cover art (needs cover size).
    pub fn uses_cover_size(self) -> bool {
        matches!(self, Self::Mkv2Mp3 | Self::Mp32Mp4)
    }

    /// Whether this workflow writes into a separate output directory.
    pub fn uses_output_dir(self) -> bool {
        !matches!(self, Self::Vidwrap)
    }

    /// Computes the workflow-specific output path for one input file.
    pub fn output_path(self, input: &Path, options: &WorkflowOptions) -> Result<PathBuf> {
        match self {
            Self::Vidwrap => files::output_path_for_video_with_image(input),
            _ => output::output_path(input, &options.output_dir, self.output_extension()),
        }
    }

    /// Executes one input file asynchronously using the workflow's ffmpeg sequence.
    pub async fn run_async(
        self,
        input: PathBuf,
        output: PathBuf,
        options: &WorkflowOptions,
        ffmpeg_binary: &Path,
        cancel: CancellationToken,
        log_tx: Option<UnboundedSender<String>>,
    ) -> Result<PathBuf> {
        match self {
            Self::Ts2Mp4 => {
                run_ffmpeg_async(
                    ffmpeg_binary,
                    args::remux_copy(&input, &output, options.force),
                    cancel,
                    log_tx,
                )
                .await?;
                Ok(output)
            }
            Self::Mkv2Mp3 => {
                run_mkv2mp3(input, output, options, ffmpeg_binary, cancel, log_tx).await
            }
            Self::Mp32Mp4 => {
                run_mp32mp4(input, output, options, ffmpeg_binary, cancel, log_tx).await
            }
            Self::Vidwrap => {
                let image = files::companion_image(&input)?;
                run_ffmpeg_async(
                    ffmpeg_binary,
                    args::replace_video_with_image(&image, &input, &output, &[]),
                    cancel,
                    log_tx,
                )
                .await?;
                Ok(output)
            }
        }
    }
}

/// Shared execution settings for workflow runs.
///
/// Some fields are only meaningful for specific workflows; unused fields are
/// simply ignored by workflows that do not need them.
#[derive(Debug, Clone)]
pub struct WorkflowOptions {
    /// Directory where outputs are written for workflows that use one.
    pub output_dir: PathBuf,
    /// Whether existing outputs should be overwritten.
    pub force: bool,
    /// Audio bitrate used by audio/video encoding workflows.
    pub bitrate: u32,
    /// Cover-art size used when extracting a frame from video.
    pub cover_size: u32,
    /// Whether MP3 → MP4 should skip files that have no embedded cover art.
    pub no_cover_fallback: bool,
}
impl Default for WorkflowOptions {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("out"),
            force: false,
            bitrate: 320,
            cover_size: 600,
            no_cover_fallback: false,
        }
    }
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Extracts MP3 audio from an MKV file, optionally with cover art.
async fn run_mkv2mp3(
    input: PathBuf,
    output: PathBuf,
    options: &WorkflowOptions,
    ffmpeg_binary: &Path,
    cancel: CancellationToken,
    log_tx: Option<UnboundedSender<String>>,
) -> Result<PathBuf> {
    let cover = files::temp_path("toolkitrs-cover-", ".jpg")?;

    // Step 1 (non-fatal): extract a frame for cover art.
    let has_cover = run_ffmpeg_async(
        ffmpeg_binary,
        args::extract_frame(&input, &cover, options.cover_size),
        cancel.clone(),
        log_tx.clone(),
    )
    .await
    .is_ok()
        && has_file_content(&cover);

    cancel_if_requested(&cancel)?;

    if !has_cover {
        eprintln!("WARNING: cover extraction failed for {}", input.display());
    }

    // Step 2: encode the MP3, attaching the cover if present.
    let result = run_ffmpeg_async(
        ffmpeg_binary,
        args::encode_mp3(
            &input,
            has_cover.then_some(cover.as_path()),
            &output,
            options.bitrate,
            options.force,
        ),
        cancel,
        log_tx,
    )
    .await;

    cleanup_temp_file(&cover);
    result?;
    Ok(output)
}

/// Converts an MP3 to an MP4 video with optional embedded cover art.
async fn run_mp32mp4(
    input: PathBuf,
    output: PathBuf,
    options: &WorkflowOptions,
    ffmpeg_binary: &Path,
    cancel: CancellationToken,
    log_tx: Option<UnboundedSender<String>>,
) -> Result<PathBuf> {
    let cover = files::temp_path("toolkitrs-cover-", ".jpg")?;

    // Step 1 (non-fatal): try to extract embedded cover art.
    let has_cover = match run_ffmpeg_async(
        ffmpeg_binary,
        args::extract_embedded_cover(&input, &cover),
        cancel.clone(),
        log_tx.clone(),
    )
    .await
    {
        Ok(_) => has_file_content(&cover),
        Err(error) => {
            eprintln!(
                "WARNING: failed to extract cover from {}: {error}",
                input.display()
            );
            cleanup_temp_file(&cover);
            false
        }
    };

    cancel_if_requested(&cancel)?;

    if !has_cover && options.no_cover_fallback {
        cleanup_temp_file(&cover);
        return Ok(output);
    }

    // Step 2: encode the MP4.
    let result = run_ffmpeg_async(
        ffmpeg_binary,
        args::encode_mp4(
            has_cover.then_some(cover.as_path()),
            &input,
            &output,
            options.bitrate,
            options.force,
        ),
        cancel,
        log_tx,
    )
    .await;

    cleanup_temp_file(&cover);
    result?;
    Ok(output)
}

/// Returns true when the file exists and contains data.
fn has_file_content(path: &Path) -> bool {
    path.metadata().map(|meta| meta.len() > 0).unwrap_or(false)
}

/// Best-effort temp file cleanup.
fn cleanup_temp_file(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Returns a typed cancellation error when cancellation has already been requested.
fn cancel_if_requested(cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() {
        Err(Cancelled.into())
    } else {
        Ok(())
    }
}
