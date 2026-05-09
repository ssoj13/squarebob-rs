//! Standalone helper functions for `Renderer3D`.
//!
//! Extracted from `lib.rs` in the post-sprint-3 modularization pass.
//! All items are `pub(crate)` so the parent crate can use them; no
//! external API change.

use pt_core::bvh::GpuMaterial;
use render_shared::Render3DOptions;

pub(crate) fn compute_slice_normal(opts: &Render3DOptions) -> [f32; 3] {
    if opts.slice_use_vector {
        opts.slice_normal
    } else {
        match opts.slice_axis {
            0 => [1.0, 0.0, 0.0], // X
            1 => [0.0, 1.0, 0.0], // Y
            _ => [0.0, 0.0, 1.0], // Z
        }
    }
}

pub(crate) fn compute_slice_position(opts: &Render3DOptions) -> f32 {
    if opts.slice_use_vector {
        opts.slice_position_vector
    } else {
        opts.slice_position
    }
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub(crate) fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
        lerp(a[3], b[3], t),
    ]
}

pub(crate) fn hash_f32(hash: u32, salt: u32) -> f32 {
    let h = hash
        .wrapping_mul(1664525)
        .wrapping_add(salt)
        .wrapping_mul(1013904223);
    (h as f32) / (u32::MAX as f32)
}

pub(crate) fn mix_material(base: GpuMaterial, glass: GpuMaterial, t: f32) -> GpuMaterial {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return base;
    }
    if t >= 1.0 {
        let mut out = glass;
        // Preserve emission so lights remain visible even at full transparency.
        for i in 0..4 {
            out.emission_color_weight[i] =
                out.emission_color_weight[i].max(base.emission_color_weight[i]);
        }
        return out;
    }
    let mut out = GpuMaterial {
        base_color_weight: lerp4(base.base_color_weight, glass.base_color_weight, t),
        specular_color_weight: lerp4(base.specular_color_weight, glass.specular_color_weight, t),
        transmission_color_weight: lerp4(
            base.transmission_color_weight,
            glass.transmission_color_weight,
            t,
        ),
        subsurface_color_weight: lerp4(
            base.subsurface_color_weight,
            glass.subsurface_color_weight,
            t,
        ),
        coat_color_weight: lerp4(base.coat_color_weight, glass.coat_color_weight, t),
        emission_color_weight: lerp4(base.emission_color_weight, glass.emission_color_weight, t),
        opacity: lerp4(base.opacity, glass.opacity, t),
        params1: lerp4(base.params1, glass.params1, t),
        params2: lerp4(base.params2, glass.params2, t),
    };
    for i in 0..4 {
        out.emission_color_weight[i] =
            out.emission_color_weight[i].max(base.emission_color_weight[i]);
    }
    out
}

pub(crate) fn kelvin_to_rgb(kelvin: f32) -> [f32; 3] {
    let k = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let (mut r, mut g, mut b);
    if k <= 66.0 {
        r = 255.0;
        g = 99.470_8 * k.ln() - 161.119_57;
        b = if k <= 19.0 {
            0.0
        } else {
            138.517_73 * (k - 10.0).ln() - 305.044_8
        };
    } else {
        r = 329.698_73 * (k - 60.0).powf(-0.133_204_76);
        g = 288.122_16 * (k - 60.0).powf(-0.075_514_846);
        b = 255.0;
    }
    r = r.clamp(0.0, 255.0);
    g = g.clamp(0.0, 255.0);
    b = b.clamp(0.0, 255.0);
    [r / 255.0, g / 255.0, b / 255.0]
}

pub(crate) fn apply_glass_controls(mut glass: GpuMaterial, opts: &Render3DOptions) -> GpuMaterial {
    let spec = opts.pt_glass_specular.clamp(0.0, 1.0);
    let base = opts.pt_glass_base.clamp(0.0, 1.0);
    let rough = opts.pt_glass_roughness.clamp(0.0, 1.0);
    let ior = opts.pt_glass_ior.clamp(1.0, 3.0);
    let dispersion = opts.pt_glass_dispersion.clamp(0.0, 1.0);
    let temp = opts.pt_glass_temp.clamp(1000.0, 12000.0);
    let tint = kelvin_to_rgb(temp);

    glass.base_color_weight[3] *= base;
    glass.specular_color_weight[3] *= spec;
    glass.params1[2] = rough;
    glass.params1[3] = ior;
    glass.params2[0] = dispersion;

    glass.transmission_color_weight[0] *= tint[0];
    glass.transmission_color_weight[1] *= tint[1];
    glass.transmission_color_weight[2] *= tint[2];

    // When global transparency is high, bias glass toward transmission clarity.
    // This keeps it "glassier" without switching to a non-physical ghost mode.
    let clarity = opts.pt_global_transparency.clamp(0.0, 1.0);
    if clarity > 0.0 {
        let spec_scale = 1.0 - 0.6 * clarity;
        let base_scale = 1.0 - 0.9 * clarity;
        glass.specular_color_weight[3] *= spec_scale.max(0.05);
        glass.base_color_weight[3] *= base_scale.max(0.0);
        let r = glass.params1[2];
        glass.params1[2] = (r * (1.0 - 0.7 * clarity)).max(0.0);
    }

    if opts.pt_glass_thin {
        glass.base_color_weight[3] = 0.0;
        glass.opacity = [1.0, 1.0, 1.0, 0.0];
    }

    glass
}
