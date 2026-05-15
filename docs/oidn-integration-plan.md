# OIDN Integration Plan — squarebob-rs

**Date:** 2026-05-14
**Target:** Replace the current à-trous denoiser with a fully GPU pipeline of
our Intel-OIDN-port (`oidn-rs`), running on the same `wgpu::Device` as the
path tracer. ML denoising as production-grade finishing for path tracing.
**Non-goals:** No back-compat with the à-trous code path. No CPU fallback. No
runtime model download.

---

## 1. Final architecture (after integration)

```
┌──────────────────────────────────────────────────────────────────────┐
│ squarebob-rs                                                         │
│                                                                      │
│  main.rs ─► WgpuSetup { instance, adapter, device, queue, backend } │
│             │                                                        │
│             ├──► eframe (WgpuConfiguration::Existing)               │
│             ├──► render-core::GpuContext (Arc<...> on all 4)        │
│             └──► cubecl_wgpu::init_device(setup) → Burn WgpuDevice  │
│                                                                      │
│  PT (wavefront / megakernel)                                         │
│  ├─ output_texture (Rgba32Float) ─────────┐                          │
│  ├─ albedo_buf  (array<vec4<f32>>) ───────┤  device-local            │
│  └─ normal_buf  (array<vec4<f32>>) ───────┤  copy_buffer_to_buffer   │
│                                            │  to Burn tensor buffers │
│  pt-denoise-oidn (RtFilter<WgpuBackend>) ─┘                          │
│  └─ writes Rgba32Float result_texture                                │
│                                                                      │
│  blit_pass(result_texture) → egui_wgpu display                       │
└──────────────────────────────────────────────────────────────────────┘

Weights bundled: data/oidn-weights/{rt_hdr,rt_hdr_alb,rt_hdr_alb_nrm,
                                    rt_alb,rt_nrm}.tza  (~9 MB total)
```

**Key technical commitments:**
- Single `wgpu::Device` shared between squarebob's PT and Burn's compute pipelines.
- No host roundtrip during denoise — only `copy_buffer_to_buffer` device-to-device.
- `rt_hdr_alb_nrm` model is the production target (color+albedo+normal).
- Denoiser runs **on demand**: manual button, or auto when `current_spp >= target_spp`. **Not per-frame.**
- À-trous filter is **removed entirely**. No "OIDN-or-à-trous" toggle. OIDN-or-off.

---

## 2. Phase I — `oidn-rs`: pipeline to GPU-only

**Repo:** `git@github.com:ssoj13/oidn-rs.git` (our fork, freely modify).
**Goal:** Eliminate every CPU-side `Vec<f32>` in the per-frame hot path. Inference is already on GPU via `burn-wgpu`; this phase removes the pre/post host plumbing around it.

### I.1 — Tensor-native I/O API
**Files:** `crates/oidn-rs/src/image.rs`, `crates/oidn-rs/src/filters/rt.rs`, `crates/oidn-rs/src/filters/rtlightmap.rs`

