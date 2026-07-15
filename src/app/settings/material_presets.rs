//! Factory preset library for the material `playa_ae::PresetBank`.
//!
//! Ships ~30 hand-tuned `StandardSurfaceParams` looks grouped by
//! category — Plastic, Metal, Glass, Stone, Cloth, Paper, Emissive,
//! Organic. Names use `Group / Name` so the playa-ae preset menu
//! buckets them into sub-menus automatically.
//!
//! On app start we read `{config_dir}/material_presets.json`; if
//! that file is missing or fails to parse we re-seed from
//! [`factory_material_preset_bank`]. Every time the user saves /
//! renames / removes a preset through the AE button we write the
//! whole bank back. Deleting the JSON file resets to factory.

use std::path::PathBuf;

use glam::Vec4;
use playa_ae::{AttrValue, Attrs, PresetBank};
use pt_material::StandardSurfaceParams;

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
/// factory bank. Bad JSON is logged and treated as missing so the
/// app never refuses to start over a malformed presets file.
pub fn load_preset_bank() -> PresetBank {
    let Some(path) = preset_file_path() else {
        return factory_material_preset_bank();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return factory_material_preset_bank();
    };
    match serde_json::from_slice::<PresetBank>(&bytes) {
        Ok(bank) => bank,
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

/// Build the factory bank — 30 grouped material looks.
pub fn factory_material_preset_bank() -> PresetBank {
    let mut bank = PresetBank::default();
    let key = material_schema_key();
    for (name, params) in factory_presets() {
        let attrs = material_to_attrs_pub(&params);
        bank.save(key, name, &attrs);
    }
    bank
}

/// `Attrs` built directly from category + raw fields, useful when
/// caller wants to push a preset into the bank without going
/// through `StandardSurfaceParams`. Currently unused but kept as
/// a stable extension point.
#[allow(dead_code)]
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

// ============================================================================
// 30 factory materials, grouped by category. Names use `Group / Leaf`
// so playa-ae's preset menu nests them into sub-menus automatically.
// ============================================================================

fn factory_presets() -> Vec<(&'static str, StandardSurfaceParams)> {
    vec![
        // ---- Plastics ----------------------------------------------------
        ("Plastic / Red Glossy", plastic_glossy([0.95, 0.10, 0.10])),
        ("Plastic / Blue Matte", plastic_matte([0.10, 0.30, 0.85])),
        (
            "Plastic / Yellow Glossy",
            plastic_glossy([0.95, 0.85, 0.05]),
        ),
        (
            "Plastic / Green Translucent",
            plastic_translucent([0.20, 0.80, 0.40]),
        ),
        ("Plastic / Black Rubber", rubber()),
        // ---- Metals ------------------------------------------------------
        (
            "Metal / Brushed Gold",
            metal_brushed([1.0, 0.78, 0.34], 0.32),
        ),
        (
            "Metal / Polished Chrome",
            metal_polished([0.95, 0.95, 0.97], 0.04),
        ),
        ("Metal / Copper", metal_polished([0.95, 0.55, 0.30], 0.18)),
        (
            "Metal / Brushed Aluminum",
            metal_brushed([0.91, 0.92, 0.94], 0.35),
        ),
        ("Metal / Iron", metal_polished([0.55, 0.55, 0.55], 0.25)),
        ("Metal / Brass", metal_polished([0.95, 0.80, 0.45], 0.15)),
        // ---- Glass -------------------------------------------------------
        ("Glass / Clear", glass_clear([0.98, 0.99, 0.99], 1.52)),
        (
            "Glass / Frosted",
            glass_rough([0.98, 0.98, 0.98], 1.52, 0.4),
        ),
        ("Glass / Amber", glass_clear([0.95, 0.55, 0.20], 1.55)),
        ("Glass / Blue", glass_clear([0.20, 0.50, 0.95], 1.50)),
        (
            "Glass / Green Bottle",
            glass_clear([0.15, 0.65, 0.30], 1.51),
        ),
        // ---- Stone -------------------------------------------------------
        ("Stone / Polished Marble", marble([0.95, 0.93, 0.88], 0.18)),
        ("Stone / Granite", stone_rough([0.55, 0.55, 0.55], 0.7)),
        ("Stone / Slate", stone_rough([0.22, 0.22, 0.25], 0.6)),
        ("Stone / Sandstone", stone_rough([0.80, 0.65, 0.45], 0.85)),
        // ---- Cloth & Paper ----------------------------------------------
        ("Cloth / Velvet Red", velvet([0.65, 0.05, 0.10])),
        ("Cloth / Silk Cream", silk([0.95, 0.88, 0.72])),
        ("Cloth / Felt Grey", felt([0.45, 0.45, 0.45])),
        ("Paper / Matte White", matte_paper([0.92, 0.91, 0.88])),
        // ---- Emissive ----------------------------------------------------
        ("Emissive / Neon Cyan", emissive([0.15, 0.95, 1.0], 8.0)),
        ("Emissive / Hot Lava", lava()),
        ("Emissive / Bulb Warm", bulb([1.0, 0.85, 0.55], 6.0)),
        ("Emissive / Bulb Cool", bulb([0.85, 0.92, 1.0], 6.0)),
        // ---- Organic / SSS ----------------------------------------------
        ("Organic / Wax", wax([1.0, 0.85, 0.65])),
        ("Organic / Skin", skin([1.0, 0.78, 0.65])),
    ]
}

// ---- Construction helpers -------------------------------------------------

fn plastic_glossy(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        coat_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.35),
        params1: Vec4::new(0.0, 0.0, 0.12, 1.5),
        params2: Vec4::new(0.0, 0.04, 1.5, 1.0),
        ..Default::default()
    }
}

