//! module github::tui - TUI integration for the GitHub contributions exporter.

use crate::{
    github,
    tui::{
        app::RunOptions,
        command::{CommandOption, FilePickerMode, TuiCommand},
        event::AppEvent,
    },
};
use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};
use tokio_util::sync::CancellationToken;
// ------------------------------------------ Types & Impls ------------------------------------- //

/// The gh-contrib command as a TUI-executable workflow.
#[derive(Debug)]
pub struct GhContribCommand;
impl TuiCommand for GhContribCommand {
    fn id(&self) -> &'static str {
        "gh-contrib"
    }

    fn title(&self) -> &'static str {
        "gh-contrib"
    }

    fn description(&self) -> &'static str {
        "Export GitHub commits to a text file"
    }

    fn file_picker_mode(&self) -> FilePickerMode {
        // No file picker — this command fetches from the GitHub API.
        FilePickerMode::NotNeeded
    }

    fn options(&self) -> Vec<CommandOption> {
        vec![
            CommandOption::Text {
                label: "GitHub username",
                default: "",
                placeholder: "e.g., octocat",
            },
            CommandOption::Text {
                label: "Since (YYYY-MM-DD)",
                default: "yesterday",
                placeholder: "YYYY-MM-DD or 'yesterday'",
            },
            CommandOption::Text {
                label: "Until (YYYY-MM-DD)",
                default: "today",
                placeholder: "YYYY-MM-DD or 'today'",
            },
            CommandOption::Toggle {
                label: "Skip README files",
                default: false,
            },
        ]
    }

    // ... inside impl TuiCommand for GhContribCommand ...

    fn execute(
        &self,
        _files: Vec<PathBuf>, // unused — no file picker
        options: &RunOptions,
        cancel: CancellationToken,
        _ffmpeg_path: &Path, // unused — not a media command
        tx: Sender<AppEvent>,
    ) -> Result<()> {
        let username = options
            .custom
            .get("GitHub username")
            .cloned()
            .unwrap_or_default();
        let since = options
            .custom
            .get("Since (YYYY-MM-DD)")
            .cloned()
            .unwrap_or_default();
        let until = options
            .custom
            .get("Until (YYYY-MM-DD)")
            .cloned()
            .unwrap_or_default();
        let no_readme = options
            .custom
            .get("Skip README files")
            .map(|v| v == "true")
            .unwrap_or(false);

        if username.is_empty() || since.is_empty() || until.is_empty() {
            let _ = tx.send(AppEvent::Log("❌ Missing username or dates".into()));
            let _ = tx.send(AppEvent::AllDone {
                succeeded: 0,
                failed: 1,
            });
            return Ok(());
        }

        let cfg = match github::config::new_config(&username, &since, &until, !no_readme) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(AppEvent::Log(format!("❌ {e}")));
                let _ = tx.send(AppEvent::AllDone {
                    succeeded: 0,
                    failed: 1,
                });
                return Ok(());
            }
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        runtime.block_on(async {
            if cancel.is_cancelled() {
                return;
            }
            let _ = tx.send(AppEvent::Log("Fetching repositories...".into()));

            // Wrap initial fetch in select! for instant TUI response
            let repos = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = tx.send(AppEvent::Log("⚠️ Cancelled".into()));
                    let _ = tx.send(AppEvent::AllDone { succeeded: 0, failed: 0 });
                    return;
                }
                res = github::api::fetch_repositories(&cfg) => match res {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.send(AppEvent::Log(format!("❌ {e}")));
                        let _ = tx.send(AppEvent::AllDone { succeeded: 0, failed: 1 });
                        return;
                    }
                }
            };

            let _ = tx.send(AppEvent::Log(format!("Found {} repositories", repos.len())));

            if cancel.is_cancelled() {
                let _ = tx.send(AppEvent::Log("⚠️ Cancelled".into()));
                let _ = tx.send(AppEvent::AllDone {
                    succeeded: 0,
                    failed: 0,
                });
                return;
            }

            let output_file = PathBuf::from(github::types::output_filename(
                &cfg.username,
                &cfg.since,
                &cfg.until,
            ));

            let file_writer = match github::writer::SafeFileWriter::new(&output_file) {
                Ok(w) => std::sync::Arc::new(w),
                Err(e) => {
                    let _ = tx.send(AppEvent::Log(format!("❌ {e}")));
                    let _ = tx.send(AppEvent::AllDone {
                        succeeded: 0,
                        failed: 1,
                    });
                    return;
                }
            };

            file_writer.write_header(&cfg.username, &cfg.since, &cfg.until);

            // ✅ Pass the cancellation token down to the processor
            let (total_commits, repo_count) =
                github::processor::process_repositories(&cfg, &repos, &file_writer, cancel.clone())
                    .await;

            if cancel.is_cancelled() {
                let _ = tx.send(AppEvent::Log(
                    "⚠️ Cancelled by user. Output may be incomplete.".into(),
                ));
                let _ = tx.send(AppEvent::AllDone {
                    succeeded: 0,
                    failed: 0,
                });
                return;
            }

            file_writer.write_summary(total_commits, repo_count);
            let _ = tx.send(AppEvent::Log(format!(
                "✅ Done! Output: {}",
                output_file.display()
            )));
            let _ = tx.send(AppEvent::AllDone {
                succeeded: 1,
                failed: 0,
            });
        });
        Ok(())
    }
}
