# Autopilot Plan — Phase 4: Strip MaterialClass + Land pt-material as the One True Library

**Goal**: Replace the legacy discrete `pt_mats::MaterialClass` + `pt_mats::MaterialLibrary`
material system with the new `pt_material::MaterialLibrary` (already in
`crates/pt-material/`, already plumbed into `Render3DOptions.material_library`). After
this plan lands, every cube on screen gets a `StandardSurfaceParams` resolved
per-cube from the user-editable library, with optional per-attribute variance.

**Scope**: ~1000–1200 LoC across 6 files. Edit order matters — each step
*will* break the build until the next step lands. Build between phases A/B/C
to catch issues early.

---

## Pre-flight (5 min)

State to confirm before starting:

- [ ] `cargo build -p pt-material --release` passes (4/4 tests)
- [ ] `cargo build -p squarebob-widgets --release` passes
- [ ] `cargo build -p render-shared --release` passes
- [ ] `Render3DOptions.material_library: pt_material::MaterialLibrary` field exists
- [ ] `Render3DOptions::default()` constructs `material_library:
      pt_material::MaterialLibrary::default()`
- [ ] No uncommitted changes you don't want to keep

Run: `cd C:/projects/projects.rust.cg/squarebob-rs && python bootstrap.py b`
Expect: clean build, exit 0.

If anything above fails, **stop** and resolve before continuing.

---

## Phase A — pt-mats rip (60–90 min)

Goal: delete `MaterialClass` and friends from `pt-mats`, leaving only
classification logic (`MaterializeSettings`, `MaterialSource`, `MaterialDistribution`,
`MaterializeMode`, `MaterialInput`, palettes as math functions, `hierarchical_path_value`).

### A1. `crates/pt-mats/src/lib.rs`

Delete the following items (find by symbol, not line number — the file is 1267 lines):

- `pub enum MaterialClass { ... }` (~140 lines)
- `impl MaterialClass { color, id, from_id, is_emissive, is_transparent, is_light,
  is_temperature_light, is_glass, ALL constant }`
- `const BASE_CLASSES: &[MaterialClass] = &[...]`
- `const GLASS_CLASSES: &[MaterialClass] = &[...]`
- `const LIGHT_WARM: &[MaterialClass] = &[...]`
- `const LIGHT_NEUTRAL: &[MaterialClass] = &[...]`
- `const LIGHT_COOL: &[MaterialClass] = &[...]`
- `pub struct MaterialLibrary { materials: Vec<GpuMaterial> }` and its
  entire `impl` block (~150 lines)
- All `make_plastic / make_metal / make_glass / make_emissive / make_paint /
  make_gem / make_velvet / make_marble` helper functions (~120 lines)
- `material_for_class()` function
- `material_for_palette_sample()` function
- `pub fn classify_to_id` — replace with new `classify_to_index` (see A3)

Keep:
- `pub enum MaterializeMode { ... }` + impls
- `pub enum MaterialSource { ... }` + impls
- `pub enum MaterialDistribution { ... }` + impls
- `pub struct MaterializeSettings { ... }` + impl Default
- `pub struct MaterialInput { ... }` + impl Default
- Anything from `palette.rs` (re-exported via `pub use palette::*`)

### A2. `crates/pt-mats/src/palette.rs`

Strip MaterialLibrary-specific arithmetic:
- Delete `pub const BASE_LIBRARY_SIZE: u32 = 34;` line
- Delete `pub const PALETTE_BINS: u32 = 256;` line (still used? if any caller
  imports it, deal in A3/B/C — likely material_cache, instance_collect)

Keep:
- `pub enum Palette { Viridis, Magma, ... }`
- `pub fn sample_palette(p: Palette, t: f32) -> [f32; 3]`
- `pub fn auto_palette_for_source(s: MaterialSource) -> Palette`
- `pub fn hierarchical_path_value(path: &Path) -> f32`
- All polynomial helpers (`poly_viridis`, `poly_magma`, etc., `stops_sunset`,
  `cubehelix`, `lerp_stops`)

