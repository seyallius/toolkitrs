//! module runner - Background ffmpeg execution that streams logs into the TUI.
//!
//! This intentionally does NOT reuse the blocking `RealRunner`, because a TUI
//! needs incremental output. Instead it reuses the *argument builders* in
//! `ffmpeg::args` (the real domain logic) and spawns processes itself with
//! piped stderr so each line is pushed to the event channel live.

use crate::{
    ffmpeg::args,
    tui::{app::RunOptions, event::AppEvent, workflow::Workflow},
    util::{files, output},
};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::Sender,
    thread::{self, JoinHandle},
};

/// Spawns a worker thread that processes every file sequentially.
///
/// Emits `FileStarted`, `Log`, `FileDone`, and finally `AllDone` events.
pub fn spawn_worker(
    tx: Sender<AppEvent>,
    ffmpeg_path: PathBuf,
    workflow: Workflow,
    files: Vec<PathBuf>,
    options: RunOptions,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut succeeded = 0usize;
        let mut failed = 0usize;

        // Ensure the output directory exists for workflows that use it.
        if workflow != Workflow::Vidwrap {
            if let Err(e) = output::ensure_directory(&options.output_dir) {
                let _ = tx.send(AppEvent::Log(format!("✗ {e}")));
            }
        }

        for (index, input) in files.iter().enumerate() {
            let _ = tx.send(AppEvent::FileStarted(index));
            let _ = tx.send(AppEvent::Log(format!("── Processing {}", input.display())));

            match run_one(&tx, &ffmpeg_path, workflow, input, &options) {
                Ok(()) => {
                    succeeded += 1;
                    let _ = tx.send(AppEvent::FileDone(index, true));
                }
                Err(e) => {
                    failed += 1;
                    let _ = tx.send(AppEvent::Log(format!("✗ Error: {e}")));
                    let _ = tx.send(AppEvent::FileDone(index, false));
                }
            }
        }
        let _ = tx.send(AppEvent::AllDone { succeeded, failed });
    })
}

/// Executes the correct ffmpeg sequence for one file of a given workflow.
///
/// This is the one place that knows each workflow's step composition,
/// and it reuses the shared `ffmpeg::args` builders rather than re-deriving them.
fn run_one(
    tx: &Sender<AppEvent>,
    ffmpeg_path: &Path,
    workflow: Workflow,
    input: &Path,
    options: &RunOptions,
) -> Result<()> {
    match workflow {
        Workflow::Ts2Mp4 => {
            let out = output::output_path(input, &options.output_dir, "mp4")?;
            exec(
                tx,
                ffmpeg_path,
                args::remux_copy(input, &out, options.force),
            )
        }
        Workflow::Vidwrap => {
            let image = files::companion_image(input)?;
            let out = vidwrap_output(input)?;
            exec(
                tx,
                ffmpeg_path,
                args::replace_video_with_image(&image, input, &out, &[]),
            )
        }
        Workflow::Mp32Mp4 => {
            let out = output::output_path(input, &options.output_dir, "mp4")?;
            let cover = files::temp_path("toolkit-tui-cover-", ".jpg")?;
            // Step 1 (non-fatal): try to extract embedded cover art.
            let _ = exec(tx, ffmpeg_path, args::extract_embedded_cover(input, &cover));
            let has_cover = cover.metadata().map(|m| m.len() > 0).unwrap_or(false);
            // Step 2: encode with cover if we got one, else black fallback.
            let result = exec(
                tx,
                ffmpeg_path,
                args::encode_mp4(
                    has_cover.then_some(cover.as_path()),
                    input,
                    &out,
                    options.bitrate,
                    options.force,
                ),
            );
            let _ = fs::remove_file(&cover);
            result
        }
        Workflow::Mkv2Mp3 => {
            let out = output::output_path(input, &options.output_dir, "mp3")?;
            let cover = files::temp_path("toolkit-tui-cover-", ".jpg")?;
            // Step 1 (non-fatal): grab a frame for cover art.
            let has_cover = exec(
                tx,
                ffmpeg_path,
                args::extract_frame(input, &cover, options.cover_size),
            )
            .is_ok()
                && cover.metadata().map(|m| m.len() > 0).unwrap_or(false);
            // Step 2: encode the MP3, attaching the cover if present.
            let result = exec(
                tx,
                ffmpeg_path,
                args::encode_mp3(
                    input,
                    has_cover.then_some(cover.as_path()),
                    &out,
                    options.bitrate,
                    options.force,
                ),
            );
            let _ = fs::remove_file(&cover);
            result
        }
    }
}

/// Spawns ffmpeg, streams its stderr into the channel, and reports success.
///
/// stdout is sent to null to avoid a pipe deadlock (ffmpeg logs to stderr).
fn exec(tx: &Sender<AppEvent>, ffmpeg_path: &Path, cmd_args: Vec<String>) -> Result<()> {
    let _ = tx.send(AppEvent::Log(format!(
        "▶ {} {}",
        ffmpeg_path.display(),
        cmd_args.join(" ")
    )));

    let mut child = Command::new(ffmpeg_path)
        .args(&cmd_args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", ffmpeg_path.display()))?;

    // Stream stderr line-by-line into the UI.
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines() {
            if let Ok(line) = line {
                let _ = tx.send(AppEvent::Log(line));
            }
        }
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        bail!("ffmpeg exited with {status}")
    }
}

/// Computes the vidwrap output path next to the source video.
fn vidwrap_output(video: &Path) -> Result<PathBuf> {
    let dir = video.parent().context("video has no parent")?;
    let stem = video
        .file_stem()
        .context("video has no stem")?
        .to_string_lossy();
    Ok(dir.join(format!("{stem}_with_image.mp4")))
}
