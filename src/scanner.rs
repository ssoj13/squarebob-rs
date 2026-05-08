use crossbeam_channel::Sender;
use dirstat_core::DirEntry;
use log::{debug, info, trace};
/// Multithreaded filesystem scanner using jwalk.
/// Builds a tree of DirEntry nodes with recursive size aggregation.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Progress message from scanner to UI
#[derive(Debug)]
pub enum ScanMsg {
    /// Periodic progress: files scanned so far
    Progress {
        files: u64,
        dirs: u64,
        bytes: u64,
        errors: u64,
    },
    /// Scan complete, here's the tree (owned, no Arc - Cell<rect> is !Sync)
    Done(DirEntry),
    /// Error during scan
    Error(String),
    /// NTFS fast path failed; standard scanner continues (Windows only).
    #[cfg(windows)]
    NtfsFallback(String),
}

/// Launch a background scan of `root` path.
/// Sends progress updates and final tree via `tx`.
/// Returns a cancel flag that can be set to true to abort the scan.
pub fn scan_bg(root: PathBuf, tx: Sender<ScanMsg>) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    std::thread::Builder::new()
        .name("scanner".into())
        .spawn(move || {
            info!("Scan started: {:?}", root);
            match scan_dir(&root, &tx, &cancel_clone) {
                Ok(mut tree) => {
                    tree.sort_by_size();
                    info!(
                        "Scan done: {} files, {} dirs, {} bytes",
                        tree.file_count, tree.dir_count, tree.size
                    );
                    let _ = tx.send(ScanMsg::Done(tree));
                }
                Err(e) => {
                    let _ = tx.send(ScanMsg::Error(format!("{e:#}")));
                }
            }
        })
        .expect("failed to spawn scanner thread");
    cancel
}

/// Visible to the NTFS MFT module when it falls back to a standard walk (`scanner_ntfs`).
#[cfg(windows)]
pub fn scan_dir_public(
    root: &Path,
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
) -> anyhow::Result<DirEntry> {
    scan_dir(root, tx, cancel)
}

fn scan_dir(root: &Path, tx: &Sender<ScanMsg>, cancel: &AtomicBool) -> anyhow::Result<DirEntry> {
    use std::collections::HashMap;

    let root_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    let root_entry = DirEntry::new_dir(root_name, root.to_path_buf());

    // Collect all entries via jwalk (parallel walk)
    let walker = jwalk::WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(jwalk::Parallelism::RayonNewPool(num_cpus::get()));

    // Map from parent dir path -> list of entries
    let mut dirs: HashMap<PathBuf, Vec<DirEntry>> = HashMap::new();
    let mut all_dirs: Vec<PathBuf> = vec![root.to_path_buf()];

    let mut file_count: u64 = 0;
    let mut dir_count: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut progress_counter: u64 = 0;
    let mut error_count: u64 = 0;

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                debug!("Walk error: {e}");
                error_count += 1;
                continue;
            }
        };

        let path = entry.path();

        // Skip the root itself
        if path == root {
            continue;
        }

        let parent = match path.parent() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };

        let name = entry.file_name().to_string_lossy().to_string();

        if entry.file_type().is_dir() {
            dir_count += 1;
            all_dirs.push(path.clone());
            dirs.entry(parent)
                .or_default()
                .push(DirEntry::new_dir(name, path));
        } else {
            let meta = entry.metadata();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified_time = meta
                .as_ref()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            file_count += 1;
            total_bytes += size;
            dirs.entry(parent).or_default().push(DirEntry::new_file(
                name,
                path,
                size,
                ext,
                modified_time,
            ));
        }

        // Check cancellation
        progress_counter += 1;
        if cancel.load(Ordering::Relaxed) {
            info!("Scan cancelled by user");
            return Err(anyhow::anyhow!("Scan cancelled"));
        }

        // Send progress every 5000 entries
        if progress_counter.is_multiple_of(5000) {
            trace!("Progress: {file_count} files, {dir_count} dirs, {error_count} errors");
            let _ = tx.send(ScanMsg::Progress {
                files: file_count,
                dirs: dir_count,
                bytes: total_bytes,
                errors: error_count,
            });
        }
    }

    // Send final progress
    let _ = tx.send(ScanMsg::Progress {
        files: file_count,
        dirs: dir_count,
        bytes: total_bytes,
        errors: error_count,
    });

    // Assemble tree bottom-up: process dirs from deepest to shallowest
    all_dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

    // Map path -> assembled DirEntry
    let mut assembled: HashMap<PathBuf, DirEntry> = HashMap::new();

    // Header-only copy of root for assembly
    let mut result = DirEntry::new_dir(root_entry.name.clone(), root_entry.path.clone());

    for dir_path in &all_dirs {
        let children = dirs.remove(dir_path).unwrap_or_default();
        let dir_name = dir_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| dir_path.to_string_lossy().to_string());

        let mut dir_entry = if *dir_path == root {
            DirEntry::new_dir(result.name.clone(), result.path.clone())
        } else {
            DirEntry::new_dir(dir_name, dir_path.clone())
        };

        // Add file children directly, collect dir children from assembled
        for child in children {
            if child.is_dir {
                if let Some(assembled_child) = assembled.remove(&child.path) {
                    dir_entry.size += assembled_child.size;
                    dir_entry.file_count += assembled_child.file_count;
                    dir_entry.dir_count += assembled_child.dir_count + 1; // +1 for the directory itself
                    dir_entry.children.push(assembled_child);
                } else {
                    // Empty directory - still count it
                    dir_entry.dir_count += 1;
                    dir_entry.children.push(child);
                }
            } else {
                dir_entry.size += child.size;
                dir_entry.file_count += child.file_count;
                dir_entry.children.push(child);
            }
        }

        if *dir_path == root {
            result = dir_entry;
        } else {
            assembled.insert(dir_path.clone(), dir_entry);
        }
    }

    // Use aggregated counts from tree assembly (matches actual tree content)
    // raw file_count/dir_count from walker may differ if some entries couldn't be assembled
    Ok(result)
}
