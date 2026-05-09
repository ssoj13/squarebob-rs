//! Per-path material classification cache and material settings helpers.
//!
//! Extracted from `lib.rs` (Stage B.1 of TODO4 roadmap). Pure mechanical
//! move — no behaviour change.

use pt_mats::{classify_path_filtered, MaterialClass, MaterializeMode, MaterializeSettings};
use render_shared::{name_hash, Render3DOptions};

/// Global material params shared across all cubes in the PBR shader. Currently holds
/// only the materialize-mix (instance color → library albedo blend factor). Kept
/// separate from per-instance data so the slider doesn't trigger an instance rebuild.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MatGlobalUniform {
    pub(crate) materialize_mix: f32,
    pub(crate) _pad: [f32; 3],
}

impl Default for MatGlobalUniform {
    fn default() -> Self {
        Self {
            materialize_mix: 1.0,
            _pad: [0.0; 3],
        }
    }
}

/// Per-path material classification cache. Invalidated only when material settings
/// change; survives layout/animation/camera updates.
#[derive(Default)]
pub(crate) struct MaterialCache {
    pub(crate) settings_hash: u64,
    pub(crate) classes_pbr: std::collections::HashMap<u32, MaterialClass>,
    pub(crate) classes_pt: std::collections::HashMap<u32, MaterialClass>,
}

impl MaterialCache {
    /// Drop cached classifications when mat-settings hash changes. Call once per frame
    /// before any classify_or_get calls.
    pub(crate) fn ensure(&mut self, opts: &Render3DOptions) {
        let h = mat_settings_hash(opts);
        if h != self.settings_hash {
            self.classes_pbr.clear();
            self.classes_pt.clear();
            self.settings_hash = h;
        }
    }

    /// Look up the cached class for `path` or compute it. `is_pt` selects the PT-specific
    /// classification path (light overrides depend on `is_pt`).
    pub(crate) fn classify_or_get(
        &mut self,
        path: &std::path::Path,
        size: u64,
        opts: &Render3DOptions,
        is_pt: bool,
    ) -> MaterialClass {
        if opts.materialize_mode == MaterializeMode::None {
            return MaterialClass::Default;
        }
        let path_str = path.to_string_lossy();
        let path_hash = name_hash(&path_str);
        let bucket = if is_pt {
            &mut self.classes_pt
        } else {
            &mut self.classes_pbr
        };
        if let Some(&c) = bucket.get(&path_hash) {
            return c;
        }
        let class = classify_path_filtered(
            path,
            size,
            path_hash,
            opts.materialize_mode,
            settings_from_opts(opts, is_pt),
        );
        bucket.insert(path_hash, class);
        class
    }
}

/// Hash of all option fields the material classifier reads. Excludes runtime knobs
/// like `materialize_mix` (handled in shader) and animation/camera state.
pub(crate) fn mat_settings_hash(opts: &Render3DOptions) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (opts.materialize_mode as u8).hash(&mut h);
    opts.mat_allow_lights.hash(&mut h);
    opts.mat_light_prob.to_bits().hash(&mut h);
    opts.mat_light_warm.to_bits().hash(&mut h);
    opts.mat_light_cool.to_bits().hash(&mut h);
    opts.mat_allow_glass.hash(&mut h);
    opts.mat_glass_prob.to_bits().hash(&mut h);
    opts.mat_seed.hash(&mut h);
    (opts.mat_source as u8).hash(&mut h);
    (opts.mat_distribution as u8).hash(&mut h);
    opts.mat_quant_levels.hash(&mut h);
    opts.mat_band_count.hash(&mut h);
    opts.mat_spatial_scale.to_bits().hash(&mut h);
    opts.mat_include_dirs.hash(&mut h);
    h.finish()
}

/// Build `MaterializeSettings` from `Render3DOptions`. Single source of truth so PBR
/// and PT paths stay in sync.
pub(crate) fn settings_from_opts(opts: &Render3DOptions, is_pt: bool) -> MaterializeSettings {
    MaterializeSettings {
        allow_lights: opts.mat_allow_lights,
        light_prob: opts.mat_light_prob,
        light_warm: opts.mat_light_warm,
        light_cool: opts.mat_light_cool,
        allow_glass: opts.mat_allow_glass,
        glass_prob: opts.mat_glass_prob,
        is_pt,
        seed: opts.mat_seed,
        source: opts.mat_source,
        distribution: opts.mat_distribution,
        quant_levels: opts.mat_quant_levels,
        band_count: opts.mat_band_count,
        spatial_scale: opts.mat_spatial_scale,
    }
}
