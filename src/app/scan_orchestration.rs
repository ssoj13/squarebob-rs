//! Scan orchestration: generation ownership, typed outcomes, and ordered cache I/O.

use crossbeam_channel::TryRecvError;
use log::{info, warn};

use crate::cache::{CacheEvent, CacheService, PreparedCache};
use crate::exclusions::{self, Exclusions};
use crate::path_key::ScanRoot;
use crate::scanner::{self, PreparedScan, ScanBackend, ScanMsg, ScanOutcome};
#[cfg(windows)]
use crate::scanner_ntfs;

use super::state::{ScanPresentation, ScanProgress};
use super::{App, ScannerMode};

impl App {
    fn backend_for_mode(mode: ScannerMode, root: &ScanRoot) -> (ScanBackend, String) {
        match mode {
            ScannerMode::Standard => (ScanBackend::Standard, "jwalk".to_owned()),
            ScannerMode::Ntfs => {
                #[cfg(windows)]
                {
                    if scanner_ntfs::is_ntfs_available(root.path()) {
                        (ScanBackend::Ntfs, "NTFS MFT".to_owned())
                    } else {
                        (ScanBackend::Standard, "jwalk (NTFS unavailable)".to_owned())
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = root;
                    (ScanBackend::Standard, "jwalk (NTFS unavailable)".to_owned())
                }
            }
        }
    }

    fn advance_scan_generation(&mut self) -> Result<u64, String> {
        let next = self
            .scan_generation
            .checked_add(1)
            .ok_or_else(|| "scan generation counter exhausted".to_owned())?;
        self.scan_generation = next;
        Ok(next)
    }

    fn retire_active_scan(&mut self) {
        if let Some(session) = self.active_scan.take() {
            session.cancel();
            self.retired_scans.push(session);
        }
    }

    fn reap_retired_scans(&mut self) {
        let mut index = self.retired_scans.len();
        while index > 0 {
            index -= 1;
            if !self.retired_scans[index].is_finished() {
                continue;
            }
            let mut session = self.retired_scans.swap_remove(index);
            if let Err(error) = session.reap_finished() {
                warn!(
                    "retired scan generation {} failed: {}",
                    session.generation, error
                );
            }
        }
    }

    fn ensure_cache_service(&mut self) -> Result<&CacheService, String> {
        if self.cache_service.is_none() {
            self.cache_service = Some(CacheService::spawn().map_err(|error| format!("{error:#}"))?);
        }
        self.cache_service
            .as_ref()
            .ok_or_else(|| "cache service initialization failed".to_owned())
    }

    fn push_scan_warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        match &mut self.progress.warning {
            Some(existing) if !existing.is_empty() => {
                existing.push_str("; ");
                existing.push_str(&warning);
            }
            slot => *slot = Some(warning),
        }
    }

    fn clear_scan_presentation(&mut self) {
        self.scan_presentation = ScanPresentation::Empty;
        self.tree = None;
        self.filtered_tree = None;
        self.display_tree_cache = None;
        self.ext_stats.clear();
        self.scan_min_size = 0;
        self.scan_max_size = 0;
        self.filter_min = 0;
        self.filter_max = u64::MAX;
        self.expanded.clear();
        self.lod_expanded_paths.clear();
        self.zoom_path = None;
        self.filtered_paths_cache = None;
        self.treemap_tex = None;
        self.hovered = None;
        self.selected_path = None;
        self.selected_3d_ids.clear();
        self.sticky_hover = None;
        self.ctx_menu_path = None;
        self.cache_age = None;
        self.last_render_size = (0, 0);
        self.needs_layout = true;
        self.needs_render_3d = true;
        self.screenshot_start_time = None;
        self.screenshot_taken = false;
    }

    fn update_path_history(&mut self, root: &ScanRoot) {
        let root_id = root.id().to_owned();
        self.path_history.retain(|candidate| {
            ScanRoot::from_input(candidate)
                .map(|candidate_root| candidate_root.id() != root_id)
                .unwrap_or(true)
        });
        self.path_history.insert(0, root.display().to_owned());
        self.path_history.truncate(20);
    }

    fn install_tree(
        &mut self,
        generation: u64,
        tree: squarebob_core::DirEntry,
        ext_stats: Vec<(String, u64, u64)>,
        size_range: (u64, u64),
        presentation: ScanPresentation,
        cache_age: Option<u64>,
    ) {
        let root_path = tree.path.clone();
        self.progress.files = tree.file_count;
        self.progress.dirs = tree.dir_count;
        self.progress.bytes = tree.size;
        self.ext_stats = ext_stats;
        self.scan_min_size = size_range.0;
        self.scan_max_size = size_range.1;
        self.filter_min = size_range.0;
        self.filter_max = size_range.1;
        self.expanded.clear();
        self.expanded.insert(root_path);
        self.tree = Some(tree);
        self.filtered_tree = None;
        self.display_tree_cache = None;
        self.zoom_path = None;
        self.filtered_paths_cache = None;
        self.cache_age = cache_age;
        self.scan_presentation = presentation;
        self.rebuild_display_tree();
        self.needs_layout = true;
        self.needs_render_3d = true;

        debug_assert!(matches!(
            presentation,
            ScanPresentation::CachePreview { generation: g }
                | ScanPresentation::LivePartial { generation: g }
                | ScanPresentation::LiveComplete { generation: g }
                if g == generation
        ));
    }

