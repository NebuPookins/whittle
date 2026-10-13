//! End-to-end checks against the real ffmpeg/ffprobe binaries.
//!
//! These exist because the trim command's correctness is not something unit
//! tests on argument vectors can establish. In particular they pin down the
//! Matroska + B-frame seeking behaviour that `-seek2any` is there to fix:
//! without that flag, asking for 3.2s–6.4s of such a file yields ~4.9s of
//! footage starting a whole GOP early, with no discard flag to hide it.
//!
//! Skipped automatically when ffmpeg is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;

#[macro_use]
mod common;

use whittle_lib::error::AppError;
use whittle_lib::keyframes::{keyframe_args, normalize, parse_packet_line};
use whittle_lib::probe::{metadata_args, parse_metadata};
use whittle_lib::proc::run;
use whittle_lib::trim::{self, ExportMode, ExportReport};

const DURATION: f64 = 12.0;
/// 48-frame GOP at 30fps.
const GOP: f64 = 1.6;

fn workdir() -> PathBuf {
    let dir = std::env::temp_dir().join("whittle-it");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Encode a fixture with B-frames (the interesting case) in `container`.
fn fixture(name: &str) -> PathBuf {
    let path = workdir().join(name);
    if path.exists() {
        return path;
    }
    let status = Command::new("ffmpeg")
        .args([
            "-v", "error", "-nostdin", "-y",
            "-f", "lavfi", "-i", &format!("testsrc2=size=320x240:rate=30:duration={DURATION}"),
            "-f", "lavfi", "-i", &format!("sine=frequency=440:duration={DURATION}"),
            "-c:v", "libx264", "-g", "48", "-bf", "2", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-shortest",
        ])
        .arg(&path)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "could not build fixture {name}");
    path
}

/// Export `[start, end)` of `source` to `dest`, replacing whatever was there.
fn export_to(
    source: &Path,
    dest: &Path,
    start: f64,
    end: f64,
    mode: ExportMode,
) -> Result<ExportReport, AppError> {
    let _ = std::fs::remove_file(dest);
    trim::export(&source.to_string_lossy(), start, end, &dest.to_string_lossy(), mode)
}

fn probe_keyframes(path: &Path) -> Vec<f64> {
    let out = run("ffprobe", &keyframe_args(&path.to_string_lossy())).expect("probe keyframes");
    normalize(out.lines().filter_map(parse_packet_line).collect(), 0.0)
}

fn duration_of(path: &Path) -> f64 {
    let json = run("ffprobe", &metadata_args(&path.to_string_lossy())).expect("probe");
    parse_metadata(&path.to_string_lossy(), &json).expect("parse").duration
}

