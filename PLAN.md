# `squarebob-rs` — Consolidated Master Plan & Architecture Roadmap (`PLAN.md`)

**Last Updated**: 2026-11-08
**Status**: All active milestones (`plan10..plan19`, `BUG0`, `BUG1`) implemented, verified on `D:\temp`, installed to `~/.cargo/bin/squarebob.exe`, and pushed to `origin/main`.

---

## 1. Non-Blocking Background PT Pipeline Compilation & Startup Repair (`2026-11-08`, formerly `plan19` & `BUG0`)

### Root Cause
- On first 3D Path Tracing frame, `PathTraceCompute::ensure_megakernel_pipeline` compiled uncached specialized `bvh_traverse.wgsl` compute pipelines synchronously on the UI thread (`46.736s` inside `nvoglv64.dll` SPIR-V compilation on cold driver cache), freezing the window (`Not Responding`).
- Additionally, `select(runtime_expr, SPEC_*, SPEC_USE_OVERRIDES)` in `bvh_traverse.wgsl` prevented `naga::back::pipeline_constants::process_function` from folding dead specialization branches before SPIR-V codegen, and `PathTraceCompute::create` eagerly compiled an unused `PipelineVariantKey::INITIAL` (`two_level: false`).

### Production Repair (Implemented & Verified)
- [x] **Naga Constant-Branch Folding (`render-rs@35e38da`)**: Replaced `select(runtime_expr, SPEC_*, SPEC_USE_OVERRIDES)` in `bvh_traverse.wgsl` with `if SPEC_USE_OVERRIDES { return SPEC_*; } return runtime_expr;`, cutting cold NVIDIA driver compile time per variant from `46.74s` to `21.82s` (`0.017s` on warm disk cache).
- [x] **Retained Background Compilation Worker (`PendingPipelineJob` in `render-rs`)**: Removed eager `PipelineVariantKey::INITIAL` from `PathTraceCompute::create` and moved `compile_megakernel_variant` + `gpu_mem::checked` onto background thread `pt-pipeline-compile`, retaining `Checked<wgpu::ComputePipeline>` across `Poll::Pending` frames.
- [x] **Responsive Preparation State in `squarebob-rs`**:
  - Exposed `Renderer3D::{pt_is_preparing_pipeline, pt_preparing_pipeline_elapsed, wait_for_pt_pipeline}` (`crates/render-3d/src/lib.rs`).
  - Guarded `pt_auto_spp` ramp-up and `static_converged` against `0`-sample preparation frames, and added `wait_for_pipeline` for synchronous `PtOutput::CpuReadback` (`crates/render-3d/src/pt/megakernel/render.rs`) and `gpu_pt_smoke` (`src/cli_test.rs`).
  - Added viewport spinner + `"Compiling PT shader (X.Xs)..."` indicator (`src/app/treemap_view.rs`), status-bar progress bar (`src/app/status_bar.rs`), and `--screenshot <delay>` countdown gating (`src/app/screenshot.rs`).
- [x] **Readback Buffer Reservation Fix**: Fixed `pixels.try_reserve_exact(output_size)` after `pixels.clear()` in `crates/render-core/src/lib.rs` (with growing-buffer regression test `map_readback_into_reserves_full_output_size_after_clear`) and `crates/render-3d/src/pt/blit.rs`.

### Measured Verification (`D:\temp`, 4,043 cubes, `1231x969`)
- **Cold cache**: UI frames tick at `0.16ms–0.34ms` during `21.816s` background compile; frame 1 renders in `113.72ms`, subsequent frames `58–69ms`.
- **Warm cache**: pipeline loads in `17ms` (`compile=0.017s, total=0.029s`), full `--screenshot 2 --exit-after-screenshot` run finishes in `4.59s` with exit `0`.
- **`squarebob.exe test gpu-pt-smoke`**: `OK` (`finite=16384/16384 nonzero=16384`).

---

