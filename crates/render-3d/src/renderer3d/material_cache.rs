//! Per-path material classification cache and material settings helpers.
//!
//! Extracted from `lib.rs` (Stage B.1 of TODO4 roadmap). Pure mechanical
//! move — no behaviour change.

use pt_mats::{classify_path_filtered_id, MaterializeMode, MaterializeSettings};
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

/// Per-path material classification cache. Caches the final
/// `MaterialLibrary` index (u32), so legacy `MaterialClass` slots and
/// palette samples share a single lookup table. Invalidated only when
/// material settings change; survives layout/animation/camera updates.
#[derive(Default)]
pub(crate) struct MaterialCache {
    pub(crate) settings_hash: u64,
    pub(crate) ids_pbr: std::collections::HashMap<u32, u32>,
    pub(crate) ids_pt: std::collections::HashMap<u32, u32>,
}

impl MaterialCache {
    /// Drop cached classifications when mat-settings hash changes. Call once per frame
    /// before any classify_or_get calls.
    pub(crate) fn ensure(&mut self, opts: &Render3DOptions) {
        let h = mat_settings_hash(opts);
        if h != self.settings_hash {
            self.ids_pbr.clear();
            self.ids_pt.clear();
            self.settings_hash = h;
        }
    }

    /// Look up the cached material id for `path` or compute it. `is_pt` selects
    /// the PT-specific path (light overrides depend on `is_pt`). Returns a
    /// final `MaterialLibrary` index — either a legacy `MaterialClass`
    /// slot (for light/glass overrides) or a palette sample.
    pub(crate) fn classify_or_get(
        &mut self,
        path: &std::path::Path,
        size: u64,
        opts: &Render3DOptions,
        is_pt: bool,
    ) -> u32 {
        if opts.materialize_mode == MaterializeMode::None {
            // Library slot 0 == MaterialClass::Default (Pure grey plastic).
            return 0;
        }
        let path_str = path.to_string_lossy();
        let path_hash = name_hash(&path_str);
        let bucket = if is_pt {
            &mut self.ids_pt
        } else {
            &mut self.ids_pbr
        };
        if let Some(&id) = bucket.get(&path_hash) {
            return id;
        }
        let id = classify_path_filtered_id(
            path,
            size,
            path_hash,
            opts.materialize_mode,
            settings_from_opts(opts, is_pt),
        );
        bucket.insert(path_hash, id);
        id
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
    opts.mat_palette.hash(&mut h);
    opts.mat_path_hierarchical.hash(&mut h);
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
        palette: opts.mat_palette,
        path_hierarchical: opts.mat_path_hierarchical,
    }
}
