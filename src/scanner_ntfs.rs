//! Squarebob adapter for the NTFS scanner in fscan-rs.

use std::path::Path;

#[cfg(windows)]
use crate::path_key::ScanRoot;
#[cfg(windows)]
use crate::scanner::{
    ScanBuild, ScanDiagnostics, ScanFailure, ScanMsg, ScanOutcome, ScanPhase, ScanProgressUpdate,
    finish_build,
};
#[cfg(windows)]
use crossbeam_channel::Sender;
#[cfg(windows)]
use squarebob_core::DirEntry;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::AtomicBool;

#[cfg(windows)]
pub fn is_ntfs_available(path: &Path) -> bool {
    fscan_rs::is_ntfs_available(path)
}

#[cfg(not(windows))]
pub fn is_ntfs_available(_path: &Path) -> bool {
    false
}

#[cfg(windows)]
pub fn probe_raw_volume_access(path: &Path) -> anyhow::Result<()> {
    fscan_rs::probe_raw_volume_access(path)
}

#[cfg(not(windows))]
pub fn probe_raw_volume_access(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("raw NTFS access is only available on Windows")
}

#[cfg(windows)]
pub fn diagnose_fsctl_enum_usn(path: &Path, max_ioctl_loops: usize) -> anyhow::Result<String> {
    fscan_rs::diagnose_fsctl_enum_usn(path, max_ioctl_loops)
}

#[cfg(not(windows))]
pub fn diagnose_fsctl_enum_usn(_path: &Path, _max_ioctl_loops: usize) -> anyhow::Result<String> {
    anyhow::bail!("raw NTFS access is only available on Windows")
}

#[cfg(windows)]
pub fn mft_dump_names(path: &Path, max_names: usize) -> anyhow::Result<String> {
    fscan_rs::mft_dump_names(path, max_names)
}

#[cfg(not(windows))]
pub fn mft_dump_names(_path: &Path, _max_names: usize) -> anyhow::Result<String> {
    anyhow::bail!("raw NTFS access is only available on Windows")
}

#[cfg(windows)]
fn convert_tree(tree: fscan_rs::TreeEntry) -> DirEntry {
    struct Frame {
        node: fscan_rs::TreeEntry,
        remaining: std::vec::IntoIter<fscan_rs::TreeEntry>,
        built_children: Vec<DirEntry>,
    }
    fn frame(mut node: fscan_rs::TreeEntry) -> Frame {
        let remaining = std::mem::take(&mut node.children).into_iter();
        Frame { node, remaining, built_children: Vec::new() }
    }
    let mut frames = vec![frame(tree)];
    loop {
        if let Some(child) = frames.last_mut().expect("root frame").remaining.next() {
            frames.push(frame(child));
            continue;
        }
        let mut completed = frames.pop().expect("frame exists");
        let name = std::mem::take(&mut completed.node.name);
        let path = std::mem::take(&mut completed.node.path);
        let mut built = if completed.node.is_dir {
            DirEntry::new_dir(name, path)
        } else {
            DirEntry::new_file(
                name,
                path,
                completed.node.own_size,
                std::mem::take(&mut completed.node.ext),
                completed.node.modified_time,
            )
        };
        built.size = completed.node.size;
        built.own_size = completed.node.own_size;
        built.file_count = completed.node.file_count;
        built.dir_count = completed.node.dir_count;
        built.children = completed.built_children;
        if let Some(parent) = frames.last_mut() {
            parent.built_children.push(built);
        } else {
            return built;
        }
    }
}

#[cfg(windows)]
pub(crate) fn run_ntfs(
    root: ScanRoot,
    tx: Sender<ScanMsg>,
    terminal_tx: Sender<ScanMsg>,
    cancel: Arc<AtomicBool>,
) {
    let outcome = match fscan_rs::scan_ntfs_tree_with_progress(root.path(), &cancel, |progress| {
        let phase = match progress.phase {
            fscan_rs::ScanPhase::IndexingVolume => ScanPhase::IndexingVolume,
            fscan_rs::ScanPhase::SelectingTree => ScanPhase::SelectingTree,
            fscan_rs::ScanPhase::MeasuringTree => ScanPhase::MeasuringTree,
        };
        let _ = tx.try_send(ScanMsg::Progress(ScanProgressUpdate {
            phase,
            items: progress.items,
            files: progress.files,
            dirs: progress.dirs,
            bytes: progress.bytes,
            errors: progress.errors,
        }));
    }) {
        Ok((tree, diagnostics)) => finish_build(
            &root,
            ScanBuild {
                tree: convert_tree(tree),
                diagnostics: ScanDiagnostics {
                    walk_errors: diagnostics.walk_errors,
                    metadata_errors: diagnostics.metadata_errors,
                    malformed_records: diagnostics.malformed_records,
                    depth_errors: diagnostics.depth_errors,
                },
            },
        ),
        Err(fscan_rs::ScanFailure::Cancelled) => ScanOutcome::Cancelled,
        Err(fscan_rs::ScanFailure::BackendUnavailable(error)) => {
            let _ = terminal_tx.send(ScanMsg::NtfsFallback(format!("{error:#}")));
            match crate::scanner::scan_dir_public(root.path(), &tx, &cancel) {
                Ok(build) => finish_build(&root, build),
                Err(ScanFailure::Cancelled) => ScanOutcome::Cancelled,
                Err(ScanFailure::Failed(error)) => {
                    ScanOutcome::Failed(format!("standard fallback failed: {error:#}"))
                }
            }
        }
        Err(fscan_rs::ScanFailure::Failed(error)) => {
            ScanOutcome::Failed(format!("NTFS scan failed: {error:#}"))
        }
    };
    let _ = terminal_tx.send(ScanMsg::Terminal(outcome));
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn converted_tree_keeps_nested_aggregates() {
        let mut root = fscan_rs::TreeEntry::new_dir("root".into(), Path::new("C:\\root").into());
        root.children.push(fscan_rs::TreeEntry::new_file(
            "item.txt".into(),
            Path::new("C:\\root\\item.txt").into(),
            7,
            "txt".into(),
            None,
        ));
        root.size = 7;
        root.file_count = 1;
        let converted = convert_tree(root);
        assert_eq!(converted.size, 7);
        assert_eq!(converted.file_count, 1);
        assert_eq!(converted.children[0].size, 7);
        assert_eq!(converted.children[0].ext, "txt");
    }
}
