// Package main. api.go - Handles all GitHub API interactions including fetching
// repositories, commits, and README content with authentication and error handling.
package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
)

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

// fetchReadme retrieves the README.md content for a repository.
// Returns the content as a string (already decoded from base64).
func fetchReadme(cfg *Config, repoName string) (string, error) {
	url := fmt.Sprintf("%s/repos/%s/%s/readme", apiBaseURL, cfg.Username, repoName)

	var readmeResp struct {
		Content  string `json:"content"`
		Encoding string `json:"encoding"`
	}

	if err := doRequest(cfg, url, &readmeResp); err != nil {
		return "", err
	}

	// GitHub returns content as base64
	decoded, err := base64.StdEncoding.DecodeString(readmeResp.Content)
	if err != nil {
		return "", err
	}

	return string(decoded), nil
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
