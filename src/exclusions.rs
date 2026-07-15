//! Per-root path exclusions keyed by canonical scan identity.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use log::info;
use serde::{Deserialize, Serialize};

use crate::path_key::{self, ScanRoot};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Exclusions {
    #[serde(default)]
    pub root_id: String,
    pub scan_path: String,
    pub paths: HashSet<PathBuf>,
}

impl Exclusions {
    pub fn new(root: &ScanRoot) -> Self {
        Self {
            root_id: root.id().to_owned(),
            scan_path: root.display().to_owned(),
            paths: HashSet::new(),
        }
    }

    pub fn add(&mut self, path: &Path) {
        self.paths.insert(path.to_path_buf());
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.remove(path);
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.paths.contains(path)
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

    pub fn sorted_list(&self) -> Vec<String> {
        let mut list: Vec<_> = self
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        list.sort();
        list
    }
}

fn exclusions_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "squarebob-rs")
        .map(|dirs| dirs.data_dir().join("exclusions"))
}

fn exclusions_path(root: &ScanRoot) -> Option<PathBuf> {
    exclusions_dir().map(|dir| dir.join(format!("{}.json", root.id())))
}

fn legacy_exclusions_path(root: &ScanRoot) -> Option<PathBuf> {
    exclusions_dir().map(|dir| {
        dir.join(format!(
            "{}.json",
            path_key::legacy_scan_path_id_hex(root.display())
        ))
    })
}

pub fn load(root: &ScanRoot) -> anyhow::Result<Exclusions> {
    let Some(current_path) = exclusions_path(root) else {
        anyhow::bail!("could not determine exclusions directory");
    };
    let legacy_path = legacy_exclusions_path(root);
    let (path, legacy) = if current_path.exists() {
        (current_path, false)
    } else if let Some(path) = legacy_path.filter(|path| path.exists()) {
        (path, true)
    } else {
        return Ok(Exclusions::new(root));
    };

    let contents = fs::read_to_string(&path)?;
    let mut exclusions: Exclusions = serde_json::from_str(&contents)?;
    if exclusions.root_id.is_empty() {
        let envelope_root = ScanRoot::from_input(&exclusions.scan_path)?;
        if !envelope_root.same_identity(root) {
            anyhow::bail!("legacy exclusions root does not match requested root");
        }
        exclusions.root_id = root.id().to_owned();
        exclusions.scan_path = root.display().to_owned();
    }
    validate(root, &exclusions)?;

    if legacy {
        let outcome = save(&exclusions)?;
        if outcome.is_durable() {
            fs::remove_file(&path)?;
        } else if let Some(warning) = outcome.warning() {
            log::warn!("{warning}; preserving legacy exclusions backup {path:?}");
        }
    }
    Ok(exclusions)
}

pub fn save(exclusions: &Exclusions) -> anyhow::Result<crate::atomic_file::WriteOutcome> {
    let root = ScanRoot::from_input(&exclusions.scan_path)
        .map_err(|error| anyhow::anyhow!("invalid exclusions root: {error:#}"))?;
    validate(&root, exclusions)?;

    let Some(path) = exclusions_path(&root) else {
        anyhow::bail!("could not determine exclusions directory");
    };
    let json = serde_json::to_vec_pretty(exclusions)?;
    let outcome = crate::atomic_file::write(&path, &json)?;
    if let Some(warning) = outcome.warning() {
        log::warn!("{warning}");
    }
    info!("Saved {} exclusions", exclusions.len());
    Ok(outcome)
}

fn validate(root: &ScanRoot, exclusions: &Exclusions) -> anyhow::Result<()> {
    if exclusions.root_id != root.id() {
        anyhow::bail!("exclusions root identity does not match requested root");
    }
    if exclusions.scan_path != root.display() {
        anyhow::bail!("exclusions display root does not match canonical scan root");
    }
    if exclusions
        .paths
        .iter()
        .any(|path| !path.starts_with(root.path()))
    {
        anyhow::bail!("exclusions file contains a path outside the scan root");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_uses_native_paths() {
        let root = ScanRoot::from_input(".").expect("current directory must resolve");
        let mut exclusions = Exclusions::new(&root);
        let path = root.path().join("child");
        exclusions.add(&path);
        assert!(exclusions.contains(&path));
        exclusions.remove(&path);
        assert!(!exclusions.contains(&path));
    }
}
