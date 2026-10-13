//! Frame-accurate export: a "smart cut".
//!
//! A stream copy can only start on a keyframe. To start on *any* frame, the
//! frames between the requested in-point and the next keyframe are re-encoded
//! (the **head**), and everything from that keyframe on is copied untouched
//! (the **body**). The out-point needs the same treatment for a different
//! reason, below, so the last partial GOP is re-encoded too (the **tail**).
//!
//! ```text
//!   S         K1                          K2        E
//!   |  head   |            body           |  tail   |
//!   re-encode |        stream copy        | re-encode
//! ```
//!
//! # Why the out-point needs a tail
//!
//! Measured: a stream copy of a B-frame source overshoots its `-to`. Ending at
//! 7.0s from a keyframe at 3.2s gives 116 frames where 114 fit (30fps, `-bf 2`),
//! in mp4 and in mkv alike, and ending *on* a keyframe still gives two extra.
//! Input `-to` compares **decode** timestamps, which run two frames (the reorder
//! delay) behind display order, so the copy keeps every packet decoded before
//! `-to`, including the keyframe the range was meant to stop at and its
//! successor. So the cut is not frame-accurate at its end, and the body is
//! stopped just short of the dts of the keyframe it ends at
//! (`body_end_bound`), with the final partial GOP decoded and re-encoded to
//! land on the exact frame.
//!
//! # Joining
//!
//! The re-encoded segments carry their own SPS/PPS (or VPS/SPS/PPS), which can
//! differ from the source's: a source encoded with `-refs 6` and the head
//! re-encoded with the encoder's defaults have different SPS. An mp4 or
//! Matroska muxer writes one decoder configuration for the whole stream, taken
//! from the first packets, so a naive concat of mp4 pieces would describe the
//! copied body with the head's parameters. The pieces are therefore MPEG-TS
//! with Annex-B bitstreams, which carry parameter sets in-band at every
//! keyframe, and are joined with the concat demuxer; the mp4/mkv muxer then
//! keeps those in-band sets in the samples, so the body still has the source's
//! own SPS (`tests/ffmpeg_roundtrip.rs` checks that both are present). The
//! concat demuxer rebases each piece onto the end of the last, so the joined
//! stream starts at 0 and is gap-free: every frame exactly one frame apart.
//!
//! # Open GOPs
//!
//! Streams with open GOPs (x265's default) have keyframes whose *leading
//! pictures* display before the keyframe but reference the previous GOP. A
//! copy that starts at such a keyframe is undecodable for those pictures once
//! it follows other footage (ffmpeg reports `Could not find ref with POC` and
//! the body comes out a frame long). That cannot be repaired without
//! re-encoding, so when the copied body opens with leading pictures the whole
//! range is re-encoded instead. Frames that lead the keyframe at the *end* of
//! the body are the converse: they are not in the body, so the tail starts
//! after the body's last measured frame rather than at the keyframe, and a tail
//! is encoded for them even when the out-point is exactly that keyframe.
//!
//! # Finding keyframes
//!
//! ffprobe's interval reads open on the keyframe before the interval only when
//! the container can seek. On an MPEG-TS (no index) the seek is a byte estimate
//! that can land past it, so `Job::read_window` reports where a read really
//! opened and the lookups widen the window until they can trust it.
//!
//! # Limits
//!
//! H.264 and HEVC only, progressive, unrotated; anything else is refused up
//! front rather than risking a clip whose halves disagree. Frame times are
//! snapped onto a constant-rate grid, so a variable-frame-rate source is cut to
//! within half a frame. Audio and subtitles are stream-copied, so they begin on
//! a packet boundary (~21ms for AAC) of the frame the video starts on.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::error::AppError;
use crate::keyframes::is_keyframe_flags;
use crate::probe::{
    container_start_time, installed_encoder, EncoderStatus, is_video_track, metadata_args, num, packet_args, parse_json, rational_parts, uint32,
};
use crate::proc::{ffmpeg_base, format_ts, run, strings};

/// Source video codecs the smart cut can re-encode a matching head and tail for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartCodec {
    H264,
    Hevc,
}

impl SmartCodec {
    fn from_name(codec_name: &str) -> Option<Self> {
        match codec_name {
            "h264" => Some(Self::H264),
            "hevc" => Some(Self::Hevc),
            _ => None,
        }
    }

    fn encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::Hevc => "libx265",
        }
    }

    /// Converts the source's length-prefixed (mp4/mkv) NAL units to Annex-B so
    /// parameter sets travel in-band through MPEG-TS.
    fn annexb_filter(self) -> &'static str {
        match self {
            Self::H264 => "h264_mp4toannexb",
            Self::Hevc => "hevc_mp4toannexb",
        }
    }

    /// The encoder's spelling of the profile ffprobe reported, if there is one.
    fn encoder_profile(self, probed: &str) -> Option<&'static str> {
        match (self, probed.to_ascii_lowercase().as_str()) {
            (Self::H264, "baseline" | "constrained baseline") => Some("baseline"),
            (Self::H264, "main") => Some("main"),
            (Self::H264, "high") => Some("high"),
            (Self::H264, "high 10" | "high 10 intra") => Some("high10"),
            (Self::H264, "high 4:2:2" | "high 4:2:2 intra") => Some("high422"),
            (Self::H264, "high 4:4:4 predictive") => Some("high444"),
            (Self::Hevc, "main") => Some("main"),
            (Self::Hevc, "main 10") => Some("main10"),
            _ => None,
        }
    }
}

