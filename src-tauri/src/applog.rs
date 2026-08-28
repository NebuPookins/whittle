//! Diagnostic log to a file.
//!
//! The webview's console is not reachable from a normal run, and the failure
//! being chased lives in the seam between the two: whether the media element
//! ever asks for the file, and what it does with what it gets. Both sides write
//! here so the sequence can be read in one place.
//!
//! Truncated on every launch, so a file holds exactly one run.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// `$WHITTLE_LOG`, or `whittle.log` in the temp directory.
pub fn path() -> PathBuf {
    std::env::var_os("WHITTLE_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("whittle.log"))
}

fn started() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

fn sink() -> Option<&'static Mutex<File>> {
    static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    SINK.get_or_init(|| {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path())
            .ok()
            .map(Mutex::new)
    })
    .as_ref()
}

/// Open the log for this run and announce where it is.
pub fn init() {
    let _ = started();
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    match sink() {
        Some(_) => {
            line("app", &format!("log opened, unix_ms={epoch_ms}"));
            eprintln!("Whittle log: {}", path().display());
        }
        None => eprintln!(
            "Whittle could not open a log at {} — set $WHITTLE_LOG to a writable path.",
            path().display()
        ),
    }
}

/// Append one line, tagged with `scope` and milliseconds since launch.
pub fn line(scope: &str, message: &str) {
    let Some(sink) = sink() else { return };
    let ms = started().elapsed().as_millis();
    let mut file = sink.lock().unwrap_or_else(|e| e.into_inner());
    let _ = writeln!(file, "[{ms:>8}ms] {scope:<8} {message}");
    // Flushed per line: the run this log exists to explain may end in a crash,
    // and a buffered tail would lose exactly the part that mattered.
    let _ = file.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of this module is that a line survives to disk, so that
    /// is what gets asserted — through the real file, not a fake sink.
    ///
    /// Sole test in this module by necessity: the sink binds once per process.
    #[test]
    fn lines_reach_the_file_and_carry_their_scope() {
        let target = std::env::temp_dir().join(format!(
            "whittle-applog-test-{}.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&target);
        std::env::set_var("WHITTLE_LOG", &target);

        init();
        line("probe", "opened /v/clip.mkv");
        line("preview", "-> 206 bytes 0-4194303/900000000");

        let written = std::fs::read_to_string(&target).expect("log file should exist");
        assert!(written.contains("opened /v/clip.mkv"), "{written}");
        assert!(written.contains("probe"), "{written}");
        assert!(written.contains("-> 206"), "{written}");
        assert_eq!(written.lines().count(), 3, "header plus two lines: {written}");

        let _ = std::fs::remove_file(&target);
    }
}
