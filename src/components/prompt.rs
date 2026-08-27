//! module prompt - Interactive user prompts with injectable streams.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

// ---------------------------------------------- Types ----------------------------------------- //

/// Decision for what to do when a single video is supplied but sibling videos exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiblingBatchChoice {
    /// Process only the supplied video.
    ProcessInputOnly,

    /// Process all sibling videos; stop and report on first error.
    ProcessAllStopOnError,

    /// Process all sibling videos; skip errors and report at end.
    ProcessAllSkipOnError,

    /// Process all sibling videos; ask before continuing after each video.
    ProcessAllPromptEach,
}

/// Decision returned by a continue prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinueChoice {
    /// Continue with the next video.
    Yes,

    /// Stop the batch after this video.
    No,

    /// Continue with all remaining videos without asking again.
    YesToAll,
}

/// User's choice for execution mode when multiple files are queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Process files one at a time (sync sequential).
    Sequential,
    /// Process files in parallel using N cores.
    Parallel,
}

/// User's choice for residual file cleanup after a canceled batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupChoice {
    /// Remove all partial output files from cancelled tasks.
    Remove,
    /// Keep them as-is.
    Keep,
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Parse a yes/no response, falling back to `default` for unknown input.
#[allow(dead_code)]
pub fn parse_yes_no(value: &str, default: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    }
}

/// Parse a one-based choice, falling back to the one-based default.
pub fn parse_choice(value: &str, choices: usize, default: usize) -> usize {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=choices).contains(n))
        .unwrap_or(default)
}

/// Prompt for a choice from a list, using injectable input/output streams.
///
/// It is written with injectable streams, keeping interactive code testable.
/// Automatically falls back to the default choice if stdin is not a terminal (CI/CD safety).
///
/// # Arguments
/// * `input` - Readable stream for user input.
/// * `output` - Writable stream for the prompt.
/// * `question` - The question to display.
/// * `options` - List of option labels.
/// * `default` - Default option index (1-based).
///
/// # Returns
/// The chosen index (1-based) or the default on error.
pub fn choice<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    question: &str,
    options: &[&str],
    default: usize,
) -> io::Result<usize> {
    // CI/CD Safety: If stdin isn't a terminal, we can't wait for user input.
    // Auto-select the default to prevent the pipeline from hanging indefinitely.
    if !io::stdin().is_terminal() {
        writeln!(output, "{question}")?;
        writeln!(
            output,
            "  ⚙️  Non-interactive mode detected. Auto-selecting default: {default}"
        )?;
        return Ok(default);
    }

    writeln!(output, "{question}")?;
    for (index, option) in options.iter().enumerate() {
        writeln!(output, "  {}. {option}", index + 1)?;
    }
    write!(output, "Choice [{default}]: ")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(parse_choice(&line, options.len(), default))
}

/// Parses a sibling-batch selection.
///
/// Accepts numbers and convenient words:
/// - `1`, `only`, `input`, `single`
/// - `2`, `stop`
/// - `3`, `skip`
/// - `4`, `prompt`, `ask`, `each`
fn parse_sibling_choice(value: &str, default: SiblingBatchChoice) -> SiblingBatchChoice {
    let normalized = value.trim().to_ascii_lowercase();

    match normalized.as_str() {
        "" => default,
        "1" | "only" | "input" | "single" => SiblingBatchChoice::ProcessInputOnly,
        "2" | "stop" | "error" => SiblingBatchChoice::ProcessAllStopOnError,
        "3" | "skip" => SiblingBatchChoice::ProcessAllSkipOnError,
        "4" | "prompt" | "ask" | "each" => SiblingBatchChoice::ProcessAllPromptEach,
        _ => default,
    }
}

/// Parses a continue selection.
///
/// Accepts numbers and words:
/// - `1`, `y`, `yes`
/// - `2`, `n`, `no`
/// - `3`, `a`, `all`, `yes to all`
fn parse_continue(value: &str, default: ContinueChoice) -> ContinueChoice {
    let normalized = value.trim().to_ascii_lowercase();

    match normalized.as_str() {
        "" => default,
        "1" | "y" | "yes" => ContinueChoice::Yes,
        "2" | "n" | "no" => ContinueChoice::No,
        "3" | "a" | "all" | "yes to all" | "yes-to-all" => ContinueChoice::YesToAll,
        _ => default,
    }
}

/// Ask what to do when more videos are discovered next to an explicit input.
///
/// This is used when the user runs something like:
///
/// ```bash
/// toolkitrs vidwrap input.mp4
/// ```
///
/// and `input.mp4` lives in a directory containing additional `.mp4` files.
///
/// Non-interactive fallback:
/// - If stdin is not a terminal, we safely process only the supplied input.
pub fn sibling_batch_choice<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    directory: &std::path::Path,
    count: usize,
) -> io::Result<SiblingBatchChoice> {
    const DEFAULT: SiblingBatchChoice = SiblingBatchChoice::ProcessAllPromptEach;

    // CI/CD safety: do not expand a single explicit file into a large batch
    // unless the user can actually answer the prompt.
    if !io::stdin().is_terminal() {
        writeln!(
            output,
            "Found {count} MP4 videos in {}",
            directory.display()
        )?;
        writeln!(
            output,
            "  ⚙️  Non-interactive mode detected. Processing the supplied video only."
        )?;
        return Ok(SiblingBatchChoice::ProcessInputOnly);
    }

    writeln!(
        output,
        "Found {count} MP4 videos in {}",
        directory.display()
    )?;
    writeln!(output, "How would you like to proceed?")?;
    writeln!(output, "  1. Process the input only")?;
    writeln!(
        output,
        "  2. Process the whole path (return on error, report at the end)"
    )?;
    writeln!(
        output,
        "  3. Process the whole path (skip on error, report at the end)"
    )?;
    writeln!(
        output,
        "  4. Process the whole path (prompt user on each new video)"
    )?;
    write!(output, "Choice [4]: ")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;

    Ok(parse_sibling_choice(&line, DEFAULT))
}

