// Package main provides a tool to fetch and export GitHub contributions
// (commits) for a specified user within a date range.
//
// It uses the GitHub API with token authentication, processes repositories
// concurrently with a worker pool, and writes the results to a text file.
//
// Memory Usage With Buffered Channel (worker pool pattern):
//
//	Worker1 ──┐
//	Worker2 ──┼──> [Channel Buffer] ──> Main Loop ──> File
//	Worker3 ──┘
//			  ▲
//			  └── All results sit here if main loop is slow
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"
)

// --------------------------------- Types, Constants & Variables ------------------------------- //

const (
	// envToken is the environment variable name used to read the GitHub personal access token.
	envToken = "GITHUB_TOKEN"

	// dateLayout defines the date format used for parsing input dates (YYYY-MM-DD).
	dateLayout = "2006-01-02"

	// timeLayout defines the date-time format used in the output file for commit timestamps.
	timeLayout = "2006-01-02 15:04:05"

	// apiBaseURL is the base URL for the GitHub REST API v3.
	apiBaseURL = "https://api.github.com"

	// perPage is the number of items per page when paginating through API results.
	perPage = 100

	// outputPrefix is the prefix used for the output filename.
	outputPrefix = "contributions"

	// maxWorkers limits the number of concurrent API calls to avoid rate limiting.
	maxWorkers = 10
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
	Username string       // GitHub username whose commits are being fetched
	Since    string       // Start date in YYYY-MM-DD format
	Until    string       // End date in YYYY-MM-DD format
	Token    string       // GitHub personal access token for authentication
	Client   *http.Client // HTTP client with timeout configuration
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

// run executes the main workflow: fetching repositories, processing them
// concurrently, and writing the results to an output file.
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

	outputFile := getOutputFilename(cfg.Username, cfg.Since, cfg.Until)
	file, err := os.Create(outputFile)
	if err != nil {
		return fmt.Errorf("creating output file: %w", err)
	}
	defer file.Close()

	// Write header once
	writeHeader(file, cfg.Username, cfg.Since, cfg.Until)

	// Process repositories concurrently
	results := processRepositories(cfg, repos)

	// Write results as they come in (synchronized via channel)
	totalCommits := 0
	repoCount := 0

	for result := range results {
		if result.Err != nil {
			fmt.Fprintf(os.Stderr, "Error processing %s: %v\n", result.RepoName, result.Err)
			continue
		}

		if len(result.Commits) == 0 {
			continue
		}

		repoCount++
		totalCommits += len(result.Commits)

		// Write this repo's commits immediately
		writeRepoHeader(file, cfg.Username, result.RepoName)
		for _, commit := range result.Commits {
			writeCommitEntry(file, commit)
		}
		file.Sync() // Force flush to disk
	}

	// Write summary
	writeSummary(file, totalCommits, repoCount)

	fmt.Fprintf(os.Stderr, "\nDone! Output written to: %s\n", outputFile)
	fmt.Fprintf(os.Stderr, "Total commits: %d\n", totalCommits)
	fmt.Fprintf(os.Stderr, "Repositories with commits: %d\n", repoCount)

	return nil
}

// -------------------------------------- Internal Helpers -------------------------------------- //

// --- Configuration

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
		return nil, fmt.Errorf("usage: go run gh_contrib.go <username> <since> <until>\n" +
			"  Format: YYYY-MM-DD\n" +
			"  Example: go run gh_contrib.go seyallius 2026-01-01 2026-08-27")
	}

	token := os.Getenv(envToken)
	if token == "" {
		return nil, fmt.Errorf("%s environment variable required\n"+
			"Get token at: https://github.com/settings/tokens (scope: repo)", envToken)
	}

	return &Config{
		Username: os.Args[1],
		Since:    os.Args[2],
		Until:    os.Args[3],
		Token:    token,
		Client:   &http.Client{Timeout: 30 * time.Second},
	}, nil
}

// --- API Calls

// fetchRepositories retrieves all repositories owned by the authenticated user.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token.
//
// Returns:
//   - []Repository: List of repositories
//   - error: Any API or parsing error
func fetchRepositories(cfg *Config) ([]Repository, error) {
	url := fmt.Sprintf("%s/user/repos?per_page=%d&type=all", apiBaseURL, perPage)

	var repos []Repository
	if err := doRequest(cfg, url, &repos); err != nil {
		return nil, err
	}

	return repos, nil
}

