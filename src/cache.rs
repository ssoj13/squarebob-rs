/// Cache module: save/load scan results to disk for instant startup.
use std::fs;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::SystemTime;

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};

use crate::path_key;
use squarebob_core::DirEntry;

/// Cached scan result with metadata
#[derive(Serialize, Deserialize)]
pub struct CachedScan {
    /// Version for cache format compatibility
    pub version: u32,
    /// Original scan path
    pub scan_path: String,
    /// Timestamp when scan was performed (seconds since UNIX epoch)
    pub timestamp: u64,
    /// The directory tree
    pub tree: DirEntry,
}

const CACHE_VERSION: u32 = 2;

/// Get the cache directory path
fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "squarebob-rs").map(|dirs| dirs.cache_dir().to_path_buf())
}

fn cache_filename(scan_path: &str) -> String {
    format!("{}.bin", path_key::scan_path_id_hex(scan_path))
}

/// Get the full cache file path for a scan path
pub fn cache_path(scan_path: &str) -> Option<PathBuf> {
    cache_dir().map(|dir| dir.join(cache_filename(scan_path)))
}

/// Serialize a tree to cache bytes (for async saving)
pub fn serialize_cache(scan_path: &str, tree: &DirEntry) -> anyhow::Result<Vec<u8>> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Serialize directly from the tree reference (no clone needed)
    let cached = CachedScanRef {
        version: CACHE_VERSION,
        scan_path,
        timestamp,
        tree,
    };

    Ok(bincode::serialize(&cached)?)
}

/// Internal struct for serialization without cloning
#[derive(Serialize)]
struct CachedScanRef<'a> {
    version: u32,
    scan_path: &'a str,
    timestamp: u64,
    tree: &'a DirEntry,
}

/// Wall-clock age of a loaded cache entry (seconds since `CachedScan::timestamp`).
pub fn age_secs_from_cached(cached: &CachedScan) -> u64 {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(cached.timestamp)
}

/// Write pre-serialized cache bytes to disk
pub fn write_cache_bytes(scan_path: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let Some(cache_file) = cache_path(scan_path) else {
        anyhow::bail!("Could not determine cache directory");
    };

    // Ensure cache directory exists
    if let Some(parent) = cache_file.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&cache_file, bytes)?;
    info!("Cache saved: {:?} ({} bytes)", cache_file, bytes.len());
    Ok(())
}

/// Load a cached scan result
pub fn load_cache(scan_path: &str) -> Option<CachedScan> {
    let cache_file = cache_path(scan_path)?;

    if !cache_file.exists() {
        debug!("No cache found for: {}", scan_path);
        return None;
    }

    let file = match fs::File::open(&cache_file) {
        Ok(f) => f,
        Err(e) => {
            warn!("Failed to open cache file: {}", e);
            return None;
        }
    };

    let reader = BufReader::new(file);
    match bincode::deserialize_from::<_, CachedScan>(reader) {
        Ok(cached) => {
            if cached.version != CACHE_VERSION {
                warn!("Cache version mismatch, ignoring");
                return None;
            }
            let age_secs = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs().saturating_sub(cached.timestamp))
                .unwrap_or(0);
            info!(
                "Cache loaded: {:?} ({} files, {} seconds old)",
                cache_file, cached.tree.file_count, age_secs
            );
            Some(cached)
        }
        Err(e) => {
            warn!("Failed to deserialize cache: {}", e);
            // Remove corrupted cache file
            let _ = fs::remove_file(&cache_file);
            None
        }
    }
}

/// Delete on-disk cache for a scan path (e.g. user clears cache in settings).
pub fn delete_cache(scan_path: &str) -> anyhow::Result<()> {
    if let Some(cache_file) = cache_path(scan_path) {
        if cache_file.exists() {
            fs::remove_file(&cache_file)?;
            info!("Cache deleted: {:?}", cache_file);
        }
    }
    Ok(())
}

/// Format cache age as human-readable string
pub fn format_age(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s ago", seconds)
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}
