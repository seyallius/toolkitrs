//! module args - Builders for FFmpeg command-line arguments.
#![allow(dead_code)]

use std::path::Path;

// -------------------------------------------- Macro ------------------------------------------- //

/// Create a Vec<String> from string literals and values that implement Display.
///
/// # Examples
/// ```
/// let input = Path::new("video.mp4");
/// let args = args!["-i", input, "-c", "copy"];
/// assert_eq!(args, vec!["-i", "video.mp4", "-c", "copy"]);
/// ```
#[macro_export]
macro_rules! args {
    // Base case: empty
    () => {
        Vec::<String>::new()
    };
    // One or more arguments
    ($($arg:expr),+ $(,)?) => {{
        vec![$($arg.to_string()),+]
    }};
}

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Codec name for stream copy (no re-encoding).
const CODEC_COPY: &str = "copy";

/// Codec name for MP3 encoding via LAME.
const CODEC_MP3: &str = "libmp3lame";

/// Codec name for H.264 encoding.
const CODEC_H264: &str = "libx264";

/// Codec name for AAC encoding.
const CODEC_AAC: &str = "aac";

/// Preset for ultrafast encoding.
const PRESET_ULTRAFAST: &str = "ultrafast";

/// Tune for still image video.
const TUNE_STILLIMAGE: &str = "stillimage";

/// Pixel format for YUV 4:2:0 planar.
const PIX_FMT_YUV420P: &str = "yuv420p";

/// Pixel format for YUVJ 4:2:0 planar (JPEG).
const PIX_FMT_YUVJ420P: &str = "yuvj420p";

/// Format name for lavfi (libavfilter input).
const FORMAT_LAVFI: &str = "lavfi";

/// Filtergraph for a black video with 1280x720 resolution, 1 frame per second.
const COLOR_FILTER: &str = "color=c=black:s=1280x720:r=1";

/// Filtergraph to scale to even dimensions only (no SAR or format conversion).
const VF_SCALE_EVEN_ONLY: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

/// Filtergraph to scale to even dimensions, set SAR to 1, and convert to yuv420p.
const VF_SCALE_EVEN: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2,setsar=1,format=yuv420p";

/// Filtergraph for scaling an image to a square size while preserving aspect ratio, then to rgb24.
const VF_SCALE_SQUARE_TEMPLATE: &str =
    "scale={size}:{size}:force_original_aspect_ratio=decrease,format=rgb24";

/// ID3v2 version to use.
const ID3V2_VERSION: &str = "3";

/// Write ID3v1 tag as well.
const WRITE_ID3V1: &str = "1";

/// Movflags for faststart.
const MOVFLAGS_FASTSTART: &str = "+faststart";

/// Constant rate factor for x264 encoding.
const CRF_DEFAULT: &str = "23";

/// Default output width for the generated video.
const WIDTH: u16 = 1980;

/// Default output height for the generated video.
const HEIGHT: u16 = 1080;

/// Default frames per second for the generated video.
const FRAME_RATE: u8 = 30;

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Configuration for video generation parameters.
///
/// Contains the dimensions and framerate settings used when creating a static image video.
/// Default values are provided for all fields.
#[derive(Debug, Clone)]
pub struct VideoConfig {
    /// Target width in pixels (default: 1980)
    pub width: u16,
    /// Target height in pixels (default: 1080)
    pub height: u16,
    /// Frames per second (default: 30)
    pub framerate: u8,
}
impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            width: WIDTH,
            height: HEIGHT,
            framerate: FRAME_RATE,
        }
    }
}

/// A function type that modifies a `VideoConfig` instance.
///
/// Used with the functional options pattern to apply one or more
/// configuration overrides to the default video settings.
pub type VideoOption = Box<dyn Fn(&mut VideoConfig)>;

/// A fluent builder for constructing FFmpeg CLI arguments.
///
/// This pattern is more idiomatic in Rust than macros, provides better
/// type safety, and makes it easier to conditionally add arguments.
#[derive(Default)]
pub struct FfmpegArgs {
    args: Vec<String>,
}
impl FfmpegArgs {
    /// Creates a new, empty argument builder.
    pub fn new() -> Self {
        Self { args: Vec::new() }
    }

    // -------- Inputs / Outputs -------------------------------------------------

