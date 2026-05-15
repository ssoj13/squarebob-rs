1. [x] get rid of separate "denoise" tab and move it to Settings tab
       (Denoiser UI lives under Settings → Rendering as a regular section,
       not a top-level dock tab.)

2. [x] Finish the denoiser / add Intel OIDN
   - [x] Vendor `oidn-rs` (Burn-wgpu port). All 23 TZA weights (~48 MB)
         bundled to `data/oidn-weights/`.
   - [x] Shared `wgpu::{Instance,Adapter,Device,Queue}` across PT + Burn + eframe.
   - [x] Wavefront and megakernel both write primary-hit `albedo`/`normal` AOVs.
   - [x] `OidnDenoiser` with lazy build, graceful AOV downgrade, manual + auto triggers.
   - [x] Render-loop wire: display switches raw↔denoised, latency shown in UI.
   - [x] Display routed through `blit_with_source` so OIDN result inherits
         ACES tonemap + gamma + hover/selection overlay.
   - [x] Model-size selector (Small / Base / Large) wired to OIDN's
         `_small`/base/`_large` registry; UI tooltips name the actual TZA file.
   - [x] Periodic re-fire `pt_oidn_interval` (default 128 spp) on top of
         the final-spp trigger.
   - [x] Shared-device kernels actually run (full adapter features +
         `experimental_features = ExperimentalFeatures::enabled()` for
         SPIR-V passthrough; `max_buffer_size` / `max_storage_buffer_binding_size`
         raised to adapter caps; missing texture/buffer `COPY_SRC` flags).
   - [x] **First-tier perf caching landed.**
         TZA bytes cache, reused staging buffers, idempotent
         `RtFilter::commit()`, and full `RtFilter<'static>` reuse across
         denoise calls via `Box::leak(burn_device)`. ~80-100 ms saved per
         periodic re-fire after the first.
   - [ ] **`oidn-rs` Phase I — full GPU pipeline (deferred).**
         Lift `unet_runner` pre/post-process (pack + transfer fn forward,
         reflect-pad, inverse transfer, tile stitch) and `autoexposure`
         from CPU pixel loops to Burn tensor ops on shared wgpu. Removes
         the remaining 50 ms (1080p) / 200 ms (4K) host-roundtrip /
         CPU-loop cost per denoise. Detailed plan:
         `docs/oidn-phase1-plan.md`.

3. [x] **Adaptive sampling — bugfixes landed.**
   - [x] Welford variance buffer was not cleared on accumulation reset;
         stale mean/M2 mixed with fresh samples across camera/scene
         changes → catastrophically wrong allocations. Fixed in both
         megakernel and wavefront dispatch paths.
   - [x] `allocate.wgsl` now uses DMC-style relative noise
         (`std_err / max(luminance(mean), eps)`) — single threshold works
         across full HDR range instead of clipping by absolute variance.
   - [ ] **Verify visually** that the sampler actually concentrates
         budget on noisy pixels (V-Ray-style DMC behaviour) on a reference
         scene with explicit noise asymmetry — e.g. dark interior + bright
         window. Bug-fix correctness has been audited but on-screen
         visual confirmation hasn't been recorded yet.

4. [x] **Global samples knob (unification).** `Render3DOptions::pt_samples`
       is the single V-Ray-style top-level number. Adaptive
       `min_spp`/`max_spp` removed from `Render3DOptions` and now derived
       in `render-3d` dispatchers: `min = max(samples/16, 8)`,
       `max = samples`. UI has one slider; the adaptive section shows the
       derived range as a hint instead of editable controls.

## Packaging / release polish

- **Rename release binary.** Some artifact / installer paths still emit a
  `dirstat-rs.exe`-style filename from the upstream fork. Audit
  `Cargo.toml` `[package.metadata.packager]`, `bootstrap.py`,
  `.github/workflows/ci.yml`, and any platform-specific signing/NSIS
  configs — every produced binary should be named after `squarebob` (`squarebob.exe`
  / `Squarebob.app` / `squarebob`).

## Rendering correctness landed today
- [x] **Reversed-Z + infinite far plane.** `Mat4::perspective_rh(near, far)`
      → `Mat4::perspective_infinite_reverse_rh(near)`; all PBR / wireframe
      / object-id / skybox pipelines flipped to `Greater(Equal)`; depth
      clears to `0.0`; picking ray NDC z swapped. Eliminates the strobing
      background that appeared in PBR/wireframe on camera rotation.
- [x] **OIDN dark-output fix.** Result now goes through PT's
      `blit_with_source` (ACES + gamma) instead of a separate egui-native
      registration that skipped tone-mapping.
- [x] **OIDN renderer-side shared-device bring-up.** Three fixes:
      full adapter features + `ExperimentalFeatures::enabled()` (kernels
      were silently no-op'ing); raised `max_buffer_size` /
      `max_storage_buffer_binding_size` (cubecl was panicking on pool
      init); added `COPY_SRC` to `pt_output` and wavefront AOV buffers.

## Follow-ups outside this initial sprint

- **OIDN visual regression test.** Reference noisy scene at 16 spp,
  denoise, PSNR against 4096-spp ground truth. Should exceed +10 dB if
  things work.
- **Bench CSV.** `data/benchmarks/oidn-2026-05-14.csv` across
  (mode × size × resolution). Currently relies on log line latency only.
- **Mode-driven AOV allocation (optional VRAM saving).** Today AOV
  buffers are always allocated and written. Conditional WGSL + lazy
  resize saves ~128 MB / 1080p, ~512 MB / 4K. Only worth doing if VRAM
  pressure shows up on integrated GPUs.
- **ReSTIR temporal/spatial tuning** (carried over from PT README).
- **BVH refit fallback** for extreme animated scenes
  (carried over from PT README).
- **Age-based color via real mtime plumbing** from scanner
  (carried over from PT README).
