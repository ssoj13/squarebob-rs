//! Classification settings for the materialize pipeline.
//!
//! This crate is the *classification* half of the materials system:
//! given a cube's metadata (`MaterialInput`), pick a `u32`
//! `material_index` into a caller-supplied library. The actual
//! per-scene material data lives in `pt-material::MaterialLibrary`.
//!
//! Public surface:
//! - [`MaterialSource`] — what scalar dimension to classify on
//!   (extension / path / size / age / depth / random).
//! - [`MaterialDistribution`] — how the seeded source value is
//!   reshaped before the weighted CDF lookup (Direct / Stratified /
//!   Spatial / Perlin / Gradient).
//! - [`MaterializeMode`] — preset shortcut for `MaterialSource`.
//! - [`MaterializeSettings`] — full classification knob bundle.
//! - [`MaterialInput`] — per-cube inputs handed to [`classify_to_index`].
//! - [`classify_to_index`] — pick one `material_index` in `0..weights.len()`.
//! - palette helpers re-exported from [`palette`].

use serde::{Deserialize, Serialize};

mod palette;
pub use palette::{Palette, auto_palette_for_source, hierarchical_path_value, sample_palette};

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
// Material Distribution - how the seeded source value is reshaped
// ============================================================================

/// Distribution modes that reshape the seeded source value *before*
/// the weighted CDF lookup. Each mode preserves the per-slot weight
/// ratio (slot `i` still gets `weight[i] / total` of cubes globally),
/// but rearranges *which* cube lands on which slot.
///
/// Serde aliases keep older presets parseable:
/// * `"Quantized"` → [`Direct`](Self::Direct)
/// * `"Bands"`     → [`Stratified`](Self::Stratified)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
pub enum MaterialDistribution {
    /// Raw seeded source → weighted CDF. Standard weighted picking.
    #[default]
    #[serde(alias = "Quantized")]
    Direct,
    /// Partition the source axis into `band_count` bands. Inside each
    /// band the picker walks the full `[0, 1)` range, so the weighted
    /// CDF still hits every slot proportionally — every band sees a
    /// fresh draw of the whole library. Effect: clean per-band
    /// material zoning without starving narrow weight slots.
    #[serde(alias = "Bands")]
    Stratified,
    /// 3D cellular (Voronoi-style) noise from cube position. All
    /// cubes inside the same `spatial_scale`-sized cell share the
    /// same picker → chunky clusters of one material. Source still
    /// contributes 30 % so `Extension` / `Size` etc. are not lost.
    Spatial,
    /// 3D smooth value noise from cube position. Soft pastel-like
    /// clusters without the hard cell edges of [`Spatial`](Self::Spatial).
    /// Source contributes 30 %, noise 70 %.
    Perlin,
    /// Smoothstep curve on the seeded source. Concentrates cube mass
    /// at the source extremes — useful when the library has many
    /// mid-tone variants you want under-represented.
    Gradient,
}

impl MaterialDistribution {
    pub fn name(self) -> &'static str {
        match self {
            MaterialDistribution::Direct => "Direct",
            MaterialDistribution::Stratified => "Stratified",
            MaterialDistribution::Spatial => "Spatial",
            MaterialDistribution::Perlin => "Perlin",
            MaterialDistribution::Gradient => "Gradient",
        }
    }

    pub fn all() -> &'static [MaterialDistribution] {
        &[
            MaterialDistribution::Direct,
            MaterialDistribution::Stratified,
            MaterialDistribution::Spatial,
            MaterialDistribution::Perlin,
            MaterialDistribution::Gradient,
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
    pub is_pt: bool,
    pub seed: u32,
    pub source: MaterialSource,
    pub distribution: MaterialDistribution,
    /// Number of bands for [`MaterialDistribution::Stratified`].
    pub band_count: u32,
    /// Strength of the source-vs-noise mix for [`MaterialDistribution::Spatial`]
    /// and [`MaterialDistribution::Perlin`]. Higher = bigger clusters,
    /// lower = noisier picks.
    pub spatial_scale: f32,
    /// Held for symmetry with the color-ramp side; classify itself
    /// no longer reads it (distribution shaping replaces what
    /// Quantized used to do).
    pub quant_levels: u32,
    /// `Some(p)` pins the palette for tinting; `None` means auto-pick
    /// from `source`. Consumed by the color-ramp side
    /// (`render-3d::instance_collect::sample_color_ramp`), not by
    /// [`classify_to_index`].
    pub palette: Option<Palette>,
    /// When true, the `Path` source uses `hierarchical_path_value` so
    /// sibling files cluster into nearby indices. When false, `Path`
    /// uses a flat FNV hash and adjacent files scatter randomly.
    pub path_hierarchical: bool,
}

