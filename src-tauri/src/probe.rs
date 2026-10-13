use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use crate::error::AppError;
use crate::proc::{run, strings};
use crate::smartcut;

/// Everything the UI needs to lay out a timeline for a file.
///
/// All times are **0-based**: `start_time` has already been subtracted, so
/// these line up with both `<video>.currentTime` and ffmpeg's `-ss`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoMetadata {
    pub path: String,
    pub duration: f64,
    /// Container start offset that was normalised away (0 for most files).
    pub start_time: f64,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub container: String,
    pub size_bytes: u64,
    pub frame_rate: Option<f64>,
    /// True when the video uses B-frames. Surfaced because frame reordering is
    /// what makes Matroska seeking imprecise — see `trim::trim_args`.
    pub has_b_frames: bool,
    /// Whether a frame-accurate export can handle this file, and if not, why.
    pub frame_accurate: FrameAccurateSupport,
}

/// Serialised as `{ status: "supported" }` or
/// `{ status: "unsupported", reason }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FrameAccurateSupport {
    Supported,
    Unsupported { reason: String },
}

pub fn metadata_args(path: &str) -> Vec<String> {
    strings(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", path])
}

/// Packets of one stream, one CSV row each, with only the `packet=` fields in
/// `entries`. `interval` is an ffprobe `-read_intervals` spec; without one the
/// whole file is read.
///
/// `-show_packets` reads container packet headers and never decodes, which is
/// what keeps listing them tractable on long files.
pub fn packet_args(path: &str, stream: &str, interval: Option<&str>, entries: &str) -> Vec<String> {
    [
        strings(["-v", "error", "-select_streams", stream]),
        interval.map_or_else(Vec::new, |i| strings(["-read_intervals", i])),
        strings(["-show_packets", "-show_entries", &format!("packet={entries}")]),
        strings(["-of", "csv=print_section=0", path]),
    ]
    .concat()
}

pub(crate) fn parse_json(json: &str) -> Result<Value, AppError> {
    serde_json::from_str(json).map_err(|e| AppError::Unparsable {
        tool: "ffprobe",
        detail: e.to_string(),
    })
}

/// Read a number that ffprobe may emit as either a JSON number or a string.
pub(crate) fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Like `num`, for a whole number (negative or fractional values are `None`).
pub(crate) fn uint(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse::<u64>().ok(),
        _ => None,
    }
}

pub(crate) fn uint32(v: Option<&Value>) -> Option<u32> {
    uint(v).and_then(|n| u32::try_from(n).ok())
}

/// `"30000/1001"` -> `Some((30000, 1001))`; `"0/0"` -> `None`.
pub(crate) fn rational_parts(s: &str) -> Option<(u32, u32)> {
    let (n, d) = s.split_once('/')?;
    let (n, d) = (n.trim().parse::<u32>().ok()?, d.trim().parse::<u32>().ok()?);
    (n > 0 && d > 0).then_some((n, d))
}

/// `"30000/1001"` -> `Some(29.97…)`; `"0/0"` -> `None`.
pub(crate) fn rational(v: Option<&Value>) -> Option<f64> {
    let (n, d) = rational_parts(v?.as_str()?)?;
    Some(f64::from(n) / f64::from(d))
}

/// Cover art in an audio file is a "video" stream; it is not a track.
pub(crate) fn is_video_track(s: &Value) -> bool {
    let is_video = s.get("codec_type").and_then(Value::as_str) == Some("video");
    let attached = s
        .pointer("/disposition/attached_pic")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    is_video && attached == 0
}

/// The container's start offset, which the UI's timeline normalises away.
pub(crate) fn container_start_time(root: &Value, video: &Value) -> f64 {
    num(root.get("format").and_then(|f| f.get("start_time")))
        .or_else(|| num(video.get("start_time")))
        .filter(|t| t.is_finite())
        .unwrap_or(0.0)
}

/// What the installed ffmpeg says about one encoder.
#[derive(Debug, Clone, PartialEq)]
pub enum EncoderStatus {
    /// The query failed, so nothing is known. Treated as available: ffmpeg
    /// reports the real problem itself if an export then needs the encoder,
    /// which is better than refusing a cut that would have worked.
    Unknown,
    /// This ffmpeg was built without it.
    Missing,
    /// It is there. `pix_fmts` is what it accepts, when ffmpeg said.
    Available { pix_fmts: Option<Vec<String>> },
}

