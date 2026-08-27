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
        let mut v = Vec::new();
        $(v.push($arg.to_string());)*
        v
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
const WRITE_ID3V1: bool = true;

/// Movflags for faststart.
const MOVFLAGS_FASTSTART: &str = "+faststart";

/// Constant rate factor for x264 encoding.
const CRF_DEFAULT: u8 = 23;

/// Number of FFmpeg worker threads to auto-detect.
const THREADS_AUTO: &str = "auto";

/// Template for scaling an image into a fixed output frame with padding.
const SCALE_AND_PAD_FILTER_TEMPLATE: &str =
    "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black";

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
impl VideoConfig {
    /// Builds a video configuration by applying functional options to the defaults.
    fn from_options(options: &[VideoOption]) -> Self {
        let mut config = Self::default();
        for option in options {
            option(&mut config);
        }
        config
    }

    /// Builds the scale-and-pad filter for a static-image video.
    fn scale_and_pad_filter(&self) -> String {
        SCALE_AND_PAD_FILTER_TEMPLATE
            .replace("{width}", &self.width.to_string())
            .replace("{height}", &self.height.to_string())
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
#[derive(Debug, Default)]
pub struct FfmpegArgs {
    args: Vec<String>,
}
impl FfmpegArgs {
    /// Creates a new, empty argument builder.
    pub fn new() -> Self {
        Self::default()
    }

    // -------- Inputs / Outputs -------------------------------------------------

    /// Add an input file (`-i`).
    pub fn input(self, input_path: &Path) -> Self {
        self.option("-i", path(input_path))
    }

    /// Add an input with a format specifier (`-f` + `-i`).
    pub fn input_with_format(self, format: &str, input_path: &Path) -> Self {
        self.option("-f", format).input(input_path)
    }

    /// Add an input that is a filtergraph (just `-i` with a filter expression).
    /// Use this for lavfi inputs like "color=c=black:s=1280x720:r=1".
    pub fn input_with_filter(self, filtergraph: &str) -> Self {
        self.option("-i", filtergraph)
    }

    /// Add an output file (last argument).
    pub fn output(mut self, output_path: &Path) -> Self {
        self.args.push(path(output_path));
        self
    }

    // -------- Codecs ----------------------------------------------------------

    /// Set codec for all streams (`-c`).
    pub fn codec(self, codec: &str) -> Self {
        self.option("-c", codec)
    }

    /// Set video codec (`-c:v`).
    pub fn video_codec(self, codec: &str) -> Self {
        self.option("-c:v", codec)
    }

    /// Set audio codec (`-c:a`).
    pub fn audio_codec(self, codec: &str) -> Self {
        self.option("-c:a", codec)
    }

    // -------- Filter graphs ---------------------------------------------------

    /// Add a video filter (`-vf`).
    pub fn vf(self, filter: &str) -> Self {
        self.option("-vf", filter)
    }

    /// Add an audio filter (`-af`).
    pub fn af(self, filter: &str) -> Self {
        self.option("-af", filter)
    }

    // -------- Mapping ---------------------------------------------------------

    /// Add a map option (`-map`).
    pub fn map(self, spec: &str) -> Self {
        self.option("-map", spec)
    }

    // -------- Stream specifications -------------------------------------------

    /// Add a stream disposition (`-disposition`).
    pub fn disposition(self, stream_spec: &str, value: &str) -> Self {
        self.option(format!("-disposition:{stream_spec}"), value)
    }

    // -------- Encoding parameters ---------------------------------------------

    /// Set audio bitrate (`-b:a`), in kbps.
    pub fn audio_bitrate(self, bitrate_k: u32) -> Self {
        self.option("-b:a", format!("{bitrate_k}k"))
    }

    /// Set video bitrate (`-b:v`), in kbps.
    pub fn video_bitrate(self, bitrate_k: u32) -> Self {
        self.option("-b:v", format!("{bitrate_k}k"))
    }

    /// Set CRF value (`-crf`).
    pub fn crf(self, value: u8) -> Self {
        self.option("-crf", value.to_string())
    }

    /// Set preset (`-preset`).
    pub fn preset(self, preset: &str) -> Self {
        self.option("-preset", preset)
    }

    /// Set tune (`-tune`).
    pub fn tune(self, tune: &str) -> Self {
        self.option("-tune", tune)
    }

    /// Set pixel format (`-pix_fmt`).
    pub fn pix_fmt(self, fmt: &str) -> Self {
        self.option("-pix_fmt", fmt)
    }

    // -------- Format / container options --------------------------------------

    /// Set movflags (`-movflags`).
    pub fn movflags(self, flags: &str) -> Self {
        self.option("-movflags", flags)
    }

    /// Set ID3v2 version (`-id3v2_version`).
    pub fn id3v2_version(self, version: &str) -> Self {
        self.option("-id3v2_version", version)
    }

    /// Enable/disable writing ID3v1 tag (`-write_id3v1`).
    pub fn write_id3v1(self, yes: bool) -> Self {
        self.option("-write_id3v1", if yes { "1" } else { "0" })
    }

    // -------- Duration / frame control ----------------------------------------

    /// Set duration (`-t`).
    pub fn duration(self, seconds: f64) -> Self {
        self.option("-t", seconds.to_string())
    }

    /// Use `-shortest`.
    pub fn shortest(self) -> Self {
        self.flag("-shortest")
    }

    /// Set number of frames for a stream spec (e.g., `-frames:v`).
    pub fn frames(self, stream_spec: &str, count: u32) -> Self {
        self.option(format!("-frames:{stream_spec}"), count.to_string())
    }

    /// Loop input (`-loop`). Usually used with image inputs.
    pub fn loop_input(self, count: u32) -> Self {
        self.option("-loop", count.to_string())
    }

    /// Set input framerate (`-framerate`).
    pub fn framerate(self, fps: u8) -> Self {
        self.option("-framerate", fps.to_string())
    }

    /// Seek to a timestamp (`-ss`).
    pub fn seek(self, timestamp: &str) -> Self {
        self.option("-ss", timestamp)
    }

    // -------- Stream selection (disable) --------------------------------------

    /// Disable audio (`-an`).
    pub fn no_audio(self) -> Self {
        self.flag("-an")
    }

    /// Disable video (`-vn`).
    pub fn no_video(self) -> Self {
        self.flag("-vn")
    }

    // -------- Overwrite / force ---------------------------------------------

    /// Force overwrite (`-y`).
    pub fn overwrite(self) -> Self {
        self.flag("-y")
    }

    /// Do not overwrite (`-n`).
    pub fn no_overwrite(self) -> Self {
        self.flag("-n")
    }

    /// Set overwrite based on a bool.
    pub fn overwrite_if(self, force: bool) -> Self {
        if force {
            self.overwrite()
        } else {
            self.no_overwrite()
        }
    }

    // -------- Threads ---------------------------------------------------------

    /// Set number of threads (`-threads`).
    pub fn threads(self, n: &str) -> Self {
        self.option("-threads", n)
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
    pub fn lavfi_input(self, filtergraph: &str) -> Self {
        self.option("-f", FORMAT_LAVFI).option("-i", filtergraph)
    }

    // -------- Build -----------------------------------------------------------

    /// Consume the builder and return the final argument vector.
    pub fn build(self) -> Vec<String> {
        self.args
    }

    // -------- Internal helpers -----------------------------------------------

    /// Add a single flag option.
    fn flag(mut self, flag: &str) -> Self {
        self.args.push(flag.to_string());
        self
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
        .no_audio()
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
    let builder = FfmpegArgs::new().threads(THREADS_AUTO).input(input);
    let builder = add_optional_cover_input(builder, cover);
    let builder = builder
        .map("0:a:0")
        .audio_codec(CODEC_MP3)
        .audio_bitrate(bitrate)
        .id3v2_version(ID3V2_VERSION)
        .write_id3v1(WRITE_ID3V1);
    let builder = maybe_attach_cover_stream(builder, cover);

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
    let builder = match image {
        Some(image) => FfmpegArgs::new()
            .loop_input(1)
            .input(image)
            .vf(VF_SCALE_EVEN),
        None => FfmpegArgs::new().lavfi_input(COLOR_FILTER),
    };

    builder
        .input(audio)
        .video_codec(CODEC_H264)
        .preset(PRESET_ULTRAFAST)
        .tune(TUNE_STILLIMAGE)
        .pix_fmt(PIX_FMT_YUV420P)
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
        .crf(CRF_DEFAULT)
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
        .map("0:a?")
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
    let config = VideoConfig::from_options(options);
    let scale_filter = config.scale_and_pad_filter();

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

/// Conditionally adds a cover input to the command.
fn add_optional_cover_input(builder: FfmpegArgs, cover: Option<&Path>) -> FfmpegArgs {
    match cover {
        Some(cover) => builder.input(cover),
        None => builder,
    }
}

/// Conditionally maps and tags the cover stream.
fn maybe_attach_cover_stream(builder: FfmpegArgs, cover: Option<&Path>) -> FfmpegArgs {
    match cover {
        Some(_) => builder
            .map("1:v:0")
            .video_codec(CODEC_COPY)
            .disposition("v:0", "attached_pic"),
        None => builder,
    }
}

/// Converts a path to a lossy string suitable for FFmpeg arguments.
fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Returns `-y` or `-n` based on the `force` flag.
#[deprecated(since = "0.1.6", note = "use FfmpegArgs::overwrite_if")]
fn overwrite(force: bool) -> Vec<String> {
    args![if force { "-y" } else { "-n" }]
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
        let args = encode_mp3(
            Path::new("a.mkv"),
            Some(Path::new("c.jpg")),
            Path::new("a.mp3"),
            320,
            true,
        );

        assert!(args
            .windows(2)
            .any(|window| { window[0] == "-disposition:v:0" && window[1] == "attached_pic" }));
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
