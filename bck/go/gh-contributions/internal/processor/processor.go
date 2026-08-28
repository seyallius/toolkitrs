// Package processor. processor.go - Implements concurrent processing of repositories
// using Worker pool patterns, including both buffered channel and direct file
// writing approaches for memory-efficient commit processing.
package processor

import (
	"fmt"
	"gh_contrib/internal/api"
	"gh_contrib/internal/filter"
	"gh_contrib/internal/types"
	"gh_contrib/internal/writer"
	"os"
	"sync"
)

// ProcessRepositories orchestrates concurrent processing of repositories using
// a Worker pool pattern. It creates a channel of results that are consumed
// by the caller.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token
//   - repos: The list of repositories to process
//
// Returns:
//   - <-chan RepoResult: A read-only channel that receives results as they complete
//
// Deprecated: Use ProcessRepositoriesWithDirectWrite instead.
func ProcessRepositories(cfg *types.Config, repos []types.Repository) <-chan types.RepoResult {
	// "Worker Pool Pattern"

	results := make(chan types.RepoResult, len(repos)) // Buffered to avoid blocking
	var wg sync.WaitGroup

	// Create Worker pool with limited concurrency
	Workers := types.MaxWorkers
	if len(repos) < Workers {
		Workers = len(repos)
	}

	// Job queue
	jobs := make(chan types.Repository, len(repos))
	for _, repo := range repos {
		jobs <- repo
	}
	close(jobs)

	// Start Workers
	for range Workers {
		wg.Add(1)
		go Worker(cfg, jobs, results, &wg)
	}

	// Close results channel when all Workers are done
	go func() {
		wg.Wait()
		close(results)
	}()

	return results
}

// ProcessRepositoriesWithDirectWrite orchestrates concurrent processing of repositories
// where each Worker writes directly to the file using a synchronized writer.
// This approach avoids storing all results in memory via a channel buffer.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token
//   - repos: The list of repositories to process
//   - writer: The thread-safe file writer
//
// Returns:
//   - totalCommits: Total number of commits processed
//   - repoCount: Number of repositories that had commits
func ProcessRepositoriesWithDirectWrite(cfg *types.Config, repos []types.Repository, writer *writer.SafeFileWriter) (int, int) {
	var wg sync.WaitGroup
	var mu sync.Mutex // For synchronizing counters
	totalCommits := 0
	repoCount := 0

	// Create Worker pool with limited concurrency
	Workers := types.MaxWorkers
	if len(repos) < Workers {
		Workers = len(repos)
	}

	// Job queue
	jobs := make(chan types.Repository, len(repos))
	for _, repo := range repos {
		jobs <- repo
	}
	close(jobs)

	// Start Workers
	for range Workers {
		wg.Add(1)
		go WorkerWithDirectWrite(cfg, jobs, writer, &wg, &mu, &totalCommits, &repoCount)
	}

	wg.Wait()
	return totalCommits, repoCount
}

// --- Concurrent Processing with Direct Write

// Worker is the function executed by each Worker goroutine. It processes
// repositories from the job queue and sends results to the results channel.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token
//   - jobs: A read-only channel of repositories to process
//   - results: A write-only channel to send processing results
//   - wg: WaitGroup counter to signal when this Worker is done
func Worker(cfg *types.Config, jobs <-chan types.Repository, results chan<- types.RepoResult, wg *sync.WaitGroup) {
	defer wg.Done()

	for repo := range jobs {
		fmt.Fprintf(os.Stderr, "Fetching commits for %s...\n", repo.Name)

		commits, err := api.FetchCommits(cfg, repo.Name)
		if err != nil {
			results <- types.RepoResult{
				RepoName: repo.Name,
				Err:      fmt.Errorf("fetching commits: %w", err),
			}
			continue
		}

		filtered := filter.FilterCommits(commits)
		results <- types.RepoResult{
			RepoName: repo.Name,
			Commits:  filtered,
			Err:      nil,
		}

		fmt.Fprintf(os.Stderr, "  Found %d commits for %s\n", len(filtered), repo.Name)
	}
}

// WorkerWithDirectWrite is the function executed by each Worker goroutine.
// It processes repositories from the job queue and writes results directly
// to the file using the thread-safe writer.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token
//   - jobs: A read-only channel of repositories to process
//   - writer: The thread-safe file writer
//   - wg: WaitGroup counter to signal when this Worker is done
//   - mu: Mutex for synchronizing counter updates
//   - totalCommits: Pointer to the total commits counter
//   - repoCount: Pointer to the repository count counter
func WorkerWithDirectWrite(
	cfg *types.Config,
	jobs <-chan types.Repository,
	writer *writer.SafeFileWriter,
	wg *sync.WaitGroup,
	mu *sync.Mutex,
	totalCommits *int,
	repoCount *int,
) {
	defer wg.Done()

	for repo := range jobs {
		fmt.Fprintf(os.Stderr, "Fetching commits for %s...\n", repo.Name)

		// Fetch commits
		commits, err := api.FetchCommits(cfg, repo.Name)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error fetching %s: %v\n", repo.Name, err)
			continue
		}

		filtered := filter.FilterCommits(commits)
		if len(filtered) == 0 {
			continue
		}

		// Fetch README
		var readmeContent string
		if cfg.FetchReadme {
			content, err := api.FetchReadme(cfg, repo.Name)
			if err != nil {
				fmt.Fprintf(os.Stderr, "  Warning: Could not fetch README for %s: %v\n", repo.Name, err)
			} else {
				readmeContent = content
			}
		}

		// Write directly to file (synchronized internally)
		writer.WriteRepo(cfg.Username, repo.Name, filtered, readmeContent)

		// Update counters safely
		mu.Lock()
		*totalCommits += len(filtered)
		*repoCount++
		mu.Unlock()

		fmt.Fprintf(os.Stderr, "  Written %d commits for %s\n", len(filtered), repo.Name)
	}
}
