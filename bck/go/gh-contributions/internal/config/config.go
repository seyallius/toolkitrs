package config

import (
	"fmt"
	"gh_contrib/internal/types"
	"net/http"
	"os"
	"slices"
	"time"
)

// ParseConfig parses command-line arguments and environment variables to build
// the application configuration.
//
// Expected command-line arguments:
//   - os.Args[1]: GitHub username
//   - os.Args[2]: Start date (YYYY-MM-DD)
//   - os.Args[3]: End date (YYYY-MM-DD)
//
// Environment variables:
//   - GITHUB_TOKEN: GitHub personal access token (required)
//
// Returns:
//   - *Config: The parsed configuration
//   - error: Any validation or parsing error
func ParseConfig() (*types.Config, error) {
	if len(os.Args) < 4 {
		return nil, fmt.Errorf(
			"usage: go run gh_contrib.go <username> <since> <until>\n" +
				"  Format: YYYY-MM-DD\n" +
				"  Example: go run gh_contrib.go seyallius 2026-01-01 2026-08-27",
		)
	}

	token := os.Getenv(types.EnvToken)
	if token == "" {
		return nil, fmt.Errorf("%s environment variable required\n"+
			"Get token at: https://github.com/settings/tokens (scope: repo)", types.EnvToken)
	}

	FetchReadme := true
	// Check for --no-readme flag
	if slices.Contains(os.Args, "--no-readme") {
		FetchReadme = false
	}

	return &types.Config{
		Username:    os.Args[1],
		Since:       os.Args[2],
		Until:       os.Args[3],
		Token:       token,
		Client:      &http.Client{Timeout: 30 * time.Second},
		FetchReadme: FetchReadme,
	}, nil
}
