//! Durable same-directory atomic file replacement.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);
const TEMP_CREATE_ATTEMPTS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    Durable,
    CommittedNotDurable { warning: String },
}

impl WriteOutcome {
    pub fn warning(&self) -> Option<&str> {
        match self {
            Self::Durable => None,
            Self::CommittedNotDurable { warning } => Some(warning),
        }
    }

    pub fn is_durable(&self) -> bool {
        matches!(self, Self::Durable)
    }
}

/// Writes a complete file without exposing partial contents.
///
/// The temporary file lives beside the destination, is flushed before the
/// atomic replace, and the parent directory is flushed where the platform
/// exposes directory fsync.
pub fn write(path: &Path, bytes: &[u8]) -> anyhow::Result<WriteOutcome> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("atomic destination has no parent: {path:?}"))?;
    fs::create_dir_all(parent)?;

    let (temp, mut file) = create_temp(path, parent)?;
    let write_result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);

    if let Err(error) = write_result {
        return Err(with_cleanup_error(
            anyhow::anyhow!("failed to write atomic temp {temp:?}: {error}"),
            &temp,
        ));
    }

    if let Err(error) = atomic_replace(&temp, path) {
        return Err(with_cleanup_error(error, &temp));
    }

    Ok(commit_outcome(path, sync_parent(parent)))
}

fn commit_outcome(path: &Path, parent_sync: anyhow::Result<()>) -> WriteOutcome {
    match parent_sync {
        Ok(()) => WriteOutcome::Durable,
        Err(error) => WriteOutcome::CommittedNotDurable {
            warning: format!(
                "atomic replacement of {path:?} committed, but parent directory durability is unconfirmed: {error:#}"
            ),
        },
    }
}

fn create_temp(path: &Path, parent: &Path) -> anyhow::Result<(PathBuf, fs::File)> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("atomic destination has no file name: {path:?}"))?
        .to_string_lossy();
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{file_name}.{}.{}.{}.tmp",
            std::process::id(),
            epoch_nanos,
            nonce
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "failed to create atomic temp {temp:?}: {error}"
                ));
            }
        }
    }

    anyhow::bail!(
        "failed to allocate a unique atomic temp beside {path:?} after {TEMP_CREATE_ATTEMPTS} attempts"
    )
}

fn with_cleanup_error(error: anyhow::Error, temp: &Path) -> anyhow::Error {
    match fs::remove_file(temp) {
        Ok(()) => error,
        Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => error,
        Err(cleanup) => anyhow::anyhow!("{error:#}; failed to remove temp {temp:?}: {cleanup}"),
    }
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    fn wide_path(path: &Path) -> anyhow::Result<Vec<u16>> {
        let units = path.as_os_str().encode_wide().count();
        let capacity = units
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("wide path length overflow: {path:?}"))?;
        let mut wide = Vec::new();
        wide.try_reserve_exact(capacity).map_err(|error| {
            anyhow::anyhow!("wide path allocation failed for {path:?}: {error}")
        })?;
        wide.extend(path.as_os_str().encode_wide());
        wide.push(0);
        Ok(wide)
    }

    let source_w = wide_path(source)?;
    let destination_w = wide_path(destination)?;
    // SAFETY: both buffers are NUL-terminated and remain alive for the call.
    // Source and destination are distinct, owned paths on the same filesystem.
    unsafe {
        MoveFileExW(
            PCWSTR(source_w.as_ptr()),
            PCWSTR(destination_w.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| {
        anyhow::anyhow!("atomic replace {source:?} -> {destination:?} failed: {error}")
    })
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> anyhow::Result<()> {
    fs::rename(source, destination).map_err(|error| {
        anyhow::anyhow!("atomic replace {source:?} -> {destination:?} failed: {error}")
    })
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> anyhow::Result<()> {
    fs::File::open(parent)?
        .sync_all()
        .map_err(|error| anyhow::anyhow!("failed to sync parent directory {parent:?}: {error}"))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(label: &str) -> PathBuf {
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "squarebob-atomic-file-{label}-{}-{epoch_nanos}-{}",
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn replaces_complete_file() {
        let directory = test_directory("replace");
        fs::create_dir_all(&directory).expect("test directory");
        let path = directory.join("state.json");

        write(&path, b"old").expect("initial write");
        write(&path, b"new contents").expect("replacement write");

        assert_eq!(fs::read(&path).expect("read result"), b"new contents");
        fs::remove_dir_all(directory).expect("cleanup test directory");
    }

    #[test]
    fn post_commit_sync_failure_is_not_reported_as_uncommitted() {
        let path = Path::new("state.json");
        let outcome = commit_outcome(path, Err(anyhow::anyhow!("sync failed")));

        assert!(!outcome.is_durable());
        assert!(
            outcome
                .warning()
                .expect("durability warning")
                .contains("committed")
        );
    }

    #[test]
    fn replace_failure_preserves_destination_and_removes_temp() {
        let directory = test_directory("failure");
        let destination = directory.join("destination");
        fs::create_dir_all(&destination).expect("destination directory");

        assert!(write(&destination, b"cannot replace directory").is_err());
        assert!(destination.is_dir());
        let remaining: Vec<_> = fs::read_dir(&directory)
            .expect("read test directory")
            .map(|entry| entry.expect("directory entry").path())
            .collect();
        assert_eq!(remaining, vec![destination]);

        fs::remove_dir_all(directory).expect("cleanup test directory");
    }
}