    /// Add an input file (`-i`).
    pub fn input(mut self, input_path: &Path) -> Self {
        self.args.extend(args!["-i", path(input_path)]);
        self
    }

    /// Add an input with a format specifier (`-f` + `-i`).
    pub fn input_with_format(mut self, format: &str, input_path: &Path) -> Self {
        self.args
            .extend(args!["-f", format, "-i", path(input_path)]);
        self
    }

    /// Add an input that is a filtergraph (just `-i` with a filter expression).
    /// Use this for lavfi inputs like "color=c=black:s=1280x720:r=1".
    pub fn input_with_filter(mut self, filtergraph: &str) -> Self {
        self.args.extend(args!["-i", filtergraph]);
        self
    }

    /// Add an output file (last argument).
    pub fn output(mut self, input_path: &Path) -> Self {
        self.args.push(path(input_path));
        self
    }

    // -------- Codecs ----------------------------------------------------------

    /// Set codec for all streams (`-c`).
    pub fn codec(mut self, codec: &str) -> Self {
        self.args.extend(args!["-c", codec]);
        self
    }

    /// Set video codec (`-c:v`).
    pub fn video_codec(mut self, codec: &str) -> Self {
        self.args.extend(args!["-c:v", codec]);
        self
    }

    /// Set audio codec (`-c:a`).
    pub fn audio_codec(mut self, codec: &str) -> Self {
        self.args.extend(args!["-c:a", codec]);
        self
    }

    // -------- Filter graphs ---------------------------------------------------

    /// Add a video filter (`-vf`).
    pub fn vf(mut self, filter: &str) -> Self {
        self.args.extend(args!["-vf", filter]);
        self
    }

    /// Add an audio filter (`-af`).
    pub fn af(mut self, filter: &str) -> Self {
        self.args.extend(args!["-af", filter]);
        self
    }

    // -------- Mapping ---------------------------------------------------------

    /// Add a map option (`-map`).
    pub fn map(mut self, spec: &str) -> Self {
        self.args.extend(args!["-map", spec]);
        self
    }

    // -------- Stream specifications -------------------------------------------

    /// Add a stream disposition (`-disposition:<spec> <value>`).
    ///
    /// The stream specifier belongs on the option name — ffmpeg rejects
    /// the combined `-disposition v:0:attached_pic` form.
    pub fn disposition(mut self, stream_spec: &str, value: &str) -> Self {
        self.args
            .extend(args![format!("-disposition:{}", stream_spec), value]);
        self
    }

    // -------- Encoding parameters ---------------------------------------------

    /// Set audio bitrate (`-b:a`), in kbps.
    pub fn audio_bitrate(mut self, bitrate_k: u32) -> Self {
        self.args.extend(args!["-b:a", format!("{}k", bitrate_k)]);
        self
    }

    /// Set video bitrate (`-b:v`), in kbps.
    pub fn video_bitrate(mut self, bitrate_k: u32) -> Self {
        self.args.extend(args!["-b:v", format!("{}k", bitrate_k)]);
        self
    }

    /// Set CRF value (`-crf`).
    pub fn crf(mut self, value: u8) -> Self {
        self.args.extend(args!["-crf", value.to_string()]);
        self
    }

    /// Set preset (`-preset`).
    pub fn preset(mut self, preset: &str) -> Self {
        self.args.extend(args!["-preset", preset]);
        self
    }

    /// Set tune (`-tune`).
    pub fn tune(mut self, tune: &str) -> Self {
        self.args.extend(args!["-tune", tune]);
        self
    }

    /// Set pixel format (`-pix_fmt`).
    pub fn pix_fmt(mut self, fmt: &str) -> Self {
        self.args.extend(args!["-pix_fmt", fmt]);
        self
    }

    // -------- Format / container options --------------------------------------

    /// Set movflags (`-movflags`).
    pub fn movflags(mut self, flags: &str) -> Self {
        self.args.extend(args!["-movflags", flags]);
        self
    }

    /// Set ID3v2 version (`-id3v2_version`).
    pub fn id3v2_version(mut self, version: &str) -> Self {
        self.args.extend(args!["-id3v2_version", version]);
        self
    }

    /// Enable/disable writing ID3v1 tag (`-write_id3v1`).
    pub fn write_id3v1(mut self, yes: bool) -> Self {
        self.args
            .extend(args!["-write_id3v1", if yes { "1" } else { "0" }]);
        self
    }