- New module `image_tensor.rs`. Type `TensorImage<B: Backend>` = thin wrapper around `Tensor<B, 4>` shaped `[1, C, H, W]`. C ∈ {1, 2, 3}.
- New filter API: `RtFilter::set_color_tensor(&Tensor<B,4>)`, `set_albedo_tensor`, `set_normal_tensor`, `take_output_tensor() -> Tensor<B,4>`.
- Old `Image<'_>`-based API stays — but **only inside the CLI/test paths**. The library's hot path is tensors all the way.
- `set_*_tensor` clones the tensor handle (cheap — Burn tensors are Arc-rc'd internally).
- `commit()` validates tensor shapes match `output` dims, picks the model variant.

### I.2 — GPU input pre-process
**Files:** `crates/oidn-rs/src/filters/unet_runner.rs`, new `crates/oidn-rs/src/gpu_ops.rs`

Replace the CPU `pack(...)` closure (currently `unet_runner.rs:89-152`) with Burn ops:

- **Tile crop+pad** via `Tensor::slice` + `Tensor::pad` (reflection mode). If Burn 0.21 lacks reflection padding, implement it as `cat([flip(left_slice), tile, flip(right_slice)], dim=W)` — one extra kernel, microseconds.
- **Transfer function (sRGB/PU/Log forward)** as vectorized Burn arithmetic. PU and sRGB have piecewise branches → `mask_where(mask, branch_a, branch_b)` where `mask = tensor.lower(threshold)`.
- **Normal/albedo channels**: `clamp(0,1)` for albedo, identity for normal (no remap).
- **Concat** color/albedo/normal along channel dim into the final `[1, in_c, tile_h, tile_w]` input tensor.

No `Tensor::from_data` per tile, no `Vec<f32>` allocation.

### I.3 — GPU autoexposure
**Files:** `crates/oidn-rs/src/autoexposure.rs`

Replace `compute_scale(rgb_hwc: &[f32], w, h) -> f32` with:
```rust
pub fn compute_scale_tensor<B: Backend>(rgb: &Tensor<B, 4>) -> f32
```
Implementation:
1. Compute luminance as `lum = 0.2126*r + 0.7152*g + 0.0722*b` via `Tensor::sum_dim` with a constant weight tensor.
2. Downsample to 16×16 bins via `avg_pool2d(kernel_size=16, stride=16, ceil_mode=true)`.
3. Mask `lum > eps`, take `log`, mean, `exp` — all standard Burn ops.
4. **Only the final scalar reads back to host** (4 bytes).

### I.4 — GPU output post-process
**Files:** `crates/oidn-rs/src/filters/unet_runner.rs`

Replace the per-pixel inverse-transfer + writeback loop (`unet_runner.rs:170-198`) with:
- `Tensor::slice` to crop `output_src_in_tile` out of the network output.
- Inverse transfer (sRGB/PU/Log inverse) as vectorized Burn arithmetic.
- `Tensor::slice_assign` into the accumulating output tensor.

Final `output.write_rgb_f32(...)` is removed — output stays on GPU and is exposed via `take_output_tensor()`.

### I.5 — Quality preset wiring
- `Quality::Fast` → load `_small` model when available, smaller tile cap.
- `Quality::Balanced` / `High` → existing dispatch unchanged.
- Tile planner output stays CPU-side metadata (small struct, no copy).

### I.6 — Verification
- `oidn-cli bench --resolution 1024x1024 --iters 10` runs end-to-end on tensors. Target latency on RTX 3070 / similar: ≤ 200 ms at 1024² (currently 302 ms; the 100 ms savings come from removing host roundtrip).
- Existing integration tests using `Image<'_>` API stay green — they go through the legacy wrapper that calls `Tensor::from_data` once.

---

## 3. Phase II — Shared wgpu device in squarebob

**Files:** `src/main.rs`, `crates/render-core/src/lib.rs`, `Cargo.toml` (workspace)

### II.1 — `GpuContext` extension
```rust
pub struct GpuContext {
    pub instance: Arc<wgpu::Instance>,   // new
    pub adapter:  Arc<wgpu::Adapter>,    // new
    pub device:   Arc<wgpu::Device>,
    pub queue:    Arc<wgpu::Queue>,
}
```
Remove `from_eframe(device, queue)` constructor — there will be **one** path: we own the setup, eframe receives it.

### II.2 — Custom eframe setup
`main.rs` builds `WgpuSetup` ourselves with the limits we need (storage buffers ≥ 16, POLYGON_MODE_LINE feature), then hands eframe an `egui_wgpu::WgpuConfiguration { wgpu_setup: WgpuSetup::Existing(…), … }`.

Same `wgpu::Device` is then visible to:
- eframe rendering
- treemap GPU backend
- 3D renderer
- PT backends
- **OIDN** via `cubecl_wgpu::init_device`

### II.3 — Burn bridge helper
**New file:** `crates/render-core/src/burn_bridge.rs`
```rust
pub fn make_burn_device(ctx: &GpuContext) -> burn_wgpu::WgpuDevice {
    let setup = cubecl_wgpu::WgpuSetup {
        instance: (*ctx.instance).clone(),  // wgpu types are cheaply Clone
        adapter:  (*ctx.adapter).clone(),
        device:   (*ctx.device).clone(),
        queue:    (*ctx.queue).clone(),
        backend:  cubecl_wgpu::Backend::default_for_adapter(&ctx.adapter),
    };
    cubecl_wgpu::init_device(setup, cubecl_wgpu::RuntimeOptions::default())
}
```
**Workspace `Cargo.toml`:** Add `burn = { version = "0.21", default-features = false, features = ["std", "ndarray"] }`, `burn-wgpu = { version = "0.21", default-features = false }`, `cubecl-wgpu = "*"` (transitive, pinned by burn-wgpu), `oidn-rs = { git = "git@github.com:ssoj13/oidn-rs.git" }` workspace-wide.

---

## 4. Phase III — Albedo AOV in wavefront PT

**Files:** `crates/pt-megakernel/src/wavefront/gbuffer.wgsl`, `crates/pt-megakernel/src/wavefront/shade.wgsl`, `crates/pt-megakernel/src/wavefront/finalize.wgsl`, `crates/pt-megakernel/src/wavefront/pipeline.rs` (or pt-wavefront equivalent)

### III.1 — Add `albedo_buf` storage
- New binding `@group(0) @binding(7) var<storage, read_write> albedo_buf: array<vec4<f32>>` in `gbuffer.wgsl` and the corresponding consumer/finalize passes.
- Buffer sized `full_w * full_h * 16 bytes` (~32 MB at 1080p — fine).
- Allocated alongside `normal_buf` in pipeline init; resize logic mirrors normal_buf exactly.

### III.2 — Write albedo on primary hit
In `shade.wgsl` (or `gbuffer.wgsl` if material lookup is accessible there), at primary-hit branch (bounce == 0):
```wgsl
let albedo = mat.base_color_weight.rgb * inst.color.rgb;
albedo_buf[global_id] = vec4<f32>(albedo, 1.0);
```
Only written once per pixel per sample. For accumulated samples, **first sample's albedo wins** (it's the deterministic primary hit — averaging is wasteful here; OIDN expects clean albedo).

