//! Per-path material classification cache and material settings helpers.
//!
//! Extracted from `lib.rs` (Stage B.1 of TODO4 roadmap). Pure mechanical
//! move — no behaviour change.

use std::sync::Arc;

use glam::Mat4;
use pt_core::{GpuMaterial, Instance};
use pt_mats::{
    classify_to_id, hierarchical_path_value, MaterialClass, MaterialInput, MaterialLibrary,
    MaterializeMode, MaterializeSettings,
};
use render_shared::{name_hash, Render3DOptions};

use crate::geometry::CubeInstance;
use crate::picking::PickingState;
use crate::renderer3d::helpers::{apply_glass_controls, hash_f32, mix_material};

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

    /// Read the most recent scene-level bounds (max_depth, max_size).
    /// Returns `(1, 1)` before `set_scene_meta` has been called.
    pub(crate) fn scene_meta(&self) -> (u32, u64) {
        (self.scene_max_depth, self.scene_max_size)
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

// --- Path tracing: expanded material table + per-instance ids (cached across frames) ---

/// Cache GPU material table and parallel `material_id` list when the scan + material
/// knobs that affect expansion are unchanged (animation can still move cubes).
pub(crate) struct PtExpandCacheEntry {
    pub key: u64,
    pub materials: Arc<Vec<GpuMaterial>>,
    pub material_ids: Vec<u32>,
}

fn pt_expand_opts_hash(opts: &Render3DOptions) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    opts.mat_light_intensity.to_bits().hash(&mut h);
    opts.mat_light_color_randomness.to_bits().hash(&mut h);
    opts.pt_global_transparency.to_bits().hash(&mut h);
    (opts.pt_global_glass as u8).hash(&mut h);
    opts.pt_glass_specular.to_bits().hash(&mut h);
    opts.pt_glass_base.to_bits().hash(&mut h);
    opts.pt_glass_roughness.to_bits().hash(&mut h);
    opts.pt_glass_ior.to_bits().hash(&mut h);
    opts.pt_glass_dispersion.to_bits().hash(&mut h);
    opts.pt_glass_temp.to_bits().hash(&mut h);
    opts.pt_glass_thin.hash(&mut h);
    h.finish()
}

pub(crate) fn pt_expand_cache_key(
    instances: &[CubeInstance],
    opts: &Render3DOptions,
    mat_settings_hash: u64,
    scene_meta: (u32, u64),
    picking: &PickingState,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mat_settings_hash.hash(&mut h);
    scene_meta.0.hash(&mut h);
    scene_meta.1.hash(&mut h);
    instances.len().hash(&mut h);
    pt_expand_opts_hash(opts).hash(&mut h);
    for inst in instances {
        inst.object_id.hash(&mut h);
        for c in &inst.color {
            c.to_bits().hash(&mut h);
        }
        if opts.materialize_mode == MaterializeMode::None {
            0xF0u8.hash(&mut h);
        } else if let Some(path) = picking.path_for_id(inst.object_id) {
            let is_dir = picking.is_dir_for_id(inst.object_id).unwrap_or(false);
            let size = picking.size_for_id(inst.object_id).unwrap_or(0);
            1u8.hash(&mut h);
            is_dir.hash(&mut h);
            size.hash(&mut h);
            name_hash(&path.to_string_lossy()).hash(&mut h);
        } else {
            2u8.hash(&mut h);
        }
    }
    h.finish()
}

