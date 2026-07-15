//! Canonical scan-root identity shared by scanners, cache, exclusions, and UI sessions.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// A validated directory root with one operational path and one stable local identity.
///
/// `display` preserves the user's spelling for UI/history. `path` is canonical and
/// is the only path scanners operate on. `id` is derived from the native OS string,
/// so non-UTF paths are never collapsed through a lossy conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRoot {
    display: String,
    path: PathBuf,
    id: String,
}

impl ScanRoot {
    pub fn from_input(input: &str) -> anyhow::Result<Self> {
        let requested = PathBuf::from(input);
        let absolute = if requested.is_absolute() {
            requested
        } else {
            std::env::current_dir()
                .map_err(|e| anyhow::anyhow!("cannot resolve current directory: {e}"))?
                .join(requested)
        };
        let path = std::fs::canonicalize(&absolute)
            .map_err(|e| anyhow::anyhow!("cannot resolve scan root {:?}: {e}", absolute))?;
        let metadata = std::fs::metadata(&path)
            .map_err(|e| anyhow::anyhow!("cannot inspect scan root {:?}: {e}", path))?;
        if !metadata.is_dir() {
            anyhow::bail!("scan root is not a directory: {:?}", path);
        }

        Ok(Self {
            display: input.to_owned(),
            id: native_path_id_hex(&path),
            path,
        })
    }

    pub fn from_canonical_path(path: PathBuf) -> anyhow::Result<Self> {
        let display = path.to_string_lossy().into_owned();
        Self::from_input(&display)
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

fn finish_hash(h: Sha256) -> String {
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn native_path_id_hex(path: &Path) -> String {
    let mut h = Sha256::new();

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        h.update(b"windows-utf16-v1\0");
        for unit in path.as_os_str().encode_wide() {
            h.update(unit.to_le_bytes());
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        h.update(b"unix-bytes-v1\0");
        h.update(path.as_os_str().as_bytes());
    }

    #[cfg(not(any(windows, unix)))]
    {
        h.update(b"portable-debug-v1\0");
        h.update(format!("{:?}", path.as_os_str()).as_bytes());
    }

    finish_hash(h)
}

/// Legacy raw UTF-8 hash used only to locate and migrate pre-v3 cache/exclusion files.
pub(crate) fn legacy_scan_path_id_hex(path: &str) -> String {
    let mut h = Sha256::new();
    h.update(path.as_bytes());
    finish_hash(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_relative_roots_share_identity() {
        let direct = ScanRoot::from_input(".").expect("current directory must resolve");
        let nested = ScanRoot::from_input("././").expect("equivalent path must resolve");
        assert!(direct.same_identity(&nested));
        assert_eq!(direct.path(), nested.path());
    }

    #[test]
    fn identity_uses_canonical_path_not_display_spelling() {
        let a = ScanRoot::from_input(".").expect("current directory must resolve");
        let b = ScanRoot::from_canonical_path(a.path().to_path_buf())
            .expect("canonical path must resolve");
        assert_eq!(a.id(), b.id());
        assert_ne!(a.display(), b.display());
    }
}
