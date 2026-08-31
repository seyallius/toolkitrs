//! module gh_contrib - Fetch and export GitHub contributions for a specified user.
//!
//! Implements the `gh-contrib` subcommand: fetches commits from all repositories
//! of the authenticated user within a date range, filters merge/revert commits,
//! and writes formatted output to a text file.

use crate::{
    components::banner,
    github::{api, config, processor, types, writer::SafeFileWriter},
};
use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;

// ---------------------------------------------- Types ----------------------------------------- //

/// Arguments for the `gh-contrib` subcommand.
///
/// Fetches GitHub commits for a user within a date range and exports them
/// to a formatted text file. Dates accept `YYYY-MM-DD`, `today`, or `yesterday`.
#[derive(Debug, Args)]
#[command(after_help = "Examples:
  toolkitrs gh-contrib --username seyallius --since 2026-01-01 --until 2026-08-27
  toolkitrs gh-contrib -u seyallius -s yesterday -t today
  toolkitrs gh-contrib -u seyallius -s 2026-01-01 -t 2026-08-27 --no-readme
  toolkitrs gh-contrib -u seyallius -s 2026-01-01 -t 2026-08-27 -o ./output.txt")]
pub struct GhContribArgs {
    /// GitHub username (required).
    #[arg(short, long)]
    pub username: String,

    /// Start date: YYYY-MM-DD, 'today' or 'yesterday' (required).
    #[arg(short, long)]
    pub since: String,

    /// End date: YYYY-MM-DD, 'today' or 'yesterday' (required).
    #[arg(short = 't', long)]
    pub until: String,

    /// Skip fetching README files for repositories.
    #[arg(long, default_value_t = false)]
    pub no_readme: bool,

    /// Custom output file path. Defaults to `contributions_<user>_<since>_to_<until>.txt`.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs the gh-contrib workflow: fetch repos → process concurrently → write output.
///
/// # Arguments
/// * `args_cli` - Parsed CLI arguments for the gh-contrib command.
///
/// # Errors
/// Returns an error if configuration is invalid or API calls fail fatally.
pub fn run(args_cli: GhContribArgs) -> Result<()> {
    println!(
        "{}",
        banner::render(
            "gh-contrib",
            Some("GitHub Contributions Exporter"),
            console::colors_enabled()
        )
    );

    // Create and validate configuration
    let cfg = config::new_config(
        &args_cli.username,
        &args_cli.since,
        &args_cli.until,
        !args_cli.no_readme, // fetch_readme is the inverse of --no-readme
    )?;

    // Run the async workflow
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("creating tokio runtime")?;

    runtime.block_on(async { run_async(cfg, args_cli.output).await })
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Async implementation of the gh-contrib workflow.
/// Async implementation of the gh-contrib workflow.
async fn run_async(cfg: types::GhContribConfig, output_path: Option<PathBuf>) -> Result<()> {
    // Fetch repositories
    let repos = api::fetch_repositories(&cfg)
        .await
        .context("fetching repositories")?;
    eprintln!("Found {} repositories", repos.len());

    // Determine output filename
    let output_file = output_path.unwrap_or_else(|| {
        PathBuf::from(types::output_filename(
            &cfg.username,
            &cfg.since,
            &cfg.until,
        ))
    });

    // Create the thread-safe file writer, shared across tasks via Arc
    let file_writer = std::sync::Arc::new(SafeFileWriter::new(&output_file)?);

    // Write header once
    file_writer.write_header(&cfg.username, &cfg.since, &cfg.until);

    // ✅ Set up cancellation for Ctrl+C
    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let ctrl_c_handle = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        eprintln!("\n⚠️ Cancellation requested (press Ctrl+C again to force exit)...");
        cancel_clone.cancel();
    });

    // Process repositories concurrently with direct file writing.
    // The CLI prints progress to stderr through the injected logger; the TUI
    // passes an event-channel logger instead (see github::tui).
    let (total_commits, repo_count) = processor::process_repositories(
        &cfg,
        &repos,
        &file_writer,
        cancel.clone(),
        |line: &str| eprintln!("{line}"),
    )
    .await;

    // Abort the ctrl-c listener since we're done
    ctrl_c_handle.abort();

    if cancel.is_cancelled() {
        eprintln!("\n⚠️ Cancelled by user. Output may be incomplete.");
        eprintln!("Partial output written to: {}", output_file.display());
        return Ok(());
    }

    // Write summary
    file_writer.write_summary(total_commits, repo_count);

    eprintln!("\nDone! Output written to: {}", output_file.display());
    eprintln!("Total commits: {total_commits}");
    eprintln!("Repositories with commits: {repo_count}");

    Ok(())
}
