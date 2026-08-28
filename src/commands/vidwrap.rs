//! module vidwrap - Wrap videos with companion image to create a new video with thumbnail
//! images in single or batch mode.

use crate::{
    cli,
    commands::batch,
    components::{
        banner, progress,
        prompt::{self, CleanupChoice, ContinueChoice, ExecutionMode, SiblingBatchChoice},
        spinner::{Spinner, SpinnerStyle},
    },
    ffmpeg::{args, Ffmpeg, ProcessRunner},
    util::{
        batch::{BatchPolicy, BatchReport},
        files,
        parallel::{self, BatchEvent, BatchSummary, FailurePolicy, WorkResult},
    },
    workflow::{Workflow, WorkflowOptions},
};
use anyhow::{bail, Context, Result};
use clap::Args;
use console::Style;
use std::{
    collections::HashSet,
    fs,
    io::{self, BufReader, IsTerminal},
    path::{Path, PathBuf},
    thread::JoinHandle,
};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Total number of steps in the vidwrap workflow for one file.
const TOTAL_STEPS: usize = 1;

/// Default choice index for post-processing prompt (1-based).
const DEFAULT_POST_CHOICE: usize = 2;

/// Suffix used by vidwrap outputs.
///
/// We exclude these from batch discovery so a directory scan does not
/// repeatedly re-wrap previously generated `*_with_image.mp4` files.
const VIDWRAP_OUTPUT_STEM_SUFFIX: &str = "_with_image";

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
/// easier to reason about and easier to test later.
struct VidwrapPlan {
    /// Videos to process, in processing order.
    queue: Vec<PathBuf>,

    /// Policy for errors and continuation.
    policy: BatchPolicy,

    /// Whether the original single-file post-processing prompt should run.
    ///
    /// This is enabled only for true single-file mode. In batch mode we keep
    /// both files automatically to avoid prompting destructively per file.
    interactive_post: bool,
}

/// Running state tracked while a parallel batch is active.
#[derive(Debug, Default)]
struct ParallelProgress {
    /// Indices that acquired a Worker slot and started running.
    started: HashSet<usize>,

    /// Indices that completed successfully.
    succeeded: HashSet<usize>,

    /// Final summary emitted by the batch runner.
    summary: BatchSummary,
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
            Workflow::Vidwrap.file_type_name(),
            Some("Video + Image wrapper"),
            console::colors_enabled()
        )
    );

    let plan = resolve_plan(&args_cli)?;

    if plan.queue.is_empty() {
        println!("No MP4 videos found to process.");
        return Ok(());
    }

    match resolve_mode(plan.queue.len(), args_cli.mode, plan.policy)? {
        ExecutionMode::Sequential => execute_plan(plan, ffmpeg),
        ExecutionMode::Parallel => execute_plan_parallel(plan, ffmpeg),
    }
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Resolves the execution plan from CLI arguments and interactive prompts.
fn resolve_plan(args_cli: &VidwrapArgs) -> Result<VidwrapPlan> {
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
        (None, Some(dir), _) => resolve_directory(dir, args_cli.on_error),
        (None, None, true) => {
            let cwd = std::env::current_dir().context("reading current directory")?;
            resolve_directory(&cwd, args_cli.on_error)
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
    let video = video
        .canonicalize()
        .with_context(|| format!("video file not found: {}", video.display()))?;

    let queue = files::queue_from_entry(
        &video,
        Workflow::Vidwrap.input_extension(),
        Some(VIDWRAP_OUTPUT_STEM_SUFFIX),
    )?;

    if queue.len() <= 1 {
        return Ok(VidwrapPlan {
            queue: vec![video],
            policy: BatchPolicy::Single,
            interactive_post: true,
        });
    }

    let parent = video.parent().context("video has no parent")?;
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();
    let choice = prompt::sibling_batch_choice(&mut input, &mut stdout, parent, queue.len())?;

    let (policy, queue, interactive_post) = match choice {
        SiblingBatchChoice::ProcessInputOnly => (BatchPolicy::Single, vec![video], true),
        SiblingBatchChoice::ProcessAllStopOnError => (BatchPolicy::StopOnError, queue, false),
        SiblingBatchChoice::ProcessAllSkipOnError => (BatchPolicy::SkipOnError, queue, false),
        SiblingBatchChoice::ProcessAllPromptEach => (BatchPolicy::PromptEach, queue, false),
    };

    Ok(VidwrapPlan {
        queue,
        policy,
        interactive_post,
    })
}

/// Resolves a plan for explicit directory batch mode.
fn resolve_directory(dir: &Path, on_error: Option<cli::BatchOnError>) -> Result<VidwrapPlan> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("directory not found: {}", dir.display()))?;

    let queue = files::queue_from_directory(
        &dir,
        Workflow::Vidwrap.input_extension(),
        Some(VIDWRAP_OUTPUT_STEM_SUFFIX),
    )?;

    Ok(VidwrapPlan {
        queue,
        policy: resolve_batch_policy(on_error),
        interactive_post: false,
    })
}