/// Extent of the actual video content, ignoring the container's timestamp base.
///
/// This, not `format.duration`, is the honest measure of "did we extract the
/// range that was asked for": Matroska keeps a small non-zero timestamp origin
/// that inflates the reported duration by ~0.2s without changing a frame of
/// what was copied.
fn video_span(path: &Path) -> f64 {
    let out = run(
        "ffprobe",
        &[
            "-v", "error", "-select_streams", "v:0", "-show_packets",
            "-show_entries", "packet=pts_time", "-of", "csv=print_section=0",
            &path.to_string_lossy(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
    )
    .expect("probe packets");

    let times: Vec<f64> = out
        .lines()
        .filter_map(|l| l.split(',').next()?.trim().parse::<f64>().ok())
        .collect();

    match (
        times.iter().cloned().fold(f64::INFINITY, f64::min),
        times.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    ) {
        (lo, hi) if lo.is_finite() && hi.is_finite() => hi - lo,
        _ => 0.0,
    }
}

/// First video packet's flags, e.g. `K__`.
fn first_packet_flags(path: &Path) -> String {
    let out = run(
        "ffprobe",
        &[
            "-v", "error", "-select_streams", "v:0", "-show_packets",
            "-show_entries", "packet=flags", "-of", "csv=print_section=0",
            &path.to_string_lossy(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
    )
    .expect("probe packets");
    out.lines().next().unwrap_or_default().trim().to_string()
}

#[test]
fn metadata_matches_the_encoded_fixture() {
    require_ffmpeg!();
    let path = fixture("meta.mp4");
    let json = run("ffprobe", &metadata_args(&path.to_string_lossy())).unwrap();
    let meta = parse_metadata(&path.to_string_lossy(), &json).unwrap();

    assert!((meta.duration - DURATION).abs() < 0.2, "duration {}", meta.duration);
    assert_eq!((meta.width, meta.height), (320, 240));
    assert_eq!(meta.video_codec, "h264");
    assert_eq!(meta.audio_codec.as_deref(), Some("aac"));
    assert!(meta.has_b_frames, "fixture was encoded with -bf 2");
    assert!(meta.size_bytes > 0);
}

#[test]
fn keyframes_come_back_sorted_and_on_the_gop_grid() {
    require_ffmpeg!();
    for name in ["kf.mp4", "kf.mkv"] {
        let kfs = probe_keyframes(&fixture(name));

        assert!(kfs.len() >= 5, "{name}: only {} keyframes", kfs.len());
        assert!(kfs.windows(2).all(|w| w[1] > w[0]), "{name}: not sorted/deduped");
        assert!(kfs[0].abs() < 1e-6, "{name}: should start at zero");

        for (i, t) in kfs.iter().enumerate() {
            let expected = i as f64 * GOP;
            assert!((t - expected).abs() < 0.05, "{name}: keyframe {i} at {t}, want {expected}");
        }
    }
}

/// The core guarantee: a trim delivers the range asked for, in every container.
#[test]
fn trim_delivers_the_requested_range() {
    require_ffmpeg!();
    // 3.2 and 6.4 are both keyframes, so the result should be exact.
    let (start, end) = (2.0 * GOP, 4.0 * GOP);

    for name in ["cut.mp4", "cut.mkv", "cut.mov"] {
        let source = fixture(name);
        let dest = workdir().join(format!("out-{name}"));

        let report = export_to(&source, &dest, start, end, ExportMode::KeyframeCopy)
            .unwrap_or_else(|e| panic!("{name}: export failed: {e}"));

        let want = end - start;
        // The tight assertion: an extra GOP would show up as ~+1.6s here.
        assert!(
            (video_span(&dest) - want).abs() < 0.2,
            "{name}: extracted {:.3}s of video, want {want:.3}s — roughly a GOP \
             too much means the demuxer seeked to the wrong keyframe",
            video_span(&dest),
        );
        // Container duration is looser: Matroska keeps a non-zero timestamp
        // origin, so it reads ~0.2s longer than the footage it holds.
        assert!(
            (duration_of(&dest) - want).abs() < 0.6,
            "{name}: container reports {:.3}s for a {want:.3}s range",
            duration_of(&dest),
        );
        assert!(report.actual_duration > 0.0, "{name}: no duration reported");
        assert!(report.size_bytes > 0, "{name}: empty output");
        assert!(
            first_packet_flags(&dest).contains('K'),
            "{name}: clip must open on a keyframe, got {:?}",
            first_packet_flags(&dest),
        );
    }
}

#[test]
fn trim_snapped_to_each_keyframe_stays_accurate() {
    require_ffmpeg!();
    let source = fixture("sweep.mkv");
    let keyframes = probe_keyframes(&source);

    for kf in keyframes.iter().copied().take(5) {
        let end = kf + GOP;
        if end > DURATION - 0.1 {
            continue;
        }
        let dest = workdir().join(format!("sweep-{kf:.1}.mkv"));

        export_to(&source, &dest, kf, end, ExportMode::KeyframeCopy)
            .unwrap_or_else(|e| panic!("export from {kf}: {e}"));

        assert!(
            (video_span(&dest) - GOP).abs() < 0.2,
            "cut at keyframe {kf} extracted {:.3}s of video, want {GOP:.3}s",
            video_span(&dest),
        );
        assert!(
            first_packet_flags(&dest).contains('K'),
            "cut at keyframe {kf} did not open on a keyframe",
        );
    }
}

#[test]
fn refuses_to_overwrite_its_own_source() {
    require_ffmpeg!();
    let source = fixture("guard.mp4");
    let before = std::fs::metadata(&source).unwrap().len();

    let err = trim::export(
        &source.to_string_lossy(),
        1.6,
        3.2,
        &source.to_string_lossy(),
        ExportMode::KeyframeCopy,
    )
    .expect_err("writing over the source must be refused");

    assert_eq!(err.kind(), "invalid");
    assert_eq!(
        std::fs::metadata(&source).unwrap().len(),
        before,
        "the source file was modified",
    );
}

/* ---------- frame-accurate export ---------- */

const FPS: f64 = 30.0;

fn encode(name: &str, video: &[&str], container_args: &[&str], audio_tracks: usize) -> PathBuf {
    let path = workdir().join(name);
    if path.exists() {
        return path;
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-v", "error", "-nostdin", "-y",
        "-f", "lavfi", "-i", &format!("testsrc2=size=320x240:rate=30:duration={DURATION}"),
    ]);
    for i in 0..audio_tracks {
        cmd.args(["-f", "lavfi", "-i", &format!("sine=frequency={}:duration={DURATION}", 440 * (i + 1))]);
    }
    cmd.args(["-map", "0:v"]);
    for i in 0..audio_tracks {
        cmd.args(["-map".to_string(), format!("{}:a", i + 1)]);
    }
    cmd.args(video).args(["-c:a", "aac"]).args(container_args).arg(&path);
    assert!(cmd.status().expect("run ffmpeg").success(), "could not build fixture {name}");
    path
}

const H264: [&str; 8] = ["-c:v", "libx264", "-g", "48", "-bf", "2", "-pix_fmt", "yuv420p"];

fn start_time_of(path: &Path) -> f64 {
    let json = run("ffprobe", &metadata_args(&path.to_string_lossy())).expect("probe");
    parse_metadata(&path.to_string_lossy(), &json).expect("parse").start_time
}

fn packet_times(path: &Path, stream: &str) -> Vec<f64> {
    let out = run(
        "ffprobe",
        &["-v", "error", "-select_streams", stream, "-show_packets", "-show_entries",
          "packet=pts_time", "-of", "csv=print_section=0", &path.to_string_lossy()]
            .map(String::from),
    )
    .expect("probe packets");
    out.lines().filter_map(|l| l.split(',').next()?.trim().parse().ok()).collect()
}

fn min_of(xs: &[f64]) -> f64 {
    xs.iter().cloned().fold(f64::INFINITY, f64::min)
}

fn max_of(xs: &[f64]) -> f64 {
    xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
}

fn stream_count(path: &Path, kind: &str) -> usize {
    let out = run(
        "ffprobe",
        &["-v", "error", "-select_streams", kind, "-show_entries", "stream=index",
          "-of", "csv=print_section=0", &path.to_string_lossy()]
            .map(String::from),
    )
    .expect("probe streams");
    // A transport stream lists each stream once per program, so dedupe by index.
    out.lines()
        .filter_map(|l| l.trim().parse::<u32>().ok())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// Anything ffmpeg complains about while decoding the whole file.
fn decode_errors(path: &Path) -> String {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .expect("run ffmpeg");
    String::from_utf8_lossy(&out.stderr).trim().to_string()
}

/// Decoded luma of `frames` frames starting at frame `from`, one `Vec<u8>` per
/// frame. Frames are taken by index, never by timestamp: Matroska rounds
/// timestamps to the millisecond, so anything that pairs frames by time
/// compares neighbours when the cadence jitters between 33 and 34ms.
fn luma_frames(path: &Path, frame_bytes: usize, from: usize, frames: usize) -> Vec<Vec<u8>> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-vf", &format!("trim=start_frame={from}:end_frame={}", from + frames)])
        .args(["-pix_fmt", "gray", "-fps_mode", "passthrough", "-f", "rawvideo", "-"])
        .output()
        .expect("run ffmpeg");
    out.stdout.chunks(frame_bytes).map(<[u8]>::to_vec).collect()
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let sq: f64 = a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum();
    if sq == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0_f64.powi(2) / (sq / a.len() as f64)).log10()
}

/// Lowest per-frame luma PSNR between the output and `frames` frames of the
/// source starting at frame `from`. Frames shifted by one score ~23dB on this
/// fixture; the same footage re-encoded at CRF 18 scores above 40.
fn min_psnr_against_source(out: &Path, source: &Path, frame_bytes: usize, from: usize, frames: usize) -> f64 {
    let ours = luma_frames(out, frame_bytes, 0, frames);
    let theirs = luma_frames(source, frame_bytes, from, frames);
    assert_eq!(ours.len(), frames, "output decoded to {} frames", ours.len());
    assert_eq!(theirs.len(), frames, "source has no frames {from}..{}", from + frames);
    ours.iter().zip(&theirs).map(|(a, b)| psnr(a, b)).fold(f64::INFINITY, f64::min)
}

/// What is true of a source however it is cut: computed once per source rather
/// than once per range.
struct Facts {
    /// Time of the first video frame, 0-based. Frames sit at `grid + n/30`. (A
    /// transport stream's video can start a fraction of a frame after the
    /// container does, which shifts that grid off the zero-based times; the cut
    /// then lands on the nearest frame to each.)
    grid: f64,
    audio_tracks: usize,
    /// Size of one decoded luma frame.
    frame_bytes: usize,
}

impl Facts {
    fn of(source: &Path) -> Self {
        let start_time = start_time_of(source);
        Self {
            grid: min_of(&packet_times(source, "v:0")) - start_time,
            audio_tracks: stream_count(source, "a"),
            frame_bytes: 320 * 240,
        }
    }
}

/// Export `[start, end)` frame-accurately and check the result against the
/// contract: it starts on the frame at `start`, holds exactly the frames up to
/// `end`, keeps every audio track in step, and decodes cleanly.
fn assert_frame_accurate(source: &Path, facts: &Facts, start: f64, end: f64, ext: &str, label: &str) {
    let dest = workdir().join(format!("fa-{label}-{start}-{end}.{ext}"));

    let report = export_to(source, &dest, start, end, ExportMode::FrameAccurate)
        .unwrap_or_else(|e| panic!("{label} {start}-{end}: export failed: {e}"));

    let first_frame = ((start - facts.grid) * FPS).round().max(0.0) as usize;
    let want_frames = ((end - facts.grid) * FPS).round() as usize - first_frame;
    let video = packet_times(&dest, "v:0");
    assert_eq!(video.len(), want_frames, "{label} {start}-{end}: frame count");

    // The clip begins at its own zero. Matroska and mp4 both shift timestamps
    // by up to the B-frame reorder delay (~0.07s) to keep dts non-negative.
    // (An MPEG-TS muxer starts its clock about a second in, by design.)
    if ext != "ts" {
        assert!(min_of(&video).abs() < 0.1, "{label}: first frame at {}", min_of(&video));
    }

    // The frames are the right ones, not merely the right number of them.
    let psnr = min_psnr_against_source(&dest, source, facts.frame_bytes, first_frame, want_frames);
    assert!(psnr > 35.0, "{label} {start}-{end}: worst frame PSNR {psnr}; clip is offset from the source");

    assert_eq!(stream_count(&dest, "a"), facts.audio_tracks, "{label}: audio tracks lost");
    for track in 0..facts.audio_tracks {
        let audio = packet_times(&dest, &format!("a:{track}"));
        assert!(!audio.is_empty(), "{label}: audio track {track} is empty");
        let span = max_of(&audio) - min_of(&audio);
        assert!(
            (span - (end - start)).abs() < 0.15,
            "{label}: audio track {track} spans {span:.3}s for a {:.3}s range",
            end - start,
        );
        assert!(
            (min_of(&audio) - min_of(&video)).abs() < 0.15,
            "{label}: audio starts at {:.3}s, video at {:.3}s",
            min_of(&audio),
            min_of(&video),
        );
    }

    assert_eq!(decode_errors(&dest), "", "{label} {start}-{end}: output does not decode cleanly");
    assert!(report.size_bytes > 0);
    // The report describes the cut that was made, which is the request snapped
    // to frames, and the start clamped to the first one: a frame off at most.
    assert!(
        (report.requested_duration - (end - start)).abs() <= 1.0 / FPS + 1e-6,
        "{label} {start}-{end}: reported {}s",
        report.requested_duration,
    );
    assert!(
        (report.requested_duration - want_frames as f64 / FPS).abs() < 1e-6,
        "{label} {start}-{end}: reported {}s for {want_frames} frames",
        report.requested_duration,
    );
}

/// Ranges covering every shape of plan: both ends off a keyframe (head, body
/// and tail), a keyframe at either or both ends, and a range inside one GOP.
const RANGES: [(f64, f64); 6] = [
    (2.5, 7.0),
    (3.2, 7.0),
    (2.5, 6.4),
    (3.2, 6.4),
    (2.5, 3.0),
    (0.0, 2.0),
];

fn sweep(source: &Path, ext: &str, label: &str) {
    let facts = Facts::of(source);
    for (start, end) in RANGES {
        assert_frame_accurate(source, &facts, start, end, ext, label);
    }
}

#[test]
fn frame_accurate_cuts_h264_with_b_frames_in_mp4() {
    require_ffmpeg!();
    sweep(&encode("fa.mp4", &H264, &[], 1), "mp4", "h264-mp4");
}

#[test]
fn frame_accurate_cuts_h264_in_mkv() {
    require_ffmpeg!();
    let source = encode("fa.mkv", &H264, &[], 1);
    sweep(&source, "mkv", "h264-mkv");
    // The source container does not dictate the destination's.
    assert_frame_accurate(&source, &Facts::of(&source), 2.5, 7.0, "mp4", "h264-mkv-to-mp4");
}

#[test]
fn frame_accurate_keeps_every_audio_track() {
    require_ffmpeg!();
    let source = encode("fa-2a.mp4", &H264, &[], 2);
    assert_frame_accurate(&source, &Facts::of(&source), 2.5, 7.0, "mp4", "two-audio-tracks");
}

#[test]
fn frame_accurate_handles_a_transport_stream_with_a_non_zero_start() {
    require_ffmpeg!();
    let source = encode("fa.ts", &H264, &["-output_ts_offset", "1440.5"], 1);
    assert!(start_time_of(&source) > 1000.0, "fixture should start late");
    sweep(&source, "ts", "h264-ts");
}

#[test]
fn frame_accurate_cuts_closed_gop_hevc() {
    require_ffmpeg!();
    let hevc = ["-c:v", "libx265", "-pix_fmt", "yuv420p", "-tag:v", "hvc1",
                "-x265-params", "keyint=48:min-keyint=48:bframes=2:open-gop=0:log-level=error"];
    sweep(&encode("fa-hevc.mp4", &hevc, &[], 1), "mp4", "hevc-closed");
}

/// x265 defaults to open GOPs, whose keyframes cannot start a copied body (see
/// `smartcut`), so the range is re-encoded whole. The result must still be exact.
#[test]
fn frame_accurate_cuts_open_gop_hevc() {
    require_ffmpeg!();
    let hevc = ["-c:v", "libx265", "-pix_fmt", "yuv420p", "-tag:v", "hvc1",
                "-x265-params", "keyint=48:min-keyint=48:bframes=2:log-level=error"];
    sweep(&encode("fa-hevc-open.mp4", &hevc, &[], 1), "mp4", "hevc-open");
}

/// An out-point exactly on an open-GOP keyframe: the pictures that lead that
/// keyframe display before it, so they belong to the range, yet a body stopped
/// short of the keyframe's decode time does not hold them. The first keyframe
/// (at 0) has none, so the body can be copied.
#[test]
fn frame_accurate_keeps_leading_pictures_before_an_open_gop_out_point() {
    require_ffmpeg!();
    let hevc = ["-c:v", "libx265", "-pix_fmt", "yuv420p", "-tag:v", "hvc1",
                "-x265-params", "keyint=48:min-keyint=48:bframes=2:log-level=error"];
    let source = encode("fa-hevc-open.mp4", &hevc, &[], 1);
    let facts = Facts::of(&source);
    for (start, end) in [(0.0, 4.8), (0.0, 6.4), (0.0, 4.9)] {
        assert_frame_accurate(&source, &facts, start, end, "mp4", "hevc-open-keyframe-out");
    }
}

/// A transport stream has no index, so ffprobe's interval reads seek by byte
/// estimate. Measured on a 15 minute stream: asked for the second before 1498.37
/// it opened on a plain packet past it and reported no keyframe, in about half
/// of the probes. A cut deep into such a file must still decode from the right
/// keyframe, rather than from the start of the file with a wrong grid phase.
#[test]
fn frame_accurate_cuts_deep_into_a_long_transport_stream() {
    require_ffmpeg!();
    let path = workdir().join("fa-long.ts");
    if !path.exists() {
        let status = Command::new("ffmpeg")
            .args([
                "-v", "error", "-nostdin", "-y",
                "-f", "lavfi", "-i", "testsrc2=size=160x120:rate=30:duration=900",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=900",
                "-c:v", "libx264", "-preset", "ultrafast", "-g", "50", "-bf", "2", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-output_ts_offset", "1440.5",
            ])
            .arg(&path)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "could not build the long transport stream");
    }
    let facts = Facts { frame_bytes: 160 * 120, ..Facts::of(&path) };
    // Starts whose raw time (plus the 1440.5s offset) the plain one-second read
    // missed a keyframe for, and one whose closing keyframe was missed.
    for (start, end) in [(57.87, 61.2), (322.87, 326.0), (587.87, 591.2), (700.0, 703.37)] {
        assert_frame_accurate(&path, &facts, start, end, "mp4", "long-ts");
    }
}

#[test]
fn frame_accurate_refuses_codecs_it_cannot_match() {
    require_ffmpeg!();
    let source = encode("fa-mpeg4.mkv", &["-c:v", "mpeg4"], &[], 1);
    let dest = workdir().join("fa-refused.mkv");

    let err = export_to(&source, &dest, 2.5, 7.0, ExportMode::FrameAccurate)
        .expect_err("mpeg4 is not supported by the smart cut");

    assert_eq!(err.kind(), "invalid");
    assert!(!dest.exists(), "nothing should be written for a refused export");

    // The keyframe-copy mode is unaffected by the codec.
    export_to(&source, &dest, 0.0, 3.0, ExportMode::KeyframeCopy).expect("plain copy still works");
}

/// Distinct H.264 SPS NAL units in a file's video, read as Annex-B.
fn distinct_sps(path: &Path) -> std::collections::BTreeSet<Vec<u8>> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-c", "copy", "-bsf:v", "h264_mp4toannexb", "-f", "h264", "-"])
        .output()
        .expect("run ffmpeg")
        .stdout;
    // Annex-B start codes are 00 00 01; a NAL runs to the next one. NAL type 7 is an SPS.
    let starts: Vec<usize> = (0..out.len().saturating_sub(3))
        .filter(|&i| out[i..i + 3] == [0, 0, 1])
        .collect();
    starts
        .iter()
        .zip(starts.iter().skip(1).chain(std::iter::once(&out.len())))
        .filter(|(&from, _)| out[from + 3] & 0x1f == 7)
        .map(|(&from, &to)| out[from + 3..to].to_vec())
        .map(|mut nal| {
            // Drop the zero padding that belongs to the next start code.
            while nal.last() == Some(&0) {
                nal.pop();
            }
            nal
        })
        .collect()
}

