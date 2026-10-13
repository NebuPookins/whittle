//! Shared by the integration tests that drive the real ffmpeg/ffprobe.

#![allow(dead_code)]

use std::process::{Command, Stdio};

pub fn have_ffmpeg() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|tool| {
        Command::new(tool)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    })
}

/// Skip the calling test, with a note, when ffmpeg is not installed.
macro_rules! require_ffmpeg {
    () => {
        if !common::have_ffmpeg() {
            eprintln!("skipping: ffmpeg not installed");
            return;
        }
    };
}
