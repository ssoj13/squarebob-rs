//! Per-path material classification cache and PT material expansion.
//!
//! Phase 4 rewrite: the legacy `pt_mats::MaterialLibrary` (1500-slot
//! palette + `MaterialClass` enum) is gone. The single source of truth
//! is now `pt_material::MaterialLibrary` carried inside
//! `Render3DOptions.material_library`. Per-cube materialisation:
//!
//! 1. `MaterialCache::classify_or_get` maps a path to a
//!    `material_index` in `0..library.len()`.
//! 2. [`expand_pt_materials_and_ids`] resolves *one*
//!    `StandardSurfaceParams` per cube (`Material::resolve_for_cube`),
//!    producing a `(materials, ids)` pair where `ids[i] == i`. This
//!    natively supports per-attribute variance (each cube has its own
//!    resolved record) at the cost of a slightly larger GPU material
//!    storage buffer (~144 bytes per cube).

use std::sync::Arc;

use glam::Mat4;
use pt_core::{GpuMaterial, Instance};
use pt_material::{MaterialLibrary, StandardSurfaceParams};
use pt_mats::{
    classify_to_index, hierarchical_path_value, MaterialInput, MaterializeMode,
    MaterializeSettings,
};
use render_shared::{name_hash, Render3DOptions};

use crate::geometry::CubeInstance;
use crate::picking::PickingState;

/// Capacity of the PBR `materials_buf` storage buffer (entries × 144 B
/// each → 36 KiB at 256). Library indices visible to PBR are clamped to
/// `[0, MAX_MATERIAL_SLOTS)` so a user-grown library cannot make the
/// shader read past the buffer or the per-frame upload exceed buffer
/// capacity. PT-side resolution is unaffected (its material storage is
/// sized per-cube, not from this cap).
pub(crate) const MAX_MATERIAL_SLOTS: u32 = 256;

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
/// `MaterialLibrary` index (u32) so identical paths reuse the
/// classification result. Invalidated whenever material settings change
/// or scene normalisation bounds shift (depth / size sources depend on
/// scene-wide max). Survives layout/animation/camera updates.
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
    /// selects the PT-specific cache bucket. `depth` is the node's
    /// position in the directory tree — required for the `Depth` source.
    pub(crate) fn classify_or_get(
        &mut self,
        path: &std::path::Path,
        size: u64,
        depth: u32,
        opts: &Render3DOptions,
        is_pt: bool,
    ) -> u32 {
        // Clamp visible library size to the PBR upload cap so
        // classification can never emit an index past the GPU-visible
        // slot range.
        let cap = MAX_MATERIAL_SLOTS as usize;
        let raw_lib = &opts.material_library;
        let lib_len = raw_lib.len().min(cap);
        if lib_len == 0 || opts.materialize_mode == MaterializeMode::None {
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
        // Sync legacy `materialize_mode` → `source` so classify_to_index
        // sees the right source even if callers updated only the legacy field.
        settings.source = opts.materialize_mode.to_source();
        let input = MaterialInput {
            name_hash: key,
            path_hash: key,
            size,
            max_size: self.scene_max_size,
            depth,
            max_depth: self.scene_max_depth,
            // No file mtime is plumbed through the pipeline yet, so Age
            // falls back to a deterministic hash-based proxy.
            age_normalized: (key as f32) / (u32::MAX as f32),
            position: [0.0, 0.0, 0.0],
            path_hierarchical_value: hierarchical_path_value(path),
        };
        // Per-slot user-editable weights drive the distribution: each
        // slot claims `weight / sum(weights)` of cubes. Default weight
        // is 1.0 → uniform. Sliced to the same cap so PBR storage and
        // classification stay in sync.
        let weights: Vec<f32> = raw_lib.materials[..lib_len]
            .iter()
            .map(|m| m.weight.max(0.0))
            .collect();
        let id = classify_to_index(&input, &settings, &weights);
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
    // Library identity drives the classify_to_index range AND per-cube
    // index meaning. Hash slot UUIDs (cheap, identity-only) AND
    // weights so that reorders / deletions / insertions AND weight
    // edits invalidate `ids_pbr` / `ids_pt`. PT-side params+variance
    // hashing lives in `pt_expand_opts_hash` — PBR doesn't need that
    // because the materials buffer is re-uploaded every frame.
    for m in &opts.material_library.materials {
        m.uuid.hash(&mut h);
        m.weight.to_bits().hash(&mut h);
    }
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

// --- Path tracing: per-cube resolved material table + identity ids ---

/// Cache the per-cube material table when the scan + library + variance
/// state is unchanged (animation can still move cubes but the materials
/// themselves don't depend on transform).
pub(crate) struct PtExpandCacheEntry {
    pub key: u64,
    pub materials: Arc<Vec<GpuMaterial>>,
    pub material_ids: Vec<u32>,
}

/// Hash the bits of `opts` that influence per-cube material resolution
/// but don't appear in `mat_settings_hash` — primarily the library
/// contents (params + variance) and the PT-side glass UI knobs (kept
/// for the follow-up wiring; they don't drive expansion yet).
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
    // Library contents: UUID identifies each slot, params + variance
    // drive resolution. Hash 40 lanes (10 vec4s × 4 channels) per slot.
    for m in &opts.material_library.materials {
        m.uuid.hash(&mut h);
        let p: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&m.params));
        for x in p {
            x.to_bits().hash(&mut h);
        }
        let v: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&m.variance));
        for x in v {
            x.to_bits().hash(&mut h);
        }
    }
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