    fn install_cache_preview(&mut self, generation: u64, prepared: PreparedCache) {
        if matches!(
            self.scan_presentation,
            ScanPresentation::LivePartial { generation: g }
                | ScanPresentation::LiveComplete { generation: g }
                if g == generation
        ) {
            return;
        }

        let age = crate::cache::age_secs_from_cached(&prepared.cached);
        info!(
            "Loaded cache preview for generation {}: {} files",
            generation, prepared.cached.tree.file_count
        );
        self.install_tree(
            generation,
            prepared.cached.tree,
            prepared.ext_stats,
            prepared.size_range,
            ScanPresentation::CachePreview { generation },
            Some(age),
        );
    }

    fn queue_cache_store(&mut self, generation: u64, root: ScanRoot, bytes: Vec<u8>) {
        let result = self.ensure_cache_service().and_then(|service| {
            service
                .store(generation, root, bytes)
                .map_err(|e| format!("{e:#}"))
        });
        if let Err(error) = result {
            self.push_scan_warning(format!("cache store was not queued: {error}"));
        }
    }

    fn install_live_scan(
        &mut self,
        generation: u64,
        root: ScanRoot,
        prepared: PreparedScan,
        complete: bool,
    ) {
        let PreparedScan {
            tree,
            diagnostics,
            ext_stats,
            size_range,
            cache_bytes,
            cache_error,
        } = prepared;

        let presentation = if complete {
            ScanPresentation::LiveComplete { generation }
        } else {
            ScanPresentation::LivePartial { generation }
        };
        self.install_tree(generation, tree, ext_stats, size_range, presentation, None);

        self.progress.scanning = false;
        self.progress.scan_engine_label = None;
        self.progress.error = None;
        self.progress.errors = diagnostics.total_errors();
        if let Some(start) = self.progress.start_time {
            self.progress.elapsed_secs = start.elapsed().as_secs_f32();
        }

        if complete {
            info!(
                "Scan generation {} complete: {} files",
                generation, self.progress.files
            );
            if let Some(bytes) = cache_bytes {
                self.queue_cache_store(generation, root, bytes);
            }
            if let Some(error) = cache_error {
                self.push_scan_warning(format!("cache serialization failed: {error}"));
            }
            if self.screenshot_delay.is_some() {
                self.screenshot_start_time = Some(std::time::Instant::now());
            }
        } else {
            self.push_scan_warning(format!(
                "partial scan: {} filesystem entries could not be read; result was not cached",
                diagnostics.total_errors()
            ));
        }
    }

    fn finish_active_session(&mut self) {
        let Some(mut session) = self.active_scan.take() else {
            return;
        };
        if session.is_finished() {
            if let Err(error) = session.reap_finished() {
                self.progress.error = Some(error);
            }
        } else {
            self.retired_scans.push(session);
        }
    }

    fn handle_terminal(&mut self, generation: u64, root: ScanRoot, outcome: ScanOutcome) {
        match outcome {
            ScanOutcome::Completed(prepared) => {
                self.install_live_scan(generation, root, prepared, true);
            }
            ScanOutcome::Partial(prepared) => {
                self.install_live_scan(generation, root, prepared, false);
            }
            ScanOutcome::Cancelled => {
                self.progress.scanning = false;
                self.progress.scan_engine_label = None;
                if let Some(start) = self.progress.start_time {
                    self.progress.elapsed_secs = start.elapsed().as_secs_f32();
                }
                self.push_scan_warning("scan cancelled");
            }
            ScanOutcome::Failed(error) => {
                self.progress.scanning = false;
                self.progress.scan_engine_label = None;
                if let Some(start) = self.progress.start_time {
                    self.progress.elapsed_secs = start.elapsed().as_secs_f32();
                }
                self.progress.error = Some(error);
            }
        }
    }

    pub(super) fn start_scan(&mut self) {
        self.retire_active_scan();
        self.clear_scan_presentation();
        self.active_root = None;

        let generation = match self.advance_scan_generation() {
            Ok(generation) => generation,
            Err(error) => {
                self.progress = ScanProgress {
                    error: Some(error),
                    ..Default::default()
                };
                return;
            }
        };

        let root = match ScanRoot::from_input(&self.scan_path) {
            Ok(root) => root,
            Err(error) => {
                self.progress = ScanProgress {
                    error: Some(format!("{error:#}")),
                    ..Default::default()
                };
                self.exclusions = Exclusions::default();
                return;
            }
        };

        self.exclusions = match exclusions::load(&root) {
            Ok(exclusions) => exclusions,
            Err(error) => {
                warn!("failed to load exclusions for {:?}: {error:#}", root.path());
                Exclusions::new(&root)
            }
        };
        self.update_path_history(&root);

        let (backend, scan_engine_label) = Self::backend_for_mode(self.scanner_mode, &root);
        self.active_root = Some(root.clone());
        self.progress = ScanProgress {
            scanning: true,
            start_time: Some(std::time::Instant::now()),
            scan_engine_label: Some(scan_engine_label),
            ..Default::default()
        };

        let cache_load = self.ensure_cache_service().and_then(|service| {
            service
                .load(generation, root.clone())
                .map_err(|e| format!("{e:#}"))
        });
        if let Err(error) = cache_load {
            self.push_scan_warning(format!("cache preview was not queued: {error}"));
        }

        match scanner::spawn(generation, root, backend) {
            Ok(session) => self.active_scan = Some(session),
            Err(error) => {
                self.progress.scanning = false;
                self.progress.scan_engine_label = None;
                self.progress.error = Some(format!("{error:#}"));
            }
        }
    }