/// Build expanded `GpuMaterial` table + per-cube material indices for PT.
/// Caller must run [`MaterialCache::ensure`] first.
pub(crate) fn expand_pt_materials_and_ids(
    library: &MaterialLibrary,
    mat_cache: &mut MaterialCache,
    picking: &PickingState,
    instances: &[CubeInstance],
    opts: &Render3DOptions,
) -> (Vec<GpuMaterial>, Vec<u32>) {
    let mut materials = library.materials().to_vec();
    let light_intensity = opts.mat_light_intensity.clamp(0.0, 10.0);
    let light_color_rand = opts.mat_light_color_randomness.clamp(0.0, 1.0);
    let light_variants = if light_color_rand > 0.0 { 16u32 } else { 1u32 };
    let mut light_variant_ids: std::collections::HashMap<u64, u32> =
        std::collections::HashMap::new();
    let transparency = opts.pt_global_transparency.clamp(0.0, 1.0);
    if transparency > 0.0 {
        let glass_class = opts.pt_global_glass.to_material_class();
        let glass_id = library.material_id(glass_class) as usize;
        let glass = materials.get(glass_id).copied().unwrap_or(materials[0]);
        let glass = apply_glass_controls(glass, opts);
        for mat in &mut materials {
            *mat = mix_material(*mat, glass, transparency);
        }
    }
    let default_id = library.material_id(MaterialClass::Default);

    let mut material_ids = Vec::with_capacity(instances.len());
    for inst in instances {
        let material_id = if opts.materialize_mode != MaterializeMode::None {
            let path_opt = picking.path_for_id(inst.object_id);
            let is_dir = picking.is_dir_for_id(inst.object_id).unwrap_or(false);
            let size = picking.size_for_id(inst.object_id).unwrap_or(0);
            if let Some(path) = path_opt {
                if is_dir && !opts.mat_include_dirs {
                    default_id
                } else {
                    let hash = name_hash(&path.to_string_lossy());
                    let base_id = mat_cache.classify_or_get(path, size, 0, opts, true);
                    let class_opt = MaterialClass::from_id(base_id);
                    let is_light = class_opt.map(|c| c.is_light()).unwrap_or(false);
                    if is_light && (light_intensity != 1.0 || light_color_rand > 0.0) {
                        let class = class_opt.expect("is_light implies class_opt is Some");
                        let bucket = if light_variants > 1 {
                            hash % light_variants
                        } else {
                            0
                        };
                        let vkey = ((class as u64) << 32) | bucket as u64;
                        if let Some(&id) = light_variant_ids.get(&vkey) {
                            id
                        } else {
                            let mut m = materials[base_id as usize];
                            m.emission_color_weight[3] *= light_intensity;
                            if light_color_rand > 0.0 {
                                let r = 1.0
                                    + (hash_f32(hash, 0xA1u32) - 0.5)
                                        * 2.0
                                        * (0.3 * light_color_rand);
                                let g = 1.0
                                    + (hash_f32(hash, 0xB2u32) - 0.5)
                                        * 2.0
                                        * (0.3 * light_color_rand);
                                let b = 1.0
                                    + (hash_f32(hash, 0xC3u32) - 0.5)
                                        * 2.0
                                        * (0.3 * light_color_rand);
                                m.emission_color_weight[0] =
                                    (m.emission_color_weight[0] * r).max(0.0);
                                m.emission_color_weight[1] =
                                    (m.emission_color_weight[1] * g).max(0.0);
                                m.emission_color_weight[2] =
                                    (m.emission_color_weight[2] * b).max(0.0);
                            }
                            materials.push(m);
                            let new_id = (materials.len() - 1) as u32;
                            light_variant_ids.insert(vkey, new_id);
                            new_id
                        }
                    } else if is_light && light_intensity != 1.0 {
                        let mut m = materials[base_id as usize];
                        m.emission_color_weight[3] *= light_intensity;
                        materials.push(m);
                        (materials.len() - 1) as u32
                    } else {
                        base_id
                    }
                }
            } else {
                default_id
            }
        } else {
            default_id
        };
        material_ids.push(material_id);
    }
    (materials, material_ids)
}

pub(crate) fn build_pt_instances(instances: &[CubeInstance], material_ids: &[u32]) -> Vec<Instance> {
    instances
        .iter()
        .zip(material_ids.iter())
        .map(|(inst, &material_id)| {
            let model = Mat4::from_cols_array_2d(&inst.model);
            Instance::from_cube(model, inst.color, inst.object_id, material_id)
        })
        .collect()
}

pub(crate) fn prepare_pt_expanded_materials(
    library: &MaterialLibrary,
    mat_cache: &mut MaterialCache,
    picking: &PickingState,
    cache_slot: &mut Option<PtExpandCacheEntry>,
    instances: &[CubeInstance],
    opts: &Render3DOptions,
) -> (Arc<Vec<GpuMaterial>>, Vec<Instance>) {
    mat_cache.ensure(opts);
    let key = pt_expand_cache_key(
        instances,
        opts,
        mat_cache.settings_hash,
        mat_cache.scene_meta(),
        picking,
    );
    if let Some(entry) = cache_slot {
        if entry.key == key && entry.material_ids.len() == instances.len() {
            let pt_instances = build_pt_instances(instances, &entry.material_ids);
            return (Arc::clone(&entry.materials), pt_instances);
        }
    }
    let (materials, material_ids) =
        expand_pt_materials_and_ids(library, mat_cache, picking, instances, opts);
    let arc = Arc::new(materials);
    *cache_slot = Some(PtExpandCacheEntry {
        key,
        materials: Arc::clone(&arc),
        material_ids,
    });
    let entry = cache_slot.as_ref().expect("just inserted");
    let pt_instances = build_pt_instances(instances, &entry.material_ids);
    (arc, pt_instances)
}
