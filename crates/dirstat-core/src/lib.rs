use std::cell::Cell;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A node in the directory tree.
/// `rect` uses Cell for interior mutability - treemap layout sets rects
/// without requiring &mut, eliminating the need to clone the tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,        // total recursive size
    #[allow(dead_code)]   // used by NTFS scanner on Windows
    pub own_size: u64,    // file: file size, dir: 0
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
}

fn default_rect() -> Cell<[f32; 4]> {
    Cell::new([0.0; 4])
}

impl DirEntry {
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
        }
    }

    /// Sort children by size descending (required for treemap layout)
    pub fn sort_by_size(&mut self) {
        self.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
        for child in &mut self.children {
            if child.is_dir {
                child.sort_by_size();
            }
        }
    }
}
