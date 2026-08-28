// Package main. filter.go - Provides commit filtering and message processing
// utilities, including filtering out merge/revert commits and splitting commit
// messages into subject and body components.
package main

import (
	"strings"
	"time"
)

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
