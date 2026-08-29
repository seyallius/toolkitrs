//! module github - GitHub API client for fetching contributions.
//!
//! This module provides the domain logic for interacting with the GitHub REST
//! API: fetching repositories, commits, and README content. It mirrors the
//! structure of the Go `gh-contributions` tool while following Rust idioms
//! and the toolkitrs project style.
//!
//! Responsibilities are split:
//!   * `types`     — data structures and constants
//!   * `config`    — configuration creation and validation
//!   * `api`       — HTTP interactions with GitHub
//!   * `filter`    — commit filtering and message processing
//!   * `writer`    — thread-safe file output
//!   * `processor` — concurrent repository processing

pub mod api;
pub mod config;
pub mod filter;
pub mod processor;
pub mod tui;
pub mod types;
pub mod writer;
