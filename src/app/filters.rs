//! # Filter Functions Module
//!
//! This module contains all filtering logic for the directory tree:
//! - Size range filtering (min/max file size), optional LoD merge of out-of-range files
//! - Exclusion filtering (excluded paths)
//! - Mask/glob filtering (filename patterns)
//! - Search result collection
//!
//! All functions create filtered copies of the tree without modifying the original.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::app::helpers::fmt_size;
use crate::exclusions::Exclusions;
use squarebob_core::{DirEntry, LodExpandInfo, LodKind};

fn copy_file_with_size(src: &DirEntry, size: u64) -> DirEntry {
    DirEntry::new_file(
        src.name.clone(),
        src.path.clone(),
        size,
        src.ext.clone(),
        src.modified_time,
    )
}

fn append_child(parent: &mut DirEntry, child: DirEntry) {
    parent.size += child.size;
    parent.file_count += child.file_count;
    parent.dir_count += if child.is_dir { child.dir_count + 1 } else { 0 };
    parent.children.push(child);
}

fn build_filtered_dir(src: &DirEntry, children: Vec<DirEntry>, keep_empty: bool) -> DirEntry {
    let mut node = DirEntry::new_dir(src.name.clone(), src.path.clone());
    for child in children {
        if !keep_empty && child.size == 0 && child.children.is_empty() {
            continue;
        }
        append_child(&mut node, child);
    }
    node.sort_children_by_size_desc();
    node
}

fn rebuild_tree(
    root: &DirEntry,
    terminal: &mut impl FnMut(&DirEntry) -> Option<DirEntry>,
    finish_dir: &mut impl FnMut(&DirEntry, Vec<DirEntry>) -> DirEntry,
) -> DirEntry {
    struct Frame<'a> {
        source: &'a DirEntry,
        next_child: usize,
        children: Vec<DirEntry>,
    }

    if let Some(result) = terminal(root) {
        return result;
    }

    let mut frames = vec![Frame {
        source: root,
        next_child: 0,
        children: Vec::with_capacity(root.children.len()),
    }];

    loop {
        let next_child = {
            let frame = frames.last_mut().expect("tree rebuild always has a root");
            if frame.next_child < frame.source.children.len() {
                let child = &frame.source.children[frame.next_child];
                frame.next_child += 1;
                Some(child)
            } else {
                None
            }
        };

        if let Some(child) = next_child {
            if let Some(result) = terminal(child) {
                frames
                    .last_mut()
                    .expect("child result always has a parent")
                    .children
                    .push(result);
            } else {
                debug_assert!(child.is_dir);
                frames.push(Frame {
                    source: child,
                    next_child: 0,
                    children: Vec::with_capacity(child.children.len()),
                });
            }
            continue;
        }

        let frame = frames
            .pop()
            .expect("tree rebuild always completes an existing frame");
        let completed = finish_dir(frame.source, frame.children);
        if let Some(parent) = frames.last_mut() {
            parent.children.push(completed);
        } else {
            return completed;
        }
    }
}

/// Collect all paths that match the search/mask filter (and their ancestors)
pub(super) fn collect_matching_paths(
    node: &DirEntry,
    search: &str,
    masks: &[String],
    result: &mut HashSet<PathBuf>,
) -> bool {
    let mut subtree_matches = Vec::new();
    for entry in node.iter_post_order() {
        let has_match = if entry.is_dir {
            let child_start = subtree_matches.len() - entry.children.len();
            let has_match = subtree_matches[child_start..]
                .iter()
                .any(|matched| *matched);
            subtree_matches.truncate(child_start);
            has_match
        } else {
            let matches_search = search.is_empty() || entry.name.to_lowercase().contains(search);
            let matches_mask = masks.is_empty() || matches_any_mask(&entry.name, masks);
            matches_search && matches_mask
        };

        if has_match {
            result.insert(entry.path.clone());
        }
        subtree_matches.push(has_match);
    }
    subtree_matches.pop().unwrap_or(false)
}

/// Check if filename matches any of the glob patterns
pub(super) fn matches_any_mask(filename: &str, masks: &[String]) -> bool {
    if filename.is_ascii() {
        let bytes = filename.as_bytes();
        let mut buf = [0u8; 512];
        if bytes.len() <= buf.len() {
            for (i, &b) in bytes.iter().enumerate() {
                buf[i] = b.to_ascii_lowercase();
            }
            let lowered = std::str::from_utf8(&buf[..bytes.len()]).unwrap();
            return masks.iter().any(|mask| glob_match(mask, lowered));
        }
    }
    let name_lc = filename.to_lowercase();
    masks.iter().any(|mask| glob_match(mask, &name_lc))
}

/// Simple glob matching: supports * and ? wildcards
pub(super) fn glob_match(pattern: &str, text: &str) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    let mut star_pi = None;
    let mut star_ti = 0;

    while ti < txt.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == txt[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }

    pi == pat.len()
}