/// Ask whether to continue to the next video in a prompt-each batch.
///
/// Options:
/// - yes
/// - no
/// - yes to all
///
/// Non-interactive fallback:
/// - If stdin is not a terminal, we continue with all remaining videos to
///   avoid hanging the pipeline.
pub fn continue_to_next<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    next: &std::path::Path,
) -> io::Result<ContinueChoice> {
    const DEFAULT: ContinueChoice = ContinueChoice::Yes;

    if !io::stdin().is_terminal() {
        writeln!(output, "Next video: {}", next.display())?;
        writeln!(
            output,
            "  ⚙️  Non-interactive mode detected. Continuing with all remaining videos."
        )?;
        return Ok(ContinueChoice::YesToAll);
    }

    writeln!(output, "Next video: {}", next.display())?;
    writeln!(output, "Continue?")?;
    writeln!(output, "  1. yes")?;
    writeln!(output, "  2. no")?;
    writeln!(output, "  3. yes to all")?;
    write!(output, "Choice [1]: ")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;

    Ok(parse_continue(&line, DEFAULT))
}

/// Asks the user whether to run in parallel or sequential mode.
///
/// Non-interactive fallback: returns `Sequential` (safer for resource usage).
pub fn execution_mode_choice<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    file_count: usize,
    cores: usize,
) -> io::Result<ExecutionMode> {
    if !io::stdin().is_terminal() {
        writeln!(
            output,
            "Found {file_count} files. Non-interactive mode: defaulting to sequential."
        )?;
        return Ok(ExecutionMode::Sequential);
    }

    writeln!(output, "Found {file_count} files to process.")?;
    writeln!(output, "How would you like to run them?")?;
    writeln!(output, "  1. Parallel (using {cores} cores)")?;
    writeln!(output, "  2. Sequential (one by one)")?;
    write!(output, "Choice [1]: ")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;

    Ok(match line.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "parallel" => ExecutionMode::Parallel,
        "2" | "sequential" | "seq" => ExecutionMode::Sequential,
        _ => ExecutionMode::Parallel,
    })
}

/// Asks the user whether to remove residual partial output files after a
/// canceled batch.
///
/// Non-interactive fallback: keeps the files (no destructive default).
pub fn cleanup_residual_choice<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    residual_files: &[PathBuf],
) -> io::Result<CleanupChoice> {
    if residual_files.is_empty() {
        return Ok(CleanupChoice::Keep);
    }

    if !io::stdin().is_terminal() {
        writeln!(
            output,
            "Cancelled batch left {} partial files. Keeping them.",
            residual_files.len()
        )?;
        return Ok(CleanupChoice::Keep);
    }

    writeln!(
        output,
        "Cancelled batch left {} partial files:",
        residual_files.len()
    )?;
    for path in residual_files.iter().take(10) {
        writeln!(output, "  • {}", path.display())?;
    }
    if residual_files.len() > 10 {
        writeln!(output, "  ... and {} more", residual_files.len() - 10)?;
    }
    writeln!(output, "Remove these files?")?;
    writeln!(output, "  1. Yes, remove them")?;
    writeln!(output, "  2. No, keep them")?;
    write!(output, "Choice [1]: ")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;

    Ok(match line.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "y" | "yes" | "remove" => CleanupChoice::Remove,
        "2" | "n" | "no" | "keep" => CleanupChoice::Keep,
        _ => CleanupChoice::Remove,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_is_safe() {
        assert!(parse_yes_no("yes", false));
        assert!(!parse_yes_no("?", false));
        assert_eq!(parse_choice("9", 3, 2), 2);
    }

    #[test]
    fn parses_sibling_choices() {
        let default = SiblingBatchChoice::ProcessAllPromptEach;

        assert_eq!(
            parse_sibling_choice("1", default),
            SiblingBatchChoice::ProcessInputOnly
        );
        assert_eq!(
            parse_sibling_choice("only", default),
            SiblingBatchChoice::ProcessInputOnly
        );
        assert_eq!(
            parse_sibling_choice("2", default),
            SiblingBatchChoice::ProcessAllStopOnError
        );
        assert_eq!(
            parse_sibling_choice("skip", default),
            SiblingBatchChoice::ProcessAllSkipOnError
        );
        assert_eq!(
            parse_sibling_choice("", default),
            SiblingBatchChoice::ProcessAllPromptEach
        );
        assert_eq!(
            parse_sibling_choice("wat", default),
            SiblingBatchChoice::ProcessAllPromptEach
        );
    }

    #[test]
    fn parses_continue_choices() {
        let default = ContinueChoice::Yes;

        assert_eq!(parse_continue("y", default), ContinueChoice::Yes);
        assert_eq!(parse_continue("no", default), ContinueChoice::No);
        assert_eq!(
            parse_continue("yes to all", default),
            ContinueChoice::YesToAll
        );
        assert_eq!(parse_continue("3", default), ContinueChoice::YesToAll);
        assert_eq!(parse_continue("", default), ContinueChoice::Yes);
        assert_eq!(parse_continue("??", default), ContinueChoice::Yes);
    }
}