/// Every encoder `SmartCodec::encoder` can name.
pub(crate) const ENCODERS: [&str; 2] = ["libx264", "libx265"];

/// A frame rate as the exact fraction ffprobe reports (`30000/1001`), kept
/// whole so it can be handed back to ffmpeg without rounding.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FrameRate {
    num: u32,
    den: u32,
}

impl FrameRate {
    fn parse(v: Option<&Value>) -> Option<Self> {
        let (num, den) = rational_parts(v?.as_str()?)?;
        Some(Self { num, den })
    }

    /// Seconds per frame.
    fn frame_duration(self) -> f64 {
        f64::from(self.den) / f64::from(self.num)
    }

    /// A `setpts` filter that stamps frame `N` at exactly `N` frame durations.
    fn setpts(self) -> String {
        format!("setpts=N*{}/({}*TB)", self.den, self.num)
    }
}

/// What the head and tail encodes have to match so they can join the body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourceVideo {
    /// Index of the primary video stream in the container.
    index: u32,
    codec: SmartCodec,
    profile: Option<&'static str>,
    /// H.264 `level_idc` (e.g. 41 for 4.1). Not forwarded for HEVC.
    level: Option<u32>,
    pix_fmt: Option<String>,
    colour: Vec<(&'static str, String)>,
    frame_rate: FrameRate,
    /// The container's start offset: what separates the UI's 0-based times from
    /// the raw timestamps ffprobe and `-copyts` address.
    start_time: f64,
    /// Raw timestamp of the stream's first frame, if the container says.
    first_frame: Option<f64>,
    /// Frames the decoder holds back for reordering; how far dts trails pts.
    reorder_frames: u32,
}

/// ffprobe reports the colour tags as `unknown` or omits them when unset.
fn known(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| *s != "unknown" && !s.is_empty())
        .map(str::to_string)
}

/// Pick the primary video stream out of `ffprobe -show_streams` JSON and check
/// that a smart cut can reproduce it. The error says why it cannot.
pub(crate) fn parse_source_video(root: &Value) -> Result<SourceVideo, AppError> {
    let video = root
        .get("streams")
        .and_then(Value::as_array)
        .and_then(|streams| streams.iter().find(|s| is_video_track(s)))
        .ok_or_else(|| AppError::invalid("That file has no video track, so there is nothing to cut."))?;

    let codec_name = video.get("codec_name").and_then(Value::as_str).unwrap_or("unknown");
    let codec = SmartCodec::from_name(codec_name).ok_or_else(|| {
        AppError::invalid(format!(
            "Frame-accurate export supports H.264 and HEVC video only, not {codec_name}."
        ))
    })?;

    if !matches!(
        video.get("field_order").and_then(Value::as_str),
        None | Some("progressive" | "unknown")
    ) {
        return Err(AppError::invalid("Frame-accurate export does not support interlaced video."));
    }
    // A re-encode applies the rotation to the pixels while a copy would keep it
    // as metadata, so the two halves would disagree about which way is up.
    let rotated = video
        .get("side_data_list")
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().any(|d| num(d.get("rotation")).is_some_and(|r| r != 0.0)));
    if rotated {
        return Err(AppError::invalid("Frame-accurate export does not support rotated video."));
    }

    let frame_rate = FrameRate::parse(video.get("avg_frame_rate"))
        .or_else(|| FrameRate::parse(video.get("r_frame_rate")))
        .ok_or_else(|| AppError::invalid("Could not determine the video's frame rate, which a frame-accurate cut needs."))?;

    let colour = [
        ("-colorspace", "color_space"),
        ("-color_primaries", "color_primaries"),
        ("-color_trc", "color_transfer"),
        ("-color_range", "color_range"),
    ]
    .into_iter()
    .filter_map(|(flag, key)| known(video.get(key)).map(|v| (flag, v)))
    .collect();

    Ok(SourceVideo {
        index: uint32(video.get("index")).unwrap_or(0),
        codec,
        profile: video
            .get("profile")
            .and_then(Value::as_str)
            .and_then(|p| codec.encoder_profile(p)),
        level: match codec {
            SmartCodec::H264 => uint32(video.get("level")).filter(|l| *l > 0),
            SmartCodec::Hevc => None,
        },
        pix_fmt: known(video.get("pix_fmt")),
        colour,
        frame_rate,
        start_time: container_start_time(root, video),
        first_frame: num(video.get("start_time")).filter(|t| t.is_finite()),
        reorder_frames: uint32(video.get("has_b_frames")).unwrap_or(0),
    })
}

impl SourceVideo {
    /// The encoder the head and tail are made with.
    pub(crate) fn encoder(&self) -> &'static str {
        self.codec.encoder()
    }
}

/// Whether the installed ffmpeg can make the head and tail for `source`. The
/// error says what is missing.
pub(crate) fn check_encoder(source: &SourceVideo, status: &EncoderStatus) -> Result<(), AppError> {
    let encoder = source.encoder();
    match (status, source.pix_fmt.as_deref()) {
        (EncoderStatus::Missing, _) => Err(AppError::invalid(format!(
            "Frame-accurate export needs {encoder}, and this ffmpeg was built without it."
        ))),
        (EncoderStatus::Available { pix_fmts: Some(formats) }, Some(wanted)) if !formats.iter().any(|f| f == wanted) => {
            Err(AppError::invalid(format!(
                "Frame-accurate export cannot re-encode {wanted} video: this ffmpeg's {encoder} does not support it."
            )))
        }
        _ => Ok(()),
    }
}

