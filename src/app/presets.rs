//! Render presets: save/load render settings as named presets.
//!
//! Presets are stored as JSON files in the app data directory under `presets/`.

use directories::ProjectDirs;
use render_shared::Render3DOptions;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A render preset containing all render settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPreset {
    pub name: String,
    pub render_3d: Render3DOptions,
}

/// Get presets directory path
pub fn presets_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "dirstat-rs").map(|dirs| dirs.data_local_dir().join("presets"))
}

/// Ensure presets directory exists
pub fn ensure_presets_dir() -> std::io::Result<PathBuf> {
    let dir = presets_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Cannot find app data directory",
        )
    })?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Load all presets from disk
pub fn load_all_presets() -> HashMap<String, RenderPreset> {
    let mut presets = HashMap::new();

    let Some(dir) = presets_dir() else {
        log::warn!("Cannot find presets directory");
        return presets;
    };

    if !dir.exists() {
        return presets;
    }

    let Ok(entries) = std::fs::read_dir(&dir) else {
        log::warn!("Cannot read presets directory");
        return presets;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            match load_preset_from_file(&path) {
                Ok(preset) => {
                    log::debug!("Loaded preset: {}", preset.name);
                    presets.insert(preset.name.clone(), preset);
                }
                Err(e) => {
                    log::warn!("Failed to load preset {:?}: {}", path, e);
                }
            }
        }
    }

    log::info!("Loaded {} presets", presets.len());
    presets
}

/// Load a single preset from file
fn load_preset_from_file(path: &PathBuf) -> Result<RenderPreset, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let preset: RenderPreset = serde_json::from_str(&content)?;
    Ok(preset)
}

/// Save a preset to disk
pub fn save_preset(preset: &RenderPreset) -> std::io::Result<PathBuf> {
    let dir = ensure_presets_dir()?;
    let filename = sanitize_filename(&preset.name);
    let path = dir.join(format!("{}.json", filename));

    let json = serde_json::to_string_pretty(preset)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    std::fs::write(&path, json)?;
    log::info!("Saved preset '{}' to {:?}", preset.name, path);
    Ok(path)
}

/// Delete a preset from disk
pub fn delete_preset(name: &str) -> std::io::Result<()> {
    let dir = presets_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Cannot find presets directory",
        )
    })?;
    let filename = sanitize_filename(name);
    let path = dir.join(format!("{}.json", filename));

    if path.exists() {
        std::fs::remove_file(&path)?;
        log::info!("Deleted preset '{}' from {:?}", name, path);
    }
    Ok(())
}

/// Sanitize preset name for use as filename
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Create a preset from current render settings
pub fn create_preset(name: &str, render_3d: &Render3DOptions) -> RenderPreset {
    RenderPreset {
        name: name.to_string(),
        render_3d: render_3d.clone(),
    }
}
