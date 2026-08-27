//! module workers - Async, cancellable per-file conversion functions.
//!
//! Each function takes an input file, output path, ffmpeg binary path, and a
//! [`CancellationToken`]. They build the appropriate ffmpeg arguments via
//! [`crate::ffmpeg::args`] and run them via [`crate::ffmpeg::runner::run_async`].
//!
//! The `*_job` constructors wrap these functions in [`FileJob`]
//! implementations, which is what the parallel executor and the TUI use to
//! run any workflow uniformly — output naming, skip/overwrite checks, and
//! dispatch live in one place per workflow.

use crate::{
    ffmpeg::{args, runner::run_async},
    util::{
        files, output,
        parallel::{FileJob, WorkerFuture},
    },
};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Prefix for temporary cover image files.
pub const TEMP_COVER_PREFIX: &str = "toolkitrs-cover-";

/// Suffix for temporary cover image files.
pub const TEMP_COVER_SUFFIX: &str = ".jpg";

/// Extension for MP3 files.
const MP3_EXT: &str = "mp3";

/// Extension for MP4 files.
const MP4_EXT: &str = "mp4";

// ------------------------------------------ Types & Impls ------------------------------------- //

/// [`FileJob`] for the TS → MP4 remux workflow.
struct Ts2Mp4Job {
    /// Directory that receives the converted files.
    output_dir: PathBuf,
    /// Whether to overwrite existing outputs.
    force: bool,
    /// Path to the ffmpeg executable.
    binary: PathBuf,
}
impl FileJob for Ts2Mp4Job {
    fn output_path(&self, input: &Path) -> Option<PathBuf> {
        Some(
            output::output_path(input, &self.output_dir, MP4_EXT)
                .unwrap_or_else(|_| input.with_extension(MP4_EXT)),
        )
    }

    fn force(&self) -> bool {
        self.force
    }

    fn run(&self, input: PathBuf, output: PathBuf, cancel: CancellationToken) -> WorkerFuture {
        let binary = self.binary.clone();
        let force = self.force;
        Box::pin(async move { ts2mp4(input, output, force, &binary, cancel).await })
    }
}

/// [`FileJob`] for the MKV → MP3 audio extraction workflow.
struct Mkv2Mp3Job {
    /// Directory that receives the converted files.
    output_dir: PathBuf,
    /// Audio bitrate in kbps.
    bitrate: u32,
    /// Cover art square size in pixels.
    cover_size: u32,
    /// Whether to overwrite existing outputs.
    force: bool,
    /// Path to the ffmpeg executable.
    binary: PathBuf,
}
impl FileJob for Mkv2Mp3Job {
    fn output_path(&self, input: &Path) -> Option<PathBuf> {
        Some(
            output::output_path(input, &self.output_dir, MP3_EXT)
                .unwrap_or_else(|_| input.with_extension(MP3_EXT)),
        )
    }

    fn force(&self) -> bool {
        self.force
    }

    fn run(&self, input: PathBuf, output: PathBuf, cancel: CancellationToken) -> WorkerFuture {
        let binary = self.binary.clone();
        let (bitrate, cover_size, force) = (self.bitrate, self.cover_size, self.force);
        Box::pin(async move {
            mkv2mp3(input, output, bitrate, cover_size, force, &binary, cancel).await
        })
    }
}

/// [`FileJob`] for the MP3 → MP4 video workflow.
struct Mp3ToMp4Job {
    /// Directory that receives the converted files.
    output_dir: PathBuf,
    /// Audio bitrate in kbps.
    bitrate: u32,
    /// Whether to overwrite existing outputs.
    force: bool,
    /// Whether to skip files without embedded cover art.
    no_cover_fallback: bool,
    /// Path to the ffmpeg executable.
    binary: PathBuf,
}
impl FileJob for Mp3ToMp4Job {
    fn output_path(&self, input: &Path) -> Option<PathBuf> {
        Some(
            output::output_path(input, &self.output_dir, MP4_EXT)
                .unwrap_or_else(|_| input.with_extension(MP4_EXT)),
        )
    }

    fn force(&self) -> bool {
        self.force
    }

    fn run(&self, input: PathBuf, output: PathBuf, cancel: CancellationToken) -> WorkerFuture {
        let binary = self.binary.clone();
        let (bitrate, force, no_cover_fallback) =
            (self.bitrate, self.force, self.no_cover_fallback);
        Box::pin(async move {
            mp32mp4(
                input,
                output,
                bitrate,
                force,
                no_cover_fallback,
                &binary,
                cancel,
            )
            .await
        })
    }
}

/// [`FileJob`] for the vidwrap workflow, which writes next to its input.
struct VidwrapJob {
    /// Whether to overwrite existing outputs.
    force: bool,
    /// Path to the ffmpeg executable.
    binary: PathBuf,
}
impl FileJob for VidwrapJob {
    fn output_path(&self, input: &Path) -> Option<PathBuf> {
        files::output_path_for_video_with_image(input).ok()
    }

    fn force(&self) -> bool {
        self.force
    }

