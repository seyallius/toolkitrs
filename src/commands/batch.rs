//! module batch - Generic interactive batch processing pipeline for media conversion commands.

use crate::{
    cli::{BatchArgs, BatchOnError, ExecutionModeCli},
    components::{
        banner,
        prompt::{self, CleanupChoice, ContinueChoice, ExecutionMode, SiblingBatchChoice},
    },
    ffmpeg::{Ffmpeg, ProcessRunner},
    util::{
        batch::{BatchPolicy, BatchReport},
        files,
        output::{self, OutputDecision},
        parallel::{self, BatchEvent, BatchSummary, FailurePolicy, WorkResult},
    },
};
use anyhow::{bail, Context, Result};
use std::{
    future::Future,
    io::{self, BufReader, IsTerminal},
    path::{Path, PathBuf},
    pin::Pin,
};
use tokio_util::sync::CancellationToken;

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Warning prefix printed for invalid inputs.
const WARNING_PREFIX: &str = "WARNING";

/// Outcome of processing a single file in a batch.
#[derive(Debug, Clone)]
pub enum FileOutcome {
    /// The file was successfully processed.
    Success,

    /// The file was intentionally skipped with a provided reason.
    Skipped(String),
}

/// Boxed future returned by async batch Workers.
pub type BatchFuture = Pin<Box<dyn Future<Output = Result<PathBuf>> + Send>>;

/// Trait representing a single file conversion task within a batch.
pub trait BatchTask: Clone + Send + Sync + 'static {
    /// The file extension to discover (e.g., "ts", "mkv").
    fn input_extension(&self) -> &str;

    /// The output file extension (e.g., "mp4", "mp3").
    fn output_extension(&self) -> &str;

    /// The human-readable name of the file type for log messages.
    fn file_type_name(&self) -> &str;

    /// Optional stem suffix to exclude from batch discovery (e.g., "_with_image").
    fn exclude_stem_suffix(&self) -> Option<&str> {
        None
    }

    /// Optional banner title override.
    fn banner_title(&self) -> &str {
        self.file_type_name()
    }

    /// Computes the output path for one input file.
    fn output_path(&self, input: &Path, output_dir: &Path) -> Result<PathBuf> {
        output::output_path(input, output_dir, self.output_extension())
    }

    /// Executes the conversion logic for a single file.
    fn process_file<R: ProcessRunner>(
        &self,
        input: &Path,
        output: &Path,
        ffmpeg: &Ffmpeg<R>,
    ) -> Result<FileOutcome>;

    /// Executes the conversion logic for a single file asynchronously.
    fn process_file_async(
        &self,
        input: PathBuf,
        output: PathBuf,
        ffmpeg_binary: PathBuf,
        cancel: CancellationToken,
    ) -> BatchFuture;
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Executes an interactive batch process over a collection of files.
///
/// Handles queue resolution (including sibling discovery), error policies,
/// continuation prompts, and final reporting.
pub fn run_batch<R: ProcessRunner, T: BatchTask>(
    task: &T,
    args: &BatchArgs,
    explicit_files: Vec<PathBuf>,
    ffmpeg: &Ffmpeg<R>,
) -> Result<()> {
    println!(
        "{}",
        banner::render(
            task.banner_title(),
            Some("Batch Processing"),
            console::colors_enabled()
        )
    );

    let (queue, policy) = resolve_queue_and_policy(task, args, explicit_files)?;

    if queue.is_empty() {
        println!("No {} files found to process.", task.file_type_name());
        return Ok(());
    }

    // Ensure output directory exists if we are going to process anything.
    output::ensure_directory(&args.output_dir)?;

    // Decide execution mode (parallel vs sequential) for multi-file batches.
    let requested_mode = resolve_execution_mode(queue.len(), args.mode)?;
    let mode = normalize_execution_mode(requested_mode, policy);

    match mode {
        ExecutionMode::Sequential => execute_queue(task, args, &queue, policy, ffmpeg),
        ExecutionMode::Parallel => {
            execute_queue_parallel(task.clone(), args, queue, policy, ffmpeg.binary())
        }
    }
}