Better path: write albedo *only on frame 0 after a scene reset*; subsequent frames skip the write. This makes albedo deterministic regardless of sample count.

### III.3 — Finalize pass
No changes to color finalize. Normal and albedo are not normalized by sample_count (they are deterministic primary-hit values).

### III.4 — Public API
`WavefrontPipeline` exposes:
```rust
pub fn albedo_buffer(&self) -> Option<&wgpu::Buffer>;
pub fn normal_buffer(&self) -> Option<&wgpu::Buffer>;
pub fn aov_dims(&self) -> (u32, u32);
```
These are consumed by `pt-denoise-oidn` to feed OIDN albedo/normal inputs.

---

## 5. Phase IV — G-buffer pass for megakernel

**New files:** `crates/pt-megakernel/src/gbuffer/mod.rs`, `crates/pt-megakernel/src/gbuffer/primary.wgsl`, integration in `crates/pt-megakernel/src/compute.rs`

### IV.1 — Standalone primary-hit pass
A single compute shader that traces **only primary rays** + does material lookup + writes:
- `albedo_buf: array<vec4<f32>>` (world-space primary-hit albedo, sample_count == 1)
- `normal_buf: array<vec4<f32>>` (world-space primary-hit normal)
- `depth_buf: array<f32>` (camera-space depth)

Reuses the same BVH, material bindings, and scene buffers as the megakernel — code is largely a copy of the megakernel's first iteration without the bounce loop.

### IV.2 — Lazy and cached
- Run only when scene/camera changed since last run (track a `gbuffer_dirty` flag on PT compute).
- One frame's worth of work (~5 ms at 1080p), amortized across many PT samples.
- Buffers are owned by the megakernel pipeline; new public API mirrors III.4.

