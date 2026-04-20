//! Stable identifiers derived from scan paths (on-disk cache filenames, exclusion JSON keys).

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of the scan path bytes (same string as used elsewhere for that root).
pub fn scan_path_id_hex(path: &str) -> String {
    let mut h = Sha256::new();
    h.update(path.as_bytes());
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