/// The re-encoded head has its own SPS/PPS, different from the source's when
/// the source was encoded with other settings. A plain mp4 concat would keep
/// only the head's and decode the copied body against the wrong parameters, so
/// the output has to carry the source's own SPS alongside the head's.
#[test]
fn frame_accurate_keeps_the_sources_parameter_sets_for_the_copied_body() {
    require_ffmpeg!();
    let source = encode("fa-refs.mp4", &["-c:v", "libx264", "-g", "48", "-bf", "2", "-refs", "6", "-pix_fmt", "yuv420p"], &[], 1);
    let dest = workdir().join("fa-refs-out.mp4");
    export_to(&source, &dest, 2.5, 7.0, ExportMode::FrameAccurate).expect("export");

    let original = distinct_sps(&source);
    let written = distinct_sps(&dest);
    assert_eq!(original.len(), 1, "fixture should use a single SPS");
    assert!(
        original.is_subset(&written),
        "the copied body lost the source's SPS",
    );
    assert!(written.len() >= 2, "the re-encoded head should bring an SPS of its own");
}

/// A transport stream's timestamps start well past zero. The zero-based cut
/// points go to ffmpeg as they are (it measures them from the container's
/// `start_time` itself); adding `start_time` back seeks past the end of the file
/// and writes nothing.
#[test]
fn keyframe_copy_handles_a_transport_stream_with_a_non_zero_start() {
    require_ffmpeg!();
    let source = encode("fa.ts", &H264, &["-output_ts_offset", "1440.5"], 1);
    let dest = workdir().join("copy-from-ts.ts");

    export_to(&source, &dest, 3.2, 6.4, ExportMode::KeyframeCopy).expect("export");

    assert!(
        (video_span(&dest) - 3.2).abs() < 0.3,
        "extracted {:.3}s of video, want about 3.2s",
        video_span(&dest),
    );
}
