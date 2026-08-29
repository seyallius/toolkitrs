//! module processor - Concurrent processing of repositories using tokio tasks.
//!
//! Implements a bounded-concurrency worker pool pattern using `tokio::sync::Semaphore`
//! to limit parallel API calls, matching the Go worker pool behavior.

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
pub async fn process_repositories(
    config: &GhContribConfig,
    repos: &[Repository],
    file_writer: &Arc<SafeFileWriter>,
    cancel: CancellationToken,
) -> (usize, usize) {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let cfg = Arc::new(config.clone());

    let mut handles = Vec::with_capacity(repos.len());

    for repo in repos {
        // ✅ Stop spawning new tasks if canceled
        if cancel.is_cancelled() {
            break;
        }

        let permit = semaphore.clone();
        let writer = file_writer.clone();
        let cfg = cfg.clone();
        let repo_name = repo.name.clone();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            let _permit = permit.acquire().await.expect("semaphore closed");
            if cancel_clone.is_cancelled() {
                return None;
            }
            process_single_repo(&cfg, &repo_name, &writer, cancel_clone).await
        });
        handles.push(handle);
    }

    // Collect results
    let mut total_commits = 0usize;
    let mut repo_count = 0usize;

    for handle in handles {
        match handle.await {
            Ok(Some(commit_count)) => {
                total_commits += commit_count;
                repo_count += 1;
            }
            Ok(None) => {} // No commits for this repo or canceled
            Err(e) => {
                if !cancel.is_cancelled() {
                    eprintln!("Task join error: {e}");
                }
            }
        }
    }

    (total_commits, repo_count)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Processes a single repository: fetches commits, filters them, optionally
/// fetches README, and writes to the file.
async fn process_single_repo(
    config: &GhContribConfig,
    repo_name: &str,
    file_writer: &SafeFileWriter,
    cancel: CancellationToken,
) -> Option<usize> {
    if cancel.is_cancelled() {
        return None;
    }

    eprintln!("Fetching commits for {repo_name}...");

    // ✅ Fetch commits with instant cancellation support
    let commits = tokio::select! {
        _ = cancel.cancelled() => return None,
        res = api::fetch_commits(config, repo_name) => match res {
            Ok(commits) => commits,
            Err(e) => {
                if !cancel.is_cancelled() {
                    eprintln!("Error fetching {repo_name}: {e}");
                }
                return None;
            }
        }
    };

    let filtered = filter::filter_commits(&commits);
    if filtered.is_empty() || cancel.is_cancelled() {
        return None;
    }

    // ✅ Fetch README if configured, with cancellation support
    let readme_content = if config.fetch_readme {
        tokio::select! {
            _ = cancel.cancelled() => None,
            res = api::fetch_readme(config, repo_name) => match res {
                Ok(content) => Some(content),
                Err(e) => {
                    if !cancel.is_cancelled() {
                        eprintln!("  Warning: Could not fetch README for {repo_name}: {e}");
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

    // Write directly to file (synchronized internally)
    file_writer.write_repo(
        &config.username,
        repo_name,
        &filtered,
        readme_content.as_deref(),
    );

    eprintln!("  Written {commit_count} commits for {repo_name}");
    Some(commit_count)
}
