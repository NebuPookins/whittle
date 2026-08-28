use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::AppError;
use crate::proc::run;
use crate::probe::{metadata_args, parse_metadata};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub dest_path: String,
    pub requested_duration: f64,
    pub actual_duration: f64,
    pub size_bytes: u64,
}

/// ffmpeg wants a plain decimal; `{:.6}` avoids scientific notation for the
/// very small values that show up after deep zooming.
pub fn format_ts(t: f64) -> String {
    format!("{:.6}", t)
}

/// Build the lossless trim invocation.
///
/// `-seek2any 1` is load-bearing, not decoration. Without it, seeking into a
/// **Matroska file whose video uses B-frames** lands a full GOP early and
/// silently prepends unwanted footage: asking for 3.2s–6.4s of such a file
/// yields 4.9s starting at 1.6s, with no discard flag to hide it. mp4 and
/// B-frame-free mkv are unaffected. `-seek2any` lets the demuxer land on the
/// exact requested position.
///
/// That is only safe because `start` is snapped to a real keyframe before it
/// gets here — seeking to a non-keyframe would open the clip mid-GOP.
///
/// `-ss`/`-to` are deliberately *input* options (before `-i`): input `-to` is
/// absolute on the input timeline, whereas an output-side `-to` would be
/// measured from the start of the output and produce a clip of the wrong
/// length.
///
/// `start` and `end` are 0-based (the container's `start_time` has already
/// been subtracted, so they line up with the keyframes and the UI), but input
/// `-ss`/`-to` address the raw container timeline — so `start_time` is added
/// back before they reach ffmpeg. A TS file whose first packet sits at PTS
/// 1440.5 has to seek to 1445.5 for a 0-based cut at 5.0s, not 5.0.
pub fn trim_args(input: &str, start: f64, end: f64, start_time: f64, output: &str) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-seek2any".into(),
        "1".into(),
        "-ss".into(),
        format_ts(start + start_time),
        "-to".into(),
        format_ts(end + start_time),
        "-i".into(),
        input.into(),
        // Carry every stream through (extra audio tracks, subtitles, chapters).
        "-map".into(),
        "0".into(),
        "-c".into(),
        "copy".into(),
        output.into(),
    ]
}

pub fn validate_range(start: f64, end: f64) -> Result<(), AppError> {
    if !start.is_finite() || !end.is_finite() {
        return Err(AppError::invalid("Cut points must be real numbers."));
    }
    if start < 0.0 {
        return Err(AppError::invalid("A cut cannot start before the beginning of the file."));
    }
    if end <= start {
        return Err(AppError::invalid("The end of the range must come after its start."));
    }
    Ok(())
}