/// A video packet that starts a decodable run. Times are 0-based seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Keyframe {
    pts: f64,
    dts: Option<f64>,
}

/// Where the frames `[start, end)` go.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CutPlan {
    /// No usable keyframe pair inside the range: re-encode all of it.
    Reencode,
    /// Copy `first..last`, re-encoding what falls outside it.
    Splice {
        first: Keyframe,
        last: Keyframe,
        head: bool,
        tail: bool,
    },
}

/// Decide what to copy and what to re-encode.
///
/// `start` and `end` are the requested in- and out-points; `first` is the first
/// keyframe at or after `start - frame/2` and `last` the last at or before
/// `end + frame/2`. Half a frame of slack on each side is what makes a
/// playhead time like 2.5000001 mean "the frame at 2.5" rather than "the next
/// one", and an end exactly on a keyframe count as being on it.
fn plan_cut(start: f64, end: f64, frame: f64, first: Option<Keyframe>, last: Option<Keyframe>) -> CutPlan {
    let half = frame / 2.0;
    match (first, last) {
        (Some(first), Some(last)) if last.pts - first.pts >= half && first.pts < end - half => CutPlan::Splice {
            first,
            last,
            head: first.pts - start >= half,
            tail: end - last.pts >= half,
        },
        _ => CutPlan::Reencode,
    }
}

/// The time of the frame nearest `t`, on the grid of frames `frame` apart that
/// passes through `phase`. Assumes the frame rate holds steady to within half
/// a frame over the distance to `phase`.
fn snap_to_grid(t: f64, phase: f64, frame: f64) -> f64 {
    phase + ((t - phase) / frame).round() * frame
}

/// Where the copied body must stop: just short of the keyframe it ends at.
///
/// Input `-to` compares decode timestamps (see the module docs), and the
/// keyframe's own dts trails its pts by the reorder delay, so bounding at its
/// pts would let the keyframe and its successors leak into the body. Half a
/// frame under its dts keeps exactly the packets decoded before it. Some
/// demuxers (Matroska) report no dts, in which case it is the pts minus the
/// reorder delay.
fn body_end_bound(last: Keyframe, source: &SourceVideo) -> f64 {
    let frame = source.frame_rate.frame_duration();
    last.dts
        .unwrap_or(last.pts - f64::from(source.reorder_frames) * frame)
        - frame / 2.0
}

/// Which frames of the copied body made it, measured off the body itself.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BodyExtent {
    first: f64,
    last: f64,
    /// True when frames display before the body's first packet, i.e. it opens
    /// with leading pictures that reference footage which is not there.
    has_leading_pictures: bool,
}

/// `opening` is the body's first packets and `closing` its last, both as pts in
/// decode order; together they stand in for the whole body. That holds because
/// a frame can only display before the first packet within the first GOP, and
/// the last frame to display is in the final one. They may overlap.
fn body_extent(opening: &[f64], closing: &[f64]) -> Option<BodyExtent> {
    let first_packet = *opening.first()?;
    let all = || opening.iter().chain(closing).copied();
    let min = opening.iter().copied().fold(f64::INFINITY, f64::min);
    let max = all().fold(f64::NEG_INFINITY, f64::max);
    Some(BodyExtent {
        first: min,
        last: max,
        has_leading_pictures: min < first_packet - 1e-4,
    })
}

/* ---------- argument builders ---------- */

/// Packets of one stream in `[from, to]`, for finding keyframes without
/// reading the whole file. ffprobe seeks to the keyframe *before* `from`, so
/// callers filter by time.
fn packet_window_args(path: &str, stream: u32, from: f64, to: f64) -> Vec<String> {
    packet_args(
        path,
        &stream.to_string(),
        Some(&format!("{}%{}", format_ts(from.max(0.0)), format_ts(to.max(0.0)))),
        "pts_time,dts_time,duration_time,flags",
    )
}

/// Where to open a window ending at `at`: a second back, then four times as far
/// each time, the last one at or before the start of the file. The wider the
/// window, the surer it opens before the time asked for.
fn widening_starts(at: f64) -> impl Iterator<Item = f64> {
    std::iter::successors(Some(at - 1.0), move |start| (*start > 0.0).then(|| at - (at - start) * 4.0))
}

/// What an ffprobe interval read returned.
#[derive(Debug, Clone, PartialEq, Default)]
struct Window {
    /// Decode time of the first packet read: where the interval really opened.
    opens_at: Option<f64>,
    keyframes: Vec<Keyframe>,
}

/// The decode time (pts if there is none) of any packet row, keyframe or not.
fn parse_decode_time(line: &str) -> Option<f64> {
    let mut fields = line.split(',').map(str::trim);
    let pts = fields.next()?.parse::<f64>().ok().filter(|t| t.is_finite());
    let dts = fields.next().and_then(|d| d.parse::<f64>().ok()).filter(|d| d.is_finite());
    dts.or(pts)
}

/// Parse a keyframe row: `pts,dts,duration,flags[,…]`, e.g. `3.2,3.133,0.033,K__`.
fn parse_keyframe_row(line: &str) -> Option<Keyframe> {
    let fields: Vec<&str> = line.trim().split(',').map(str::trim).collect();
    if !is_keyframe_flags(fields.get(3)?) {
        return None;
    }
    let pts = fields.first()?.parse::<f64>().ok().filter(|t| t.is_finite())?;
    Some(Keyframe {
        pts,
        dts: fields.get(1).and_then(|d| d.parse::<f64>().ok()).filter(|d| d.is_finite()),
    })
}