/// Count files strictly below `min` or strictly above `max`.
pub(super) fn count_files_outside_range(node: &DirEntry, min: u64, max: u64) -> (u64, u64) {
    node.iter()
        .filter(|entry| !entry.is_dir)
        .fold((0, 0), |(below, above), entry| {
            (
                below + u64::from(entry.size < min),
                above + u64::from(entry.size > max),
            )
        })
}

fn push_lod_bucket(
    children: &mut Vec<DirEntry>,
    src: &DirEntry,
    min: u64,
    max: u64,
    expanded: &HashSet<PathBuf>,
    kind: LodKind,
    total_size: u64,
    file_count: u64,
) {
    if file_count == 0 {
        return;
    }

    let (suffix, relation, threshold, extension) = match kind {
        LodKind::BelowMin => ("small", "below", min, "lod_small"),
        LodKind::AboveMax => ("large", "above", max, "lod_large"),
    };
    let bucket_path = src.path.join(format!("__squarebob_lod_{suffix}"));
    let name = format!(
        "{} file{} {} {}",
        file_count,
        if file_count == 1 { "" } else { "s" },
        relation,
        fmt_size(threshold)
    );

    if expanded.contains(&bucket_path) {
        let mut directory = DirEntry::new_dir(name, bucket_path);
        for child in src.children.iter().filter(|child| {
            !child.is_dir
                && match kind {
                    LodKind::BelowMin => child.size < min,
                    LodKind::AboveMax => child.size > max,
                }
        }) {
            append_child(&mut directory, copy_file_with_size(child, child.size));
        }
        directory.sort_children_by_size_desc();
        children.push(directory);
        return;
    }

    let mut synthetic =
        DirEntry::new_file(name, bucket_path, total_size, extension.to_string(), None);
    synthetic.file_count = file_count;
    synthetic.lod_expand = Some(LodExpandInfo {
        parent_dir: src.path.clone(),
        kind,
        min_threshold: min,
        max_threshold: max,
    });
    children.push(synthetic);
}

fn build_merged_dir(
    src: &DirEntry,
    rebuilt_children: Vec<DirEntry>,
    min: u64,
    max: u64,
    expanded: &HashSet<PathBuf>,
) -> DirEntry {
    let mut children = Vec::with_capacity(rebuilt_children.len() + 2);
    let mut small_sum = 0u64;
    let mut small_count = 0u64;
    let mut large_sum = 0u64;
    let mut large_count = 0u64;

    for (source, rebuilt) in src.children.iter().zip(rebuilt_children) {
        if source.is_dir {
            if rebuilt.size > 0 || !rebuilt.children.is_empty() {
                children.push(rebuilt);
            }
        } else if source.size < min {
            small_sum += source.size;
            small_count += 1;
        } else if source.size > max {
            large_sum += source.size;
            large_count += 1;
        } else {
            children.push(rebuilt);
        }
    }

    push_lod_bucket(
        &mut children,
        src,
        min,
        max,
        expanded,
        LodKind::BelowMin,
        small_sum,
        small_count,
    );
    push_lod_bucket(
        &mut children,
        src,
        min,
        max,
        expanded,
        LodKind::AboveMax,
        large_sum,
        large_count,
    );
    build_filtered_dir(src, children, true)
}

/// Build a tree like [`filter_tree`] for the middle band, but instead of dropping
/// files outside `[min, max]`, merge them into at most two synthetic leaves per directory
/// (“below min” and “above max”). Keeps total sizes and file counts consistent for treemap layout.
///
/// Paths in `expanded` (typically `…/__squarebob_lod_small` / `…/__squarebob_lod_large`) are built as
/// real directories listing individual files so the user can zoom into the bucket.
pub(super) fn merge_tree_by_size_range(
    src: &DirEntry,
    min: u64,
    max: u64,
    expanded: &HashSet<PathBuf>,
) -> DirEntry {
    rebuild_tree(
        src,
        &mut |entry| (!entry.is_dir).then(|| copy_file_with_size(entry, entry.size)),
        &mut |entry, children| build_merged_dir(entry, children, min, max, expanded),
    )
}

/// Create a filtered copy of the tree, excluding files outside size range.
/// BUG-1 fix: also filters leaf files at root level.
pub(super) fn filter_tree(src: &DirEntry, min: u64, max: u64, invert: bool) -> DirEntry {
    rebuild_tree(
        src,
        &mut |entry| {
            if entry.is_dir {
                return None;
            }
            let in_range = entry.size >= min && entry.size <= max;
            let include = if invert { !in_range } else { in_range };
            Some(copy_file_with_size(
                entry,
                if include { entry.size } else { 0 },
            ))
        },
        &mut |entry, children| build_filtered_dir(entry, children, false),
    )
}

