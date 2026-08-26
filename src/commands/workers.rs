//! module workers - Async, cancellable per-file conversion functions.
//!
//! Each function takes an input file, output path, ffmpeg binary path, and a
//! [`CancellationToken`]. They build the appropriate ffmpeg arguments via
//! [`crate::ffmpeg::args`] and run them via [`crate::ffmpeg::runner::run_async`].

use crate::{
    ffmpeg::{args, runner::run_async},
    util::files,
};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

/// Remuxes a TS file to MP4 (stream copy, no re-encoding).
pub async fn ts2mp4(
    input: PathBuf,
    output: PathBuf,
    force: bool,
    ffmpeg: &Path,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    run_async(
        ffmpeg,
        args::remux_copy(&input, &output, force),
        cancel,
        None,
    )
    .await?;
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
    let cover = files::temp_path("toolkitrs-cover-", ".jpg")?;

    // Step 1 (non-fatal): extract a frame for cover art.
    let has_cover = run_async(
        ffmpeg,
        args::extract_frame(&input, &cover, cover_size),
        cancel.clone(),
        None,
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
        None,
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
    let cover = files::temp_path("toolkitrs-cover-", ".jpg")?;

    // Step 1 (non-fatal): try to extract embedded cover art.
    let has_cover = match run_async(
        ffmpeg,
        args::extract_embedded_cover(&input, &cover),
        cancel.clone(),
        None,
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
        None,
    )
    .await;

    let _ = fs::remove_file(&cover);
    result?;
    Ok(output)
}

/// Wraps a video with a companion image, producing a new MP4 next to it.
pub async fn vidwrap(input: PathBuf, ffmpeg: &Path, cancel: CancellationToken) -> Result<PathBuf> {
    let image = files::companion_image(&input)?;
    let output = files::output_path_for_video_with_image(&input)?;

    run_async(
        ffmpeg,
        args::replace_video_with_image(&image, &input, &output, &[]),
        cancel,
        None,
    )
    .await?;

    Ok(output)
}