/// Decode `[start, end)` of the source and re-encode it to MPEG-TS, matching
/// the source's codec family, profile, pixel format, colour tags, frame size
/// and aspect ratio (the filter graph passes both through untouched).
///
/// The frames are chosen by **raw timestamp**, with `-copyts`, using a `trim`
/// filter whose thresholds sit midway between the first kept frame and its
/// predecessor, and between the last and its successor: as far from every frame
/// as they can be, so a timestamp off by a fraction of a frame cannot flip one
/// in or out. The input `-ss` only has to land on *some* keyframe at or before
/// `start`; it goes to `anchor`, one the caller has located, with
/// `-noaccurate_seek` so that ffmpeg does no frame discarding of its own. Every
/// other way of
/// choosing frames was measured and found wanting:
///
/// - A plain input `-ss` is unreliable on a transport stream, which has no
///   index: ffmpeg seeks to the keyframe *nearest* the target by byte
///   position, not the one before it. Asked for 2.48s with keyframes at 1.62s
///   and 3.22s it began at 3.22s, and the "head" held the wrong footage.
/// - ffmpeg's own timeline is not the container's. Output-side `-ss`/`-to`, or
///   discarding by the input `-ss`, put frames a fraction of a frame from where
///   `start_time` says (ffmpeg logged "Correcting start time of Input #0 by
///   23222 us" on a transport stream whose video starts after its audio, and
///   under `-copyts` the same discard then emptied the output), `-ss 0.001`
///   and `-ss -0.001` differing by two frames, and `-fps_mode cfr` duplicates
///   the first frame whenever the offset reaches half a frame. `-copyts` keeps
///   the container's own timestamps.
/// - Counting frames from the keyframe depends on exactly which keyframe the
///   seek lands on, which differs by container.
///
/// The kept frames are then renumbered onto an exact grid with `setpts`, with
/// `-fps_mode passthrough` so none is duplicated or dropped. Matroska's
/// millisecond timestamps (33ms, 33ms, 34ms…) would otherwise reach x264 as
/// they are, and it turned them into duplicate output timestamps: frames that
/// display twice.
///
/// `start` is the time of the first frame to keep and `end` the time just
/// after the last one (the next frame's, exclusive). All times are 0-based;
/// the source's `start_time` puts them back on the container's raw timeline,
/// which is what `-copyts` and `-seek_timestamp` address.
fn reencode_args(input: &str, source: &SourceVideo, anchor: f64, start: f64, end: f64, output: &str) -> Vec<String> {
    let half = source.frame_rate.frame_duration() / 2.0;
    let raw = |t: f64| t + source.start_time;
    let trim = format!(
        "trim=start={}:end={},{}",
        format_ts(raw(start) - half),
        format_ts(raw(end) - half),
        source.frame_rate.setpts(),
    );

    [
        ffmpeg_base(),
        // `-seek_timestamp` makes -ss a raw timestamp; otherwise ffmpeg adds
        // start_time itself, and which start_time it uses is not always ours.
        strings([
            "-copyts",
            "-noaccurate_seek",
            "-seek_timestamp",
            "1",
            "-ss",
            &format_ts((raw(anchor) - half).max(0.0)),
            "-i",
            input,
            "-map",
            &format!("0:{}", source.index),
            "-c:v",
            source.codec.encoder(),
            "-crf",
            "18",
        ]),
        source.profile.map_or_else(Vec::new, |p| strings(["-profile:v", p])),
        source.level.map_or_else(Vec::new, |l| strings(["-level:v", &l.to_string()])),
        source.pix_fmt.as_deref().map_or_else(Vec::new, |f| strings(["-pix_fmt", f])),
        source
            .colour
            .iter()
            .flat_map(|(flag, value)| strings([flag, value]))
            .collect(),
        strings(["-vf", &trim, "-fps_mode", "passthrough", "-f", "mpegts", output]),
    ]
    .concat()
}

