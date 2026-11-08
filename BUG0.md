# BUG0: `squarebob-rs` Freezes on Startup (White Window) in Path Tracing Mode After `render-rs` Upgrade

## 1. Executive Summary & Context

After upgrading `render-rs` (dual-backend WGPU + `cuda-oxide` upgrades, BSDF/transport parity, WGSL `override` specialization, hot/cold geometry split, and studio SSOT) and rolling out the update across all consumer repositories (`squarebob-rs`, `egui-widgets-rs`, `fractal-rs`, `nodes-rs`, `usd-rs`, `warpbro-rs`, `openvdb-rs`, `ofx-rs`, `watermark-rs`, `vfx-view-rs`, `playa`, `rv-rs`, `colmap-rs`), **`squarebob-rs` freezes indefinitely on startup with a white window** as soon as it enters the first 3D Path Tracing frame.

Even the headless built-in CLI smoke test (`squarebob.exe test gpu-pt-smoke`, which has **no GUI, no scanner, no OCIO, and no material cache**) hangs indefinitely (> 60 seconds) at the exact same point inside `pt_megakernel::PathTraceCompute::new` / `Self::create`.

Crucially, running `PathTraceCompute::new` inside `render-rs`'s own `pt-megakernel` test suite (`cargo test -p pt-megakernel upload_scene_builds_pt_bind_group_without_validation_error`) on the **exact same machine, GPU (`NVIDIA GeForce RTX 3080 Ti`), Vulkan backend, and driver (`621.14`)** succeeds in **7.37s** (cold pipeline cache) and **1.90s** (warm pipeline cache).

---

## 2. Repository & Commit State

| Repository | Path | Commit (`HEAD`) | Summary |
|---|---|---|---|
| `squarebob-rs` | `C:/projects/projects.rust.cg/cglibs/squarebob-rs` | `77452c8f8cf299b2268b023e7a8312589d75c651` (prev: `76268c8`) | `feat(render-3d,pt): adopt render-rs studio SSOT, CudaPathTracer, GPU-native OIDN, and OcioPass` |
| `render-rs` | `C:/projects/projects.rust.cg/cglibs/render-rs` | `c9c38db5` / `23c01af` (prev: `5650013`) | `feat: universal dual-backend WGPU + cuda-oxide upgrades, BSDF/transport parity, and material SSOT` |
| `gpu-info-rs` | `C:/projects/projects.rust.cg/cglibs/gpu-info-rs` | `fd6abcb5` | `WorkContext`, `Resources`, `DiskPipelineCache` (`gpu_mem::pipeline_cache`) |
| Installed binary | `C:/Users/joss1/.cargo/bin/squarebob.exe` | Built from `squarebob-rs` `77452c8` | Default features (`wgpu` megakernel path; `cuda` feature disabled by default) |

### Hardware / Driver Environment
- **OS**: Windows 11
- **GPU**: `NVIDIA GeForce RTX 3080 Ti` (`vendor 0x10de`, `device 0x2208`, `12084 MiB` VRAM)
- **Driver**: `NVIDIA 621.14`
- **Backend selected by `wgpu`**: `Vulkan` (`wgpu 30.0.0`)

---

## 3. Exact Reproduction Commands

