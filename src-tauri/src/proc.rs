use std::process::{Command, Output, Stdio};

use crate::error::AppError;

/// Run a tool to completion, capturing stdout.
///
/// `stdin` is closed so ffmpeg/ffprobe can never block waiting on input.
pub fn run(tool: &'static str, args: &[String]) -> Result<String, AppError> {
    let output = spawn(tool, args)?;
    check(tool, &output)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn spawn(tool: &'static str, args: &[String]) -> Result<Output, AppError> {
    Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::ToolMissing { tool },
            _ => AppError::Io(e.to_string()),
        })
}

fn check(tool: &'static str, output: &Output) -> Result<(), AppError> {
    if output.status.success() {
        return Ok(());
    }
    Err(AppError::ToolFailed {
        tool,
        code: output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into()),
        stderr: tail(&String::from_utf8_lossy(&output.stderr), 600),
    })
}

/// Keep the last `max` characters — ffmpeg's useful diagnostics are at the end.
pub fn tail(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    let n = trimmed.chars().count();
    if n <= max {
        return trimmed.to_string();
    }
    let skipped = n - max;
    format!("…{}", trimmed.chars().skip(skipped).collect::<String>())
}

/// Extract the version token from a `<tool> -version` banner.
///
/// `"ffmpeg version n9.0.1 Copyright…"` -> `Some("n9.0.1")`
pub fn parse_version(banner: &str) -> Option<String> {
    let first = banner.lines().next()?;
    let mut it = first.split_whitespace();
    while let Some(tok) = it.next() {
        if tok == "version" {
            return it.next().map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_from_banner() {
        let banner = "ffmpeg version n9.0.1 Copyright (c) 2000-2026 the FFmpeg developers\nbuilt with gcc";
        assert_eq!(parse_version(banner).as_deref(), Some("n9.0.1"));
    }

    #[test]
    fn parses_distro_style_version() {
        let banner = "ffprobe version 6.1.1-3ubuntu5 Copyright (c) 2007-2023";
        assert_eq!(parse_version(banner).as_deref(), Some("6.1.1-3ubuntu5"));
    }

    #[test]
    fn version_absent_is_none() {
        assert_eq!(parse_version("something unexpected"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn tail_keeps_the_end_and_marks_truncation() {
        assert_eq!(tail("abcdef", 10), "abcdef");
        let t = tail("abcdefghij", 4);
        assert!(t.ends_with("ghij"), "got {t:?}");
        assert!(t.starts_with('…'));
    }
}