/// Stream-copy the body to MPEG-TS, keyframe to keyframe.
///
/// Same `-seek2any`/input-side `-ss`/`-to` reasoning as `trim::trim_args`, with
/// two differences: the start is pulled back half a frame so float noise in a
/// keyframe's printed time cannot land just past it and skip it, and the end is
/// `body_end_bound` rather than a time the user chose.
fn copy_body_args(input: &str, source: &SourceVideo, first: Keyframe, bound: f64, output: &str) -> Vec<String> {
    [
        ffmpeg_base(),
        strings([
            "-seek2any",
            "1",
            "-ss",
            &format_ts(first.pts - source.frame_rate.frame_duration() / 2.0),
            "-to",
            &format_ts(bound),
            "-i",
            input,
            "-map",
            &format!("0:{}", source.index),
            "-c",
            "copy",
            "-bsf:v",
            source.codec.annexb_filter(),
            "-f",
            "mpegts",
            output,
        ]),
    ]
    .concat()
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// The concat demuxer's playlist.
fn concat_list(segments: &[PathBuf]) -> String {
    segments.iter().map(|p| format!("file {}\n", quoted(p))).collect()
}

/// Final mux: the joined video, plus everything else from the source over
/// exactly `[start, end]`, into whatever container `output`'s extension names.
///
/// The video comes from the concat input and the source's own primary video is
/// dropped by index (`-map -1:N`), so extra audio tracks, subtitles and
/// attachments all come along as with `-map 0` in the plain copy. The other
/// streams' `-ss`/`-to` are the usual input-side pair; they are `-c copy`, so
/// they land on packet boundaries (an AAC frame is ~21ms), which is where a
/// little audio/video skew comes from.
///
/// `-copypriorss 0` keeps ffmpeg from copying the packets that precede `-ss`.
/// Left at its default, a transport stream's audio began 0.27s early (the
/// demuxer lands before the target and the copy keeps what it landed on), and
/// an mp4's began 85ms early with its priming samples.
fn mux_args(list: &str, input: &str, source: &SourceVideo, start: f64, end: f64, output: &str) -> Vec<String> {
    [
        ffmpeg_base(),
        strings(["-f", "concat", "-safe", "0", "-i", list]),
        strings(["-seek2any", "1", "-ss", &format_ts(start), "-to", &format_ts(end), "-i", input]),
        strings(["-map", "0:v:0", "-map", "1", "-map", &format!("-1:{}", source.index)]),
        strings(["-map_metadata", "1", "-map_chapters", "1"]),
        strings(["-map_metadata:s:v:0", &format!("1:s:{}", source.index)]),
        strings(["-c", "copy", "-copypriorss", "0", output]),
    ]
    .concat()
}

/* ---------- orchestration ---------- */

/// Prefix of every scratch directory's name; the rest is `<pid>-<nanos>-<n>`.
const SCRATCH_PREFIX: &str = ".whittle-cut-";

/// The pid in a scratch directory's name, if `name` is exactly one of ours.
fn scratch_owner(name: &str) -> Option<u32> {
    let mut parts = name.strip_prefix(SCRATCH_PREFIX)?.split('-');
    let (pid, nanos, n) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (parts.next().is_none() && digits(nanos) && digits(n) && digits(pid))
        .then(|| pid.parse().ok())
        .flatten()
}

/// Whether process `pid` is running. Where `/proc` does not exist this cannot be
/// told, and the answer is yes: a directory is only ever deleted for sure.
fn process_alive(pid: u32) -> bool {
    !Path::new("/proc/self").exists() || Path::new("/proc").join(pid.to_string()).exists()
}

/// Delete scratch directories in `dir` left by exports that died without
/// cleaning up: only directories named exactly like ours, and only those whose
/// owning process is gone.
fn remove_stale_scratch(dir: &Path, alive: impl Fn(u32) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| e.file_name().to_str().and_then(scratch_owner).is_some_and(|pid| !alive(pid)))
        .for_each(|e| {
            let _ = std::fs::remove_dir_all(e.path());
        });
}

/// A uniquely named hidden directory, removed on drop so the intermediates go
/// away whether the export succeeds or fails.
///
/// A panic unwinds through the drop too, but the release build is
/// `panic = "abort"` and a crash or kill skips it altogether, so nothing is
/// guaranteed there. Instead the next export into the same directory sweeps up
/// what a dead process left (`remove_stale_scratch`).
///
/// It sits beside the destination rather than in the system temp dir: a copied
/// body can be many gigabytes, and `/tmp` is often a small tmpfs.
struct Scratch(PathBuf);

