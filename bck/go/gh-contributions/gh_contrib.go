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
	envToken     = "GITHUB_TOKEN"
	dateLayout   = "2006-01-02"
	timeLayout   = "2006-01-02 15:04:05"
	apiBaseURL   = "https://api.github.com"
	perPage      = 100
	outputPrefix = "contributions"
	maxWorkers   = 10 // Limit concurrent API calls to avoid rate limiting
)

type Repository struct {
	Name string `json:"name"`
}

type CommitResponse struct {
	Commit Commit `json:"commit"`
}

type Commit struct {
	Author  Author `json:"author"`
	Message string `json:"message"`
}

type Author struct {
	Date string `json:"date"`
}

type Config struct {
	Username string
	Since    string
	Until    string
	Token    string
	Client   *http.Client
}

type CommitInfo struct {
	Date    string
	Subject string
	Body    string
}

type RepoResult struct {
	RepoName string
	Commits  []CommitInfo
	Err      error
}

// ---------------------------------------- Main ---------------------------------------------- //

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

func fetchRepositories(cfg *Config) ([]Repository, error) {
	url := fmt.Sprintf("%s/user/repos?per_page=%d&type=all", apiBaseURL, perPage)

	var repos []Repository
	if err := doRequest(cfg, url, &repos); err != nil {
		return nil, err
	}

	return repos, nil
}

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

func shouldSkip(c CommitResponse) bool {
	msg := c.Commit.Message
	return strings.Contains(msg, "Merge") ||
		strings.Contains(msg, "Revert") ||
		strings.HasPrefix(msg, "Merge ")
}

func splitCommitMessage(full string) (subject, body string) {
	parts := strings.SplitN(full, "\n", 2)
	subject = strings.TrimSpace(parts[0]) + "\n"

	if len(parts) > 1 {
		body = strings.TrimSpace(parts[1])
	}

	return subject, body
}

// --- Output Writing

func getOutputFilename(username, since, until string) string {
	return fmt.Sprintf("%s_%s_%s_to_%s.txt", outputPrefix, username, since, until)
}

func writeHeader(w io.Writer, username, since, until string) {
	header := fmt.Sprintf(
		"GitHub Contributions for %s\nPeriod: %s to %s\n%s\n\n",
		username, since, until, strings.Repeat("=", 60),
	)
	fmt.Fprint(w, header)
}

func writeRepoHeader(w io.Writer, username, repoName string) {
	header := fmt.Sprintf(
		"\nRepository: %s/%s\n%s\n",
		username, repoName, strings.Repeat("-", len(repoName)+len(username)+13),
	)
	fmt.Fprint(w, header)
}

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
