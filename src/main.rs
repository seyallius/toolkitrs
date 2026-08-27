//! toolkitrs - FFmpeg workflows for common media tasks.

mod cli;
mod commands;
mod components;
mod ffmpeg;
mod tui;
mod util;
mod workflow;

use anyhow::Result;
use clap::Parser;

/// Entry point of the toolkitrs.
fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    commands::run(cli)
}