### A3. Add new `classify_to_index` in `pt-mats/src/lib.rs`

Replace `classify_to_id` with a stripped-down version that returns an index
into a user-supplied library (`library_size: u32`). Signature:

```rust
/// Map per-cube classification inputs to a `material_index` in
/// `0..library_size`. Replaces the legacy `classify_to_id` which
/// returned a `MaterialClass` slot id; the new function knows nothing
/// about discrete classes — it produces a numeric index that the
/// caller looks up in `pt_material::MaterialLibrary`.
pub fn classify_to_index(
    input: &MaterialInput,
    settings: &MaterializeSettings,
    library_size: u32,
) -> u32 {
    if library_size == 0 {
        return 0;
    }
    let bucket = match settings.source {
        MaterialSource::None => 0,
        MaterialSource::Extension => input.name_hash,
        MaterialSource::Path => {
            if settings.path_hierarchical {
                (input.path_hierarchical_value * u32::MAX as f32) as u32
            } else {
                input.path_hash
            }
        }
        MaterialSource::Size => {
            let t = (input.size as f64 / input.max_size.max(1) as f64) as f32;
            (t.clamp(0.0, 1.0) * u32::MAX as f32) as u32
        }
        MaterialSource::Age => (input.age_normalized.clamp(0.0, 1.0) * u32::MAX as f32) as u32,
        MaterialSource::Depth => {
            let t = input.depth as f32 / input.max_depth.max(1) as f32;
            (t.clamp(0.0, 1.0) * u32::MAX as f32) as u32
        }
        MaterialSource::Random => input.path_hash.wrapping_mul(2654435761),
    };

    match settings.distribution {
        MaterialDistribution::Direct => bucket % library_size,
        MaterialDistribution::Quantized => {
            let levels = settings.quant_levels.max(1).min(library_size);
            let t = bucket as f64 / u32::MAX as f64;
            ((t * levels as f64) as u32).min(levels - 1) * (library_size / levels)
        }
        MaterialDistribution::Bands => {
            let bands = settings.band_count.max(1);
            let band = bucket % bands;
            band * (library_size / bands.max(1))
        }
        MaterialDistribution::Spatial => {
            // Position-hash; spatial_scale unused for now (we'd need
            // position in MaterialInput to do real spatial Voronoi).
            // Fall back to bucket-modulo.
            bucket % library_size
        }
    }
}
```

### A4. Build pt-mats

```
cargo build -p pt-mats --release
```

Expect: clean. If errors mention `GpuMaterial` (was imported via `pt_core`), drop
the import; `pt-mats` no longer needs it.

---

## Phase B — render-shared cleanup (15 min)

### B1. `crates/render-shared/src/lib.rs`

Around line 250 (search `to_material_class`):

Delete `pub fn to_material_class(self) -> MaterialClass { ... }` method on
`GlassPreset`. The function is no longer callable from any of the new code
paths.

Update import at line 4:
```rust
// OLD:
use pt_mats::{MaterialClass, MaterialDistribution, MaterialSource, MaterializeMode, Palette};
// NEW (drop MaterialClass):
use pt_mats::{MaterialDistribution, MaterialSource, MaterializeMode, Palette};
```

### B2. Glass UI knobs — keep field-by-field, no class mapping

`pt_glass_specular / pt_glass_base / pt_glass_roughness / pt_glass_ior /
pt_glass_dispersion / pt_glass_temp / pt_glass_thin` stay as user-controllable
overrides that can be applied per-instance via the material editor or a global
override. For Phase 4 just keep the fields; we'll wire them in Phase 5/6.

### B3. Build

```
cargo build -p render-shared --release
```

Expect: clean (or one error pointing back at remaining `MaterialClass` import
somewhere — fix and rerun).

---

## Phase C — material_cache + instance_collect rewrite (90–120 min, the hard part)

### C1. `crates/render-3d/src/renderer3d/material_cache.rs`

This file is 413 lines, almost all of it has to go. Strategy: rewrite from
scratch. Save the old file as `.legacy.rs` for reference, then write the new
one with this structure:

