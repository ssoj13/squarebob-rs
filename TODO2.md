# Unified Material System — Implementation Plan

## Goal

Single source of truth for materials across PBR and PT pipelines. Per-cube
`material_id` indexes into `MaterialLibrary`. Per-cube `color` becomes a tint
that the shader blends with library material albedo via a global
`materialize_mix` uniform. Adding a `MaterialCache` keyed by `path_hash`
eliminates per-frame `classify_path_filtered` cost when materials don't
change.

## Current state (snapshot)

- `crates/render-3d/src/geometry.rs` — `CubeInstance { model, color, hash, object_id, _padding[2] }`, 96B.
- `crates/render-3d/shaders/cube_pbr.wgsl` — group(0): camera UBO, lights UBO, **global** `MaterialParams` UBO (roughness/metalness/IOR/specular_weight). Per-instance only color. Shader hard-codes `base = in.color.rgb` and uses global material for shading params.
- `crates/render-3d/src/lib.rs:1219` — `classify_path_filtered` per cube every cache miss; resulting color baked into `CubeInstance.color`.
- `crates/render-3d/src/pt/megakernel.rs:135` — same `classify_path_filtered` per instance when `pt_scene_dirty`; outputs `material_id` written to `Instance`.
- `MaterialLibrary` (`crates/pt-mats/src/lib.rs:320`) — `Vec<GpuMaterial>`; consumed only by PT today.
- `GpuMaterial` (`crates/pt-core/src/bvh.rs:100`) — full PBR + transmission + emission + coat. 144 bytes.
- `cached_instances` invalidates every frame when `opts.animate=true` (TRS animation).

## Target architecture

### Per-cube data

```rust
// crates/render-3d/src/geometry.rs
pub struct CubeInstance {
    pub model: [[f32; 4]; 4],   // 64B
    pub color: [f32; 4],        // 16B  per-instance tint (color_mode result)
    pub hash: u32,              //  4B
    pub object_id: u32,         //  4B
    pub material_id: u32,       //  4B  ← new
    pub _pad: u32,              //  4B
}                                // 96B (unchanged)
```

Vertex attribute slot 9 = `Uint32 material_id`. Slot 7 (hash) and 8 (object_id) keep their layout.

### GPU material storage (PBR side)

- New storage buffer on `Renderer3D`: `materials_buf` filled from `material_library.materials()` at init and after edits.
- Bound in `pbr_group0` (binding 3), `object_id_group0` if needed for `material.params2.w` (visibility), and reused in PT (PT already binds its own; can keep or share).
- Old `material_buf` (single `MaterialParams`) → repurposed: holds **global render-time uniforms** (`materialize_mix: f32`, `_pad[3]`). Single small UBO bound at the same slot or a new slot. We can reuse the slot and rename for minimal churn.

Decision: bind a new `materialize_mix` UBO at binding 4 (small struct), and `materials` storage buffer at binding 3. Drop `material_buf` from `pbr_bg0`.

### Shader changes (`cube_pbr.wgsl`)

Replace global `MaterialParams` with per-instance lookup:

```wgsl
struct GpuMaterial {
    base_color_weight: vec4<f32>,
    specular_color_weight: vec4<f32>,
    transmission_color_weight: vec4<f32>,
    subsurface_color_weight: vec4<f32>,
    coat_color_weight: vec4<f32>,
    emission_color_weight: vec4<f32>,
    opacity: vec4<f32>,
    params1: vec4<f32>,  // diff_rough, metal, spec_rough, spec_ior
    params2: vec4<f32>,  // spec_aniso, coat_rough, coat_ior, visible
};

struct GlobalMatParams { materialize_mix: f32, _pad: vec3<f32> };

@group(0) @binding(2) var<storage, read> materials: array<GpuMaterial>;
@group(0) @binding(3) var<uniform> mat_global: GlobalMatParams;
```

Vertex output forwards `material_id` (flat). Fragment fetches `mat = materials[material_id]`, computes:

```wgsl
let albedo = mix(in.color.rgb, mat.base_color_weight.rgb, mat_global.materialize_mix);
let roughness = max(mat.params1.z, 0.04);
let metalness = mat.params1.y;
let ior = mat.params1.w;
let specular_weight = mat.specular_color_weight.a;
let emission = mat.emission_color_weight.rgb * mat.emission_color_weight.a;
```

