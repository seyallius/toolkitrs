// Package config. config.go - Handles configuration creation and validation
// for the GitHub contributions fetcher, including environment variable checks.
package config

import (
	"fmt"
	"gh_contrib/internal/types"
	"net/http"
	"os"
	"time"
)

// NewConfig creates and validates a new configuration from the provided parameters.
// It checks that the GITHUB_TOKEN environment variable is set and validates
// the date formats.
//
// Parameters:
//   - username: GitHub username
//   - since: Start date in YYYY-MM-DD format
//   - until: End date in YYYY-MM-DD format
//   - fetchReadme: Whether to fetch README files
//
// Returns:
//   - *types.Config: The validated configuration
//   - error: Any validation error
func NewConfig(username, since, until string, fetchReadme bool) (*types.Config, error) {
	// Validate token
	token := os.Getenv(types.EnvToken)
	if token == "" {
		return nil, fmt.Errorf("%s environment variable required\n"+
			"Get token at: https://github.com/settings/tokens (scope: repo, read:user)", types.EnvToken)
	}

	// Validate dates (basic format check)
	if _, err := time.Parse(types.DateLayout, since); err != nil {
		return nil, fmt.Errorf("invalid since date: %w (expected YYYY-MM-DD)", err)
	}
	if _, err := time.Parse(types.DateLayout, until); err != nil {
		return nil, fmt.Errorf("invalid until date: %w (expected YYYY-MM-DD)", err)
	}

	// Validate date range
	sinceTime, _ := time.Parse(types.DateLayout, since)
	untilTime, _ := time.Parse(types.DateLayout, until)
	if sinceTime.After(untilTime) {
		return nil, fmt.Errorf("since date must be before until date")
	}

	return &types.Config{
		Username:    username,
		Since:       since,
		Until:       until,
		Token:       token,
		Client:      &http.Client{Timeout: 30 * time.Second},
		FetchReadme: fetchReadme,
	}, nil
}
