//! module filter - Commit filtering and message processing utilities.
//!
//! Provides filtering out merge/revert commits and splitting commit messages
//! into subject and body components, matching the Go implementation's behavior.

use crate::github::types::{CommitInfo, CommitResponse, TIME_LAYOUT};
use chrono::{DateTime, Utc};

// ----------------------------------------- Public API ----------------------------------------- //

/// Processes raw commit responses, filtering out merge and revert commits,
/// and formatting the commit data for output.
///
/// # Arguments
/// * `commits` - Raw commit responses from the API.
///
/// # Returns
/// Filtered and formatted commit information.
pub fn filter_commits(commits: &[CommitResponse]) -> Vec<CommitInfo> {
    commits
        .iter()
        .filter(|c| !should_skip(c))
        .filter_map(|c| {
            let date = parse_and_format_date(&c.commit.author.date)?;
            let (subject, body) = split_commit_message(&c.commit.message);
            Some(CommitInfo {
                date,
                subject,
                body,
            })
        })
        .collect()
}

/// Determines whether a commit should be excluded from the output
/// based on its message content (merge commits, revert commits, etc.).
///
/// # Arguments
/// * `commit` - The commit response to evaluate.
///
/// # Returns
/// True if the commit should be skipped, false otherwise.
pub fn should_skip(commit: &CommitResponse) -> bool {
    let msg = &commit.commit.message;
    msg.contains("Merge") || msg.contains("Revert") || msg.starts_with("Merge ")
}

/// Splits a commit message into subject (first line) and body (remaining lines).
///
/// The subject does NOT include a trailing newline (unlike the Go version which
/// adds one for formatting; we handle that in the writer).
///
/// # Arguments
/// * `full` - The complete commit message.
///
/// # Returns
/// A tuple of (subject, body).
pub fn split_commit_message(full: &str) -> (String, String) {
    let mut parts = full.splitn(2, '\n');
    let subject = parts.next().unwrap_or("").trim().to_string();
    let body = parts.next().unwrap_or("").trim().to_string();
    (subject, body)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Parses an ISO 8601 date string and formats it using the project's time layout.
fn parse_and_format_date(date_str: &str) -> Option<String> {
    let parsed: DateTime<Utc> = date_str.parse().ok()?;
    Some(parsed.format(TIME_LAYOUT).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_subject_and_body() {
        let (subject, body) = split_commit_message("fix: something\n\nDetailed explanation");
        assert_eq!(subject, "fix: something");
        assert_eq!(body, "Detailed explanation");
    }

    #[test]
    fn skips_merge_commits() {
        let commit = CommitResponse {
            commit: crate::github::types::Commit {
                author: crate::github::types::CommitAuthor {
                    date: "2026-01-01T00:00:00Z".to_string(),
                },
                message: "Merge branch 'main' into feature".to_string(),
            },
        };
        assert!(should_skip(&commit));
    }

    #[test]
    fn skips_revert_commits() {
        let commit = CommitResponse {
            commit: crate::github::types::Commit {
                author: crate::github::types::CommitAuthor {
                    date: "2026-01-01T00:00:00Z".to_string(),
                },
                message: "Revert \"some change\"".to_string(),
            },
        };
        assert!(should_skip(&commit));
    }

    #[test]
    fn keeps_normal_commits() {
        let commit = CommitResponse {
            commit: crate::github::types::Commit {
                author: crate::github::types::CommitAuthor {
                    date: "2026-01-01T00:00:00Z".to_string(),
                },
                message: "feat: add new feature".to_string(),
            },
        };
        assert!(!should_skip(&commit));
    }
}