### Repro A — Full GUI / Screenshot Run on `D:\temp`
```powershell
C:/Users/joss1/.cargo/bin/squarebob.exe -vv --log-pt --screenshot 2 --screenshot-path C:/Users/joss1/.filesystem-mcp-rs/tmp/sb_test.png --exit-after-screenshot D:\temp
```
**Observed output before hanging forever (> 30s):**
```text
Using allocator: MimallocSecure | Reason: mimalloc-secure selected by runtime hardware analysis (24 cores, 63.9GB total RAM)
[2026-11-08T19:12:21.146Z INFO  squarebob] squarebob-rs starting (log level: debug)
[2026-11-08T19:12:21.146Z INFO  squarebob] Screenshot mode: delay=2s, path="C:/Users/joss1/.filesystem-mcp-rs/tmp/sb_test.png"
[2026-11-08T19:12:23.317Z INFO  squarebob::app::presets] Loaded 8 presets from C:\Users\joss1\.squarebob\presets.json
[2026-11-08T19:12:23.321Z INFO  squarebob::app] wgpu limits: max_storage_buffer_binding_size=2147483644, max_storage_buffers_per_shader_stage=524288, max_uniform_buffers_per_shader_stage=524288, max_bind_groups=8, max_texture_dimension_2d=32768, max_buffer_size=4294967295, min_uniform_buffer_offset_alignment=64, min_storage_buffer_offset_alignment=32
[2026-11-08T19:12:23.322Z INFO  squarebob::scanner] Scan started: "\\\\?\\D:\\Temp"
[2026-11-08T19:12:23.367Z INFO  squarebob::display_host] Display output: HDR10 (PQ) (Rgb10a2Unorm, Bt2100Pq)
[2026-11-08T19:12:23.376Z INFO  squarebob::cache] Cache loaded: "C:\\Users\\joss1\\AppData\\Local\\squarebob-rs\\cache\\0f7bb4d156ba7201b8e317e67beb6df726a4e35bf8aa9b94b68e3c167c1dde17.bin" (17367 files)
[2026-11-08T19:12:23.469Z INFO  squarebob::app::scan_orchestration] Loaded cache preview for generation 1: 17367 files
[2026-11-08T19:12:23.660Z INFO  render_3d::env_map] Loaded env map: 2048x1024 (12 mips) Rgba16Float from "D:\\_example_files\\hdr\\venice_sunset_2k.hdr"
[2026-11-08T19:12:23.681Z INFO  render_3d::renderer3d::render] === render_to_view START: 1231x989, PT=true, wire=false ===
[2026-11-08T19:12:23.684Z TRACE render_3d] cache_valid: false, opts_hash: 0xde0159b4b266a3fb
[2026-11-08T19:12:23.686Z DEBUG render_3d::renderer3d::instance_collect] collect_cubes rebuild #1
[2026-11-08T19:12:23.686Z TRACE render_3d::picking] picking.reset_frame: 0 entries (reuse)
[2026-11-08T19:12:23.692Z DEBUG render_3d::renderer3d::instance_collect] collect_cubes: 6.48ms (4043 cubes)
[2026-11-08T19:12:23.692Z TRACE render_3d::renderer3d::instance_collect]   first cube: model[0]=(103.1,0.5,5.0,0.0), color=(0.00,0.00,0.07,1.00)
[2026-11-08T19:12:23.692Z TRACE render_3d::renderer3d::instance_collect]   first cube: model[3]=(51.6,-41.6,20.2,1.0) (translation column)
[2026-11-08T19:12:23.693Z INFO  render_3d::renderer3d::render] render_to_view: instance_count=4043, cache_valid=false
[2026-11-08T19:12:23.693Z INFO  render_3d::renderer3d::render] render_to_view: camera pos=(512.0,-512.0,1236.1), dist=1236.1
[2026-11-08T19:12:23.693Z INFO  render_3d::renderer3d::render] render_to_view: PATH TRACING mode
```
- `render_to_view: PATH TRACING mode` (`crates/render-3d/src/renderer3d/render.rs:100`) is the **last log line ever emitted**.
- Even with `--log-pt` (`render_3d = Trace`), `debug!("PT BVH start: ...")` at `crates/render-3d/src/pt/megakernel/render.rs:201` is **never reached**.
- No error (`PT render error` or `3D render failed`) is ever returned from `render_to_view`.

### Repro B — Minimal Headless CLI Smoke Test (`gpu-pt-smoke`, No GUI / No Scanner / No OCIO)
```powershell
$env:RUST_LOG="info,pt_megakernel=trace,bvh_gpu=trace,gpu_info=trace"
$env:VFX_PT_CACHE_DIR="C:/Users/joss1/.filesystem-mcp-rs/tmp/empty_cache_sb_1"
C:/Users/joss1/.cargo/bin/squarebob.exe test gpu-pt-smoke
```
**Observed output before hanging forever (> 60s):**
```text
[2026-11-08T19:20:34Z INFO  wgpu_hal::vulkan::adapter] Found 6 cooperative matrix configurations supported by wgpu
[2026-11-08T19:20:34Z INFO  render_core::gpu] GPU: NVIDIA GeForce RTX 3080 Ti  [vulkan, DiscreteGpu]
      vendor   : NVIDIA (0x10de)   device 0x2208
      driver   : NVIDIA 621.14
      shader   : model=Sm5  webgpu_compliant=true  subgroup=32..32
      buffers  : storage/stage=524288  uniform/stage=524288  max_storage_binding=2147483644  max_buffer=4294967295
      compute  : invocations/wg=1024  wg_size=[1024, 1024, 64]  wg/dim=65535
      binding  : bind_groups=8  sampled_textures/stage=524288  samplers/stage=524288
[2026-11-08T19:20:35Z INFO  render_core::gpu] GPU: NVIDIA GeForce RTX 3080 Ti — VRAM 12084 MiB total, 4110 MiB free (dedicated)
```
- The directory `VFX_PT_CACHE_DIR` (`empty_cache_sb_1`) is **never created** and `INFO [pt-megakernel] pipeline cache saved` (`pt-megakernel/src/compute.rs:241`) is **never logged**.

