//! Classification settings for the materialize pipeline.
//!
//! Phase 4 split: this crate no longer owns the material data model.
//! `pt-material::MaterialLibrary` is the single source of truth for the
//! per-scene material slots. `pt-mats` retains only the *classification*
//! side — given a cube's metadata (`MaterialInput`), pick a `u32`
//! `material_index` into a caller-supplied library. The legacy
//! `MaterialClass` enum and its 1500-entry `MaterialLibrary` are gone.
//!
//! Public surface:
//! - [`MaterialSource`] — what scalar dimension to classify on
//!   (extension / path / size / age / depth / random).
//! - [`MaterialDistribution`] — how the scalar maps to slot indices
//!   (direct / quantised / gradient / spatial / bands).
//! - [`MaterializeMode`] — legacy preset shortcut for `MaterialSource`.
//! - [`MaterializeSettings`] — full classification knob bundle.
//! - [`MaterialInput`] — per-cube inputs handed to [`classify_to_index`].
//! - [`classify_to_index`] — pick one `material_index` in `0..library_size`.
//! - palette helpers re-exported from [`palette`].

use serde::{Deserialize, Serialize};

mod palette;
pub use palette::{
    auto_palette_for_source, hierarchical_path_value, sample_palette, Palette,
};

// ============================================================================
// Material Source - what data determines the material
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaterialSource {
    #[default]
    None,
    Extension,
    Path,
    Size,
    Age,
    Depth,
    Random,
}

impl MaterialSource {
    pub fn name(self) -> &'static str {
        match self {
            MaterialSource::None => "None",
            MaterialSource::Extension => "Extension",
            MaterialSource::Path => "Path",
            MaterialSource::Size => "Size",
            MaterialSource::Age => "Age",
            MaterialSource::Depth => "Depth",
            MaterialSource::Random => "Random",
        }
    }

    pub fn all() -> &'static [MaterialSource] {
        &[
            MaterialSource::None,
            MaterialSource::Extension,
            MaterialSource::Path,
            MaterialSource::Size,
            MaterialSource::Age,
            MaterialSource::Depth,
            MaterialSource::Random,
        ]
    }
}

// ============================================================================
// Material Distribution - how values map to materials
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaterialDistribution {
    #[default]
    Direct,
    Quantized,
    Gradient,
    Spatial,
    Bands,
}

impl MaterialDistribution {
    pub fn name(self) -> &'static str {
        match self {
            MaterialDistribution::Direct => "Direct",
            MaterialDistribution::Quantized => "Quantized",
            MaterialDistribution::Gradient => "Gradient",
            MaterialDistribution::Spatial => "Spatial",
            MaterialDistribution::Bands => "Bands",
        }
    }

    pub fn all() -> &'static [MaterialDistribution] {
        &[
            MaterialDistribution::Direct,
            MaterialDistribution::Quantized,
            MaterialDistribution::Gradient,
            MaterialDistribution::Spatial,
            MaterialDistribution::Bands,
        ]
    }
}

// ============================================================================
// MaterializeMode (legacy preset -> MaterialSource)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterializeMode {
    None,
    ByExtension,
    ByPath,
    BySize,
    ByAge,
    Random,
}

impl MaterializeMode {
    pub fn name(self) -> &'static str {
        match self {
            MaterializeMode::None => "None",
            MaterializeMode::ByExtension => "By Extension",
            MaterializeMode::ByPath => "By Path",
            MaterializeMode::BySize => "By Size",
            MaterializeMode::ByAge => "By Age",
            MaterializeMode::Random => "Random",
        }
    }

    pub fn all() -> &'static [MaterializeMode] {
        &[
            MaterializeMode::None,
            MaterializeMode::ByExtension,
            MaterializeMode::ByPath,
            MaterializeMode::BySize,
            MaterializeMode::ByAge,
            MaterializeMode::Random,
        ]
    }

    /// Convert legacy mode to new source enum.
    pub fn to_source(self) -> MaterialSource {
        match self {
            MaterializeMode::None => MaterialSource::None,
            MaterializeMode::ByExtension => MaterialSource::Extension,
            MaterializeMode::ByPath => MaterialSource::Path,
            MaterializeMode::BySize => MaterialSource::Size,
            MaterializeMode::ByAge => MaterialSource::Age,
            MaterializeMode::Random => MaterialSource::Random,
        }
    }
}

