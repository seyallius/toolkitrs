//! module config - Configuration creation and validation for the GitHub contributions fetcher.
//!
//! Validates environment variables, date formats, and date range ordering
//! before constructing the runtime configuration. Date inputs accept literal
//! `YYYY-MM-DD` values plus the `today` / `yesterday` keywords, resolved here
//! so the CLI and the TUI share exactly the same semantics.

use crate::github::types::{GhContribConfig, DATE_LAYOUT, ENV_TOKEN};
use anyhow::{bail, Context, Result};
use chrono::{Duration, Local, NaiveDate};

// ----------------------------------------- Public API ----------------------------------------- //

/// Creates and validates a new configuration from the provided parameters.
///
/// Checks that the GITHUB_TOKEN environment variable is set, resolves the
/// `since`/`until` dates (accepting `today` / `yesterday` shortcuts), and
/// validates their ordering. The stored config always contains normalized
/// `YYYY-MM-DD` strings.
///
/// # Arguments
/// * `username` - GitHub username.
/// * `since` - Start date: `YYYY-MM-DD`, `today`, or `yesterday`.
/// * `until` - End date: `YYYY-MM-DD`, `today`, or `yesterday`.
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
    // Validate token.
    let token = std::env::var(ENV_TOKEN).unwrap_or_default();
    if token.is_empty() {
        bail!(
            "{ENV_TOKEN} environment variable required\n\
             Get token at: https://github.com/settings/tokens (scope: repo, read:user)"
        );
    }

    // Resolve + validate dates. Keywords are resolved here (not in the UI)
    // so every caller behaves identically.
    let since_date = resolve_date(since).with_context(|| format!("invalid since date: {since}"))?;
    let until_date = resolve_date(until).with_context(|| format!("invalid until date: {until}"))?;

    // Validate date range.
    if since_date > until_date {
        bail!("since date must be before until date");
    }

    Ok(GhContribConfig {
        username: username.to_string(),
        since: since_date.format(DATE_LAYOUT).to_string(),
        until: until_date.format(DATE_LAYOUT).to_string(),
        token,
        fetch_readme,
    })
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Resolves raw user input into a concrete date.
///
/// Accepts `YYYY-MM-DD` plus two keywords evaluated against the local clock:
/// * `today`     -> current local date
/// * `yesterday` -> current local date minus one day
///
/// # Errors
/// Returns an error when the input is neither a keyword nor a valid date.
fn resolve_date(input: &str) -> Result<NaiveDate> {
    match input.trim().to_ascii_lowercase().as_str() {
        "today" => Ok(Local::now().date_naive()),
        "yesterday" => Ok(Local::now().date_naive() - Duration::days(1)),
        literal => NaiveDate::parse_from_str(literal, DATE_LAYOUT)
            .with_context(|| format!("expected YYYY-MM-DD, 'today' or 'yesterday', got: {input}")),
    }
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

    #[test]
    fn resolves_keyword_dates() {
        std::env::set_var("GITHUB_TOKEN", "test-token");
        let cfg = new_config("user", "yesterday", "today", false).unwrap();
        let since = NaiveDate::parse_from_str(&cfg.since, DATE_LAYOUT).unwrap();
        let until = NaiveDate::parse_from_str(&cfg.until, DATE_LAYOUT).unwrap();
        assert_eq!(until - since, Duration::days(1));
    }

    #[test]
    fn normalizes_literal_dates() {
        std::env::set_var("GITHUB_TOKEN", "test-token");
        let cfg = new_config("user", "2026-01-01", "2026-08-27", false).unwrap();
        assert_eq!(cfg.since, "2026-01-01");
        assert_eq!(cfg.until, "2026-08-27");
    }
}
