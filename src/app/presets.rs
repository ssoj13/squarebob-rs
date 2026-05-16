//! Render presets: save/load render settings as named presets.
//!
//! Presets are stored as a single JSON file. Lookup order:
//! 1. `presets.json` next to the binary (portable / shipped overrides).
//! 2. `~/.squarebob/presets.json` (per-user). Created on first save.
//!
//! The built-in "defaults" preset is embedded from `data/default.json`
//! and is always present in the in-memory map: if the on-disk file is
//! missing the preset (or the file doesn't exist yet) we inject it.
//! Once a `presets.json` exists the user is free to overwrite or even
//! delete the "defaults" entry — on next load it will be re-injected
//! from the embedded copy.

use directories::BaseDirs;
use render_shared::Render3DOptions;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const BUNDLED_FACTORY_RENDER_3D_OPTIONS: &str = include_str!("../../data/default.json");
const ADJACENT_DEFAULT_PRESET_FILENAME: &str = "default.json";
const PRESETS_FILENAME: &str = "presets.json";
const USER_DIR_NAME: &str = ".squarebob";

/// Built-in preset name. Always materialized in the in-memory map; can
/// be overwritten by saving, and re-injected from the embedded copy if
/// missing on next load.
pub const DEFAULT_PRESET_NAME: &str = "defaults";

/// Default 3D/render options (immutable factory baseline).
///
/// Resolution order:
/// 1. `default.json` adjacent to the executable (portable override of
///    the *baseline*, before any user customization).
/// 2. Compiled-in `data/default.json` (always available).
///
/// This is the source for `apply_factory_render_defaults` (Reset button)
/// and the initial seed for the "defaults" preset.
#[inline]
pub fn factory_render_3d_options() -> Render3DOptions {
    static PARSED: OnceLock<Render3DOptions> = OnceLock::new();
    PARSED
        .get_or_init(|| {
            load_adjacent_default_render_3d_options()
                .unwrap_or_else(bundled_factory_render_3d_options)
        })
        .clone()
}

fn bundled_factory_render_3d_options() -> Render3DOptions {
    serde_json::from_str(BUNDLED_FACTORY_RENDER_3D_OPTIONS)
        .expect("data/default.json must match Render3DOptions schema")
}

fn load_adjacent_default_render_3d_options() -> Option<Render3DOptions> {
    let path = adjacent_default_preset_path()?;
    if !path.is_file() {
        return None;
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(options) => {
                log::info!("Loaded default render preset from {}", path.display());
                Some(options)
            }
            Err(e) => {
                log::error!(
                    "Failed to parse default render preset at {}: {}; using built-in defaults",
                    path.display(),
                    e
                );
                None
            }
        },
        Err(e) => {
            log::error!(
                "Failed to read default render preset at {}: {}; using built-in defaults",
                path.display(),
                e
            );
            None
        }
    }
}

fn adjacent_default_preset_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    default_preset_path_for_exe(&exe)
}

fn default_preset_path_for_exe(exe: &Path) -> Option<PathBuf> {
    exe.parent()
        .map(|dir| dir.join(ADJACENT_DEFAULT_PRESET_FILENAME))
}

/// A render preset containing all render settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPreset {
    pub name: String,
    pub render_3d: Render3DOptions,
}

/// On-disk container for all presets. Serialized to a single JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PresetsFile {
    presets: Vec<RenderPreset>,
}

/// Per-user presets path: `~/.squarebob/presets.json`.
fn user_presets_path() -> Option<PathBuf> {
    BaseDirs::new().map(|b| b.home_dir().join(USER_DIR_NAME).join(PRESETS_FILENAME))
}

/// Adjacent presets path: `<exe-dir>/presets.json`.
fn adjacent_presets_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(|d| d.join(PRESETS_FILENAME))
}

/// Resolve the on-disk presets path used for both read and write.
/// Adjacent-binary path wins if the file already exists (portable mode);
/// otherwise we use the per-user path.
fn resolved_presets_path() -> Option<PathBuf> {
    if let Some(adj) = adjacent_presets_path() {
        if adj.is_file() {
            return Some(adj);
        }
    }
    user_presets_path()
}

