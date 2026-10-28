//! Per-path material classification cache and PT material expansion.
//!
//! `pt_material::MaterialLibrary` (carried inside
//! `Render3DOptions.material_library`) is the single source of truth
//! for cube materials. Per-cube materialisation is a two-step process:
//!
//! 1. `MaterialCache::classify_or_get` maps a path to a
//!    `material_index` in `0..library.len()` via
//!    `pt_mats::classify_to_index`.
//! 2. [`expand_pt_materials_and_ids`] builds a compact variant pool of
//!    `StandardSurfaceParams` (`PT_VARIANTS_PER_SLOT` variants per
//!    library slot via `Material::resolve_for_cube`) and maps each cube
//!    to its `(slot, variant)` entry in `0..materials.len()`.

use std::sync::Arc;

use glam::Mat4;
use pt_core::{GpuMaterial, Instance};
use pt_material::{MaterialLibrary, StandardSurfaceParams};
use pt_mats::{
    MaterialInput, MaterialSource, MaterializeSettings, classify_to_index, hierarchical_path_value,
};
use render_shared::{Render3DOptions, name_hash};

use crate::geometry::CubeInstance;
use crate::picking::PickingState;

/// Capacity of the PBR `materials_buf` storage buffer (entries × 160 B
/// each → 40 KiB at 256). Library indices visible to PBR are clamped to
/// `[0, MAX_MATERIAL_SLOTS)` so a user-grown library cannot make the
/// shader read past the buffer or the per-frame upload exceed buffer
/// capacity. PT-side resolution is unaffected (its material storage is
/// sized per-cube, not from this cap).
pub(crate) const MAX_MATERIAL_SLOTS: u32 = 256;

/// Number of pre-resolved material variants per library slot in the PT
/// compact material pool (`256 * 16 = 4096` max `GpuMaterial` entries).
pub(crate) const PT_VARIANTS_PER_SLOT: u32 = 16;

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
    /// How many cubes each `Render3DOptions.material_overrides` slot
    /// painted this pass. Counted only on the PBR pass
    /// (`is_pt == false`) so the UI gets one number per override,
    /// not double. Reset on `ensure()` settings-change AND on every
    /// fresh `collect_cubes` (via [`Self::reset_overlay_counts`]).
    overlay_applied: [u32; 2],
    weights_scratch: Vec<f32>,
}

impl Default for MaterialCache {
    fn default() -> Self {
        Self {
            settings_hash: 0,
            ids_pbr: std::collections::HashMap::new(),
            ids_pt: std::collections::HashMap::new(),
            scene_max_depth: 1,
            scene_max_size: 1,
            overlay_applied: [0, 0],
            weights_scratch: Vec::new(),
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
            self.overlay_applied = [0, 0];
        }
    }

    /// Per-override cube counts from the most recent PBR
    /// `collect_cubes` pass. Indexed parallel to
    /// `Render3DOptions.material_overrides`.
    pub fn overlay_applied(&self) -> [u32; 2] {
        self.overlay_applied
    }

