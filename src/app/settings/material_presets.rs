//! Factory preset library for the material `playa_ae::PresetBank`.
//!
//! Sources all 86 curated `StandardSurfaceParams` looks directly from
//! [`pt_material::curated_presets`] (Single Source of Truth shared with
//! `render-rs` and WarpBro), grouped by category — `Plastic`, `Metal`,
//! `CarPaint`, `Glass`, `Ceramic`, `Stone`, `Wood`, `Leather`, `Fabric`,
//! `Rubber`, `Paper`, `Emissive`, `Organic`. Names use `Category / Name`
//! so the `playa-ae` preset menu buckets them into sub-menus automatically.
//!
//! On app start we read `{config_dir}/material_presets.json`; if that file
//! is missing or fails to parse we seed from [`factory_material_preset_bank`],
//! and if an existing file is loaded we ensure all factory presets are present
//! alongside any user-saved custom presets.

use std::path::PathBuf;

use glam::Vec4;
use playa_ae::{AttrValue, Attrs, PresetBank};

use super::materials::{material_schema_name, material_to_attrs_pub};

const PRESET_FILE: &str = "material_presets.json";

/// Stable schema key used by both the AE renderer and the preset
/// bank. Lives here so other callers can index the same bank from
/// non-AE entry points without a second source of truth.
pub fn material_schema_key() -> &'static str {
    material_schema_name()
}

/// Persistent storage location for the user-edited bank. `None`
/// when the platform doesn't expose a config dir (CI sandboxes,
/// stripped-down OSes). Callers fall back to factory-only in that
/// case.
pub fn preset_file_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "squarebob-rs").map(|d| d.config_dir().join(PRESET_FILE))
}

/// Load the user-customised bank from disk, or fall back to the
/// factory bank. Missing factory presets (e.g. from the WarpBro
/// catalog expansion) are merged in without overwriting user presets.
pub fn load_preset_bank() -> PresetBank {
    let Some(path) = preset_file_path() else {
        return factory_material_preset_bank();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return factory_material_preset_bank();
    };
    match serde_json::from_slice::<PresetBank>(&bytes) {
        Ok(mut bank) => {
            merge_factory_presets(&mut bank);
            bank
        }
        Err(e) => {
            log::warn!(
                "material_presets: failed to parse {} ({e}), re-seeding from factory",
                path.display()
            );
            factory_material_preset_bank()
        }
    }
}

/// Persist the bank atomically. Best-effort: writes a sibling
/// `*.tmp` then renames over the destination so a crash mid-write
/// can't truncate the file. Errors are logged, not propagated —
/// the in-memory bank stays the source of truth for the rest of
/// the session.
pub fn save_preset_bank(bank: &PresetBank) {
    let Some(path) = preset_file_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        log::warn!("material_presets: mkdir {} failed: {e}", parent.display());
        return;
    }
    let tmp = path.with_extension("json.tmp");
    let Ok(json) = serde_json::to_vec_pretty(bank) else {
        log::warn!("material_presets: serde_json::to_vec_pretty failed");
        return;
    };
    if let Err(e) = std::fs::write(&tmp, &json) {
        log::warn!("material_presets: write {} failed: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log::warn!(
            "material_presets: rename {} → {} failed: {e}",
            tmp.display(),
            path.display()
        );
    }
}

/// Build the factory bank from [`pt_material::curated_presets`].
pub fn factory_material_preset_bank() -> PresetBank {
    let mut bank = PresetBank::default();
    merge_factory_presets(&mut bank);
    bank
}

fn merge_factory_presets(bank: &mut PresetBank) {
    let key = material_schema_key();
    for preset in pt_material::curated_presets() {
        let menu_name = preset.menu_key();
        if !bank.contains(key, &menu_name) {
            let attrs = material_to_attrs_pub(&preset.params);
            bank.save(key, menu_name, &attrs);
        }
    }
}

/// `Attrs` built directly from category + raw fields, useful when
/// caller wants to push a preset into the bank without going
/// through `StandardSurfaceParams`. Currently unused but kept as
/// a stable extension point.
#[expect(
    dead_code,
    clippy::too_many_arguments,
    reason = "preset helper takes all 9 StandardSurface lobes directly"
)]
pub fn make_attrs_from_lobes(
    base: Vec4,
    specular: Vec4,
    transmission: Vec4,
    subsurface: Vec4,
    coat: Vec4,
    emission: Vec4,
    opacity: Vec4,
    params1: Vec4,
    params2: Vec4,
) -> Attrs {
    let mut a = Attrs::new();
    a.set("Base Color", AttrValue::Vec4(base.into()));
    a.set("Specular", AttrValue::Vec4(specular.into()));
    a.set("Transmission", AttrValue::Vec4(transmission.into()));
    a.set("Subsurface", AttrValue::Vec4(subsurface.into()));
    a.set("Coat", AttrValue::Vec4(coat.into()));
    a.set("Emission", AttrValue::Vec4(emission.into()));
    a.set("Opacity", AttrValue::Vec4(opacity.into()));
    a.set("Diffuse Roughness", AttrValue::Float(params1.x));
    a.set("Metalness", AttrValue::Float(params1.y));
    a.set("Specular Roughness", AttrValue::Float(params1.z));
    a.set("Specular IOR", AttrValue::Float(params1.w));
    a.set("Spec Anisotropy", AttrValue::Float(params2.x));
    a.set("Coat Roughness", AttrValue::Float(params2.y));
    a.set("Coat IOR", AttrValue::Float(params2.z));
    a
}