fn plastic_matte(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.5),
        params1: Vec4::new(0.4, 0.0, 0.6, 1.5),
        ..Default::default()
    }
}

fn plastic_translucent(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 0.5),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        transmission_color_weight: Vec4::new(c[0], c[1], c[2], 0.5),
        params1: Vec4::new(0.0, 0.0, 0.2, 1.45),
        ..Default::default()
    }
}

fn rubber() -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(0.04, 0.04, 0.04, 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.2),
        params1: Vec4::new(0.6, 0.0, 0.7, 1.4),
        ..Default::default()
    }
}

fn metal_brushed(c: [f32; 3], roughness: f32) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        params1: Vec4::new(0.0, 1.0, roughness, 1.5),
        params2: Vec4::new(0.5, 0.1, 1.5, 1.0),
        ..Default::default()
    }
}

fn metal_polished(c: [f32; 3], roughness: f32) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        params1: Vec4::new(0.0, 1.0, roughness, 1.5),
        ..Default::default()
    }
}

fn glass_clear(c: [f32; 3], ior: f32) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        transmission_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        params1: Vec4::new(0.0, 0.0, 0.02, ior),
        ..Default::default()
    }
}

fn glass_rough(c: [f32; 3], ior: f32, roughness: f32) -> StandardSurfaceParams {
    let mut p = glass_clear(c, ior);
    p.params1.z = roughness;
    p
}

fn marble(c: [f32; 3], sss_weight: f32) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        subsurface_color_weight: Vec4::new(c[0] * 0.95, c[1] * 0.9, c[2] * 0.85, sss_weight),
        params1: Vec4::new(0.0, 0.0, 0.18, 1.5),
        ..Default::default()
    }
}

fn stone_rough(c: [f32; 3], roughness: f32) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.4),
        params1: Vec4::new(0.5, 0.0, roughness, 1.5),
        ..Default::default()
    }
}

fn velvet(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.4),
        params1: Vec4::new(0.9, 0.0, 0.6, 1.5),
        ..Default::default()
    }
}

fn silk(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.7),
        params1: Vec4::new(0.3, 0.0, 0.25, 1.5),
        params2: Vec4::new(0.4, 0.0, 1.5, 1.0),
        ..Default::default()
    }
}

fn felt(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.1),
        params1: Vec4::new(0.8, 0.0, 0.85, 1.5),
        ..Default::default()
    }
}

fn matte_paper(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
        params1: Vec4::new(0.5, 0.0, 0.5, 1.5),
        ..Default::default()
    }
}

fn emissive(c: [f32; 3], intensity: f32) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0], c[1], c[2], 0.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
        emission_color_weight: Vec4::new(c[0], c[1], c[2], intensity),
        ..Default::default()
    }
}

fn lava() -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(0.40, 0.06, 0.0, 0.3),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.2),
        emission_color_weight: Vec4::new(1.0, 0.30, 0.05, 4.0),
        params1: Vec4::new(0.6, 0.0, 0.7, 1.5),
        ..Default::default()
    }
}

fn bulb(c: [f32; 3], intensity: f32) -> StandardSurfaceParams {
    emissive(c, intensity)
}

fn wax(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0] * 0.85, c[1] * 0.8, c[2] * 0.7, 0.5),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.7),
        subsurface_color_weight: Vec4::new(c[0], c[1] * 0.6, c[2] * 0.45, 0.65),
        params1: Vec4::new(0.0, 0.0, 0.25, 1.45),
        ..Default::default()
    }
}

fn skin(c: [f32; 3]) -> StandardSurfaceParams {
    StandardSurfaceParams {
        base_color_weight: Vec4::new(c[0] * 0.9, c[1] * 0.85, c[2] * 0.75, 0.6),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.5),
        subsurface_color_weight: Vec4::new(c[0], c[1] * 0.5, c[2] * 0.35, 0.7),
        params1: Vec4::new(0.3, 0.0, 0.35, 1.4),
        ..Default::default()
    }
}
