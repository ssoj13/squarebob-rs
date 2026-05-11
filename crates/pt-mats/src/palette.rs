//! Perceptual color palettes for material gradients.
//!
//! Provides smooth color ramps suitable for visualizing ordered scalar data
//! (file size, directory depth, age) and high-quality categorical mappings
//! (extension, random). Output is linear RGB in [0, 1]^3 — ready to drop
//! into `GpuMaterial.base_color_weight`.
//!
//! Polynomials for Viridis/Magma/Plasma/Turbo come from Inigo Quilez's
//! shader approximations of matplotlib's colormaps (close enough for
//! perception). Cubehelix follows Green 2011. Sunset is a hand-picked
//! diverging palette.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::MaterialSource;

/// Number of pre-baked samples per palette stored in `MaterialLibrary`.
/// 256 gives perceptually-continuous gradients on 8-bit displays.
pub const PALETTE_BINS: u32 = 256;

/// Number of materials reserved for legacy `MaterialClass` entries
/// (Default..NeonBlue = 34). Palette samples start at this offset.
pub const BASE_LIBRARY_SIZE: u32 = 34;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
pub enum Palette {
    #[default]
    Viridis,
    Magma,
    Plasma,
    Turbo,
    Sunset,
    Cubehelix,
}

impl Palette {
    pub fn all() -> &'static [Palette] {
        &[
            Palette::Viridis,
            Palette::Magma,
            Palette::Plasma,
            Palette::Turbo,
            Palette::Sunset,
            Palette::Cubehelix,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            Palette::Viridis => "Viridis",
            Palette::Magma => "Magma",
            Palette::Plasma => "Plasma",
            Palette::Turbo => "Turbo",
            Palette::Sunset => "Sunset",
            Palette::Cubehelix => "Cubehelix",
        }
    }

    /// Stable index for material-id arithmetic. Order MUST match `all()`.
    pub fn idx(self) -> u32 {
        match self {
            Palette::Viridis => 0,
            Palette::Magma => 1,
            Palette::Plasma => 2,
            Palette::Turbo => 3,
            Palette::Sunset => 4,
            Palette::Cubehelix => 5,
        }
    }

    pub fn count() -> u32 {
        Self::all().len() as u32
    }
}

/// Pick a palette that matches the natural ordering of a source. Continuous
/// ordered sources (Size, Depth, Age) get sequential / diverging ramps;
/// categorical / random sources get high-frequency rainbow-ish palettes so
/// adjacent values are easy to distinguish.
pub fn auto_palette_for_source(s: MaterialSource) -> Palette {
    match s {
        MaterialSource::Size => Palette::Viridis,
        MaterialSource::Depth => Palette::Cubehelix,
        MaterialSource::Age => Palette::Sunset,
        MaterialSource::Path => Palette::Turbo,
        MaterialSource::Extension => Palette::Plasma,
        MaterialSource::Random => Palette::Turbo,
        // `None` → constant midpoint; any palette is fine, pick Viridis.
        MaterialSource::None => Palette::Viridis,
    }
}

/// Sample a palette at `t in [0, 1]`. Returns linear-RGB.
pub fn sample_palette(p: Palette, t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let raw = match p {
        Palette::Viridis => poly_viridis(t),
        Palette::Magma => poly_magma(t),
        Palette::Plasma => poly_plasma(t),
        Palette::Turbo => poly_turbo(t),
        Palette::Sunset => stops_sunset(t),
        Palette::Cubehelix => cubehelix(t),
    };
    [raw[0].clamp(0.0, 1.0), raw[1].clamp(0.0, 1.0), raw[2].clamp(0.0, 1.0)]
}

/// Hierarchical hash of a path: components closer to the root dominate, so
/// sibling files (sharing parents) hash to nearby values and therefore to
/// nearby palette colors. Decay 0.4 → each deeper segment contributes 40%
/// of the previous's amplitude, fast enough to keep siblings visibly
/// clustered while still distinguishing files within one directory.
pub fn hierarchical_path_value(path: &Path) -> f32 {
    let mut acc = 0.0f32;
    let mut weight = 1.0f32;
    for seg in path.components() {
        let s = seg.as_os_str().to_string_lossy();
        let h = component_hash(&s) as f32 / u32::MAX as f32;
        acc += h * weight;
        weight *= 0.4;
    }
    acc.fract()
}