    fn run(&self, input: PathBuf, output: PathBuf, cancel: CancellationToken) -> WorkerFuture {
        let binary = self.binary.clone();
        Box::pin(async move { vidwrap(input, output, &binary, cancel).await })
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Creates the parallel job for TS → MP4 remuxing.
pub fn ts2mp4_job(output_dir: &Path, force: bool, binary: &Path) -> Arc<dyn FileJob> {
    Arc::new(Ts2Mp4Job {
        output_dir: output_dir.to_path_buf(),
        force,
        binary: binary.to_path_buf(),
    })
}

/// Creates the parallel job for MKV → MP3 extraction.
pub fn mkv2mp3_job(
    output_dir: &Path,
    bitrate: u32,
    cover_size: u32,
    force: bool,
    binary: &Path,
) -> Arc<dyn FileJob> {
    Arc::new(Mkv2Mp3Job {
        output_dir: output_dir.to_path_buf(),
        bitrate,
        cover_size,
        force,
        binary: binary.to_path_buf(),
    })
}

/// Creates the parallel job for the vidwrap workflow.
///
/// The CLI always passes `force = true` (vidwrap's ffmpeg arguments
/// overwrite unconditionally); the TUI passes its user-selected setting.
pub fn vidwrap_job(force: bool, binary: &Path) -> Arc<dyn FileJob> {
    Arc::new(VidwrapJob {
        force,
        binary: binary.to_path_buf(),
    })
}

/// Creates the parallel job for MP3 → MP4 conversion.
pub fn mp32mp4_job(
    output_dir: &Path,
    bitrate: u32,
    force: bool,
    no_cover_fallback: bool,
    binary: &Path,
) -> Arc<dyn FileJob> {
    Arc::new(Mp3ToMp4Job {
        output_dir: output_dir.to_path_buf(),
        bitrate,
        force,
        no_cover_fallback,
        binary: binary.to_path_buf(),
    })
}

/// Remuxes a TS file to MP4 (stream copy, no re-encoding).
pub async fn ts2mp4(
    input: PathBuf,
    output: PathBuf,
    force: bool,
    ffmpeg: &Path,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    run_async(ffmpeg, args::remux_copy(&input, &output, force), cancel).await?;
    Ok(output)
}

/// Extracts MP3 audio from an MKV file, optionally with cover art.
pub async fn mkv2mp3(
    input: PathBuf,
    output: PathBuf,
    bitrate: u32,
    cover_size: u32,
    force: bool,
    ffmpeg: &Path,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    let cover = files::temp_path(TEMP_COVER_PREFIX, TEMP_COVER_SUFFIX)?;

    // Step 1 (non-fatal): extract a frame for cover art.
    let has_cover = run_async(
        ffmpeg,
        args::extract_frame(&input, &cover, cover_size),
        cancel.clone(),
    )
    .await
    .is_ok()
        && cover.metadata().map(|m| m.len() > 0).unwrap_or(false);

    if !has_cover {
        eprintln!("WARNING: cover extraction failed for {}", input.display());
    }

    // Step 2: encode the MP3, attaching the cover if present.
    let result = run_async(
        ffmpeg,
        args::encode_mp3(&input, has_cover.then_some(&cover), &output, bitrate, force),
        cancel,
    )
    .await;

    // Always clean up the temp cover file, success or failure.
    let _ = fs::remove_file(&cover);
    result?;
    Ok(output)
}

/// Converts an MP3 to an MP4 video with optional embedded cover art.
pub async fn mp32mp4(
    input: PathBuf,
    output: PathBuf,
    bitrate: u32,
    force: bool,
    no_cover_fallback: bool,
    ffmpeg: &Path,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    let cover = files::temp_path(TEMP_COVER_PREFIX, TEMP_COVER_SUFFIX)?;

    // Step 1 (non-fatal): try to extract embedded cover art.
    let has_cover = match run_async(
        ffmpeg,
        args::extract_embedded_cover(&input, &cover),
        cancel.clone(),
    )
    .await
    {
        Ok(_) => cover.metadata().map(|m| m.len() > 0).unwrap_or(false),
        Err(e) => {
            eprintln!(
                "WARNING: failed to extract cover from {}: {e}",
                input.display()
            );
            let _ = fs::remove_file(&cover);
            false
        }
    };

    if !has_cover && no_cover_fallback {
        let _ = fs::remove_file(&cover);
        return Ok(output); // skip silently, treat as success
    }

    // Step 2: encode the MP4.
    let result = run_async(
        ffmpeg,
        args::encode_mp4(has_cover.then_some(&cover), &input, &output, bitrate, force),
        cancel,
    )
    .await;

    let _ = fs::remove_file(&cover);
    result?;
    Ok(output)
}

/// Wraps a video with a companion image, producing a new MP4 next to it.
pub async fn vidwrap(
    input: PathBuf,
    output: PathBuf,
    ffmpeg: &Path,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    let image = files::companion_image(&input)?;

    run_async(
        ffmpeg,
        args::replace_video_with_image(&image, &input, &output, &[]),
        cancel,
    )
    .await?;

    Ok(output)
}