/// Materialize the built-in "defaults" preset in the map if missing.
fn ensure_builtin_default(presets: &mut HashMap<String, RenderPreset>) {
    if !presets.contains_key(DEFAULT_PRESET_NAME) {
        presets.insert(
            DEFAULT_PRESET_NAME.to_string(),
            RenderPreset {
                name: DEFAULT_PRESET_NAME.to_string(),
                render_3d: factory_render_3d_options(),
            },
        );
    }
}

/// Load all presets from disk into a `name -> preset` map.
///
/// Always ensures the embedded "defaults" preset is present in the map.
/// If neither adjacent nor user file exists, the map contains just
/// "defaults" (file is not created until the user saves).
pub fn load_all_presets() -> HashMap<String, RenderPreset> {
    let mut presets: HashMap<String, RenderPreset> = HashMap::new();

    let paths: Vec<PathBuf> = [adjacent_presets_path(), user_presets_path()]
        .into_iter()
        .flatten()
        .collect();

    for path in paths {
        if !path.is_file() {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<PresetsFile>(&content) {
                Ok(file) => {
                    for preset in file.presets {
                        presets.insert(preset.name.clone(), preset);
                    }
                    log::info!(
                        "Loaded {} presets from {}",
                        presets.len(),
                        path.display()
                    );
                    break;
                }
                Err(e) => log::warn!("Failed to parse {}: {}", path.display(), e),
            },
            Err(e) => log::warn!("Failed to read {}: {}", path.display(), e),
        }
    }

    ensure_builtin_default(&mut presets);
    presets
}

/// Persist the full preset map to disk. Path is chosen by
/// `resolved_presets_path` (adjacent file wins if present, else
/// per-user `~/.squarebob/presets.json`). Parent dirs are created.
pub fn save_all_presets(presets: &HashMap<String, RenderPreset>) -> std::io::Result<PathBuf> {
    let path = resolved_presets_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Cannot determine presets file location",
        )
    })?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut list: Vec<RenderPreset> = presets.values().cloned().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    let file = PresetsFile { presets: list };

    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)?;

    log::info!(
        "Saved {} presets to {}",
        file.presets.len(),
        path.display()
    );
    Ok(path)
}

/// Create a preset from current render settings
pub fn create_preset(name: &str, render_3d: &Render3DOptions) -> RenderPreset {
    RenderPreset {
        name: name.to_string(),
        render_3d: render_3d.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_render_json_parses_and_matches_motion_flags() {
        let o = factory_render_3d_options();
        assert!(
            !o.animate,
            "bundled factory preset keeps cube motion off by default"
        );
        assert!(
            !o.env_animate,
            "bundled factory preset keeps env rotation off by default"
        );
        assert_eq!(o.animation_time, 0.0);
        assert_eq!(o.env_time, 0.0);
    }

    #[test]
    fn adjacent_default_preset_path_uses_executable_directory() {
        let exe = Path::new("bin").join("squarebob");
        assert_eq!(
            default_preset_path_for_exe(&exe),
            Some(Path::new("bin").join("default.json"))
        );
    }

    #[test]
    fn builtin_default_is_injected_when_missing() {
        let mut map: HashMap<String, RenderPreset> = HashMap::new();
        ensure_builtin_default(&mut map);
        assert!(map.contains_key(DEFAULT_PRESET_NAME));
    }

    #[test]
    fn builtin_default_is_not_overwritten_when_present() {
        let mut map: HashMap<String, RenderPreset> = HashMap::new();
        let mut custom = factory_render_3d_options();
        custom.animate = true; // distinguishable from bundled.
        map.insert(
            DEFAULT_PRESET_NAME.to_string(),
            RenderPreset {
                name: DEFAULT_PRESET_NAME.to_string(),
                render_3d: custom,
            },
        );
        ensure_builtin_default(&mut map);
        assert!(map[DEFAULT_PRESET_NAME].render_3d.animate);
    }
}