    /// Reset both overlay counters. Called at the start of each
    /// `collect_cubes` pass so the cubes we count this frame are
    /// only the ones that survive into the live instance buffer.
    pub(crate) fn reset_overlay_counts(&mut self) {
        self.overlay_applied = [0, 0];
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

    /// Look up the cached material id for `path` or compute it.
    /// `is_pt` selects the PT-specific cache bucket. `depth` is the
    /// node's position in the directory tree (drives `Depth` source).
    /// `position` is the world-space cube centre — required by the
    /// `Spatial` / `Perlin` distributions; pass `[0.0; 3]` from
    /// callers that don't have it (Direct / Stratified / Gradient
    /// ignore it).
    pub(crate) fn classify_or_get(
        &mut self,
        path: &std::path::Path,
        size: u64,
        depth: u32,
        position: [f32; 3],
        opts: &Render3DOptions,
        is_pt: bool,
    ) -> u32 {
        // Clamp visible library size to the PBR upload cap so
        // classification can never emit an index past the GPU-visible
        // slot range.
        let cap = MAX_MATERIAL_SLOTS as usize;
        let raw_lib = &opts.material_library;
        let lib_len = raw_lib.len().min(cap);
        if lib_len == 0 || opts.mat_source == MaterialSource::None {
            return 0;
        }
        let path_key = match path.to_str() {
            Some(s) => name_hash(s),
            None => name_hash(&path.to_string_lossy()),
        };
        let settings = settings_from_opts(opts, is_pt);
        // Position-dependent distributions can't use the per-path
        // cache: re-layout changes positions while leaving paths
        // intact, and we'd return stale slot picks. Recompute on
        // every call instead — `classify_to_index` is cheap.
        let position_dependent = matches!(
            settings.distribution,
            pt_mats::MaterialDistribution::Spatial | pt_mats::MaterialDistribution::Perlin
        );
        let has_active_override = opts.material_overrides.iter().any(|over| {
            over.enabled && over.material_uuid.is_some() && over.probability.clamp(0.0, 1.0) > 0.0
        });
        if !position_dependent && !has_active_override {
            let bucket = if is_pt {
                &self.ids_pt
            } else {
                &self.ids_pbr
            };
            if let Some(&cached) = bucket.get(&path_key) {
                return cached;
            }
        }
        // `Extension` source needs all cubes that share an extension
        // to land on the *same* source value — that's the whole point
        // of the mode. The previous implementation hashed the full
        // path for both `name_hash` and `path_hash`, which made
        // Extension and Path behave identically. Now `name_hash` is
        // a hash of just the extension; `path_hash` keeps the full
        // path.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let ext_key = name_hash(ext);
        let input = MaterialInput {
            name_hash: ext_key,
            path_hash: path_key,
            size,
            max_size: self.scene_max_size,
            depth,
            max_depth: self.scene_max_depth,
            // No file mtime is plumbed through the pipeline yet, so Age
            // falls back to a deterministic hash-based proxy.
            age_normalized: (path_key as f32) / (u32::MAX as f32),
            position,
            path_hierarchical_value: hierarchical_path_value(path),
        };

        // Base classification — cached by `path_key` for non-position
        // distributions. Reuses `weights_scratch` on cache miss to avoid
        // per-cube heap allocations.
        let base_id = {
            let bucket = if is_pt {
                &mut self.ids_pt
            } else {
                &mut self.ids_pbr
            };
            if !position_dependent && let Some(&cached) = bucket.get(&path_key) {
                cached
            } else {
                self.weights_scratch.clear();
                self.weights_scratch.extend(
                    raw_lib.materials[..lib_len]
                        .iter()
                        .map(|m| m.weight.max(0.0)),
                );
                let id = classify_to_index(&input, &settings, &self.weights_scratch);
                if !position_dependent {
                    bucket.insert(path_key, id);
                }
                id
            }
        };

        // Post-classify overrides — two independent slots, each
        // with its own (distribution, seed) vote. The base id is
        // kept when no override claims the cube; otherwise the
        // last override that claims the cube wins (deterministic
        // by array order, not by random precedence). Overrides
        // are evaluated on EVERY call, not cached — the base
        // cache above already cut the expensive part; the per-
        // override coin flip + UUID lookup is O(lib_len).
        let mut id = base_id;
        for (i, over) in opts.material_overrides.iter().enumerate() {
            if !over.enabled {
                continue;
            }
            let Some(uuid) = over.material_uuid else {
                continue;
            };
            let p = over.probability.clamp(0.0, 1.0);
            if p <= 0.0 {
                continue;
            }
            let picker = pt_mats::override_picker(
                &input,
                over.distribution,
                over.band_count,
                over.spatial_scale,
                over.seed,
            );
            if picker < p
                && let Some(slot) = raw_lib.materials[..lib_len]
                    .iter()
                    .position(|m| m.uuid == uuid)
            {
                id = slot as u32;
                if !is_pt && i < self.overlay_applied.len() {
                    self.overlay_applied[i] = self.overlay_applied[i].saturating_add(1);
                }
            }
        }
        id
    }
}

/// Hash of all option fields the material classifier reads. Excludes runtime knobs
/// like `materialize_mix` (handled in shader) and animation/camera state.
pub(crate) fn mat_settings_hash(opts: &Render3DOptions) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    opts.mat_seed.hash(&mut h);
    (opts.mat_source as u8).hash(&mut h);
    (opts.mat_distribution as u8).hash(&mut h);
    opts.mat_quant_levels.hash(&mut h);
    opts.mat_band_count.hash(&mut h);
    opts.mat_spatial_scale.to_bits().hash(&mut h);
    opts.mat_include_dirs.hash(&mut h);
    opts.mat_palette.hash(&mut h);
    opts.mat_path_hierarchical.hash(&mut h);
    // Each override fully contributes — toggling the enabled flag,
    // changing the target material, the probability, the
    // distribution shape, or the seed must invalidate the cache.
    for over in &opts.material_overrides {
        over.enabled.hash(&mut h);
        if let Some(uuid) = over.material_uuid {
            uuid.hash(&mut h);
        } else {
            0u128.hash(&mut h);
        }
        over.probability.to_bits().hash(&mut h);
        (over.distribution as u8).hash(&mut h);
        over.band_count.hash(&mut h);
        over.spatial_scale.to_bits().hash(&mut h);
        over.seed.hash(&mut h);
    }
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
        is_pt,
        seed: opts.mat_seed,
        source: opts.mat_source,
        distribution: opts.mat_distribution,
        quant_levels: opts.mat_quant_levels,
        band_count: opts.mat_band_count,
        spatial_scale: opts.mat_spatial_scale,
        palette: opts.mat_palette,
        path_hierarchical: opts.mat_path_hierarchical,
        ..MaterializeSettings::default()
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
    // Library contents: UUID identifies each slot, params + variance
    // drive resolution. Hash 36 lanes (9 vec4s × 4 channels) per slot.
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
    _picking: &PickingState,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mat_settings_hash.hash(&mut h);
    scene_meta.0.hash(&mut h);
    scene_meta.1.hash(&mut h);
    instances.len().hash(&mut h);
    (opts.mat_source as u8).hash(&mut h);
    pt_expand_opts_hash(opts).hash(&mut h);
    if let Some(first) = instances.first() {
        first.object_id.hash(&mut h);
        first.material_id.hash(&mut h);
    }
    if let Some(last) = instances.last() {
        last.object_id.hash(&mut h);
        last.material_id.hash(&mut h);
    }
    h.finish()
}

