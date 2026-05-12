//! Helper functions for the application
//!
//! This module contains standalone utility functions extracted from the main app module
//! for better organization and maintainability.

use dirstat_core::DirEntry;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Format byte size to human-readable string (KB, MB, GB, TB)
///
/// Used throughout the application for displaying file and directory sizes.
/// This function is public because it's also used in main.rs and other modules.
pub fn fmt_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Parse human-readable size string back to bytes.
/// Accepts: bare numbers ("1024"), suffixed ("1.5G", "100mb", "2 KiB", "512 b").
/// Returns `None` on malformed input. Used as `custom_parser` for size sliders.
pub fn parse_size(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num_part, unit_part) = s.split_at(split);
    let num: f64 = num_part.trim().parse().ok()?;

    let mult: f64 = match unit_part
        .trim()
        .to_ascii_lowercase()
        .trim_end_matches('b')
        .trim_end_matches('i')
        .trim()
    {
        "" => 1.0,
        "k" => 1024.0,
        "m" => 1024.0 * 1024.0,
        "g" => 1024.0 * 1024.0 * 1024.0,
        "t" => 1024.0_f64.powi(4),
        _ => return None,
    };
    Some(num * mult)
}

/// Layout direction for multi-button groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiButtonAxis {
    Horizontal,
    /// Reserved for future compact toolbars (all call sites use horizontal today).
    #[allow(dead_code)]
    Vertical,
}

/// Render a compact set of toggle-style buttons for exclusive selection.
pub(super) fn multibutton_exclusive<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
    axis: MultiButtonAxis,
) -> bool {
    let mut changed = false;
    let layout = |ui: &mut egui::Ui| {
        for (opt, label) in options {
            let selected = *value == *opt;
            let resp = ui.add(egui::Button::new(*label).selected(selected));
            if resp.clicked() && !selected {
                *value = *opt;
                changed = true;
            }
        }
    };
    match axis {
        MultiButtonAxis::Horizontal => ui.horizontal(layout),
        MultiButtonAxis::Vertical => ui.vertical(layout),
    };
    changed
}

/// Open a folder picker dialog and return selected path
pub(super) fn rfd_pick_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

/// Starting directory for the env-map file picker: parent of the current env map file if it
/// exists, otherwise the executable's directory (so packaged assets next to the binary are easy to find).
pub(super) fn rfd_env_map_pick_start_dir(current: Option<&std::path::PathBuf>) -> Option<std::path::PathBuf> {
    if let Some(p) = current {
        if p.is_file() {
            let path_for_parent = p.canonicalize().unwrap_or_else(|_| p.clone());
            if let Some(parent) = path_for_parent.parent() {
                return Some(parent.to_path_buf());
            }
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
}

/// Collect extension statistics from directory tree
///
/// Returns vector of (extension, total_size, file_count) sorted by size descending.
/// Files without extensions are grouped under "<none>".
pub(super) fn compute_ext_stats(root: &DirEntry) -> Vec<(String, u64, u64)> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    collect_ext(root, &mut map);
    let mut stats: Vec<(String, u64, u64)> = map
        .into_iter()
        .map(|(ext, (size, count))| (ext, size, count))
        .collect();
    stats.sort_by_key(|b| std::cmp::Reverse(b.1));
    stats
}

/// Helper for compute_ext_stats - recursively collect extension data
fn collect_ext(node: &DirEntry, map: &mut HashMap<String, (u64, u64)>) {
    if !node.is_dir {
        let ext = if node.ext.is_empty() {
            "<none>".to_string()
        } else {
            node.ext.clone()
        };
        let e = map.entry(ext).or_insert((0, 0));
        e.0 += node.size;
        e.1 += 1;
    }
    for child in &node.children {
        collect_ext(child, map);
    }
}

/// Find a node by path in the directory tree
///
/// Returns reference to the node if found, None otherwise.
pub(super) fn find_node_by_path<'a>(node: &'a DirEntry, target: &PathBuf) -> Option<&'a DirEntry> {
    if &node.path == target {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_node_by_path(child, target) {
            return Some(found);
        }
    }
    None
}