impl Scratch {
    fn beside(output: &str) -> Result<Self, AppError> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let parent = Path::new(output)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        remove_stale_scratch(parent, process_alive);
        let dir = parent.join(format!(
            "{SCRATCH_PREFIX}{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&dir)?;
        Ok(Self(dir))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// pts of every row of a `packet=pts_time` listing, in decode order.
fn pts_times(listing: &str) -> Vec<f64> {
    listing
        .lines()
        .filter_map(|l| l.split(',').next()?.trim().parse::<f64>().ok())
        .collect()
}

/// Measure the copied body by probing only its opening and its final stretch,
/// never the whole file: a body can run to many gigabytes.
///
/// `span` is the body's expected length. The closing read starts two seconds
/// before the expected end; ffprobe opens an interval on the keyframe before
/// it, so it covers the final GOP however long that is.
fn measure_body(path: &Path, span: f64) -> Result<Option<BodyExtent>, AppError> {
    let file = path_str(path);
    let times = |interval: &str| -> Result<Vec<f64>, AppError> {
        run("ffprobe", &packet_args(&file, "v:0", Some(interval), "pts_time")).map(|out| pts_times(&out))
    };
    let opening = times("%+2")?;
    let Some(&opens_at) = opening.first() else {
        return Ok(None);
    };
    let closing = times(&format!("{}%", format_ts((opens_at + span - 2.0).max(0.0))))?;
    Ok(body_extent(&opening, &closing))
}

/// The range being cut, on the grid of frames: `[start, end)`, and a keyframe at
/// or before `start` to decode it from.
#[derive(Debug, Clone, Copy)]
struct Span {
    anchor: f64,
    start: f64,
    end: f64,
}

/// One export in flight: the source, and where its intermediates go.
struct Job<'a> {
    input: &'a str,
    source: &'a SourceVideo,
    scratch: &'a Scratch,
}

impl Job<'_> {
    /// The packets in `[from, to]`, in the same 0-based seconds as everything
    /// else, along with when the first one was decoded. ffprobe addresses the raw
    /// container timeline, so `start_time` is added going in and taken off coming
    /// out.
    ///
    /// ffprobe is meant to open an interval on the keyframe before it, but on a
    /// container without an index (MPEG-TS) the seek is a byte estimate, and
    /// measured on a 15 minute transport stream it landed *past* `from` in half
    /// of the probes: the interval opened on a plain packet and the keyframe
    /// before it was missing. `opens_at` is how a caller tells.
    fn read_window(&self, from: f64, to: f64) -> Result<Window, AppError> {
        let offset = self.source.start_time;
        // A little past `to`, since the interval compares decode times and a
        // frame displayed by `to` can be decoded after it; an empty interval
        // (`to` at the very start of the file) reads nothing at all.
        let pad = f64::from(self.source.reorder_frames + 1) * self.source.frame_rate.frame_duration();
        let out = run(
            "ffprobe",
            &packet_window_args(self.input, self.source.index, from + offset, to + pad + offset),
        )?;
        Ok(Window {
            opens_at: out.lines().find_map(parse_decode_time).map(|t| t - offset),
            keyframes: out
                .lines()
                .filter_map(parse_keyframe_row)
                .map(|k| Keyframe { pts: k.pts - offset, dts: k.dts.map(|d| d - offset) })
                .collect(),
        })
    }

    /// The first keyframe at or after `from` and before `limit`. Searched in
    /// 16s windows so a long range is never read end to end. Each window opens
    /// early enough to be sure of the whole of `[from, to]` (see `read_window`),
    /// widening until it does.
    fn first_keyframe_from(&self, from: f64, limit: f64) -> Result<Option<Keyframe>, AppError> {
        let to = (from + 16.0).min(limit);
        let window = widening_starts(from)
            .find_map(|start| match self.read_window(start, to) {
                Ok(w) if start <= 0.0 || w.opens_at.is_some_and(|t| t <= from) => Some(Ok(w)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .unwrap_or_else(|| Ok(Window::default()))?;
        let found = window
            .keyframes
            .into_iter()
            .filter(|k| k.pts >= from && k.pts < limit)
            .min_by(|a, b| a.pts.total_cmp(&b.pts));
        match found {
            Some(k) => Ok(Some(k)),
            None if to >= limit => Ok(None),
            None => self.first_keyframe_from(to, limit),
        }
    }

    /// The last keyframe at or before `at`. A window ending at `at` holds every
    /// packet from wherever it opened to `at`, so the last keyframe in it is the
    /// right one however the seek landed; when the seek overshoots and the window
    /// holds none, a wider one is read, back to the start of the file.
    fn last_keyframe_until(&self, at: f64) -> Result<Keyframe, AppError> {
        widening_starts(at)
            .find_map(|start| match self.read_window(start, at) {
                Ok(w) => {
                    let last = w
                        .keyframes
                        .into_iter()
                        .filter(|k| k.pts <= at)
                        .max_by(|a, b| a.pts.total_cmp(&b.pts));
                    (last.is_some() || start <= 0.0).then_some(Ok(last))
                }
                Err(e) => Some(Err(e)),
            })
            .unwrap_or(Ok(None))?
            .ok_or_else(|| {
                AppError::invalid("Could not find a keyframe to decode the cut from, so it cannot be made frame-accurate.")
            })
    }

    /// Re-encode `[from, to)` to a segment named `name`, decoding from `anchor`.
    fn encode(&self, name: &str, anchor: f64, from: f64, to: f64) -> Result<PathBuf, AppError> {
        let out = self.scratch.file(name);
        run(
            "ffmpeg",
            &reencode_args(self.input, self.source, anchor, from, to, &path_str(&out)),
        )?;
        if std::fs::metadata(&out)?.len() == 0 {
            return Err(AppError::invalid("The selected range contains no video frames."));
        }
        Ok(out)
    }

    /// The segments that make up the span, in order.
    fn segments(&self, plan: CutPlan, span: Span) -> Result<Vec<PathBuf>, AppError> {
        let spliced = match plan {
            CutPlan::Reencode => None,
            CutPlan::Splice { first, last, head, tail } => self.splice(first, last, head, tail, span)?,
        };
        match spliced {
            Some(segments) => Ok(segments),
            None => Ok(vec![self.encode("all.ts", span.anchor, span.start, span.end)?]),
        }
    }

    /// Copy `first..last` and re-encode the head and tail around it. `None`
    /// when the copied body cannot be spliced onto other footage, and the whole
    /// range has to be re-encoded instead.
    fn splice(
        &self,
        first: Keyframe,
        last: Keyframe,
        head: bool,
        tail: bool,
        span: Span,
    ) -> Result<Option<Vec<PathBuf>>, AppError> {
        let frame = self.source.frame_rate.frame_duration();
        let body = self.scratch.file("body.ts");
        let bound = body_end_bound(last, self.source);
        run(
            "ffmpeg",
            &copy_body_args(self.input, self.source, first, bound, &path_str(&body)),
        )?;
        // The body's own timestamps are the muxer's, not the source's; only its
        // length carries over.
        let Some(extent) = measure_body(&body, bound - first.pts)?.filter(|e| !e.has_leading_pictures) else {
            return Ok(None);
        };
        let body_end = first.pts + (extent.last - extent.first);
        let tail_from = body_end + frame;
        // The plan only knows where the keyframes are. A closing keyframe with
        // leading pictures also leaves a gap, since the body stops before its
        // decode time and those pictures display before it, so the measured
        // body decides whether anything is left over, whatever the plan said.
        let tail = tail || span.end - tail_from >= frame / 2.0;

        let head = head.then(|| self.encode("head.ts", span.anchor, span.start, first.pts)).transpose()?;
        let tail = tail
            .then(|| {
                // The last keyframe anchors the tail unless its own leading
                // pictures put the tail's start before it.
                let anchor = if last.pts <= tail_from + frame / 2.0 {
                    last
                } else {
                    self.last_keyframe_until(tail_from)?
                };
                self.encode("tail.ts", anchor.pts, tail_from, span.end)
            })
            .transpose()?;
        Ok(Some(head.into_iter().chain([body]).chain(tail).collect()))
    }
}

/// Write `[start, end)` of `input` to `output`, starting on the exact frame.
///
/// Returns the range actually cut: `start`/`end` snapped onto the source's frames
/// (and `start` clamped to its first), which can differ from the request by up to
/// half a frame at either end.
///
/// `start`/`end` are 0-based like the rest of the UI, which is also what
/// ffmpeg's input `-ss`/`-to` take (it measures them from the container's
/// `start_time` itself) in the body copy and the final mux. The re-encodes, and
/// ffprobe's `-read_intervals`, address raw container timestamps instead, so
/// `start_time` is added back there (see `reencode_args`, `Job::keyframes_in`).
pub fn export(input: &str, start: f64, end: f64, output: &str) -> Result<Range<f64>, AppError> {
    let source = parse_source_video(&parse_json(&run("ffprobe", &metadata_args(input))?)?)?;
    check_encoder(&source, &installed_encoder(source.encoder()))?;
    let scratch = Scratch::beside(output)?;
    let job = Job { input, source: &source, scratch: &scratch };
    let frame = source.frame_rate.frame_duration();
    let half = frame / 2.0;

    // Nothing precedes the first frame, and a cut asked for before it (a mark at
    // 0 on a stream whose video starts a fraction of a frame late) would
    // otherwise snap to the grid slot *before* it and select no frames at all.
    let start = source.first_frame.map_or(start, |t| start.max(t - source.start_time));
    // Frames sit on a grid whose phase is whatever the first frame's was (a
    // transport stream's video can start a fraction of a frame after the
    // container does), so the requested times are snapped onto it. Every cut
    // boundary then falls midway between two frames, as far as it can be from
    // either, whichever way a timestamp is off by a little. The keyframe that
    // gives the phase also anchors the decode of whatever starts at `start`.
    let anchor = job.last_keyframe_until(start)?.pts;
    let (start, end) = (snap_to_grid(start, anchor, frame), snap_to_grid(end, anchor, frame));

    let plan = plan_cut(
        start,
        end,
        frame,
        job.first_keyframe_from(start - half, end - half)?,
        Some(job.last_keyframe_until(end + half)?),
    );
    let segments = job.segments(plan, Span { anchor, start, end })?;

    let list = scratch.file("segments.txt");
    std::fs::write(&list, concat_list(&segments))?;
    run("ffmpeg", &mux_args(&path_str(&list), input, &source, start, end, output))?;
    Ok(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f64 = 1.0 / 30.0;

    fn kf(pts: f64) -> Keyframe {
        Keyframe { pts, dts: Some(pts - 2.0 * FRAME) }
    }

    fn parse(json: &str) -> Result<SourceVideo, AppError> {
        parse_source_video(&parse_json(json)?)
    }

    fn source() -> SourceVideo {
        parse(SAMPLE).unwrap()
    }

    const SAMPLE: &str = r#"{"streams":[
        {"index":0,"codec_name":"mjpeg","codec_type":"video","disposition":{"attached_pic":1}},
        {"index":1,"codec_name":"h264","codec_type":"video","profile":"High","level":41,
         "pix_fmt":"yuv420p","color_space":"bt709","color_range":"tv","color_primaries":"unknown",
         "field_order":"progressive","has_b_frames":2,"avg_frame_rate":"30/1","r_frame_rate":"30/1",
         "disposition":{"attached_pic":0}},
        {"index":2,"codec_name":"aac","codec_type":"audio"}]}"#;

    #[test]
    fn only_h264_and_hevc_are_supported() {
        assert_eq!(SmartCodec::from_name("h264"), Some(SmartCodec::H264));
        assert_eq!(SmartCodec::from_name("hevc"), Some(SmartCodec::Hevc));
        for other in ["vp9", "av1", "mpeg4", "prores"] {
            assert_eq!(SmartCodec::from_name(other), None, "{other}");
        }
    }

    #[test]
    fn an_unsupported_codec_is_rejected_as_invalid() {
        let json = r#"{"streams":[{"index":0,"codec_name":"vp9","codec_type":"video","avg_frame_rate":"30/1"}]}"#;
        let err = parse(json).unwrap_err();
        assert_eq!(err.kind(), "invalid");
        assert!(err.to_string().contains("vp9"), "{err}");
    }

    #[test]
    fn rotated_and_interlaced_video_is_rejected() {
        let rotated = r#"{"streams":[{"index":0,"codec_name":"h264","codec_type":"video",
            "avg_frame_rate":"30/1","side_data_list":[{"rotation":-90}]}]}"#;
        let interlaced = r#"{"streams":[{"index":0,"codec_name":"h264","codec_type":"video",
            "avg_frame_rate":"30/1","field_order":"tt"}]}"#;
        assert_eq!(parse(rotated).unwrap_err().kind(), "invalid");
        assert_eq!(parse(interlaced).unwrap_err().kind(), "invalid");
    }

    #[test]
    fn picks_the_real_video_track_over_cover_art() {
        assert_eq!(source().index, 1);
    }

    #[test]
    fn concat_lists_escape_quotes() {
        let list = concat_list(&[PathBuf::from("/t/a.ts"), PathBuf::from("/t/it's.ts")]);
        assert_eq!(list, "file '/t/a.ts'\nfile '/t/it'\\''s.ts'\n");
    }

    #[test]
    fn parses_only_keyframe_rows() {
        assert_eq!(
            parse_keyframe_row("3.200000,3.133333,0.033333,K__"),
            Some(Keyframe { pts: 3.2, dts: Some(3.133333) }),
        );
        assert_eq!(parse_keyframe_row("1.6,N/A,0.033,K__").map(|k| k.dts), Some(None));
        assert_eq!(parse_keyframe_row("3.3,3.16,0.033,___"), None);
        assert_eq!(parse_keyframe_row("1441.9,1441.83,0.033,K__,MPEGTS Stream ID,224").map(|k| k.pts), Some(1441.9));
    }

    #[test]
    fn splices_when_both_ends_are_off_keyframe() {
        let plan = plan_cut(2.5, 7.0, FRAME, Some(kf(3.2)), Some(kf(6.4)));
        assert_eq!(plan, CutPlan::Splice { first: kf(3.2), last: kf(6.4), head: true, tail: true });
    }

    #[test]
    fn nothing_is_re_encoded_at_a_boundary_that_is_a_keyframe() {
        // A playhead a hair past the keyframe still means the keyframe's frame.
        let plan = plan_cut(3.2000001, 6.4, FRAME, Some(kf(3.2)), Some(kf(6.4)));
        assert_eq!(plan, CutPlan::Splice { first: kf(3.2), last: kf(6.4), head: false, tail: false });
    }

    #[test]
    fn a_range_without_two_distinct_keyframes_is_re_encoded_whole() {
        assert_eq!(plan_cut(2.5, 3.0, FRAME, None, Some(kf(1.6))), CutPlan::Reencode);
        assert_eq!(plan_cut(2.5, 3.0, FRAME, Some(kf(3.2)), Some(kf(3.2))), CutPlan::Reencode);
        // The only keyframe sits at the end: it starts nothing.
        assert_eq!(plan_cut(2.5, 3.2, FRAME, Some(kf(3.2)), Some(kf(3.2))), CutPlan::Reencode);
    }

    #[test]
    fn the_body_stops_before_its_closing_keyframe_is_decoded() {
        let s = source();
        // dts trails pts by two frames: bound is half a frame under that.
        let bound = body_end_bound(kf(6.4), &s);
        assert!((bound - (6.4 - 2.5 * FRAME)).abs() < 1e-9);
        // Without a dts the reorder delay stands in for it.
        let no_dts = Keyframe { pts: 6.4, dts: None };
        assert!((body_end_bound(no_dts, &s) - bound).abs() < 1e-9);
    }

    #[test]
    fn times_snap_to_the_nearest_frame_on_the_sources_own_grid() {
        // Frames at 0.0232 + n/30: a playhead near 2.5 means frame 74 (2.4899).
        let phase = 0.0232;
        assert!((snap_to_grid(2.5, phase, FRAME) - (phase + 74.0 * FRAME)).abs() < 1e-9);
        assert!((snap_to_grid(2.4900001, phase, FRAME) - (phase + 74.0 * FRAME)).abs() < 1e-9);
        // Already on a frame stays put.
        assert!((snap_to_grid(3.2232, 3.2232, FRAME) - 3.2232).abs() < 1e-12);
    }

    #[test]
    fn leading_pictures_are_detected_from_the_body_packets() {
        // Closed GOP: the keyframe is the earliest-displayed frame.
        let closed = body_extent(&[3.2, 3.3, 3.233, 3.267], &[]).unwrap();
        assert!(!closed.has_leading_pictures);
        assert_eq!((closed.first, closed.last), (3.2, 3.3));
        // Open GOP: pictures that display before the keyframe follow it.
        assert!(body_extent(&[3.2, 3.133, 3.167, 3.3], &[]).unwrap().has_leading_pictures);
        assert_eq!(body_extent(&[], &[4.0]), None);
    }

    #[test]
    fn only_directories_named_exactly_like_scratch_ones_have_an_owner() {
        assert_eq!(scratch_owner(".whittle-cut-4242-1700000000123456789-3"), Some(4242));
        for other in [
            ".whittle-cut-",
            ".whittle-cut-notes",
            ".whittle-cut-42-17",
            ".whittle-cut-42-17-3-extra",
            ".whittle-cut-x-17-3",
            ".whittle-cut--17-3",
            "whittle-cut-42-17-3",
            "holiday",
        ] {
            assert_eq!(scratch_owner(other), None, "{other}");
        }
    }

    #[test]
    fn the_decode_time_of_a_packet_row_is_its_dts_or_else_its_pts() {
        assert_eq!(parse_decode_time("2040.633333,2040.500000,___,MPEGTS Stream ID,224"), Some(2040.5));
        assert_eq!(parse_decode_time("1.6,N/A,0.033,K__"), Some(1.6));
        assert_eq!(parse_decode_time("N/A,N/A,0.033,___"), None);
        assert_eq!(parse_decode_time(""), None);
    }

    #[test]
    fn windows_open_ever_earlier_until_one_reaches_the_start_of_the_file() {
        let starts: Vec<f64> = widening_starts(100.0).collect();
        assert!(starts.windows(2).all(|w| w[1] < w[0]), "{starts:?}");
        assert!(starts.iter().all(|s| *s < 100.0));
        assert_eq!(starts.iter().filter(|s| **s <= 0.0).count(), 1, "{starts:?}");
        assert!(starts.last().is_some_and(|s| *s <= 0.0));
    }

    #[test]
    fn the_last_frame_can_come_from_the_closing_packets_alone() {
        let extent = body_extent(&[3.2, 3.3, 3.233], &[6.1, 6.2, 6.133]).unwrap();
        assert_eq!((extent.first, extent.last), (3.2, 6.2));
    }
}
