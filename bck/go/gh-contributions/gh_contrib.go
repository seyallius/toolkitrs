package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
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

	writeHeader(file, cfg.Username, cfg.Since, cfg.Until)

	commitsByRepo := collectCommits(cfg, repos)
	writeCommits(file, cfg.Username, commitsByRepo)
	writeSummary(file, commitsByRepo)

	fmt.Fprintf(os.Stderr, "\nDone! Output written to: %s\n", outputFile)
	printSummary(commitsByRepo)

	return nil
}

// ------------------------------------ Configuration ----------------------------------------- //

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
		Client:   &http.Client{},
	}, nil
}

// ------------------------------------ API Calls --------------------------------------------- //

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

// ------------------------------------ Data Processing --------------------------------------- //

type RepoCommits struct {
	RepoName string
	Commits  []CommitInfo
}

func collectCommits(cfg *Config, repos []Repository) []RepoCommits {
	var results []RepoCommits

	for _, repo := range repos {
		fmt.Fprintf(os.Stderr, "Fetching commits for %s...\n", repo.Name)

		commits, err := fetchCommits(cfg, repo.Name)
		if err != nil {
			//fmt.Fprintf(os.Stderr, "  Skipping %s: %v\n", repo.Name, err)
			continue
		}

		filtered := filterCommits(commits)
		if len(filtered) == 0 {
			continue
		}

		results = append(results, RepoCommits{
			RepoName: repo.Name,
			Commits:  filtered,
		})
	}

	return results
}

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
	subject = strings.TrimSpace(parts[0])

	if len(parts) > 1 {
		body = strings.TrimSpace(parts[1])
	}

	return subject, body
}

// ------------------------------------ Output Writing ---------------------------------------- //

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

func writeCommits(w io.Writer, username string, commitsByRepo []RepoCommits) {
	for _, rc := range commitsByRepo {
		writeRepoHeader(w, username, rc.RepoName)

		for _, commit := range rc.Commits {
			writeCommitEntry(w, commit)
		}
	}
}

func writeRepoHeader(w io.Writer, username, repoName string) {
	header := fmt.Sprintf(
		"\nRepository: %s/%s\n%s\n",
		username, repoName, strings.Repeat("-", len(repoName)+len(username)+13),
	)
	fmt.Fprint(w, header)
}

func writeCommitEntry(w io.Writer, commit CommitInfo) {
	entry := fmt.Sprintf(
		"    Date: %s\n    %s\n",
		commit.Date, commit.Subject,
	)
	fmt.Fprint(w, entry)

	if commit.Body != "" {
		fmt.Fprintf(w, "\n%s\n", indentLines(commit.Body, "    "))
	}
	fmt.Fprintln(w)
}

func indentLines(text, indent string) string {
	lines := strings.Split(text, "\n")
	for i, line := range lines {
		lines[i] = indent + line
	}
	return strings.Join(lines, "\n")
}

func writeSummary(w io.Writer, commitsByRepo []RepoCommits) {
	totalCommits := 0
	for _, rc := range commitsByRepo {
		totalCommits += len(rc.Commits)
	}

	summary := fmt.Sprintf(
		"\n%s\nSummary:\n%s\nTotal commits: %d\nRepositories with commits: %d\n",
		strings.Repeat("=", 60),
		strings.Repeat("-", 60),
		totalCommits,
		len(commitsByRepo),
	)
	fmt.Fprint(w, summary)
}

func printSummary(commitsByRepo []RepoCommits) {
	totalCommits := 0
	for _, rc := range commitsByRepo {
		totalCommits += len(rc.Commits)
	}

	fmt.Fprintf(os.Stderr, "Total commits: %d\n", totalCommits)
	fmt.Fprintf(os.Stderr, "Repositories with commits: %d\n", len(commitsByRepo))
}
