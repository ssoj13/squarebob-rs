use squarebob_core::DirEntry;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub mod path_key {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct ScanRoot(PathBuf);

    impl ScanRoot {
        pub fn new(path: PathBuf) -> Self {
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }
}

pub mod scanner {
    use super::*;
    use crate::path_key::ScanRoot;
    use crossbeam_channel::Sender;

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
    }

    #[derive(Debug)]
    pub struct ScanBuild {
        pub tree: DirEntry,
        pub diagnostics: ScanDiagnostics,
    }

    #[derive(Debug)]
    pub enum ScanFailure {
        Cancelled,
        BackendUnavailable(anyhow::Error),
        Failed(anyhow::Error),
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
    pub enum ScanOutcome {
        Completed,
        Partial,
        Cancelled,
        Failed(String),
    }

    #[derive(Debug)]
    pub enum ScanMsg {
        Progress(ScanProgressUpdate),
        Terminal(ScanOutcome),
        NtfsFallback(String),
    }

    pub fn finish_build(_root: &ScanRoot, _build: ScanBuild) -> ScanOutcome {
        ScanOutcome::Completed
    }

    pub fn send_progress_update(
        tx: &Sender<ScanMsg>,
        cancel: &AtomicBool,
        update: ScanProgressUpdate,
    ) -> Result<(), ScanFailure> {
        if cancel.load(Ordering::Acquire) {
            return Err(ScanFailure::Cancelled);
        }
        let _ = tx.try_send(ScanMsg::Progress(update));
        Ok(())
    }

    pub fn scan_dir_public(
        _root: &Path,
        _tx: &Sender<ScanMsg>,
        _cancel: &AtomicBool,
    ) -> Result<ScanBuild, ScanFailure> {
        Err(ScanFailure::BackendUnavailable(anyhow::anyhow!(
            "unused harness fallback"
        )))
    }
}

#[path = "../../../src/scanner_ntfs.rs"]
pub mod scanner_ntfs;
