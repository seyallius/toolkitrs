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
        parallel::{self, BatchEvent, FileJob, WorkOutcome},
    },
};
use anyhow::{bail, Context, Result};
use std::{
    io::{self, BufReader, IsTerminal},
    path::{Path, PathBuf},
    sync::Arc,
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

/// Resolves the execution queue and batch policy from CLI arguments and interactive prompts.
///
/// This is the single resolution pass shared by every batch command: it
/// canonicalizes explicit files, discovers siblings, prompts when needed,
/// and applies the `--on-error` policy. Commands call it once and dispatch
/// on the result, so prompts never fire twice.
pub fn resolve_queue<T: BatchTask>(
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
            resolve_directory_queue(dir, ext, exclude, args.on_error)
        }
        // No explicit files (batch flag or bare invocation): scan the CWD
        (0, None, _) => {
            let cwd = std::env::current_dir().context("reading current directory")?;
            resolve_directory_queue(&cwd, ext, exclude, args.on_error)
        }
        // Single file provided, no batch flags -> sibling discovery
        (1, None, false) => {
            if args.on_error.is_some() {
                bail!("--on-error can only be used with --batch or --input-dir");
            }
            let file = explicit_files.into_iter().next().unwrap();
            resolve_single_file_with_siblings(ext, exclude, &file)
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
        // Files combined with the batch flag
        _ => bail!("Invalid combination of batch arguments"),
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

/// Runs a batch sequentially with interactive skip/overwrite decisions and error policies.
///
/// `queue` and `policy` come from [`resolve_queue`]; this function only
/// executes them, so the queue is resolved exactly once per run.
pub fn run_sequential<R: ProcessRunner, T: BatchTask>(
    task: &T,
    args: &BatchArgs,
    queue: Vec<PathBuf>,
    policy: BatchPolicy,
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

    if queue.is_empty() {
        println!("No {} files found to process.", task.file_type_name());
        return Ok(());
    }

    output::ensure_directory(&args.output_dir)?;

    execute_queue(task, args, &queue, policy, ffmpeg)
}

/// Runs a batch in parallel with live console reporting and Ctrl+C support.
///
/// This is shared by every workflow: the caller passes a [`FileJob`] that
/// knows how to convert one file for its specific workflow.
///
/// # Arguments
/// * `banner_title` - Title for the banners (e.g. "TS").
/// * `queue` - Input file paths, in order.
/// * `job` - Strategy that converts one input file.
pub fn run_parallel_console(
    banner_title: &str,
    queue: Vec<PathBuf>,
    job: Arc<dyn FileJob>,
) -> Result<()> {
    println!(
        "{}",
        banner::render(
            banner_title,
            Some(&format!("Parallel Batch · {} files", queue.len())),
            console::colors_enabled()
        )
    );

    let names = queue.clone();
    let run = parallel::run_blocking(
        queue,
        parallel::num_cpus(),
        CancellationToken::new(),
        job,
        |event| report_console(&names, event),
    )?;

    if run.summary.cancelled {
        prompt_residual_cleanup(&run.residual_files)?;
    }

    println!(
        "{}",
        banner::render(
            "Done",
            Some(&format!(
                "{} succeeded · {} failed{}",
                run.summary.succeeded,
                run.summary.failed,
                if run.summary.cancelled {
                    " · cancelled"
                } else {
                    ""
                }
            )),
            console::colors_enabled()
        )
    );

    if run.summary.failed > 0 {
        bail!("one or more conversions failed");
    }
    Ok(())
}

/// Prints the banner and a "nothing found" notice, then returns success.
///
/// Used by commands that resolved an empty queue and have nothing to do.
pub fn report_empty_queue(file_type_name: &str) -> Result<()> {
    println!(
        "{}",
        banner::render(
            file_type_name,
            Some("Batch Processing"),
            console::colors_enabled()
        )
    );
    println!("No {file_type_name} files found to process.");
    Ok(())
}

/// Scans a directory for a batch and applies the error policy.
pub fn resolve_directory_queue(
    directory: &Path,
    input_extension: &str,
    exclude_stem_suffix: Option<&str>,
    on_error: Option<BatchOnError>,
) -> Result<(Vec<PathBuf>, BatchPolicy)> {
    let dir = directory
        .canonicalize()
        .with_context(|| format!("directory not found: {}", directory.display()))?;
    let queue = files::queue_from_directory(&dir, input_extension, exclude_stem_suffix)?;
    Ok((queue, resolve_explicit_policy(on_error)))
}

/// Handles sibling discovery and prompting for a single explicit file.
///
/// If additional files with the same extension are discovered next to it,
/// the user is asked whether to expand the operation into a batch.
pub fn resolve_single_file_with_siblings(
    input_extension: &str,
    exclude_stem_suffix: Option<&str>,
    file: &Path,
) -> Result<(Vec<PathBuf>, BatchPolicy)> {
    let canonical = file
        .canonicalize()
        .with_context(|| format!("file not found: {}", file.display()))?;

    let queue = files::queue_from_entry(&canonical, input_extension, exclude_stem_suffix)?;

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

// ----------------------------------------- Internal Helpers --------------------------------------- //

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

/// Prints one batch event to the console.
fn report_console(names: &[PathBuf], event: BatchEvent) {
    match event {
        BatchEvent::Started(index) => {
            println!("▶ Started [{}]: {}", index + 1, names[index].display())
        }
        BatchEvent::Done(index, WorkOutcome::Success(path)) => {
            println!("✔ Success [{}]: {}", index + 1, path.display())
        }
        BatchEvent::Done(index, WorkOutcome::Failed(error)) => {
            eprintln!("✖ Failed [{}]: {}", index + 1, error)
        }
        BatchEvent::Done(index, WorkOutcome::Cancelled) => {
            eprintln!("⊗ Cancelled [{}]: {}", index + 1, names[index].display())
        }
        BatchEvent::AllDone(_) => {}
    }
}

/// Asks whether leftover partial outputs should be deleted, then acts on it.
fn prompt_residual_cleanup(residual: &[PathBuf]) -> Result<()> {
    if residual.is_empty() {
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    match prompt::cleanup_residual_choice(&mut input, &mut stdout, residual)? {
        CleanupChoice::Remove => {
            for path in residual {
                let _ = std::fs::remove_file(path);
            }
            println!("Removed {} residual files.", residual.len());
        }
        CleanupChoice::Keep => println!("Kept {} residual files.", residual.len()),
    }
    Ok(())
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