fn component_hash(s: &str) -> u32 {
    // FNV-1a — small, deterministic, decent avalanche
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

// ---- Polynomial approximations (Inigo Quilez) ----
// Coefficients are taken verbatim from published shader sources; clippy
// objects to the precision (more digits than f32 actually holds) but
// rounding them would silently shift palette colours. Suppress per-fn.

fn horner6(t: f32, c: [[f32; 3]; 7]) -> [f32; 3] {
    let mut r = c[6];
    for i in (0..6).rev() {
        for j in 0..3 {
            r[j] = r[j] * t + c[i][j];
        }
    }
    r
}

#[allow(clippy::excessive_precision)]
fn poly_viridis(t: f32) -> [f32; 3] {
    horner6(
        t,
        [
            [0.277727_3, 0.005407_344, 0.334099_8],
            [0.105093_0, 1.404613, 1.384590],
            [-0.330861_8, 0.214847, 0.09509516],
            [-4.634230, -5.799100, -19.33244],
            [6.228269, 14.17993, 56.69055],
            [4.776384, -13.74514, -65.353],
            [-5.435455, 4.645852, 26.3124],
        ],
    )
}

#[allow(clippy::excessive_precision)]
fn poly_magma(t: f32) -> [f32; 3] {
    horner6(
        t,
        [
            [-0.002136485, -0.000749655, -0.005386127],
            [0.251723, 0.677381, 2.494659],
            [8.353717, 1.561712, 0.190912],
            [-27.66888, -6.941049, -19.42667],
            [52.17613, 11.60900, 44.35671],
            [-50.76863, -12.86618, -38.05349],
            [18.65459, 4.139130, 12.10796],
        ],
    )
}

#[allow(clippy::excessive_precision)]
fn poly_plasma(t: f32) -> [f32; 3] {
    horner6(
        t,
        [
            [0.058873_1, 0.029624_5, 0.526474_2],
            [2.176514, 0.238383, 0.838522],
            [-2.689460, -7.455851, 3.0993395],
            [6.130348, 32.46721, -12.51845],
            [-11.10743, -60.58235, 22.36433],
            [10.02306, 53.26092, -20.81009],
            [-3.658636, -18.86244, 7.452109],
        ],
    )
}

#[allow(clippy::excessive_precision)]
fn poly_turbo(t: f32) -> [f32; 3] {
    horner6(
        t,
        [
            [0.135955_8, 0.091791_38, 0.106408_4],
            [4.617916, 2.196833, 11.45810],
            [-42.66032, 4.842665, -60.58000],
            [132.1311, -14.18550, 222.7762],
            [-152.9405, 4.272602, -413.4144],
            [59.28637, 2.823930, 339.2127],
            [0.0, 0.0, -104.1846],
        ],
    )
}

/// Cubehelix (Green 2011). Brightness is monotonic in `t`, so values
/// remain orderable even when printed grayscale. Hue rotates smoothly
/// through the spectrum.
fn cubehelix(t: f32) -> [f32; 3] {
    let start = 0.5;
    let rotations: f32 = -1.5;
    let hue: f32 = 1.2;
    let gamma: f32 = 1.0;

    let l = t.powf(gamma);
    let angle = std::f32::consts::TAU * (start / 3.0 + 1.0 + rotations * t);
    let amp = hue * l * (1.0 - l) / 2.0;
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let r = l + amp * (-0.14861 * cos_a + 1.78277 * sin_a);
    let g = l + amp * (-0.29227 * cos_a - 0.90649 * sin_a);
    let b = l + amp * (1.97294 * cos_a);
    [r, g, b]
}

/// Sunset — diverging cold→neutral→warm. Useful for Age where t=0.5 is the
/// median.
fn stops_sunset(t: f32) -> [f32; 3] {
    const STOPS: &[(f32, [f32; 3])] = &[
        (0.00, [0.012, 0.039, 0.196]),
        (0.20, [0.071, 0.349, 0.624]),
        (0.45, [0.831, 0.890, 0.937]),
        (0.55, [0.969, 0.882, 0.737]),
        (0.80, [0.886, 0.388, 0.157]),
        (1.00, [0.388, 0.027, 0.090]),
    ];
    lerp_stops(t, STOPS)
}

fn lerp_stops(t: f32, stops: &[(f32, [f32; 3])]) -> [f32; 3] {
    if t <= stops[0].0 {
        return stops[0].1;
    }
    if t >= stops[stops.len() - 1].0 {
        return stops[stops.len() - 1].1;
    }
    for w in stops.windows(2) {
        let (a_t, a_c) = w[0];
        let (b_t, b_c) = w[1];
        if t >= a_t && t <= b_t {
            let f = (t - a_t) / (b_t - a_t).max(1e-6);
            return [
                a_c[0] + f * (b_c[0] - a_c[0]),
                a_c[1] + f * (b_c[1] - a_c[1]),
                a_c[2] + f * (b_c[2] - a_c[2]),
            ];
        }
    }
    stops[stops.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_idx_is_dense_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in Palette::all() {
            assert!(seen.insert(p.idx()), "duplicate idx for {:?}", p);
        }
        assert_eq!(seen.len(), Palette::count() as usize);
    }

    #[test]
    fn sample_in_unit_box() {
        // All palettes must stay inside [0, 1]^3 across the full domain.
        for &p in Palette::all() {
            for i in 0..=20 {
                let t = i as f32 / 20.0;
                let c = sample_palette(p, t);
                for (k, v) in c.iter().enumerate() {
                    assert!(
                        (0.0..=1.0).contains(v),
                        "palette {:?} sample at t={} produced {}={}",
                        p,
                        t,
                        ["r", "g", "b"][k],
                        v
                    );
                }
            }
        }
    }

    #[test]
    fn hierarchical_path_groups_siblings() {
        // Files in the same directory should land in a narrower range than
        // files in unrelated directories.
        let sibling_a = Path::new("/repo/src/main.rs");
        let sibling_b = Path::new("/repo/src/util.rs");
        let unrelated = Path::new("/totally/different/path.bin");
        let a = hierarchical_path_value(sibling_a);
        let b = hierarchical_path_value(sibling_b);
        let u = hierarchical_path_value(unrelated);
        // Siblings differ only in the last (small-weight) segment, so they
        // should be much closer to each other than either is to `unrelated`.
        let sib_gap = (a - b).abs();
        let unrelated_gap = (a - u).abs().min((b - u).abs());
        assert!(
            sib_gap < unrelated_gap,
            "siblings gap {} should be < unrelated gap {}",
            sib_gap,
            unrelated_gap
        );
    }

    #[test]
    fn auto_route_covers_every_source() {
        // Compile-time exhaustiveness: ensures new sources can't silently
        // skip palette routing.
        use crate::MaterialSource::*;
        for s in [None, Extension, Path, Size, Age, Depth, Random] {
            let _ = auto_palette_for_source(s);
        }
    }
}
