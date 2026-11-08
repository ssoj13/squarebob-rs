# Plan19 — PT startup compilation stall (2026-11-08)

Evidence and reproduction: [startup investigation](../bughunt/startup-2026-11-08.md).

## Verified conclusion

The first GUI dispatch compiles a missing specialized PT pipeline synchronously on the UI thread. Measured dispatch 46.736 s / frame 46.890 s; the application then rendered and saved its screenshot. Warm repeat: dispatch 113.47 ms / frame 269.28 ms / whole invocation 11.120 s, exit 0. The installed smoke test also passes. This supersedes permanent-hang/constructor-only interpretations in historical BUG0.md for this measured configuration.

## Completed investigation

- [x] Compare today's Squarebob/dependency revisions and preserve existing BUG0.md.
- [x] Refresh GitNexus to HEAD and inspect PT call path plus native call stack.
- [x] Correlate compiler stack with CPU consumption and actual eventual completion.
- [x] Run full GUI warm-repeat, headless PT smoke and raster screenshot controls.
- [x] Check appbus integration: bus-only, no REST server startup in this executable.
- [x] Inspect sibling ensure paths and avoid duplicate fixes.
- [x] Update root/targeted operational notes and real dataflow diagrams.
- [x] Finish workspace/all-target Clippy: final structured exit 0 in 3.341 s,295 stderr lines; all warning sites inspected and classified in the report.

## Production repair — Implemented & Verified (2026-11-08)

1. [x] In canonical render-rs pt-megakernel (`5eeebc2`), made variant compilation a retained background job (`PendingPipelineJob` on worker thread `pt-pipeline-compile`) per active key. Kept GPU device/context ownership unchanged. Compilation and its entire error-scope lifecycle (`gpu_mem::checked`) execute outside the GUI event handler.
2. [x] Retained the `Checked<wgpu::ComputePipeline>` validation result on `PendingPipelineJob` until settled (`Poll::Ready`), including `Poll::Pending`; never recompiles or drops the result each frame. Genuine validation errors surface once and cache the key in `failed_pipeline_keys`; stale jobs are ignored when the active `PipelineVariantKey` changes.
3. [x] In Squarebob (`render-3d`, `src/app/{treemap_view, status_bar, screenshot}.rs`), presented a responsive preparation state (`pt_is_preparing_pipeline()`, `pt_preparing_pipeline_elapsed()`, viewport spinner + status bar progress indicator) while the requested pipeline compiles in the background; preserved strict explicit-backend semantics and guarded `pt_auto_spp` / `static_converged` against `0`-sample preparation frames. Synchronous CLI/CPU readbacks (`PtOutput::CpuReadback`, `gpu_pt_smoke`) wait via `wait_for_pipeline`.
4. [x] Removed eager `PipelineVariantKey::INITIAL` synchronous compilation from `PathTraceCompute::create`. Only the actual first-use variant is requested through `ensure_megakernel_pipeline`.
5. [x] Added start/end/duration/key/cache diagnostics (`[pt-megakernel] compiling pipeline variant ...` and `[pt-megakernel] compiled pipeline variant ... (cache=..., compile=...s, total=...s)`) at the canonical pipeline owner.
6. [x] Replaced `select(runtime_expr, SPEC_*, SPEC_USE_OVERRIDES)` in `bvh_traverse.wgsl` with `if SPEC_USE_OVERRIDES { return SPEC_*; } return runtime_expr;` so `naga::back::pipeline_constants::process_function` folds dead specialization branches before SPIR-V codegen, cutting cold NVIDIA driver compilation time per variant from `46.74s` to `21.82s` (`0.017s` on warm disk cache).
7. [x] Fixed `pixels.try_reserve_exact(output_size)` after `clear()` in `crates/render-core/src/lib.rs` (with growing-buffer regression test `map_readback_into_reserves_full_output_size_after_clear`) and `crates/render-3d/src/pt/blit.rs`.

## Acceptance gates

- [x] A controlled first-use compile can take tens of seconds (`21.816s` cold) while the window keeps painting (`0.16ms–0.34ms` per UI frame) and accepts close/settings input.
- [x] Delayed compilation/validation and repeated UI polling cause one compile per key, with errors propagated and no dropped `Pending` result (`nonblocking_pipeline_compilation_retains_pending_and_rejects_stale_keys`).
- [x] Changing key/scene/backend during preparation cannot publish stale resources or reset unrelated state (`nonblocking_pipeline_compilation_retains_pending_and_rejects_stale_keys`).
- [x] Warm (`compile=0.017s`, total process `4.59s`) and controlled uncached-variant (`compile=21.816s`) runs both render finite expected pixels, including TLAS, adaptive and AOV configurations.
- [x] Tested both `dispatch` (sample-only) and `dispatch_and_blit` (combined sample/blit) recording entries (`58 passed` in `pt-megakernel`, `17 passed` in `pt-renderer`).
- [x] Preserved GPU/CPU PT, OIDN guides, display/exposure and explicit CUDA behavior.
- [x] Ran focused `render-rs` tests, Squarebob `gpu-pt-smoke` (`OK`), `render-core` / `render-3d` unit tests (`18 passed`), and GUI verification on `D:\temp`.
