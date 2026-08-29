//! module types - Core data structures and constants for the GitHub contributions fetcher.
//!
//! Defines Repository, CommitResponse, Config, CommitInfo, and RepoResult types
//! used throughout the gh-contrib workflow.

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Environment variable name for the GitHub personal access token.
pub const ENV_TOKEN: &str = "GITHUB_TOKEN";

/// Date format used for parsing input dates (YYYY-MM-DD).
pub const DATE_LAYOUT: &str = "%Y-%m-%d";

/// Date-time format used in the output file for commit timestamps.
pub const TIME_LAYOUT: &str = "%Y-%m-%d %H:%M:%S";

/// Base URL for the GitHub REST API v3.
pub const API_BASE_URL: &str = "https://api.github.com";

/// Number of items per page when paginating through API results.
pub const PER_PAGE: u32 = 100;

/// Prefix used for the output filename.
pub const OUTPUT_PREFIX: &str = "contributions";

/// Maximum number of concurrent API calls to avoid rate limiting.
pub const MAX_WORKERS: usize = 10;

/// Length of the "Repository: " prefix used for separator calculation.
const REPOSITORY_STR_LEN: usize = 13;

/// Width of the summary separator lines.
const SEPARATOR_WIDTH: usize = 60;

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Represents a GitHub repository with its name.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Repository {
    /// The repository name.
    pub name: String,
}

/// Represents the API response structure for a commit.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CommitResponse {
    /// The commit details.
    pub commit: Commit,
}

/// Contains the author and message details of a commit.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Commit {
    /// The commit author information.
    pub author: CommitAuthor,
    /// The full commit message.
    pub message: String,
}

/// Contains the date of the commit.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CommitAuthor {
    /// ISO 8601 timestamp of the commit.
    pub date: String,
}

/// Holds the configuration and dependencies for the gh-contrib workflow.
#[derive(Debug, Clone)]
pub struct GhContribConfig {
    /// GitHub username whose commits are being fetched.
    pub username: String,
    /// Start date in YYYY-MM-DD format.
    pub since: String,
    /// End date in YYYY-MM-DD format.
    pub until: String,
    /// GitHub personal access token for authentication.
    pub token: String,
    /// Whether to fetch README files for each repository.
    pub fetch_readme: bool,
}

/// Holds the processed commit details for output.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// Formatted commit date.
    pub date: String,
    /// First line of the commit message.
    pub subject: String,
    /// Remaining lines of the commit message (if any).
    pub body: String,
}

/// Represents the result of processing a single repository.
#[derive(Debug)]
pub struct RepoResult {
    /// Name of the repository.
    pub repo_name: String,
    /// List of commit information.
    pub commits: Vec<CommitInfo>,
    /// Error encountered while processing (None if successful).
    pub error: Option<String>,
    /// Optional README content.
    pub readme_content: Option<String>,
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Returns the repository separator string for the given username and repo name.
///
/// Used in the output file to create a visual divider under repository headers.
pub fn repo_separator(username: &str, repo_name: &str) -> String {
    let width = repo_name.len() + username.len() + REPOSITORY_STR_LEN;
    "-".repeat(width)
}

/// Returns the header separator line (60 `=` characters).
pub fn header_separator() -> String {
    "=".repeat(SEPARATOR_WIDTH)
}

/// Returns the summary separator line (60 `-` characters).
pub fn summary_separator() -> String {
    "-".repeat(SEPARATOR_WIDTH)
}

/// Generates the output filename based on the username and date range.
///
/// # Example
/// ```
/// let name = output_filename("seyallius", "2026-01-01", "2026-08-27");
/// assert_eq!(name, "contributions_seyallius_2026-01-01_to_2026-08-27.txt");
/// ```
pub fn output_filename(username: &str, since: &str, until: &str) -> String {
    format!("{OUTPUT_PREFIX}_{username}_{since}_to_{until}.txt")
}
