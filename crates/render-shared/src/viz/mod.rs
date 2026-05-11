//! Shared visualization primitives.
//!
//! `Render3DOptions` accumulates the same shape of knob across several
//! features: per-mode scalar curves (height), per-mode color ramps
//! (color / folder tint), and palette-driven materials. This module
//! lifts those shapes into reusable types so the renderer code and the
//! settings UI can drive them through one API.
//!
//! - [`CurveParams`] — scalar (scale + exponent), used by Height.
//! - [`RampParams`] — color ramp (palette + distribution + curve),
//!   used by Color / Folder / Materials.
//! - [`Mapping<P, N>`] — persistent per-mode storage, indexed by
//!   `mode as usize`. Switching modes preserves each variant's params.

use pt_mats::{MaterialDistribution, Palette};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Number of `CubeHeightMode` variants. Kept in sync with the enum so
/// `Mapping<_, N_HEIGHT_MODES>` size-checks at compile time. Bump when
/// adding/removing modes.
pub const N_HEIGHT_MODES: usize = 8;
pub const N_COLOR_MODES: usize = 5;
pub const N_FOLDER_COLOR_MODES: usize = 3;
pub const N_HASH_EFFECTS: usize = 16;

/// Non-linear scalar transform: `output = (value ^ exponent) * scale`.
/// `exponent = 1.0` is linear, `< 1.0` compresses, `> 1.0` amplifies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CurveParams {
    pub scale: f32,
    pub exponent: f32,
}

impl Default for CurveParams {
    fn default() -> Self {
        Self {
            scale: 1.0,
            exponent: 1.0,
        }
    }
}

impl CurveParams {
    /// Apply curve to `value`. Negative inputs clamp to 0 before `powf`
    /// to avoid NaN.
    pub fn apply(self, value: f32) -> f32 {
        let v = value.max(0.0);
        let shaped = if (self.exponent - 1.0).abs() < f32::EPSILON {
            v
        } else {
            v.powf(self.exponent)
        };
        shaped * self.scale
    }
}

/// Color-ramp parameters. Drives a scalar `t ∈ [0, 1]` through optional
/// distribution reshaping then samples a palette.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RampParams {
    /// `None` = auto-route by context (e.g. Size→Viridis, Age→Sunset).
    pub palette: Option<Palette>,
    pub distribution: MaterialDistribution,
    /// For `Quantized`: number of steps. Clamped 2..=14.
    pub quant_levels: u32,
    /// For `Bands`: number of bands. Clamped 2..=20.
    pub band_count: u32,
    /// For `Spatial`: world-space frequency for the noise input.
    pub spatial_scale: f32,
    /// Pre-palette curve shaping. Applied to `t` before distribution so
    /// the user can re-balance histograms (e.g. exponent=2 emphasises
    /// large values, scale stretches the dynamic range).
    pub curve: CurveParams,
}

impl Default for RampParams {
    fn default() -> Self {
        Self {
            palette: None,
            distribution: MaterialDistribution::Direct,
            quant_levels: 5,
            band_count: 8,
            spatial_scale: 0.01,
            curve: CurveParams::default(),
        }
    }
}

/// Persistent per-mode storage. Indexed by `mode as usize`. `N` must
/// match the enum's variant count.
///
/// Custom serde impl serialises as a JSON array; deserialisation pads
/// with defaults (too few entries) or truncates (too many) so config
/// drift across refactors stays non-fatal.
#[derive(Debug, Clone)]
pub struct Mapping<P, const N: usize>
where
    P: Default + Copy,
{
    pub per_mode: [P; N],
}

impl<P, const N: usize> Default for Mapping<P, N>
where
    P: Default + Copy,
{
    fn default() -> Self {
        Self {
            per_mode: [P::default(); N],
        }
    }
}

impl<P, const N: usize> Serialize for Mapping<P, N>
where
    P: Serialize + Default + Copy,
{
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.per_mode[..].serialize(ser)
    }
}

impl<'de, P, const N: usize> Deserialize<'de> for Mapping<P, N>
where
    P: Deserialize<'de> + Default + Copy,
{
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Vec::<P>::deserialize(de)?;
        let mut per_mode = [P::default(); N];
        for (slot, value) in per_mode.iter_mut().zip(raw) {
            *slot = value;
        }
        Ok(Self { per_mode })
    }
}

impl<P, const N: usize> Mapping<P, N>
where
    P: Default + Copy,
{
    /// Read the slot for `idx`. Out-of-range indices clamp to the last
    /// slot (defensive; should never trigger when `N` matches the enum).
    pub fn get(&self, idx: usize) -> P {
        self.per_mode[idx.min(N - 1)]
    }

    /// Mutable slot for `idx`. Out-of-range indices clamp.
    pub fn get_mut(&mut self, idx: usize) -> &mut P {
        let i = idx.min(N - 1);
        &mut self.per_mode[i]
    }
}

/// Per-effect parameters. All hash effects today share this shape;
/// future heterogeneous effects can add their own struct.
///
/// `speed` is a per-variant multiplier on the global animation speed so
/// "Wave" can shimmer quickly while "Pulse" breathes slowly without
/// the user re-tuning the global slider every time they switch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HashEffectParams {
    pub strength: f32,
    pub speed: f32,
}

impl Default for HashEffectParams {
    fn default() -> Self {
        Self {
            strength: 2.0,
            speed: 1.0,
        }
    }
}

/// Heterogeneous container for all effect categories. Today only hash
/// effects exist; new effect families add their own field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EffectsState {
    /// One slot per [`HashTransformEffect`] variant. Switching effects
    /// preserves each variant's strength.
    pub hash_per_variant: Mapping<HashEffectParams, N_HASH_EFFECTS>,
}

/// Animation timelines. Objects (cube transforms, hash effects) and the
/// environment (sky time-of-day, daylight cycle) advance independently
/// so the user can pause / slow / speed them separately.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AnimationState {
    pub object_speed: f32,
    pub env_speed: f32,
    /// Accumulated time consumed by object effects (rotation, wave,
    /// etc.). Persists across frames; resumes from where it left off
    /// when `animate` toggles back on.
    pub object_time: f32,
    /// Accumulated time consumed by env effects (sky day/night cycle,
    /// procedural atmosphere). Advances independently of `object_time`.
    pub env_time: f32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            object_speed: 1.0,
            env_speed: 1.0,
            object_time: 0.0,
            env_time: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_identity() {
        let c = CurveParams::default();
        assert!((c.apply(0.5) - 0.5).abs() < 1e-6);
        assert!((c.apply(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(c.apply(-0.5), 0.0);
    }

    #[test]
    fn curve_exponent_compresses_or_amplifies() {
        let sqrt = CurveParams {
            scale: 1.0,
            exponent: 0.5,
        };
        let squared = CurveParams {
            scale: 1.0,
            exponent: 2.0,
        };
        // sqrt(0.25) = 0.5 (amplifies low values)
        assert!((sqrt.apply(0.25) - 0.5).abs() < 1e-6);
        // 0.5^2 = 0.25 (compresses low values)
        assert!((squared.apply(0.5) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn mapping_default_fills_all_slots() {
        let m: Mapping<CurveParams, 4> = Mapping::default();
        for p in m.per_mode {
            assert_eq!(p.scale, 1.0);
            assert_eq!(p.exponent, 1.0);
        }
    }

    #[test]
    fn mapping_get_clamps_out_of_range() {
        let m: Mapping<CurveParams, 4> = Mapping::default();
        // Should not panic for oversized index.
        let _ = m.get(999);
    }
}
