/// Exclusion list: per-location path exclusions stored as JSON.
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use log::{info, warn};
use serde::{Deserialize, Serialize};

use crate::path_key;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Exclusions {
    pub scan_path: String,
    pub paths: HashSet<String>,
}

impl Exclusions {
    pub fn new(scan_path: &str) -> Self {
        Self {
            scan_path: scan_path.to_string(),
            paths: HashSet::new(),
        }
    }

    pub fn add(&mut self, path: &std::path::Path) {
        self.paths.insert(path.to_string_lossy().to_string());
    }

    pub fn remove(&mut self, path: &std::path::Path) {
        self.paths.remove(&path.to_string_lossy().to_string());
    }

    pub fn contains(&self, path: &std::path::Path) -> bool {
        self.paths.contains(&path.to_string_lossy().to_string())
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Get sorted list of excluded paths for display
    pub fn sorted_list(&self) -> Vec<String> {
        let mut list: Vec<_> = self.paths.iter().cloned().collect();
        list.sort();
        list
    }
}

fn exclusions_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "squarebob-rs")
        .map(|dirs| dirs.data_dir().join("exclusions"))
}

fn exclusions_path(scan_path: &str) -> Option<PathBuf> {
    exclusions_dir().map(|dir| dir.join(format!("{}.json", path_key::scan_path_id_hex(scan_path))))
}

pub fn load(scan_path: &str) -> Exclusions {
    let Some(path) = exclusions_path(scan_path) else {
        return Exclusions::new(scan_path);
    };

    if !path.exists() {
        return Exclusions::new(scan_path);
    }

    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
            warn!("Failed to parse exclusions: {}", e);
            Exclusions::new(scan_path)
        }),
        Err(_) => Exclusions::new(scan_path),
    }
}

pub fn save(exclusions: &Exclusions) {
    let Some(path) = exclusions_path(&exclusions.scan_path) else {
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    match serde_json::to_string_pretty(exclusions) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, json) {
                warn!("Failed to save exclusions: {}", e);
            } else {
                info!("Saved {} exclusions", exclusions.len());
            }
        }
        Err(e) => warn!("Failed to serialize exclusions: {}", e),
    }
}