// ============================================================================
// MaterializeSettings — full classification knob bundle
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub struct MaterializeSettings {
    pub allow_lights: bool,
    pub light_prob: f32,
    pub light_warm: f32,
    pub light_cool: f32,
    pub allow_glass: bool,
    pub glass_prob: f32,
    pub is_pt: bool,
    pub seed: u32,
    pub source: MaterialSource,
    pub distribution: MaterialDistribution,
    pub quant_levels: u32,
    pub band_count: u32,
    pub spatial_scale: f32,
    /// `Some(p)` pins the palette for tinting; `None` means auto-pick from
    /// `source`. The library is now driven by `pt-material` so the palette
    /// is only used by upstream colour-ramp consumers (`render-3d`
    /// instance_collect), not by [`classify_to_index`].
    pub palette: Option<Palette>,
    /// When true, the `Path` source uses `hierarchical_path_value` so
    /// sibling files cluster into nearby indices. When false, `Path` uses a
    /// flat FNV hash and adjacent files scatter randomly.
    pub path_hierarchical: bool,
}

impl Default for MaterializeSettings {
    fn default() -> Self {
        Self {
            allow_lights: false,
            light_prob: 0.15,
            light_warm: 0.5,
            light_cool: 0.5,
            allow_glass: false,
            glass_prob: 0.5,
            is_pt: false,
            seed: 2_654_435_761,
            source: MaterialSource::None,
            distribution: MaterialDistribution::Direct,
            quant_levels: 5,
            band_count: 8,
            spatial_scale: 0.01,
            palette: None,
            path_hierarchical: true,
        }
    }
}

// ============================================================================
// MaterialInput — per-cube classification inputs
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub struct MaterialInput {
    pub name_hash: u32,
    pub path_hash: u32,
    pub size: u64,
    pub max_size: u64,
    pub depth: u32,
    pub max_depth: u32,
    pub age_normalized: f32,
    pub position: [f32; 3],
    /// Hierarchical accumulation of the path components (0..1). Set by the
    /// caller so the classifier doesn't have to own the `&Path`.
    pub path_hierarchical_value: f32,
}

impl Default for MaterialInput {
    fn default() -> Self {
        Self {
            name_hash: 0,
            path_hash: 0,
            size: 0,
            max_size: 1,
            depth: 0,
            max_depth: 1,
            age_normalized: 0.5,
            position: [0.0, 0.0, 0.0],
            path_hierarchical_value: 0.0,
        }
    }
}

// ============================================================================
// classify_to_index — the one and only public classification entry point
// ============================================================================

/// Map per-cube classification inputs to a `material_index` in
/// `0..weights.len()`, sampling proportional to per-slot weights.
///
/// `weights` are unnormalised non-negative magnitudes — they're summed
/// and treated as a probability mass function. Slot `i` claims a
/// fraction `weights[i] / sum(weights)` of the cube population, so
/// `[5, 1]` yields ~83% on slot 0 and ~17% on slot 1. Pass
/// `&[1.0; n]` for uniform sampling (the legacy behaviour).
///
/// The flow is:
/// 1. Pull a normalised scalar (0..1) from the chosen `source`.
/// 2. Mix in a seed-derived noise so identical inputs across libraries
///    don't all collapse onto slot 0.
/// 3. Apply the distribution shaping (quantise / gradient / bands /
///    spatial noise).
/// 4. Walk the cumulative weight array to pick the slot.
///
/// Edge cases:
/// - Empty `weights` or all-zero weights: returns 0.
/// - `MaterialSource::None`: pins slot 0 regardless of weights.
/// - Negative weights are clamped to zero before normalisation.
///
/// Light / glass overrides from the old library are now handled by the
/// per-material PT shader path, not by classification — once a cube
/// has resolved its `Material`, the renderer applies emission / IOR /
/// dispersion based on the StandardSurface params themselves.
pub fn classify_to_index(
    input: &MaterialInput,
    settings: &MaterializeSettings,
    weights: &[f32],
) -> u32 {
    if weights.is_empty() {
        return 0;
    }
    if settings.source == MaterialSource::None {
        return 0;
    }

    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    if total <= 0.0 {
        return 0;
    }

    let raw = source_value(input, settings);
    let seeded = apply_seed(raw, input.name_hash, settings.seed);
    let distributed = apply_distribution(seeded, input, settings).clamp(0.0, 1.0);
    debug_assert!(distributed.is_finite(), "distribution returned non-finite value");

    let target = distributed * total;
    let mut cum = 0.0f32;
    for (i, &w) in weights.iter().enumerate() {
        cum += w.max(0.0);
        if target <= cum {
            return i as u32;
        }
    }
    (weights.len() - 1) as u32
}