### Control C — `pt-megakernel` Unit Test in `render-rs` (PASSES in 7.37s cold / 1.90s warm)
```powershell
$env:VFX_PT_CACHE_DIR="C:/Users/joss1/.filesystem-mcp-rs/tmp/empty_cache_test_1"
cargo test -p pt-megakernel upload_scene_builds_pt_bind_group_without_validation_error -- --nocapture
```
**Observed result:**
- Succeeds in **7.37s** on a cold cache and writes `empty_cache_test_1/pt_megakernel_ea69a688175931a0.bin`.
- Succeeds in **1.90s** on a warm cache.

---

## 4. What We Investigated & Proven Facts

### Fact 1: The Hang Occurs Inside `PathTraceCompute::new` / `Self::create` Before `cache.save()`
In `squarebob-rs/src/cli_test.rs` (`gpu_pt_smoke`, lines 134–154):
```rust
let ctx = render_core::gpu::GpuContext::new()
    .ok_or_else(|| anyhow::anyhow!("GPU initialization failed"))?;
let device = &ctx.device;
let queue = &ctx.queue;
let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
let smoke_result = (|| -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let settle = |tracer: &mut pt_megakernel::PathTraceCompute| -> anyhow::Result<()> {
        if tracer.pending() > 0 {
            device.poll(wgpu::PollType::wait_indefinitely())?;
        }
        tracer.collect()?;
        Ok(())
    };

    let mut tracer = pt_megakernel::PathTraceCompute::new(
        &ctx.work_context,
        WIDTH,
        HEIGHT,
        wgpu::TextureFormat::Rgba8Unorm,
    )?;
    settle(&mut tracer)?;
```
And in `squarebob-rs/crates/render-3d/src/pt/megakernel/render.rs` (lines 65–74):
```rust
if renderer.pt.path_tracer.is_none() {
    let mut pt = PathTraceCompute::new_with_adapter(
        &renderer.ctx.work_context,
        &renderer.adapter_info,
        width,
        height,
        surface_format,
    )
    .map_err(pt_err)?;
    settle_pt(&renderer.ctx.device, &mut pt)?;
```
Inside `PathTraceCompute::create` (`render-rs/crates/render-engine-pt/pt-megakernel/src/compute.rs:2435-2691`):
1. `let (state, plan, initial_upload) = Snapshot::initial(context, width, height)?;`
2. `let shader = device.create_shader_module(...);` (`megakernel_source()`)
3. `let bind_group_layout = device.create_bind_group_layout(...);` (bindings `0..31` = 32 entries)
4. `let mut pipeline_cache = PtPipelineCache::open(device, adapter);`
5. `let initial_pipeline = Self::compile_megakernel_variant(device, &pipeline_layout, &shader, PipelineVariantKey::INITIAL, pipeline_cache_ref);`
6. `let pick_pipeline = device.create_compute_pipeline(...);`
7. `let blit_pipeline = Self::create_blit_render_pipeline(...);`
8. `if let Some(cache) = &mut pipeline_cache { cache.save(); }`

Because `cache.save()` at step 8 was **never reached** in Repro B (`empty_cache_sb_1` was never written even after 60s), the freeze happens inside steps 1–7 of `PathTraceCompute::create` (most likely `compile_megakernel_variant` / `create_shader_module` or `GpuBvhBuilder::new` if it reached past step 8).

---

### Fact 2: Exact Differences Between `pt-megakernel` (`test_gpu::device()`, Works) and `squarebob-rs` (`GpuContext::new()`, Hangs)

Compare `render-rs/crates/render-engine-pt/pt-megakernel/src/test_gpu.rs:58-90` (where `PathTraceCompute::new` finishes in 7.37s) with `squarebob-rs/crates/render-core/src/lib.rs:404-514` (where `PathTraceCompute::new` hangs > 60s):