Output `color = pbr_shade + emission` (emission additive, gives "neon"/lights look in PBR too).

`fs_gbuffer` and `fs_wireframe` updated likewise to use `albedo` from the same path.

### Material cache

```rust
struct MaterialCache {
    settings_hash: u64,
    classes: rustc_hash::FxHashMap<u32 /* path_hash */, MaterialClass>,
}

impl MaterialCache {
    fn ensure(&mut self, opts: &Render3DOptions) {
        let h = mat_settings_hash(opts);
        if h != self.settings_hash {
            self.classes.clear();
            self.settings_hash = h;
        }
    }

    fn classify_or_get(
        &mut self, node: &DirEntry, opts: &Render3DOptions, is_pt: bool,
    ) -> MaterialClass {
        if opts.materialize_mode == MaterializeMode::None {
            return MaterialClass::Default;
        }
        let path_str = node.path.to_string_lossy();
        let path_hash = render_shared::name_hash(&path_str);
        if let Some(&c) = self.classes.get(&path_hash) {
            return c;
        }
        let class = classify_path_filtered(
            &node.path, node.size, path_hash, opts.materialize_mode,
            settings_from_opts(opts, is_pt),
        );
        self.classes.insert(path_hash, class);
        class
    }
}
```

`mat_settings_hash` covers only the inputs `classify_path_filtered` reads:
`materialize_mode`, `mat_allow_lights`, `mat_light_prob`, `mat_light_warm`,
`mat_light_cool`, `mat_allow_glass`, `mat_glass_prob`, `mat_seed`, `mat_source`,
`mat_distribution`, `mat_quant_levels`, `mat_band_count`, `mat_spatial_scale`,
`mat_include_dirs`. NOT `materialize_mix` (lives in shader uniform now), NOT
animation_time, NOT camera, NOT layout size.

Note: `is_pt` differs between PT and PBR paths (`MaterializeSettings.is_pt`).
Two cache slots could be needed: one for PBR, one for PT. Simpler and likely
sufficient: cache only PBR results; PT reuses the same path through the cache
because `classify_material` only branches on `is_pt` for light overrides.
Decision: keep two sub-maps (`classes_pbr`, `classes_pt`) — small, cheap,
correct.

### Single classify call site

Both `collect_recursive` (PBR) and `pt_instances` build (`megakernel.rs`) call
`renderer.mat_cache.classify_or_get(node, opts, is_pt)` followed by
`material_library.material_id(class)`.

### Where the per-cube `material_id` is written

`collect_recursive` in `lib.rs`:

```rust
let mat_class = self.mat_cache.classify_or_get(node, opts, false);
let material_id = self.material_library.material_id(mat_class);
let color_f = compute_color_only(node, opts);   // color_mode result, no mat blend
out.push(CubeInstance::new(model, color_f, hash, oid, material_id));
```

The CPU lerp `color_f = mix(base_color, mat_color, mix)` is GONE. Shader does
the mix using `materialize_mix` uniform. This means changing the slider does
not require re-collecting cubes; only `materialize_mix` UBO is updated.

### PT path

`megakernel.rs` already writes `material_id` per Instance. Replace its
`classify_path_filtered` call with `renderer.mat_cache.classify_or_get(node, opts, true)`.
PT scene rebuild gating (`pt_scene_dirty`) stays as is.

### Invalidation surfaces

