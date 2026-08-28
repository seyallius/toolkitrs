// Package types. types.go - Defines the core data structures and constants used
// throughout the application, including Repository, CommitResponse, Commit, Author,
// Config, CommitInfo, RepoResult, and SafeFileWriter types.
package types

import "net/http"

const (
	// EnvToken is the environment variable name used to read the GitHub personal access token.
	EnvToken = "GITHUB_TOKEN"

	// DateLayout defines the date format used for parsing input dates (YYYY-MM-DD).
	DateLayout = "2006-01-02"

	// TimeLayout defines the date-time format used in the output file for commit timestamps.
	TimeLayout = "2006-01-02 15:04:05"

	// ApiBaseURL is the base URL for the GitHub REST API v3.
	ApiBaseURL = "https://api.github.com"

	// PerPage is the number of items per page when paginating through API results.
	PerPage = 100

	// OutputPrefix is the prefix used for the output filename.
	OutputPrefix = "contributions"

	// MaxWorkers limits the number of concurrent API calls to avoid rate limiting.
	MaxWorkers = 10
)

// Repository represents a GitHub repository with its name.
type Repository struct {
	Name string `json:"name"`
}

// CommitResponse represents the API response structure for a commit.
type CommitResponse struct {
	Commit Commit `json:"commit"`
}

// Commit contains the author and message details of a commit.
type Commit struct {
	Author  Author `json:"author"`
	Message string `json:"message"`
}

// Author contains the date of the commit.
type Author struct {
	Date string `json:"date"`
}

// Config holds the configuration and dependencies for the application.
type Config struct {
	Username    string       // GitHub username whose commits are being fetched
	Since       string       // Start date in YYYY-MM-DD format
	Until       string       // End date in YYYY-MM-DD format
	Token       string       // GitHub personal access token for authentication
	Client      *http.Client // HTTP client with timeout configuration
	FetchReadme bool         // Should fetch README file as well
}

// CommitInfo holds the processed commit details for output.
type CommitInfo struct {
	Date    string // Formatted commit date
	Subject string // First line of the commit message
	Body    string // Remaining lines of the commit message (if any)
}

// RepoResult represents the result of processing a single repository.
type RepoResult struct {
	RepoName string       // Name of the repository
	Commits  []CommitInfo // List of commit information
	Err      error        // Error encountered while processing (nil if successful)
}
