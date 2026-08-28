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

use whittle_lib::keyframes::{keyframe_args, normalize, parse_packet_line};
use whittle_lib::probe::{metadata_args, parse_metadata};
use whittle_lib::proc::run;
use whittle_lib::trim;

const DURATION: f64 = 12.0;
/// 48-frame GOP at 30fps.
const GOP: f64 = 1.6;

fn have_ffmpeg() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|tool| {
        Command::new(tool)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    })
}

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
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not installed");
        return;
    }
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
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not installed");
        return;
    }
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
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not installed");
        return;
    }
    // 3.2 and 6.4 are both keyframes, so the result should be exact.
    let (start, end) = (2.0 * GOP, 4.0 * GOP);

    for name in ["cut.mp4", "cut.mkv", "cut.mov"] {
        let source = fixture(name);
        let dest = workdir().join(format!("out-{name}"));
        let _ = std::fs::remove_file(&dest);

        let report = trim::export(
            &source.to_string_lossy(),
            start,
            end,
            0.0,
            &dest.to_string_lossy(),
        )
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
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not installed");
        return;
    }
    let source = fixture("sweep.mkv");
    let keyframes = probe_keyframes(&source);

    for kf in keyframes.iter().copied().take(5) {
        let end = kf + GOP;
        if end > DURATION - 0.1 {
            continue;
        }
        let dest = workdir().join(format!("sweep-{kf:.1}.mkv"));
        let _ = std::fs::remove_file(&dest);

        trim::export(&source.to_string_lossy(), kf, end, 0.0, &dest.to_string_lossy())
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
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not installed");
        return;
    }
    let source = fixture("guard.mp4");
    let before = std::fs::metadata(&source).unwrap().len();

    let err = trim::export(
        &source.to_string_lossy(),
        1.6,
        3.2,
        0.0,
        &source.to_string_lossy(),
    )
    .expect_err("writing over the source must be refused");

    assert_eq!(err.kind(), "invalid");
    assert_eq!(
        std::fs::metadata(&source).unwrap().len(),
        before,
        "the source file was modified",
    );
}
