//! module workflow - Static metadata describing each media workflow the TUI can run.
//! Keeps the TUI open for extension: adding a workflow here is the only change needed.

/// The media workflows available in the TUI home screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workflow {
    /// Remux TS → MP4 (stream copy).
    Ts2Mp4,
    /// Extract audio MKV → MP3.
    Mkv2Mp3,
    /// Create video MP3 → MP4.
    Mp32Mp4,
    /// Wrap a video with its companion image.
    Vidwrap,
}
impl Workflow {
    /// Returns every workflow in display order.
    pub fn all() -> Vec<Workflow> {
        vec![
            Workflow::Ts2Mp4,
            Workflow::Mkv2Mp3,
            Workflow::Mp32Mp4,
            Workflow::Vidwrap,
        ]
    }

    /// Short title shown in the home list.
    pub fn title(&self) -> &'static str {
        match self {
            Workflow::Ts2Mp4 => "ts2mp4",
            Workflow::Mkv2Mp3 => "mkv2mp3",
            Workflow::Mp32Mp4 => "mp32mp4",
            Workflow::Vidwrap => "vidwrap",
        }
    }

    /// One-line description shown next to the title.
    pub fn description(&self) -> &'static str {
        match self {
            Workflow::Ts2Mp4 => "Remux TS to MP4 (no re-encode)",
            Workflow::Mkv2Mp3 => "Extract audio from MKV to MP3",
            Workflow::Mp32Mp4 => "Turn MP3 into MP4 with cover art",
            Workflow::Vidwrap => "Wrap video with a companion image",
        }
    }

    /// File extension the picker should show for this workflow.
    pub fn input_extension(&self) -> &'static str {
        match self {
            Workflow::Ts2Mp4 => "ts",
            Workflow::Mkv2Mp3 => "mkv",
            Workflow::Mp32Mp4 => "mp3",
            Workflow::Vidwrap => "mp4",
        }
    }

    /// Whether this workflow uses audio encoding options (bitrate).
    pub fn uses_bitrate(&self) -> bool {
        matches!(self, Workflow::Mkv2Mp3 | Workflow::Mp32Mp4)
    }

    /// Whether this workflow extracts cover art (needs cover size).
    pub fn uses_cover_size(&self) -> bool {
        matches!(self, Workflow::Mkv2Mp3 | Workflow::Mp32Mp4)
    }
}
