//! module files - File discovery and companion image lookup.
//! Handles file discovery, companion image lookup, and temporary file creation.

use anyhow::{bail, Context, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    time::SystemTime,
};

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Supported image extensions for companion image lookup.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp"];

/// Suffix appended to the source stem for vidwrap outputs.
///
/// Also excluded from batch discovery so a directory scan does not
/// repeatedly re-wrap previously generated `*_with_image.mp4` files.
pub const VIDWRAP_OUTPUT_STEM_SUFFIX: &str = "_with_image";

/// Prefix prepended to each extension in error messages.
const EXTENSION_DISPLAY_PREFIX: &str = ".";

/// Fallback parent directory when a path has no parent component.
const FALLBACK_PARENT_DIR: &str = ".";

// ----------------------------------------- Public API ----------------------------------------- //

/// Finds regular files in one directory with a case-insensitive extension.
///
/// # Arguments
/// * `directory` - The directory to search.
/// * `extension` - The file extension to match (without dot).
///
/// # Returns
/// A sorted vector of matching file paths.
#[allow(dead_code)]
pub fn discover(directory: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let wanted = extension.trim_start_matches(EXTENSION_DISPLAY_PREFIX);
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(wanted))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Finds a same-basename companion image in the documented priority order.
///
/// # Arguments
/// * `video` - Path to the video file.
///
/// # Returns
/// The path to the first existing image with the same stem.
///
/// # Errors
/// Returns an error if no image is found.
pub fn companion_image(video: &Path) -> Result<PathBuf> {
    let stem = video.file_stem().context("video has no file stem")?;
    let parent = video
        .parent()
        .unwrap_or_else(|| Path::new(FALLBACK_PARENT_DIR));
    for ext in IMAGE_EXTENSIONS {
        // Append the extension to the full stem instead of using
        // `with_extension`, which would replace everything after the last
        // dot (breaking names like "a [x].video.mp4").
        let mut name = stem.to_os_string();
        name.push(".");
        name.push(ext);
        let candidate = parent.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "no image found for {} (tried: {})",
        video.display(),
        IMAGE_EXTENSIONS
            .iter()
            .map(|e| format!("{EXTENSION_DISPLAY_PREFIX}{e}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Returns true when the file stem ends with the given suffix.
///
/// This is useful for excluding generated outputs from batch discovery.
/// For example, vidwrap creates `input_with_image.mp4`, and we usually do
/// not want to re-wrap those generated files in a later batch scan.
pub fn has_stem_suffix(path: &Path, suffix: &str) -> bool {
    path.file_stem()
        .is_some_and(|stem| stem.to_string_lossy().ends_with(suffix))
}

/// Finds regular files in a directory with a case-insensitive extension,
/// canonicalizes them, and optionally excludes generated files by stem suffix.
///
/// # Arguments
/// * `directory` - Directory to scan.
/// * `extension` - File extension to match, without dot.
/// * `exclude_stem_suffix` - Optional stem suffix to exclude.
///
/// # Returns
/// A sorted vector of canonicalized matching file paths.
pub fn canonical_discover(
    directory: &Path,
    extension: &str,
    exclude_stem_suffix: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let wanted = extension.trim_start_matches(EXTENSION_DISPLAY_PREFIX);
    let mut paths = Vec::new();

    for entry in
        fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let path = entry?.path();

        if !path.is_file() {
            continue;
        }

        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(wanted))
        {
            continue;
        }

        if let Some(suffix) = exclude_stem_suffix {
            if has_stem_suffix(&path, suffix) {
                continue;
            }
        }

        // If a file cannot be canonicalized, skip it instead of failing the
        // whole batch. This keeps discovery resilient to broken symlinks.
        if let Ok(canonical) = path.canonicalize() {
            paths.push(canonical);
        }
    }

    paths.sort();
    Ok(paths)
}

/// Builds a sorted queue for all matching files in a directory.
///
/// This is used for explicit batch mode:
///
/// ```bash
/// toolkitrs vidwrap --batch
/// toolkitrs vidwrap --input-dir /videos
/// ```
pub fn queue_from_directory(
    directory: &Path,
    extension: &str,
    exclude_stem_suffix: Option<&str>,
) -> Result<Vec<PathBuf>> {
    canonical_discover(directory, extension, exclude_stem_suffix)
}

/// Builds a queue anchored by `entry`, with the anchor first and siblings after.
///
/// This is used when the user explicitly supplies one file, but we discover
/// additional related files in the same directory and ask whether to expand
/// the operation into a batch.
///
/// # Arguments
/// * `entry` - The explicit input file.
/// * `extension` - Extension to search for in the same directory.
/// * `exclude_stem_suffix` - Optional generated-output suffix to exclude.
///
/// # Returns
/// A vector with `entry` first, followed by sorted sibling files.
pub fn queue_from_entry(
    entry: &Path,
    extension: &str,
    exclude_stem_suffix: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let parent = entry
        .parent()
        .with_context(|| format!("file has no parent: {}", entry.display()))?;

    let canonical_entry = entry
        .canonicalize()
        .with_context(|| format!("file not found: {}", entry.display()))?;

    let siblings = canonical_discover(parent, extension, exclude_stem_suffix)?;

    let mut queue = Vec::with_capacity(siblings.len() + 1);
    queue.push(canonical_entry.clone());

    for path in siblings {
        if path != canonical_entry {
            queue.push(path);
        }
    }

    Ok(queue)
}

/// Checks if a path has the given extension (case-insensitive).
#[allow(dead_code)]
pub fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|e| {
        e.eq_ignore_ascii_case(extension.trim_start_matches(EXTENSION_DISPLAY_PREFIX))
    })
}

/// Creates a temporary file path with the given prefix and suffix.
///
/// Unlike `tempfile::Builder::keep()`, this does not create an empty file on disk,
/// preventing orphaned 0-byte files if the process crashes before FFmpeg writes to it.
pub fn temp_path(prefix: &str, suffix: &str) -> Result<PathBuf> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("system time before Unix epoch")?
        .as_nanos();

    let pid = process::id();
    let filename = format!("{}{}_{}{}", prefix, pid, now, suffix);

    Ok(env::temp_dir().join(filename))
}

/// Computes the vidwrap output path next to the source video.
///
/// Example: `/videos/input.mp4` -> `/videos/input_with_image.mp4`
pub fn output_path_for_video_with_image(video: &Path) -> Result<PathBuf> {
    let dir = video.parent().context("video has no parent")?;
    let mut name = video
        .file_stem()
        .context("video has no stem")?
        .to_os_string();
    name.push(VIDWRAP_OUTPUT_STEM_SUFFIX);
    name.push(".mp4");
    Ok(dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discovers_no_subdirectories() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.TS"), "").unwrap();
        fs::create_dir(dir.path().join("nested.ts")).unwrap();
        assert_eq!(discover(dir.path(), "ts").unwrap().len(), 1);
    }

    #[test]
    fn finds_priority_and_special_names() {
        let dir = tempdir().unwrap();
        let video = dir.path().join("a [x].video.mp4");
        fs::write(&video, "").unwrap();
        fs::write(dir.path().join("a [x].video.jpg"), "").unwrap();
        assert!(companion_image(&video)
            .unwrap()
            .ends_with("a [x].video.jpg"));
    }
}