/// Build the per-cube `GpuMaterial` table for the PT pipeline.
///
/// Each cube becomes one entry in the output `Vec<GpuMaterial>`; the
/// `material_ids` vec is therefore the identity map `[0, 1, .., N-1]`.
/// This trades a slightly larger GPU buffer (~144 bytes per cube) for a
/// model that natively supports per-cube variance — each cube hashes
/// its `object_id` through `Material::resolve_for_cube`, so two cubes
/// pointing at the same library slot can land on different shades of
/// the same family.
pub(crate) fn expand_pt_materials_and_ids(
    library: &MaterialLibrary,
    mat_cache: &mut MaterialCache,
    picking: &PickingState,
    instances: &[CubeInstance],
    opts: &Render3DOptions,
) -> (Vec<GpuMaterial>, Vec<u32>) {
    // Mirror the PBR cap so a user-grown library cannot drive
    // `material_index` past the slot range the shader can read.
    let lib_size = library.len().min(MAX_MATERIAL_SLOTS as usize);
    let mut materials = Vec::with_capacity(instances.len());
    let mut material_ids = Vec::with_capacity(instances.len());

    for inst in instances {
        let lib_idx = if lib_size > 0 && opts.materialize_mode != MaterializeMode::None {
            let path_opt = picking.path_for_id(inst.object_id);
            let is_dir = picking.is_dir_for_id(inst.object_id).unwrap_or(false);
            let size = picking.size_for_id(inst.object_id).unwrap_or(0);
            if let Some(path) = path_opt {
                if is_dir && !opts.mat_include_dirs {
                    0
                } else {
                    mat_cache.classify_or_get(path, size, 0, opts, true) as usize
                }
            } else {
                0
            }
        } else {
            0
        };
        let lib_idx = lib_idx.min(lib_size.saturating_sub(1));
        let resolved: StandardSurfaceParams = if lib_size > 0 {
            library.materials[lib_idx].resolve_for_cube(inst.object_id as u64)
        } else {
            StandardSurfaceParams::default()
        };
        // `StandardSurfaceParams` and `GpuMaterial` are `#[repr(C)]` with
        // matching field order and identical size (144 bytes / 9 × vec4).
        // `bytemuck::cast` is the safe equivalent of a transmute: it
        // compiles to a no-op and panics at build-time if layouts ever
        // diverge.
        let gpu: GpuMaterial = bytemuck::cast(resolved);
        materials.push(gpu);
        material_ids.push(materials.len() as u32 - 1);
    }
    (materials, material_ids)
}

pub(crate) fn build_pt_instances(
    instances: &[CubeInstance],
    material_ids: &[u32],
) -> Vec<Instance> {
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
    let entry = cache_slot.insert(PtExpandCacheEntry {
        key,
        materials: Arc::clone(&arc),
        material_ids,
    });
    let pt_instances = build_pt_instances(instances, &entry.material_ids);
    (arc, pt_instances)
}
