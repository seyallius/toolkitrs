// Package filter. filter.go - Provides commit filtering and message processing
// utilities, including filtering out merge/revert commits and splitting commit
// messages into subject and body components.
package filter

import (
	"gh_contrib/internal/types"
	"strings"
	"time"
)

// FilterCommits processes raw commit responses, filtering out merge and revert commits,
// and formatting the commit data for output.
//
// Parameters:
//   - commits: Raw commit responses from the API
//
// Returns:
//   - []CommitInfo: Filtered and formatted commit information
func FilterCommits(commits []types.CommitResponse) []types.CommitInfo {
	var filtered []types.CommitInfo

	for _, c := range commits {
		if ShouldSkip(c) {
			continue
		}

		date, _ := time.Parse(time.RFC3339, c.Commit.Author.Date)
		subject, body := SplitCommitMessage(c.Commit.Message)

		filtered = append(filtered, types.CommitInfo{
			Date:    date.Format(types.TimeLayout),
			Subject: subject,
			Body:    body,
		})
	}

	return filtered
}

// ShouldSkip determines whether a commit should be excluded from the output
// based on its message content (merge commits, revert commits, etc.).
//
// Parameters:
//   - c: The commit response to evaluate
//
// Returns:
//   - bool: True if the commit should be skipped, false otherwise
func ShouldSkip(c types.CommitResponse) bool {
	msg := c.Commit.Message
	return strings.Contains(msg, "Merge") ||
		strings.Contains(msg, "Revert") ||
		strings.HasPrefix(msg, "Merge ")
}

// SplitCommitMessage splits a commit message into subject (first line) and body
// (remaining lines). The subject includes a trailing newline for formatting.
//
// Parameters:
//   - full: The complete commit message
//
// Returns:
//   - subject: The first line of the commit message
//   - body: The remaining lines (empty if none)
func SplitCommitMessage(full string) (subject, body string) {
	parts := strings.SplitN(full, "\n", 2)
	subject = strings.TrimSpace(parts[0]) + "\n"

	if len(parts) > 1 {
		body = strings.TrimSpace(parts[1])
	}

	return subject, body
}