/// Resolves execution mode from CLI flag or interactive prompt.
///
/// Single-file queues always run sequentially (no benefit from parallelism).
pub fn resolve_execution_mode(
    file_count: usize,
    cli_mode: Option<ExecutionModeCli>,
) -> Result<ExecutionMode> {
    if file_count <= 1 {
        return Ok(ExecutionMode::Sequential);
    }

    if let Some(m) = cli_mode {
        return Ok(match m {
            ExecutionModeCli::Sequential => ExecutionMode::Sequential,
            ExecutionModeCli::Parallel => ExecutionMode::Parallel,
        });
    }

    let cores = parallel::num_cpus();
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();
    Ok(prompt::execution_mode_choice(
        &mut input,
        &mut stdout,
        file_count,
        cores,
    )?)
}

/// Runs a parallel batch with the given Worker closure.
///
/// This is shared by all workflows: the caller passes a closure that knows
/// how to process one file for its specific workflow.
///
/// # Arguments
/// * `banner_title` - Title for the banner (e.g. "TS").
/// * `queue` - Input file paths, in order.
/// * `output_dir` - Where outputs go.
/// * `output_ext` - Output extension (used to pre-compute output paths for cleanup).
/// * `force` - Overwrite existing outputs.
/// * `ffmpeg_binary` - Path to the ffmpeg binary.
/// * `Worker` - Async closure: `(input, cancel) -> Result<PathBuf>`.
pub fn run_batch_parallel<F, Fut>(
    banner_title: &str,
    queue: Vec<PathBuf>,
    output_dir: &Path,
    output_ext: &str,
    force: bool,
    _ffmpeg_binary: &Path,
    Worker: F,
) -> Result<()>
where
    F: Fn(PathBuf, CancellationToken) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<PathBuf>> + Send + 'static,
{
    println!("{}", runner_banner(banner_title, queue.len()));

    output::ensure_directory(output_dir)?;

    let outputs = queue
        .iter()
        .map(|path| output::output_path(path, output_dir, output_ext))
        .collect::<Result<Vec<_>>>()?;

    let output_dir = output_dir.to_path_buf();
    let output_ext = output_ext.to_string();
    let wrapped_Worker = move |input: PathBuf, cancel: CancellationToken| {
        let output_dir = output_dir.clone();
        let output_ext = output_ext.clone();
        let Worker = Worker.clone();
        async move {
            let output = output::output_path(&input, &output_dir, &output_ext)
                .unwrap_or_else(|_| input.with_extension(&output_ext));
            if matches!(
                output::decision(&output, force),
                OutputDecision::SkipExisting
            ) {
                return Ok(output);
            }
            Worker(input, cancel).await
        }
    };

    drive_parallel_batch(
        &queue,
        outputs,
        FailurePolicy::Continue,
        String::new,
        || final_error_message("one or more conversions failed"),
        wrapped_Worker,
    )
}