/// Resolves sequential vs parallel execution mode.
fn resolve_mode(
    file_count: usize,
    cli_mode: Option<cli::ExecutionModeCli>,
    policy: BatchPolicy,
) -> Result<ExecutionMode> {
    let requested_mode = batch::resolve_execution_mode(file_count, cli_mode)?;
    Ok(normalize_execution_mode(requested_mode, policy))
}

/// Ensures the execution mode is compatible with the chosen policy.
fn normalize_execution_mode(mode: ExecutionMode, policy: BatchPolicy) -> ExecutionMode {
    if mode == ExecutionMode::Parallel && policy == BatchPolicy::PromptEach {
        println!("Parallel mode does not support per-file prompts; using sequential mode.");
        ExecutionMode::Sequential
    } else {
        mode
    }
}

/// Resolves the effective batch policy for directory-based runs.
fn resolve_batch_policy(on_error: Option<cli::BatchOnError>) -> BatchPolicy {
    match on_error {
        Some(cli::BatchOnError::Stop) => BatchPolicy::StopOnError,
        Some(cli::BatchOnError::Skip) => BatchPolicy::SkipOnError,
        Some(cli::BatchOnError::Prompt) => BatchPolicy::PromptEach,
        None => {
            if io::stdin().is_terminal() {
                BatchPolicy::PromptEach
            } else {
                BatchPolicy::SkipOnError
            }
        }
    }
}

/// Executes the resolved vidwrap plan.
///
/// This owns the batch loop, error policy, continue prompts, and final report.
fn execute_plan<R: ProcessRunner>(plan: VidwrapPlan, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let total = plan.queue.len();
    let is_single = plan.policy == BatchPolicy::Single && total == 1;
    let interactive_post = plan.interactive_post && is_single;

    let mut policy = plan.policy;
    let mut report = BatchReport::new();

    for (index, video) in plan.queue.iter().enumerate() {
        if total > 1 {
            println!("Video {}/{}: {}", index + 1, total, video.display());
        }

        let processed = match process_one(video, ffmpeg, interactive_post) {
            Ok(_) => {
                report.record_success(video.clone());
                true
            }
            Err(error) => {
                report.record_failed(video.clone(), error.to_string());
                handle_sequential_error(&mut report, &mut policy, video, error, is_single)?;
                true
            }
        };

        if processed && should_prompt_to_continue(policy, index, total) {
            let next = &plan.queue[index + 1];
            if !continue_after_prompt(next, &mut policy)? {
                break;
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
    let output = output_path_for(&video)?;

    println!(
        "Found image: {}
Found video: {}",
        image.display(),
        video.display()
    );

    let label = "Creating video with static image".to_string();
    println!("{}", progress::render(1, TOTAL_STEPS, &label));

    let spinner = Spinner::start(SpinnerStyle::Bounce, label.clone(), false);
    let result = ffmpeg.run(args::replace_video_with_image(&image, &video, &output, &[]));
    let spinner_enabled = spinner.enabled();
    spinner.stop();

    if let Err(error) = result {
        eprintln!(
            "Failed; output file may be incomplete: {}",
            output.display()
        );
        return Err(error);
    }

    print_success_line(spinner_enabled);
    println!("Output: {}", output.display());

    if interactive_post {
        post_process(&video, &image, &output)?;
    }

    Ok(output)
}

/// Computes the vidwrap output path for a video.
///
/// Example:
/// `/videos/input.mp4` -> `/videos/input_with_image.mp4`
fn output_path_for(video: &Path) -> Result<PathBuf> {
    Workflow::Vidwrap.output_path(video, &workflow_options())
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

/// Executes the resolved plan in parallel mode.
fn execute_plan_parallel<R: ProcessRunner>(plan: VidwrapPlan, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let queue = plan.queue;
    let cancel = CancellationToken::new();
    let _ctrl_c = spawn_ctrl_c_handler(cancel.clone());
    let Worker = build_parallel_Worker(ffmpeg.binary().to_path_buf());
    let failure_policy = parallel_failure_policy(plan.policy);

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let runner_handle = runtime.spawn(parallel::run_parallel(
        queue.clone(),
        parallel::num_cpus(),
        cancel,
        failure_policy,
        event_tx,
        Worker,
    ));

    let progress =
        runtime.block_on(async { collect_parallel_progress(&queue, &mut event_rx).await });
    let _ = runtime.block_on(runner_handle)?;

    if progress.summary.stopped_early() {
        prompt_cleanup_residual_files(&queue, &progress)?;
    }

    print_parallel_summary(progress.summary);

    if progress.summary.failed > 0 {
        bail!("one or more vidwrap operations failed");
    }

    Ok(())
}

/// Handles a failed sequential item according to the active batch policy.
fn handle_sequential_error(
    report: &mut BatchReport,
    policy: &mut BatchPolicy,
    video: &Path,
    error: anyhow::Error,
    is_single: bool,
) -> Result<()> {
    match *policy {
        BatchPolicy::Single => {
            if !is_single {
                report.print_summary();
            }
            Err(error)
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
            Ok(())
        }
        BatchPolicy::PromptEach => {
            eprintln!("Failed: {error}");
            Ok(())
        }
    }
}

/// Returns true when the sequential loop should prompt before moving on.
fn should_prompt_to_continue(policy: BatchPolicy, index: usize, total: usize) -> bool {
    policy == BatchPolicy::PromptEach && index + 1 < total
}

/// Prompts the user before the next sequential file.
fn continue_after_prompt(next: &Path, policy: &mut BatchPolicy) -> Result<bool> {
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    match prompt::continue_to_next(&mut input, &mut stdout, next)? {
        ContinueChoice::Yes => Ok(true),
        ContinueChoice::YesToAll => {
            *policy = BatchPolicy::SkipOnError;
            Ok(true)
        }
        ContinueChoice::No => Ok(false),
    }
}

/// Prints the final success line after a single ffmpeg run.
fn print_success_line(spinner_enabled: bool) {
    if spinner_enabled {
        let green = Style::new().green().bold();
        println!("  {} Success", green.apply_to("✔"));
    } else {
        println!("  [OK] Success");
    }
}

/// Spawns a Ctrl+C listener that cancels the active parallel batch.
fn spawn_ctrl_c_handler(cancel: CancellationToken) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("ctrl-c runtime");
        runtime.block_on(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\nCancellation requested (press Ctrl+C again to force exit)...");
            cancel.cancel();
        });
    })
}

