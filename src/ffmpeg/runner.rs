//! module runner - FFmpeg process execution with a trait for testability.
//! Provides structured output and error types for deterministic command testing
//! and clean error propagation throughout the CLI pipeline.

use anyhow::{bail, Context, Result};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

// -------------------------------------------- Types ------------------------------------------- //

/// Captured output from a completed child process.
/// Used by both real and fake runners to provide consistent execution results.
#[derive(Debug, Clone)]
pub struct ProcessOutput {
    /// Whether the process exited with a success status code (typically 0).
    pub success: bool,

    /// The raw OS exit code, if available.
    /// May be `None` if the process was terminated by a signal on Unix systems.
    pub code: Option<i32>,

    /// Complete standard output captured from the child process.
    /// Empty string if no stdout was produced or capture failed.
    pub stdout: String,

    /// Complete standard error captured from the child process.
    /// Contains FFmpeg diagnostic logs and error messages.
    pub stderr: String,
}

/// Structured error returned when an FFmpeg command fails.
/// Implements [std::fmt::Display] via thiserror for user-friendly error messages.
#[derive(Debug, Error)]
#[error("ffmpeg failed (exit {code:?}): {stderr}\ncommand: {command}")]
pub struct ProcessError {
    /// The full command string that was attempted, including binary path and all arguments.
    /// Useful for reproducing failures manually in a terminal.
    pub command: String,

    /// The exit code from the failed process.
    /// Helps distinguish between different failure modes (e.g., 1 = general error, 255 = missing file).
    pub code: Option<i32>,

    /// Standard output captured before or during the failure.
    /// Sometimes contains partial progress or warnings even on failure.
    pub stdout: String,

    /// Standard error containing the actual FFmpeg error diagnostics.
    /// This is typically where the root cause explanation lives.
    pub stderr: String,
}

/// Minimal seam around process execution, allowing deterministic command tests.
/// Implement this trait to inject fake behavior without touching the filesystem.
pub trait ProcessRunner {
    /// Executes a binary with the given arguments and returns structured output.
    fn run(&self, binary: &str, args: &[String]) -> Result<ProcessOutput>;
}

/// Real implementation using [std::process::Command].
/// Blocks the current thread until the child process completes.
pub struct RealRunner;
impl ProcessRunner for RealRunner {
    fn run(&self, binary: &str, args: &[String]) -> Result<ProcessOutput> {
        let output = Command::new(binary).args(args).output()?;
        Ok(ProcessOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Configured FFmpeg facade used by commands.
/// Encapsulates binary path resolution, verbose logging, and runner injection.
pub struct Ffmpeg<R> {
    /// Path to the ffmpeg executable. Defaults to "ffmpeg" if not explicitly provided.
    binary: PathBuf,

    /// When true, prints the full command to stderr before execution
    /// and includes stdout in error output for debugging.
    verbose: bool,

    /// Injectable process runner for testing and alternative execution strategies.
    runner: R,
}
impl<R: ProcessRunner> Ffmpeg<R> {
    /// Creates a new `Ffmpeg` instance with the specified configuration.
    ///
    /// # Arguments
    /// * `binary` - Optional path to the ffmpeg executable; defaults to "ffmpeg".
    /// * `verbose` - Whether to print commands and diagnostic output.
    /// * `runner` - The process runner implementation to use.
    pub fn new(binary: Option<PathBuf>, verbose: bool, runner: R) -> Self {
        Self {
            binary: binary.unwrap_or_else(|| PathBuf::from("ffmpeg")),
            verbose,
            runner,
        }
    }

    /// Runs the FFmpeg command with the given arguments.
    ///
    /// # Errors
    /// Returns a `ProcessError` wrapped in `anyhow::Error` if the command fails.
    pub fn run(&self, args: Vec<String>) -> Result<()> {
        let binary = self.binary.to_string_lossy();
        if self.verbose {
            eprintln!("[ffmpeg] {binary} {}", args.join(" "));
        }
        let output = self.runner.run(&binary, &args)?;
        if output.success {
            Ok(())
        } else {
            if self.verbose && !output.stdout.is_empty() {
                eprintln!("{}", output.stdout);
            }
            bail!(ProcessError {
                command: format!("{binary} {}", args.join(" ")),
                code: output.code,
                stdout: output.stdout,
                stderr: output.stderr
            });
        }
    }
}
impl<R> Ffmpeg<R> {
    /// Returns the ffmpeg binary path that this facade will invoke.
    pub fn binary(&self) -> &Path {
        &self.binary
    }
}

/// Runs an FFmpeg command asynchronously, with cancellation support.
///
/// This is the cancellable counterpart to [`Ffmpeg::run`]. It is used by the
/// parallel batch executor so long-running ffmpeg processes can be killed
/// gracefully when the user requests cancellation.
///
/// # Arguments
/// * `binary` - Path to the ffmpeg executable.
/// * `args` - FFmpeg arguments (typically built via [`crate::ffmpeg::args`]).
/// * `cancel` - Cancellation token. When triggered, the child process is killed.
/// * `log_tx` - Optional channel to receive each line of ffmpeg's stderr.
///   Used by the TUI to show live logs.
///
/// # Errors
/// - If the process cannot be spawned.
/// - If `cancel` fires: returns an error with the message `"canceled"`.
/// - If the process exits with a non-zero status.
pub async fn run_async(
    binary: &Path,
    args: Vec<String>,
    cancel: CancellationToken,
    log_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
) -> Result<()> {
    // Only pipe stderr if we have a channel to read it. Otherwise, null it
    // to prevent SIGPIPE crashes when ffmpeg writes to a closed pipe.
    let stderr_cfg = if log_tx.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };

    let mut child = tokio::process::Command::new(binary)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(stderr_cfg)
        // Kill the child if this future is dropped (e.g., the task is aborted).
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning {} {}", binary.display(), args.join(" ")))?;

    // Optional: stream stderr line-by-line to the log channel.
    let log_task = if let (Some(stderr), Some(tx)) = (child.stderr.take(), log_tx) {
        Some(tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(line);
            }
        }))
    } else {
        None
    };

    // Wait for the process to finish — or for cancellation to fire.
    let result = tokio::select! {
        r = child.wait() => r,
        _ = cancel.cancelled() => {
            // Kill the child to free up system resources.
            let _ = child.kill().await;
            if let Some(task) = log_task {
                let _ = task.await;
            }
            bail!("cancelled");
        }
    };

    if let Some(task) = log_task {
        let _ = task.await;
    }

    match result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!("ffmpeg exited with {status}"),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl ProcessRunner for Fake {
        fn run(&self, _: &str, _: &[String]) -> Result<ProcessOutput> {
            Ok(ProcessOutput {
                success: false,
                code: Some(2),
                stdout: String::new(),
                stderr: "bad input".into(),
            })
        }
    }

    #[test]
    fn error_has_stderr() {
        let error = Ffmpeg::new(None, false, Fake)
            .run(vec!["-i".into(), "x".into()])
            .unwrap_err();
        assert!(error.to_string().contains("bad input"));
    }
}