/// Resolve a path far enough to compare two of them, even when `p` does not
/// exist yet (the export destination usually doesn't).
fn resolved(p: &str) -> PathBuf {
    let path = Path::new(p);
    if let Ok(c) = path.canonicalize() {
        return c;
    }
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => match dir.canonicalize() {
            Ok(d) => d.join(name),
            Err(_) => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

/// ffmpeg runs with `-y`; writing to its own input would destroy the source.
pub fn ensure_distinct(input: &str, dest: &str) -> Result<(), AppError> {
    if resolved(input) == resolved(dest) {
        return Err(AppError::invalid(
            "Choose a different destination — writing over the source file would destroy it.",
        ));
    }
    Ok(())
}

pub fn export(
    input: &str,
    start: f64,
    end: f64,
    start_time: f64,
    dest: &str,
) -> Result<ExportReport, AppError> {
    validate_range(start, end)?;
    ensure_distinct(input, dest)?;

    run("ffmpeg", &trim_args(input, start, end, start_time, dest))?;

    // Verify rather than assume, so the UI can show real keyframe drift instead
    // of the requested figure. The file is already written by this point, so a
    // failed probe must not turn a successful copy into an "Export failed" that
    // leaves a phantom file behind — fall back to the requested duration and
    // the size on disk rather than erroring.
    let probed = run("ffprobe", &metadata_args(dest))
        .ok()
        .and_then(|json| parse_metadata(dest, &json).ok());

    if probed.is_none() {
        crate::applog::line(
            "trim",
            &format!("output probe failed; reporting requested duration for {dest}"),
        );
    }

    let (actual_duration, size_bytes) = match probed {
        Some(meta) => (meta.duration, meta.size_bytes),
        None => (
            end - start,
            std::fs::metadata(Path::new(dest)).map(|m| m.len()).unwrap_or(0),
        ),
    };

    Ok(ExportReport {
        dest_path: dest.to_string(),
        requested_duration: end - start,
        actual_duration,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Vec<String> {
        trim_args("/v/in.mkv", 3.2, 6.4, 0.0, "/v/out.mkv")
    }

    fn index_of(args: &[String], needle: &str) -> usize {
        args.iter().position(|a| a == needle).expect(needle)
    }

    #[test]
    fn seeks_on_the_input_side_so_to_is_absolute() {
        let a = args();
        // -ss and -to must both precede -i, or -to changes meaning.
        assert!(index_of(&a, "-ss") < index_of(&a, "-i"));
        assert!(index_of(&a, "-to") < index_of(&a, "-i"));
        assert_eq!(a[index_of(&a, "-ss") + 1], "3.200000");
        assert_eq!(a[index_of(&a, "-to") + 1], "6.400000");
    }

    #[test]
    fn rebases_cut_points_onto_the_container_timeline() {
        // A TS file's raw timestamps are offset from zero; the UI works in
        // 0-based seconds, so the container start_time has to be added back or
        // -ss/-to would seek before the first packet.
        let a = trim_args("/v/in.ts", 5.0, 10.0, 1440.5, "/v/out.ts");
        assert_eq!(a[index_of(&a, "-ss") + 1], "1445.500000");
        assert_eq!(a[index_of(&a, "-to") + 1], "1450.500000");
    }

    #[test]
    fn copies_streams_without_re_encoding() {
        let a = args();
        assert_eq!(a[index_of(&a, "-c") + 1], "copy");
        assert_eq!(a[index_of(&a, "-map") + 1], "0");
    }

    #[test]
    fn enables_exact_seeking_for_b_frame_matroska() {
        let a = args();
        assert_eq!(a[index_of(&a, "-seek2any") + 1], "1");
        assert!(index_of(&a, "-seek2any") < index_of(&a, "-i"));
    }

    #[test]
    fn input_and_output_land_in_the_expected_places() {
        let a = args();
        assert_eq!(a[index_of(&a, "-i") + 1], "/v/in.mkv");
        assert_eq!(a.last().unwrap(), "/v/out.mkv");
    }

    #[test]
    fn timestamps_are_plain_fixed_point() {
        // ffmpeg cannot read scientific notation, which is what Rust's default
        // float formatting would reach for at the extremes of a zoomed timeline.
        assert_eq!(format_ts(0.5), "0.500000");
        assert_eq!(format_ts(3661.5), "3661.500000");
        assert_eq!(format_ts(0.0), "0.000000");
        for t in [1e-9, 1e-7, 1.0 / 3.0, 86_400.0, 1e7] {
            let s = format_ts(t);
            assert!(!s.contains('e'), "{t} formatted as {s}");
            assert_eq!(s.split('.').nth(1).map(str::len), Some(6), "{s}");
        }
    }

    #[test]
    fn rejects_inverted_or_empty_ranges() {
        assert!(validate_range(5.0, 5.0).is_err());
        assert!(validate_range(6.0, 5.0).is_err());
        assert!(validate_range(-1.0, 5.0).is_err());
        assert!(validate_range(f64::NAN, 5.0).is_err());
        assert!(validate_range(0.0, f64::INFINITY).is_err());
        assert!(validate_range(0.0, 5.0).is_ok());
    }

    #[test]
    fn refuses_to_overwrite_the_source() {
        assert!(ensure_distinct("/v/in.mkv", "/v/in.mkv").is_err());
        assert!(ensure_distinct("/v/in.mkv", "/v/out.mkv").is_ok());
    }

    #[test]
    fn refuses_source_reached_by_a_different_spelling() {
        assert!(ensure_distinct("/tmp/./in.mkv", "/tmp/in.mkv").is_err());
    }
}
