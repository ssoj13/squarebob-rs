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
use dirstat_core::{DirEntry, LodExpandInfo, LodKind};

/// Collect all paths that match the search/mask filter (and their ancestors)
pub(super) fn collect_matching_paths(
    node: &DirEntry,
    search: &str,
    masks: &[String],
    result: &mut HashSet<PathBuf>,
) -> bool {
    if !node.is_dir {
        // File: check if it matches
        let matches_search = search.is_empty() || node.name.to_lowercase().contains(search);
        let matches_mask = masks.is_empty() || matches_any_mask(&node.name, masks);
        if matches_search && matches_mask {
            result.insert(node.path.clone());
            return true;
        }
        return false;
    }
    
    // Directory: check children recursively
    let mut has_match = false;
    for child in &node.children {
        if collect_matching_paths(child, search, masks, result) {
            has_match = true;
        }
    }
    
    // If any child matched, include this directory
    if has_match {
        result.insert(node.path.clone());
    }
    has_match
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

/// Count files strictly below `min` or strictly above `max` (recursive).
pub(super) fn count_files_outside_range(node: &DirEntry, min: u64, max: u64) -> (u64, u64) {
    if !node.is_dir {
        if node.size < min {
            return (1, 0);
        }
        if node.size > max {
            return (0, 1);
        }
        return (0, 0);
    }
    let mut below = 0u64;
    let mut above = 0u64;
    for child in &node.children {
        let (b, a) = count_files_outside_range(child, min, max);
        below += b;
        above += a;
    }
    (below, above)
}

/// Build a tree like [`filter_tree`] for the middle band, but instead of dropping
/// files outside `[min, max]`, merge them into at most two synthetic leaves per directory
/// (“below min” and “above max”). Keeps total sizes and file counts consistent for treemap layout.
///
/// Paths in `expanded` (typically `…/__dirstat_lod_small` / `…/__dirstat_lod_large`) are built as
/// real directories listing individual files so the user can zoom into the bucket.
pub(super) fn merge_tree_by_size_range(
    src: &DirEntry,
    min: u64,
    max: u64,
    expanded: &HashSet<PathBuf>,
) -> DirEntry {
    if !src.is_dir {
        return DirEntry::new_file(
            src.name.clone(),
            src.path.clone(),
            src.size,
            src.ext.clone(),
            src.modified_time,
        );
    }

    let mut node = DirEntry::new_dir(src.name.clone(), src.path.clone());
    let mut children: Vec<DirEntry> = Vec::new();

    let mut small_sum = 0u64;
    let mut small_n = 0u64;
    let mut large_sum = 0u64;
    let mut large_n = 0u64;

    for child in &src.children {
        if child.is_dir {
            let merged = merge_tree_by_size_range(child, min, max, expanded);
            if merged.size > 0 || !merged.children.is_empty() {
                children.push(merged);
            }
        } else if child.size < min {
            small_sum += child.size;
            small_n += 1;
        } else if child.size > max {
            large_sum += child.size;
            large_n += 1;
        } else {
            children.push(DirEntry::new_file(
                child.name.clone(),
                child.path.clone(),
                child.size,
                child.ext.clone(),
                child.modified_time,
            ));
        }
    }

    let lod_small_path = src.path.join("__dirstat_lod_small");
    if small_n > 0 {
        let name = format!(
            "{} file{} below {}",
            small_n,
            if small_n == 1 { "" } else { "s" },
            fmt_size(min)
        );
        if expanded.contains(&lod_small_path) {
            let mut dir = DirEntry::new_dir(name, lod_small_path.clone());
            for child in &src.children {
                if !child.is_dir && child.size < min {
                    dir.children.push(DirEntry::new_file(
                        child.name.clone(),
                        child.path.clone(),
                        child.size,
                        child.ext.clone(),
                        child.modified_time,
                    ));
                }
            }
            dir.sort_children_by_size_desc();
            for c in &dir.children {
                dir.size += c.size;
                dir.file_count += c.file_count;
                dir.dir_count += if c.is_dir { c.dir_count + 1 } else { 0 };
            }
            children.push(dir);
        } else {
            let mut syn = DirEntry::new_file(
                name,
                lod_small_path.clone(),
                small_sum,
                "lod_small".to_string(),
                None,
            );
            syn.file_count = small_n;
            syn.lod_expand = Some(LodExpandInfo {
                parent_dir: src.path.clone(),
                kind: LodKind::BelowMin,
                min_threshold: min,
                max_threshold: max,
            });
            children.push(syn);
        }
    }
    let lod_large_path = src.path.join("__dirstat_lod_large");
    if large_n > 0 {
        let name = format!(
            "{} file{} above {}",
            large_n,
            if large_n == 1 { "" } else { "s" },
            fmt_size(max)
        );
        if expanded.contains(&lod_large_path) {
            let mut dir = DirEntry::new_dir(name, lod_large_path.clone());
            for child in &src.children {
                if !child.is_dir && child.size > max {
                    dir.children.push(DirEntry::new_file(
                        child.name.clone(),
                        child.path.clone(),
                        child.size,
                        child.ext.clone(),
                        child.modified_time,
                    ));
                }
            }
            dir.sort_children_by_size_desc();
            for c in &dir.children {
                dir.size += c.size;
                dir.file_count += c.file_count;
                dir.dir_count += if c.is_dir { c.dir_count + 1 } else { 0 };
            }
            children.push(dir);
        } else {
            let mut syn = DirEntry::new_file(
                name,
                lod_large_path.clone(),
                large_sum,
                "lod_large".to_string(),
                None,
            );
            syn.file_count = large_n;
            syn.lod_expand = Some(LodExpandInfo {
                parent_dir: src.path.clone(),
                kind: LodKind::AboveMax,
                min_threshold: min,
                max_threshold: max,
            });
            children.push(syn);
        }
    }

    for c in &children {
        node.size += c.size;
        node.file_count += c.file_count;
        node.dir_count += if c.is_dir { c.dir_count + 1 } else { 0 };
    }

    node.children = children;
    node.sort_children_by_size_desc();
    node
}

/// Create a filtered copy of the tree, excluding files outside size range.
/// BUG-1 fix: also filters leaf files at root level.
pub(super) fn filter_tree(src: &DirEntry, min: u64, max: u64, invert: bool) -> DirEntry {
    if !src.is_dir {
        let in_range = src.size >= min && src.size <= max;
        let include = if invert { !in_range } else { in_range };
        if include {
            return DirEntry::new_file(src.name.clone(), src.path.clone(), src.size, src.ext.clone(), src.modified_time);
        } else {
            // Excluded file: return zero-size placeholder
            return DirEntry::new_file(src.name.clone(), src.path.clone(), 0, src.ext.clone(), src.modified_time);
        }
    }
    let mut node = DirEntry::new_dir(src.name.clone(), src.path.clone());
    for child in &src.children {
        if child.is_dir {
            let filtered_child = filter_tree(child, min, max, invert);
            if filtered_child.size > 0 || !filtered_child.children.is_empty() {
                node.size += filtered_child.size;
                node.file_count += filtered_child.file_count;
                node.dir_count += filtered_child.dir_count + 1;
                node.children.push(filtered_child);
            }
        } else {
            let in_range = child.size >= min && child.size <= max;
            let include = if invert { !in_range } else { in_range };
            if include {
                node.size += child.size;
                node.file_count += 1;
                node.children.push(DirEntry::new_file(
                    child.name.clone(), child.path.clone(), child.size, child.ext.clone(), child.modified_time,
                ));
            }
        }
    }
    node.sort_children_by_size_desc();
    node
}

/// Filter out excluded paths from tree. If show_excluded is true, keeps them with __excluded__ marker.
pub(super) fn filter_excluded(src: &DirEntry, exclusions: &Exclusions, show_excluded: bool) -> DirEntry {
    filter_excluded_recursive(src, exclusions, show_excluded)
}

pub(super) fn filter_excluded_recursive(src: &DirEntry, exclusions: &Exclusions, show_excluded: bool) -> DirEntry {
    let is_excluded = exclusions.contains(&src.path);
    
    // For excluded items
    if is_excluded {
        if show_excluded {
            // Show as grayed out (use __excluded__ marker)
            let node = if src.is_dir {
                let mut d = DirEntry::new_dir(src.name.clone(), src.path.clone());
                d.ext = "__excluded__".to_string();
                d.size = src.size;
                d.file_count = src.file_count;
                d.dir_count = src.dir_count;
                d
            } else {
                DirEntry::new_file(src.name.clone(), src.path.clone(), src.size, "__excluded__".to_string(), src.modified_time)
            };
            // Don't recurse into excluded directories
            return node;
        } else {
            // Return zero-size node (effectively hidden)
            return DirEntry::new_dir(src.name.clone(), src.path.clone());
        }
    }
    
    // Not excluded - process normally
    if !src.is_dir {
        return DirEntry::new_file(src.name.clone(), src.path.clone(), src.size, src.ext.clone(), src.modified_time);
    }
    
    let mut node = DirEntry::new_dir(src.name.clone(), src.path.clone());
    
    for child in &src.children {
        let filtered = filter_excluded_recursive(child, exclusions, show_excluded);
        
        // Skip empty nodes (hidden exclusions)
        if filtered.size == 0 && filtered.children.is_empty() && !show_excluded {
            continue;
        }
        
        node.size += filtered.size;
        node.file_count += filtered.file_count;
        node.dir_count += if filtered.is_dir { filtered.dir_count + 1 } else { 0 };
        node.children.push(filtered);
    }
    
    node.sort_children_by_size_desc();
    node
}

/// Filter tree to only include files matching the glob masks
pub(super) fn filter_by_mask(src: &DirEntry, masks: &[String]) -> DirEntry {
    if !src.is_dir {
        // File: include only if it matches any mask
        if matches_any_mask(&src.name, masks) {
            return DirEntry::new_file(src.name.clone(), src.path.clone(), src.size, src.ext.clone(), src.modified_time);
        } else {
            // Return zero-size placeholder (will be filtered out)
            return DirEntry::new_file(src.name.clone(), src.path.clone(), 0, src.ext.clone(), src.modified_time);
        }
    }
    
    // Directory: recurse and only keep children that have content
    let mut node = DirEntry::new_dir(src.name.clone(), src.path.clone());
    
    for child in &src.children {
        let filtered = filter_by_mask(child, masks);
        
        // Skip empty nodes
        if filtered.size == 0 && filtered.children.is_empty() {
            continue;
        }
        
        node.size += filtered.size;
        node.file_count += filtered.file_count;
        node.dir_count += if filtered.is_dir { filtered.dir_count + 1 } else { 0 };
        node.children.push(filtered);
    }
    
    node.sort_children_by_size_desc();
    node
}

/// Filter tree to only include files matching selected extensions.
/// If invert is true, excludes selected extensions instead.
pub(super) fn filter_by_extension(src: &DirEntry, exts: &HashSet<String>, invert: bool) -> DirEntry {
    if !src.is_dir {
        let ext_key = if src.ext.is_empty() { "<none>" } else { src.ext.as_str() }.to_lowercase();
        let in_set = exts.contains(&ext_key);
        let include = if invert { !in_set } else { in_set };
        if include {
            return DirEntry::new_file(src.name.clone(), src.path.clone(), src.size, src.ext.clone(), src.modified_time);
        } else {
            return DirEntry::new_file(src.name.clone(), src.path.clone(), 0, src.ext.clone(), src.modified_time);
        }
    }

    let mut node = DirEntry::new_dir(src.name.clone(), src.path.clone());
    for child in &src.children {
        let filtered = filter_by_extension(child, exts, invert);
        if filtered.size == 0 && filtered.children.is_empty() {
            continue;
        }
        node.size += filtered.size;
        node.file_count += filtered.file_count;
        node.dir_count += if filtered.is_dir { filtered.dir_count + 1 } else { 0 };
        node.children.push(filtered);
    }
    node.sort_children_by_size_desc();
    node
}

/// Count files that match size range (min/max) with optional invert.
pub(super) fn count_files_in_range(node: &DirEntry, min: u64, max: u64, invert: bool) -> u64 {
    if !node.is_dir {
        let in_range = node.size >= min && node.size <= max;
        let include = if invert { !in_range } else { in_range };
        return if include { 1 } else { 0 };
    }
    let mut count = 0u64;
    for child in &node.children {
        count += count_files_in_range(child, min, max, invert);
    }
    count
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
        root.children.push(file("huge", root_path.join("c"), 10_000));
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
            .find(|c| c.path.ends_with("__dirstat_lod_small"))
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
        exp.insert(root_path.join("__dirstat_lod_small"));
        let merged = merge_tree_by_size_range(&root, 100, 1000, &exp);
        let lod = merged
            .children
            .iter()
            .find(|c| c.path.ends_with("__dirstat_lod_small"))
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