/// Get normalised value (0.0-1.0) from the selected source.
fn source_value(input: &MaterialInput, settings: &MaterializeSettings) -> f32 {
    match settings.source {
        MaterialSource::None => 0.5,
        MaterialSource::Extension => hash_to_float(input.name_hash),
        MaterialSource::Path => {
            if settings.path_hierarchical && input.path_hierarchical_value > 0.0 {
                input.path_hierarchical_value.clamp(0.0, 1.0)
            } else {
                hash_to_float(input.path_hash)
            }
        }
        MaterialSource::Size => {
            if input.max_size == 0 {
                0.5
            } else {
                let log_size = (input.size as f64 + 1.0).log10();
                let log_max = (input.max_size as f64 + 1.0).log10();
                (log_size / log_max.max(1.0)) as f32
            }
        }
        MaterialSource::Age => input.age_normalized.clamp(0.0, 1.0),
        MaterialSource::Depth => {
            if input.max_depth == 0 {
                0.0
            } else {
                (input.depth as f32 / input.max_depth as f32).clamp(0.0, 1.0)
            }
        }
        MaterialSource::Random => hash_to_float(input.name_hash.wrapping_mul(0x9E37_79B9)),
    }
}

/// Mix the seed into the value so identical inputs across different seeds
/// scatter to different library slots. Keeps result in [0, 1).
fn apply_seed(value: f32, hash: u32, seed: u32) -> f32 {
    let seeded_hash = hash.wrapping_mul(seed);
    let noise = hash_to_float(seeded_hash) * 0.1;
    (value + noise).fract()
}

/// Apply the distribution shaping to a normalised scalar.
fn apply_distribution(value: f32, input: &MaterialInput, settings: &MaterializeSettings) -> f32 {
    match settings.distribution {
        MaterialDistribution::Direct => value,

        MaterialDistribution::Quantized => {
            let levels = settings.quant_levels.max(1) as f32;
            (value * levels).floor() / (levels - 1.0).max(1.0)
        }

        MaterialDistribution::Gradient => {
            let t = value.clamp(0.0, 1.0);
            // Smoothstep: more visual weight near the ends, fewer
            // mid-tone slots — useful when most materials in the
            // library are mid-tone variants.
            t * t * (3.0 - 2.0 * t)
        }

        MaterialDistribution::Spatial => {
            let scale = settings.spatial_scale;
            let px = input.position[0] * scale;
            let py = input.position[1] * scale;
            let pz = input.position[2] * scale;
            let noise = spatial_noise(px, py, pz, settings.seed);
            (value * 0.3 + noise * 0.7).clamp(0.0, 1.0)
        }

        MaterialDistribution::Bands => {
            let bands = settings.band_count.max(1) as f32;
            let band_idx = (value * bands).floor();
            band_idx / (bands - 1.0).max(1.0)
        }
    }
}

/// Simple 3D coherent noise. Used by `MaterialDistribution::Spatial`.
fn spatial_noise(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let iz = z.floor() as i32;

    let fx = x - x.floor();
    let fy = y - y.floor();
    let fz = z - z.floor();

    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let uz = fz * fz * (3.0 - 2.0 * fz);

    let h000 = grid_hash(ix, iy, iz, seed);
    let h001 = grid_hash(ix, iy, iz + 1, seed);
    let h010 = grid_hash(ix, iy + 1, iz, seed);
    let h011 = grid_hash(ix, iy + 1, iz + 1, seed);
    let h100 = grid_hash(ix + 1, iy, iz, seed);
    let h101 = grid_hash(ix + 1, iy, iz + 1, seed);
    let h110 = grid_hash(ix + 1, iy + 1, iz, seed);
    let h111 = grid_hash(ix + 1, iy + 1, iz + 1, seed);

    let lerp = |a: f32, b: f32, t: f32| a + t * (b - a);

    let x00 = lerp(h000, h100, ux);
    let x01 = lerp(h001, h101, ux);
    let x10 = lerp(h010, h110, ux);
    let x11 = lerp(h011, h111, ux);

    let y0 = lerp(x00, x10, uy);
    let y1 = lerp(x01, x11, uy);

    lerp(y0, y1, uz)
}