// fetchCommits retrieves commits from a specific repository for the authenticated user
// within the configured date range.
//
// Parameters:
//   - cfg: The configuration containing the user, date range, and authentication
//   - repoName: The name of the repository to fetch commits from
//
// Returns:
//   - []CommitResponse: List of commit responses from the API
//   - error: Any API or parsing error
func fetchCommits(cfg *Config, repoName string) ([]CommitResponse, error) {
	since := cfg.Since + "T00:00:00Z"
	until := cfg.Until + "T23:59:59Z"

	url := fmt.Sprintf(
		"%s/repos/%s/%s/commits?author=%s&since=%s&until=%s&per_page=%d",
		apiBaseURL, cfg.Username, repoName, cfg.Username, since, until, perPage,
	)

	var commits []CommitResponse
	if err := doRequest(cfg, url, &commits); err != nil {
		return nil, err
	}

	return commits, nil
}

// doRequest performs an authenticated HTTP GET request to the GitHub API and
// unmarshals the JSON response into the provided target.
//
// Parameters:
//   - cfg: The configuration containing the authentication token
//   - url: The full API endpoint URL
//   - target: The pointer to unmarshal the JSON response into
//
// Returns:
//   - error: Any request, status code, or unmarshaling error
func doRequest(cfg *Config, url string, target interface{}) error {
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return err
	}

	req.Header.Set("Authorization", "token "+cfg.Token)
	req.Header.Set("Accept", "application/vnd.github+json")

	resp, err := cfg.Client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("API returned %d: %s", resp.StatusCode, string(body))
	}

	return json.Unmarshal(body, target)
}

// --- Concurrent Processing

// processRepositories orchestrates concurrent processing of repositories using
// a worker pool pattern. It creates a channel of results that are consumed
// by the caller.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token
//   - repos: The list of repositories to process
//
// Returns:
//   - <-chan RepoResult: A read-only channel that receives results as they complete
func processRepositories(cfg *Config, repos []Repository) <-chan RepoResult {
	// "Worker Pool Pattern"

	results := make(chan RepoResult, len(repos)) // Buffered to avoid blocking
	var wg sync.WaitGroup

	// Create worker pool with limited concurrency
	workers := maxWorkers
	if len(repos) < workers {
		workers = len(repos)
	}

	// Job queue
	jobs := make(chan Repository, len(repos))
	for _, repo := range repos {
		jobs <- repo
	}
	close(jobs)

	// Start workers
	for range workers {
		wg.Add(1)
		go worker(cfg, jobs, results, &wg)
	}

	// Close results channel when all workers are done
	go func() {
		wg.Wait()
		close(results)
	}()

	return results
}

// worker is the function executed by each worker goroutine. It processes
// repositories from the job queue and sends results to the results channel.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token
//   - jobs: A read-only channel of repositories to process
//   - results: A write-only channel to send processing results
//   - wg: WaitGroup counter to signal when this worker is done
func worker(cfg *Config, jobs <-chan Repository, results chan<- RepoResult, wg *sync.WaitGroup) {
	defer wg.Done()

	for repo := range jobs {
		fmt.Fprintf(os.Stderr, "Fetching commits for %s...\n", repo.Name)

		commits, err := fetchCommits(cfg, repo.Name)
		if err != nil {
			results <- RepoResult{
				RepoName: repo.Name,
				Err:      fmt.Errorf("fetching commits: %w", err),
			}
			continue
		}

		filtered := filterCommits(commits)
		results <- RepoResult{
			RepoName: repo.Name,
			Commits:  filtered,
			Err:      nil,
		}

		fmt.Fprintf(os.Stderr, "  Found %d commits for %s\n", len(filtered), repo.Name)
	}
}

// --- Data Processing