/// Names of the video encoders in `ffmpeg -encoders` output: rows of
/// ` V....D libx264  description` after the legend's `------` line.
pub(crate) fn parse_encoder_names(listing: &str) -> BTreeSet<String> {
    listing
        .lines()
        .skip_while(|l| l.trim() != "------")
        .skip(1)
        .filter_map(|l| {
            let mut fields = l.split_whitespace();
            let flags = fields.next()?;
            let name = fields.next()?;
            flags.starts_with('V').then(|| name.to_string())
        })
        .collect()
}

/// The `Supported pixel formats:` row of `ffmpeg -h encoder=NAME` output.
pub(crate) fn parse_pix_fmts(help: &str) -> Option<Vec<String>> {
    help.lines()
        .find_map(|l| l.trim().strip_prefix("Supported pixel formats:"))
        .map(|fmts| fmts.split_whitespace().map(str::to_string).collect())
}

/// Combine the two ffmpeg queries, each `None` when it could not be run.
pub(crate) fn encoder_status(name: &str, listing: Option<&str>, help: Option<&str>) -> EncoderStatus {
    match listing {
        None => EncoderStatus::Unknown,
        Some(listing) if !parse_encoder_names(listing).contains(name) => EncoderStatus::Missing,
        Some(_) => EncoderStatus::Available { pix_fmts: help.and_then(parse_pix_fmts) },
    }
}

/// The smart cut's encoders as the installed ffmpeg reports them. Asked once per
/// run: the answer only changes if ffmpeg itself is replaced.
pub fn installed_encoder(name: &str) -> EncoderStatus {
    static INSTALLED: OnceLock<HashMap<&'static str, EncoderStatus>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            let listing = run("ffmpeg", &strings(["-hide_banner", "-encoders"])).ok();
            smartcut::ENCODERS
                .into_iter()
                .map(|name| {
                    let help = run("ffmpeg", &strings(["-hide_banner", "-h", &format!("encoder={name}")])).ok();
                    (name, encoder_status(name, listing.as_deref(), help.as_deref()))
                })
                .collect()
        })
        .get(name)
        .cloned()
        .unwrap_or(EncoderStatus::Unknown)
}

/// `parse_metadata`, with the smart cut's encoder requirement checked against
/// the installed ffmpeg.
pub fn parse_metadata_for_installed_ffmpeg(path: &str, json: &str) -> Result<VideoMetadata, AppError> {
    parse_metadata_with(path, json, installed_encoder)
}

/// Stream properties alone decide frame-accurate support; see
/// `parse_metadata_with` for the encoder check on top.
pub fn parse_metadata(path: &str, json: &str) -> Result<VideoMetadata, AppError> {
    parse_metadata_with(path, json, |_| EncoderStatus::Unknown)
}

