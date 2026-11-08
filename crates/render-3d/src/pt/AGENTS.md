# PT startup and compilation ownership

The startup diagnosis and implemented repair are recorded in [PLAN.md](../../../../PLAN.md).

- The WGPU megakernel's first dispatch can synchronously compile a missing specialization in canonical render-rs. This blocked the GUI for 46.736 s in the measured 2026-11-08 run, then completed.
- `megakernel/render.rs:539` calls dispatch inside the GUI frame. The same shared ensure path is also used by render-rs's combined sample/blit recording.
- `gpu_mem::checked` executes its operation immediately; its Pending status describes validation, not asynchronous shader compilation.
- Preserve shared WorkContext/device/cache ownership and retain pending compilation/validation state. Keep owned UI trees on the UI side.
- Do not infer an infinite deadlock from the last log or an interrupted cold startup. Measure cache misses, main-thread stacks, eventual completion and warm restarts separately.
- Preserve explicit CUDA failure behavior, AOV/denoiser modes and display/exposure settings. Do not silently disable rendering features as a startup repair.
- **2026-11-08 — Background PT pipeline compilation implemented (`render-rs@5eeebc2`)**:
  - `PathTraceCompute::ensure_megakernel_pipeline` now spawns `PendingPipelineJob` (`pt-pipeline-compile` worker thread) and retains `Checked<wgpu::ComputePipeline>` across `Poll::Pending` frames without blocking the UI thread (`0.16ms–0.34ms` per frame while compiling).
  - Constant-branch specialization in `bvh_traverse.wgsl` (`if SPEC_USE_OVERRIDES { return SPEC_*; } return runtime_expr;`) reduced cold NVIDIA driver compile time per variant from `46.74s` to `21.82s` (`0.017s` on warm disk cache).
  - `Renderer3D` exposes `pt_is_preparing_pipeline()`, `pt_preparing_pipeline_elapsed()`, and `wait_for_pt_pipeline()`, wired into `treemap_view.rs`, `status_bar.rs`, `screenshot.rs`, `megakernel/render.rs` (`PtOutput::CpuReadback`), and `cli_test.rs` (`gpu_pt_smoke`).
