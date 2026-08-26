//! module runner - Background ffmpeg execution that streams logs into the TUI.
//!
//! This intentionally does NOT reuse the blocking `RealRunner`, because a TUI
//! needs incremental output. Instead it reuses the *argument builders* in
//! `ffmpeg::args` (the real domain logic) and spawns processes itself with
//! piped stderr so each line is pushed to the event channel live.

use crate::{
    ffmpeg::args,
    tui::{app::RunOptions, event::AppEvent, workflow::Workflow},
    util::{
        files, output,
        parallel::{self, BatchEvent},
    },
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
use tokio_util::sync::CancellationToken;

/// Spawns a worker thread that processes files (sequentially or in parallel).
///
/// Emits `FileStarted`, `Log`, `FileDone`, and finally `AllDone` events.
pub fn spawn_worker(
    tx: Sender<AppEvent>,
    ffmpeg_path: PathBuf,
    workflow: Workflow,
    files: Vec<PathBuf>,
    options: RunOptions,
    parallel_mode: bool,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(AppEvent::Log(format!("✗ Failed to start runtime: {e}")));
                let _ = tx.send(AppEvent::AllDone {
                    succeeded: 0,
                    failed: files.len(),
                });
                return;
            }
        };

        // Ensure the output directory exists for workflows that use it.
        if workflow != Workflow::Vidwrap {
            if let Err(e) = output::ensure_directory(&options.output_dir) {
                let _ = tx.send(AppEvent::Log(format!("✗ {e}")));
            }
        }

        let concurrency = if parallel_mode {
            parallel::num_cpus()
        } else {
            1
        };

        rt.block_on(async move {
            let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();

            // Build the workflow-specific worker.
            let worker = build_worker(workflow, options.clone(), ffmpeg_path.clone());

            // Spawn the parallel runner.
            let files_for_runner = files.clone();
            let runner_task = tokio::spawn(parallel::run_parallel(
                files_for_runner,
                concurrency,
                cancel.clone(),
                event_tx,
                worker,
            ));

            // Track which files were started but didn't finish successfully,
            // so we can offer cleanup on cancellation.
            let mut started: std::collections::HashSet<usize> = std::collections::HashSet::new();
            let mut succeeded: std::collections::HashSet<usize> = std::collections::HashSet::new();
            let mut failed = 0usize;
            let mut cancelled = false;

            while let Some(event) = event_rx.recv().await {
                match event {
                    BatchEvent::Started(i) => {
                        started.insert(i);
                        let _ = tx.send(AppEvent::FileStarted(i));
                    }
                    BatchEvent::Done(i, work_result) => {
                        if work_result.is_success() {
                            succeeded.insert(i);
                            let _ = tx.send(AppEvent::FileDone(i, true));
                        } else if let Some(e) = work_result.error {
                            if e.to_string() == "cancelled" {
                                let _ = tx.send(AppEvent::FileDone(i, false));
                            } else {
                                failed += 1;
                                let _ = tx.send(AppEvent::Log(format!("✗ {e}")));
                                let _ = tx.send(AppEvent::FileDone(i, false));
                            }
                        }
                    }
                    BatchEvent::AllDone(summary) => {
                        cancelled = summary.cancelled;
                        if cancelled {
                            // Compute residual files for cleanup prompt.
                            let residual: Vec<PathBuf> = started
                                .iter()
                                .filter(|i| !succeeded.contains(i))
                                .filter_map(|i| {
                                    compute_output_path(workflow, &files[*i], &options).ok()
                                })
                                .filter(|p| p.exists())
                                .collect();

                            let _ = tx.send(AppEvent::AllDone {
                                succeeded: summary.succeeded,
                                failed: summary.failed,
                            });
                            if !residual.is_empty() {
                                let _ = tx.send(AppEvent::CancelledWithResidual(residual));
                            }
                        } else {
                            let _ = tx.send(AppEvent::AllDone {
                                succeeded: summary.succeeded,
                                failed: summary.failed,
                            });
                        }
                        break;
                    }
                }
            }

            let _ = runner_task.await;
        });
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
            let out = files::output_path_for_video_with_image(input)?;
            exec(
                tx,
                ffmpeg_path,
                args::replace_video_with_image(&image, input, &out, &[]),
            )
        }
        Workflow::Mp32Mp4 => {
            let out = output::output_path(input, &options.output_dir, "mp4")?;
            let cover = files::temp_path("toolkitrs-tui-cover-", ".jpg")?;
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
            let cover = files::temp_path("toolkitrs-tui-cover-", ".jpg")?;
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

/// Builds the workflow-specific async worker closure.
fn build_worker(
    workflow: Workflow,
    options: RunOptions,
    ffmpeg_path: PathBuf,
) -> impl Fn(
    PathBuf,
    CancellationToken,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send>>
       + Send
       + Sync
       + Clone
       + 'static {
    move |input, cancel| {
        let workflow = workflow;
        let options = options.clone();
        let ffmpeg_path = ffmpeg_path.clone();
        Box::pin(async move {
            let out = compute_output_path(workflow, &input, &options)?;
            if out.exists() && !options.force {
                return Ok(out);
            }
            match workflow {
                Workflow::Ts2Mp4 => {
                    crate::commands::workers::ts2mp4(
                        input,
                        out,
                        options.force,
                        &ffmpeg_path,
                        cancel,
                    )
                    .await
                }
                Workflow::Mkv2Mp3 => {
                    crate::commands::workers::mkv2mp3(
                        input,
                        out,
                        options.bitrate,
                        options.cover_size,
                        options.force,
                        &ffmpeg_path,
                        cancel,
                    )
                    .await
                }
                Workflow::Mp32Mp4 => {
                    crate::commands::workers::mp32mp4(
                        input,
                        out,
                        options.bitrate,
                        options.force,
                        false,
                        &ffmpeg_path,
                        cancel,
                    )
                    .await
                }
                Workflow::Vidwrap => {
                    crate::commands::workers::vidwrap(input, &ffmpeg_path, cancel).await
                }
            }
        })
    }
}

/// Computes the output path for a given workflow + input + options.
fn compute_output_path(workflow: Workflow, input: &Path, options: &RunOptions) -> Result<PathBuf> {
    match workflow {
        Workflow::Vidwrap => {
            let dir = input.parent().context("no parent")?;
            let stem = input.file_stem().context("no stem")?.to_string_lossy();
            Ok(dir.join(format!("{stem}_with_image.mp4")))
        }
        _ => output::output_path(input, &options.output_dir, workflow_output_ext(workflow)),
    }
}

fn workflow_output_ext(workflow: Workflow) -> &'static str {
    match workflow {
        Workflow::Ts2Mp4 | Workflow::Mp32Mp4 | Workflow::Vidwrap => "mp4",
        Workflow::Mkv2Mp3 => "mp3",
    }
}
