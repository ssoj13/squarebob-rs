//! Scan orchestration: starting, polling, and cancelling background scans.
//!
//! Extracted from `mod.rs` for review/merge sanity. No behaviour change.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use log::info;

use crate::cache;
use crate::exclusions;
use crate::scanner::{self, ScanMsg};
#[cfg(windows)]
use crate::scanner_ntfs;

use super::helpers::{compute_ext_stats, compute_size_range};
use super::state::ScanProgress;
use super::{App, ScannerMode};

impl App {
    // ── Scanning ──

    /// Effective backend name for UI (matches spawn choice in [`start_scan`])
    fn scan_engine_label_for_mode(mode: ScannerMode, _path: &std::path::Path) -> String {
        match mode {
            ScannerMode::Standard => "jwalk".to_string(),
            ScannerMode::Ntfs => {
                #[cfg(windows)]
                {
                    if scanner_ntfs::is_ntfs_available(path) {
                        "NTFS MFT".to_string()
                    } else {
                        "jwalk".to_string()
                    }
                }
                #[cfg(not(windows))]
                {
                    "jwalk".to_string()
                }
            }
        }
    }

    pub(super) fn start_scan(&mut self) {
        let path = PathBuf::from(&self.scan_path);
        if !path.exists() {
            self.progress.error = Some(format!("Path not found: {}", self.scan_path));
            return;
        }
        self.exclusions = exclusions::load(&self.scan_path);

        // History
        let p = self.scan_path.clone();
        self.path_history.retain(|x| x != &p);
        self.path_history.insert(0, p);
        self.path_history.truncate(20);

        // Try cache
        if let Some(cached) = cache::load_cache(&self.scan_path) {
            info!("Loaded cache for: {}", self.scan_path);
            self.cache_age = Some(cache::age_secs_from_cached(&cached));
            self.ext_stats = compute_ext_stats(&cached.tree);
            let (smin, smax) = compute_size_range(&cached.tree);
            self.scan_min_size = smin;
            self.scan_max_size = smax;
            self.filter_min = smin;
            self.filter_max = smax;
            self.expanded.insert(cached.tree.path.clone());
            self.tree = Some(cached.tree);
            self.filtered_tree = None;
            self.zoom_path = None;
            self.rebuild_display_tree();
            self.needs_layout = true;

            // Start screenshot timer after cache load
            if self.screenshot_delay.is_some() && self.screenshot_start_time.is_none() {
                self.screenshot_start_time = Some(std::time::Instant::now());
            }
        } else {
            self.tree = None;
            self.filtered_tree = None;
            self.cache_age = None;
        }

        let (tx, rx) = crossbeam_channel::unbounded();
        self.scan_rx = Some(rx);
        self.treemap_tex = None;
        self.hovered = None;
        self.selected_path = None;
        if self.tree.is_none() {
            self.expanded.clear();
        }
        self.ctx_menu_path = None;
        let scan_engine_label = Self::scan_engine_label_for_mode(self.scanner_mode, &path);
        self.progress = ScanProgress {
            scanning: true,
            start_time: Some(std::time::Instant::now()),
            scan_engine_label: Some(scan_engine_label),
            ..Default::default()
        };

        let cancel = match self.scanner_mode {
            ScannerMode::Ntfs => {
                #[cfg(windows)]
                {
                    if scanner_ntfs::is_ntfs_available(&path) {
                        scanner_ntfs::scan_ntfs_bg(path, tx)
                    } else {
                        scanner::scan_bg(path, tx)
                    }
                }
                #[cfg(not(windows))]
                {
                    scanner::scan_bg(path, tx)
                }
            }
            ScannerMode::Standard => scanner::scan_bg(path, tx),
        };
        self.scan_cancel = Some(cancel);
    }

    pub(super) fn stop_scan(&mut self) {
        if let Some(cancel) = &self.scan_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn poll_scan(&mut self) {
        let messages: Vec<ScanMsg> = if let Some(rx) = &self.scan_rx {
            rx.try_iter().collect()
        } else {
            return;
        };

        let mut needs_display_rebuild = false;

        for msg in messages {
            match msg {
                ScanMsg::Progress {
                    files,
                    dirs,
                    bytes,
                    errors,
                } => {
                    self.progress.files = files;
                    self.progress.dirs = dirs;
                    self.progress.bytes = bytes;
                    self.progress.errors = errors;
                }
                ScanMsg::Done(tree) => {
                    info!("Scan complete: {} files", tree.file_count);
                    self.progress.scanning = false;
                    self.progress.scan_engine_label = None;
                    self.progress.error = None;
                    if let Some(t) = self.progress.start_time {
                        self.progress.elapsed_secs = t.elapsed().as_secs_f32();
                    }
                    self.ext_stats = compute_ext_stats(&tree);
                    let (smin, smax) = compute_size_range(&tree);
                    self.scan_min_size = smin;
                    self.scan_max_size = smax;
                    self.filter_min = smin;
                    self.filter_max = smax;
                    self.expanded.insert(tree.path.clone());

                    // Serialize cache on main thread (avoids cloning the tree)
                    let scan_path = self.scan_path.clone();
                    match cache::serialize_cache(&scan_path, &tree) {
                        Ok(cache_bytes) => {
                            std::thread::spawn(move || {
                                if let Err(e) = cache::write_cache_bytes(&scan_path, &cache_bytes) {
                                    log::warn!("Failed to write cache: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            log::warn!("Failed to serialize cache: {}", e);
                        }
                    }

                    self.tree = Some(tree);
                    self.filtered_tree = None;
                    self.zoom_path = None;
                    self.cache_age = None;
                    self.filtered_paths_cache = None;
                    needs_display_rebuild = true;

                    // Start screenshot timer after scan completes
                    if self.screenshot_delay.is_some() && self.screenshot_start_time.is_none() {
                        self.screenshot_start_time = Some(std::time::Instant::now());
                    }
                    self.needs_layout = true;
                }
                #[cfg(windows)]
                ScanMsg::NtfsFallback(msg) => {
                    // Do NOT mutate self.scanner_mode here — it persists into PersistState
                    // and would silently strip the user's NTFS preference on the next save.
                    // The fallback is per-scan recovery; user opt-in is preserved.
                    self.progress.scan_engine_label = Some("jwalk (NTFS fallback)".to_string());
                    self.progress.error =
                        Some(format!("NTFS failed ({}), using standard scanner", msg));
                }
                ScanMsg::Error(e) => {
                    self.progress.scanning = false;
                    self.progress.scan_engine_label = None;
                    if let Some(t) = self.progress.start_time {
                        self.progress.elapsed_secs = t.elapsed().as_secs_f32();
                    }
                    self.progress.error = Some(e);
                }
            }
        }

        if needs_display_rebuild {
            self.rebuild_display_tree();
        }
    }
}
