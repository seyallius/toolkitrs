// Package writer. writer.go - Implements thread-safe file writing operations with
// mutex synchronization, including methods for writing headers, repository
// sections, commit entries, and summary statistics to the output file.
package writer

import (
	"fmt"
	"gh_contrib/internal/types"
	"io"
	"os"
	"strings"
	"sync"
)

// ---------------------------------------------- Types ----------------------------------------- //

// SafeFileWriter provides thread-safe file writing operations using a mutex
// to synchronize concurrent writes from multiple goroutines.
type SafeFileWriter struct {
	mu   sync.Mutex
	file *os.File
}

// ---------------------------------------- Constructor(s) -------------------------------------- //

// NewSafeFileWriter creates a new SafeFileWriter instance with the given filename.
// It creates the file and returns a writer that can be safely used concurrently.
//
// Parameters:
//   - filename: The name of the file to create
//
// Returns:
//   - *SafeFileWriter: The thread-safe file writer
//   - error: Any file creation error
func NewSafeFileWriter(filename string) (*SafeFileWriter, error) {
	file, err := os.Create(filename)
	if err != nil {
		return nil, err
	}
	return &SafeFileWriter{file: file}, nil
}

// ----------------------------------------- Public API ----------------------------------------- //

// WriteHeader writes the header section of the output file in a thread-safe manner.
//
// Parameters:
//   - username: The GitHub username
//   - since: The start date
//   - until: The end date
func (w *SafeFileWriter) WriteHeader(username, since, until string) {
	w.mu.Lock()
	defer w.mu.Unlock()

	header := fmt.Sprintf(
		"GitHub Contributions for %s\nPeriod: %s to %s\n%s\n\n",
		username, since, until, strings.Repeat("=", 60),
	)
	w.file.WriteString(header)
	w.file.Sync()
}

// WriteRepo writes a repository's commits to the file in a thread-safe manner.
// It writes the README (if available), repository header, and all commit entries.
//
// Parameters:
//   - username: The GitHub username
//   - repoName: The repository name
//   - commits: The list of commit information to write
func (w *SafeFileWriter) WriteRepo(username, repoName string, commits []types.CommitInfo, readmeContent string) {
	w.mu.Lock()
	defer w.mu.Unlock()

	// Write repo header
	const repositoryStrLen = 13
	header := fmt.Sprintf(
		"Repository: %s/%s\n%s\n",
		username, repoName, strings.Repeat("-", len(repoName)+len(username)+repositoryStrLen),
	)
	w.file.WriteString(header)

	// Write README if available
	if readmeContent != "" {
		w.file.WriteString("\n")
		// Indent each line of README with 2 spaces
		lines := strings.Split(readmeContent, "\n")
		for _, line := range lines {
			fmt.Fprintf(w.file, "  %s\n", line)
		}
		w.file.WriteString("\n")
	}

	// Write commits header
	w.file.WriteString("  Commits:\n")

	// Write all commits for this repo
	for _, commit := range commits {
		w.WriteCommitLocked(commit)
	}

	w.file.Sync()
}

// WriteCommitLocked writes a single commit entry to the file.
// This method assumes the mutex is already held by the caller.
//
// Parameters:
//   - commit: The commit information to write
func (w *SafeFileWriter) WriteCommitLocked(commit types.CommitInfo) {
	fmt.Fprintf(w.file, "  Date: %s\n", commit.Date)
	fmt.Fprintf(w.file, "    %s\n", commit.Subject)

	if commit.Body != "" {
		lines := strings.Split(commit.Body, "\n")
		for _, line := range lines {
			if line == "" {
				fmt.Fprintln(w.file)
			} else {
				fmt.Fprintf(w.file, "    %s\n", line)
			}
		}
	}
	fmt.Fprintln(w.file)
}

