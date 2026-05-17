//! Single `Material` slot — base params + parallel variance + identity.

use serde::{Deserialize, Serialize};
use standard_surface::StandardSurfaceParams;
use uuid::Uuid;

/// One material entry in a [`super::MaterialLibrary`].
///
/// `params` are the editable base values; `variance` is a parallel
/// struct of the same layout that holds "± spread" applied per cube
/// at resolve time. A zero `variance` produces a deterministic
/// material across every cube that references this slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    /// Stable identifier — survives reorder / rename / library export.
    /// Use this when referring to a material from external scene data
    /// that should survive library shuffles; the array `index` is
    /// only stable within one snapshot of the library.
    #[serde(default = "Uuid::new_v4")]
    pub uuid: Uuid,

    /// Display name, free-form. Shown in the editor list.
    pub name: String,

    /// Relative weight in the per-cube distribution. Weights across
    /// the library are normalised to sum 1.0 at classification time —
    /// e.g. `[5.0, 1.0]` puts ~83% of cubes on slot 0 and ~17% on slot
    /// 1. Range 0.0 ..= 10.0 in the UI; default 1.0 (uniform). Set to
    /// 0 to effectively exclude a slot from the distribution.
    #[serde(default = "default_weight")]
    pub weight: f32,

    /// Base material parameters. Edited directly via the UI.
    pub params: StandardSurfaceParams,

    /// Per-attribute random spread, applied symmetrically in
    /// `[-variance, +variance]` per channel per cube. Same layout as
    /// `params` — every editable field has its own spread slot.
    /// Default = zero variance everywhere (no per-cube spread).
    pub variance: StandardSurfaceParams,
}

fn default_weight() -> f32 {
    1.0
}

impl Material {
    /// New material with the given name + base params; variance
    /// zeroed and weight = 1.0 (uniform contribution to the
    /// distribution).
    pub fn new(name: impl Into<String>, params: StandardSurfaceParams) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name: name.into(),
            weight: default_weight(),
            params,
            variance: zero_params(),
        }
    }

    /// Resolve the per-cube material by applying per-attribute random
    /// offsets in `[-variance, +variance]`, deterministic in `cube_hash`.
    /// The output is GPU-ready — same `Pod+Zeroable` layout the WGSL
    /// `Material` struct reads from the material storage buffer.
    pub fn resolve_for_cube(&self, cube_hash: u64) -> StandardSurfaceParams {
        let mut out = self.params;
        // We iterate over the 40 underlying `f32` lanes of the struct
        // (10 `vec4`s × 4 channels) via a `bytemuck` view, so the loop
        // doesn't need to know the field layout. New fields added to
        // `StandardSurfaceParams` are automatically variance-aware.
        let base: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&self.params));
        let var: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&self.variance));
        let out_lanes: &mut [f32] =
            bytemuck::cast_slice_mut(std::slice::from_mut(&mut out));
        for (i, ((b, v), o)) in base
            .iter()
            .zip(var.iter())
            .zip(out_lanes.iter_mut())
            .enumerate()
        {
            *o = apply_variance(*b, *v, channel_seed(cube_hash, i));
        }
        out
    }
}

/// Salt-mix the per-cube hash with the per-channel index so different
/// channels of the same cube get independent random offsets. Plain
/// SplitMix64 step — fast, deterministic, no allocations.
fn channel_seed(cube_hash: u64, channel: usize) -> u64 {
    let mut z = cube_hash
        .wrapping_add((channel as u64).wrapping_mul(0xD2B74407B1CE6E93))
        .wrapping_mul(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Map a 64-bit hash to a uniform `f32` in `(-1, 1)` and multiply by
/// `variance`. Hash → IEEE-754 mantissa trick: build a `[1.0, 2.0)`
/// float, subtract `1.0` for `[0, 1)`, then `* 2 - 1` for `(-1, 1)`.
fn apply_variance(base: f32, variance: f32, seed: u64) -> f32 {
    if variance.abs() < 1e-7 {
        return base;
    }
    let bits = ((seed >> 32) as u32 & 0x007F_FFFF) | 0x3F80_0000;
    let unit01 = f32::from_bits(bits) - 1.0;
    let signed = unit01 * 2.0 - 1.0;
    base + signed * variance
}

/// All-zero `StandardSurfaceParams` — used as the default
/// `Material::variance` (no per-cube spread).
fn zero_params() -> StandardSurfaceParams {
    bytemuck::Zeroable::zeroed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_variance_is_identity() {
        let mat = Material::new("test", StandardSurfaceParams::default());
        let resolved = mat.resolve_for_cube(0xDEAD_BEEF);
        let lhs: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&mat.params));
        let rhs: &[f32] =
            bytemuck::cast_slice(std::slice::from_ref(&resolved));
        for (a, b) in lhs.iter().zip(rhs.iter()) {
            assert_eq!(a.to_bits(), b.to_bits(), "zero-variance must be identity");
        }
    }

    #[test]
    fn variance_is_deterministic() {
        let mut mat = Material::new("test", StandardSurfaceParams::default());
        let var_lanes: &mut [f32] =
            bytemuck::cast_slice_mut(std::slice::from_mut(&mut mat.variance));
        for lane in var_lanes.iter_mut() {
            *lane = 0.1;
        }
        let a = mat.resolve_for_cube(0xCAFE);
        let b = mat.resolve_for_cube(0xCAFE);
        let la: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&a));
        let lb: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&b));
        for (a, b) in la.iter().zip(lb.iter()) {
            assert_eq!(a.to_bits(), b.to_bits(), "same cube_hash → same result");
        }
    }

    #[test]
    fn variance_differs_between_cubes() {
        let mut mat = Material::new("test", StandardSurfaceParams::default());
        // Bump every lane's spread so almost every cube gets a unique
        // resolved value (still possible to alias by accident on one
        // lane, but extremely unlikely across all 40).
        let var_lanes: &mut [f32] =
            bytemuck::cast_slice_mut(std::slice::from_mut(&mut mat.variance));
        for lane in var_lanes.iter_mut() {
            *lane = 0.5;
        }
        let a = mat.resolve_for_cube(1);
        let b = mat.resolve_for_cube(2);
        let la: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&a));
        let lb: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&b));
        let any_diff = la.iter().zip(lb.iter()).any(|(x, y)| x != y);
        assert!(any_diff, "different cube_hash should perturb at least one lane");
    }
}
