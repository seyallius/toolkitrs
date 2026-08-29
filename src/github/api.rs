//! module api - GitHub API interactions including fetching repositories,
//! commits, and README content with authentication and error handling.

use crate::github::types::{CommitResponse, GhContribConfig, Repository, API_BASE_URL, PER_PAGE};
use anyhow::{bail, Context, Result};
use base64::Engine;

// ---------------------------------------------- Types ----------------------------------------- //

/// A minimal struct for deserializing the README API response.
#[derive(Debug, serde::Deserialize)]
struct ReadmeResponse {
    /// Base64-encoded README content.
    content: String,
    /// Encoding type (always "base64" for GitHub).
    #[allow(dead_code)]
    encoding: String,
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Retrieves all repositories owned by the authenticated user.
///
/// # Arguments
/// * `config` - The configuration containing the authenticated user and token.
///
/// # Returns
/// A vector of repositories or an error.
pub async fn fetch_repositories(config: &GhContribConfig) -> Result<Vec<Repository>> {
    let url = format!("{API_BASE_URL}/user/repos?per_page={PER_PAGE}&type=all");
    do_request(config, &url).await
}

/// Retrieves commits from a specific repository for the authenticated user
/// within the configured date range.
///
/// # Arguments
/// * `config` - The configuration containing the user, date range, and authentication.
/// * `repo_name` - The name of the repository to fetch commits from.
///
/// # Returns
/// A vector of commit responses from the API or an error.
pub async fn fetch_commits(
    config: &GhContribConfig,
    repo_name: &str,
) -> Result<Vec<CommitResponse>> {
    let since = format!("{}T00:00:00Z", config.since);
    let until = format!("{}T23:59:59Z", config.until);
    let url = format!(
        "{API_BASE_URL}/repos/{}/{repo_name}/commits?author={}&since={since}&until={until}&per_page={PER_PAGE}",
        config.username, config.username
    );
    do_request(config, &url).await
}

/// Retrieves the README.md content for a repository.
///
/// Returns the content as a decoded string (base64-decoded from the API response).
///
/// # Arguments
/// * `config` - The configuration containing authentication details.
/// * `repo_name` - The repository to fetch the README for.
///
/// # Returns
/// The README content as a string, or an error.
pub async fn fetch_readme(config: &GhContribConfig, repo_name: &str) -> Result<String> {
    let url = format!(
        "{API_BASE_URL}/repos/{}/{repo_name}/readme",
        config.username
    );

    let response = build_request(config, &url)?
        .send()
        .await
        .with_context(|| format!("fetching README for {repo_name}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        bail!("API returned {status}: {body}");
    }

    let readme_response: ReadmeResponse =
        serde_json::from_str(&body).context("parsing README response")?;

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&readme_response.content)
        .context("decoding README base64 content")?;

    String::from_utf8(decoded).context("converting README to UTF-8")
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Performs an authenticated HTTP GET request and deserializes the JSON response.
///
/// # Arguments
/// * `config` - The configuration containing the authentication token.
/// * `url` - The full API endpoint URL.
///
/// # Returns
/// The deserialized response or an error.
async fn do_request<T: serde::de::DeserializeOwned>(
    config: &GhContribConfig,
    url: &str,
) -> Result<T> {
    let response = build_request(config, url)?
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        bail!("API returned {status}: {body}");
    }

    serde_json::from_str(&body).context("parsing JSON response")
}

/// Builds an authenticated HTTP GET request with proper headers.
fn build_request(config: &GhContribConfig, url: &str) -> Result<reqwest::RequestBuilder> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("creating HTTP client")?;

    Ok(client
        .get(url)
        .header("Authorization", format!("token {}", config.token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", config.username.clone()))
}
