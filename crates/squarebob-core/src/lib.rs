use std::cell::Cell;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which side of the size band was merged into an LoD bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LodKind {
    BelowMin,
    AboveMax,
}

/// Metadata on a collapsed LoD leaf: enough to expand into real files without duplicating them in memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LodExpandInfo {
    /// Directory whose direct file children were merged into this bucket.
    pub parent_dir: PathBuf,
    pub kind: LodKind,
    pub min_threshold: u64,
    pub max_threshold: u64,
}

/// A node in the directory tree.
/// `rect` uses Cell for interior mutability - treemap layout sets rects
/// without requiring &mut, eliminating the need to clone the tree.
///
/// Intentionally has no recursive serde implementation. Persistence must use an
/// explicit flat representation so untrusted or deeply nested trees never consume
/// the process call stack.
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64, // total recursive size
    /// Per-file size on disk; for directories typically 0. Used by 3D height cues and NTFS fill pass.
    pub own_size: u64,
    pub children: Vec<DirEntry>,
    pub is_dir: bool,
    pub ext: String, // lowercase extension for color mapping
    pub file_count: u64,
    pub dir_count: u64,
    /// Modified time as Unix timestamp (seconds since epoch)
    pub modified_time: Option<u64>,
    /// Layout rect (x, y, w, h) set by treemap via interior mutability
    pub rect: Cell<[f32; 4]>,
    /// Set on collapsed LoD synthetic leaves; used to expand into per-file children on zoom.
    pub lod_expand: Option<LodExpandInfo>,
}

fn default_rect() -> Cell<[f32; 4]> {
    Cell::new([0.0; 4])
}

struct DirEntryDebugNode<'a> {
    entry: &'a DirEntry,
    depth: usize,
}

impl std::fmt::Debug for DirEntryDebugNode<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirEntryNode")
            .field("depth", &self.depth)
            .field("name", &self.entry.name)
            .field("path", &self.entry.path)
            .field("size", &self.entry.size)
            .field("own_size", &self.entry.own_size)
            .field("child_count", &self.entry.children.len())
            .field("is_dir", &self.entry.is_dir)
            .field("ext", &self.entry.ext)
            .field("file_count", &self.entry.file_count)
            .field("dir_count", &self.entry.dir_count)
            .field("modified_time", &self.entry.modified_time)
            .field("rect", &self.entry.rect.get())
            .field("lod_expand", &self.entry.lod_expand)
            .finish()
    }
}

struct DirEntryDebugTree<'a>(&'a DirEntry);

impl std::fmt::Debug for DirEntryDebugTree<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut pending = vec![(self.0, 0usize)];
        let mut entries = formatter.debug_list();
        while let Some((entry, depth)) = pending.pop() {
            entries.entry(&DirEntryDebugNode { entry, depth });
            let child_depth = depth.saturating_add(1);
            pending.extend(
                entry
                    .children
                    .iter()
                    .rev()
                    .map(|child| (child, child_depth)),
            );
        }
        entries.finish()
    }
}

impl std::fmt::Debug for DirEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("DirEntry")
            .field(&DirEntryDebugTree(self))
            .finish()
    }
}

/// Pre-order immutable traversal of a directory tree.
pub struct DirEntryIter<'a> {
    pending: Vec<&'a DirEntry>,
}

impl<'a> Iterator for DirEntryIter<'a> {
    type Item = &'a DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.pending.pop()?;
        self.pending.extend(entry.children.iter().rev());
        Some(entry)
    }
}

/// Post-order immutable traversal of a directory tree.
pub struct DirEntryPostOrderIter<'a> {
    pending: Vec<(&'a DirEntry, bool)>,
}

impl<'a> Iterator for DirEntryPostOrderIter<'a> {
    type Item = &'a DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((entry, visited)) = self.pending.pop() {
            if visited {
                return Some(entry);
            }
            self.pending.push((entry, true));
            self.pending
                .extend(entry.children.iter().rev().map(|child| (child, false)));
        }
        None
    }
}

impl Clone for DirEntry {
    fn clone(&self) -> Self {
        struct CloneFrame<'a> {
            source: &'a DirEntry,
            clone: DirEntry,
            next_child: usize,
        }

        let mut frames = vec![CloneFrame {
            source: self,
            clone: self.clone_without_children(),
            next_child: 0,
        }];

        loop {
            let next_child = {
                let frame = frames
                    .last_mut()
                    .expect("clone traversal always has a root");
                if frame.next_child < frame.source.children.len() {
                    let child = &frame.source.children[frame.next_child];
                    frame.next_child += 1;
                    Some(child)
                } else {
                    None
                }
            };

            if let Some(child) = next_child {
                frames.push(CloneFrame {
                    source: child,
                    clone: child.clone_without_children(),
                    next_child: 0,
                });
                continue;
            }

            let completed = frames
                .pop()
                .expect("clone traversal always completes an existing frame")
                .clone;
            if let Some(parent) = frames.last_mut() {
                parent.clone.children.push(completed);
            } else {
                return completed;
            }
        }
    }
}

impl Drop for DirEntry {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(mut entry) = pending.pop() {
            pending.append(&mut entry.children);
        }
    }
}

impl DirEntry {
    fn clone_without_children(&self) -> Self {
        Self {
            name: self.name.clone(),
            path: self.path.clone(),
            size: self.size,
            own_size: self.own_size,
            children: Vec::with_capacity(self.children.len()),
            is_dir: self.is_dir,
            ext: self.ext.clone(),
            file_count: self.file_count,
            dir_count: self.dir_count,
            modified_time: self.modified_time,
            rect: Cell::new(self.rect.get()),
            lod_expand: self.lod_expand.clone(),
        }
    }

