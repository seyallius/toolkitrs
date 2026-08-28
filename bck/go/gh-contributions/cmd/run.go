// Package main. run.go - Implements the main execution logic for the gh-contrib
// command, orchestrating the workflow after flag parsing and validation.
package main

import (
	"fmt"
	"gh_contrib/internal/api"
	"gh_contrib/internal/config"
	"gh_contrib/internal/processor"
	"gh_contrib/internal/writer"
	"os"

	"github.com/spf13/cobra"
)

// runCmd executes the main workflow: fetching repositories, processing them
// concurrently with direct file writing, and producing the output file.
func runCmd(_ *cobra.Command, _ []string) error {
	// Create config from flags
	cfg, err := config.NewConfig(username, since, until, fetchReadme)
	if err != nil {
		return err
	}

	// Fetch repositories
	repos, err := api.FetchRepositories(cfg)
	if err != nil {
		return fmt.Errorf("fetching repositories: %w", err)
	}

	fmt.Fprintf(os.Stderr, "Found %d repositories\n", len(repos))

	// Create output file
	outputFile := writer.GetOutputFilenameWithSafeFileWriter(cfg.Username, cfg.Since, cfg.Until)
	w, err := writer.NewSafeFileWriter(outputFile)
	if err != nil {
		return fmt.Errorf("creating output file: %w", err)
	}
	defer w.Close()

	// Write header once
	w.WriteHeader(cfg.Username, cfg.Since, cfg.Until)

	// Process repositories concurrently with direct file writing
	totalCommits, repoCount := processor.ProcessRepositoriesWithDirectWrite(cfg, repos, w)

	// Write summary
	w.WriteSummary(totalCommits, repoCount)

	fmt.Fprintf(os.Stderr, "\nDone! Output written to: %s\n", outputFile)
	fmt.Fprintf(os.Stderr, "Total commits: %d\n", totalCommits)
	fmt.Fprintf(os.Stderr, "Repositories with commits: %d\n", repoCount)

	return nil
}
