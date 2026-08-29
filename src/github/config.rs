//! module config - Configuration creation and validation for the GitHub contributions fetcher.
//!
//! Validates environment variables, date formats, and date range ordering
//! before constructing the runtime configuration.

use crate::github::types::{GhContribConfig, DATE_LAYOUT, ENV_TOKEN};
use anyhow::{bail, Context, Result};
use chrono::NaiveDate;

// ----------------------------------------- Public API ----------------------------------------- //

/// Creates and validates a new configuration from the provided parameters.
///
/// Checks that the GITHUB_TOKEN environment variable is set and validates
/// the date formats and ordering.
///
/// # Arguments
/// * `username` - GitHub username.
/// * `since` - Start date in YYYY-MM-DD format.
/// * `until` - End date in YYYY-MM-DD format.
/// * `fetch_readme` - Whether to fetch README files.
///
/// # Errors
/// Returns an error if the token is missing or dates are invalid.
pub fn new_config(
    username: &str,
    since: &str,
    until: &str,
    fetch_readme: bool,
) -> Result<GhContribConfig> {
    // Validate token
    let token = std::env::var(ENV_TOKEN).unwrap_or_default();
    if token.is_empty() {
        bail!(
            "{ENV_TOKEN} environment variable required\n\
             Get token at: https://github.com/settings/tokens (scope: repo, read:user)"
        );
    }

    // Validate dates
    let since_date = NaiveDate::parse_from_str(since, DATE_LAYOUT)
        .with_context(|| format!("invalid since date: {since} (expected YYYY-MM-DD)"))?;
    let until_date = NaiveDate::parse_from_str(until, DATE_LAYOUT)
        .with_context(|| format!("invalid until date: {until} (expected YYYY-MM-DD)"))?;

    // Validate date range
    if since_date > until_date {
        bail!("since date must be before until date");
    }

    Ok(GhContribConfig {
        username: username.to_string(),
        since: since.to_string(),
        until: until.to_string(),
        token,
        fetch_readme,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_date() {
        std::env::set_var("GITHUB_TOKEN", "test-token");
        let result = new_config("user", "2026-13-01", "2026-01-01", false);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_inverted_range() {
        std::env::set_var("GITHUB_TOKEN", "test-token");
        let result = new_config("user", "2026-08-01", "2026-01-01", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("before"));
    }
}
