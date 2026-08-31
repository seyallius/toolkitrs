//! module processor - Concurrent processing of repositories using tokio tasks.
//!
//! Implements a bounded-concurrency worker pool pattern using `tokio::sync::Semaphore`
//! to limit parallel API calls, matching the Go worker pool behavior.
//!
//! Progress is reported through an injected logger closure instead of writing
//! to stderr directly: the TUI owns the terminal while a batch runs, so any
//! direct `eprintln!` would corrupt the rendered frame. The CLI passes a
//! closure that forwards to stderr; the TUI forwards into its event channel.

use crate::github::{api, filter, types::*, writer::SafeFileWriter};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Semaphore permits limiting concurrent API calls.
/// Matches the Go `MaxWorkers = 10` constant.
const MAX_CONCURRENT: usize = MAX_WORKERS;

// ----------------------------------------- Public API ----------------------------------------- //

/// Orchestrates concurrent processing of repositories where each task writes
/// directly to the file using a synchronized writer.
///
/// This approach avoids storing all results in memory, matching the Go
/// `ProcessRepositoriesWithDirectWrite` behavior.
///
/// # Arguments
/// * `config` - Validated gh-contrib configuration.
/// * `repos` - Repositories to scan.
/// * `file_writer` - Shared, mutex-guarded output writer.
/// * `cancel` - Cancellation token for graceful shutdown.
/// * `log` - Progress sink. Human-readable status lines are sent here; this
///   module never writes to the terminal itself.
pub async fn process_repositories<L>(
    config: &GhContribConfig,
    repos: &[Repository],
    file_writer: &Arc<SafeFileWriter>,
    cancel: CancellationToken,
    log: L,
) -> (usize, usize)
where
    L: Fn(&str) + Clone + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let cfg = Arc::new(config.clone());
    let mut handles = Vec::with_capacity(repos.len());

    for repo in repos {
        // ⏹ Stop spawning new tasks if canceled.
        if cancel.is_cancelled() {
            break;
        }
        let permit = semaphore.clone();
        let writer = file_writer.clone();
        let cfg = cfg.clone();
        let repo_name = repo.name.clone();
        let cancel_clone = cancel.clone();
        let log = log.clone();
        let handle = tokio::spawn(async move {
            let _permit = permit.acquire().await.expect("semaphore closed");
            if cancel_clone.is_cancelled() {
                return None;
            }
            process_single_repo(&cfg, &repo_name, &writer, cancel_clone, log).await
        });
        handles.push(handle);
    }

    // Collect results.
    let mut total_commits = 0usize;
    let mut repo_count = 0usize;
    for handle in handles {
        match handle.await {
            Ok(Some(commit_count)) => {
                total_commits += commit_count;
                repo_count += 1;
            }
            Ok(None) => {} // No commits for this repo or cancelled.
            Err(e) => {
                if !cancel.is_cancelled() {
                    log(&format!("Task join error: {e}"));
                }
            }
        }
    }
    (total_commits, repo_count)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Processes a single repository: fetches commits, filters them, optionally
/// fetches README, and writes to the file.
///
/// All progress reporting goes through `log`; nothing here touches the
/// terminal directly.
async fn process_single_repo<L>(
    config: &GhContribConfig,
    repo_name: &str,
    file_writer: &SafeFileWriter,
    cancel: CancellationToken,
    log: L,
) -> Option<usize>
where
    L: Fn(&str),
{
    if cancel.is_cancelled() {
        return None;
    }
    log(&format!("Fetching commits for {repo_name}..."));

    // 📥 Fetch commits with instant cancellation support.
    let commits = tokio::select! {
        _ = cancel.cancelled() => return None,
        res = api::fetch_commits(config, repo_name) => match res {
            Ok(commits) => commits,
            Err(e) => {
                if !cancel.is_cancelled() {
                    log(&format!("Error fetching {repo_name}: {e}"));
                }
                return None;
            }
        }
    };

    let filtered = filter::filter_commits(&commits);
    if filtered.is_empty() || cancel.is_cancelled() {
        return None;
    }

    // Fetch README if configured, with cancellation support.
    let readme_content = if config.fetch_readme {
        tokio::select! {
            _ = cancel.cancelled() => None,
            res = api::fetch_readme(config, repo_name) => match res {
                Ok(content) => Some(content),
                Err(e) => {
                    if !cancel.is_cancelled() {
                        log(&format!(
                            "Warning: could not fetch README for {repo_name}: {e}"
                        ));
                    }
                    None
                }
            }
        }
    } else {
        None
    };

    if cancel.is_cancelled() {
        return None;
    }

    let commit_count = filtered.len();
    // Write directly to file (synchronized internally).
    file_writer.write_repo(
        &config.username,
        repo_name,
        &filtered,
        readme_content.as_deref(),
    );
    log(&format!("Written {commit_count} commits for {repo_name}"));
    Some(commit_count)
}