### IV.3 — Wiring
`pt-denoise-oidn` doesn't know or care if AOVs come from wavefront or megakernel — both expose the same `wgpu::Buffer` API.

---

## 6. Phase V — New crate `pt-denoise-oidn`

**New crate:** `crates/pt-denoise-oidn/` with `Cargo.toml`, `src/lib.rs`, optional `src/weights.rs`

### V.1 — Workspace registration
Add to root `Cargo.toml`:
```toml
[workspace.members]
... existing ...,
"crates/pt-denoise-oidn",
```

### V.2 — Crate manifest
```toml
[package]
name = "pt-denoise-oidn"
version = "0.1.0"
edition = "2021"

[dependencies]
wgpu = { workspace = true }
bytemuck = { workspace = true }
burn = { workspace = true }
burn-wgpu = { workspace = true }
cubecl-wgpu = "*"
oidn-rs = { git = "https://github.com/ssoj13/oidn-rs.git" }
render-core = { path = "../render-core" }
anyhow = { workspace = true }
log = { workspace = true }
```

### V.3 — Public API
```rust
pub struct OidnDenoiser {
    burn_device: burn_wgpu::WgpuDevice,
    filter: Option<RtFilter<'static, WgpuBackend>>,
    mode: OidnMode,
    quality: oidn_rs::Quality,
    width: u32,
    height: u32,
    weights_dir: PathBuf,
    result_texture: wgpu::Texture,
    result_view:    wgpu::TextureView,
    last_latency_ms: Option<f32>,
}

pub enum OidnMode {
    Color,            // rt_hdr
    ColorAlbedo,      // rt_hdr_alb
    ColorAlbedoNormal, // rt_hdr_alb_nrm  ← production target
}

impl OidnDenoiser {
    pub fn new(ctx: &GpuContext, width: u32, height: u32, weights_dir: PathBuf) -> Self;
    pub fn resize(&mut self, ctx: &GpuContext, width: u32, height: u32);
    pub fn set_mode(&mut self, mode: OidnMode);
    pub fn set_quality(&mut self, q: oidn_rs::Quality);

    /// Run denoise. Inputs are raw wgpu buffers/textures that already live on the GPU.
    /// Wraps them as Burn tensors via copy_buffer_to_buffer (device-local), runs OIDN,
    /// blits result into self.result_texture.
    pub fn denoise(
        &mut self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        color_tex: &wgpu::Texture,                  // PT accumulator (already normalized)
        albedo_buf: Option<&wgpu::Buffer>,          // wavefront/megakernel AOV
        normal_buf: Option<&wgpu::Buffer>,
    ) -> Result<()>;

    pub fn result_view(&self) -> &wgpu::TextureView;
    pub fn last_latency_ms(&self) -> Option<f32>;
}
```

### V.4 — Implementation notes
- `RtFilter` is built once at first `denoise()` call (or when mode/quality changes). `commit()` is invoked then.
- Color tensor is built from `color_tex` via:
  1. `copy_texture_to_buffer` into a staging wgpu::Buffer of `width*height*16` bytes.
  2. Wrap that buffer as `cubecl_wgpu::WgpuResource::new(buffer, 0, size)`.
  3. **Bridging step:** since Burn 0.21 doesn't publicly expose `from_wgpu_resource`, we patch `oidn-rs` to add a `WgpuBackend`-specialized `Image::from_wgpu_buffer(buffer, w, h)` constructor that internally builds the tensor primitive. This is one ~30-line patch in `oidn-rs/crates/oidn-rs/src/image_tensor.rs` against the internal burn-wgpu API. **Mitigation if private:** keep using the staging buffer + a Burn `Tensor::from_data` over a CPU-side `&[u8]` that's `mmap`-ed from a `buffer_map` of the staging buffer. Async map_async forces one `poll`. Still no PCIe traffic on integrated GPUs; on discrete GPUs the staging path is single-PCIe-crossing instead of two.
