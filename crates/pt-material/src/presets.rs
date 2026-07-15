//! Built-in defaults — the ten materials a fresh scene opens with.
//!
//! Each preset is written as a direct `StandardSurfaceParams`
//! struct-literal patch on `Default` so every per-attribute tweak
//! stays visually paired with its field. The `StandardSurfaceParams`
//! 4th channel on every colour Vec4 is a **lobe weight** (`0..1`),
//! not an alpha — see `crates/standard-surface/src/params.rs`.
//!
//! Coverage:
//! * one diffuse (Matte White, slightly warm)
//! * one glossy plastic with clear coat
//! * two metals (Brushed Gold and Polished Chrome — covers warm /
//!   cool, rough / mirror)
//! * two glasses (Clear and Amber Stained — covers neutral /
//!   chromatic transmission)
//! * two emissives (Neon Cyan and Hot Lava — covers cool / warm
//!   high-energy)
//! * one subsurface (Polished Marble)
//! * one Oren-Nayar–pronounced cloth (Soft Velvet)

use glam::Vec4;
use standard_surface::StandardSurfaceParams;

use crate::library::MaterialLibrary;
use crate::material::Material;

/// A blank-slate library of ten prebuilt materials covering the
/// common BSDF lobes and a few hero looks (chrome, stained glass,
/// hot-lava emissive). No per-cube variance is baked in — defaults
/// stay uniform until the user dials a spread on individual
/// attributes.
pub fn default_library() -> MaterialLibrary {
    MaterialLibrary {
        materials: vec![
            matte_white(),
            glossy_plastic_blue(),
            brushed_gold(),
            polished_chrome(),
            clear_glass(),
            amber_stained_glass(),
            neon_cyan(),
            hot_lava(),
            polished_marble(),
            soft_velvet_red(),
        ],
        active: 0,
    }
}

/// Warm paper-white diffuse. Specular fully muted so the look stays
/// flat — useful baseline material that pairs cleanly with any
/// background.
fn matte_white() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(0.92, 0.91, 0.88, 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
        params1: Vec4::new(0.5, 0.0, 0.5, 1.5), // Oren-Nayar diffuse
        ..StandardSurfaceParams::default()
    };
    Material::new("Matte White", p)
}

/// Vibrant blue plastic with a faint clear coat. The coat lobe
/// reads as the "wet" highlight on top of the slightly rough
/// dielectric body.
fn glossy_plastic_blue() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(0.10, 0.55, 0.95, 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        coat_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.35),
        params1: Vec4::new(0.0, 0.0, 0.12, 1.5),
        params2: Vec4::new(0.0, 0.04, 1.5, 1.0),
        ..StandardSurfaceParams::default()
    };
    Material::new("Glossy Plastic Blue", p)
}

/// Warm gold with a brushed roughness band. `params2.x` is
/// anisotropy — non-zero so the highlight elongates along the brush
/// direction the renderer applies in tangent space.
fn brushed_gold() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(1.0, 0.78, 0.34, 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        params1: Vec4::new(0.0, 1.0, 0.32, 1.5), // metalness, roughness
        params2: Vec4::new(0.5, 0.1, 1.5, 1.0),  // anisotropy
        ..StandardSurfaceParams::default()
    };
    Material::new("Brushed Gold", p)
}

/// Mirror-finish chrome with a slight cool tint. Almost-zero
/// roughness gives crisp environment reflections.
fn polished_chrome() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(0.95, 0.95, 0.97, 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        params1: Vec4::new(0.0, 1.0, 0.04, 1.5),
        params2: Vec4::new(0.0, 0.0, 1.5, 1.0),
        ..StandardSurfaceParams::default()
    };
    Material::new("Polished Chrome", p)
}

/// Clean window glass. Base lobe disabled — colour comes entirely
/// from the transmission lobe, which lets the env map / background
/// show through.
fn clear_glass() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        transmission_color_weight: Vec4::new(0.98, 0.99, 0.99, 1.0),
        params1: Vec4::new(0.0, 0.0, 0.02, 1.52),
        ..StandardSurfaceParams::default()
    };
    Material::new("Clear Glass", p)
}

/// Tinted glass — orange amber, like a beer bottle. Slightly
/// rougher than the clear variant so the absorption read is more
/// volumetric.
fn amber_stained_glass() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        transmission_color_weight: Vec4::new(0.95, 0.55, 0.20, 1.0),
        params1: Vec4::new(0.0, 0.0, 0.05, 1.55),
        ..StandardSurfaceParams::default()
    };
    Material::new("Amber Stained Glass", p)
}

/// Cyan neon — high-intensity emission with no diffuse / specular
/// contribution so it reads as pure light source.
fn neon_cyan() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(0.15, 0.95, 1.0, 0.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.0),
        emission_color_weight: Vec4::new(0.15, 0.95, 1.0, 8.0),
        ..StandardSurfaceParams::default()
    };
    Material::new("Neon Cyan", p)
}

/// Warm orange-red lava — keeps a faint diffuse base so the
/// surface reads as opaque hot rock, with strong emission on top.
fn hot_lava() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(0.40, 0.06, 0.0, 0.3),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.2),
        emission_color_weight: Vec4::new(1.0, 0.30, 0.05, 4.0),
        params1: Vec4::new(0.6, 0.0, 0.7, 1.5),
        ..StandardSurfaceParams::default()
    };
    Material::new("Hot Lava", p)
}

/// Off-white marble — diffuse base plus a small subsurface lobe
/// for that "light hits the stone and softens" feel.
fn polished_marble() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(0.95, 0.93, 0.88, 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 1.0),
        subsurface_color_weight: Vec4::new(0.95, 0.85, 0.78, 0.18),
        params1: Vec4::new(0.0, 0.0, 0.18, 1.5),
        ..StandardSurfaceParams::default()
    };
    Material::new("Polished Marble", p)
}

/// Cloth — deep red velvet. High `diffuse_roughness` triggers
/// Oren-Nayar, the broad specular weight gives the soft sheen at
/// grazing angles.
fn soft_velvet_red() -> Material {
    let p = StandardSurfaceParams {
        base_color_weight: Vec4::new(0.65, 0.05, 0.10, 1.0),
        specular_color_weight: Vec4::new(1.0, 1.0, 1.0, 0.4),
        params1: Vec4::new(0.9, 0.0, 0.6, 1.5),
        ..StandardSurfaceParams::default()
    };
    Material::new("Soft Velvet", p)
}
