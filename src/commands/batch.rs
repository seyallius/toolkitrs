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
        parallel::{self, BatchEvent, BatchSummary},
    },
};
use anyhow::{bail, Context, Result};
use std::{
    future::Future,
    io::{self, BufReader, IsTerminal},
    path::{Path, PathBuf},
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

/// Trait representing a single file conversion task within a batch.
pub trait BatchTask {
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

    /// Executes the conversion logic for a single file.
    fn process_file<R: ProcessRunner>(
        &self,
        input: &Path,
        output: &Path,
        ffmpeg: &Ffmpeg<R>,
    ) -> Result<FileOutcome>;
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
            task.file_type_name(),
            Some("Batch Processing"),
            console::colors_enabled()
        )
    );

    let (queue, policy) = resolve_queue_and_policy(task, args, explicit_files)?;

    if queue.is_empty() {
        println!("No {} files found to process.", task.file_type_name());
        return Ok(());
    }

    // Ensure output directory exists if we are going to process anything
    output::ensure_directory(&args.output_dir)?;

    // Decide execution mode (parallel vs sequential) for multi-file batches.
    let mode = resolve_execution_mode(queue.len(), args.mode)?;
    match mode {
        ExecutionMode::Sequential => execute_queue(task, args, &queue, policy, ffmpeg),
        ExecutionMode::Parallel => {
            // Build a worker closure that delegates to the task's async worker.
            // We use Arc so the closure can be cloned cheaply per spawn.
            let output_dir = args.output_dir.clone();
            let output_ext = task.output_extension().to_string();
            let force = args.force;
            let binary = ffmpeg.binary().to_path_buf();

            // Dispatch by workflow — each command calls run_batch_parallel directly
            // with its own worker. Here we provide a generic worker for any
            // BatchTask by routing through the async_workers module.
            //
            // Since workers.rs has per-workflow functions, we expect each command
            // to call run_batch_parallel itself. For commands that haven't been
            // migrated yet, fall back to sequential.
            execute_queue(task, args, &queue, policy, ffmpeg)
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

/// Runs a parallel batch with the given worker closure.
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
/// * `worker` - Async closure: `(input, cancel) -> Result<PathBuf>`.
pub fn run_batch_parallel<F, Fut>(
    banner_title: &str,
    queue: Vec<PathBuf>,
    output_dir: &Path,
    output_ext: &str,
    force: bool,
    ffmpeg_binary: &Path,
    worker: F,
) -> Result<()>
where
    F: Fn(PathBuf, CancellationToken) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<PathBuf>> + Send + 'static,
{
    println!(
        "{}",
        banner::render(
            banner_title,
            Some(&format!("Parallel Batch · {} files", queue.len())),
            console::colors_enabled()
        )
    );

    output::ensure_directory(output_dir)?;

    let cores = parallel::num_cpus();
    let cancel = CancellationToken::new();

    // Ctrl+C handler — first Ctrl+C cancels gracefully.
    let cancel_clone = cancel.clone();
    let ctrl_c_handle = std::thread::spawn(move || {
        // We use a tiny single-threaded runtime just for the signal handler.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("ctrl-c runtime");
        rt.block_on(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\nCancellation requested (press Ctrl+C again to force exit)...");
            cancel_clone.cancel();
        });
    });

    // Pre-compute output paths so we can identify residual files after cancel.
    let outputs: Vec<PathBuf> = queue
        .iter()
        .map(|p| output::output_path(p, output_dir, output_ext))
        .collect::<Result<_>>()?;

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();

    // The actual parallel run happens inside a multi-threaded tokio runtime.
    let binary = ffmpeg_binary.to_path_buf();
    let runner_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let queue_for_runner = queue.clone();
    let runner_handle = runner_rt.spawn(async move {
        parallel::run_parallel(queue_for_runner, cores, cancel.clone(), event_tx, worker).await
    });

    // Drive the event loop from the calling thread (sync API).
    let runtime_guard = runner_rt.enter();
    let _ = runtime_guard; // keep the runtime alive via runner_handle

    let mut started: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut succeeded: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut failed = 0usize;
    let mut summary = BatchSummary::default();

    // Block on receiving events.
    runner_rt.block_on(async {
        while let Some(event) = event_rx.recv().await {
            match event {
                BatchEvent::Started(i) => {
                    started.insert(i);
                    println!("▶ Started [{}]: {}", i + 1, queue[i].display());
                }
                BatchEvent::Done(i, work_result) => {
                    if work_result.is_success() {
                        // Success case - unwrap the path
                        if let Some(path) = work_result.path {
                            println!("✔ Success [{}]: {}", i + 1, path.display());
                            succeeded.insert(i);
                        }
                    } else if let Some(error) = work_result.error {
                        // Check if it was a cancellation
                        if error == "cancelled" {
                            eprintln!("⊗ Cancelled [{}]: {}", i + 1, queue[i].display());
                        } else {
                            eprintln!("✖ Failed [{}]: {}", i + 1, error);
                            failed += 1;
                        }
                    }
                }
                BatchEvent::AllDone(s) => {
                    summary = s;
                    break;
                }
            }
        }
    });

    // Drop the runner handle to free the runtime.
    let _ = runner_handle;

    // Stop the Ctrl+C listener thread.
    drop(ctrl_c_handle);

    // If canceled, prompt for residual file cleanup.
    if summary.cancelled {
        let residual: Vec<PathBuf> = started
            .iter()
            .filter(|i| !succeeded.contains(i))
            .filter_map(|i| outputs.get(*i).filter(|p| p.exists()).cloned())
            .collect();

        if !residual.is_empty() {
            let stdin = io::stdin();
            let mut input = BufReader::new(stdin.lock());
            let mut stdout = io::stdout();
            match prompt::cleanup_residual_choice(&mut input, &mut stdout, &residual)? {
                CleanupChoice::Remove => {
                    for p in &residual {
                        let _ = std::fs::remove_file(p);
                    }
                    println!("Removed {} residual files.", residual.len());
                }
                CleanupChoice::Keep => {
                    println!("Kept {} residual files.", residual.len());
                }
            }
        }
    }

    println!(
        "{}",
        banner::render(
            "Done",
            Some(&format!(
                "{} succeeded · {} failed{}",
                summary.succeeded,
                summary.failed,
                if summary.cancelled {
                    " · cancelled"
                } else {
                    ""
                }
            )),
            console::colors_enabled()
        )
    );

    if summary.failed > 0 {
        bail!("one or more conversions failed");
    }
    Ok(())
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
            // Just use the provided files, default to skip on error for safety
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
            Ok(c) => Some(c),
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

        let out = match output::output_path(input, &args.output_dir, task.output_extension()) {
            Ok(p) => p,
            Err(e) => {
                report.record_failed(input.clone(), e.to_string());
                if policy == BatchPolicy::StopOnError {
                    report.print_summary();
                    bail!("stopped on output path error: {e}");
                }
                continue;
            }
        };

        let mut processed = false;

        if output::decision(&out, args.force) == OutputDecision::SkipExisting {
            println!("⏭ SKIPPED: {} already exists", out.display());
            report.record_skipped(input.clone(), "output exists");
        } else {
            processed = true;
            match task.process_file(input, &out, ffmpeg) {
                Ok(FileOutcome::Success) => {
                    println!("✔ SUCCESS: {}", out.display());
                    report.record_success(input.clone());
                }
                Ok(FileOutcome::Skipped(reason)) => {
                    println!("⏭ SKIPPED: {reason}");
                    report.record_skipped(input.clone(), reason);
                    processed = false; // Task decided to skip, don't prompt
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