// WriteSummary writes the summary section at the end of the output file
// in a thread-safe manner.
//
// Parameters:
//   - totalCommits: Total number of commits processed
//   - repoCount: Number of repositories that had commits
func (w *SafeFileWriter) WriteSummary(totalCommits, repoCount int) {
	w.mu.Lock()
	defer w.mu.Unlock()

	summary := fmt.Sprintf(
		"\n%s\nSummary:\n%s\nTotal commits: %d\nRepositories with commits: %d\n",
		strings.Repeat("=", 60),
		strings.Repeat("-", 60),
		totalCommits,
		repoCount,
	)
	w.file.WriteString(summary)
	w.file.Sync()
}

// Close closes the underlying file in a thread-safe manner.
//
// Returns:
//   - error: Any file close error
func (w *SafeFileWriter) Close() error {
	w.mu.Lock()
	defer w.mu.Unlock()

	return w.file.Close()
}

// GetOutputFilenameWithSafeFileWriter generates the output filename based on the username and date range.
//
// Parameters:
//   - username: The GitHub username
//   - since: The start date
//   - until: The end date
//
// Returns:
//   - string: The generated filename (e.g., contributions_username_2026-01-01_to_2026-08-27.txt)
func GetOutputFilenameWithSafeFileWriter(username, since, until string) string {
	return fmt.Sprintf("%s_%s_%s_to_%s.txt", types.OutputPrefix, username, since, until)
}

// ----------------------------------------- Deprecated ----------------------------------------- //

// GetOutputFilename generates the output filename based on the username and date range.
//
// Parameters:
//   - username: The GitHub username
//   - since: The start date
//   - until: The end date
//
// Returns:
//   - string: The generated filename (e.g., contributions_username_2026-01-01_to_2026-08-27.txt)
//
// Deprecated: Use getOutputFilenameWithSafeFileWriter instead.
func GetOutputFilename(username, since, until string) string {
	return fmt.Sprintf("%s_%s_%s_to_%s.txt", types.OutputPrefix, username, since, until)
}

// WriteHeader writes the header section of the output file, including the
// username, date range, and a decorative separator.
//
// Parameters:
//   - w: The writer to write to
//   - username: The GitHub username
//   - since: The start date
//   - until: The end date
//
// Deprecated: Use SafeFileWriter.WriteHeader instead.
func WriteHeader(w io.Writer, username, since, until string) {
	header := fmt.Sprintf(
		"GitHub Contributions for %s\nPeriod: %s to %s\n%s\n\n",
		username, since, until, strings.Repeat("=", 60),
	)
	fmt.Fprint(w, header)
}

// WriteRepoHeader writes the header section for a repository, showing the
// full repository path (username/repo) with a decorative separator.
//
// Parameters:
//   - w: The writer to write to
//   - username: The GitHub username
//   - repoName: The repository name
//
// Deprecated: Use SafeFileWriter.WriteRepo instead.
func WriteRepoHeader(w io.Writer, username, repoName string) {
	header := fmt.Sprintf(
		"\nRepository: %s/%s\n%s\n",
		username, repoName, strings.Repeat("-", len(repoName)+len(username)+13),
	)
	fmt.Fprint(w, header)
}

// WriteCommitEntry writes a single commit entry to the output, including the
// formatted date, subject line, and indented body lines (if any).
//
// Parameters:
//   - w: The writer to write to
//   - commit: The commit information to write
//
// Deprecated: Use SafeFileWriter.WriteCommitLocked instead.
func WriteCommitEntry(w io.Writer, commit types.CommitInfo) {
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

// WriteSummary writes the summary section at the end of the output file,
// including the total number of commits and repositories with commits.
//
// Parameters:
//   - w: The writer to write to
//   - totalCommits: Total number of commits processed
//   - repoCount: Number of repositories that had commits
//
// Deprecated: Use SafeFileWriter.WriteSummary instead.
func WriteSummary(w io.Writer, totalCommits, repoCount int) {
	summary := fmt.Sprintf(
		"\n%s\nSummary:\n%s\nTotal commits: %d\nRepositories with commits: %d\n",
		strings.Repeat("=", 60),
		strings.Repeat("-", 60),
		totalCommits,
		repoCount,
	)
	fmt.Fprint(w, summary)
}
