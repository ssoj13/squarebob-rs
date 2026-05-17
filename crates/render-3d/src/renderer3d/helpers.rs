//! Standalone helper functions for `Renderer3D`.
//!
//! Post Phase-4 trim: the legacy glass-mixing helpers
//! (`apply_glass_controls`, `mix_material`, `kelvin_to_rgb`, `lerp`,
//! `lerp4`, `hash_f32`) belonged to the discrete-`MaterialClass`
//! pipeline. With glass / lights now driven by per-material
//! StandardSurface params they have no callers; deleted rather than
//! left behind as silent dead-code warnings.

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
