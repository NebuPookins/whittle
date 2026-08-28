use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::AppError;
use crate::proc::tail;

pub const EVENT_BATCH: &str = "keyframes-batch";
pub const EVENT_DONE: &str = "keyframes-done";

/// Flush at whichever of these comes first, so short files feel instant and
/// long ones still show steady progress.
const BATCH_SIZE: usize = 512;
const BATCH_INTERVAL: Duration = Duration::from_millis(150);

/// Monotonic scan counter. A scan whose id is no longer current has been
/// superseded (the user opened another file) and kills its own child process.
#[derive(Default)]
pub struct ScanGeneration(pub Arc<AtomicU64>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyframeBatch {
    pub scan_id: u64,
    pub times: Vec<f64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyframeDone {
    pub scan_id: u64,
    pub total: usize,
    pub cancelled: bool,
    pub error: Option<String>,
}

/// Demux-only keyframe listing.
///
/// `-show_packets` reads container packet headers; it never decodes, which is
/// what keeps this tractable on long files. (`frame=key_frame` would decode
/// every frame and take minutes on a feature-length video.)
pub fn keyframe_args(path: &str) -> Vec<String> {
    [
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_packets",
        "-show_entries",
        "packet=pts_time,flags",
        "-of",
        "csv=print_section=0",
        path,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Parse one CSV row, yielding the timestamp only for keyframe packets.
///
/// Rows look like `3.200000,K__`. Packets with an unknown pts (`N/A`) are
/// skipped, as are side-data rows such as `-0.023000,Skip Samples,1024,0,0,0`
/// which ffprobe can interleave — the flags field is always a short
/// `K`/`D`/`_` triple, so anything longer is not a packet row.
pub fn parse_packet_line(line: &str) -> Option<f64> {
    let mut fields = line.trim().split(',');
    let pts = fields.next()?.trim();
    let flags = fields.next()?.trim();

    if flags.len() > 4 || !flags.contains('K') {
        return None;
    }
    let t = pts.parse::<f64>().ok()?;
    t.is_finite().then_some(t)
}

/// Rebase to a 0-based timeline, then sort and dedupe.
///
/// Sorting is mandatory: packets arrive in *decode* order, so with B-frames the
/// presentation timestamps are not monotonic.
pub fn normalize(mut times: Vec<f64>, start_time: f64) -> Vec<f64> {
    for t in times.iter_mut() {
        *t = (*t - start_time).max(0.0);
    }
    times.retain(|t| t.is_finite());
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    times
}

/// Spawn a background scan. Returns immediately; results arrive as events.
pub fn spawn_scan(app: AppHandle, generation: Arc<AtomicU64>, path: String, scan_id: u64, start_time: f64) {
    generation.store(scan_id, Ordering::SeqCst);

    std::thread::spawn(move || {
        let outcome = scan(&app, &generation, &path, scan_id, start_time);
        let (total, cancelled, error) = match outcome {
            Ok(Scan::Finished { total }) => (total, false, None),
            Ok(Scan::Cancelled) => (0, true, None),
            Err(e) => (0, false, Some(e.to_string())),
        };
        let _ = app.emit(
            EVENT_DONE,
            KeyframeDone { scan_id, total, cancelled, error },
        );
    });
}

enum Scan {
    Finished { total: usize },
    Cancelled,
}

fn superseded(generation: &AtomicU64, scan_id: u64) -> bool {
    generation.load(Ordering::SeqCst) != scan_id
}

fn scan(
    app: &AppHandle,
    generation: &AtomicU64,
    path: &str,
    scan_id: u64,
    start_time: f64,
) -> Result<Scan, AppError> {
    let mut child = Command::new("ffprobe")
        .args(keyframe_args(path))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::ToolMissing { tool: "ffprobe" },
            _ => AppError::Io(e.to_string()),
        })?;

    // Drain stderr concurrently; a full pipe would otherwise deadlock the child.
    let mut stderr_pipe = child.stderr.take();
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.take(64 * 1024).read_to_string(&mut buf);
        }
        buf
    });

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Io("ffprobe produced no stdout".into()))?;

    let mut total = 0usize;
    let mut pending: Vec<f64> = Vec::with_capacity(BATCH_SIZE);
    let mut last_flush = Instant::now();

    let flush = |pending: &mut Vec<f64>, last: &mut Instant| {
        if pending.is_empty() {
            return;
        }
        let times = normalize(std::mem::take(pending), start_time);
        let _ = app.emit(EVENT_BATCH, KeyframeBatch { scan_id, times });
        *last = Instant::now();
    };

    for line in BufReader::new(stdout).lines() {
        if superseded(generation, scan_id) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Scan::Cancelled);
        }
        let Ok(line) = line else { break };
        if let Some(t) = parse_packet_line(&line) {
            pending.push(t);
            total += 1;
            if pending.len() >= BATCH_SIZE || last_flush.elapsed() >= BATCH_INTERVAL {
                flush(&mut pending, &mut last_flush);
            }
        }
    }
    flush(&mut pending, &mut last_flush);

    let status = child.wait()?;
    let stderr = stderr_thread.join().unwrap_or_default();

    if !status.success() {
        return Err(AppError::ToolFailed {
            tool: "ffprobe",
            code: status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
            stderr: tail(&stderr, 600),
        });
    }
    Ok(Scan::Finished { total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_keyframe_rows() {
        assert_eq!(parse_packet_line("3.200000,K__"), Some(3.2));
        assert_eq!(parse_packet_line("0.000000,K__"), Some(0.0));
        assert_eq!(parse_packet_line("0.100000,___"), None);
    }

    #[test]
    fn keeps_discardable_keyframes() {
        // A keyframe flagged discard is still a real seek point.
        assert_eq!(parse_packet_line("-0.300000,KD_"), Some(-0.3));
    }

    #[test]
    fn skips_unknown_timestamps_and_side_data_rows() {
        assert_eq!(parse_packet_line("N/A,K__"), None);
        assert_eq!(parse_packet_line("-0.023000,Skip Samples,1024,0,0,0"), None);
        assert_eq!(parse_packet_line(""), None);
        assert_eq!(parse_packet_line("3.2"), None);
    }

    #[test]
    fn sorts_decode_order_into_presentation_order() {
        // Packets arrive in decode order when B-frames are present.
        let got = normalize(vec![0.0, 3.2, 1.6], 0.0);
        assert_eq!(got, vec![0.0, 1.6, 3.2]);
    }

    #[test]
    fn removes_duplicates() {
        let got = normalize(vec![1.0, 1.0000001, 2.0], 0.0);
        assert_eq!(got, vec![1.0, 2.0]);
    }

    #[test]
    fn rebases_onto_a_zero_based_timeline() {
        let got = normalize(vec![1440.5, 1442.1], 1440.5);
        assert_eq!(got.len(), 2);
        assert!((got[0] - 0.0).abs() < 1e-9);
        assert!((got[1] - 1.6).abs() < 1e-9);
    }

    #[test]
    fn clamps_pre_start_keyframes_to_zero() {
        assert_eq!(normalize(vec![9.0], 10.0), vec![0.0]);
    }

    #[test]
    fn empty_input_stays_empty() {
        assert!(normalize(vec![], 0.0).is_empty());
    }

    #[test]
    fn args_request_packets_not_frames() {
        let args = keyframe_args("/v/a.mp4");
        assert!(args.contains(&"-show_packets".to_string()));
        assert!(args.iter().any(|a| a.contains("pts_time")));
        assert_eq!(args.last().unwrap(), "/v/a.mp4");
    }
}