1. **`wgpu::Features` & `ExperimentalFeatures` on `DeviceDescriptor`**:
   - **`test_gpu::device()` (works)**:
     ```rust
     let required_features = if adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
         wgpu::Features::PIPELINE_CACHE
     } else {
         wgpu::Features::empty()
     };
     adapter.request_device(&wgpu::DeviceDescriptor {
         label: Some("pt_megakernel_test_device"),
         required_features,
         required_limits: adapter.limits(),
         ..Default::default()
     })
     ```
   - **`GpuContext::new()` in `squarebob-rs` (hangs)**:
     ```rust
     let required_features = adapter
         .features()
         .difference(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS)
         | wgpu::Features::POLYGON_MODE_LINE;
     let required_limits = gpu_mem::recommended_limits(&adapter);
     adapter.request_device(&wgpu::DeviceDescriptor {
         label: Some("squarebob GPU Device"),
         required_features,
         required_limits,
         memory_hints: wgpu::MemoryHints::MemoryUsage,
         trace: Default::default(),
         experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
     })
     ```
     Note that in `squarebob-rs`, `wgpu_hal::vulkan::adapter` logs:
     `Found 6 cooperative matrix configurations supported by wgpu`
     because `adapter.features()` + `ExperimentalFeatures::enabled()` turns on every Vulkan extension supported by the adapter (cooperative matrix, ray query, subgroup, int64/float64, etc.).

2. **DXGI / PDH Queries in `GpuContext::new()` After `request_device`**:
   - Right after `adapter.request_device(...)`, `GpuContext::new()` (`crates/render-core/src/lib.rs:463-485`) calls:
     - `gpu_mem::os::query()` (which opens DXGI `EnumAdapters1` + Windows PDH `GPU Adapter Memory` counters)
     - `gpu_mem::vram_budget_bytes(&adapter)`
   - Recall the documented NVIDIA Windows driver (`616.64` / `621.14`) loader/ICD lock hazard in `pt-megakernel/src/test_gpu.rs:3-13` when multiple graphics APIs / adapter enumerations interact with `vkCreateComputePipelines`.

3. **WGSL `override` Specialization Pattern in `bvh_traverse.wgsl` (`render-rs` commit `23c01af`)**:
   - In `render-rs/crates/render-engine-pt/pt-megakernel/src/bvh_traverse.wgsl:16-23` and `233-259`, `plan4.md` (`O11`) introduced 8 `override SPEC_*` constants:
     ```wgsl
     override SPEC_USE_OVERRIDES: bool = false;
     override SPEC_TWO_LEVEL: bool = false;
     ...
     fn pt_spec_two_level() -> bool {
         return select(camera.two_level != 0u, SPEC_TWO_LEVEL, SPEC_USE_OVERRIDES);
     }
     ```
   - Notice two issues with this pattern:
     1. In WGSL/Naga, `select(camera.two_level != 0u, SPEC_TWO_LEVEL, SPEC_USE_OVERRIDES)` evaluates `camera.two_level != 0u` eagerly (a runtime uniform load), so Naga's `process_overrides` pass does **not** constant-fold `pt_spec_two_level()` into a compile-time constant `bool`, leaving both flat-BVH and two-level TLAS/BLAS traversal loops, volumes, spectral, and AOV branches alive in the generated SPIR-V module.
     2. `PathTraceCompute::create` compiles `PipelineVariantKey::INITIAL` (`two_level: false, adaptive: false`), and then on the very first `dispatch()` in `squarebob-rs` (which always uses `two_level: true`), `ensure_megakernel_pipeline` compiles a **second** full megakernel pipeline variant (`two_level: true, adaptive: ...`) on the UI thread.
   - In `render-rs` commit `23c01af`, `bvh_traverse.wgsl` + `pt-integrator` + `standard-surface-bsdf` also grew significantly (hot/cold `GpuTriGeom` + `GpuTriShade` bindings `0..31`, emissive triangle NEE, 4-way `SunSky` MIS, Turquin multiscatter, Conty-Kulla sheen, anisotropic Smith `G2`, multi-dimensional Sobol + blue-noise tables).

4. **Cargo Profile Difference (`[profile.release]` vs `[profile.test]`)**:
   - `squarebob.exe` is built with `[profile.release]` (`opt-level = 3`), whereas `cargo test -p pt-megakernel` ran with `[profile.dev]` (`[profile.dev.package."*"] debug = false`).

---

## 5. Immediate Next Diagnostic Steps for Investigation

