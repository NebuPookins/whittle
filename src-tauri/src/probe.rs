use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;

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
}

pub fn metadata_args(path: &str) -> Vec<String> {
    [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        path,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Read a number that ffprobe may emit as either a JSON number or a string.
fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// `"30000/1001"` -> `Some(29.97…)`; `"0/0"` -> `None`.
fn rational(v: Option<&Value>) -> Option<f64> {
    let s = v?.as_str()?;
    let (n, d) = s.split_once('/')?;
    let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
    if d == 0.0 || n == 0.0 {
        return None;
    }
    let r = n / d;
    r.is_finite().then_some(r)
}

fn is_video_track(s: &Value) -> bool {
    let is_video = s.get("codec_type").and_then(Value::as_str) == Some("video");
    // Cover art in an audio file is a "video" stream; it is not a track.
    let attached = s
        .pointer("/disposition/attached_pic")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    is_video && attached == 0
}

pub fn parse_metadata(path: &str, json: &str) -> Result<VideoMetadata, AppError> {
    let root: Value = serde_json::from_str(json).map_err(|e| AppError::Unparsable {
        tool: "ffprobe",
        detail: e.to_string(),
    })?;

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

    let start_time = num(format.and_then(|f| f.get("start_time")))
        .or_else(|| num(video.get("start_time")))
        .filter(|t| t.is_finite())
        .unwrap_or(0.0);

    let width = num(video.get("width")).unwrap_or(0.0).max(0.0) as u32;
    let height = num(video.get("height")).unwrap_or(0.0).max(0.0) as u32;

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
        size_bytes: num(format.and_then(|f| f.get("size"))).unwrap_or(0.0).max(0.0) as u64,
        frame_rate: rational(video.get("r_frame_rate")),
        has_b_frames: num(video.get("has_b_frames")).unwrap_or(0.0) > 0.0,
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