    /// Visit every node in pre-order without using the call stack.
    pub fn iter(&self) -> DirEntryIter<'_> {
        DirEntryIter {
            pending: vec![self],
        }
    }

    /// Visit every node in post-order without using the call stack.
    pub fn iter_post_order(&self) -> DirEntryPostOrderIter<'_> {
        DirEntryPostOrderIter {
            pending: vec![(self, false)],
        }
    }

    /// Mutate every node in pre-order without using the call stack.
    pub fn for_each_mut(&mut self, mut visitor: impl FnMut(&mut Self)) {
        let mut pending = vec![self as *mut Self];
        while let Some(entry_ptr) = pending.pop() {
            // SAFETY: every pointer comes from a unique node in this exclusively borrowed tree.
            // The visitor borrow ends before child pointers are collected, and each node is visited once.
            let entry = unsafe { &mut *entry_ptr };
            visitor(entry);
            pending.extend(
                entry
                    .children
                    .iter_mut()
                    .rev()
                    .map(|child| child as *mut Self),
            );
        }
    }

    /// Mutate every node in post-order without using the call stack.
    pub fn for_each_post_order_mut(&mut self, mut visitor: impl FnMut(&mut Self)) {
        let mut pending = vec![(self as *mut Self, false)];
        while let Some((entry_ptr, visited)) = pending.pop() {
            if visited {
                // SAFETY: children are visited before their parent. No pointer is duplicated, and
                // structural mutation of this node cannot invalidate any pointer still pending.
                visitor(unsafe { &mut *entry_ptr });
                continue;
            }

            // SAFETY: the pointer belongs to the exclusively borrowed tree and is only observed
            // here to enqueue its unique children before the node is mutably visited.
            let entry = unsafe { &mut *entry_ptr };
            pending.push((entry_ptr, true));
            pending.extend(
                entry
                    .children
                    .iter_mut()
                    .rev()
                    .map(|child| (child as *mut Self, false)),
            );
        }
    }

    /// Sort direct children by total size descending (treemap / filtered views).
    pub fn sort_children_by_size_desc(&mut self) {
        self.children
            .sort_unstable_by_key(|c| std::cmp::Reverse(c.size));
    }

    pub fn new_file(
        name: String,
        path: PathBuf,
        size: u64,
        ext: String,
        modified_time: Option<u64>,
    ) -> Self {
        Self {
            name,
            path,
            size,
            own_size: size,
            children: Vec::new(),
            is_dir: false,
            ext,
            file_count: 1,
            dir_count: 0,
            modified_time,
            rect: default_rect(),
            lod_expand: None,
        }
    }

    pub fn new_dir(name: String, path: PathBuf) -> Self {
        Self {
            name,
            path,
            size: 0,
            own_size: 0,
            children: Vec::new(),
            is_dir: true,
            ext: String::new(),
            file_count: 0,
            dir_count: 0,
            modified_time: None,
            rect: default_rect(),
            lod_expand: None,
        }
    }

    /// Sort children by size descending (required for treemap layout).
    pub fn sort_by_size(&mut self) {
        self.for_each_mut(Self::sort_children_by_size_desc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deep_tree(depth: usize) -> DirEntry {
        let mut node = DirEntry::new_file(
            "leaf".to_string(),
            PathBuf::from("leaf"),
            1,
            String::new(),
            None,
        );
        for index in (0..depth).rev() {
            let mut parent = DirEntry::new_dir(
                format!("dir-{index}"),
                PathBuf::from(format!("dir-{index}")),
            );
            parent.children.push(node);
            node = parent;
        }
        node
    }

    #[test]
    fn traversals_clone_and_drop_do_not_use_call_stack() {
        const DEPTH: usize = 20_000;
        let mut tree = deep_tree(DEPTH);

        assert_eq!(tree.iter().count(), DEPTH + 1);
        assert_eq!(tree.iter_post_order().count(), DEPTH + 1);

        let mut visited = 0usize;
        tree.for_each_mut(|_| visited += 1);
        assert_eq!(visited, DEPTH + 1);

        let clone = tree.clone();
        assert_eq!(clone.iter().count(), DEPTH + 1);

        tree.sort_by_size();
        drop(clone);
        drop(tree);
    }

    #[test]
    fn debug_is_iterative_and_complete() {
        const DEPTH: usize = 4_096;
        let tree = deep_tree(DEPTH);

        let debug = format!("{tree:?}");

        assert_eq!(debug.matches("DirEntryNode").count(), DEPTH + 1);
        assert!(debug.contains(&format!("depth: {DEPTH}")));
        assert!(debug.contains("name: \"leaf\""));
    }

    #[test]
    fn post_order_visits_children_before_parent() {
        let mut root = DirEntry::new_dir("root".to_string(), PathBuf::from("root"));
        root.children.push(DirEntry::new_file(
            "file".to_string(),
            PathBuf::from("root/file"),
            1,
            String::new(),
            None,
        ));

        let names: Vec<_> = root
            .iter_post_order()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["file", "root"]);

        let mut mutable_names = Vec::new();
        root.for_each_post_order_mut(|entry| mutable_names.push(entry.name.clone()));
        assert_eq!(mutable_names, ["file", "root"]);
    }
}
