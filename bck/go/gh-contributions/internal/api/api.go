// Package api. api.go - Handles all GitHub API interactions including fetching
// repositories, commits, and README content with authentication and error handling.
package api

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"gh_contrib/internal/types"
	"io"
	"net/http"
)

// FetchRepositories retrieves all repositories owned by the authenticated user.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token.
//
// Returns:
//   - []Repository: List of repositories
//   - error: Any API or parsing error
func FetchRepositories(cfg *types.Config) ([]types.Repository, error) {
	url := fmt.Sprintf("%s/user/repos?per_page=%d&type=all", types.ApiBaseURL, types.PerPage)

	var repos []types.Repository
	if err := DoRequest(cfg, url, &repos); err != nil {
		return nil, err
	}

	return repos, nil
}

// FetchCommits retrieves commits from a specific repository for the authenticated user
// within the configured date range.
//
// Parameters:
//   - cfg: The configuration containing the user, date range, and authentication
//   - repoName: The name of the repository to fetch commits from
//
// Returns:
//   - []CommitResponse: List of commit responses from the API
//   - error: Any API or parsing error
func FetchCommits(cfg *types.Config, repoName string) ([]types.CommitResponse, error) {
	since := cfg.Since + "T00:00:00Z"
	until := cfg.Until + "T23:59:59Z"

	url := fmt.Sprintf(
		"%s/repos/%s/%s/commits?author=%s&since=%s&until=%s&per_page=%d",
		types.ApiBaseURL, cfg.Username, repoName, cfg.Username, since, until, types.PerPage,
	)

	var commits []types.CommitResponse
	if err := DoRequest(cfg, url, &commits); err != nil {
		return nil, err
	}

	return commits, nil
}

// FetchReadme retrieves the README.md content for a repository.
// Returns the content as a string (already decoded from base64).
func FetchReadme(cfg *types.Config, repoName string) (string, error) {
	url := fmt.Sprintf("%s/repos/%s/%s/readme", types.ApiBaseURL, cfg.Username, repoName)

	var readmeResp struct {
		Content  string `json:"content"`
		Encoding string `json:"encoding"`
	}

	if err := DoRequest(cfg, url, &readmeResp); err != nil {
		return "", err
	}

	// GitHub returns content as base64
	decoded, err := base64.StdEncoding.DecodeString(readmeResp.Content)
	if err != nil {
		return "", err
	}

	return string(decoded), nil
}

// DoRequest performs an authenticated HTTP GET request to the GitHub API and
// unmarshals the JSON response into the provided target.
//
// Parameters:
//   - cfg: The configuration containing the authentication token
//   - url: The full API endpoint URL
//   - target: The pointer to unmarshal the JSON response into
//
// Returns:
//   - error: Any request, status code, or unmarshaling error
func DoRequest(cfg *types.Config, url string, target interface{}) error {
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
