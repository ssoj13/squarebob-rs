//! Background filesystem scanning with typed terminal outcomes and owned workers.

use crossbeam_channel::{Receiver, Sender, TrySendError};
use log::{debug, info, trace, warn};
use squarebob_core::DirEntry;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use crate::cache::{self, CacheQuality};
use crate::path_key::ScanRoot;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanDiagnostics {
    pub walk_errors: u64,
    pub metadata_errors: u64,
    pub malformed_records: u64,
    pub depth_errors: u64,
}

impl ScanDiagnostics {
    pub fn total_errors(&self) -> u64 {
        self.walk_errors
            .saturating_add(self.metadata_errors)
            .saturating_add(self.malformed_records)
            .saturating_add(self.depth_errors)
    }

    pub fn is_complete(&self) -> bool {
        self.total_errors() == 0
    }
}

#[derive(Debug)]
pub struct PreparedScan {
    pub tree: DirEntry,
    pub diagnostics: ScanDiagnostics,
    pub ext_stats: Vec<(String, u64, u64)>,
    pub size_range: (u64, u64),
    pub cache_bytes: Option<Vec<u8>>,
    pub cache_error: Option<String>,
}

#[derive(Debug)]
pub enum ScanOutcome {
    Completed(PreparedScan),
    Partial(PreparedScan),
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Walking,
    IndexingVolume,
    SelectingTree,
    MeasuringTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanProgressUpdate {
    pub phase: ScanPhase,
    pub items: u64,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
}

#[derive(Debug)]
pub enum ScanMsg {
    Progress(ScanProgressUpdate),
    Terminal(ScanOutcome),
    #[cfg(windows)]
    NtfsFallback(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanBackend {
    Standard,
    Ntfs,
}

/// Complete ownership of one worker generation.
///
/// Dropping a session first cancels, then joins. Active sessions are moved to
/// the app's retired list on replacement, so UI never blocks while they wind
/// down; only already-finished sessions are reaped during normal frames.
pub struct ScanSession {
    pub generation: u64,
    pub root: ScanRoot,
    pub receiver: Receiver<ScanMsg>,
    pub terminal_receiver: Receiver<ScanMsg>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ScanSession {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn is_finished(&self) -> bool {
        self.worker
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }

    pub fn reap_finished(&mut self) -> Result<bool, String> {
        if !self.is_finished() {
            return Ok(false);
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|payload| format!("scanner worker panicked: {payload:?}"))?;
        }
        Ok(true)
    }
}

impl Drop for ScanSession {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take()
            && let Err(payload) = worker.join()
        {
            warn!("scanner worker panicked during shutdown: {payload:?}");
        }
    }
}

pub fn spawn(generation: u64, root: ScanRoot, backend: ScanBackend) -> anyhow::Result<ScanSession> {
    let (tx, receiver) = crossbeam_channel::bounded(16);
    let (terminal_tx, terminal_receiver) = crossbeam_channel::bounded(1);
    let session_root = root.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let thread_name = match backend {
        ScanBackend::Standard => "scanner",
        ScanBackend::Ntfs => "ntfs-scanner",
    };

    let worker = std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || match backend {
            ScanBackend::Standard => run_standard(root, tx, terminal_tx, worker_cancel),
            ScanBackend::Ntfs => {
                #[cfg(windows)]
                crate::scanner_ntfs::run_ntfs(root, tx, terminal_tx, worker_cancel);
                #[cfg(not(windows))]
                run_standard(root, tx, terminal_tx, worker_cancel);
            }
        })
        .map_err(|e| anyhow::anyhow!("failed to spawn {thread_name}: {e}"))?;