/// Filter out excluded paths from tree. If show_excluded is true, keeps them with __excluded__ marker.
pub(super) fn filter_excluded(
    src: &DirEntry,
    exclusions: &Exclusions,
    show_excluded: bool,
) -> DirEntry {
    rebuild_tree(
        src,
        &mut |entry| {
            if exclusions.contains(&entry.path) {
                return Some(if show_excluded {
                    if entry.is_dir {
                        let mut excluded =
                            DirEntry::new_dir(entry.name.clone(), entry.path.clone());
                        excluded.ext = "__excluded__".to_string();
                        excluded.size = entry.size;
                        excluded.file_count = entry.file_count;
                        excluded.dir_count = entry.dir_count;
                        excluded
                    } else {
                        DirEntry::new_file(
                            entry.name.clone(),
                            entry.path.clone(),
                            entry.size,
                            "__excluded__".to_string(),
                            entry.modified_time,
                        )
                    }
                } else {
                    DirEntry::new_dir(entry.name.clone(), entry.path.clone())
                });
            }
            (!entry.is_dir).then(|| copy_file_with_size(entry, entry.size))
        },
        &mut |entry, children| build_filtered_dir(entry, children, show_excluded),
    )
}

/// Filter tree to only include files matching the glob masks
pub(super) fn filter_by_mask(src: &DirEntry, masks: &[String]) -> DirEntry {
    rebuild_tree(
        src,
        &mut |entry| {
            (!entry.is_dir).then(|| {
                let size = if matches_any_mask(&entry.name, masks) {
                    entry.size
                } else {
                    0
                };
                copy_file_with_size(entry, size)
            })
        },
        &mut |entry, children| build_filtered_dir(entry, children, false),
    )
}

/// Filter tree to only include files matching selected extensions.
/// If invert is true, excludes selected extensions instead.
pub(super) fn filter_by_extension(
    src: &DirEntry,
    exts: &HashSet<String>,
    invert: bool,
) -> DirEntry {
    rebuild_tree(
        src,
        &mut |entry| {
            if entry.is_dir {
                return None;
            }
            let ext_key = if entry.ext.is_empty() {
                "<none>"
            } else {
                entry.ext.as_str()
            }
            .to_lowercase();
            let in_set = exts.contains(&ext_key);
            let include = if invert { !in_set } else { in_set };
            Some(copy_file_with_size(
                entry,
                if include { entry.size } else { 0 },
            ))
        },
        &mut |entry, children| build_filtered_dir(entry, children, false),
    )
}

/// Count files that match size range (min/max) with optional invert.
pub(super) fn count_files_in_range(node: &DirEntry, min: u64, max: u64, invert: bool) -> u64 {
    node.iter()
        .filter(|entry| !entry.is_dir)
        .filter(|entry| {
            let in_range = entry.size >= min && entry.size <= max;
            if invert { !in_range } else { in_range }
        })
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str, path: PathBuf, size: u64) -> DirEntry {
        DirEntry::new_file(name.to_string(), path, size, "txt".to_string(), None)
    }

    #[test]
    fn merge_buckets_outside_range() {
        let root_path = PathBuf::from("/tmp");
        let mut root = DirEntry::new_dir("tmp".to_string(), root_path.clone());
        root.children.push(file("tiny", root_path.join("a"), 10));
        root.children.push(file("mid", root_path.join("b"), 500));
        root.children
            .push(file("huge", root_path.join("c"), 10_000));
        root.size = 10 + 500 + 10_000;
        root.file_count = 3;

        let empty = HashSet::new();
        let merged = merge_tree_by_size_range(&root, 100, 1000, &empty);
        assert_eq!(merged.children.len(), 3);
        assert_eq!(merged.size, root.size);
        assert_eq!(merged.file_count, 3);

        let names: Vec<_> = merged.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("below")));
        assert!(names.iter().any(|n| n.contains("above")));
        assert!(names.contains(&"mid"));
        let tiny = merged
            .children
            .iter()
            .find(|c| c.path.ends_with("__squarebob_lod_small"))
            .expect("lod small");
        assert!(tiny.lod_expand.is_some());
    }

    #[test]
    fn merge_expanded_small_is_directory() {
        let root_path = PathBuf::from("/tmp");
        let mut root = DirEntry::new_dir("tmp".to_string(), root_path.clone());
        root.children.push(file("tiny", root_path.join("a"), 10));
        root.children.push(file("mid", root_path.join("b"), 500));
        root.size = 10 + 500;
        root.file_count = 2;

        let mut exp = HashSet::new();
        exp.insert(root_path.join("__squarebob_lod_small"));
        let merged = merge_tree_by_size_range(&root, 100, 1000, &exp);
        let lod = merged
            .children
            .iter()
            .find(|c| c.path.ends_with("__squarebob_lod_small"))
            .expect("lod");
        assert!(lod.is_dir);
        assert_eq!(lod.children.len(), 1);
        assert!(lod.lod_expand.is_none());
    }

    #[test]
    fn count_outside_range() {
        let root_path = PathBuf::from("/r");
        let mut root = DirEntry::new_dir("r".to_string(), root_path.clone());
        root.children.push(file("a", root_path.join("a"), 5));
        root.children.push(file("b", root_path.join("b"), 500));
        let (below, above) = count_files_outside_range(&root, 100, 1000);
        assert_eq!(below, 1);
        assert_eq!(above, 0);
    }
}
