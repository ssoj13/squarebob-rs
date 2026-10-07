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
use std::io::Write;
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
    if let Some(adj) = adjacent_presets_path()
        && adj.is_file()
    {
        return Some(adj);
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
                    log::info!("Loaded {} presets from {}", presets.len(), path.display());
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

    save_presets_at(&path, presets)?;
    log::info!("Saved {} presets to {}", presets.len(), path.display());
    Ok(path)
}

fn save_presets_at(path: &Path, presets: &HashMap<String, RenderPreset>) -> std::io::Result<()> {
    let mut list: Vec<RenderPreset> = presets.values().cloned().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    let file = PresetsFile { presets: list };

    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut output = av_util_core::outfile::AtomicOut::create(path, true)?;
    output.file()?.write_all(json.as_bytes())?;
    output.commit()
}

/// Create a preset from current render settings
pub fn create_preset(name: &str, render_3d: &Render3DOptions) -> RenderPreset {
    RenderPreset {
        name: name.to_string(),
        render_3d: render_3d.clone(),
    }
}

pub(super) enum PresetChange<'a> {
    Save { name: &'a str, create: bool },
    Rename { from: &'a str, to: &'a str },
    Delete(&'a str),
}

pub(super) fn change_presets(
    presets: &mut HashMap<String, RenderPreset>,
    change: PresetChange<'_>,
    current: &Render3DOptions,
    persist: impl FnOnce(&HashMap<String, RenderPreset>) -> Result<(), String>,
) -> Result<String, String> {
    let mut next = presets.clone();
    let name = match change {
        PresetChange::Save { name, create } => {
            let name = checked_name(name)?;
            if create && next.contains_key(name) {
                return Err(format!("Preset '{name}' already exists."));
            }
            next.insert(name.to_owned(), create_preset(name, current));
            name.to_owned()
        }
        PresetChange::Rename { from, to } => {
            let to = checked_name(to)?;
            if from != to && next.contains_key(to) {
                return Err(format!("Preset '{to}' already exists."));
            }
            let mut preset = next
                .remove(from)
                .ok_or_else(|| format!("Preset '{from}' no longer exists."))?;
            preset.name = to.to_owned();
            next.insert(to.to_owned(), preset);
            to.to_owned()
        }
        PresetChange::Delete(name) => {
            next.remove(name)
                .ok_or_else(|| format!("Preset '{name}' no longer exists."))?;
            name.to_owned()
        }
    };
    persist(&next)?;
    *presets = next;
    Ok(name)
}

fn checked_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        Err("Enter a preset name.".into())
    } else {
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_presets_atomic_file_roundtrip_replaces_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        std::fs::write(&path, b"old preset bytes").unwrap();
        let settings = factory_render_3d_options();
        let presets = HashMap::from([("Cinema".into(), create_preset("Cinema", &settings))]);
        save_presets_at(&path, &presets).unwrap();
        let file: PresetsFile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(file.presets.len(), 1);
        assert_eq!(file.presets[0].name, "Cinema");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_preset_publish_preserves_previous_file_and_cleans_temp() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        let old = b"old preset bytes must survive a failed replacement";
        std::fs::write(&path, old).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let settings = factory_render_3d_options();
        let mut presets =
            HashMap::from([("Original".into(), create_preset("Original", &settings))]);
        assert!(
            change_presets(
                &mut presets,
                PresetChange::Save {
                    name: "New",
                    create: true
                },
                &settings,
                |candidate| {
                    save_presets_at(&path, candidate).map_err(|error| error.to_string())
                }
            )
            .is_err()
        );
        drop(lock);
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert_eq!(presets.len(), 1);
        assert!(presets.contains_key("Original"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn named_preset_changes_preserve_payload_and_serialized_identity() {
        let mut presets = HashMap::new();
        let mut settings = factory_render_3d_options();
        settings.animate = true;
        assert_eq!(
            change_presets(
                &mut presets,
                PresetChange::Save {
                    name: "  Cinema  ",
                    create: true
                },
                &settings,
                |_| Ok(())
            )
            .unwrap(),
            "Cinema"
        );
        change_presets(
            &mut presets,
            PresetChange::Rename {
                from: "Cinema",
                to: "  Final  ",
            },
            &factory_render_3d_options(),
            |candidate| {
                let json = serde_json::to_string(&PresetsFile {
                    presets: candidate.values().cloned().collect(),
                })
                .unwrap();
                let decoded: PresetsFile = serde_json::from_str(&json).unwrap();
                assert_eq!(decoded.presets[0].name, "Final");
                assert!(decoded.presets[0].render_3d.animate);
                Ok(())
            },
        )
        .unwrap();
        assert!(!presets.contains_key("Cinema"));
        assert!(presets["Final"].render_3d.animate);
    }

    #[test]
    fn failed_preset_persistence_preserves_map_and_reports_error() {
        let settings = factory_render_3d_options();
        let mut presets =
            HashMap::from([("Original".into(), create_preset("Original", &settings))]);
        for change in [
            PresetChange::Save {
                name: "New",
                create: true,
            },
            PresetChange::Rename {
                from: "Original",
                to: "New",
            },
            PresetChange::Delete("Original"),
        ] {
            assert_eq!(
                change_presets(&mut presets, change, &settings, |_| Err(
                    "read-only destination".into()
                ))
                .unwrap_err(),
                "read-only destination"
            );
            assert_eq!(presets.len(), 1);
            assert_eq!(presets["Original"].name, "Original");
        }
    }

    #[test]
    fn preset_name_collisions_and_empty_names_do_not_overwrite() {
        let settings = factory_render_3d_options();
        let mut presets = HashMap::from([
            ("One".into(), create_preset("One", &settings)),
            ("Two".into(), create_preset("Two", &settings)),
        ]);
        for change in [
            PresetChange::Save {
                name: " One ",
                create: true,
            },
            PresetChange::Save {
                name: "  ",
                create: true,
            },
            PresetChange::Rename {
                from: "One",
                to: " Two ",
            },
        ] {
            assert!(
                change_presets(&mut presets, change, &settings, |_| panic!(
                    "invalid changes must not reach disk"
                ))
                .is_err()
            );
            assert_eq!(presets.len(), 2);
        }
    }

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