    // -------- Duration / frame control ----------------------------------------

    /// Set duration (`-t`).
    pub fn duration(mut self, seconds: f64) -> Self {
        self.args.extend(args!["-t", seconds.to_string()]);
        self
    }

    /// Use `-shortest`.
    pub fn shortest(mut self) -> Self {
        self.args.push("-shortest".into());
        self
    }

    /// Set number of frames for a stream spec (e.g., `-frames:v`).
    pub fn frames(mut self, stream_spec: &str, count: u32) -> Self {
        self.args
            .extend(args![format!("-frames:{}", stream_spec), count.to_string()]);
        self
    }

    /// Loop input (`-loop`). Usually used with image inputs.
    pub fn loop_input(mut self, count: u32) -> Self {
        self.args.extend(args!["-loop", count.to_string()]);
        self
    }

    /// Set input framerate (`-framerate`).
    pub fn framerate(mut self, fps: u8) -> Self {
        self.args.extend(args!["-framerate", fps.to_string()]);
        self
    }

    /// Seek to a timestamp (`-ss`).
    pub fn seek(mut self, timestamp: &str) -> Self {
        self.args.extend(args!["-ss", timestamp]);
        self
    }

    // -------- Stream selection (disable) --------------------------------------

    /// Disable audio (`-an`).
    pub fn no_audio(mut self) -> Self {
        self.args.push("-an".into());
        self
    }

    /// Disable video (`-vn`).
    pub fn no_video(mut self) -> Self {
        self.args.push("-vn".into());
        self
    }

    // -------- Overwrite / force ---------------------------------------------

    /// Force overwrite (`-y`).
    pub fn overwrite(mut self) -> Self {
        self.args.push("-y".into());
        self
    }

    /// Do not overwrite (`-n`).
    pub fn no_overwrite(mut self) -> Self {
        self.args.push("-n".into());
        self
    }

    /// Set overwrite based on a bool.
    pub fn overwrite_if(mut self, force: bool) -> Self {
        if force {
            self.args.push("-y".into());
        } else {
            self.args.push("-n".into());
        }
        self
    }

    // -------- Threads ---------------------------------------------------------

    /// Set number of threads (`-threads`).
    pub fn threads(mut self, n: &str) -> Self {
        self.args.extend(args!["-threads", n]);
        self
    }

    // -------- Generic raw argument --------------------------------------------

