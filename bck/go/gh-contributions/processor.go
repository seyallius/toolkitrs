// Package main. processor.go - Implements concurrent processing of repositories
// using worker pool patterns, including both buffered channel and direct file
// writing approaches for memory-efficient commit processing.
package main

import (
	"fmt"
	"os"
	"sync"
)

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
//
// Deprecated: Use processRepositoriesWithDirectWrite instead.
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

// --- Concurrent Processing with Direct Write

// processRepositoriesWithDirectWrite orchestrates concurrent processing of repositories
// where each worker writes directly to the file using a synchronized writer.
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
func processRepositoriesWithDirectWrite(cfg *Config, repos []Repository, writer *SafeFileWriter) (int, int) {
	var wg sync.WaitGroup
	var mu sync.Mutex // For synchronizing counters
	totalCommits := 0
	repoCount := 0

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
		go workerWithDirectWrite(cfg, jobs, writer, &wg, &mu, &totalCommits, &repoCount)
	}

	wg.Wait()
	return totalCommits, repoCount
}

// workerWithDirectWrite is the function executed by each worker goroutine.
// It processes repositories from the job queue and writes results directly
// to the file using the thread-safe writer.
//
// Parameters:
//   - cfg: The configuration containing the authenticated user and token
//   - jobs: A read-only channel of repositories to process
//   - writer: The thread-safe file writer
//   - wg: WaitGroup counter to signal when this worker is done
//   - mu: Mutex for synchronizing counter updates
//   - totalCommits: Pointer to the total commits counter
//   - repoCount: Pointer to the repository count counter
func workerWithDirectWrite(
	cfg *Config,
	jobs <-chan Repository,
	writer *SafeFileWriter,
	wg *sync.WaitGroup,
	mu *sync.Mutex,
	totalCommits *int,
	repoCount *int,
) {
	defer wg.Done()

	for repo := range jobs {
		fmt.Fprintf(os.Stderr, "Fetching commits for %s...\n", repo.Name)

		// Fetch commits
		commits, err := fetchCommits(cfg, repo.Name)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error fetching %s: %v\n", repo.Name, err)
			continue
		}

		filtered := filterCommits(commits)
		if len(filtered) == 0 {
			continue
		}

		// Fetch README
		var readmeContent string
		if cfg.FetchReadme {
			content, err := fetchReadme(cfg, repo.Name)
			if err != nil {
				fmt.Fprintf(os.Stderr, "  Warning: Could not fetch README for %s: %v\n", repo.Name, err)
			} else {
				readmeContent = content
			}
		}

		// Write directly to file (synchronized internally)
		writer.writeRepo(cfg.Username, repo.Name, filtered, readmeContent)

		// Update counters safely
		mu.Lock()
		*totalCommits += len(filtered)
		*repoCount++
		mu.Unlock()

		fmt.Fprintf(os.Stderr, "  Written %d commits for %s\n", len(filtered), repo.Name)
	}
}