    pub(super) fn stop_scan(&mut self) {
        if let Some(session) = &self.active_scan {
            session.cancel();
        }
    }

    pub(super) fn clear_current_cache(&mut self) -> Result<(), String> {
        let root = match &self.active_root {
            Some(root) => root.clone(),
            None => ScanRoot::from_input(&self.scan_path).map_err(|e| format!("{e:#}"))?,
        };
        let generation = self.scan_generation;
        self.ensure_cache_service()?
            .delete(generation, root)
            .map_err(|e| format!("{e:#}"))?;
        self.cache_age = None;
        Ok(())
    }

    fn poll_cache_events(&mut self) {
        let events: Vec<CacheEvent> = self
            .cache_service
            .as_ref()
            .map(|service| service.try_iter().collect())
            .unwrap_or_default();

        for event in events {
            match event {
                CacheEvent::Loaded {
                    generation,
                    root_id,
                    result,
                } => {
                    let current = generation == self.scan_generation
                        && self
                            .active_root
                            .as_ref()
                            .is_some_and(|root| root.id() == root_id);
                    if !current {
                        continue;
                    }
                    match result {
                        Ok(Some(prepared)) => self.install_cache_preview(generation, prepared),
                        Ok(None) => {}
                        Err(error) => {
                            warn!("cache load failed for generation {generation}: {error}");
                            self.push_scan_warning(format!("cache preview failed: {error}"));
                        }
                    }
                }
                CacheEvent::Stored {
                    generation,
                    root_id,
                    result,
                } => {
                    let current = generation == self.scan_generation
                        && self
                            .active_root
                            .as_ref()
                            .is_some_and(|root| root.id() == root_id);
                    if let Err(error) = result {
                        warn!("cache store failed for generation {generation}: {error}");
                        if current {
                            self.push_scan_warning(format!("cache store failed: {error}"));
                        }
                    }
                }
                CacheEvent::Deleted {
                    generation,
                    root_id,
                    result,
                } => {
                    let current = generation == self.scan_generation
                        && self
                            .active_root
                            .as_ref()
                            .is_some_and(|root| root.id() == root_id);
                    match result {
                        Ok(()) if current => self.cache_age = None,
                        Ok(()) => {}
                        Err(error) => {
                            warn!("cache delete failed for generation {generation}: {error}");
                            if current {
                                self.push_scan_warning(format!("cache delete failed: {error}"));
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn poll_scan(&mut self) {
        self.poll_cache_events();
        self.reap_retired_scans();

        let (generation, root, messages, disconnected) = {
            let Some(session) = self.active_scan.as_ref() else {
                return;
            };
            let generation = session.generation;
            let root = session.root.clone();
            let mut messages = Vec::new();
            loop {
                match session.receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            let disconnected = loop {
                match session.terminal_receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(TryRecvError::Empty) => break false,
                    Err(TryRecvError::Disconnected) => break true,
                }
            };
            (generation, root, messages, disconnected)
        };

        if generation != self.scan_generation
            || !self
                .active_root
                .as_ref()
                .is_some_and(|active| active.same_identity(&root))
        {
            self.finish_active_session();
            return;
        }

        let mut terminal_received = false;
        for message in messages {
            match message {
                ScanMsg::Progress(update) => {
                    self.progress.phase = Some(update.phase);
                    self.progress.items = update.items;
                    self.progress.files = update.files;
                    self.progress.dirs = update.dirs;
                    self.progress.bytes = update.bytes;
                    self.progress.errors = update.errors;
                }
                ScanMsg::Terminal(outcome) => {
                    terminal_received = true;
                    self.handle_terminal(generation, root.clone(), outcome);
                }
                #[cfg(windows)]
                ScanMsg::NtfsFallback(error) => {
                    self.progress.scan_engine_label = Some("jwalk (NTFS fallback)".to_owned());
                    self.push_scan_warning(format!(
                        "NTFS backend unavailable ({error}); using standard scanner"
                    ));
                }
            }
        }

        if terminal_received {
            self.finish_active_session();
        } else if disconnected {
            self.finish_active_session();
            self.progress.scanning = false;
            self.progress.scan_engine_label = None;
            self.progress.error.get_or_insert_with(|| {
                "scanner worker stopped without a terminal outcome".to_owned()
            });
        }
    }
}
