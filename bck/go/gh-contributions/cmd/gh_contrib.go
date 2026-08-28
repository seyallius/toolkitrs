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
// The writer.SafeFileWriter uses a mutex to ensure only one worker writes to the
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
	"gh_contrib/internal/api"
	"gh_contrib/internal/config"
	"gh_contrib/internal/processor"
	"gh_contrib/internal/types"
	writer2 "gh_contrib/internal/writer"
	"os"
)

// ---------------------------------------- Main ---------------------------------------------- //

// main is the entry point of the program. It parses command-line arguments,
// validates configuration, and executes the contribution fetching process.
func main() {
	cfg, err := config.ParseConfig()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	if err = run(cfg); err != nil {
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
func run(cfg *types.Config) error {
	repos, err := api.FetchRepositories(cfg)
	if err != nil {
		return fmt.Errorf("fetching repositories: %w", err)
	}

	fmt.Fprintf(os.Stderr, "Found %d repositories\n", len(repos))

	outputFile := writer2.GetOutputFilenameWithSafeFileWriter(cfg.Username, cfg.Since, cfg.Until)
	writer, err := writer2.NewSafeFileWriter(outputFile)
	if err != nil {
		return fmt.Errorf("creating output file: %w", err)
	}
	defer writer.Close()

	// Write header once
	writer.WriteHeader(cfg.Username, cfg.Since, cfg.Until)

	// Process repositories concurrently with direct file writing
	totalCommits, repoCount := processor.ProcessRepositoriesWithDirectWrite(cfg, repos, writer)

	// Write summary
	writer.WriteSummary(totalCommits, repoCount)

	fmt.Fprintf(os.Stderr, "\nDone! Output written to: %s\n", outputFile)
	fmt.Fprintf(os.Stderr, "Total commits: %d\n", totalCommits)
	fmt.Fprintf(os.Stderr, "Repositories with commits: %d\n", repoCount)

	return nil
}
