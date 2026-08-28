// Package main provides a tool to fetch and export GitHub contributions
// (commits) for a specified user within a date range.
//
// It uses the GitHub API with token authentication, processes repositories
// concurrently with a worker pool, and writes the results to a text file.
//
// Memory Usage With Direct File Writing (mutex-synchronized worker pool):
//
//	Worker1 ──┐
//	Worker2 ──┼──> [Mutex] ──> File
//	Worker3 ──┘
//			  ▲
//			  └── Each worker writes immediately, then discards data.
//			      No channel buffer means no accumulation of results in memory.
//
// The writer.SafeFileWriter uses a mutex to ensure only one worker writes to the
// file at a time, preventing corruption while maintaining memory efficiency.
//
// Memory Usage With Buffered Channel (worker pool pattern) -- Deprecated:
//
//	Worker1 ──┐
//	Worker2 ──┼──> [Channel Buffer] ──> Main Loop ──> File
//	Worker3 ──┘
//			  ▲
//			  └── All results sit here if main loop is slow
package main

// main is the entry point of the program. It parses command-line arguments,
// validates configuration, and executes the contribution fetching process.
func main() { Execute() }