    /// Add any raw argument (for options not yet covered).
    pub fn arg<S: Into<String>>(mut self, arg: S) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add a key‑value option (e.g. `("-ss", "00:00:01")`).
    pub fn option<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.args.extend([key.into(), value.into()]);
        self
    }

    // -------- Special inputs ------------------------------------------------

    /// Add a lavfi (libavfilter) input: `-f lavfi -i <filtergraph>`.
    /// Use this for filtergraph inputs like "color=c=black:s=1280x720:r=1".
    #[rustfmt::skip]
    pub fn lavfi_input(mut self, filtergraph: &str) -> Self {
        self.args.extend(args![
            "-f", FORMAT_LAVFI,
            "-i", filtergraph,
        ]);
        self
    }

    // -------- Build -----------------------------------------------------------

    /// Consume the builder and return the final argument vector.
    pub fn build(self) -> Vec<String> {
        self.args
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Stream-copy a transport stream into an MP4 container.
pub fn remux_copy(input: &Path, output: &Path, force: bool) -> Vec<String> {
    FfmpegArgs::new()
        .input(input)
        .codec(CODEC_COPY)
        .overwrite_if(force)
        .output(output)
        .build()
}

/// Extract a single scaled frame for album art.
pub fn extract_frame(input: &Path, output: &Path, size: u32) -> Vec<String> {
    let filter = VF_SCALE_SQUARE_TEMPLATE.replace("{size}", &size.to_string());
    FfmpegArgs::new()
        .seek("00:00:01")
        .input(input)
        .frames("v", 1)
        .vf(&filter)
        .overwrite()
        .output(output)
        .build()
}

/// Extract an MP3 attached picture without decoding it.
pub fn extract_embedded_cover(input: &Path, output: &Path) -> Vec<String> {
    FfmpegArgs::new()
        .input(input)
        .no_audio() // -an
        .video_codec(CODEC_COPY)
        .overwrite()
        .output(output)
        .build()
}

/// Encode the first audio track as tagged MP3, optionally adding cover art.
pub fn encode_mp3(
    input: &Path,
    cover: Option<&Path>,
    output: &Path,
    bitrate: u32,
    force: bool,
) -> Vec<String> {
    let mut builder = FfmpegArgs::new().threads("auto").input(input);

    if let Some(cover) = cover {
        builder = builder.input(cover);
    }

    builder = builder
        .map("0:a:0")
        .audio_codec(CODEC_MP3)
        .audio_bitrate(bitrate)
        .id3v2_version(ID3V2_VERSION)
        .write_id3v1(true);

    if cover.is_some() {
        builder = builder
            .map("1:v:0")
            .video_codec(CODEC_COPY)
            .disposition("v:0", "attached_pic");
    }

    builder.overwrite_if(force).output(output).build()
}

/// Produce an H.264 MP4 from cover art (or a black video) and MP3 audio.
pub fn encode_mp4(
    image: Option<&Path>,
    audio: &Path,
    output: &Path,
    bitrate: u32,
    force: bool,
) -> Vec<String> {
    let mut builder = FfmpegArgs::new();

    if let Some(image) = image {
        builder = builder.loop_input(1).input(image);
    } else {
        builder = builder.lavfi_input(COLOR_FILTER)
    }

    builder = builder
        .input(audio)
        .video_codec(CODEC_H264)
        .preset(PRESET_ULTRAFAST)
        .tune(TUNE_STILLIMAGE)
        .pix_fmt(PIX_FMT_YUV420P);

    if image.is_some() {
        builder = builder.vf(VF_SCALE_EVEN);
    }

    builder
        .audio_codec(CODEC_AAC)
        .audio_bitrate(bitrate)
        .shortest()
        .movflags(MOVFLAGS_FASTSTART)
        .overwrite_if(force)
        .output(output)
        .build()
}

/// Convert an image to an even-dimension JPEG.
pub fn image_to_jpg(input: &Path, output: &Path) -> Vec<String> {
    FfmpegArgs::new()
        .input(input)
        .vf(VF_SCALE_EVEN_ONLY)
        .pix_fmt(PIX_FMT_YUVJ420P)
        .overwrite()
        .output(output)
        .build()
}

/// Copy A/V streams and omit attached pictures/metadata.
/// It returns arguments to copy streams without metadata.
/// Removes existing embedded thumbnails while preserving A/V content.
pub fn strip_thumbnail_args(input: &Path, output: &Path) -> Vec<String> {
    FfmpegArgs::new()
        .input(input)
        .map("0:v")
        .map("0:a")
        .codec(CODEC_COPY)
        .overwrite()
        .output(output)
        .build()
}

/// Encode a looped image with the supplied media's audio.
///
/// Creates a video where a still image is displayed for the entire
/// duration of the audio track. Uses ultrafast preset and CRF 23
/// for quick encoding suitable for thumbnails/previews.
pub fn encode_loop_args(image: &Path, media: &Path, output: &Path) -> Vec<String> {
    FfmpegArgs::new()
        .loop_input(1)
        .input(image)
        .input(media)
        .video_codec(CODEC_H264)
        .preset(PRESET_ULTRAFAST)
        .crf(23)
        .audio_codec(CODEC_COPY)
        .shortest()
        .overwrite()
        .output(output)
        .build()
}

/// Attach a JPEG as an MP4 thumbnail.
///
/// It Sets proper disposition for media player thumbnail recognition.
pub fn attach_thumbnail_args(video: &Path, image: &Path, output: &Path) -> Vec<String> {
    FfmpegArgs::new()
        .input(video)
        .input(image)
        .map("0:v")
        .map("0:a?") // optional audio
        .map("1")
        .codec(CODEC_COPY)
        .disposition("v:1", "attached_pic")
        .overwrite()
        .output(output)
        .build()
}

/// Replaces a video stream with a static image while preserving audio.
///
/// Takes a video file and an image, creates a new video where the image is displayed
/// for the entire duration of the original video's audio track. The image is scaled
/// to fit within the target dimensions while preserving aspect ratio, then padded
/// to exactly fill the frame. Useful for creating static visualizers or replacing
/// video content while keeping the audio.
///
/// # Arguments
/// * `image` - Path to the source image to use as the static video
/// * `video` - Path to the source video file providing the audio stream
/// * `output` - Destination path for the encoded video
/// * `options` - Slice of `VideoOption` functions to override default settings
///
/// # Returns
/// Vector of FFmpeg CLI arguments ready to replace video with a static image.
///
/// # Defaults
/// If no options are provided, the following defaults are used:
/// - Width: 1980 pixels
/// - Height: 1080 pixels
/// - Framerate: 30 fps
///
/// # Examples
/// ```
/// use std::path::Path;
///
/// // Use all defaults
/// let args = replace_video_with_image(
///     Path::new("thumbnail.png"),
///     Path::new("input_video.mp4"),
///     Path::new("output.mp4"),
///     &[],  // No options = defaults
/// );
///
/// // Override only the width
/// let args = replace_video_with_image(
///     Path::new("thumbnail.png"),
///     Path::new("input_video.mp4"),
///     Path::new("output.mp4"),
///     &[with_width(1920)],
/// );
///
/// // Override multiple parameters
/// let args = replace_video_with_image(
///     Path::new("thumbnail.png"),
///     Path::new("input_video.mp4"),
///     Path::new("output.mp4"),
///     &[with_width(1280), with_height(720), with_framerate(60)],
/// );
/// ```
pub fn replace_video_with_image(
    image: &Path,
    video: &Path,
    output: &Path,
    options: &[VideoOption],
) -> Vec<String> {
    let mut config = VideoConfig::default();
    for opt in options {
        opt(&mut config);
    }

    let scale_filter = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:black",
        config.width, config.height, config.width, config.height
    );

    FfmpegArgs::new()
        .loop_input(1)
        .framerate(config.framerate)
        .input(image)
        .input(video)
        .map("0:v")
        .map("1:a")
        .vf(&scale_filter)
        .video_codec(CODEC_H264)
        .preset(PRESET_ULTRAFAST)
        .tune(TUNE_STILLIMAGE)
        .pix_fmt(PIX_FMT_YUV420P)
        .audio_codec(CODEC_COPY)
        .movflags(MOVFLAGS_FASTSTART)
        .shortest()
        .overwrite()
        .output(output)
        .build()
}