    Ok(ScanSession {
        generation,
        root: session_root,
        receiver,
        terminal_receiver,
        cancel,
        worker: Some(worker),
    })
}

fn run_standard(
    root: ScanRoot,
    tx: Sender<ScanMsg>,
    terminal_tx: Sender<ScanMsg>,
    cancel: Arc<AtomicBool>,
) {
    info!("Scan started: {:?}", root.path());
    let outcome = match scan_dir(root.path(), &tx, &cancel) {
        Ok(build) => finish_build(&root, build),
        Err(ScanFailure::Cancelled) => ScanOutcome::Cancelled,
        Err(ScanFailure::BackendUnavailable(error) | ScanFailure::Failed(error)) => {
            ScanOutcome::Failed(format!("{error:#}"))
        }
    };
    let _ = terminal_tx.send(ScanMsg::Terminal(outcome));
}

pub(crate) struct ScanBuild {
    pub tree: DirEntry,
    pub diagnostics: ScanDiagnostics,
}

pub(crate) enum ScanFailure {
    Cancelled,
    BackendUnavailable(anyhow::Error),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for ScanFailure {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

pub(crate) fn finish_build(root: &ScanRoot, mut build: ScanBuild) -> ScanOutcome {
    build.tree.sort_by_size();
    let ext_stats = crate::app::helpers::compute_ext_stats(&build.tree);
    let size_range = crate::app::helpers::compute_size_range(&build.tree);
    let complete = build.diagnostics.is_complete();
    let (cache_bytes, cache_error) = if complete {
        match cache::serialize_cache_ref(
            root,
            &build.tree,
            CacheQuality {
                complete: true,
                errors: 0,
            },
        ) {
            Ok(bytes) => (Some(bytes), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        }
    } else {
        (None, None)
    };
    let prepared = PreparedScan {
        tree: build.tree,
        diagnostics: build.diagnostics,
        ext_stats,
        size_range,
        cache_bytes,
        cache_error,
    };
    if complete {
        ScanOutcome::Completed(prepared)
    } else {
        ScanOutcome::Partial(prepared)
    }
}

#[cfg(windows)]
pub(crate) fn scan_dir_public(
    root: &Path,
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
) -> Result<ScanBuild, ScanFailure> {
    scan_dir(root, tx, cancel)
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Acquire)
}

pub(crate) fn send_progress(
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
    files: u64,
    dirs: u64,
    bytes: u64,
    errors: u64,
) -> Result<(), ScanFailure> {
    send_progress_update(
        tx,
        cancel,
        ScanProgressUpdate {
            phase: ScanPhase::Walking,
            items: files.saturating_add(dirs),
            files,
            dirs,
            bytes,
            errors,
        },
    )
}

pub(crate) fn send_progress_update(
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
    update: ScanProgressUpdate,
) -> Result<(), ScanFailure> {
    if cancelled(cancel) {
        return Err(ScanFailure::Cancelled);
    }
    match tx.try_send(ScanMsg::Progress(update)) {
        Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
        Err(TrySendError::Disconnected(_)) => Err(ScanFailure::Cancelled),
    }
}

fn scan_dir(
    root: &Path,
    tx: &Sender<ScanMsg>,
    cancel: &AtomicBool,
) -> Result<ScanBuild, ScanFailure> {
    use std::collections::HashMap;

    let root_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    let root_entry = DirEntry::new_dir(root_name, root.to_path_buf());
    let walker = jwalk::WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(jwalk::Parallelism::RayonNewPool(num_cpus::get()));

    let mut dirs: HashMap<PathBuf, Vec<DirEntry>> = HashMap::new();
    let mut all_dirs: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut file_count = 0u64;
    let mut dir_count = 0u64;
    let mut total_bytes = 0u64;
    let mut progress_counter = 0u64;
    let mut diagnostics = ScanDiagnostics::default();

    for entry in walker {
        if cancelled(cancel) {
            info!("Scan cancelled by user");
            return Err(ScanFailure::Cancelled);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if error.path() == Some(root) {
                    return Err(ScanFailure::Failed(anyhow::anyhow!(
                        "cannot enumerate scan root {:?}: {error}",
                        root
                    )));
                }
                debug!("Walk error: {error}");
                diagnostics.walk_errors = diagnostics.walk_errors.saturating_add(1);
                continue;
            }
        };

        let path = entry.path();
        if path == root {
            continue;
        }
        let Some(parent) = path.parent().map(Path::to_path_buf) else {
            diagnostics.walk_errors = diagnostics.walk_errors.saturating_add(1);
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();

        if entry.file_type().is_dir() {
            dir_count = dir_count
                .checked_add(1)
                .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("directory count overflow")))?;
            all_dirs.push(path.clone());
            dirs.entry(parent)
                .or_default()
                .push(DirEntry::new_dir(name, path));
        } else {
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    diagnostics.metadata_errors = diagnostics.metadata_errors.saturating_add(1);
                    debug!("Metadata error for {:?}: {error}", path);
                    continue;
                }
            };
            let size = metadata.len();
            let modified_time = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            file_count = file_count
                .checked_add(1)
                .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("file count overflow")))?;
            total_bytes = total_bytes
                .checked_add(size)
                .ok_or_else(|| ScanFailure::Failed(anyhow::anyhow!("byte count overflow")))?;
            dirs.entry(parent).or_default().push(DirEntry::new_file(
                name,
                path,
                size,
                ext,
                modified_time,
            ));
        }

        progress_counter = progress_counter.saturating_add(1);
        if progress_counter.is_multiple_of(5000) {
            trace!(
                "Progress: {file_count} files, {dir_count} dirs, {} errors",
                diagnostics.total_errors()
            );
            send_progress(
                tx,
                cancel,
                file_count,
                dir_count,
                total_bytes,
                diagnostics.total_errors(),
            )?;
        }
    }

    send_progress(
        tx,
        cancel,
        file_count,
        dir_count,
        total_bytes,
        diagnostics.total_errors(),
    )?;

    all_dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    let mut assembled: HashMap<PathBuf, DirEntry> = HashMap::new();
    let mut result = DirEntry::new_dir(root_entry.name.clone(), root_entry.path.clone());

    for dir_path in &all_dirs {
        if cancelled(cancel) {
            return Err(ScanFailure::Cancelled);
        }
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

        for child in children {
            if child.is_dir {
                if let Some(assembled_child) = assembled.remove(&child.path) {
                    dir_entry.size = dir_entry
                        .size
                        .checked_add(assembled_child.size)
                        .ok_or_else(|| {
                            ScanFailure::Failed(anyhow::anyhow!(
                                "directory size overflow at {:?}",
                                dir_entry.path
                            ))
                        })?;
                    dir_entry.file_count = dir_entry
                        .file_count
                        .checked_add(assembled_child.file_count)
                        .ok_or_else(|| {
                            ScanFailure::Failed(anyhow::anyhow!(
                                "file count overflow at {:?}",
                                dir_entry.path
                            ))
                        })?;
                    dir_entry.dir_count = dir_entry
                        .dir_count
                        .checked_add(assembled_child.dir_count)
                        .and_then(|n| n.checked_add(1))
                        .ok_or_else(|| {
                            ScanFailure::Failed(anyhow::anyhow!(
                                "directory count overflow at {:?}",
                                dir_entry.path
                            ))
                        })?;
                    dir_entry.children.push(assembled_child);
                } else {
                    dir_entry.dir_count = dir_entry.dir_count.checked_add(1).ok_or_else(|| {
                        ScanFailure::Failed(anyhow::anyhow!(
                            "directory count overflow at {:?}",
                            dir_entry.path
                        ))
                    })?;
                    dir_entry.children.push(child);
                }
            } else {
                dir_entry.size = dir_entry.size.checked_add(child.size).ok_or_else(|| {
                    ScanFailure::Failed(anyhow::anyhow!(
                        "directory size overflow at {:?}",
                        dir_entry.path
                    ))
                })?;
                dir_entry.file_count = dir_entry
                    .file_count
                    .checked_add(child.file_count)
                    .ok_or_else(|| {
                        ScanFailure::Failed(anyhow::anyhow!(
                            "file count overflow at {:?}",
                            dir_entry.path
                        ))
                    })?;
                dir_entry.children.push(child);
            }
        }

        if *dir_path == root {
            result = dir_entry;
        } else {
            assembled.insert(dir_path.clone(), dir_entry);
        }
    }

    Ok(ScanBuild {
        tree: result,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_control_completeness() {
        let mut diagnostics = ScanDiagnostics::default();
        assert!(diagnostics.is_complete());
        diagnostics.metadata_errors = 1;
        assert!(!diagnostics.is_complete());
        assert_eq!(diagnostics.total_errors(), 1);
    }

    #[test]
    fn disconnected_progress_cancels_worker() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        drop(rx);
        let cancel = AtomicBool::new(false);
        assert!(matches!(
            send_progress(&tx, &cancel, 0, 0, 0, 0),
            Err(ScanFailure::Cancelled)
        ));
    }

    #[test]
    fn full_progress_queue_cannot_block_terminal_or_session_drop() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let (progress_tx, receiver) = crossbeam_channel::bounded(1);
        progress_tx
            .send(ScanMsg::Progress(ScanProgressUpdate {
                phase: ScanPhase::Walking,
                items: 1,
                files: 1,
                dirs: 0,
                bytes: 1,
                errors: 0,
            }))
            .expect("fill progress queue");
        let (terminal_tx, terminal_receiver) = crossbeam_channel::bounded(1);
        let worker = std::thread::spawn(move || {
            terminal_tx
                .send(ScanMsg::Terminal(ScanOutcome::Cancelled))
                .expect("terminal channel remains independent");
        });
        let session = ScanSession {
            generation: 1,
            root,
            receiver,
            terminal_receiver,
            cancel: Arc::new(AtomicBool::new(false)),
            worker: Some(worker),
        };
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            drop(session);
            let _ = done_tx.send(());
        });

        done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("session drop must not wait for progress drain");
    }
}