/// Parse `ffprobe` output. `encoder` says what the machine's ffmpeg can encode
/// with, by encoder name.
pub fn parse_metadata_with(
    path: &str,
    json: &str,
    encoder: impl Fn(&str) -> EncoderStatus,
) -> Result<VideoMetadata, AppError> {
    let root = parse_json(json)?;

    let empty = Vec::new();
    let streams = root
        .get("streams")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let video = streams.iter().find(|s| is_video_track(s)).ok_or_else(|| {
        AppError::invalid("That file has no video track, so there is nothing to whittle.")
    })?;

    let format = root.get("format");

    let duration = num(format.and_then(|f| f.get("duration")))
        .or_else(|| num(video.get("duration")))
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or_else(|| {
            AppError::invalid(
                "ffprobe reported no usable duration for that file — it may be truncated or still being written.",
            )
        })?;

    let start_time = container_start_time(&root, video);

    let width = uint32(video.get("width")).unwrap_or(0);
    let height = uint32(video.get("height")).unwrap_or(0);

    Ok(VideoMetadata {
        path: path.to_string(),
        duration,
        start_time,
        width,
        height,
        video_codec: video
            .get("codec_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        audio_codec: streams
            .iter()
            .find(|s| s.get("codec_type").and_then(Value::as_str) == Some("audio"))
            .and_then(|s| s.get("codec_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        container: format
            .and_then(|f| f.get("format_name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        size_bytes: uint(format.and_then(|f| f.get("size"))).unwrap_or(0),
        frame_rate: rational(video.get("r_frame_rate")),
        has_b_frames: num(video.get("has_b_frames")).unwrap_or(0.0) > 0.0,
        frame_accurate: match smartcut::parse_source_video(&root)
            .and_then(|source| smartcut::check_encoder(&source, &encoder(source.encoder())))
        {
            Ok(()) => FrameAccurateSupport::Supported,
            Err(e) => FrameAccurateSupport::Unsupported { reason: e.to_string() },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from real `ffprobe -show_format -show_streams` output (ffmpeg 9).
    const SAMPLE: &str = r#"{
      "streams": [
        { "index": 0, "codec_name": "h264", "codec_type": "video",
          "width": 320, "height": 240, "has_b_frames": 2,
          "r_frame_rate": "30/1", "start_time": "0.000000", "duration": "12.000000",
          "disposition": { "attached_pic": 0 } },
        { "index": 1, "codec_name": "aac", "codec_type": "audio",
          "r_frame_rate": "0/0", "start_time": "0.000000", "duration": "12.000000",
          "disposition": { "attached_pic": 0 } }
      ],
      "format": {
        "filename": "sample.mp4", "nb_streams": 2,
        "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
        "start_time": "0.000000", "duration": "12.000000",
        "size": "584913", "bit_rate": "389942"
      }
    }"#;

    #[test]
    fn reads_the_fields_the_timeline_needs() {
        let m = parse_metadata("/v/sample.mp4", SAMPLE).unwrap();
        assert_eq!(m.path, "/v/sample.mp4");
        assert_eq!(m.duration, 12.0);
        assert_eq!((m.width, m.height), (320, 240));
        assert_eq!(m.video_codec, "h264");
        assert_eq!(m.audio_codec.as_deref(), Some("aac"));
        assert_eq!(m.size_bytes, 584_913);
        assert_eq!(m.frame_rate, Some(30.0));
        assert!(m.has_b_frames);
    }

    #[test]
    fn tolerates_numeric_and_string_encodings() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"vp9",
            "width":1920,"height":1080,"r_frame_rate":"30000/1001"}],
            "format":{"duration":8.5,"size":42,"format_name":"matroska,webm"}}"#;
        let m = parse_metadata("/v/a.webm", json).unwrap();
        assert_eq!(m.duration, 8.5);
        assert_eq!(m.size_bytes, 42);
        assert!((m.frame_rate.unwrap() - 29.97).abs() < 0.01);
        assert!(!m.has_b_frames);
        assert_eq!(m.audio_codec, None);
    }

    #[test]
    fn reports_whether_a_frame_accurate_export_is_possible() {
        let supported = parse_metadata("/v/sample.mp4", SAMPLE).unwrap();
        assert_eq!(supported.frame_accurate, FrameAccurateSupport::Supported);

        // An unsupported file still probes; it just says why it cannot be cut so.
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"vp9",
            "width":8,"height":8,"r_frame_rate":"30/1"}],
            "format":{"duration":"5","format_name":"matroska,webm"}}"#;
        match parse_metadata("/v/a.webm", json).unwrap().frame_accurate {
            FrameAccurateSupport::Unsupported { reason } => assert!(reason.contains("vp9"), "{reason}"),
            FrameAccurateSupport::Supported => panic!("vp9 should not be supported"),
        }
    }

    const H264_YUV444: &str = r#"{"streams":[{"codec_type":"video","codec_name":"h264",
        "width":8,"height":8,"r_frame_rate":"30/1","pix_fmt":"yuv444p"}],
        "format":{"duration":"5","format_name":"mp4"}}"#;

    fn support_with(status: EncoderStatus, json: &str) -> FrameAccurateSupport {
        parse_metadata_with("/v/a.mp4", json, |_| status.clone()).unwrap().frame_accurate
    }

    #[test]
    fn a_missing_encoder_makes_frame_accurate_export_unsupported_with_a_reason() {
        match support_with(EncoderStatus::Missing, SAMPLE) {
            FrameAccurateSupport::Unsupported { reason } => assert!(reason.contains("libx264"), "{reason}"),
            FrameAccurateSupport::Supported => panic!("no encoder, no frame-accurate export"),
        }
    }

    #[test]
    fn an_encoder_that_cannot_take_the_pixel_format_is_unsupported_with_a_reason() {
        let eight_bit = EncoderStatus::Available { pix_fmts: Some(vec!["yuv420p".into(), "nv12".into()]) };
        match support_with(eight_bit, H264_YUV444) {
            FrameAccurateSupport::Unsupported { reason } => assert!(reason.contains("yuv444p"), "{reason}"),
            FrameAccurateSupport::Supported => panic!("yuv444p is not encodable here"),
        }
        let wide = EncoderStatus::Available { pix_fmts: Some(vec!["yuv444p".into()]) };
        assert_eq!(support_with(wide, H264_YUV444), FrameAccurateSupport::Supported);
    }

    #[test]
    fn an_encoder_query_that_failed_does_not_block_the_cut() {
        assert_eq!(support_with(EncoderStatus::Unknown, H264_YUV444), FrameAccurateSupport::Supported);
        let silent = EncoderStatus::Available { pix_fmts: None };
        assert_eq!(support_with(silent, H264_YUV444), FrameAccurateSupport::Supported);
    }

    const ENCODERS: &str = "Encoders:
 V..... = Video
 A..... = Audio
 ------
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC (codec h264)
 V....D libx265              libx265 H.265 / HEVC (codec hevc)
 A....D aac                  AAC (Advanced Audio Coding)