impl Default for MaterializeSettings {
    fn default() -> Self {
        Self {
            is_pt: false,
            seed: 2_654_435_761,
            source: MaterialSource::None,
            distribution: MaterialDistribution::Direct,
            band_count: 8,
            spatial_scale: 0.01,
            quant_levels: 5,
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
    /// Hash of the file *extension* only (e.g. `"jpg"`). All cubes
    /// with the same extension share this hash, so
    /// `MaterialSource::Extension` actually groups by extension.
    pub name_hash: u32,
    /// Hash of the full path. Unique per cube — `MaterialSource::Path`
    /// scatters across the library.
    pub path_hash: u32,
    pub size: u64,
    pub max_size: u64,
    pub depth: u32,
    pub max_depth: u32,
    pub age_normalized: f32,
    /// World-space cube centre. Drives the [`MaterialDistribution::Spatial`]
    /// and [`MaterialDistribution::Perlin`] modes; left at zero for
    /// callers that don't have position handy (Direct / Stratified /
    /// Gradient ignore it).
    pub position: [f32; 3],
    /// Hierarchical accumulation of the path components (0..1). Set by
    /// the caller so the classifier doesn't have to own the `&Path`.
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
/// `[5, 1]` yields ~83 % on slot 0 and ~17 % on slot 1. Pass
/// `&[1.0; n]` for uniform sampling.
///
/// The flow is:
/// 1. Pull a normalised scalar (0..1) from the chosen `source`.
/// 2. Mix in a seed-derived phase rotation so identical inputs
///    across seeds scatter to entirely different slots.
/// 3. Apply the distribution reshape (`Direct` / `Stratified` /
///    `Spatial` / `Perlin` / `Gradient`) — every mode preserves the
///    per-slot weight ratio globally; only the per-cube structure
///    of the picker changes.
/// 4. Walk the cumulative weight array to pick the slot.
///
/// Edge cases:
/// - Empty `weights` or all-zero weights: returns 0.
/// - `MaterialSource::None`: pins slot 0 regardless of weights.
/// - Negative weights are clamped to zero before normalisation.
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
    let seeded = apply_seed(raw, input.name_hash, settings.seed).clamp(0.0, 1.0);
    let picker = reshape(seeded, input, settings).clamp(0.0, 1.0);

    let target = picker * total;
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
///
/// `Path` honours `settings.path_hierarchical`: when true, cubes
/// sharing a common path prefix cluster into nearby source values
/// (siblings end up on neighbouring slots); when false, the flat
/// FNV hash scatters them.
fn source_value(input: &MaterialInput, settings: &MaterializeSettings) -> f32 {
    match settings.source {
        MaterialSource::None => 0.5,
        MaterialSource::Extension => hash_to_float(input.name_hash),
        MaterialSource::Path => {
            if settings.path_hierarchical {
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

/// Mix the seed into the value as a full phase rotation. Two cubes
/// with the same source value but different `name_hash` end up at
/// completely different points in `[0, 1)`, and swapping the seed
/// reshuffles the entire mapping (not just a 10 % jitter).
///
/// `seed == 0` is a passthrough — useful for tests / determinism.
fn apply_seed(value: f32, hash: u32, seed: u32) -> f32 {
    if seed == 0 {
        return value;
    }
    let phase = hash_to_float(hash.wrapping_mul(0x9E37_79B9).wrapping_add(seed));
    (value + phase).fract()
}

/// Apply [`MaterialDistribution`] reshape to `seeded` and return the
/// final picker that the weighted CDF samples. Every branch returns
/// a value in `[0, 1)` and preserves the global weight-as-PMF
/// invariant — see [`classify_to_index`] for the contract.
fn reshape(seeded: f32, input: &MaterialInput, settings: &MaterializeSettings) -> f32 {
    match settings.distribution {
        MaterialDistribution::Direct => seeded,
        MaterialDistribution::Stratified => {
            let n = settings.band_count.max(1) as f32;
            let band = (seeded * n).floor();
            let local = (seeded * n).fract();
            // Per-band phase permutation so adjacent bands don't both
            // start at slot 0.
            let perm = hash_to_float(
                (band as u32)
                    .wrapping_mul(0x9E37_79B9)
                    .wrapping_add(settings.seed),
            );
            (local + perm).fract()
        }
        MaterialDistribution::Spatial => {
            let s = settings.spatial_scale.max(1.0e-4);
            let cx = (input.position[0] / s).floor() as i32;
            let cy = (input.position[1] / s).floor() as i32;
            let cz = (input.position[2] / s).floor() as i32;
            let cell = grid_hash(cx, cy, cz, settings.seed);
            // Mix source 30 % / cell 70 %: source semantics survive
            // but cubes inside one cell share a slot family.
            (cell * 0.7 + seeded * 0.3).fract()
        }
        MaterialDistribution::Perlin => {
            let s = settings.spatial_scale.max(1.0e-4);
            let n = spatial_noise(
                input.position[0] / s,
                input.position[1] / s,
                input.position[2] / s,
                settings.seed,
            );
            (n * 0.7 + seeded * 0.3).fract()
        }
        MaterialDistribution::Gradient => {
            // Smoothstep: heavier tails, lighter middle.
            let t = seeded;
            t * t * (3.0 - 2.0 * t)
        }
    }
}

/// Per-cube vote (`[0, 1)`) for a `MaterialOverride`. Used by
/// `render-3d::material_cache::classify_or_get` to decide whether
/// the override claims a cube: compare the returned scalar against
/// the override's probability.
///
/// The voting reuses the same [`reshape`] machinery as the main
/// `classify_to_index`, so the override sees the same family of
/// per-cube structures (Direct uniform, Stratified bands, Spatial
/// cells, Perlin clusters, Gradient smoothstep) — just driven off
/// the override's own seed instead of `MaterializeSettings::seed`,
/// which is what guarantees that two overrides with different
/// seeds claim *disjoint* cube subsets.
pub fn override_picker(
    input: &MaterialInput,
    distribution: MaterialDistribution,
    band_count: u32,
    spatial_scale: f32,
    seed: u32,
) -> f32 {
    let raw = hash_to_float(input.path_hash.wrapping_mul(0x9E37_79B9).wrapping_add(seed));
    let settings = MaterializeSettings {
        seed,
        distribution,
        band_count,
        spatial_scale,
        ..Default::default()
    };
    reshape(raw, input, &settings).clamp(0.0, 1.0)
}

/// 3D coherent value noise with trilinear interpolation. Output in
/// `[0, 1)`. Used by [`MaterialDistribution::Perlin`].
pub fn spatial_noise(x: f32, y: f32, z: f32, seed: u32) -> f32 {
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

/// Hash a 3D integer cell to a `[0, 1)` scalar. Used as the
/// cellular kernel for [`MaterialDistribution::Spatial`] and the
/// lattice corners for [`spatial_noise`].
pub fn grid_hash(x: i32, y: i32, z: i32, seed: u32) -> f32 {
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
    fn index_in_range_across_distributions() {
        for distribution in [
            MaterialDistribution::Direct,
            MaterialDistribution::Stratified,
            MaterialDistribution::Spatial,
            MaterialDistribution::Perlin,
            MaterialDistribution::Gradient,
        ] {
            let s = MaterializeSettings {
                source: MaterialSource::Extension,
                distribution,
                band_count: 5,
                ..Default::default()
            };
            for lib in [1usize, 2, 5, 8, 16, 64] {
                let w = uniform(lib);
                for h in 0..200u32 {
                    let mut i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
                    i.position = [h as f32 * 0.5, h as f32 * 0.3, h as f32 * 0.7];
                    let idx = classify_to_index(&i, &s, &w) as usize;
                    assert!(idx < lib, "idx {idx} >= lib {lib} ({distribution:?})");
                }
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
    fn seed_phase_rotation_reshuffles() {
        // Same library, same inputs, two different seeds → the slot
        // assignment for at least 80 % of inputs must change. With
        // the old `noise * 0.1` jitter this was ~10 %.
        let mut s = MaterializeSettings {
            source: MaterialSource::Extension,
            seed: 1,
            ..Default::default()
        };
        let w = uniform(8);
        let mut a = Vec::with_capacity(500);
        for h in 0..500u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
            a.push(classify_to_index(&i, &s, &w));
        }
        s.seed = 0xDEAD_BEEF;
        let mut diff = 0;
        for h in 0..500u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
            if classify_to_index(&i, &s, &w) != a[h as usize] {
                diff += 1;
            }
        }
        let frac = diff as f32 / 500.0;
        assert!(
            frac >= 0.5,
            "seed change reshuffled only {pct:.0}% of slots (want ≥50 %)",
            pct = frac * 100.0
        );
    }

    #[test]
    fn narrow_weight_still_reachable_with_stratified() {
        // 7 slots, one with a narrow weight (~6.5 % of total),
        // Stratified at 14 bands. Slot 4 must still receive a
        // non-trivial share of cubes — Stratified must not starve
        // narrow weight slots.
        let s = MaterializeSettings {
            source: MaterialSource::Extension,
            distribution: MaterialDistribution::Stratified,
            band_count: 14,
            ..Default::default()
        };
        let w = vec![1.0, 1.0, 1.0, 1.0, 0.42, 1.0, 1.0];
        let mut hits = 0u32;
        for h in 0..10_000u32 {
            let i = input_for(h.wrapping_mul(2_654_435_761), 0.0);
            if classify_to_index(&i, &s, &w) == 4 {
                hits += 1;
            }
        }
        // Expected ~6.5 % — accept [3 %, 12 %] for hash bias.
        assert!(
            (300..1200).contains(&hits),
            "expected slot 4 to receive ~6.5 % of cubes, got {hits} of 10000"
        );
    }

    #[test]
    fn weights_skew_distribution() {
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

    #[test]
    fn legacy_quantized_deserializes_as_direct() {
        let d: MaterialDistribution = serde_json::from_str("\"Quantized\"").unwrap();
        assert_eq!(d, MaterialDistribution::Direct);
    }

    #[test]
    fn legacy_bands_deserializes_as_stratified() {
        let d: MaterialDistribution = serde_json::from_str("\"Bands\"").unwrap();
        assert_eq!(d, MaterialDistribution::Stratified);
    }
}