/// Builds the workflow-specific Worker used by the parallel executor.
fn build_parallel_Worker(
    ffmpeg_binary: PathBuf,
) -> impl Fn(
    PathBuf,
    CancellationToken,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send>>
       + Send
       + Sync
       + Clone
       + 'static {
    move |input, cancel| {
        let ffmpeg_binary = ffmpeg_binary.clone();
        let options = workflow_options();
        Box::pin(async move {
            let output = Workflow::Vidwrap.output_path(&input, &options)?;
            Workflow::Vidwrap
                .run_async(input, output, &options, &ffmpeg_binary, cancel, None)
                .await
        })
    }
}

/// Converts the vidwrap batch policy into the shared parallel failure policy.
fn parallel_failure_policy(policy: BatchPolicy) -> FailurePolicy {
    match policy {
        BatchPolicy::StopOnError => FailurePolicy::CancelRemaining,
        BatchPolicy::Single | BatchPolicy::SkipOnError | BatchPolicy::PromptEach => {
            FailurePolicy::Continue
        }
    }
}

/// Collects progress and logging information from the parallel executor.
async fn collect_parallel_progress(
    queue: &[PathBuf],
    event_rx: &mut UnboundedReceiver<BatchEvent>,
) -> ParallelProgress {
    let mut progress = ParallelProgress::default();

    while let Some(event) = event_rx.recv().await {
        match event {
            BatchEvent::Started(index) => {
                progress.started.insert(index);
                println!("▶ Started [{}]: {}", index + 1, queue[index].display());
            }
            BatchEvent::Done(index, result) => match result {
                WorkResult::Success(path) => {
                    progress.succeeded.insert(index);
                    println!("✔ Success [{}]: {}", index + 1, path.display());
                }
                WorkResult::Failed(error) => {
                    eprintln!("✖ Failed [{}]: {error}", index + 1);
                }
                WorkResult::Cancelled => {
                    eprintln!("⊗ Cancelled [{}]: {}", index + 1, queue[index].display());
                }
            },
            BatchEvent::AllDone(summary) => {
                progress.summary = summary;
                break;
            }
        }
    }

    progress
}

/// Prompts to remove residual outputs left behind by a cancelled or aborted run.
fn prompt_cleanup_residual_files(queue: &[PathBuf], progress: &ParallelProgress) -> Result<()> {
    let options = workflow_options();
    let residual: Vec<PathBuf> = progress
        .started
        .iter()
        .filter(|index| !progress.succeeded.contains(index))
        .filter_map(|index| Workflow::Vidwrap.output_path(&queue[*index], &options).ok())
        .filter(|path| path.exists())
        .collect();

    if residual.is_empty() {
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    if matches!(
        prompt::cleanup_residual_choice(&mut input, &mut stdout, &residual)?,
        CleanupChoice::Remove
    ) {
        for path in &residual {
            let _ = fs::remove_file(path);
        }
        println!("Removed {} residual files.", residual.len());
    } else {
        println!("Kept {} residual files.", residual.len());
    }

    Ok(())
}

/// Prints the final summary banner for a parallel run.
fn print_parallel_summary(summary: BatchSummary) {
    println!(
        "{}",
        banner::render(
            "Done",
            Some(&format!(
                "{} succeeded · {} failed{}",
                summary.succeeded,
                summary.failed,
                summary_suffix(summary)
            )),
            console::colors_enabled()
        )
    );
}

/// Returns the status suffix shown in the final summary banner.
fn summary_suffix(summary: BatchSummary) -> &'static str {
    if summary.was_cancelled() {
        " · cancelled"
    } else if summary.stopped_on_error() {
        " · stopped on error"
    } else {
        ""
    }
}

/// Returns the shared workflow options for vidwrap.
fn workflow_options() -> WorkflowOptions {
    WorkflowOptions::default()
}