";

    #[test]
    fn reads_video_encoder_names_from_the_listing() {
        let names = parse_encoder_names(ENCODERS);
        assert!(names.contains("libx264") && names.contains("libx265"));
        assert!(!names.contains("aac"), "audio encoders are not video encoders");
        assert!(!names.contains("="), "the legend is not an encoder");
    }

    #[test]
    fn reads_pixel_formats_from_encoder_help() {
        let help = "Encoder libx264 [libx264 H.264]:\n    General capabilities: dr1 delay\n    Threading capabilities: auto\n    Supported pixel formats: yuv420p yuv444p gray\nlibx264 AVOptions:\n";
        assert_eq!(parse_pix_fmts(help), Some(vec!["yuv420p".into(), "yuv444p".into(), "gray".into()]));
        assert_eq!(parse_pix_fmts("Codec 'nope' is not recognized by FFmpeg."), None);
    }

    #[test]
    fn combines_the_listing_and_the_help_into_a_status() {
        let help = "    Supported pixel formats: yuv420p";
        assert_eq!(
            encoder_status("libx264", Some(ENCODERS), Some(help)),
            EncoderStatus::Available { pix_fmts: Some(vec!["yuv420p".into()]) },
        );
        assert_eq!(encoder_status("libx264", Some(ENCODERS), None), EncoderStatus::Available { pix_fmts: None });
        assert_eq!(encoder_status("libsvtav1", Some(ENCODERS), None), EncoderStatus::Missing);
        assert_eq!(encoder_status("libx264", None, None), EncoderStatus::Unknown);
    }

    #[test]
    fn supported_and_unsupported_serialise_as_tagged_objects() {
        assert_eq!(
            serde_json::to_value(FrameAccurateSupport::Supported).unwrap(),
            serde_json::json!({ "status": "supported" }),
        );
        assert_eq!(
            serde_json::to_value(FrameAccurateSupport::Unsupported { reason: "why".into() }).unwrap(),
            serde_json::json!({ "status": "unsupported", "reason": "why" }),
        );
    }

    #[test]
    fn falls_back_to_stream_duration_when_format_lacks_one() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264",
            "width":8,"height":8,"duration":"3.5"}],"format":{"format_name":"avi"}}"#;
        assert_eq!(parse_metadata("/v/a.avi", json).unwrap().duration, 3.5);
    }

    #[test]
    fn normalises_a_non_zero_container_start() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264",
            "width":8,"height":8}],
            "format":{"duration":"10","start_time":"1440.5","format_name":"mpegts"}}"#;
        let m = parse_metadata("/v/a.ts", json).unwrap();
        assert_eq!(m.start_time, 1440.5);
        assert_eq!(m.duration, 10.0);
    }

    #[test]
    fn cover_art_does_not_count_as_a_video_track() {
        let json = r#"{"streams":[
            {"codec_type":"video","codec_name":"mjpeg","width":600,"height":600,
             "disposition":{"attached_pic":1}},
            {"codec_type":"audio","codec_name":"mp3"}],
            "format":{"duration":"180","format_name":"mp3"}}"#;
        let err = parse_metadata("/v/song.mp3", json).unwrap_err();
        assert_eq!(err.kind(), "invalid");
    }

    #[test]
    fn zero_or_missing_duration_is_rejected() {
        for bad in [r#""0""#, r#""0.0""#, "null"] {
            let json = format!(
                r#"{{"streams":[{{"codec_type":"video","codec_name":"h264","width":8,"height":8}}],
                    "format":{{"duration":{bad},"format_name":"mp4"}}}}"#
            );
            assert_eq!(parse_metadata("/v/a.mp4", &json).unwrap_err().kind(), "invalid");
        }
    }

    #[test]
    fn malformed_json_is_reported_as_unparsable() {
        assert_eq!(parse_metadata("/v/a.mp4", "not json").unwrap_err().kind(), "unparsable");
    }

    #[test]
    fn degenerate_frame_rates_are_none() {
        for r in ["0/0", "1/0", "garbage"] {
            let json = format!(
                r#"{{"streams":[{{"codec_type":"video","codec_name":"h264","width":8,"height":8,
                    "r_frame_rate":"{r}"}}],"format":{{"duration":"5","format_name":"mp4"}}}}"#
            );
            assert_eq!(parse_metadata("/v/a.mp4", &json).unwrap().frame_rate, None);
        }
    }
}