- `mat_settings_hash` in `MaterialCache::ensure` — automatic on any of the listed mat_* fields.
- Scene rescan: cache survives. Stale entries (paths gone) are bounded leaks. Pruning hook can be added later.
- `material_library` edits (e.g., user retunes a preset's color): library uploads new `materials_buf`; cache classes by path stay valid because they identify the *class*, not the absorbed values.

## Migration steps

Each step keeps the build green and all visual behavior preserved.

### Step 0 — TODO file & branch checkpoint
- Write `TODO2.md` (this file).
- Commit as a checkpoint.

### Step 1 — extend `CubeInstance`
- Add `material_id: u32`, replace `_padding: [u32; 2]` with `material_id: u32, _pad: u32`.
- Add vertex attribute slot 9 (`Uint32`) to `ATTRIBS`.
- Update all `CubeInstance::new(...)` call sites (3 in `lib.rs`) to pass `0` as material_id placeholder for now.
- Verify build: `cargo check --workspace`.

### Step 2 — `MaterialCache` + helpers
- Add `MaterialCache` struct + `mat_settings_hash` + `MaterializeSettings` builder helper inside `lib.rs`.
- Add `mat_cache: MaterialCache` field on `Renderer3D`. Construct in `new()`.
- No call sites yet. Build green.

### Step 3 — wire cache into `collect_recursive`
- In leaf branch (around `lib.rs:1219`), replace inline `classify_path_filtered` with `self.mat_cache.classify_or_get(node, opts, false)`.
- Compute `material_id = self.material_library.material_id(mat_class)`.
- Write to new `CubeInstance` field. Keep CPU color blend for now (visual parity).
- In `collect_cubes` entry, call `self.mat_cache.ensure(opts)` once per call.
- Build + smoke run.

### Step 4 — wire cache into PT
- `megakernel.rs:135` — same swap. Pass `is_pt=true`.
- `megakernel.rs:630` — same swap (second instance build site).
- Build + run PT, verify identical behavior.

### Step 5 — GPU side: introduce `materials_buf` + `materialize_mix` UBO
- Add storage buffer `materials_buf` (filled from `material_library.materials()`) on Renderer3D init. Re-upload only when library changes (rarely).
- Add UBO `mat_global_buf` with `{ materialize_mix: f32, _pad[3] }`. Update each frame from `opts.materialize_mix` (cheap).
- Update `layouts.pbr_group0` to include both. Update `pbr_bg0` accordingly.
- Update `object_id_group0` to include `materials_buf` only if needed (we use `params2.w` visibility — yes, useful for hide-by-material). Optional; keep it out for now to minimize churn.
- Build green; no visual change yet (shader still uses the old globals).

### Step 6 — switch `cube_pbr.wgsl` to per-instance materials
- Replace `MaterialParams` block with `materials` storage array + `mat_global` UBO.
- Add `material_id` to `InstanceInput` and `VertexOutput` (flat).
- Fragment: compute `mat = materials[material_id]`, derive `albedo`, `roughness`, `metalness`, `ior`, `specular_weight`, `emission`. Add emission to final color.
- `fs_gbuffer`, `fs_wireframe` use the same `mat`.
- Build + visual check. PBR-only (no materialize) should look ~identical to today; with materialize on, shader handles the blend (CPU blend removed in step 7).

### Step 7 — drop CPU `color_f` material blend
- In `collect_recursive`, stop doing `base_color = lerp(base_color, mat_color, mix)`. `color_f` is the pure color_mode result. The shader does the blend.
- This is what makes the `materialize_mix` slider live (no rebuild).

### Step 8 — remove dead code
- Delete `MaterialParamsUniform` if unused.
- Delete the legacy `material_buf: wgpu::Buffer` if no longer referenced.
- Update `lib.rs`, `pipelines.rs`, `Cargo.lock` if needed.

### Step 9 — verification
- Smoke test with: animate ON, PT ON, materialize=None vs On — measure FPS delta; expect near-zero delta now.
- Toggle `materialize_mix` slider — confirm no rebuild (instance count debug doesn't bump).
- Visual diff vs current code on a few directories.

## Open questions / risks

1. `GpuMaterial` is 144 bytes. On older mobile GPUs storage buffer access can be slower than UBO. We're on desktop wgpu — non-issue.
2. PT pipeline already binds its own materials buffer. The new PBR `materials_buf` is a second copy unless we share. Sharing requires `pt-megakernel` and `render-3d` to point at the same `wgpu::Buffer`. Defer sharing; minor mem cost (~tens of KB).
3. `MaterialClass::Default` for `materialize_mode=None` — material_library guarantees id 0 for Default. Verify in `MaterialLibrary::new()`.
4. `is_pt` cache split: 99% of work is shared; only light selection branches. Two sub-maps avoid correctness bugs at trivial memory cost.
5. Wireframe and outline pipelines use the same instance buffer. Their shaders need either to read material (for tinting consistency) or just use `in.color`. Wireframe currently uses `in.color` only — keep as is. Outline is a fullscreen post-pass — unaffected.

## Out of scope

- Per-cube material editing UI.
- Sharing `materials_buf` across PBR and PT (deferred).
- Pruning stale cache entries on rescan (deferred; safe leak).
- Replacing the 18-color palette / `ext_color` system. That stays as the per-instance tint source.