fn grid_hash(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let h = (x as u32).wrapping_mul(73_856_093)
        ^ (y as u32).wrapping_mul(19_349_663)
        ^ (z as u32).wrapping_mul(83_492_791)
        ^ seed;
    hash_to_float(h)
}

fn hash_to_float(h: u32) -> f32 {
    (h as f32) / (u32::MAX as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn input_for(hash: u32, hier: f32) -> MaterialInput {
        MaterialInput {
            name_hash: hash,
            path_hash: hash,
            size: 1024,
            max_size: 1_048_576,
            depth: 3,
            max_depth: 8,
            age_normalized: hash_to_float(hash),
            position: [0.0, 0.0, 0.0],
            path_hierarchical_value: hier,
        }
    }

    fn uniform(n: usize) -> Vec<f32> {
        vec![1.0; n]
    }

    #[test]
    fn empty_library_returns_zero() {
        let s = MaterializeSettings {
            source: MaterialSource::Extension,
            ..Default::default()
        };
        assert_eq!(classify_to_index(&input_for(42, 0.0), &s, &[]), 0);
    }

    #[test]
    fn source_none_returns_zero() {
        let s = MaterializeSettings::default();
        let w = uniform(8);
        for h in 0..50u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
            assert_eq!(
                classify_to_index(&i, &s, &w),
                0,
                "source=None must pin slot 0"
            );
        }
    }

    #[test]
    fn index_in_range() {
        let s = MaterializeSettings {
            source: MaterialSource::Extension,
            ..Default::default()
        };
        for lib in [1usize, 2, 5, 8, 16, 64] {
            let w = uniform(lib);
            for h in 0..200u32 {
                let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
                let idx = classify_to_index(&i, &s, &w) as usize;
                assert!(idx < lib, "idx {idx} >= lib {lib}");
            }
        }
    }

    #[test]
    fn determinism_same_inputs_same_index() {
        let s = MaterializeSettings {
            source: MaterialSource::Path,
            ..Default::default()
        };
        let w = uniform(16);
        for h in 0..50u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.3);
            let a = classify_to_index(&i, &s, &w);
            let b = classify_to_index(&i, &s, &w);
            assert_eq!(a, b, "non-deterministic for hash {h}");
        }
    }

    #[test]
    fn distribution_quantized_uses_quant_levels() {
        let s = MaterializeSettings {
            source: MaterialSource::Extension,
            distribution: MaterialDistribution::Quantized,
            quant_levels: 3,
            ..Default::default()
        };
        let w = uniform(64);
        let mut seen = std::collections::HashSet::new();
        for h in 0..1000u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
            seen.insert(classify_to_index(&i, &s, &w));
        }
        // 3 quantisation levels → at most 3 distinct output indices.
        assert!(seen.len() <= 3, "quantised distribution produced {} distinct indices", seen.len());
    }

    #[test]
    fn weights_skew_distribution() {
        // weight 9 vs 1 → ~90% of cubes on slot 0, ~10% on slot 1.
        // Verify by counting over a large sample; the per-slot count
        // ratio should follow the weight ratio within a generous
        // tolerance (this is a probabilistic test, not a tight one).
        let s = MaterializeSettings {
            source: MaterialSource::Extension,
            ..Default::default()
        };
        let w = vec![9.0, 1.0];
        let mut c0 = 0u32;
        let mut c1 = 0u32;
        for h in 0..10_000u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
            match classify_to_index(&i, &s, &w) {
                0 => c0 += 1,
                1 => c1 += 1,
                other => panic!("idx {other} out of range"),
            }
        }
        let ratio = c0 as f32 / c1.max(1) as f32;
        // Expected ~9.0; allow 5..15 — wide tolerance for hash bias.
        assert!(
            (5.0..15.0).contains(&ratio),
            "expected slot0:slot1 ~9:1, got {c0}:{c1} (ratio {ratio:.2})"
        );
    }

    #[test]
    fn zero_weights_collapse_to_zero() {
        let s = MaterializeSettings {
            source: MaterialSource::Extension,
            ..Default::default()
        };
        let w = vec![0.0; 5];
        for h in 0..50u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
            assert_eq!(classify_to_index(&i, &s, &w), 0);
        }
    }

    #[test]
    fn hierarchical_path_re_exports_correctly() {
        let v = hierarchical_path_value(Path::new("/a/b/c"));
        assert!((0.0..=1.0).contains(&v));
    }
}