```rust
//! Per-path material classification cache. Each cube ends up with a
//! `material_index` into `Render3DOptions.material_library`; the
//! per-cube material is resolved at expand-time via
//! `Material::resolve_for_cube` (applies variance).

use std::sync::Arc;
use glam::Mat4;
use pt_core::{GpuMaterial, Instance};
use pt_mats::{
    classify_to_index, hierarchical_path_value, MaterialInput, MaterializeMode,
    MaterializeSettings,
};
use pt_material::{MaterialLibrary, StandardSurfaceParams};
use render_shared::{name_hash, Render3DOptions};

use crate::geometry::CubeInstance;
use crate::picking::PickingState;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MatGlobalUniform {
    pub(crate) materialize_mix: f32,
    pub(crate) _pad: [f32; 3],
}

impl Default for MatGlobalUniform {
    fn default() -> Self { Self { materialize_mix: 1.0, _pad: [0.0; 3] } }
}

pub(crate) struct MaterialCache {
    pub(crate) settings_hash: u64,
    pub(crate) ids_pbr: std::collections::HashMap<u32, u32>,
    pub(crate) ids_pt: std::collections::HashMap<u32, u32>,
    scene_max_depth: u32,
    scene_max_size: u64,
}

impl Default for MaterialCache { /* same as before */ }

impl MaterialCache {
    pub(crate) fn ensure(&mut self, opts: &Render3DOptions) { /* same */ }
    pub(crate) fn scene_meta(&self) -> (u32, u64) { /* same */ }
    pub(crate) fn set_scene_meta(&mut self, max_depth: u32, max_size: u64) { /* same */ }

    pub(crate) fn classify_or_get(
        &mut self,
        path: &std::path::Path,
        size: u64,
        depth: u32,
        opts: &Render3DOptions,
        is_pt: bool,
    ) -> u32 {
        let library_size = opts.material_library.len() as u32;
        if library_size == 0 || opts.materialize_mode == MaterializeMode::None {
            return 0;
        }
        let path_str = path.to_string_lossy();
        let key = name_hash(&path_str);
        let bucket = if is_pt { &mut self.ids_pt } else { &mut self.ids_pbr };
        if let Some(&id) = bucket.get(&key) { return id; }
        let mut settings = settings_from_opts(opts, is_pt);
        settings.source = opts.materialize_mode.to_source();
        let input = MaterialInput {
            name_hash: key, path_hash: key, size,
            max_size: self.scene_max_size, depth,
            max_depth: self.scene_max_depth,
            age_normalized: (key as f32) / (u32::MAX as f32),
            position: [0.0, 0.0, 0.0],
            path_hierarchical_value: hierarchical_path_value(path),
        };
        let id = classify_to_index(&input, &settings, library_size);
        bucket.insert(key, id);
        id
    }
}

pub(crate) fn mat_settings_hash(opts: &Render3DOptions) -> u64 { /* same */ }
pub(crate) fn settings_from_opts(opts: &Render3DOptions, is_pt: bool) -> MaterializeSettings {
    /* same — drop palette field if it broke */
}

pub(crate) struct PtExpandCacheEntry {
    pub key: u64,
    pub materials: Arc<Vec<GpuMaterial>>,  // per-cube resolved Standard Surface params
    pub material_ids: Vec<u32>,            // identity map: cube_idx -> cube_idx
}

fn pt_expand_opts_hash(opts: &Render3DOptions) -> u64 {
    // Hash material_library uuids + variance state. Drop pt_glass_*
    // bytes from hash since those don't drive expansion anymore.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for m in &opts.material_library.materials {
        m.uuid.hash(&mut h);
        let p: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&m.params));
        for x in p { x.to_bits().hash(&mut h); }
        let v: &[f32] = bytemuck::cast_slice(std::slice::from_ref(&m.variance));
        for x in v { x.to_bits().hash(&mut h); }
    }
    h.finish()
}

pub(crate) fn pt_expand_cache_key(
    instances: &[CubeInstance], opts: &Render3DOptions,
    mat_settings_hash: u64, scene_meta: (u32, u64),
    picking: &PickingState,
) -> u64 {
    /* same shape — incorporate pt_expand_opts_hash */
}

/// Build per-cube resolved material table.
///
/// Each cube becomes one entry in the output `Vec<GpuMaterial>` — the
/// `material_ids` vec is then just `[0, 1, 2, ..., N-1]`. This
/// trades a slightly larger GPU buffer (~160 bytes per cube) for a
/// simpler model that natively supports per-cube variance.
pub(crate) fn expand_pt_materials_and_ids(
    library: &MaterialLibrary,
    mat_cache: &mut MaterialCache,
    picking: &PickingState,
    instances: &[CubeInstance],
    opts: &Render3DOptions,
) -> (Vec<GpuMaterial>, Vec<u32>) {
    let lib_size = library.len();
    let mut materials = Vec::with_capacity(instances.len());
    let mut material_ids = Vec::with_capacity(instances.len());
    for inst in instances {
        let lib_idx = if opts.materialize_mode != MaterializeMode::None {
            let path_opt = picking.path_for_id(inst.object_id);
            let is_dir = picking.is_dir_for_id(inst.object_id).unwrap_or(false);
            let size = picking.size_for_id(inst.object_id).unwrap_or(0);
            if let Some(path) = path_opt {
                if is_dir && !opts.mat_include_dirs {
                    0
                } else {
                    mat_cache.classify_or_get(path, size, 0, opts, true) as usize
                }
            } else { 0 }
        } else { 0 };
        let lib_idx = lib_idx.min(lib_size.saturating_sub(1));
        let resolved: StandardSurfaceParams = library.materials[lib_idx]
            .resolve_for_cube(inst.object_id as u64);
        // GpuMaterial layout MATCHES StandardSurfaceParams. Cast.
        let gpu: GpuMaterial = unsafe { std::mem::transmute(resolved) };
        materials.push(gpu);
        material_ids.push(materials.len() as u32 - 1);
    }
    (materials, material_ids)
}

pub(crate) fn build_pt_instances(
    instances: &[CubeInstance], material_ids: &[u32],
) -> Vec<Instance> { /* same */ }

pub(crate) fn prepare_pt_expanded_materials(
    library: &MaterialLibrary,
    mat_cache: &mut MaterialCache,
    picking: &PickingState,
    cache_slot: &mut Option<PtExpandCacheEntry>,
    instances: &[CubeInstance],
    opts: &Render3DOptions,
) -> (Arc<Vec<GpuMaterial>>, Vec<Instance>) { /* same shape, just calls new expand */ }
```