/// Sets the output video width.
///
/// # Arguments
/// * `w` - Width in pixels
///
/// # Example
/// ```
/// let opts = &[with_width(1280)];
/// replace_video_with_image(&image, &video, &output, opts);
/// ```
pub fn with_width(w: u16) -> VideoOption {
    Box::new(move |cfg| cfg.width = w)
}

/// Sets the output video height.
///
/// # Arguments
/// * `h` - Height in pixels
///
/// # Example
/// ```
/// let opts = &[with_height(720)];
/// replace_video_with_image(&image, &video, &output, opts);
/// ```
pub fn with_height(h: u16) -> VideoOption {
    Box::new(move |cfg| cfg.height = h)
}

/// Sets the output video framerate.
///
/// # Arguments
/// * `f` - Frames per second
///
/// # Example
/// ```
/// let opts = &[with_framerate(60)];
/// replace_video_with_image(&image, &video, &output, opts);
/// ```
pub fn with_framerate(f: u8) -> VideoOption {
    Box::new(move |cfg| cfg.framerate = f)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Converts a path to a lossy string suitable for FFmpeg arguments.
fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn remux_uses_copy() {
        assert_eq!(
            remux_copy(Path::new("a.ts"), Path::new("out/a.mp4"), false),
            vec!["-i", "a.ts", "-c", CODEC_COPY, "-n", "out/a.mp4"]
        );
    }
    #[test]
    fn mp3_with_cover_maps_picture() {
        let a = encode_mp3(
            Path::new("a.mkv"),
            Some(Path::new("c.jpg")),
            Path::new("a.mp3"),
            320,
            true,
        );
        // check that -disposition:v:0 attached_pic appears somewhere
        assert!(a
            .windows(2)
            .any(|x| x == ["-disposition:v:0", "attached_pic"]));
    }

    #[test]
    fn builder_works() {
        let args = FfmpegArgs::new()
            .input(Path::new("in.mp4"))
            .codec("copy")
            .output(Path::new("out.mp4"))
            .build();
        assert_eq!(args, vec!["-i", "in.mp4", "-c", "copy", "out.mp4"]);
    }
}