/// Find minimum and maximum file sizes in the tree
///
/// Only considers leaf files (not directories). Returns (min, max).
/// If no files found, returns (0, 0).
pub(super) fn compute_size_range(root: &DirEntry) -> (u64, u64) {
    let mut min = u64::MAX;
    let mut max = 0u64;
    collect_size_range(root, &mut min, &mut max);
    if min == u64::MAX {
        min = 0;
    }
    (min, max)
}

/// Helper for compute_size_range - recursively collect min/max sizes
fn collect_size_range(node: &DirEntry, min: &mut u64, max: &mut u64) {
    if !node.is_dir && node.size > 0 {
        *min = (*min).min(node.size);
        *max = (*max).max(node.size);
    }
    for child in &node.children {
        collect_size_range(child, min, max);
    }
}

/// Format a tree label with name, size, and percentage
///
/// Example: "Documents [1.2 GB] 45%"
pub(super) fn format_tree_label(name: &str, size: u64, parent_size: u64) -> String {
    let pct = if parent_size > 0 {
        size as f64 / parent_size as f64 * 100.0
    } else {
        100.0
    };
    format!("{} [{}] {:.0}%", name, fmt_size(size), pct)
}

/// Get directory from path (handles both files and directories)
///
/// If path is a file, returns its parent directory.
/// If path is already a directory, returns the path itself.
pub(super) fn path_to_dir(path: &Path) -> &Path {
    if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    }
}

/// Get free and total disk space for a given path
///
/// Returns Some((free_bytes, total_bytes)) on success, None on failure.
/// Platform-specific implementation for Windows and Unix systems.
pub(super) fn disk_free_total(path: &str) -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        if path.len() < 2 || path.as_bytes()[1] != b':' {
            return None;
        }
        let root = format!("{}:\\", &path[..1]);
        let wide: Vec<u16> = OsStr::new(&root)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free: u64 = 0;
        let mut total: u64 = 0;
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                windows::core::PCWSTR(wide.as_ptr()),
                None,
                Some(&mut total as *mut u64 as *mut _),
                Some(&mut free as *mut u64 as *mut _),
            );
        }
        if total > 0 {
            Some((free, total))
        } else {
            None
        }
    }
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        // Use the path itself or find a valid path on the filesystem
        let check_path = if Path::new(path).exists() {
            path.to_string()
        } else {
            // Try to find a parent that exists
            let mut p = PathBuf::from(path);
            while !p.exists() && p.parent().is_some() {
                p = p.parent().unwrap().to_path_buf();
            }
            p.to_string_lossy().to_string()
        };

        let c_path = CString::new(check_path).ok()?;
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        let result = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
        if result == 0 {
            let stat = unsafe { stat.assume_init() };
            fn statvfs_field_to_u64<T: Into<u64>>(value: T) -> u64 {
                value.into()
            }

            let block_size = statvfs_field_to_u64(stat.f_frsize);
            let total = statvfs_field_to_u64(stat.f_blocks).saturating_mul(block_size);
            let free = statvfs_field_to_u64(stat.f_bavail).saturating_mul(block_size);
            Some((free, total))
        } else {
            None
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// Get formatted disk space information string for status bar
///
/// Returns empty string if disk info unavailable.
/// Example: "  |  Disk: 45.2 GB free / 256.0 GB total"
#[allow(unused_variables)]
pub(super) fn disk_free_info(path: &str) -> String {
    if let Some((free, total)) = disk_free_total(path) {
        format!(
            "  |  Disk: {} free / {} total",
            fmt_size(free),
            fmt_size(total)
        )
    } else {
        String::new()
    }
}

/// Collect all directory paths from the tree into a HashSet
///
/// Only includes directories, not files.
pub(super) fn collect_all_dir_paths(node: &DirEntry, result: &mut HashSet<PathBuf>) {
    if node.is_dir {
        result.insert(node.path.clone());
        for child in &node.children {
            collect_all_dir_paths(child, result);
        }
    }
}
