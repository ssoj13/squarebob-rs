//! Per-path material classification cache and material settings helpers.
//!
//! Extracted from `lib.rs` (Stage B.1 of TODO4 roadmap). Pure mechanical
//! move — no behaviour change.

use pt_mats::{
    classify_to_id, hierarchical_path_value, MaterialInput, MaterializeMode, MaterializeSettings,
};
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
///
/// Scene-level metadata (`scene_max_depth`, `scene_max_size`) is
/// recomputed every frame by `instance_collect::collect_cubes` via
/// `set_scene_meta` before classification starts. Without this, the
/// `Depth` source would normalise against `max_depth=1` (collapsing to
/// 0) and the `Size` source would normalise against `max_size=size`
/// (collapsing to 1).
pub(crate) struct MaterialCache {
    pub(crate) settings_hash: u64,
    pub(crate) ids_pbr: std::collections::HashMap<u32, u32>,
    pub(crate) ids_pt: std::collections::HashMap<u32, u32>,
    scene_max_depth: u32,
    scene_max_size: u64,
}

impl Default for MaterialCache {
    fn default() -> Self {
        Self {
            settings_hash: 0,
            ids_pbr: std::collections::HashMap::new(),
            ids_pt: std::collections::HashMap::new(),
            scene_max_depth: 1,
            scene_max_size: 1,
        }
    }
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

    /// Update scene-level normalisation bounds. Called once per frame by
    /// `collect_cubes` after pre-walking the tree. Clears caches when
    /// bounds change so `Depth`/`Size` sources stay consistent.
    pub(crate) fn set_scene_meta(&mut self, max_depth: u32, max_size: u64) {
        let max_depth = max_depth.max(1);
        let max_size = max_size.max(1);
        if max_depth != self.scene_max_depth || max_size != self.scene_max_size {
            self.scene_max_depth = max_depth;
            self.scene_max_size = max_size;
            self.ids_pbr.clear();
            self.ids_pt.clear();
        }
    }

    /// Look up the cached material id for `path` or compute it. `is_pt`
    /// selects the PT-specific path (light overrides depend on `is_pt`).
    /// `depth` is the node's position in the directory tree — required
    /// for the `Depth` source to produce meaningful values. PT callers
    /// that only need cached lookups can pass `0`; lookups by `path_hash`
    /// hit cache regardless of depth.
    pub(crate) fn classify_or_get(
        &mut self,
        path: &std::path::Path,
        size: u64,
        depth: u32,
        opts: &Render3DOptions,
        is_pt: bool,
    ) -> u32 {
        if opts.materialize_mode == MaterializeMode::None {
            // Library slot 0 == MaterialClass::Default (Pure grey plastic).
            return 0;
        }
        let path_str = path.to_string_lossy();
        let key = name_hash(&path_str);
        let bucket = if is_pt {
            &mut self.ids_pt
        } else {
            &mut self.ids_pbr
        };
        if let Some(&id) = bucket.get(&key) {
            return id;
        }
        let mut settings = settings_from_opts(opts, is_pt);
        // Sync legacy `materialize_mode` → `source` so classify_to_id sees
        // the right source even if callers updated only the legacy field.
        settings.source = opts.materialize_mode.to_source();
        let input = MaterialInput {
            name_hash: key,
            path_hash: key,
            size,
            max_size: self.scene_max_size,
            depth,
            max_depth: self.scene_max_depth,
            // No file mtime is plumbed through the pipeline yet, so Age
            // falls back to a deterministic hash-based proxy. Real mtime
            // wiring is a follow-up.
            age_normalized: (key as f32) / (u32::MAX as f32),
            position: [0.0, 0.0, 0.0],
            path_hierarchical_value: hierarchical_path_value(path),
        };
        let id = classify_to_id(&input, &settings);
        bucket.insert(key, id);
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