// filterCommits processes raw commit responses, filtering out merge and revert commits,
// and formatting the commit data for output.
//
// Parameters:
//   - commits: Raw commit responses from the API
//
// Returns:
//   - []CommitInfo: Filtered and formatted commit information
func filterCommits(commits []CommitResponse) []CommitInfo {
	var filtered []CommitInfo

	for _, c := range commits {
		if shouldSkip(c) {
			continue
		}

		date, _ := time.Parse(time.RFC3339, c.Commit.Author.Date)
		subject, body := splitCommitMessage(c.Commit.Message)

		filtered = append(filtered, CommitInfo{
			Date:    date.Format(timeLayout),
			Subject: subject,
			Body:    body,
		})
	}

	return filtered
}

// shouldSkip determines whether a commit should be excluded from the output
// based on its message content (merge commits, revert commits, etc.).
//
// Parameters:
//   - c: The commit response to evaluate
//
// Returns:
//   - bool: True if the commit should be skipped, false otherwise
func shouldSkip(c CommitResponse) bool {
	msg := c.Commit.Message
	return strings.Contains(msg, "Merge") ||
		strings.Contains(msg, "Revert") ||
		strings.HasPrefix(msg, "Merge ")
}

// splitCommitMessage splits a commit message into subject (first line) and body
// (remaining lines). The subject includes a trailing newline for formatting.
//
// Parameters:
//   - full: The complete commit message
//
// Returns:
//   - subject: The first line of the commit message
//   - body: The remaining lines (empty if none)
func splitCommitMessage(full string) (subject, body string) {
	parts := strings.SplitN(full, "\n", 2)
	subject = strings.TrimSpace(parts[0]) + "\n"

	if len(parts) > 1 {
		body = strings.TrimSpace(parts[1])
	}

	return subject, body
}

// --- Output Writing

// getOutputFilename generates the output filename based on the username and date range.
//
// Parameters:
//   - username: The GitHub username
//   - since: The start date
//   - until: The end date
//
// Returns:
//   - string: The generated filename (e.g., contributions_username_2026-01-01_to_2026-08-27.txt)
func getOutputFilename(username, since, until string) string {
	return fmt.Sprintf("%s_%s_%s_to_%s.txt", outputPrefix, username, since, until)
}

// writeHeader writes the header section of the output file, including the
// username, date range, and a decorative separator.
//
// Parameters:
//   - w: The writer to write to
//   - username: The GitHub username
//   - since: The start date
//   - until: The end date
func writeHeader(w io.Writer, username, since, until string) {
	header := fmt.Sprintf(
		"GitHub Contributions for %s\nPeriod: %s to %s\n%s\n\n",
		username, since, until, strings.Repeat("=", 60),
	)
	fmt.Fprint(w, header)
}

// writeRepoHeader writes the header section for a repository, showing the
// full repository path (username/repo) with a decorative separator.
//
// Parameters:
//   - w: The writer to write to
//   - username: The GitHub username
//   - repoName: The repository name
func writeRepoHeader(w io.Writer, username, repoName string) {
	header := fmt.Sprintf(
		"\nRepository: %s/%s\n%s\n",
		username, repoName, strings.Repeat("-", len(repoName)+len(username)+13),
	)
	fmt.Fprint(w, header)
}

// writeCommitEntry writes a single commit entry to the output, including the
// formatted date, subject line, and indented body lines (if any).
//
// Parameters:
//   - w: The writer to write to
//   - commit: The commit information to write
func writeCommitEntry(w io.Writer, commit CommitInfo) {
	fmt.Fprintf(w, "  Date: %s\n", commit.Date)
	fmt.Fprintf(w, "    %s\n", commit.Subject)

	if commit.Body != "" {
		lines := strings.Split(commit.Body, "\n")
		for _, line := range lines {
			if line == "" {
				fmt.Fprintln(w)
			} else {
				fmt.Fprintf(w, "    %s\n", line)
			}
		}
	}
	fmt.Fprintln(w)
}

// writeSummary writes the summary section at the end of the output file,
// including the total number of commits and repositories with commits.
//
// Parameters:
//   - w: The writer to write to
//   - totalCommits: Total number of commits processed
//   - repoCount: Number of repositories that had commits
func writeSummary(w io.Writer, totalCommits, repoCount int) {
	summary := fmt.Sprintf(
		"\n%s\nSummary:\n%s\nTotal commits: %d\nRepositories with commits: %d\n",
		strings.Repeat("=", 60),
		strings.Repeat("-", 60),
		totalCommits,
		repoCount,
	)
	fmt.Fprint(w, summary)
}
