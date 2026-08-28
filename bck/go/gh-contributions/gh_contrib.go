// Package main provides a tool to fetch and export GitHub contributions
// (commits) for a specified user within a date range.
//
// It uses the GitHub API with token authentication, processes repositories
// concurrently with a worker pool, and writes the results to a text file.
//
// Memory Usage With Direct File Writing (mutex-synchronized worker pool):
//
//	Worker1 ──┐
//	Worker2 ──┼──> [Mutex] ──> File
//	Worker3 ──┘
//			  ▲
//			  └── Each worker writes immediately, then discards data.
//			      No channel buffer means no accumulation of results in memory.
//
// The SafeFileWriter uses a mutex to ensure only one worker writes to the
// file at a time, preventing corruption while maintaining memory efficiency.
//
// Memory Usage With Buffered Channel (worker pool pattern) -- Deprecated:
//
//	Worker1 ──┐
//	Worker2 ──┼──> [Channel Buffer] ──> Main Loop ──> File
//	Worker3 ──┘
//			  ▲
//			  └── All results sit here if main loop is slow
package main

import (
	"fmt"
	"net/http"
	"os"
	"slices"
	"time"
)

// ---------------------------------------- Main ---------------------------------------------- //

// main is the entry point of the program. It parses command-line arguments,
// validates configuration, and executes the contribution fetching process.
func main() {
	cfg, err := parseConfig()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	if err := run(cfg); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}

// -------------------------------------- Internal Helpers -------------------------------------- //

// run executes the main workflow: fetching repositories, processing them
// concurrently with direct file writing, and producing the output file.
//
// Parameters:
//   - cfg: The configuration containing username, date range, token, and HTTP client.
//
// Returns:
//   - error: Any error encountered during execution.
func run(cfg *Config) error {
	repos, err := fetchRepositories(cfg)
	if err != nil {
		return fmt.Errorf("fetching repositories: %w", err)
	}

	fmt.Fprintf(os.Stderr, "Found %d repositories\n", len(repos))

	outputFile := getOutputFilenameWithSafeFileWriter(cfg.Username, cfg.Since, cfg.Until)
	writer, err := newSafeFileWriter(outputFile)
	if err != nil {
		return fmt.Errorf("creating output file: %w", err)
	}
	defer writer.Close()

	// Write header once
	writer.writeHeader(cfg.Username, cfg.Since, cfg.Until)

	// Process repositories concurrently with direct file writing
	totalCommits, repoCount := processRepositoriesWithDirectWrite(cfg, repos, writer)

	// Write summary
	writer.writeSummary(totalCommits, repoCount)

	fmt.Fprintf(os.Stderr, "\nDone! Output written to: %s\n", outputFile)
	fmt.Fprintf(os.Stderr, "Total commits: %d\n", totalCommits)
	fmt.Fprintf(os.Stderr, "Repositories with commits: %d\n", repoCount)

	return nil
}

// parseConfig parses command-line arguments and environment variables to build
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
func parseConfig() (*Config, error) {
	if len(os.Args) < 4 {
		return nil, fmt.Errorf(
			"usage: go run gh_contrib.go <username> <since> <until>\n" +
				"  Format: YYYY-MM-DD\n" +
				"  Example: go run gh_contrib.go seyallius 2026-01-01 2026-08-27",
		)
	}

	token := os.Getenv(envToken)
	if token == "" {
		return nil, fmt.Errorf("%s environment variable required\n"+
			"Get token at: https://github.com/settings/tokens (scope: repo)", envToken)
	}

	fetchReadme := true
	// Check for --no-readme flag
	if slices.Contains(os.Args, "--no-readme") {
		fetchReadme = false
	}

	return &Config{
		Username:    os.Args[1],
		Since:       os.Args[2],
		Until:       os.Args[3],
		Token:       token,
		Client:      &http.Client{Timeout: 30 * time.Second},
		FetchReadme: fetchReadme,
	}, nil
}
