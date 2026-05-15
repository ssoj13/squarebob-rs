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
   - [x] Model-size selector (Small / Base / Large) wired to OIDN's
         `_small`/base/`_large` registry; UI tooltips name the actual TZA file.
   - [ ] **oidn-rs Phase I (deferred)**: lift `unet_runner` pre/post-process
         and `autoexposure` from CPU loops into Burn ops on shared wgpu.
         Saves ~80 ms/1080p and ~250 ms/4K of host roundtrip. Work happens
         in the `oidn-rs` repo, not here.

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
