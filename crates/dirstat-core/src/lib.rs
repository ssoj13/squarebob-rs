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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,        // total recursive size
    /// Per-file size on disk; for directories typically 0. Used by 3D height cues and NTFS fill pass.
    pub own_size: u64,
    pub children: Vec<DirEntry>,
    pub is_dir: bool,
    pub ext: String,      // lowercase extension for color mapping
    pub file_count: u64,
    pub dir_count: u64,
    /// Modified time as Unix timestamp (seconds since epoch)
    #[serde(default)]
    pub modified_time: Option<u64>,
    /// Layout rect (x, y, w, h) set by treemap via interior mutability
    #[serde(skip, default = "default_rect")]
    pub rect: Cell<[f32; 4]>,
    /// Set on collapsed LoD synthetic leaves; used to expand into per-file children on zoom.
    #[serde(default)]
    pub lod_expand: Option<LodExpandInfo>,
}

fn default_rect() -> Cell<[f32; 4]> {
    Cell::new([0.0; 4])
}

impl DirEntry {
    /// Sort direct children by total size descending (treemap / filtered views).
    pub fn sort_children_by_size_desc(&mut self) {
        self.children
            .sort_unstable_by_key(|c| std::cmp::Reverse(c.size));
    }

    pub fn new_file(name: String, path: PathBuf, size: u64, ext: String, modified_time: Option<u64>) -> Self {
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
            rect: Cell::new([0.0; 4]),
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
            rect: Cell::new([0.0; 4]),
            lod_expand: None,
        }
    }

    /// Sort children by size descending (required for treemap layout)
    pub fn sort_by_size(&mut self) {
        self.sort_children_by_size_desc();
        for child in &mut self.children {
            if child.is_dir {
                child.sort_by_size();
            }
        }
    }
}
