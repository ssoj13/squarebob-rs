//! Built-in defaults — the six materials a fresh scene opens with.

use glam::Vec3;
use standard_surface::StandardSurfaceParams;

use crate::library::MaterialLibrary;
use crate::material::Material;

/// A blank-slate library of six prebuilt materials covering the
/// common BSDF lobes (matte, plastic, metal, glass, emissive,
/// translucent). Each uses `StandardSurfaceParams` helper
/// constructors; no per-cube variance is set so the defaults are
/// uniform until the user dials a spread on individual attributes.
pub fn default_library() -> MaterialLibrary {
    MaterialLibrary {
        materials: vec![
            matte_white(),
            glossy_plastic(Vec3::new(0.15, 0.55, 0.85)),
            brushed_gold(),
            clear_glass(),
            neon_cyan(),
            polished_marble(),
        ],
        active: 0,
    }
}

fn matte_white() -> Material {
    let mut p = StandardSurfaceParams::diffuse(Vec3::new(0.8, 0.8, 0.8));
    p.params1.x = 0.5; // diffuse_roughness — fall back from Oren-Nayar fudge factor
    Material::new("Matte White", p)
}

fn glossy_plastic(color: Vec3) -> Material {
    let mut p = StandardSurfaceParams::plastic(color, 0.25);
    p.coat_color_weight.w = 0.6; // light coat for the glossy look
    p.params2.y = 0.05; // coat roughness
    Material::new("Glossy Plastic", p)
}

fn brushed_gold() -> Material {
    Material::new(
        "Brushed Gold",
        StandardSurfaceParams::metal(Vec3::new(1.0, 0.78, 0.34), 0.35),
    )
}

fn clear_glass() -> Material {
    Material::new(
        "Clear Glass",
        StandardSurfaceParams::glass(Vec3::new(0.97, 0.99, 1.0), 1.52),
    )
}

fn neon_cyan() -> Material {
    Material::new(
        "Neon Cyan",
        StandardSurfaceParams::emissive(Vec3::new(0.15, 0.95, 1.0), 8.0),
    )
}

fn polished_marble() -> Material {
    let mut p = StandardSurfaceParams::default();
    p.base_color_weight = glam::Vec4::new(0.95, 0.93, 0.88, 1.0);
    p.params1.z = 0.18; // specular roughness — gentle sheen
    p.subsurface_color_weight = glam::Vec4::new(0.95, 0.92, 0.85, 0.15);
    Material::new("Polished Marble", p)
}