**Important**: `GpuMaterial` is in `pt-core/src/bvh.rs:100`. Its layout matches
`StandardSurfaceParams` exactly (documented in the comment). The
`transmute` is safe given matching `#[repr(C)]` + identical field order.

Alternative: change `pt-core` to re-export `StandardSurfaceParams` as a type
alias for `GpuMaterial`:
```rust
// pt-core/src/bvh.rs
pub type GpuMaterial = standard_surface::StandardSurfaceParams;
```
(Would require adding `standard-surface` as a dep to pt-core.) The transmute
is simpler for Phase 4 — alias migration can be Phase 4b.

### C2. `crates/render-3d/src/renderer3d/instance_collect.rs`

Around line 11 and line 258:

```rust
// OLD:
use pt_mats::{
    classify_to_id, hierarchical_path_value, MaterialClass, MaterialDistribution, MaterializeMode,
};
// NEW:
use pt_mats::{
    classify_to_index, hierarchical_path_value, MaterialDistribution, MaterializeMode,
};

// Line ~258 — old: self.material_library.material_id(MaterialClass::Default)
// New: simply `0` (the first library entry is the default).
let material_id = if opts.materialize_mode != MaterializeMode::None && allow_dirs {
    self.mat_cache.classify_or_get(&node.path, node.size, depth, opts, false)
} else {
    0
};
```

