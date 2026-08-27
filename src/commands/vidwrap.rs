//! module vidwrap - Wrap videos with companion image to create a new video with thumbnail
//! images in single or batch mode.

use crate::{
    cli,
    commands::{batch, workers},
    components::{
        banner, progress,
        prompt::{self, ContinueChoice, ExecutionMode},
        spinner::{Spinner, SpinnerStyle},
    },
    ffmpeg::{args, Ffmpeg, ProcessRunner},
    util::{
        batch::{BatchPolicy, BatchReport},
        files,
    },
};
use anyhow::{bail, Context, Result};
use clap::Args;
use console::Style;
use std::{
    fs,
    io::{self, BufReader},
    path::{Path, PathBuf},
};

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Total number of steps in the vidwrap workflow for one file.
const TOTAL_STEPS: usize = 1;

/// Default choice index for post-processing prompt (1-based).
const DEFAULT_POST_CHOICE: usize = 2;

/// Extension scanned for batch video discovery.
const MP4_EXT: &str = "mp4";

/// Arguments for the `vidwrap` subcommand.
///
/// The doc comments on each field are rendered by clap in:
///
/// ```bash
/// toolkitrs vidwrap --help
/// ```
#[derive(Debug, Args)]
#[command(after_help = "Examples:
  toolkitrs vidwrap input.mp4               Prompt when sibling MP4 files exist
  toolkitrs vidwrap --batch                 Process all MP4 files in the current directory
  toolkitrs vidwrap --input-dir /videos     Process all MP4 files in /videos
  toolkitrs vidwrap --batch --on-error skip Continue past errors and report at the end")]
pub struct VidwrapArgs {
    /// Video with a same-basename companion image.
    ///
    /// Omit this when using --batch or --input-dir.
    #[arg(value_name = "VIDEO")]
    pub video: Option<PathBuf>,
    /// Process all MP4 videos in the current directory.
    #[arg(long)]
    pub batch: bool,
    /// Directory to scan for MP4 videos.
    ///
    /// This implies batch processing.
    #[arg(long, value_name = "DIR")]
    pub input_dir: Option<PathBuf>,
    /// Error policy for explicit batch mode.
    ///
    /// Requires --batch or --input-dir. If omitted, interactive terminals
    /// prompt after each video, and non-interactive terminals skip errors.
    #[arg(long, value_enum, value_name = "POLICY")]
    pub on_error: Option<cli::BatchOnError>,
    /// Execution mode for multi-file batches.
    #[arg(long, value_enum, value_name = "MODE")]
    pub mode: Option<cli::ExecutionModeCli>,
}

/// Resolved execution plan for vidwrap.
///
/// This separates queue construction from execution, which makes the command
/// easier to reason about and easier to test later. The interactive
/// post-processing prompt runs exactly when `policy` is [`BatchPolicy::Single`]
/// (true single-file mode); batch modes keep both files automatically to
/// avoid prompting destructively per file.
struct VidwrapPlan {
    /// Videos to process, in processing order.
    queue: Vec<PathBuf>,

