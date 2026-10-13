//! The smart cut writes intermediates to a hidden directory beside the
//! destination. Nothing but the output may be left there afterwards, whether
//! the export succeeded or failed.

#[macro_use]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use whittle_lib::trim::{self, ExportMode};

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").path())
        .collect();
    found.sort();
    found
}

fn export(source: &Path, dest: &Path) -> Result<trim::ExportReport, whittle_lib::error::AppError> {
    trim::export(&source.to_string_lossy(), 0.5, 4.0, &dest.to_string_lossy(), ExportMode::FrameAccurate)
}

#[test]
fn intermediates_are_removed_after_success_and_after_failure() {
    require_ffmpeg!();
    let root = std::env::temp_dir().join(format!("whittle-cleanup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let source = root.join("in.mp4");

    let built = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=6"])
        .args(["-c:v", "libx264", "-g", "48", "-bf", "2", "-pix_fmt", "yuv420p"])
        .arg(&source)
        .status()
        .expect("run ffmpeg");
    assert!(built.success());

    let ok = out.join("ok.mp4");
    export(&source, &ok).expect("export succeeds");
    assert_eq!(entries(&out), vec![ok], "the output should be all that is left after success");

    // An extension ffmpeg has no muxer for fails the final mux, after the
    // intermediates exist.
    let bad = out.join("x.nonsense");
    export(&source, &bad).expect_err("export to an unknown container fails");
    assert_eq!(entries(&out), vec![out.join("ok.mp4")], "left behind after failure");

    let _ = std::fs::remove_dir_all(&root);
}

/// A process that died mid-export leaves its scratch directory behind. The next
/// export beside it sweeps those up, and nothing else: not another running
/// export's directory, not anything that merely looks similar.
#[test]
fn leftovers_of_dead_exports_are_swept_and_nothing_else_is() {
    require_ffmpeg!();
    let root = std::env::temp_dir().join(format!("whittle-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let source = root.join("in.mp4");

    let built = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=6"])
        .args(["-c:v", "libx264", "-g", "48", "-bf", "2", "-pix_fmt", "yuv420p"])
        .arg(&source)
        .status()
        .expect("run ffmpeg");
    assert!(built.success());

    // Above Linux's largest pid_max (2^22), so no process can own it.
    let dead = out.join(".whittle-cut-4999999-1700000000000000000-0");
    // This test process is running, so is the owner of this one.
    let running = out.join(format!(".whittle-cut-{}-1700000000000000000-7", std::process::id()));
    let lookalikes = [
        out.join(".whittle-cut-notes"),
        out.join(".whittle-cut-4999999-1-0-extra"),
        out.join("holiday-footage"),
    ];
    for dir in [&dead, &running].into_iter().chain(&lookalikes) {
        std::fs::create_dir(dir).unwrap();
        std::fs::write(dir.join("keep.txt"), "x").unwrap();
    }
    let file_named_like_scratch = out.join(".whittle-cut-4999999-2-0");
    std::fs::write(&file_named_like_scratch, "not a directory").unwrap();

    let ok = out.join("ok.mp4");
    export(&source, &ok).expect("export succeeds");

    assert!(!dead.exists(), "the dead export's leftovers should be swept");
    for kept in [&running, &file_named_like_scratch, &ok].into_iter().chain(&lookalikes) {
        assert!(kept.exists(), "{} should not have been touched", kept.display());
    }
    for dir in lookalikes.iter().chain([&running]) {
        assert!(dir.join("keep.txt").exists(), "{} lost its contents", dir.display());
    }
    let _ = std::fs::remove_dir_all(&root);
}