## 2. Dual-Backend PT (`Auto` / `WGPU` / `CUDA`), `render-env` / `render-settings` SSOT & CUDA+OIDN Interop (`2026-11-08`)

- [x] **Environment & Settings SSOT (`crates/render-3d/src/env_map.rs`, `crates/render-shared/src/lib.rs`)**:
  - Deduplicated equirectangular HDRI format detection, sRGB EOTF linearization, and spherical `sin(theta)` CDF construction onto `render_env` with solid-angle-weighted `Rgba16Float` mip generation.
  - Re-exported `AdaptivePreset`, `DenoiserSettings`, `OidnModeOption`, `OidnQualityOption`, `PtComputeBackend`, `PtSamplerMode`, and `SpectralMode` from `render_settings`.
- [x] **Dual-Backend PT & Strict No-Silent-Fallback (`crates/render-3d/src/pt/`, `src/app/settings/renderer.rs`, `src/cli.rs`)**:
  - Wired `PtComputeBackend::{Auto, Wgpu, Cuda}` (`--pt-backend <auto|wgpu|cuda>` and Settings → Backend selector with live `Active: <WGPU|CUDA>` indicator).
- [x] **CUDA + OIDN Interop, Lazy AOVs, Zero-Churn Readback & GPU TLAS Refit**:
  - `pt-cuda::CudaPathTracer` uploads scene-linear `Rgba32Float` beauty (`read_linear_into`) and on-demand primary-hit albedo/normal AOV guides (`Renderer3D::sync_oidn_guides`) into persistent `COPY_SRC` `wgpu` resources consumed by `pt_denoise_oidn::OidnDenoiser` (`DenoiseInput::TexturesUntracked`).
  - Wired lazy AOV allocation (`set_aovs_enabled`), `oidn_last_denoised_spp` final-pass gating, transform-only GPU TLAS refit (`refit_scene_two_level_gpu` / `refit_two_level_instances_gpu`), `MaterialExt` (`transmission_depth`, sheen, anisotropy), `set_emissive_sampling`, and zero-steady-state-allocation `read_pt_linear_into` / `readback_render_texture_into`.

---

## 3. Completed Historical Milestones (Compressed FIFO)

- [x] **2026-10-27 (`BUG1` & 1M–20M Instancing)**: Removed duplicate in-tree PT/BVH/OIDN/color crates in favor of canonical `render-rs` and `gpu-info-rs`; implemented analytic unit-cube TLAS (`ANALYTIC_UNIT_CUBE_BLAS_ROOT = u32::MAX`, `0` triangles/cube), compact 4,096-entry `GpuMaterial` variant pool (`PT_VARIANTS_PER_SLOT = 16`), single-pass MRT PBR/Wireframe + `R32Uint` `object_id`, and 12-edge `LineList` wireframe.
- [x] **2026-10-23 (Exact OCIO Display & `render-color-gpu`)**: Replaced `crates/color-pipeline` and baked $33^3$ LUTs with `render-color` + `render-color-gpu::OcioPass` exact WGSL shader and `egui-display` HDR10 (PQ) / HLG / scRGB / SDR presentation (`plan16`–`plan18`).
- [x] **2026-10-15 (`pt-denoise-oidn` Shared Bridge)**: Unified GPU OIDN denoising onto `render-rs`'s `pt-denoise-oidn` and `burn_device::shared` single-process registration.
- [x] **2026-09-23 (`plan11`–`plan15` Systemic Scanner, Cache & Readback Hardening)**: Canonical `ScanRoot` identity, `fscan-rs` standard + typed NTFS MFT scanner with generation-gated `poll_scan`, atomic `CacheService` v4, fallible `TextureReadbackLayout` / `map_readback`, and RON persistence.
- [x] **2026-07-15 (`PLAN10` Pure-Rust Video Export via `ffmpeg-rs`)**: Replaced `playa-ffmpeg` / `ffmpeg-sys-next` with `ffmpeg-rs` (`OpenH264`, HEVC 8/10-bit, AV1, ProRes, `MovWriter`).