### C3. `crates/render-3d/src/renderer3d/material_cache.rs` — palette / glass mixing

Drop:
- `apply_glass_controls` import (helpers module) — function still exists but
  unused; leave or delete from `crates/render-3d/src/renderer3d/helpers.rs`
- `mix_material` import — same story
- `hash_f32` import — keep (used by other helpers? grep first)

### C4. `crates/render-3d/Cargo.toml`

Add `pt-material = { path = "../pt-material" }` to `[dependencies]`.
`crates/render-3d/src/renderer3d/material_cache.rs` will need
`use pt_material::MaterialLibrary;`.

### C5. Build

```
cargo build -p render-3d --release 2>&1 | tail -60
```

Expect a wall of errors. Work through them top-down:
- Drop any remaining `MaterialClass` imports
- Fix `pt_mats::MaterialLibrary` → `pt_material::MaterialLibrary` everywhere
- Adjust `material_library` access — was field on RenderState, now lives at
  `opts.material_library` in the new model

Once render-3d builds clean, the full workspace should too:
```
cd C:/projects/projects.rust.cg/squarebob-rs && python bootstrap.py b
```

---

## Phase D — verify no regressions (15 min)

- [ ] `python bootstrap.py b` — clean release build
- [ ] Run squarebob, load a folder, switch to 3D
- [ ] All cubes visible, no black holes, no garbled materials
- [ ] Materialize mode dropdown (None / ByExtension / ByPath / BySize / ByAge /
      Random) all work — produces variety across cubes
- [ ] Variance test: in `Render3DOptions::default()` set
      `material_library.materials[0].variance.base_color_weight = Vec4::new(0.5, 0.5, 0.5, 0)`
      — cubes assigned material[0] should each show a different shade of red/green/blue

If any check fails, **stop and triage** before Phase E.

---

## Phase E — Cleanup tasks for follow-up sessions

These are NOT part of this autopilot run; just notes for the next session(s):

- **Materials UI (was Phase 5 / task #25)**: build `src/app/settings/materials.rs`
  with list view (rows of materials) + VariableSlider editor for active
  material. Save/load .json buttons. Use `pt_material::io::{save_library,
  load_library}` and `squarebob_widgets::{VariableF32, VariableVec3, VariableColor}`.

- **Slider unification (was Phase 6 / task #26)**: replace `egui::Slider` calls
  in `app/settings/renderer.rs` (camera + path tracer rows) with
  `squarebob_widgets::VariableF32`. Most won't enable variance — they'll just
  use the unified visual style.

- **GpuMaterial cleanup**: replace `pt_core::GpuMaterial` with `pub type
  GpuMaterial = standard_surface::StandardSurfaceParams` alias. Removes the
  `transmute` from `material_cache::expand_pt_materials_and_ids`.

- **L3 effects (task #15)**: vignetting / chromatic aberration / bokeh shape /
  lens distortion / tilt-shift. Independent of materials.

- **Color pipeline (task #20)**: vfx-color / vfx-ocio integration, ACEScg
  working space. Independent of materials.

- **Reconstruction Phase B (task #16)**: filter overlap via atomic splat.
  Independent of materials.

---

## Build commands cheatsheet

```bash
# Full workspace build
cd C:/projects/projects.rust.cg/squarebob-rs && python bootstrap.py b

# Single-crate fast iteration
cargo build -p pt-mats --release
cargo build -p render-shared --release
cargo build -p render-3d --release

# pt-material tests (sanity after edits)
cargo test -p pt-material --release

# Squarebob runtime smoke
target/release/squarebob.exe
```

## Rollback if surgery goes sideways

The new infrastructure (pt-material crate, squarebob-widgets crate,
Render3DOptions.material_library field) is independent of the old
MaterialClass path. To revert just Phase 4 surgery, restore the deleted
items in `pt-mats/src/lib.rs` from git history and revert
`material_cache.rs` to its `.legacy.rs` backup if you made one. The
infrastructure stays — Phase 5/6 can still proceed.