    /// Policy for errors and continuation.
    policy: BatchPolicy,
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs the vidwrap workflow in either single-file or batch mode.
///
/// Single-file mode:
/// ```bash
/// toolkitrs vidwrap input.mp4
/// ```
///
/// Batch mode:
/// ```bash
/// toolkitrs vidwrap --batch
/// toolkitrs vidwrap --input-dir /videos
/// ```
pub fn run<R: ProcessRunner>(args_cli: VidwrapArgs, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    println!(
        "{}",
        banner::render(
            "Vidwrap",
            Some("Video + Image wrapper"),
            console::colors_enabled()
        )
    );

    let plan = resolve_plan(&args_cli)?;

    if plan.queue.is_empty() {
        println!("No MP4 videos found to process.");
        return Ok(());
    }

    match batch::resolve_execution_mode(plan.queue.len(), args_cli.mode)? {
        ExecutionMode::Sequential => execute_plan(plan, ffmpeg),
        ExecutionMode::Parallel => {
            // vidwrap's ffmpeg arguments always overwrite, so force = true.
            let job = workers::vidwrap_job(true, ffmpeg.binary());
            batch::run_parallel_console("Vidwrap", plan.queue, job)
        }
    }
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Resolves the execution plan from CLI arguments and interactive prompts.
fn resolve_plan(args_cli: &VidwrapArgs) -> Result<VidwrapPlan> {
    let exclude = Some(files::VIDWRAP_OUTPUT_STEM_SUFFIX);

    match (&args_cli.video, &args_cli.input_dir, args_cli.batch) {
        (Some(video), None, false) => {
            if args_cli.on_error.is_some() {
                bail!("--on-error can only be used with --batch or --input-dir");
            }

            resolve_explicit_video(video)
        }
        (Some(_), _, _) => {
            bail!("VIDEO cannot be combined with --batch or --input-dir. Omit VIDEO to scan a directory.")
        }
        (None, Some(dir), _) => {
            let (queue, policy) =
                batch::resolve_directory_queue(dir, MP4_EXT, exclude, args_cli.on_error)?;
            Ok(VidwrapPlan { queue, policy })
        }
        (None, None, true) => {
            let cwd = std::env::current_dir().context("reading current directory")?;
            let (queue, policy) =
                batch::resolve_directory_queue(&cwd, MP4_EXT, exclude, args_cli.on_error)?;
            Ok(VidwrapPlan { queue, policy })
        }
        (None, None, false) => {
            bail!("provide VIDEO, or use --batch / --input-dir <DIR>")
        }
    }
}

/// Resolves a plan when the user explicitly supplies one video.
///
/// If additional MP4 siblings are discovered, this prompts the user with:
/// - process input only
/// - process whole path, stop on error
/// - process whole path, skip on error
/// - process whole path, prompt each
fn resolve_explicit_video(video: &Path) -> Result<VidwrapPlan> {
    let (queue, policy) = batch::resolve_single_file_with_siblings(
        MP4_EXT,
        Some(files::VIDWRAP_OUTPUT_STEM_SUFFIX),
        video,
    )?;
    Ok(VidwrapPlan { queue, policy })
}

/// Executes the resolved vidwrap plan.
///
/// This owns the batch loop, error policy, continue prompts, and final report.
fn execute_plan<R: ProcessRunner>(plan: VidwrapPlan, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let total = plan.queue.len();
    let is_single = plan.policy == BatchPolicy::Single && total == 1;

    let mut policy = plan.policy;
    let mut report = BatchReport::new();
    let mut index = 0;

    while index < total {
        let video = plan.queue[index].clone();

        if total > 1 {
            println!("Video {}/{}: {}", index + 1, total, video.display());
        }

        match process_one(&video, ffmpeg, is_single) {
            Ok(_) => {
                report.record_success(video.clone());
            }
            Err(error) => {
                report.record_failed(video.clone(), error.to_string());

                match policy {
                    BatchPolicy::Single => {
                        if !is_single {
                            report.print_summary();
                        }
                        return Err(error);
                    }
                    BatchPolicy::StopOnError => {
                        report.print_summary();
                        bail!(
                            "vidwrap stopped after error on {}: {error}",
                            video.display()
                        );
                    }
                    BatchPolicy::SkipOnError => {
                        eprintln!("Failed; continuing: {error}");
                    }
                    BatchPolicy::PromptEach => {
                        eprintln!("Failed: {error}");
                    }
                }
            }
        }

        index += 1;

        if policy == BatchPolicy::PromptEach && index < total {
            let next = &plan.queue[index];

            let stdin = io::stdin();
            let mut input = BufReader::new(stdin.lock());
            let mut stdout = io::stdout();

            match prompt::continue_to_next(&mut input, &mut stdout, next)? {
                ContinueChoice::Yes => {}
                ContinueChoice::YesToAll => {
                    // After "yes to all", stop prompting and continue
                    // automatically. Errors are skipped so the batch can
                    // finish and report at the end.
                    policy = BatchPolicy::SkipOnError;
                }
                ContinueChoice::No => break,
            }
        }
    }

    if !is_single {
        report.print_summary();
    }

    if report.has_failures() {
        bail!("one or more vidwrap operations failed");
    }

    Ok(())
}

/// Processes one video with its companion image.
///
/// When `interactive_post` is true, the original post-processing prompt is
/// shown after success. In batch mode this is disabled so we do not ask
/// destructive per-file questions for every video.
fn process_one<R: ProcessRunner>(
    video: &Path,
    ffmpeg: &Ffmpeg<R>,
    interactive_post: bool,
) -> Result<PathBuf> {
    let video = video
        .canonicalize()
        .with_context(|| format!("video file not found: {}", video.display()))?;

    let image = files::companion_image(&video)?;

    println!(
        "Found image: {}
Found video: {}",
        image.display(),
        video.display()
    );

    let output = files::output_path_for_video_with_image(&video)?;

    let label = "Creating video with static image".to_string();
    println!("{}", progress::render(1, TOTAL_STEPS, &label));

    let spin = Spinner::start(SpinnerStyle::Bounce, label.clone(), false);
    let result = ffmpeg.run(args::replace_video_with_image(&image, &video, &output, &[]));
    let was_enabled = spin.enabled();
    spin.stop();

    if let Err(error) = result {
        eprintln!(
            "Failed; output file may be incomplete: {}",
            output.display()
        );
        return Err(error);
    }

    if was_enabled {
        let green = Style::new().green().bold();
        println!("  {} Success", green.apply_to("✔"));
    } else {
        println!("  [OK] Success");
    }

    println!("Output: {}", output.display());

    if interactive_post {
        post_process(&video, &image, &output)?;
    }

    Ok(output)
}

/// Prompts the user for post-processing actions on the original video and image.
///
/// This remains enabled only for true single-file mode. In batch mode we keep
/// all files automatically to avoid destructive prompts for every file.
fn post_process(original: &Path, image: &Path, new_video: &Path) -> Result<()> {
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    let choice = prompt::choice(
        &mut input,
        &mut stdout,
        "What would you like to do?",
        &[
            "Replace original",
            "Keep both files",
            "Delete original only",
        ],
        DEFAULT_POST_CHOICE,
    )?;

    match choice {
        1 => {
            fs::remove_file(original)?;
            fs::rename(new_video, original)?;
            let _ = fs::remove_file(image);
            println!("Replaced original and cleaned up")
        }
        3 => {
            fs::remove_file(original)?;
            let _ = fs::remove_file(image);
            println!("Deleted original and source image")
        }
        _ => println!("Kept all files"),
    }

    Ok(())
}
