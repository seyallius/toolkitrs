//! module writer - Thread-safe file writing operations for the contributions output.
//!
//! Uses a Mutex to synchronize concurrent writes from multiple async tasks,
//! ensuring the output file is never corrupted.

use crate::github::types::{header_separator, repo_separator, summary_separator, CommitInfo};
use anyhow::{Context, Result};
use std::{fs::File, io::Write, path::Path, sync::Mutex};

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Thread-safe file writer using a Mutex to synchronize concurrent writes.
///
/// This is the Rust analogue of the Go `SafeFileWriter` with `sync.Mutex`.
/// Each write method acquires the lock, writes, and releases — ensuring
/// atomicity of each logical section.
pub struct SafeFileWriter {
    /// The underlying file handle, protected by a Mutex.
    inner: Mutex<File>,
}
impl SafeFileWriter {
    /// Creates a new `SafeFileWriter` with the given filename.
    ///
    /// # Arguments
    /// * `filename` - The path of the file to create.
    ///
    /// # Errors
    /// Returns an error if the file cannot be created.
    pub fn new(filename: &Path) -> Result<Self> {
        let file = File::create(filename)
            .with_context(|| format!("creating output file: {}", filename.display()))?;
        Ok(Self {
            inner: Mutex::new(file),
        })
    }

    // ----------------------------------------- Public API ----------------------------------------- //

    /// Writes the header section of the output file.
    ///
    /// # Arguments
    /// * `username` - The GitHub username.
    /// * `since` - The start date.
    /// * `until` - The end date.
    pub fn write_header(&self, username: &str, since: &str, until: &str) {
        let header = format!(
            "GitHub Contributions for {username}\nPeriod: {since} to {until}\n{}\n",
            header_separator()
        );
        self.write_str(&header);
    }

    /// Writes a repository's commits to the file.
    ///
    /// Writes the repository header, optional README content (indented),
    /// and all commit entries.
    ///
    /// # Arguments
    /// * `username` - The GitHub username.
    /// * `repo_name` - The repository name.
    /// * `commits` - The list of commit information to write.
    /// * `readme_content` - Optional README content to include.
    pub fn write_repo(
        &self,
        username: &str,
        repo_name: &str,
        commits: &[CommitInfo],
        readme_content: Option<&str>,
    ) {
        let mut output = String::new();

        // Repository header
        output.push_str(&format!(
            "\nRepository: {username}/{repo_name}\n{}\n",
            repo_separator(username, repo_name)
        ));

        // README content (indented with 2 spaces)
        if let Some(readme) = readme_content {
            output.push('\n');
            for line in readme.lines() {
                output.push_str(&format!("  {line}\n"));
            }
            output.push('\n');
        }

        // Commits header
        output.push_str("\n  Commits:\n");

        // All commits
        for commit in commits {
            output.push_str(&format_commit_entry(commit));
        }

        self.write_str(&output);
    }

    /// Writes the summary section at the end of the output file.
    ///
    /// # Arguments
    /// * `total_commits` - Total number of commits processed.
    /// * `repo_count` - Number of repositories that had commits.
    pub fn write_summary(&self, total_commits: usize, repo_count: usize) {
        let summary = format!(
            "\n{}\nSummary:\n{}\nTotal commits: {total_commits}\nRepositories with commits: {repo_count}\n",
            header_separator(),
            summary_separator()
        );
        self.write_str(&summary);
    }

    // -------------------------------------- Internal Helpers -------------------------------------- //

    /// Writes a string to the file, acquiring the Mutex lock.
    fn write_str(&self, content: &str) {
        let mut file = self.inner.lock().expect("writer mutex poisoned");
        let _ = file.write_all(content.as_bytes());
        let _ = file.flush();
    }
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Formats a single commit entry matching the Go output format.
///
/// ```text
///   Date: 2026-01-15 10:30:00
///     feat: add something
///     Additional body text
///
/// ```
fn format_commit_entry(commit: &CommitInfo) -> String {
    let mut entry = String::new();
    entry.push_str(&format!("  Date: {}\n", commit.date));
    entry.push_str(&format!("    {}\n", commit.subject));

    if !commit.body.is_empty() {
        for line in commit.body.lines() {
            if line.is_empty() {
                entry.push('\n');
            } else {
                entry.push_str(&format!("    {line}\n"));
            }
        }
    }
    entry.push('\n');
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_commit_entry() {
        let commit = CommitInfo {
            date: "2026-01-15 10:30:00".to_string(),
            subject: "feat: add stuff".to_string(),
            body: String::new(),
        };
        let entry = format_commit_entry(&commit);
        assert!(entry.contains("  Date: 2026-01-15 10:30:00\n"));
        assert!(entry.contains("    feat: add stuff\n"));
    }
}