- Output: same logic in reverse — Burn output tensor → wgpu::Buffer → `copy_buffer_to_texture` into `result_texture`.
- Wrap encoder lifetime carefully: OIDN's internal Burn dispatches use their own encoder; our outer encoder records the copies on either side. Submit order: outer encoder → submit → burn dispatch (own queue.submit internally) → another encoder for the read copy → submit.

### V.5 — Tile management
- `RtFilter` already plans tiles internally (see `oidn-rs/crates/oidn-rs/src/tile.rs`). Plan is recomputed on `commit()`, which fires when dims or mode change.
- For viewports larger than `DEFAULT_MAX_TILE_SIZE` (2160²), tiling is automatic.

### V.6 — Weight loading
`weights.rs`: lightweight helper that:
- Validates `data/oidn-weights/` directory exists.
- Maps a `(OidnMode, Quality)` pair to the expected `.tza` filename.
- Errors with a clear message if a required model is missing.

---

## 7. Phase VI — Removal of à-trous + integration

**Files to modify:** `crates/pt-megakernel/src/compute.rs`, `crates/render-3d/src/pt/megakernel/render.rs`, `crates/render-3d/src/pt/megakernel/render_no_readback.rs`, `src/app/settings/denoiser.rs`, `src/app/settings/mod.rs`, `src/app/state.rs`, `src/cli.rs`, `src/app/cli_apply.rs`, `crates/render-shared/src/lib.rs`

### VI.1 — Delete à-trous (no compat layer)
- Remove `crates/pt-megakernel/src/denoiser/` directory entirely (`atrous.wgsl`, `mod.rs`, `pipeline.rs`).
- Remove from `crates/pt-megakernel/src/lib.rs`: `pub mod denoiser;`.
- Remove from `crates/pt-megakernel/src/compute.rs`:
  - field `denoiser: Option<DenoiserPipeline>`
  - fields `denoise_enabled`, `denoise_iterations`, `denoise_sigma_color`
  - field `blit_bg_uses_denoiser`
  - methods `set_denoise_enabled`, `set_denoise_options`, `apply_denoiser`
  - all init in `new()` and `resize()` paths

### VI.2 — Replace state fields
In `crates/render-shared/src/lib.rs` `Render3DOptions`:
```rust
// DELETED
pub pt_denoise_enabled:    bool,
pub pt_denoise_iterations: u32,
pub pt_denoise_sigma_color: f32,

// REPLACED WITH
pub pt_oidn_mode:    OidnMode,         // serde enum
pub pt_oidn_quality: OidnQuality,      // serde enum
pub pt_oidn_auto:    bool,             // auto-run on target_spp reached
```
`OidnMode` enum (serializable): `Off`, `Color`, `ColorAlbedo`, `ColorAlbedoNormal`.
`OidnQuality`: `High`, `Balanced`, `Fast`.

### VI.3 — Wire `OidnDenoiser` into the megakernel render path
In `render-3d/src/pt/megakernel/render.rs` and `render_no_readback.rs`:
- Replace `pt.set_denoise_*` + `pt.apply_denoiser(...)` with:
  ```rust
  if pt_oidn_should_run(opts, current_spp, target_spp, user_requested) {
      let albedo = pt.albedo_buffer();   // or megakernel gbuffer
      let normal = pt.normal_buffer();
      oidn.denoise(&renderer.ctx, &mut encoder, &pt.output_texture(), albedo, normal)?;
      blit_source = oidn.result_view();
  } else {
      blit_source = pt.output_view();
  }
  ```
- `OidnDenoiser` instance lives in `RenderState3D` (or wherever `MegakernelPathTracer` lives now), behind `Option<OidnDenoiser>` so it's lazily built.

### VI.4 — Trigger logic
`pt_oidn_should_run(...)`:
- `opts.pt_oidn_mode == Off` → never run
- `user_requested` (one-shot from UI button) → run once, reset flag
- `opts.pt_oidn_auto && current_spp >= target_spp && !already_run_this_accumulation` → run once

