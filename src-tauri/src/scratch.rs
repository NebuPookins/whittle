//! Scratch space for an export: a hidden directory beside the destination that
//! holds intermediates and the output while it is being written.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::AppError;

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
/// body can be many gigabytes, and `/tmp` is often a small tmpfs. Being on the
/// destination's filesystem also lets a finished output be renamed into place.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn beside(output: &Path) -> Result<Self, AppError> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let parent = output
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

    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Where to stage the finished output, named `name`. It gets a directory
    /// of its own so that no name, `body.ts` say, can collide with an
    /// intermediate written by `file`.
    pub fn output(&self, name: &OsStr) -> Result<PathBuf, AppError> {
        let dir = self.0.join("output");
        std::fs::create_dir(&dir)?;
        Ok(dir.join(name))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