/// Build the compact `GpuMaterial` variant pool and per-instance material IDs for the PT pipeline.
///
/// Pre-resolves `PT_VARIANTS_PER_SLOT` variants per library slot (`<= 4096` entries total)
/// and maps each cube to `slot * PT_VARIANTS_PER_SLOT + variant` using its pre-classified
/// `inst.material_id` and deterministic `inst.object_id` hash.
pub(crate) fn expand_pt_materials_and_ids(
    library: &MaterialLibrary,
    _mat_cache: &mut MaterialCache,
    _picking: &PickingState,
    instances: &[CubeInstance],
    opts: &Render3DOptions,
) -> (Vec<GpuMaterial>, Vec<u32>) {
    let lib_size = library.len().min(MAX_MATERIAL_SLOTS as usize);
    let mut materials = Vec::with_capacity((lib_size * PT_VARIANTS_PER_SLOT as usize).max(1));
    if lib_size > 0 && opts.mat_source != MaterialSource::None {
        for slot_idx in 0..lib_size {
            let mat = &library.materials[slot_idx];
            for v in 0..PT_VARIANTS_PER_SLOT {
                let resolved = mat.resolve_for_cube(u64::from(v) + 1).sanitized_for_gpu();
                materials.push(GpuMaterial::from(&resolved));
            }
        }
    } else {
        let fallback = StandardSurfaceParams::default().sanitized_for_gpu();
        materials.push(GpuMaterial::from(&fallback));
    }

    let material_ids: Vec<u32> = if lib_size > 0 && opts.mat_source != MaterialSource::None {
        let max_slot = (lib_size - 1) as u32;
        instances
            .iter()
            .map(|inst| {
                let slot = inst.material_id.min(max_slot);
                let variant =
                    (inst.object_id.wrapping_mul(0x9E37_79B9) >> 28) % PT_VARIANTS_PER_SLOT;
                slot * PT_VARIANTS_PER_SLOT + variant
            })
            .collect()
    } else {
        vec![0; instances.len()]
    };

    (materials, material_ids)
}

pub(crate) fn build_pt_instances(
    instances: &[CubeInstance],
    material_ids: &[u32],
) -> Vec<Instance> {
    use rayon::prelude::*;
    if instances.len() >= 4096 {
        instances
            .par_iter()
            .zip(material_ids.par_iter())
            .map(|(inst, &material_id)| {
                let model = Mat4::from_cols_array_2d(&inst.model);
                Instance::from_cube(model, inst.color, inst.object_id, material_id)
            })
            .collect()
    } else {
        instances
            .iter()
            .zip(material_ids.iter())
            .map(|(inst, &material_id)| {
                let model = Mat4::from_cols_array_2d(&inst.model);
                Instance::from_cube(model, inst.color, inst.object_id, material_id)
            })
            .collect()
    }
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
    if let Some(entry) = cache_slot
        && entry.key == key && entry.material_ids.len() == instances.len() {
            let pt_instances = build_pt_instances(instances, &entry.material_ids);
            return (Arc::clone(&entry.materials), pt_instances);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_material_stride_and_pt_expansion_parity() {
        assert_eq!(
            std::mem::size_of::<GpuMaterial>(),
            160,
            "GpuMaterial must match 160B (10 x vec4<f32>) shader layout"
        );
        assert_eq!(
            std::mem::size_of::<StandardSurfaceParams>(),
            144,
            "StandardSurfaceParams has 9 x vec4<f32> (144B) before GpuMaterial::from conversion"
        );

        let mut mat_cache = MaterialCache::default();
        let picking = PickingState::new();
        let opts = Render3DOptions {
            mat_source: MaterialSource::Extension,
            ..Render3DOptions::default()
        };
        let lib = opts.material_library.clone();

        let instances = vec![
            CubeInstance {
                model: Mat4::IDENTITY.to_cols_array_2d(),
                color: [0.8, 0.2, 0.1, 1.0],
                object_id: 1,
                hash: 101,
                material_id: 0,
                _pad: 0,
            },
            CubeInstance {
                model: Mat4::IDENTITY.to_cols_array_2d(),
                color: [0.1, 0.7, 0.3, 1.0],
                object_id: 2,
                hash: 202,
                material_id: 1,
                _pad: 0,
            },
        ];

        let (materials, material_ids) =
            expand_pt_materials_and_ids(&lib, &mut mat_cache, &picking, &instances, &opts);
        assert_eq!(
            materials.len(),
            lib.len().min(MAX_MATERIAL_SLOTS as usize) * PT_VARIANTS_PER_SLOT as usize
        );
        assert_eq!(material_ids.len(), instances.len());
        for &id in &material_ids {
            assert!((id as usize) < materials.len());
        }
    }
}