**Not per-frame.** Avoiding ~300 ms/frame is the whole point.

---

## 8. Phase VII — UI + presets + CLI

### VII.1 — Settings panel rewrite
`src/app/settings/denoiser.rs` becomes:
```
┌─ Denoiser (OIDN) ────────────────────────────────────┐
│ Mode:       [ Off ▾ ]   (Off / Color / +Albedo /    │
│                          +Albedo+Normal)             │
│ Quality:    [ Balanced ▾ ]   (High / Balanced / Fast)│
│ Trigger:    [✓] Auto on target spp                  │
│             [ Denoise now ]   ← manual button       │
│                                                      │
│ Status:     287 ms (last run)                        │
│             rt_hdr_alb_nrm  •  1920×1080  •  1 tile │
│                                                      │
│ Tip: OIDN runs once per converged frame, not per-    │
│      sample. Enable Auto for hands-off operation.   │
└──────────────────────────────────────────────────────┘
```
Drop all à-trous sliders/presets.

### VII.2 — Persistence migration
`Render3DOptions::deserialize`: if old fields `pt_denoise_*` are present, ignore them (drop silently). Default-initialize new `pt_oidn_*` fields. Old preset files keep loading without panic.

### VII.3 — Factory preset
Update `data/default.json` (formerly `factory_render3d_options.json`):
```json
"pt_oidn_mode": "ColorAlbedoNormal",
"pt_oidn_quality": "Base",
"pt_oidn_auto": true
```

### VII.4 — CLI
`src/cli.rs` — remove `--pt-denoise`. Add:
- `--oidn-mode <off|color|color_albedo|color_albedo_normal>`
- `--oidn-quality <high|balanced|fast>`
- `--oidn-auto` (boolean flag)

`src/app/cli_apply.rs`: map CLI flags into `Render3DOptions::pt_oidn_*`.

---

## 9. Phase VIII — Weights bundling and packaging

### VIII.1 — Bundled subset
`data/oidn-weights/` (new directory), copied from `oidn-rs/data/weights/`:
- `rt_hdr.tza`            (1.8 MB) — color-only
- `rt_hdr_alb.tza`        (1.8 MB) — color+albedo
- `rt_hdr_alb_nrm.tza`    (1.8 MB) — color+albedo+normal (production target)
- `rt_alb.tza`            (1.8 MB) — albedo prefilter (optional, future)
- `rt_nrm.tza`            (1.8 MB) — normal prefilter (optional, future)

Total ≈ **9 MB** in the repo. We can defer `rt_alb`/`rt_nrm` until prefilter quality path is needed.

Since `oidn-rs` weights are now regular git blobs (LFS migration done), these can be **vendored** by `cp oidn-rs/data/weights/{...} squarebob-rs/data/oidn-weights/` and committed. No submodule, no LFS dependency, no `xtask` automation needed.

### VIII.2 — Runtime location
- Dev: `data/oidn-weights/` (relative to CWD when running `cargo run`).
- Installed: `<install_dir>/data/oidn-weights/` — packager already copies `data/*`.

`OidnDenoiser::new` resolves weights dir in this order:
1. `OIDN_WEIGHTS_DIR` env var if set
2. `<exe_dir>/data/oidn-weights/`
3. `<cwd>/data/oidn-weights/`

### VIII.3 — Packager
`Cargo.toml` `[package.metadata.packager]` already lists `resources = [{ src = "data", target = "data" }]` — weights ride along automatically.

---

## 10. Phase IX — Documentation and verification

### IX.1 — README updates
- New section "Denoising" replacing "color-only a-trous" mention.
- Performance row: latency at common resolutions.
- Note on autoexposure and per-mode model selection.

### IX.2 — `CHANGELOG.md`
Single entry: "Replaced color-only à-trous filter with full GPU OIDN denoiser (color/albedo/normal AOV). À-trous code removed."

