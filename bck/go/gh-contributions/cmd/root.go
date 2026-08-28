// Package main. root.go - Defines the root command for the GitHub contributions
// fetcher CLI using Cobra, including command structure, flag definitions,
// and help text.
package main

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

var (
	username    string
	since       string
	until       string
	fetchReadme bool
)

// rootCmd represents the base command when called without any subcommands
var rootCmd = &cobra.Command{
	Use:   "gh-contrib",
	Short: "Fetch and export GitHub contributions (commits) for a specified user",
	Long: `gh-contrib is a tool that fetches GitHub commits for a specified user
within a date range, processes them, and exports the results to a text file.

It uses the GitHub API with token authentication, processes repositories
concurrently, and writes the results with proper formatting.

Examples:
  gh-contrib --username seyallius --since 2026-01-01 --until 2026-08-27
  gh-contrib -u seyallius -s 2026-01-01 -u 2026-08-27 --no-readme`,
	RunE: runCmd,
}

// Execute adds all child commands to the root command and sets flags appropriately.
// This is called by main.main(). It only needs to happen once to the rootCmd.
func Execute() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}

func init() {
	// Define flags
	rootCmd.Flags().StringVarP(&username, "username", "u", "", "GitHub username (required)")
	rootCmd.Flags().StringVarP(&since, "since", "s", "", "Start date in YYYY-MM-DD format (required)")
	rootCmd.Flags().StringVarP(&until, "until", "t", "", "End date in YYYY-MM-DD format (required)")
	rootCmd.Flags().BoolVar(&fetchReadme, "no-readme", true, "Skip fetching README files")

	// Mark required flags
	rootCmd.MarkFlagRequired("username")
	rootCmd.MarkFlagRequired("since")
	rootCmd.MarkFlagRequired("until")
}