1. **Step-by-step instrumentation inside `PathTraceCompute::create` (`render-rs/crates/render-engine-pt/pt-megakernel/src/compute.rs:2435-2765`)**:
   - Print timestamps before/after:
     - `Snapshot::initial`
     - `device.create_shader_module` (`bvh_traverse_shader`)
     - `PtPipelineCache::open`
     - `Self::compile_megakernel_variant` (`PipelineVariantKey::INITIAL`)
     - `device.create_compute_pipeline` (`pt_pick_pipeline`)
     - `Self::create_blit_render_pipeline`
     - `GpuBvhBuilder::new`
   - Run against both `test_gpu::device()` and a test using `GpuContext::new()`'s exact `DeviceDescriptor` (`required_features`, `required_limits`, `experimental_features`).
2. **Isolate `required_features` / `experimental_features` vs Shader Complexity**:
   - Check whether stripping `experimental_features` / extraneous `adapter.features()` in `GpuContext::new()` (keeping only what Burn/CubeCL and `squarebob-rs` actually need) or fixing `if (SPEC_USE_OVERRIDES) { return SPEC_TWO_LEVEL; } else { return camera.two_level != 0u; }` in `bvh_traverse.wgsl` (so Naga's constant folding / DCE eliminates dead branches before SPIR-V emission) resolves the `vkCreateComputePipelines` hang/slowdown.
3. **Eliminate Double Megakernel Compilation on First Frame**:
   - Do not eagerly compile an unused `PipelineVariantKey::INITIAL` (`two_level: false`) in `PathTraceCompute::create` if the caller immediately dispatches with `two_level: true`, or match `INITIAL` to the default production variant.

---

## 6. Resolution & Verification (2026-11-08)

Full root-cause report: [`docs/bughunt/startup-2026-11-08.md`](docs/bughunt/startup-2026-11-08.md) | Repair plan: [`docs/plans/plan19.md`](docs/plans/plan19.md).

1. **Confirmed Root Cause**:
   - `ensure_megakernel_pipeline` compiled uncached specialized `bvh_traverse` compute pipelines synchronously on the UI thread (`46.736s` inside `nvoglv64.dll` SPIR-V compilation on first launch), after `PathTraceCompute::create` had already compiled an unused `PipelineVariantKey::INITIAL` (`two_level: false`).
   - Additionally, `select(runtime_expr, SPEC_*, SPEC_USE_OVERRIDES)` in `bvh_traverse.wgsl` prevented `naga::back::pipeline_constants::process_function` from folding dead specialization branches before SPIR-V codegen.

2. **Systemic Fixes Applied**:
   - **`render-rs` (`5eeebc2` on `origin/main`)**:
     - Replaced `select(runtime_expr, SPEC_*, SPEC_USE_OVERRIDES)` in `bvh_traverse.wgsl` with `if SPEC_USE_OVERRIDES { return SPEC_*; } return runtime_expr;`, allowing Naga to fold constant branches and reducing cold NVIDIA driver compile time from `46.74s` to `21.82s` (`0.017s` on warm disk cache).
     - Removed eager synchronous `PipelineVariantKey::INITIAL` compilation from `PathTraceCompute::create`.
     - Moved `compile_megakernel_variant` + `gpu_mem::checked` onto a retained background worker job (`PendingPipelineJob`, thread `pt-pipeline-compile`) in `PathTraceCompute` (`is_preparing_pipeline`, `preparing_pipeline_elapsed`, `poll_pipeline`, `wait_for_pipeline`, `set_blocking_pipeline_compile`), retaining `Checked<wgpu::ComputePipeline>` across `Poll::Pending` frames and forwarding the API on `PtRenderer`.
   - **`squarebob-rs`**:
     - Updated `render-3d`, `src/app/{treemap_view, status_bar, screenshot}.rs`, and `src/cli_test.rs` to keep the UI responsive (`0.16ms–0.34ms` per frame) with a status indicator while the PT shader compiles in the background, and fixed `try_reserve_exact(output_size)` after `clear()` in `crates/render-core/src/lib.rs` and `crates/render-3d/src/pt/blit.rs`.

3. **Measured Verification (`D:\temp`, 4,043 cubes, 1231x969)**:
   - **Cold cache**: UI frames tick at `0.16ms–0.34ms` during `21.816s` background compile; frame 1 renders in `113.72ms`, subsequent frames `58–69ms`.
   - **Warm cache**: pipeline loads in `17ms` (`compile=0.017s, total=0.029s`), full `--screenshot 2 --exit-after-screenshot` run finishes in `4.59s` with exit `0`.
   - **`squarebob.exe test gpu-pt-smoke`**: `OK` (`finite=16384/16384 nonzero=16384`).