### IX.3 — Benchmark
`xtask` or one-shot script:
- For each `(mode, quality, resolution ∈ {720p, 1080p, 1440p, 2160p})`, run 10 denoise iterations, log median ms.
- Output CSV to `data/benchmarks/oidn-2026-05-14.csv`.
- Reproduce on RTX 3060/3070 baseline.

### IX.4 — Visual regression
- Reference scenes: `data/scenes/*` (existing test scenes for PT).
- Generate noisy PT output at 16 spp, run OIDN, compare to ground-truth 4096 spp via PSNR.
- Bench gate: PSNR improvement > 10 dB over the noisy input. (Should easily exceed this — synthetic test in `oidn-rs` already shows ~11× RMSE reduction.)

### IX.5 — CI
- `cargo test --workspace` must remain green. New crate's tests can be `#[ignore]` if they require weights.
- `cargo clippy --workspace --all-targets -- -D warnings` zero warnings.
- `rust-toolchain.toml` stays at 1.95 (Burn 0.21 supports it).

---

## 11. Execution waves (concurrency plan)

Tasks marked **W**ave **n** run concurrently if independent. Sequential dependency is shown with arrows.

```
W1 (parallel):
  ├─ oidn-rs Phase I (tensor I/O API + GPU pre/post + autoexposure)
  └─ squarebob Phase II (GpuContext extension + shared WgpuSetup)

W2 (parallel, blocks on W1):
  ├─ Phase III (wavefront albedo AOV)
  └─ Phase V (pt-denoise-oidn crate, with Phase I API and Phase II shared device)

W3 (sequential, blocks on W2):
  └─ Phase VI (delete à-trous + wire OIDN into render paths)

W4 (parallel, blocks on W3):
  ├─ Phase VII (UI + CLI + presets)
  └─ Phase VIII (weights bundle + packager metadata)

W5 (parallel, can start once W3 done):
  ├─ Phase IV (megakernel G-buffer pass)  ← optional / quality-of-life
  └─ Phase IX (docs + benchmarks)
```

## 12. Time estimate

| Phase | Hours | Notes |
|---|---|---|
| I — oidn-rs GPU-only | 16-24 | Largest single block; mostly Burn ops |
| II — shared wgpu device | 4-6 | GpuContext + eframe setup |
| III — wavefront albedo AOV | 6-8 | Mostly wgsl plumbing |
| IV — megakernel G-buffer (optional) | 16-20 | Defer if interactive path is enough |
| V — pt-denoise-oidn crate | 12-16 | Burn↔wgpu buffer bridge is the tricky bit |
| VI — delete à-trous + wire OIDN | 6-8 | Mechanical; touches several files |
| VII — UI/CLI/presets | 6-8 | egui forms + clap derives |
| VIII — weights bundle | 2-3 | cp + update factory preset |
| IX — docs + bench | 4-6 | + visual regression check |

**MVP path (no megakernel AOV):** 56-79 hours ≈ **7-10 working days**.
**Full plan including megakernel G-buffer:** 72-99 hours ≈ **9-12 working days**.

## 13. Risks and mitigations

| Risk | Probability | Mitigation |
|---|---|---|
| **Burn `Tensor::from_wgpu_buffer` is private** | High | Implement the bridge in our `oidn-rs` fork via `pub(crate)` access — our fork, no problem. Worst case: staging buffer + `buffer_map_async` (no PCIe roundtrip on integrated GPUs). |
| **wgpu version drift** (Burn 0.21 pins 29.0.3, squarebob is on 29) | Low | Pin squarebob's `wgpu` to `=29.0.3` in workspace deps. |
| **Burn's GPU op set missing `pad(Reflect)` / piecewise-conditional ops** | Medium | All needed ops emulatable via existing ops; see I.2 notes. |
| **OIDN latency too high for "auto on spp" UX** | Low | At 1080p `rt_hdr_alb_nrm` ≈ 300 ms after GPU-only pass; acceptable as a one-shot finishing step. Add a spinner overlay during execution. |
| **eframe-wgpu version mismatch on `WgpuSetup::Existing`** | Low | `egui-wgpu 0.34` supports `WgpuConfiguration::wgpu_setup` field directly. |
| **Weight files diverge between oidn-rs and squarebob over time** | Low | Vendoring is intentional. Add a one-line note in `data/oidn-weights/README.md` documenting the source commit hash. |