/// Resolves only the queue + policy without executing anything.
/// Used by commands that want to dispatch to parallel themselves.
pub fn resolve_queue_only<T: BatchTask>(
    task: &T,
    args: &BatchArgs,
    explicit_files: Vec<PathBuf>,
) -> Result<(Vec<PathBuf>, BatchPolicy)> {
    resolve_queue_and_policy(task, args, explicit_files)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Resolves the execution queue and batch policy from CLI arguments and interactive prompts.
fn resolve_queue_and_policy<T: BatchTask>(
    task: &T,
    args: &BatchArgs,
    explicit_files: Vec<PathBuf>,
) -> Result<(Vec<PathBuf>, BatchPolicy)> {
    let ext = task.input_extension();
    let exclude = task.exclude_stem_suffix();

    match (explicit_files.len(), &args.input_dir, args.batch) {
        // Explicit directory scan
        (0, Some(dir), _) | (_, Some(dir), _) => {
            if !explicit_files.is_empty() {
                bail!("Cannot combine explicit files with --input-dir");
            }
            let dir = dir
                .canonicalize()
                .with_context(|| format!("directory not found: {}", dir.display()))?;
            let queue = files::queue_from_directory(&dir, ext, exclude)?;
            let policy = resolve_explicit_policy(args.on_error);
            Ok((queue, policy))
        }
        // Explicit batch flag without files or input-dir -> scan CWD
        (0, None, true) => {
            let cwd = std::env::current_dir().context("reading current directory")?;
            let queue = files::queue_from_directory(&cwd, ext, exclude)?;
            let policy = resolve_explicit_policy(args.on_error);
            Ok((queue, policy))
        }
        // Single file provided, no batch flags -> sibling discovery
        (1, None, false) => {
            if args.on_error.is_some() {
                bail!("--on-error can only be used with --batch or --input-dir");
            }
            let file = explicit_files.into_iter().next().unwrap();
            resolve_single_file_with_siblings(task, &file)
        }
        // Multiple files provided, no batch flags
        (n, None, false) if n > 1 => {
            if args.on_error.is_some() {
                bail!("--on-error can only be used with --batch or --input-dir");
            }
            // Just use the provided files, default to skip on error for safety.
            let queue = filter_and_canonicalize(explicit_files, task.file_type_name());
            Ok((queue, BatchPolicy::SkipOnError))
        }
        // No files, no flags -> Fallback: scan CWD
        (0, None, false) => {
            let cwd = std::env::current_dir().context("reading current directory")?;
            let queue = files::queue_from_directory(&cwd, ext, exclude)?;
            let policy = resolve_explicit_policy(args.on_error);
            Ok((queue, policy))
        }
        _ => bail!("Invalid combination of batch arguments"),
    }
}

/// Resolves policy for explicit batch runs (--batch or --input-dir).
fn resolve_explicit_policy(on_error: Option<BatchOnError>) -> BatchPolicy {
    match on_error {
        Some(BatchOnError::Stop) => BatchPolicy::StopOnError,
        Some(BatchOnError::Skip) => BatchPolicy::SkipOnError,
        Some(BatchOnError::Prompt) => BatchPolicy::PromptEach,
        None => {
            if io::stdin().is_terminal() {
                BatchPolicy::PromptEach
            } else {
                BatchPolicy::SkipOnError
            }
        }
    }
}

/// Handles sibling discovery and prompting for a single explicit file.
fn resolve_single_file_with_siblings<T: BatchTask>(
    task: &T,
    file: &Path,
) -> Result<(Vec<PathBuf>, BatchPolicy)> {
    let canonical = file
        .canonicalize()
        .with_context(|| format!("file not found: {}", file.display()))?;

    let queue = files::queue_from_entry(
        &canonical,
        task.input_extension(),
        task.exclude_stem_suffix(),
    )?;

    if queue.len() <= 1 {
        return Ok((vec![canonical], BatchPolicy::Single));
    }

    let parent = canonical.parent().unwrap_or_else(|| Path::new("."));
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    let choice = prompt::sibling_batch_choice(&mut input, &mut stdout, parent, queue.len())?;

    let (policy, final_queue) = match choice {
        SiblingBatchChoice::ProcessInputOnly => (BatchPolicy::Single, vec![canonical]),
        SiblingBatchChoice::ProcessAllStopOnError => (BatchPolicy::StopOnError, queue),
        SiblingBatchChoice::ProcessAllSkipOnError => (BatchPolicy::SkipOnError, queue),
        SiblingBatchChoice::ProcessAllPromptEach => (BatchPolicy::PromptEach, queue),
    };

    Ok((final_queue, policy))
}

/// Filters explicit files, warning on invalid ones, and canonicalizes them.
fn filter_and_canonicalize(files: Vec<PathBuf>, type_name: &str) -> Vec<PathBuf> {
    files
        .into_iter()
        .filter_map(|p| match p.canonicalize() {
            Ok(canonical) => Some(canonical),
            Err(_) => {
                eprintln!(
                    "{WARNING_PREFIX}: skipping invalid {type_name} input: {}",
                    p.display()
                );
                None
            }
        })
        .collect()
}

/// Resolves the effective execution mode for the current policy.
fn normalize_execution_mode(mode: ExecutionMode, policy: BatchPolicy) -> ExecutionMode {
    if mode == ExecutionMode::Parallel && policy == BatchPolicy::PromptEach {
        println!("Parallel mode does not support per-file prompts; using sequential mode.");
        ExecutionMode::Sequential
    } else {
        mode
    }
}

/// Executes the resolved queue with the given policy.
fn execute_queue<R: ProcessRunner, T: BatchTask>(
    task: &T,
    args: &BatchArgs,
    queue: &[PathBuf],
    initial_policy: BatchPolicy,
    ffmpeg: &Ffmpeg<R>,
) -> Result<()> {
    let total = queue.len();
    let is_single = initial_policy == BatchPolicy::Single && total == 1;

    let mut policy = initial_policy;
    let mut report = BatchReport::new();

    for (index, input) in queue.iter().enumerate() {
        if total > 1 {
            println!("File {}/{}: {}", index + 1, total, input.display());
        }

        let output = match task.output_path(input, &args.output_dir) {
            Ok(path) => path,
            Err(error) => {
                report.record_failed(input.clone(), error.to_string());
                if policy == BatchPolicy::StopOnError {
                    report.print_summary();
                    bail!("stopped on output path error: {error}");
                }
                continue;
            }
        };

        let mut processed = false;

        if output::decision(&output, args.force) == OutputDecision::SkipExisting {
            println!("⏭ SKIPPED: {} already exists", output.display());
            report.record_skipped(input.clone(), "output exists");
        } else {
            processed = true;
            match task.process_file(input, &output, ffmpeg) {
                Ok(FileOutcome::Success) => {
                    println!("✔ SUCCESS: {}", output.display());
                    report.record_success(input.clone());
                }
                Ok(FileOutcome::Skipped(reason)) => {
                    println!("⏭ SKIPPED: {reason}");
                    report.record_skipped(input.clone(), reason);
                    processed = false; // Task decided to skip, do not prompt.
                }
                Err(error) => {
                    eprintln!("✖ FAILED: {}: {error}", input.display());
                    report.record_failed(input.clone(), error.to_string());

                    match policy {
                        BatchPolicy::Single | BatchPolicy::StopOnError => {
                            report.print_summary();
                            bail!("stopped after error on {}: {error}", input.display());
                        }
                        BatchPolicy::SkipOnError => {
                            eprintln!("Continuing past error...");
                        }
                        BatchPolicy::PromptEach => {
                            eprintln!("Error occurred.");
                        }
                    }
                }
            }
        }

        if processed && policy == BatchPolicy::PromptEach && index + 1 < total {
            let next = &queue[index + 1];
            let stdin = io::stdin();
            let mut input_stream = BufReader::new(stdin.lock());
            let mut stdout = io::stdout();

            match prompt::continue_to_next(&mut input_stream, &mut stdout, next)? {
                ContinueChoice::Yes => {}
                ContinueChoice::YesToAll => {
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
        bail!("one or more conversions failed")
    } else {
        Ok(())
    }
}

/// Executes the resolved queue in parallel mode.
fn execute_queue_parallel<T: BatchTask>(
    task: T,
    args: &BatchArgs,
    queue: Vec<PathBuf>,
    policy: BatchPolicy,
    ffmpeg_binary: &Path,
) -> Result<()> {
    let outputs = queue
        .iter()
        .map(|path| task.output_path(path, &args.output_dir))
        .collect::<Result<Vec<_>>>()?;

    let output_dir = args.output_dir.clone();
    let force = args.force;
    let binary = ffmpeg_binary.to_path_buf();
    let failure_policy = match policy {
        BatchPolicy::StopOnError => FailurePolicy::CancelRemaining,
        BatchPolicy::Single | BatchPolicy::SkipOnError | BatchPolicy::PromptEach => {
            FailurePolicy::Continue
        }
    };

    let Worker_task = task.clone();
    let Worker = move |input: PathBuf, cancel: CancellationToken| -> BatchFuture {
        let task = Worker_task.clone();
        let output_dir = output_dir.clone();
        let binary = binary.clone();
        Box::pin(async move {
            let output = task.output_path(&input, &output_dir)?;
            if matches!(
                output::decision(&output, force),
                OutputDecision::SkipExisting
            ) {
                return Ok(output);
            }
            task.process_file_async(input, output, binary, cancel).await
        })
    };

    drive_parallel_batch(
        &queue,
        outputs,
        failure_policy,
        || String::new(),
        || final_error_message("one or more conversions failed"),
        Worker,
    )
}

/// Runs the shared synchronous event loop around the async parallel runner.
fn drive_parallel_batch<F, Fut, B, E>(
    queue: &[PathBuf],
    outputs: Vec<PathBuf>,
    failure_policy: FailurePolicy,
    mut banner_message: B,
    mut error_message: E,
    Worker: F,
) -> Result<()>
where
    F: Fn(PathBuf, CancellationToken) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<PathBuf>> + Send + 'static,
    B: FnMut() -> String,
    E: FnMut() -> String,
{
    let banner = banner_message();
    if !banner.is_empty() {
        println!("{banner}");
    }

    let cancel = CancellationToken::new();
    let cancel_for_signal = cancel.clone();

    // Ctrl+C handler — first Ctrl+C cancels gracefully.
    let _ctrl_c_handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("ctrl-c runtime");
        runtime.block_on(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\nCancellation requested (press Ctrl+C again to force exit)...");
            cancel_for_signal.cancel();
        });
    });

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();
    let queue_for_runner = queue.to_vec();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let runner_handle = runtime.spawn(parallel::run_parallel(
        queue_for_runner,
        parallel::num_cpus(),
        cancel.clone(),
        failure_policy,
        event_tx,
        Worker,
    ));

    let mut started = std::collections::HashSet::new();
    let mut succeeded = std::collections::HashSet::new();
    let mut summary = BatchSummary::default();

    runtime.block_on(async {
        while let Some(event) = event_rx.recv().await {
            match event {
                BatchEvent::Started(index) => {
                    started.insert(index);
                    println!("▶ Started [{}]: {}", index + 1, queue[index].display());
                }
                BatchEvent::Done(index, result) => match result {
                    WorkResult::Success(path) => {
                        println!("✔ Success [{}]: {}", index + 1, path.display());
                        succeeded.insert(index);
                    }
                    WorkResult::Failed(error) => {
                        eprintln!("✖ Failed [{}]: {error}", index + 1);
                    }
                    WorkResult::Cancelled => {
                        eprintln!("⊗ Cancelled [{}]: {}", index + 1, queue[index].display());
                    }
                },
                BatchEvent::AllDone(batch_summary) => {
                    summary = batch_summary;
                    break;
                }
            }
        }
    });

    let _ = runtime.block_on(runner_handle)?;

    if summary.stopped_early() {
        prompt_for_residual_cleanup(&outputs, &started, &succeeded)?;
    }

    println!(
        "{}",
        banner::render(
            "Done",
            Some(&format!(
                "{} succeeded · {} failed{}",
                summary.succeeded,
                summary.failed,
                final_summary_suffix(summary)
            )),
            console::colors_enabled()
        )
    );

    if summary.failed > 0 {
        bail!("{}", error_message());
    }

    Ok(())
}

/// Prompts for cleanup of partially written outputs.
fn prompt_for_residual_cleanup(
    outputs: &[PathBuf],
    started: &std::collections::HashSet<usize>,
    succeeded: &std::collections::HashSet<usize>,
) -> Result<()> {
    let residual: Vec<PathBuf> = started
        .iter()
        .filter(|index| !succeeded.contains(index))
        .filter_map(|index| outputs.get(*index).filter(|path| path.exists()).cloned())
        .collect();

    if residual.is_empty() {
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    match prompt::cleanup_residual_choice(&mut input, &mut stdout, &residual)? {
        CleanupChoice::Remove => {
            for path in &residual {
                let _ = std::fs::remove_file(path);
            }
            println!("Removed {} residual files.", residual.len());
        }
        CleanupChoice::Keep => {
            println!("Kept {} residual files.", residual.len());
        }
    }

    Ok(())
}

/// Returns the final status suffix for the batch summary banner.
fn final_summary_suffix(summary: BatchSummary) -> &'static str {
    if summary.was_cancelled() {
        " · cancelled"
    } else if summary.stopped_on_error() {
        " · stopped on error"
    } else {
        ""
    }
}

/// Builds the parallel-run banner.
fn runner_banner(title: &str, file_count: usize) -> String {
    banner::render(
        title,
        Some(&format!("Parallel Batch · {} files", file_count)),
        console::colors_enabled(),
    )
}

/// Returns the final error message for failed batches.
fn final_error_message(message: &str) -> String {
    message.to_string()
}