---

## 14. Done criteria

- [ ] `crates/pt-megakernel/src/denoiser/` directory does not exist.
- [ ] `git grep -i "pt_denoise"` returns zero hits.
- [ ] `cargo build --workspace --release` succeeds; binary contains `oidn-rs` symbols.
- [ ] In a fresh checkout, `python bootstrap.py b` builds without external weight setup steps.
- [ ] App launches, scans `C:\Users`, switches to 3D + path-trace, accumulates 64 spp, denoiser button produces a visibly cleaner image within ≤ 500 ms at 1080p.
- [ ] Settings panel shows the new OIDN section with mode/quality/trigger controls and live latency readout.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- [ ] One CHANGELOG entry, one README section update, one benchmark CSV committed.

---

## 15. Resolved decisions

1. **Output color space:** OIDN writes **linear** Rgba32Float into `result_texture`. Tone-mapping stays in the existing blit pass (a dedicated tonemap crate already covers this). `pt-denoise-oidn` performs no display-gamma conversion.
2. **Weight loading:** **Lazy.** TZA files are read from `data/oidn-weights/` the first time `denoise()` runs with a non-`Off` mode. First click pays the ~tens-of-ms load cost; subsequent clicks reuse the cached `RtFilter`.
3. **Release default mode:** `pt_oidn_mode = ColorAlbedoNormal` (production target). `pt_oidn_auto = true`. Out-of-box behavior: user accumulates samples → at `target_spp` denoiser fires automatically and replaces the displayed frame. Factory preset in `data/default.json` reflects this.

---

## 16. Post-merge work (after the initial OIDN landing)

These items were tackled in follow-up sessions on top of the integration plan above. Kept here as part of the doc for context.

- **Adaptive sampling bugfix.** Welford variance buffer in `adaptive::AdaptivePipeline` was not cleared on accumulation reset — Welford mean/M2 mixed stale samples with fresh ones across camera/scene changes. Fixed in both megakernel and wavefront `dispatch` paths.
- **DMC-style noise threshold.** `adaptive/allocate.wgsl` switched from raw luminance variance to relative noise (`std_err / max(luminance(mean), eps)`), so a single `variance_threshold` works across the full HDR range.
- **`pt_samples` unification.** Renamed `pt_max_samples` → `pt_samples` everywhere. Removed `pt_adaptive_min_spp` / `pt_adaptive_max_spp` — adaptive per-pixel range is now derived from `pt_samples` (`min = max(samples/16, 8)`, `max = samples`). One global samples knob; everything else derived.
- **Full TZA bundle.** All 23 OIDN model variants Intel ships (~48 MB) are vendored to `data/oidn-weights/`, not just the 5 base models originally planned. Quality selector renamed to **Model size** (`Small` / `Base` / `Large`) — names match what user actually controls (which TZA file to load).
- **Megakernel AOV.** Megakernel writes its own primary-hit `albedo` + `normal` AOV buffers (not only wavefront). `PathTraceCompute::albedo_buffer()` / `normal_buffer()` transparently return wavefront's buffer when wavefront is active, megakernel's otherwise — OIDN works in either mode.
- **Denoiser UI polish.** Compact 3-row grid (Mode / Size / Trigger), wide ComboBoxes, colour-coded status indicator, per-option tooltips that name the TZA file. Auto checkbox shares the trigger row with the "Denoise now" button.
- **`default.json` rename.** `data/factory_render3d_options.json` → `data/default.json` — same filename convention as the runtime override beside the executable.
